//! Bounded file payloads using the shared no-replacement publication primitive.
use super::{Definition, LIMIT, Material};
use anyhow::{Context, Result, ensure};
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
const RECIPE_LIMIT: usize = 65536;
fn read(path: &Path, limit: usize, cancel: &AtomicBool) -> Result<Vec<u8>> {
    ensure!(path.is_absolute(), "请选择完整文件路径");
    ensure!(!cancel.load(Ordering::Relaxed), "操作已取消");
    let mut bytes = Vec::new();
    crate::local_files::open_regular(path, limit)?
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= limit, "文件超出读取上限");
    ensure!(!cancel.load(Ordering::Relaxed), "操作已取消");
    Ok(bytes)
}
pub(super) fn input(path: &Path, cancel: &AtomicBool) -> Result<String> {
    String::from_utf8(read(path, LIMIT, cancel)?)
        .context("请选择UTF-8文本文件，二进制文件不作为文本读取")
}
pub(super) fn recipe(path: &Path, cancel: &AtomicBool) -> Result<Definition> {
    let definition: Definition = serde_json::from_slice(&read(path, RECIPE_LIMIT, cancel)?)?;
    definition.validate()?;
    Ok(definition)
}
pub(super) fn save_recipe(
    definition: &Definition,
    path: &Path,
    cancel: &AtomicBool,
) -> Result<PathBuf> {
    definition.validate()?;
    let bytes = serde_json::to_vec_pretty(definition)?;
    ensure!(bytes.len() <= RECIPE_LIMIT, "流程配方最多64 KiB");
    crate::local_files::save_new_moved(path, &bytes, cancel)
}
pub(super) struct Output {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub summary: String,
    pub preview: String,
    pub format: &'static str,
}
impl Output {
    pub fn prepare(material: &Material, path: PathBuf) -> Result<Self> {
        ensure!(path.is_absolute(), "请选择完整输出路径");
        crate::local_files::leaf(&path)?;
        let parent = path.parent().context("目标目录无效")?.canonicalize()?;
        ensure!(parent.is_dir(), "目标父路径不是目录");
        let path = parent.join(path.file_name().context("目标文件名无效")?);
        ensure!(
            std::fs::symlink_metadata(&path)
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
            "目标已存在，不覆盖，请选择新文件名"
        );
        let bytes = material.render()?.into_bytes();
        ensure!(bytes.len() <= LIMIT, "结果最多1 MiB");
        Ok(Self {
            path,
            bytes,
            summary: material.summary(),
            preview: material.preview()?,
            format: match material {
                Material::Text(_) => "UTF-8原文本（不因扩展名转换）",
                Material::Table(_) => "列结构JSON（headers/rows，保留空表、列序和值类型）",
            },
        })
    }
    pub fn save(self, cancel: &AtomicBool) -> Result<(PathBuf, usize)> {
        let parent = self.path.parent().context("目标目录无效")?;
        ensure!(
            parent.canonicalize()? == parent,
            "目标目录位置已改变，请重新审核"
        );
        let path = crate::local_files::save_new_moved(&self.path, &self.bytes, cancel)?;
        Ok((path, self.bytes.len()))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn files_roundtrip_complete_typed_material_and_definition_without_execution() {
        let root = std::env::temp_dir().join(format!("zi-flow-files-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let cancel = AtomicBool::new(false);
        let original = "a\0中文🦀\r\n";
        let saved = Output::prepare(&Material::Text(original.into()), root.join("text.txt"))
            .unwrap()
            .save(&cancel)
            .unwrap();
        assert_eq!(saved.1, original.len());
        assert_eq!(input(&saved.0, &cancel).unwrap(), original);
        let data = crate::workbench::Dataset::from_parts(
            vec!["z".into(), "a".into()],
            vec![vec![
                serde_json::json!(9007199254740993i64),
                serde_json::json!([true, null, "001"]),
            ]],
        )
        .unwrap();
        let saved = Output::prepare(&Material::Table(data.clone()), root.join("table.json"))
            .unwrap()
            .save(&cancel)
            .unwrap();
        let loaded: crate::workbench::Dataset =
            serde_json::from_str(&input(&saved.0, &cancel).unwrap()).unwrap();
        assert_eq!(loaded, data);
        let empty =
            crate::workbench::Dataset::from_parts(vec!["z".into(), "a".into()], vec![]).unwrap();
        let saved = Output::prepare(&Material::Table(empty.clone()), root.join("empty.json"))
            .unwrap()
            .save(&cancel)
            .unwrap();
        assert_eq!(
            serde_json::from_str::<crate::workbench::Dataset>(&input(&saved.0, &cancel).unwrap())
                .unwrap(),
            empty
        );
        let definition = Definition {
            version: 2,
            steps: vec![super::super::Step {
                action: "table.parse_schema".into(),
                version: 1,
            }],
        };
        let path = save_recipe(&definition, &root.join("recipe.json"), &cancel).unwrap();
        assert_eq!(recipe(&path, &cancel).unwrap(), definition);
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 4);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn frozen_output_conflict_cancel_and_invalid_files_preserve_existing() {
        let root = std::env::temp_dir().join(format!("zi-flow-files-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("result.txt");
        let review = Output::prepare(&Material::Text("frozen".into()), path.clone()).unwrap();
        std::fs::write(&path, b"competitor").unwrap();
        assert!(review.save(&AtomicBool::new(false)).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"competitor");
        assert!(Output::prepare(&Material::Text("another".into()), path.clone()).is_err());
        let cancelled = root.join("cancelled.txt");
        assert!(
            Output::prepare(&Material::Text("frozen".into()), cancelled.clone())
                .unwrap()
                .save(&AtomicBool::new(true))
                .is_err()
        );
        assert!(!cancelled.exists());
        let bad = root.join("bad.json");
        std::fs::write(&bad, b"{\"version\":2,\"steps\":[],\"command\":\"run\"}").unwrap();
        assert!(recipe(&bad, &AtomicBool::new(false)).is_err());
        std::fs::write(&bad, [0xff, 0xfe]).unwrap();
        assert!(input(&bad, &AtomicBool::new(false)).is_err());
        std::fs::write(&bad, vec![b'a'; LIMIT + 1]).unwrap();
        assert!(input(&bad, &AtomicBool::new(false)).is_err());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }
}
