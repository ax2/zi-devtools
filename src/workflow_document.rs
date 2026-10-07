//! One metadata/read contract for table workflows and native typed-tool recipes.
use anyhow::{Context, Result, ensure};
use std::{io::Read, path::Path};
pub(crate) enum Document {
    Table(crate::workbench::workflow::Definition),
    Tool(crate::text_flow::Definition),
}
impl Document {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= 256 * 1024, "流程文件最多256 KiB");
        if let Ok(definition) = crate::workbench::workflow::Definition::parse(bytes) {
            return Ok(Self::Table(definition));
        }
        ensure!(bytes.len() <= 65536, "工具配方最多64 KiB");
        let definition: crate::text_flow::Definition =
            serde_json::from_slice(bytes).context("不支持的表格流程或工具配方")?;
        definition.validate()?;
        ensure!(!definition.steps.is_empty(), "工具配方需至少一个步骤");
        Ok(Self::Tool(definition))
    }
    pub fn load(path: &Path) -> Result<Self> {
        ensure!(path.is_absolute(), "请选择绝对路径");
        let mut bytes = Vec::new();
        crate::local_files::open_regular(path, 256 * 1024)?
            .take(256 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        Self::parse(&bytes)
    }
    pub fn metadata(&self, path: &Path) -> crate::preferences::SavedWorkflow {
        let (name, steps) = match self {
            Self::Table(def) => (def.name.clone(), def.steps.len()),
            Self::Tool(def) => (
                format!(
                    "工具流程 · {}",
                    path.file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .chars()
                        .take(100)
                        .collect::<String>()
                ),
                def.steps.len(),
            ),
        };
        crate::preferences::SavedWorkflow {
            path: path.to_path_buf(),
            name,
            steps,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_recipe_contract_rejects_unknown_authority_empty_and_incompatible_chains() {
        for bytes in [br#"{"version":2,"steps":[],"input":"secret"}"#.as_slice(),br#"{"version":2,"steps":[]}"#,br#"{"version":2,"steps":[{"action":"table.parse_csv","version":1},{"action":"base64.encode","version":1}]}"#]{assert!(Document::parse(bytes).is_err());}
        assert!(
            Document::parse(br#"{"version":1,"steps":[{"action":"base64.encode","version":1}]}"#)
                .is_ok()
        );
    }
}
