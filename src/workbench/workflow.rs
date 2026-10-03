//! Pure table workflow contract. No persistence, file writes or implicit execution.
use super::{Dataset, transform};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_STEPS: usize = 32;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_DEFINITION_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ColumnOperation {
    Trim,
    Lower,
    Upper,
    EmptyToNull,
    FillNull,
    Rename,
    ToText,
    ToInteger,
    ToNumber,
    ToBool,
}

impl From<ColumnOperation> for transform::Operation {
    fn from(value: ColumnOperation) -> Self {
        match value {
            ColumnOperation::Trim => Self::Trim,
            ColumnOperation::Lower => Self::Lower,
            ColumnOperation::Upper => Self::Upper,
            ColumnOperation::EmptyToNull => Self::EmptyToNull,
            ColumnOperation::FillNull => Self::FillNull,
            ColumnOperation::Rename => Self::Rename,
            ColumnOperation::ToText => Self::ToText,
            ColumnOperation::ToInteger => Self::ToInteger,
            ColumnOperation::ToNumber => Self::ToNumber,
            ColumnOperation::ToBool => Self::ToBool,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    Column {
        column: String,
        operation: ColumnOperation,
        value: String,
    },
    /// Columns stay in source order, matching the existing projection UI.
    SelectColumns { columns: Vec<String> },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub version: u32,
    pub name: String,
    pub steps: Vec<Step>,
}

#[derive(Debug)]
pub struct StepReport {
    pub step: usize,
    pub description: String,
    pub changed: usize,
    pub examples: Vec<(String, String)>,
    pub rows: usize,
    pub columns: usize,
}

pub struct Preview {
    pub result: Dataset,
    pub steps: Vec<StepReport>,
}

impl Definition {
    /// Parse and validate only. Imported definitions never run automatically.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_DEFINITION_BYTES, "流程定义最多256 KiB");
        let definition: Self = serde_json::from_slice(bytes).context("流程定义格式无效")?;
        definition.validate()?;
        Ok(definition)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "不支持的流程定义版本");
        ensure!(
            !self.name.trim().is_empty() && self.name.chars().count() <= 120,
            "流程名称须为1–120字"
        );
        ensure!(
            !self.steps.is_empty() && self.steps.len() <= MAX_STEPS,
            "流程须包含1–32个步骤"
        );
        for (index, step) in self.steps.iter().enumerate() {
            let valid_column = |name: &str| !name.trim().is_empty() && name.len() <= 256;
            let valid = match step {
                Step::Column { column, value, .. } => valid_column(column) && value.len() <= 4096,
                Step::SelectColumns { columns } => {
                    !columns.is_empty()
                        && columns.len() <= 128
                        && columns.iter().all(|column| valid_column(column))
                        && columns
                            .iter()
                            .collect::<std::collections::BTreeSet<_>>()
                            .len()
                            == columns.len()
                }
            };
            ensure!(valid, "第{}步的列或参数无效", index + 1);
        }
        Ok(())
    }

    /// Never mutates the input. Keep only the current table and small step reports.
    /// Cancellation is checked before and after each bounded pure conversion.
    pub fn preview(&self, input: &Dataset, cancel: &AtomicBool) -> Result<Preview> {
        self.validate()?;
        input.validate_saved()?;
        check_cancel(cancel)?;
        bounded(input)?;
        let mut current = input.clone();
        let mut reports = Vec::with_capacity(self.steps.len());
        for (index, step) in self.steps.iter().enumerate() {
            check_cancel(cancel)?;
            let proposal = (|| -> Result<transform::Proposal> {
                match step {
                    Step::Column {
                        column,
                        operation,
                        value,
                    } => {
                        let position = current
                            .headers
                            .iter()
                            .position(|name| name == column)
                            .with_context(|| format!("找不到列：{column}"))?;
                        transform::propose(&current, position, (*operation).into(), value)
                    }
                    Step::SelectColumns { columns } => {
                        for column in columns {
                            ensure!(current.headers.contains(column), "找不到列：{column}");
                        }
                        let keep = current
                            .headers
                            .iter()
                            .map(|name| columns.contains(name))
                            .collect::<Vec<_>>();
                        transform::select_columns(&current, &keep)
                    }
                }
            })()
            .with_context(|| format!("第{}步失败；来源未修改", index + 1))?;
            check_cancel(cancel)?;
            bounded(&proposal.data)?;
            reports.push(StepReport {
                step: index + 1,
                rows: proposal.data.rows.len(),
                columns: proposal.data.headers.len(),
                description: proposal.description,
                changed: proposal.changed,
                examples: proposal.examples,
            });
            current = proposal.data;
        }
        check_cancel(cancel)?;
        Ok(Preview {
            result: current,
            steps: reports,
        })
    }
}

fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    ensure!(!cancel.load(Ordering::Relaxed), "流程已取消；来源未修改");
    Ok(())
}

fn bounded(data: &Dataset) -> Result<()> {
    let mut bytes = data.headers.iter().map(String::len).sum::<usize>();
    ensure!(bytes <= MAX_BYTES, "流程表格最多8 MiB");
    for cell in data.rows.iter().flatten() {
        bytes = bytes
            .checked_add(cell.to_string().len())
            .context("流程表格大小溢出")?;
        ensure!(bytes <= MAX_BYTES, "流程表格最多8 MiB");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workbench::DataFormat;
    use serde_json::json;

    fn definition(steps: Vec<Step>) -> Definition {
        Definition {
            version: 1,
            name: "重复清洗".into(),
            steps,
        }
    }
    fn column(name: &str, operation: ColumnOperation, value: &str) -> Step {
        Step::Column {
            column: name.into(),
            operation,
            value: value.into(),
        }
    }

    #[test]
    fn chain_preserves_source_types_and_resolves_names_after_projection_and_rename() {
        let source = Dataset::parse(
            "编号,姓名,数量\n001, Zi ,2\n002, Code ,3",
            DataFormat::Csv,
            b',',
        )
        .unwrap();
        let original = source.clone();
        let workflow = definition(vec![
            Step::SelectColumns {
                columns: vec!["数量".into(), "编号".into()],
            },
            column("数量", ColumnOperation::Rename, "count"),
            column("count", ColumnOperation::ToInteger, ""),
        ]);
        let preview = workflow.preview(&source, &AtomicBool::new(false)).unwrap();
        assert_eq!(source, original);
        assert_eq!(preview.result.headers, ["编号", "count"]);
        assert_eq!(preview.result.rows[0], [json!("001"), json!(2)]);
        assert_eq!(preview.steps.len(), 3);
        assert_eq!(preview.steps[0].changed, 1);
        assert_eq!(preview.steps[2].changed, 2);
        assert_eq!(
            Definition::parse(&serde_json::to_vec(&workflow).unwrap()).unwrap(),
            workflow
        );
    }

    #[test]
    fn failed_downstream_step_and_pre_cancel_never_modify_input() {
        let source = Dataset::parse("姓名,数量\n Zi ,bad", DataFormat::Csv, b',').unwrap();
        let original = source.clone();
        let workflow = definition(vec![
            column("姓名", ColumnOperation::Trim, ""),
            column("数量", ColumnOperation::ToInteger, ""),
        ]);
        let error = workflow
            .preview(&source, &AtomicBool::new(false))
            .err()
            .unwrap();
        assert!(format!("{error:#}").contains("第2步失败"));
        assert_eq!(source, original);
        assert!(workflow.preview(&source, &AtomicBool::new(true)).is_err());
        assert_eq!(source, original);
    }

    #[test]
    fn reject_unknown_version_fields_missing_columns_and_excessive_definitions() {
        assert!(Definition::parse(br#"{"version":2,"name":"x","steps":[]}"#).is_err());
        assert!(
            Definition::parse(br#"{"version":1,"name":"x","steps":[],"permission":"all"}"#)
                .is_err()
        );
        let source = Dataset {
            headers: vec!["x".into()],
            rows: vec![vec![json!(null)]],
        };
        let missing = definition(vec![column("missing", ColumnOperation::Trim, "")]);
        assert!(missing.preview(&source, &AtomicBool::new(false)).is_err());
        let duplicate = definition(vec![Step::SelectColumns {
            columns: vec!["x".into(), "x".into()],
        }]);
        assert!(duplicate.validate().is_err());
        assert!(
            definition(vec![column("x", ColumnOperation::Trim, ""); 33])
                .validate()
                .is_err()
        );
        let huge = Dataset {
            headers: vec!["x".into()],
            rows: vec![vec![json!("x".repeat(MAX_BYTES))]],
        };
        assert!(
            definition(vec![column("x", ColumnOperation::Trim, "")])
                .preview(&huge, &AtomicBool::new(false))
                .is_err()
        );
    }
}
