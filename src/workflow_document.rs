//! One metadata/read contract for table workflows and native typed-tool recipes.
use anyhow::{Context, Result, ensure};
use std::{io::Read, path::Path};
pub(crate) enum Document {
    Table(crate::workbench::workflow::Definition),
    Tool(crate::text_flow::Definition),
    Image(crate::image_tools::WorkflowDefinition),
}
impl Document {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= 256 * 1024, "流程文件最多256 KiB");
        if let Ok(definition) = crate::workbench::workflow::Definition::parse(bytes) {
            return Ok(Self::Table(definition));
        }
        ensure!(bytes.len() <= 65536, "工具配方最多64 KiB");
        let header: serde_json::Value =
            serde_json::from_slice(bytes).context("流程JSON格式无效")?;
        if header.get("kind").and_then(serde_json::Value::as_str) == Some("image-workflow") {
            return Ok(Self::Image(crate::image_tools::WorkflowDefinition::parse(
                bytes,
            )?));
        }
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
            Self::Image(def) => (
                format!(
                    "图片流程 · {}",
                    path.file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .chars()
                        .take(100)
                        .collect::<String>()
                ),
                def.steps.len(),
            ),
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
    fn image_definitions_reject_unknown_fields_versions_empty_and_oversize() {
        for bytes in [
            br#"{"kind":"image-workflow","schema_version":1,"steps":[],"input":"private"}"#.as_slice(),
            br#"{"kind":"image-workflow","schema_version":1,"steps":[]}"#,
            br#"{"kind":"image-workflow","schema_version":2,"steps":[{"action":"image.info","version":1}]}"#,
            br#"{"kind":"image-workflow","schema_version":1,"steps":[{"action":"image.crop","version":1,"x":9999,"y":0,"width":2,"height":10000}]}"#,
        ] { assert!(Document::parse(bytes).is_err()); }
        let mut bytes=br#"{"kind":"image-workflow","schema_version":1,"steps":[{"action":"image.info","version":1}]}"#.to_vec();
        assert!(matches!(
            Document::parse(&bytes).unwrap(),
            Document::Image(_)
        ));
        bytes.resize(65537, b' ');
        assert!(Document::parse(&bytes).is_err());
    }
    #[test]
    fn typed_recipe_contract_rejects_unknown_authority_empty_and_incompatible_chains() {
        for bytes in [br#"{"version":2,"steps":[],"input":"secret"}"#.as_slice(),br#"{"version":2,"steps":[]}"#,br#"{"version":2,"steps":[{"action":"table.parse_csv","version":1},{"action":"base64.encode","version":1}]}"#]{assert!(Document::parse(bytes).is_err());}
        assert!(
            Document::parse(br#"{"version":1,"steps":[{"action":"base64.encode","version":1}]}"#)
                .is_ok()
        );
    }
}
