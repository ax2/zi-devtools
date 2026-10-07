//! Explicit workflow bookmarks store metadata only, never definitions or tables.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedWorkflow {
    pub path: PathBuf,
    pub name: String,
    pub steps: usize,
}
impl SavedWorkflow {
    pub fn valid(&self) -> bool {
        valid_workflow_folder(&self.path)
            && self.path.file_name().is_some()
            && self
                .path
                .components()
                .all(|part| !matches!(part, std::path::Component::ParentDir))
            && !self.name.trim().is_empty()
            && self.name.chars().count() <= 120
            && (1..=32).contains(&self.steps)
    }
    pub fn matches(&self, query: &str) -> bool {
        self.matches_kind(query, "收藏 favorite bookmark")
    }
    pub fn matches_recent(&self, query: &str) -> bool {
        self.matches_kind(query, "最近 载入 recent loaded")
    }
    fn matches_kind(&self, query: &str, kind: &str) -> bool {
        let text = format!(
            "{kind} 流程 workflow recipe {} {}",
            self.name,
            self.path.file_name().unwrap_or_default().to_string_lossy()
        )
        .to_lowercase();
        query
            .split_whitespace()
            .all(|word| text.contains(&word.to_lowercase()))
    }
}
pub(super) fn normalize(entries: &mut Vec<SavedWorkflow>) {
    let mut seen = std::collections::HashSet::new();
    entries.retain(|entry| entry.valid() && seen.insert(entry.path.clone()));
    entries.truncate(100);
}
impl Preferences {
    pub fn workflow_loaded(&mut self, entry: SavedWorkflow) {
        if !entry.valid() {
            return;
        }
        self.removed_workflow_loads
            .retain(|path| path != &entry.path);
        self.pending_workflow_loads
            .retain(|item| item.path != entry.path);
        self.pending_workflow_loads.insert(0, entry.clone());
        self.pending_workflow_loads.truncate(20);
        self.workflow_recent.retain(|item| item.path != entry.path);
        self.workflow_recent.insert(0, entry);
        self.workflow_recent.truncate(20);
    }
    pub fn workflow_history_pending(&self) -> bool {
        !self.pending_workflow_loads.is_empty() || !self.removed_workflow_loads.is_empty()
    }
    pub fn forget_workflow_load(&mut self, config: &Path, entry: &SavedWorkflow) -> Result<()> {
        let mut next = self.clone();
        next.workflow_recent.retain(|item| item.path != entry.path);
        next.pending_workflow_loads
            .retain(|item| item.path != entry.path);
        if !next.removed_workflow_loads.contains(&entry.path) {
            next.removed_workflow_loads.push(entry.path.clone());
        }
        next.save(config)?;
        *self = next;
        Ok(())
    }
    /// Persist before committing live state. File contents are never touched.
    pub fn toggle_workflow(&mut self, config: &Path, entry: SavedWorkflow) -> Result<bool> {
        anyhow::ensure!(entry.valid(), "流程收藏信息无效");
        let mut next = self.clone();
        let existed = next
            .workflow_favorites
            .iter()
            .any(|item| item.path == entry.path);
        if existed {
            next.workflow_favorites
                .retain(|item| item.path != entry.path);
        } else {
            anyhow::ensure!(
                next.workflow_favorites.len() < 100,
                "已收藏100条流程，请先移除不再需要的收藏"
            );
            next.workflow_favorites.push(entry);
        }
        next.save(config)?;
        *self = next;
        Ok(!existed)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recent_workflow_merge_removal_retry_bounds_and_current_metadata() {
        let dir = std::env::temp_dir().join(format!("zi-recent-flow-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let config = dir.join("prefs.json");
        let a = SavedWorkflow {
            path: dir.join("a.json"),
            name: "旧摘要".into(),
            steps: 1,
        };
        let b = SavedWorkflow {
            path: dir.join("b.json"),
            name: "另一个".into(),
            steps: 2,
        };
        let mut first = Preferences::load(&config);
        let mut stale = Preferences::load(&config);
        first.workflow_loaded(a.clone());
        assert!(first.save(&dir).is_err());
        assert!(first.workflow_history_pending());
        stale.workflow_loaded(b.clone());
        stale.save(&config).unwrap();
        assert!(!first.refresh_discovery(&config).unwrap());
        first.save(&config).unwrap();
        assert!(!first.workflow_history_pending());
        assert_eq!(
            Preferences::load(&config).workflow_recent,
            [a.clone(), b.clone()]
        );
        let mut update = a.clone();
        update.name = "实际新摘要".into();
        update.steps = 3;
        stale.workflow_loaded(update.clone());
        stale.save(&config).unwrap();
        assert_eq!(
            Preferences::load(&config).workflow_recent,
            [update, b.clone()]
        );
        first.forget_workflow_load(&config, &a).unwrap();
        stale.light = true;
        stale.save(&config).unwrap();
        assert_eq!(Preferences::load(&config).workflow_recent, [b.clone()]);
        assert!(first.forget_workflow_load(&dir, &b).is_err());
        assert_eq!(first.workflow_recent, [b]);
        for i in 0..25 {
            first.workflow_loaded(SavedWorkflow {
                path: dir.join(format!("{i}.json")),
                name: format!("流程{i}"),
                steps: 1,
            });
        }
        fs::write(&config, "damaged").unwrap();
        assert!(first.save(&config).is_err());
        assert_eq!(fs::read_to_string(&config).unwrap(), "damaged");
        assert_eq!(first.workflow_recent.len(), 20);
        fs::write(&config, "{}").unwrap();
        first.save(&config).unwrap();
        let reloaded = Preferences::load(&config);
        assert_eq!(reloaded.workflow_recent.len(), 20);
        assert_eq!(reloaded.workflow_recent[0].name, "流程24");
        assert!(reloaded.workflow_favorites.is_empty());
        let document: serde_json::Value =
            serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        assert!(
            document["workflow_recent"][0]
                .get("steps")
                .unwrap()
                .is_number()
        );
        assert!(document["workflow_recent"][0].get("definition").is_none());
        assert!(!a.path.exists());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn workflow_bookmarks_roundtrip_merge_remove_and_failure_without_reading_files() {
        let dir = std::env::temp_dir().join(format!("zi-bookmarks-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let config = dir.join("preferences.json");
        let a = SavedWorkflow {
            path: dir.join("missing-a.json"),
            name: "每日资料".into(),
            steps: 2,
        };
        let b = SavedWorkflow {
            path: dir.join("missing-b.json"),
            name: "每月统计".into(),
            steps: 3,
        };
        let mut first = Preferences::load(&config);
        let mut stale = Preferences::load(&config);
        assert!(first.toggle_workflow(&config, a.clone()).unwrap());
        assert!(stale.toggle_workflow(&config, b.clone()).unwrap());
        let mut restored = Preferences::load(&config);
        assert_eq!(restored.workflow_favorites, [a.clone(), b.clone()]);
        assert!(first.refresh_discovery(&config).unwrap());
        assert_eq!(first.workflow_favorites, restored.workflow_favorites);
        assert!(!restored.toggle_workflow(&config, a.clone()).unwrap());
        first.light = true;
        first.save(&config).unwrap();
        assert_eq!(Preferences::load(&config).workflow_favorites, [b.clone()]);
        assert!(first.toggle_workflow(&dir, b.clone()).is_err());
        assert_eq!(first.workflow_favorites, [b]);
        assert!(!a.path.exists());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn workflow_bookmarks_normalize_bounds_and_invalid_entries() {
        let root = std::env::temp_dir();
        let mut entries: Vec<_> = (0..120)
            .map(|i| SavedWorkflow {
                path: root.join(format!("{i}.json")),
                name: format!("流程{i}"),
                steps: 1,
            })
            .collect();
        entries.insert(
            0,
            SavedWorkflow {
                path: PathBuf::from("relative.json"),
                name: "无效".into(),
                steps: 1,
            },
        );
        entries.insert(1, entries[2].clone());
        normalize(&mut entries);
        assert_eq!(entries.len(), 100);
        assert!(entries.iter().all(SavedWorkflow::valid));
        assert!(entries.iter().any(|entry| entry.matches("收藏 流程0")));
        let mut prefs = Preferences {
            workflow_favorites: entries,
            ..Default::default()
        };
        let extra = SavedWorkflow {
            path: root.join("extra.json"),
            name: "额外".into(),
            steps: 1,
        };
        assert!(
            prefs
                .toggle_workflow(&root.join("unused-preferences.json"), extra)
                .is_err()
        );
    }
}
