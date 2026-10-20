use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
mod storage;
mod workflows;
pub use workflows::SavedWorkflow;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    #[serde(skip)]
    baseline: Option<serde_json::Value>,
    #[serde(skip)]
    pending_recent: Vec<String>,
    #[serde(skip)]
    pending_workflow_loads: Vec<SavedWorkflow>,
    #[serde(skip)]
    removed_workflow_loads: Vec<PathBuf>,
    pub updates: crate::updates::Policy,
    pub command_bindings: crate::commands::Bindings,
    pub light: bool,
    pub hotkey: crate::hotkey::Setting,
    pub favorites: Vec<String>,
    pub recent: Vec<String>,
    pub usage: std::collections::BTreeMap<String, u32>,
    pub tool_collections: Vec<ToolCollection>,
    pub workflow_library_folder: Option<PathBuf>,
    pub workflow_favorites: Vec<SavedWorkflow>,
    pub workflow_recent: Vec<SavedWorkflow>,
    pub recorder_auto_minimize: bool,
    pub recorder_auto_stop_minutes: u16,
    pub recorder_quality: crate::recorder::RecordingQuality,
    pub task_history_local_enabled: bool,
}

/// A named, user-curated group of tool IDs. IDs remain stable across catalog updates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCollection {
    pub id: String,
    pub name: String,
    pub tool_ids: Vec<String>,
}

const TOOL_COLLECTIONS_EXPORT_SCHEMA: &str = "zi-devtools/tool-collections";
pub const TOOL_COLLECTIONS_EXPORT_MAX_BYTES: usize = 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct PortableToolCollections {
    schema: String,
    schema_version: u32,
    collections: Vec<PortableToolCollection>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct PortableToolCollection {
    name: String,
    tool_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCollectionImportItem {
    pub source_name: String,
    pub collection: ToolCollection,
    pub renamed_to_avoid_conflict: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCollectionImportPreview {
    pub source_name: String,
    pub items: Vec<ToolCollectionImportItem>,
    baseline: Vec<ToolCollection>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            baseline: None,
            pending_recent: Vec::new(),
            pending_workflow_loads: Vec::new(),
            removed_workflow_loads: Vec::new(),
            updates: Default::default(),
            command_bindings: Default::default(),
            light: false,
            hotkey: Default::default(),
            favorites: Vec::new(),
            recent: Vec::new(),
            usage: Default::default(),
            tool_collections: Vec::new(),
            workflow_library_folder: None,
            workflow_favorites: Vec::new(),
            workflow_recent: Vec::new(),
            recorder_auto_minimize: true,
            recorder_auto_stop_minutes: 0,
            recorder_quality: crate::recorder::RecordingQuality::default(),
            task_history_local_enabled: false,
        }
    }
}
pub fn path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".zi-devtools/ui-preferences.json")
}
impl Preferences {
    /// Export only portable collection names and ordered tool IDs. Preferences,
    /// tool inputs, and local collection UUIDs are intentionally excluded.
    pub fn export_tool_collections(&self) -> Result<String> {
        let portable = PortableToolCollections {
            schema: TOOL_COLLECTIONS_EXPORT_SCHEMA.into(),
            schema_version: 1,
            collections: self
                .tool_collections
                .iter()
                .map(|collection| PortableToolCollection {
                    name: collection.name.clone(),
                    tool_ids: collection.tool_ids.clone(),
                })
                .collect(),
        };
        let json = serde_json::to_string_pretty(&portable)?;
        anyhow::ensure!(
            json.len() <= TOOL_COLLECTIONS_EXPORT_MAX_BYTES,
            "工具集合导出超过1 MiB"
        );
        Ok(json)
    }

    /// Validate a portable file without mutating preferences or writing data.
    pub fn preview_tool_collections_import(
        &self,
        source_name: impl Into<String>,
        bytes: &[u8],
    ) -> Result<ToolCollectionImportPreview> {
        anyhow::ensure!(
            !bytes.is_empty() && bytes.len() <= TOOL_COLLECTIONS_EXPORT_MAX_BYTES,
            "导入文件必须为1 MiB以内的非空JSON"
        );
        let input: PortableToolCollections =
            serde_json::from_slice(bytes).context("工具集合文件格式无效或包含未知字段")?;
        anyhow::ensure!(
            input.schema == TOOL_COLLECTIONS_EXPORT_SCHEMA && input.schema_version == 1,
            "不支持的工具集合文件版本"
        );
        anyhow::ensure!(
            !input.collections.is_empty() && input.collections.len() <= 128,
            "导入文件必须包含1到128个工具集合"
        );
        anyhow::ensure!(
            self.tool_collections.len() + input.collections.len() <= 128,
            "导入后工具集合将超过128个"
        );

        let mut incoming_names = std::collections::HashSet::new();
        let mut reserved_names = self
            .tool_collections
            .iter()
            .map(|item| item.name.to_lowercase())
            .collect::<std::collections::HashSet<_>>();
        let mut items = Vec::with_capacity(input.collections.len());
        for portable in input.collections {
            let source_name = valid_collection_name(&portable.name)?;
            anyhow::ensure!(
                incoming_names.insert(source_name.to_lowercase()),
                "导入文件中存在重复的集合名称：{source_name}"
            );
            anyhow::ensure!(
                portable.tool_ids.len() <= 512,
                "集合“{source_name}”包含超过512个工具入口"
            );
            let mut seen_tools = std::collections::HashSet::new();
            for tool_id in &portable.tool_ids {
                anyhow::ensure!(
                    !tool_id.is_empty() && tool_id.len() <= 512,
                    "集合“{source_name}”包含无效工具 ID"
                );
                anyhow::ensure!(
                    seen_tools.insert(tool_id),
                    "集合“{source_name}”包含重复工具 ID：{tool_id}"
                );
            }

            let mut name = source_name.clone();
            let mut renamed_to_avoid_conflict = false;
            if reserved_names.contains(&name.to_lowercase()) {
                renamed_to_avoid_conflict = true;
                let mut suffix_number = 1usize;
                loop {
                    let suffix = format!("（导入 {suffix_number}）");
                    let prefix_len = 40usize.saturating_sub(suffix.chars().count());
                    let prefix = source_name.chars().take(prefix_len).collect::<String>();
                    name = format!("{prefix}{suffix}");
                    if reserved_names.insert(name.to_lowercase()) {
                        break;
                    }
                    suffix_number += 1;
                    anyhow::ensure!(suffix_number <= 1000, "无法为冲突集合生成唯一名称");
                }
            } else {
                reserved_names.insert(name.to_lowercase());
            }
            items.push(ToolCollectionImportItem {
                source_name,
                collection: ToolCollection {
                    id: uuid::Uuid::new_v4().to_string(),
                    name,
                    tool_ids: portable.tool_ids,
                },
                renamed_to_avoid_conflict,
            });
        }
        Ok(ToolCollectionImportPreview {
            source_name: source_name.into(),
            items,
            baseline: self.tool_collections.clone(),
        })
    }

    /// Commit a preview only while the collection snapshot used to prepare it
    /// is still current. Any persistence failure leaves the live state intact.
    pub fn import_tool_collections(
        &mut self,
        path: &Path,
        preview: &ToolCollectionImportPreview,
    ) -> Result<()> {
        anyhow::ensure!(
            self.tool_collections == preview.baseline,
            "工具集合在预览后已变化，请重新预览导入文件"
        );
        anyhow::ensure!(
            self.tool_collections.len() + preview.items.len() <= 128,
            "导入后工具集合将超过128个"
        );
        let mut next = self.clone();
        let mut names = next
            .tool_collections
            .iter()
            .map(|item| item.name.to_lowercase())
            .collect::<std::collections::HashSet<_>>();
        for item in &preview.items {
            anyhow::ensure!(
                names.insert(item.collection.name.to_lowercase()),
                "集合名称冲突，请重新预览导入文件"
            );
            next.tool_collections.push(item.collection.clone());
        }
        let imported_ids = preview
            .items
            .iter()
            .map(|item| item.collection.id.clone())
            .collect::<Vec<_>>();
        storage::save_tool_collection_import(&mut next, path, &preview.baseline)
            .context("无法提交工具集合导入")?;
        anyhow::ensure!(
            imported_ids.iter().all(|id| next
                .tool_collections
                .iter()
                .any(|collection| &collection.id == id)),
            "其他窗口同时修改了工具集合，请刷新后确认导入结果"
        );
        *self = next;
        Ok(())
    }

    pub fn create_tool_collection(&mut self, path: &Path, name: &str) -> Result<String> {
        let name = valid_collection_name(name)?;
        anyhow::ensure!(self.tool_collections.len() < 128, "最多创建128个工具集合");
        ensure_unique_collection_name(&self.tool_collections, &name, None)?;
        let id = uuid::Uuid::new_v4().to_string();
        let mut next = self.clone();
        next.tool_collections.push(ToolCollection {
            id: id.clone(),
            name,
            tool_ids: Vec::new(),
        });
        next.save(path)?;
        anyhow::ensure!(
            next.tool_collections
                .iter()
                .any(|collection| collection.id == id),
            "其他窗口已创建同名工具集合，请刷新后重试"
        );
        *self = next;
        Ok(id)
    }

    pub fn rename_tool_collection(&mut self, path: &Path, id: &str, name: &str) -> Result<()> {
        let name = valid_collection_name(name)?;
        ensure_unique_collection_name(&self.tool_collections, &name, Some(id))?;
        let mut next = self.clone();
        let collection = next
            .tool_collections
            .iter_mut()
            .find(|collection| collection.id == id)
            .ok_or_else(|| anyhow::anyhow!("工具集合已不存在"))?;
        collection.name = name.clone();
        next.save(path)?;
        anyhow::ensure!(
            next.tool_collections
                .iter()
                .any(|collection| collection.id == id && collection.name == name),
            "集合名称与其他窗口的修改冲突，请刷新后重试"
        );
        *self = next;
        Ok(())
    }

    pub fn delete_tool_collection(&mut self, path: &Path, id: &str) -> Result<()> {
        anyhow::ensure!(
            self.tool_collections
                .iter()
                .any(|collection| collection.id == id),
            "工具集合已不存在"
        );
        let mut next = self.clone();
        next.tool_collections
            .retain(|collection| collection.id != id);
        next.save(path)?;
        *self = next;
        Ok(())
    }

    pub fn toggle_tool_in_collection(
        &mut self,
        path: &Path,
        collection_id: &str,
        tool_id: &str,
    ) -> Result<bool> {
        anyhow::ensure!(!tool_id.is_empty() && tool_id.len() <= 512, "工具 ID 无效");
        let mut next = self.clone();
        let collection = next
            .tool_collections
            .iter_mut()
            .find(|collection| collection.id == collection_id)
            .ok_or_else(|| anyhow::anyhow!("工具集合已不存在"))?;
        let added = if let Some(index) = collection.tool_ids.iter().position(|id| id == tool_id) {
            collection.tool_ids.remove(index);
            false
        } else {
            anyhow::ensure!(collection.tool_ids.len() < 512, "单个工具集合最多包含512项");
            collection.tool_ids.push(tool_id.to_owned());
            true
        };
        next.save(path)?;
        let persisted = next
            .tool_collections
            .iter()
            .find(|collection| collection.id == collection_id)
            .is_some_and(|collection| collection.tool_ids.iter().any(|id| id == tool_id));
        anyhow::ensure!(persisted == added, "集合已被其他窗口更新，请刷新后重试");
        *self = next;
        Ok(added)
    }

    pub fn move_tool_in_collection(
        &mut self,
        path: &Path,
        collection_id: &str,
        tool_id: &str,
        target_tool_id: &str,
        after: bool,
    ) -> Result<()> {
        anyhow::ensure!(
            !tool_id.is_empty()
                && tool_id.len() <= 512
                && !target_tool_id.is_empty()
                && target_tool_id.len() <= 512,
            "工具 ID 无效"
        );
        if tool_id == target_tool_id {
            return Ok(());
        }
        let mut next = self.clone();
        let collection = next
            .tool_collections
            .iter_mut()
            .find(|collection| collection.id == collection_id)
            .ok_or_else(|| anyhow::anyhow!("工具集合已不存在"))?;
        let from = collection
            .tool_ids
            .iter()
            .position(|id| id == tool_id)
            .ok_or_else(|| anyhow::anyhow!("要移动的工具已不在集合中"))?;
        let moved = collection.tool_ids.remove(from);
        let target = collection
            .tool_ids
            .iter()
            .position(|id| id == target_tool_id)
            .ok_or_else(|| anyhow::anyhow!("排序目标已不在集合中"))?;
        collection
            .tool_ids
            .insert(target + usize::from(after), moved);
        let requested_order = collection.tool_ids.clone();
        next.save(path)?;
        let persisted = next
            .tool_collections
            .iter()
            .find(|collection| collection.id == collection_id)
            .ok_or_else(|| anyhow::anyhow!("工具集合已被其他窗口删除"))?;
        let expected_survivors = requested_order
            .iter()
            .filter(|id| persisted.tool_ids.contains(id))
            .collect::<Vec<_>>();
        let persisted_requested = persisted
            .tool_ids
            .iter()
            .filter(|id| requested_order.contains(id))
            .collect::<Vec<_>>();
        anyhow::ensure!(
            expected_survivors == persisted_requested,
            "集合顺序已被其他窗口修改，请刷新后重试"
        );
        *self = next;
        Ok(())
    }

    pub fn save_workflow_folder(&mut self, path: &Path, folder: Option<PathBuf>) -> Result<()> {
        anyhow::ensure!(
            folder
                .as_ref()
                .is_none_or(|folder| valid_workflow_folder(folder)),
            "流程目录必须为有效的绝对路径，最多32768字节"
        );
        let mut next = self.clone();
        next.workflow_library_folder = folder;
        next.save(path)?;
        *self = next;
        Ok(())
    }
    /// Save before activating, preserving the live state on failure.
    pub fn save_command_bindings(
        &mut self,
        path: &Path,
        bindings: crate::commands::Bindings,
    ) -> Result<()> {
        let mut next = self.clone();
        next.command_bindings = bindings;
        next.save(path)?;
        *self = next;
        Ok(())
    }
    pub fn visit(&mut self, id: &str) {
        self.pending_recent.retain(|item| item != id);
        self.pending_recent.insert(0, id.to_owned());
        self.pending_recent.truncate(20);
        self.recent.retain(|s| s != id);
        self.recent.insert(0, id.into());
        self.recent.truncate(20);
        if self.usage.len() < 4096 || self.usage.contains_key(id) {
            let count = self.usage.entry(id.into()).or_default();
            *count = count.saturating_add(1);
        }
    }
    pub fn load(path: &Path) -> Self {
        let mut bytes = Vec::new();
        let read =
            fs::File::open(path).and_then(|file| file.take(8_388_609).read_to_end(&mut bytes));
        let mut value: Self = if read.is_ok() && bytes.len() <= 8_388_608 {
            serde_json::from_slice(&bytes).unwrap_or_default()
        } else {
            Self::default()
        };
        value.normalize();
        value.baseline = serde_json::to_value(&value).ok();
        value
    }
    fn normalize(&mut self) {
        workflows::normalize(&mut self.workflow_favorites);
        workflows::normalize(&mut self.workflow_recent);
        self.workflow_recent.truncate(20);
        if self
            .workflow_library_folder
            .as_ref()
            .is_some_and(|folder| !valid_workflow_folder(folder))
        {
            self.workflow_library_folder = None;
        }
        let normalize = |ids: &mut Vec<String>, limit: usize| {
            let mut seen = std::collections::HashSet::new();
            ids.retain(|id| !id.is_empty() && id.len() <= 512 && seen.insert(id.clone()));
            ids.truncate(limit);
        };
        normalize(&mut self.favorites, 4096);
        normalize(&mut self.recent, 20);
        self.usage = std::mem::take(&mut self.usage)
            .into_iter()
            .filter(|(id, count)| !id.is_empty() && id.len() <= 512 && *count > 0)
            .take(4096)
            .collect();
        let mut collection_ids = std::collections::HashSet::new();
        let mut collection_names = std::collections::HashSet::new();
        self.tool_collections.retain_mut(|collection| {
            let Ok(name) = valid_collection_name(&collection.name) else {
                return false;
            };
            let Ok(id) = uuid::Uuid::parse_str(&collection.id) else {
                return false;
            };
            let id = id.to_string();
            let folded = name.to_lowercase();
            if !collection_ids.insert(id.clone()) || !collection_names.insert(folded) {
                return false;
            }
            collection.id = id;
            collection.name = name;
            let mut seen = std::collections::HashSet::new();
            collection
                .tool_ids
                .retain(|tool| !tool.is_empty() && tool.len() <= 512 && seen.insert(tool.clone()));
            collection.tool_ids.truncate(512);
            true
        });
        self.tool_collections.truncate(128);
        if ![0, 1, 5, 15, 30, 60].contains(&self.recorder_auto_stop_minutes) {
            self.recorder_auto_stop_minutes = 0;
        }
    }
    pub fn save(&mut self, path: &Path) -> Result<()> {
        storage::save(self, path).context("无法保存界面偏好")
    }
    pub fn refresh_discovery(&mut self, path: &Path) -> Result<bool> {
        storage::refresh_discovery(self, path)
    }
    pub fn toggle(&mut self, id: &str) {
        if self.favorites.iter().any(|s| s == id) {
            self.favorites.retain(|s| s != id);
        } else {
            self.favorites.push(id.to_owned());
        }
    }
}

fn valid_collection_name(name: &str) -> Result<String> {
    let name = name.trim();
    anyhow::ensure!(!name.is_empty(), "请输入工具集合名称");
    anyhow::ensure!(name.chars().count() <= 40, "工具集合名称最多40个字符");
    anyhow::ensure!(
        !name.chars().any(char::is_control),
        "工具集合名称不能含控制字符"
    );
    Ok(name.to_owned())
}

fn ensure_unique_collection_name(
    collections: &[ToolCollection],
    name: &str,
    except_id: Option<&str>,
) -> Result<()> {
    anyhow::ensure!(
        !collections.iter().any(|collection| {
            Some(collection.id.as_str()) != except_id && collection.name.eq_ignore_ascii_case(name)
        }),
        "已有同名工具集合"
    );
    Ok(())
}

fn valid_workflow_folder(path: &Path) -> bool {
    path.is_absolute() && path.as_os_str().len() <= 32768 && !path.to_string_lossy().contains('\0')
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn task_history_storage_is_opt_in_and_legacy_preferences_default_off() {
        assert!(!Preferences::default().task_history_local_enabled);
        let legacy: Preferences =
            serde_json::from_value(serde_json::json!({ "light": true })).unwrap();
        assert!(!legacy.task_history_local_enabled);
        assert!(legacy.light);
    }

    #[test]
    fn workflow_folder_save_roundtrip_failure_and_forget_preserve_files() {
        let dir = std::env::temp_dir().join(format!("zi-flow-prefs-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let folder = dir.join("recipes");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("keep.json"), b"unchanged").unwrap();
        let path = dir.join("prefs.json");
        let mut prefs = Preferences::load(&path);
        prefs.visit("json");
        prefs
            .save_workflow_folder(&path, Some(folder.clone()))
            .unwrap();
        assert_eq!(
            Preferences::load(&path).workflow_library_folder,
            Some(folder.clone())
        );
        assert!(prefs.save_workflow_folder(&dir, None).is_err());
        assert_eq!(prefs.workflow_library_folder, Some(folder.clone()));
        assert!(
            prefs
                .save_workflow_folder(&path, Some(PathBuf::from("relative")))
                .is_err()
        );
        // A stale second window changing an unrelated preference preserves the folder.
        let mut stale = Preferences {
            light: true,
            ..Default::default()
        };
        stale.save(&path).unwrap();
        assert_eq!(
            Preferences::load(&path).workflow_library_folder,
            Some(folder.clone())
        );
        prefs.save_workflow_folder(&path, None).unwrap();
        let restored = Preferences::load(&path);
        assert!(restored.workflow_library_folder.is_none());
        assert!(restored.light);
        assert_eq!(fs::read(folder.join("keep.json")).unwrap(), b"unchanged");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn binding_save_is_transactional_and_preserves_other_preferences() {
        let dir = std::env::temp_dir().join(format!("zi-bindings-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let mut prefs = Preferences::default();
        prefs.visit("json");
        let mut bindings = crate::commands::Bindings::new();
        bindings.insert("open:json".into(), "Q J".into());
        bindings.insert("open:disabled-plugin".into(), String::new());
        assert!(prefs.save_command_bindings(&dir, bindings.clone()).is_err());
        assert!(prefs.command_bindings.is_empty());
        assert_eq!(prefs.recent, ["json"]);
        let path = dir.join("preferences.json");
        prefs
            .save_command_bindings(&path, bindings.clone())
            .unwrap();
        assert_eq!(Preferences::load(&path).command_bindings, bindings);
        assert_eq!(Preferences::load(&path).recent, ["json"]);
        assert!(
            !fs::read_dir(&dir).unwrap().any(|f| f
                .unwrap()
                .path()
                .extension()
                .is_some_and(|e| e == "tmp"))
        );
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn saves_replace_existing_preferences_and_load_deduplicates() {
        let dir = std::env::temp_dir().join(format!("zi-preferences-{}", uuid::Uuid::new_v4()));
        let path = dir.join("preferences.json");
        let mut prefs = Preferences::default();
        prefs.toggle("plugin:disabled/tool");
        prefs.save(&path).unwrap();
        prefs.visit("json");
        prefs.recorder_auto_minimize = false;
        prefs.recorder_auto_stop_minutes = 15;
        prefs.recorder_quality = crate::recorder::RecordingQuality::Detailed;
        prefs.favorites.push("plugin:disabled/tool".into());
        prefs.save(&path).unwrap();
        let restored = Preferences::load(&path);
        assert_eq!(restored.favorites, ["plugin:disabled/tool"]);
        assert_eq!(restored.recent, ["json"]);
        assert_eq!(restored.usage["json"], 1);
        assert!(!restored.recorder_auto_minimize);
        assert_eq!(restored.recorder_auto_stop_minutes, 15);
        assert_eq!(
            restored.recorder_quality,
            crate::recorder::RecordingQuality::Detailed
        );
        assert!(
            !fs::read_dir(&dir).unwrap().any(|f| f
                .unwrap()
                .path()
                .extension()
                .is_some_and(|e| e == "tmp"))
        );
        fs::write(&path, "invalid").unwrap();
        assert!(Preferences::load(&path).recent.is_empty());
        fs::write(&path, r#"{"recorder_auto_stop_minutes":999}"#).unwrap();
        assert_eq!(Preferences::load(&path).recorder_auto_stop_minutes, 0);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn old_preferences_migrate_and_usage_stays_bounded() {
        let mut p: Preferences =
            serde_json::from_str(r#"{"light":true,"favorites":["json"]}"#).unwrap();
        assert!(p.command_bindings.is_empty());
        assert!(p.workflow_library_folder.is_none());
        assert!(p.recent.is_empty());
        assert!(p.recorder_auto_minimize);
        assert_eq!(p.recorder_auto_stop_minutes, 0);
        assert_eq!(
            p.recorder_quality,
            crate::recorder::RecordingQuality::Balanced
        );
        for i in 0..30 {
            p.visit(&format!("tool-{i}"));
        }
        p.visit("tool-1");
        assert_eq!(p.recent.len(), 20);
        assert_eq!(p.recent[0], "tool-1");
        assert_eq!(p.usage["tool-1"], 2);
        assert_eq!(p.favorites, vec!["json"]);
    }

    #[test]
    fn tool_collection_crud_is_transactional_and_validated() {
        let dir = std::env::temp_dir().join(format!("zi-collections-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("preferences.json");
        let mut prefs = Preferences::load(&path);
        let id = prefs
            .create_tool_collection(&path, "  Java 排查  ")
            .unwrap();
        assert!(
            prefs
                .toggle_tool_in_collection(&path, &id, "java-diagnostics")
                .unwrap()
        );
        assert!(
            !prefs
                .toggle_tool_in_collection(&path, &id, "java-diagnostics")
                .unwrap()
        );
        prefs
            .toggle_tool_in_collection(&path, &id, "java-diagnostics")
            .unwrap();
        prefs
            .rename_tool_collection(&path, &id, "JVM 诊断")
            .unwrap();
        assert!(prefs.create_tool_collection(&path, "jvm 诊断").is_err());
        let restored = Preferences::load(&path);
        assert_eq!(restored.tool_collections.len(), 1);
        assert_eq!(restored.tool_collections[0].name, "JVM 诊断");
        assert_eq!(restored.tool_collections[0].tool_ids, ["java-diagnostics"]);
        prefs.delete_tool_collection(&path, &id).unwrap();
        assert!(Preferences::load(&path).tool_collections.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn tool_collection_changes_do_not_commit_when_preferences_cannot_be_saved() {
        let dir =
            std::env::temp_dir().join(format!("zi-collections-fail-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let mut prefs = Preferences::default();
        let error = prefs.create_tool_collection(&dir, "Java").unwrap_err();
        assert!(error.to_string().contains("无法保存界面偏好"));
        assert!(prefs.tool_collections.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn tool_collection_reorder_persists_before_after_and_keeps_failure_state() {
        let dir =
            std::env::temp_dir().join(format!("zi-collections-order-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("preferences.json");
        let mut prefs = Preferences::load(&path);
        let id = prefs.create_tool_collection(&path, "日常").unwrap();
        for tool in ["json", "uuid", "base64"] {
            prefs.toggle_tool_in_collection(&path, &id, tool).unwrap();
        }
        prefs
            .move_tool_in_collection(&path, &id, "base64", "json", false)
            .unwrap();
        prefs
            .move_tool_in_collection(&path, &id, "json", "uuid", true)
            .unwrap();
        assert_eq!(
            Preferences::load(&path).tool_collections[0].tool_ids,
            ["base64", "uuid", "json"]
        );
        assert!(
            prefs
                .move_tool_in_collection(&path, &id, "missing", "uuid", false)
                .is_err()
        );
        assert_eq!(
            prefs.tool_collections[0].tool_ids,
            ["base64", "uuid", "json"]
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn portable_tool_collections_export_preview_and_import_preserve_order_and_unknown_ids() {
        let dir =
            std::env::temp_dir().join(format!("zi-collection-portable-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let source_path = dir.join("source.json");
        let mut source = Preferences::load(&source_path);
        let id = source
            .create_tool_collection(&source_path, "Java 排查")
            .unwrap();
        for tool in ["java-diagnostics", "plugin:offline/tool", "django-check"] {
            source
                .toggle_tool_in_collection(&source_path, &id, tool)
                .unwrap();
        }
        source.favorites.push("private-favorite".into());
        let exported = source.export_tool_collections().unwrap();
        assert!(!exported.contains(&id));
        assert!(!exported.contains("private-favorite"));
        assert!(exported.contains("plugin:offline/tool"));

        let destination_path = dir.join("destination.json");
        let mut destination = Preferences::load(&destination_path);
        destination
            .create_tool_collection(&destination_path, "Java 排查")
            .unwrap();
        let preview = destination
            .preview_tool_collections_import("backup.json", exported.as_bytes())
            .unwrap();
        assert_eq!(preview.items.len(), 1);
        assert_eq!(preview.items[0].source_name, "Java 排查");
        assert_eq!(preview.items[0].collection.name, "Java 排查（导入 1）");
        assert_ne!(preview.items[0].collection.id, id);
        assert!(preview.items[0].renamed_to_avoid_conflict);
        assert_eq!(
            preview.items[0].collection.tool_ids,
            ["java-diagnostics", "plugin:offline/tool", "django-check"]
        );
        destination
            .import_tool_collections(&destination_path, &preview)
            .unwrap();
        let restored = Preferences::load(&destination_path);
        assert_eq!(restored.tool_collections.len(), 2);
        assert_eq!(restored.tool_collections[1].name, "Java 排查（导入 1）");
        assert_eq!(
            restored.tool_collections[1].tool_ids,
            preview.items[0].collection.tool_ids
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn portable_tool_collection_import_rejects_malformed_duplicate_and_oversized_files() {
        let prefs = Preferences::default();
        let duplicate = serde_json::json!({
            "schema": TOOL_COLLECTIONS_EXPORT_SCHEMA,
            "schemaVersion": 1,
            "collections": [{"name":"X", "toolIds":["json", "json"]}]
        });
        assert!(
            prefs
                .preview_tool_collections_import(
                    "duplicate.json",
                    &serde_json::to_vec(&duplicate).unwrap()
                )
                .is_err()
        );
        let duplicate_names = serde_json::json!({
            "schema": TOOL_COLLECTIONS_EXPORT_SCHEMA,
            "schemaVersion": 1,
            "collections": [
                {"name":"Java", "toolIds":["java-diagnostics"]},
                {"name":"java", "toolIds":["django-check"]}
            ]
        });
        assert!(
            prefs
                .preview_tool_collections_import(
                    "duplicate-names.json",
                    &serde_json::to_vec(&duplicate_names).unwrap()
                )
                .is_err()
        );
        let unknown = br#"{"schema":"zi-devtools/tool-collections","schemaVersion":1,"collections":[],"favorites":["json"]}"#;
        assert!(
            prefs
                .preview_tool_collections_import("unknown.json", unknown)
                .is_err()
        );
        assert!(
            prefs
                .preview_tool_collections_import(
                    "large.json",
                    &vec![b' '; TOOL_COLLECTIONS_EXPORT_MAX_BYTES + 1]
                )
                .is_err()
        );
    }

    #[test]
    fn portable_tool_collection_import_rejects_stale_preview_and_failed_save_transactionally() {
        let dir =
            std::env::temp_dir().join(format!("zi-collection-stale-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let file = serde_json::to_vec(&PortableToolCollections {
            schema: TOOL_COLLECTIONS_EXPORT_SCHEMA.into(),
            schema_version: 1,
            collections: vec![PortableToolCollection {
                name: "Imported".into(),
                tool_ids: vec!["json".into()],
            }],
        })
        .unwrap();
        let path = dir.join("prefs.json");
        let mut prefs = Preferences::load(&path);
        let preview = prefs
            .preview_tool_collections_import("in.json", &file)
            .unwrap();
        let mut other_window = Preferences::load(&path);
        other_window
            .create_tool_collection(&path, "Changed after preview")
            .unwrap();
        assert!(prefs.import_tool_collections(&path, &preview).is_err());
        assert_eq!(Preferences::load(&path).tool_collections.len(), 1);

        let mut fresh = Preferences::load(&dir);
        let failing_preview = fresh
            .preview_tool_collections_import("in.json", &file)
            .unwrap();
        assert!(
            fresh
                .import_tool_collections(&dir, &failing_preview)
                .is_err()
        );
        assert!(fresh.tool_collections.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn concurrent_same_name_collection_creation_reports_the_losing_window() {
        let dir =
            std::env::temp_dir().join(format!("zi-collections-race-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("preferences.json");
        let mut a = Preferences::load(&path);
        let mut b = Preferences::load(&path);
        a.create_tool_collection(&path, "Java").unwrap();
        assert!(b.create_tool_collection(&path, "java").is_err());
        assert!(b.tool_collections.is_empty());
        let stored = Preferences::load(&path);
        assert_eq!(stored.tool_collections.len(), 1);
        assert_eq!(stored.tool_collections[0].name, "Java");
        fs::remove_dir_all(dir).unwrap();
    }
}
