//! Metadata discovery never scans disk. Opening revalidates the current list.
use super::*;
use std::{
    collections::hash_map::DefaultHasher,
    ffi::OsString,
    hash::{Hash, Hasher},
    sync::Arc,
};

#[derive(Clone)]
pub struct WorkflowMatch {
    pub instance_id: String,
    pub instance_name: String,
    pub file_name: OsString,
    pub name: String,
    pub steps: usize,
    pub listing_revision: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workbench::workflow::{Definition, Step};
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("zi-flow-search-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn definition(&self, file: &str, name: &str) {
            let definition = Definition {
                output: None,
                version: 2,
                name: name.into(),
                steps: vec![Step::SelectColumns {
                    columns: vec!["name".into()],
                }],
            };
            std::fs::write(self.0.join(file), serde_json::to_vec(&definition).unwrap()).unwrap();
        }
        fn workspace(&self) -> Workspace {
            Workspace::new(self.0.join("instances.sqlite3"))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            assert_eq!(self.0.parent(), Some(std::env::temp_dir().as_path()));
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    #[test]
    fn mixed_recipe_library_bookmark_reload_routes_actual_content_and_keeps_drafts() {
        let fixture = Fixture::new();
        fixture.definition("table.json", "表格清洗");
        let path = fixture.0.join("编码配方.json");
        let definition = crate::text_flow::Definition {
            version: 1,
            steps: vec![crate::text_flow::Step {
                action: "base64.encode".into(),
                version: 1,
            }],
        };
        std::fs::write(&path, serde_json::to_vec(&definition).unwrap()).unwrap();
        let mut work = fixture.workspace();
        work.input = "source table".into();
        work.text_flow.receive("source tools".into()).unwrap();
        let original = serde_json::to_value(&work.text_flow).unwrap();
        work.workflow.files.list(fixture.0.clone()).unwrap();
        wait(&mut work);
        assert_eq!(work.workflow_matches("流程").total, 2);
        let found = work.workflow_matches("工具 编码");
        assert_eq!(found.total, 1);
        let bookmark = work.workflow_bookmark(&found.entries[0]).unwrap();
        let prefs_path = fixture.0.join("preferences.json");
        let mut prefs = crate::preferences::Preferences::default();
        prefs
            .toggle_workflow(&prefs_path, bookmark.clone())
            .unwrap();
        let prefs = crate::preferences::Preferences::load(&prefs_path);
        assert_eq!(prefs.workflow_favorites[0], bookmark);
        work.open_workflow_bookmark(&prefs.workflow_favorites[0])
            .unwrap();
        wait(&mut work);
        assert_eq!(work.active_tool_id(), "text-flow");
        assert!(work.text_flow.modal_open());
        assert_eq!(serde_json::to_value(&work.text_flow).unwrap(), original);
        assert_eq!(work.input, "source table");
        assert!(work.workflow.files.review.is_none());
        assert_eq!(work.take_workflow_visits(), vec!["text-flow"]);
        let events = work.take_workflow_loads();
        assert_eq!(events, vec![bookmark.clone()]);
        assert!(work.take_workflow_loads().is_empty());
        let mut reopened = fixture.workspace();
        reopened.input = "reopened draft".into();
        reopened.open_workflow_bookmark(&bookmark).unwrap();
        wait(&mut reopened);
        assert!(reopened.text_flow.modal_open());
        assert_eq!(reopened.input, "reopened draft");
        assert!(reopened.workflow.files.listing.is_none());
        let mut changed = fixture.workspace();
        changed.input = "never replace on failure".into();
        std::fs::write(&path, b"bad").unwrap();
        changed.open_workflow_bookmark(&bookmark).unwrap();
        wait(&mut changed);
        assert!(changed.take_workflow_loads().is_empty());
        assert!(changed.take_workflow_visits().is_empty());
        assert!(!changed.text_flow.modal_open());
        assert_eq!(changed.input, "never replace on failure");
        assert_eq!(changed.active_tool_id(), "data");
        fixture.definition("编码配方.json", "文件已改为表格配方");
        changed.open_workflow_bookmark(&bookmark).unwrap();
        wait(&mut changed);
        assert_eq!(
            changed.workflow.files.review.as_ref().unwrap().name,
            "文件已改为表格配方"
        );
        assert!(!changed.text_flow.modal_open());
        assert_eq!(changed.take_workflow_visits(), vec!["pipeline"]);
    }
    #[test]
    fn background_typed_recipe_read_keeps_owner_and_does_not_visit_other_instance() {
        let fixture = Fixture::new();
        let path = fixture.0.join("typed.json");
        std::fs::write(
            &path,
            br#"{"version":1,"steps":[{"action":"sha256.digest","version":1}]}"#,
        )
        .unwrap();
        let mut work = fixture.workspace();
        let owner = work.active_id().to_owned();
        work.workflow.files.read(path).unwrap();
        let other = work.create("other").unwrap();
        work.input = "other draft".into();
        wait(&mut work);
        assert_eq!(work.active_id(), other);
        assert_eq!(work.input, "other draft");
        assert!(work.take_workflow_visits().is_empty());
        assert_eq!(work.take_workflow_loads().len(), 1);
        let source = work.instances.iter().find(|i| i.id == owner).unwrap();
        assert!(source.state.text_flow.modal_open());
        assert_eq!(source.state.active_tool_id(), "text-flow");
    }
    fn wait(workspace: &mut Workspace) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while workspace
            .instances
            .iter()
            .any(|i| i.state.workflow.files.job.phase.active())
        {
            assert!(std::time::Instant::now() < deadline);
            workspace.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    #[test]
    fn successful_workflow_load_events_are_once_bound_to_live_owner_and_never_failure() {
        let fixture = Fixture::new();
        fixture.definition("a.json", "实际载入");
        let mut workspace = fixture.workspace();
        let owner = workspace.active_id().to_owned();
        workspace
            .workflow
            .files
            .read(fixture.0.join("a.json"))
            .unwrap();
        let other = workspace.create("另一个实例").unwrap();
        workspace.input = "另一份数据".into();
        wait(&mut workspace);
        let events = workspace.take_workflow_loads();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "实际载入");
        assert_eq!(workspace.active_id(), other);
        assert_eq!(workspace.input, "另一份数据");
        assert!(workspace.take_workflow_loads().is_empty());
        assert!(workspace.instances[0].state.workflow.files.review.is_some());
        workspace
            .workflow
            .files
            .read(fixture.0.join("missing.json"))
            .unwrap();
        wait(&mut workspace);
        assert!(workspace.take_workflow_loads().is_empty());
        workspace.instances[0].state.workflow.files.review = None;
        workspace.instances[0]
            .state
            .workflow
            .files
            .read(fixture.0.join("a.json"))
            .unwrap();
        wait(&mut workspace);
        workspace.instances.retain(|instance| instance.id != owner);
        assert!(workspace.take_workflow_loads().is_empty());
    }
    #[test]
    fn workflow_bookmark_opens_without_scan_reloads_and_preserves_active_content_on_failures() {
        let fixture = Fixture::new();
        fixture.definition("daily.json", "收藏时摘要");
        let mut checked = fixture.workspace();
        checked.workflow.files.list(fixture.0.clone()).unwrap();
        wait(&mut checked);
        let found = checked.workflow_matches("收藏时");
        let bookmark = checked.workflow_bookmark(&found.entries[0]).unwrap();
        checked.workflow.files.list(fixture.0.clone()).unwrap();
        assert!(checked.workflow_bookmark(&found.entries[0]).is_err());
        wait(&mut checked);
        let mut reopened = fixture.workspace();
        reopened.input = "current input untouched".into();
        assert!(reopened.workflow.files.listing.is_none());
        fixture.definition("daily.json", "文件新内容");
        reopened.open_workflow_bookmark(&bookmark).unwrap();
        wait(&mut reopened);
        assert_eq!(
            reopened.workflow.files.review.as_ref().unwrap().name,
            "文件新内容"
        );
        assert_eq!(reopened.input, "current input untouched");
        assert!(reopened.workflow.files.listing.is_none());
        assert!(reopened.open_workflow_bookmark(&bookmark).is_err());
        reopened.workflow.files.review = None;
        std::fs::remove_file(&bookmark.path).unwrap();
        reopened.open_workflow_bookmark(&bookmark).unwrap();
        wait(&mut reopened);
        assert!(reopened.workflow.files.review.is_none());
        assert_eq!(reopened.workflow.files.job.phase, Phase::Failed);
        assert_eq!(reopened.input, "current input untouched");
        let active = reopened.active_id().to_owned();
        reopened.workflow.files.job.begin();
        assert!(reopened.open_workflow_bookmark(&bookmark).is_err());
        assert_eq!(reopened.active_id(), active);
        reopened
            .workflow
            .files
            .job
            .finish(Phase::Done, "test resumed");
        std::fs::write(&bookmark.path, "not a workflow").unwrap();
        reopened.open_workflow_bookmark(&bookmark).unwrap();
        wait(&mut reopened);
        assert!(reopened.workflow.files.review.is_none());
        assert_eq!(reopened.input, "current input untouched");
    }
    #[test]
    fn workflow_search_cache_routes_original_instance_and_reloads_changed_file_for_review() {
        let fixture = Fixture::new();
        fixture.definition("daily.json", "每日清洗");
        let mut workspace = fixture.workspace();
        let first = workspace.active_id().to_owned();
        workspace.input = "original table draft".into();
        workspace.workflow.files.list(fixture.0.clone()).unwrap();
        wait(&mut workspace);
        let found = workspace.workflow_matches("每日");
        assert_eq!(found.total, 1);
        assert!(Arc::ptr_eq(
            &found.entries,
            &workspace.workflow_matches("每日").entries
        ));
        let selected = found.entries[0].clone();
        let second = workspace.create("另一份资料").unwrap();
        workspace.input = "other draft".into();
        workspace.instances[0].state.workflow.files.job.begin();
        assert!(workspace.open_workflow_match(&selected).is_err());
        assert_eq!(workspace.active_id(), second);
        workspace.instances[0]
            .state
            .workflow
            .files
            .job
            .finish(Phase::Cancelled, "fixture ended");
        fixture.definition("daily.json", "文件变更后的流程");
        workspace.open_workflow_match(&selected).unwrap();
        assert_eq!(workspace.active_id(), first);
        wait(&mut workspace);
        assert_eq!(
            workspace.workflow.files.review.as_ref().unwrap().name,
            "文件变更后的流程"
        );
        assert_eq!(workspace.input, "original table draft");
        assert_eq!(workspace.instances[1].state.input, "other draft");
        assert!(workspace.dataset.is_none());
        assert!(workspace.open_workflow_match(&selected).is_err());
        workspace.workflow.files.review = None;
        let mut forged = selected.clone();
        forged.file_name = "../outside.json".into();
        assert!(workspace.open_workflow_match(&forged).is_err());
        // Refresh with the same filename must still reject an older result key.
        workspace.workflow.files.list(fixture.0.clone()).unwrap();
        wait(&mut workspace);
        assert!(workspace.open_workflow_match(&selected).is_err());
        let refreshed = workspace.workflow_matches("文件变更");
        assert_eq!(refreshed.total, 1);
        assert_ne!(
            refreshed.entries[0].listing_revision,
            selected.listing_revision
        );
        let alternate = fixture.0.join("alternate");
        std::fs::create_dir(&alternate).unwrap();
        fixture.definition("alternate/daily.json", "另一目录的同名文件");
        workspace.workflow.files.list(alternate).unwrap();
        wait(&mut workspace);
        assert!(
            workspace
                .open_workflow_match(&refreshed.entries[0])
                .is_err()
        );
        assert_eq!(workspace.workflow_matches("同名文件").total, 1);
        workspace.workflow.files.list(fixture.0.clone()).unwrap();
        wait(&mut workspace);
        std::fs::remove_file(fixture.0.join("daily.json")).unwrap();
        workspace.workflow.files.list(fixture.0.clone()).unwrap();
        wait(&mut workspace);
        assert_eq!(workspace.workflow_matches("每日").total, 0);
        assert!(workspace.open_workflow_match(&selected).is_err());
        workspace.select(&second).unwrap();
        workspace.close(&first, true).unwrap();
        assert!(workspace.open_workflow_match(&selected).is_err());
        assert_eq!(workspace.active_id(), second);
    }
    #[test]
    fn workflow_search_bounds_4096_candidates_and_reuses_cached_metadata() {
        let fixture = Fixture::new();
        for index in 0..256 {
            fixture.definition(&format!("{index}.json"), &format!("批次{index:03}"));
        }
        let mut workspace = fixture.workspace();
        for index in 0..16 {
            if index > 0 {
                workspace.create(&format!("表格{index}")).unwrap();
            }
            workspace.workflow.files.list(fixture.0.clone()).unwrap();
        }
        wait(&mut workspace);
        let started = std::time::Instant::now();
        let found = workspace.workflow_matches("流程");
        let elapsed = started.elapsed();
        assert_eq!(found.total, 4096);
        assert_eq!(found.entries.len(), 100);
        assert_eq!(found.entries[0].instance_id, workspace.active_id());
        let started = std::time::Instant::now();
        let cached = workspace.workflow_matches("流程");
        eprintln!(
            "SEARCH candidates=4096 first_us={} cache_us={}",
            elapsed.as_micros(),
            started.elapsed().as_micros()
        );
        assert!(Arc::ptr_eq(&found.entries, &cached.entries));
        workspace
            .select(&workspace.instances[0].id.clone())
            .unwrap();
        assert!(!Arc::ptr_eq(
            &cached.entries,
            &workspace.workflow_matches("流程").entries
        ));
        assert!(workspace.workflow_matches("").entries.is_empty());
    }
}
#[derive(Clone, Default)]
pub struct WorkflowMatches {
    pub entries: Arc<Vec<WorkflowMatch>>,
    pub total: usize,
}
#[derive(Default)]
pub(super) struct Cache {
    query: String,
    revision: u64,
    value: WorkflowMatches,
}
impl Workspace {
    pub fn take_workflow_loads(&mut self) -> Vec<crate::preferences::SavedWorkflow> {
        std::mem::take(&mut self.workflow_loads)
            .into_iter()
            .filter(|(id, metadata)| {
                metadata.valid() && self.instances.iter().any(|instance| &instance.id == id)
            })
            .map(|(_, metadata)| metadata)
            .collect()
    }
    pub fn workflow_is_bookmarked(
        &self,
        selected: &WorkflowMatch,
        bookmarks: &[crate::preferences::SavedWorkflow],
    ) -> bool {
        self.instances
            .iter()
            .find(|instance| instance.id == selected.instance_id)
            .filter(|instance| {
                instance.state.workflow.files.listing_revision == selected.listing_revision
            })
            .and_then(|instance| instance.state.workflow.files.listing.as_ref())
            .and_then(|listing| {
                listing
                    .entries
                    .iter()
                    .find(|entry| entry.path.file_name() == Some(selected.file_name.as_os_str()))
            })
            .is_some_and(|entry| bookmarks.iter().any(|bookmark| bookmark.path == entry.path))
    }
    pub fn workflow_bookmark(
        &self,
        selected: &WorkflowMatch,
    ) -> Result<crate::preferences::SavedWorkflow> {
        let instance = self
            .instances
            .iter()
            .find(|instance| instance.id == selected.instance_id)
            .context("该流程所属实例已关闭，请重新搜索")?;
        let files = &instance.state.workflow.files;
        anyhow::ensure!(
            files.listing_revision == selected.listing_revision,
            "流程列表已变化，请重新搜索"
        );
        let entry = files
            .listing
            .as_ref()
            .and_then(|listing| {
                listing
                    .entries
                    .iter()
                    .find(|entry| entry.path.file_name() == Some(selected.file_name.as_os_str()))
            })
            .context("流程列表已变化，请重新搜索")?;
        let bookmark = crate::preferences::SavedWorkflow {
            path: entry.path.clone(),
            name: entry.name.clone(),
            steps: entry.steps,
        };
        anyhow::ensure!(bookmark.valid(), "流程收藏信息无效");
        Ok(bookmark)
    }
    pub fn open_workflow_bookmark(
        &mut self,
        bookmark: &crate::preferences::SavedWorkflow,
    ) -> Result<()> {
        anyhow::ensure!(
            !self.operation_pending() && !self.modal_open(),
            "请先完成当前实例的保存或确认操作"
        );
        anyhow::ensure!(bookmark.valid(), "流程收藏信息无效");
        self.open_bookmarked_workflow(bookmark.path.clone())
    }
    pub fn workflow_matches(&mut self, query: &str) -> WorkflowMatches {
        let query = query.trim().to_lowercase();
        let mut hasher = DefaultHasher::new();
        self.active_id().hash(&mut hasher);
        for instance in &self.instances {
            instance.id.hash(&mut hasher);
            instance.name.hash(&mut hasher);
            instance
                .state
                .workflow
                .files
                .listing_revision
                .hash(&mut hasher);
            instance
                .state
                .workflow
                .files
                .listing
                .is_some()
                .hash(&mut hasher);
        }
        let revision = hasher.finish();
        if self.workflow_search.query == query && self.workflow_search.revision == revision {
            return self.workflow_search.value.clone();
        }
        let mut entries = Vec::new();
        if !query.is_empty() {
            for instance in &self.instances {
                if let Some(listing) = &instance.state.workflow.files.listing {
                    for entry in &listing.entries {
                        let file = entry.path.file_name().unwrap_or_default();
                        let text = format!(
                            "流程 workflow recipe {} {} {}",
                            entry.name,
                            file.to_string_lossy(),
                            instance.name
                        )
                        .to_lowercase();
                        if query.split_whitespace().all(|word| text.contains(word)) {
                            entries.push(WorkflowMatch {
                                instance_id: instance.id.clone(),
                                instance_name: instance.name.clone(),
                                file_name: file.to_owned(),
                                name: entry.name.clone(),
                                steps: entry.steps,
                                listing_revision: instance.state.workflow.files.listing_revision,
                            });
                        }
                    }
                }
            }
        }
        entries.sort_by(|a, b| {
            (b.name.to_lowercase() == query)
                .cmp(&(a.name.to_lowercase() == query))
                .then_with(|| {
                    (b.instance_id == self.active_id()).cmp(&(a.instance_id == self.active_id()))
                })
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.file_name.cmp(&b.file_name))
                .then_with(|| a.instance_id.cmp(&b.instance_id))
        });
        let total = entries.len();
        entries.truncate(100);
        let value = WorkflowMatches {
            entries: Arc::new(entries),
            total,
        };
        self.workflow_search = Cache {
            query,
            revision,
            value: value.clone(),
        };
        value
    }
    pub fn open_workflow_match(&mut self, selected: &WorkflowMatch) -> Result<()> {
        anyhow::ensure!(
            !self.operation_pending() && !self.modal_open(),
            "请先完成当前实例的保存或确认操作"
        );
        let index = self
            .instances
            .iter()
            .position(|instance| instance.id == selected.instance_id)
            .context("该流程所属实例已关闭，请重新搜索")?;
        self.instances[index]
            .state
            .open_searched_workflow(&selected.file_name, selected.listing_revision)?;
        self.active = index;
        Ok(())
    }
}
