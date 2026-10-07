//! Pure table workflow contract. No persistence, file writes or implicit execution.
use super::{Dataset, transform};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
mod rows;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Predicate {
    Contains,
    NotContains,
    Equals,
    NotEquals,
    IsNull,
    IsNotNull,
}
impl Predicate {
    pub fn label(self) -> &'static str {
        match self {
            Self::Contains => "包含文本",
            Self::NotContains => "不包含文本",
            Self::Equals => "等于JSON值",
            Self::NotEquals => "不等于JSON值",
            Self::IsNull => "是null",
            Self::IsNotNull => "不是null",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SortKey {
    pub column: String,
    pub descending: bool,
}

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
    SelectColumns {
        columns: Vec<String>,
    },
    Filter {
        column: String,
        predicate: Predicate,
        value: String,
        case_sensitive: bool,
    },
    Sort {
        keys: Vec<SortKey>,
    },
    Deduplicate {
        columns: Vec<String>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub version: u32,
    pub name: String,
    pub steps: Vec<Step>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Output>,
}

/// Versioned output settings only. Targets and permission are supplied per run.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "format", rename_all = "snake_case", deny_unknown_fields)]
pub enum Output {
    Csv {
        version: u32,
        protect_formulas: bool,
    },
    Sqlite {
        version: u32,
        table: String,
    },
}
impl Output {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Csv { version, .. } => ensure!(*version == 1, "不支持的CSV输出版本"),
            Self::Sqlite { version, table } => {
                ensure!(*version == 1, "不支持的SQLite输出版本");
                ensure!(
                    !table.trim().is_empty() && table.len() <= 128 && !table.contains('\0'),
                    "输出表名须为1–128字节且不能包含NUL"
                );
            }
        }
        Ok(())
    }
    pub fn summary(&self) -> String {
        match self {
            Self::Csv {
                protect_formulas, ..
            } => format!(
                "CSV · 文本公式保护{}",
                if *protect_formulas {
                    "开启"
                } else {
                    "关闭"
                }
            ),
            Self::Sqlite { table, .. } => format!("SQLite · 表名 {table}"),
        }
    }
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
        ensure!(matches!(self.version, 1..=3), "不支持的流程定义版本");
        if let Some(output) = &self.output {
            ensure!(self.version == 3, "保存输出配置需要流程格式3");
            output.validate()?;
        }
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
                Step::SelectColumns { columns } | Step::Deduplicate { columns } => {
                    !columns.is_empty()
                        && columns.len() <= 128
                        && columns.iter().all(|column| valid_column(column))
                        && columns
                            .iter()
                            .collect::<std::collections::BTreeSet<_>>()
                            .len()
                            == columns.len()
                }
                Step::Filter {
                    column,
                    predicate,
                    value,
                    ..
                } => {
                    valid_column(column)
                        && value.len() <= 4096
                        && match predicate {
                            Predicate::Equals | Predicate::NotEquals => {
                                serde_json::from_str::<serde_json::Value>(value).is_ok()
                            }
                            Predicate::IsNull | Predicate::IsNotNull => value.is_empty(),
                            _ => !value.is_empty(),
                        }
                }
                Step::Sort { keys } => {
                    !keys.is_empty()
                        && keys.len() <= 4
                        && keys.iter().all(|key| valid_column(&key.column))
                        && keys
                            .iter()
                            .map(|key| &key.column)
                            .collect::<std::collections::BTreeSet<_>>()
                            .len()
                            == keys.len()
                }
            };
            ensure!(
                self.version >= 2
                    || matches!(step, Step::Column { .. } | Step::SelectColumns { .. }),
                "第{}步的行操作需要格式版本2",
                index + 1
            );
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
                    _ => rows::propose(&current, step, cancel),
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

    #[test]
    fn versioned_outputs_roundtrip_without_a_target_or_execution_grant() {
        let mut def = definition(vec![Step::SelectColumns {
            columns: vec!["id".into()],
        }]);
        let source = Dataset::parse("id,name\n001,original", DataFormat::Csv, b',').unwrap();
        for output in [
            Output::Csv {
                version: 1,
                protect_formulas: false,
            },
            Output::Sqlite {
                version: 1,
                table: "教程\"数据".into(),
            },
        ] {
            def.version = 3;
            def.output = Some(output);
            let bytes = serde_json::to_vec(&def).unwrap();
            let loaded = Definition::parse(&bytes).unwrap();
            assert_eq!(loaded, def);
            assert_eq!(
                loaded
                    .preview(&source, &AtomicBool::new(false))
                    .unwrap()
                    .result
                    .rows[0][0],
                "001"
            );
            let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            let keys: Vec<_> = json["output"]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert!(
                keys.iter()
                    .all(|key| ["format", "version", "table", "protect_formulas"].contains(key))
            );
        }
        for version in [1, 2] {
            def.version = version;
            def.output = None;
            let bytes = serde_json::to_vec(&def).unwrap();
            assert!(!String::from_utf8(bytes.clone()).unwrap().contains("output"));
            assert_eq!(Definition::parse(&bytes).unwrap().output, None);
        }
    }
    #[test]
    fn output_contract_rejects_old_envelopes_unknown_actions_and_grants() {
        let mut value = json!({"version":3,"name":"clean","steps":[{"action":"select_columns","columns":["id"]}],"output":{"format":"csv","version":1,"protect_formulas":true}});
        for (field, data) in [
            ("path", json!("C:/old.csv")),
            ("authorized", json!(true)),
            ("command", json!("run")),
        ] {
            let mut unsafe_value = value.clone();
            unsafe_value["output"][field] = data;
            assert!(Definition::parse(&serde_json::to_vec(&unsafe_value).unwrap()).is_err());
        }
        for version in [1, 2, 4] {
            let mut wrong = value.clone();
            wrong["version"] = json!(version);
            assert!(Definition::parse(&serde_json::to_vec(&wrong).unwrap()).is_err());
        }
        value["output"]["version"] = json!(2);
        assert!(Definition::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        value["output"] = json!({"format":"shell","version":1});
        assert!(Definition::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        for table in ["".to_owned(), " ".into(), "x\0y".into(), "中".repeat(43)] {
            value["output"] = json!({"format":"sqlite","version":1,"table":table});
            assert!(Definition::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        }
    }

    fn definition(steps: Vec<Step>) -> Definition {
        Definition {
            output: None,
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
