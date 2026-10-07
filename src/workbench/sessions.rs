use super::*;
use crate::workspace_store::{Entry, MAX_SNAPSHOT, Store, validate_name};
use std::ops::{Deref, DerefMut};
mod ui;
mod workflows;
pub use workflows::{WorkflowMatch, WorkflowMatches};

pub struct Instance {
    pub id: String,
    pub name: String,
    pub state: DataState,
    saved: Option<(String, i64)>,
}

pub struct Workspace {
    pub instances: Vec<Instance>,
    active: usize,
    store: Store,
    entries: Vec<Entry>,
    receiver: Option<Receiver<Reply>>,
    library_open: bool,
    library_query: String,
    save_confirm: bool,
    save_copy: bool,
    close_confirm: Option<String>,
    delete_confirm: Option<Entry>,
    status: String,
    workflow_search: workflows::Cache,
    workflow_loads: Vec<(String, crate::preferences::SavedWorkflow)>,
}

enum Reply {
    Listed(Result<Vec<Entry>>),
    Saved {
        live: String,
        saved: String,
        result: Result<i64>,
    },
    Loaded(Result<(Entry, Box<DataState>)>),
    Deleted {
        id: String,
        result: Result<()>,
    },
}

impl DataState {
    pub fn busy(&self) -> bool {
        self.text_flow.busy()
            || self.parse_job.phase.active()
            || self.join.job.phase.active()
            || self.sqlite_export.job.phase.active()
            || self.workflow.job.phase.active()
            || self.workflow.files.job.phase.active()
            || self.workflow.output.busy()
    }
    pub fn has_content(&self) -> bool {
        self.text_flow.has_content()
            || !self.input.is_empty()
            || self.dataset.is_some()
            || !self.output.is_empty()
            || self.join.has_content()
            || self.workflow.has_content()
    }

    pub fn snapshot(&self) -> Result<Vec<u8>> {
        anyhow::ensure!(
            !self.busy(),
            "请等待当前实例的解析、合并、操作流程或数据库另存任务结束再保存"
        );
        self.validate_saved()?;
        let bytes = serde_json::to_vec(self)?;
        anyhow::ensure!(
            bytes.len() <= MAX_SNAPSHOT,
            "当前快照超过 64 MiB，请缩小材料"
        );
        Ok(bytes)
    }

    pub fn restore(bytes: &[u8]) -> Result<Self> {
        anyhow::ensure!(bytes.len() <= MAX_SNAPSHOT, "保存内容过大");
        let mut state: Self = serde_json::from_slice(bytes).context("工作实例内容格式无效")?;
        state.validate_saved()?;
        if let Some(data) = &state.dataset {
            state.visible = data.view(&state.query, state.sort, state.descending);
        }
        Ok(state)
    }

    fn validate_saved(&self) -> Result<()> {
        self.text_flow.validate()?;
        anyhow::ensure!(
            self.input.len() <= INPUT_LIMIT
                && self.output.len() <= 8 * 1024 * 1024
                && self.path.len() <= 32768
                && self.export_path.len() <= 32768
                && self.query.len() <= INPUT_LIMIT,
            "数据草稿超过保存上限"
        );
        if let Some(data) = &self.dataset {
            data.validate_saved()?;
        }
        let columns = self.dataset.as_ref().map_or(0, |d| d.headers.len());
        anyhow::ensure!(self.sort.is_none_or(|i| i < columns), "保存的排序列无效");
        self.join.validate_saved(columns)?;
        self.transform.validate_saved(columns)?;
        Ok(())
    }
}

impl Workspace {
    pub(crate) fn import_table(&mut self, data: Dataset, name: &str) -> Result<()> {
        data.validate_saved()?;
        // Column-aware JSON preserves empty tables, order and value types on reparse.
        let input = serde_json::to_string(&data)?;
        anyhow::ensure!(
            input.len() <= INPUT_LIMIT,
            "当前页输入表示超过2 MiB，未创建实例"
        );
        let message = format!(
            "已接收独立表格材料：{}行 / {}列；未运行流程，范围以来源审核为准",
            data.rows.len(),
            data.headers.len()
        );
        self.create(name)?;
        let state = self.deref_mut();
        state.input = input;
        state.format = DataFormat::Json;
        state.replace_with_join(data);
        state.message = message;
        Ok(())
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self, show_library: bool, show_save: bool) {
        self.instances.clear();
        self.active = 0;
        self.create("客户名单 · 清洗版本").unwrap();
        self.deref_mut().preview_transform();
        let first = self.active_id().to_owned();
        self.create("订单关联 · 十月数据").unwrap();
        self.deref_mut().preview_join();
        self.entries.clear();
        for i in &mut self.instances {
            let saved = uuid::Uuid::new_v4().to_string();
            let revision = self
                .store
                .save(
                    &saved,
                    &i.name,
                    "data-v1",
                    &i.state.snapshot().unwrap(),
                    None,
                )
                .unwrap();
            i.saved = Some((saved.clone(), revision));
            self.entries.push(self.store.load(&saved).unwrap().0);
            i.state.parse_job.begin();
            i.state.parse_job.finish(Phase::Done, "合成预览数据已解析");
        }
        self.select(&first).unwrap();
        self.library_open = show_library;
        self.save_confirm = show_save;
    }
    pub fn new(path: PathBuf) -> Self {
        Self {
            instances: vec![Instance {
                id: uuid::Uuid::new_v4().to_string(),
                name: "未命名数据 1".into(),
                state: DataState::default(),
                saved: None,
            }],
            active: 0,
            store: Store { path },
            entries: Vec::new(),
            receiver: None,
            library_open: false,
            library_query: String::new(),
            save_confirm: false,
            save_copy: false,
            close_confirm: None,
            delete_confirm: None,
            status: String::new(),
            workflow_search: workflows::Cache::default(),
            workflow_loads: Vec::new(),
        }
    }
    pub fn active_id(&self) -> &str {
        &self.instances[self.active].id
    }
    pub fn has_work(&self) -> bool {
        self.instances
            .iter()
            .any(|i| i.state.has_content() || i.state.busy())
            || self.receiver.is_some()
    }
    pub fn operation_pending(&self) -> bool {
        self.receiver.is_some()
    }
    pub fn has_active_tasks(&self) -> bool {
        self.operation_pending() || self.instances.iter().any(|instance| instance.state.busy())
    }
    pub fn modal_open(&self) -> bool {
        self.save_confirm
            || self.close_confirm.is_some()
            || self.delete_confirm.is_some()
            || self.text_flow.modal_open()
    }
    pub fn open_library(&mut self) {
        if !self.operation_pending() {
            self.library_open = true;
            self.list_saved();
        }
    }
    pub fn create(&mut self, name: &str) -> Result<String> {
        anyhow::ensure!(
            self.instances.len() < 16,
            "最多同时打开 16 个数据实例，请先保存并关闭不需要的实例"
        );
        validate_name(name)?;
        let id = uuid::Uuid::new_v4().to_string();
        self.instances.push(Instance {
            id: id.clone(),
            name: name.trim().into(),
            state: DataState::default(),
            saved: None,
        });
        self.active = self.instances.len() - 1;
        Ok(id)
    }
    pub fn select(&mut self, id: &str) -> Result<()> {
        self.active = self
            .instances
            .iter()
            .position(|i| i.id == id)
            .context("该实例已关闭；可从已保存实例恢复")?;
        Ok(())
    }
    pub fn close(&mut self, id: &str, discard: bool) -> Result<()> {
        anyhow::ensure!(!self.operation_pending(), "请等待保存或恢复完成");
        let index = self
            .instances
            .iter()
            .position(|i| i.id == id)
            .context("实例已关闭")?;
        anyhow::ensure!(
            !self.instances[index].state.busy(),
            "实例仍在运行，请等待结束或取消相关任务"
        );
        anyhow::ensure!(
            discard || !self.instances[index].state.has_content(),
            "关闭需要确认放弃未保存内容"
        );
        self.instances.remove(index);
        if self.instances.is_empty() {
            self.create("未命名数据 1")?;
        } else if index < self.active {
            self.active -= 1;
        } else {
            self.active = self.active.min(self.instances.len() - 1);
        }
        Ok(())
    }
    pub fn import_new(
        &mut self,
        text: String,
        format: DataFormat,
        tsv: bool,
        name: &str,
    ) -> Result<()> {
        anyhow::ensure!(text.len() <= INPUT_LIMIT, "导入最多 2 MiB");
        self.create(name)?;
        self.deref_mut().import_text(text, format, tsv)
    }
    pub fn import_text_flow(&mut self, text: String, name: &str) -> Result<()> {
        // Validate before creating an instance, including the stricter action limit.
        let mut state = crate::text_flow::State::default();
        state.receive(text)?;
        self.create(name)?;
        self.deref_mut().text_flow = state;
        self.deref_mut().set_active_tool("text-flow");
        Ok(())
    }
    pub fn snapshots(&self) -> Vec<crate::tasks::Row> {
        self.instances
            .iter()
            .flat_map(|instance| {
                [
                    instance.state.parse_job.snapshot("data", "数据解析", false),
                    instance
                        .state
                        .join
                        .job
                        .snapshot("csv-merge", "表格合并与关联", true),
                    instance.state.sqlite_export.job.snapshot(
                        "sqlite-export",
                        "表格另存 SQLite",
                        true,
                    ),
                    instance
                        .state
                        .workflow
                        .job
                        .snapshot("pipeline", "操作流程预览", true),
                    instance.state.workflow.files.job.snapshot(
                        "workflow-file",
                        "流程文件保存/读取",
                        false,
                    ),
                    instance.state.workflow.output.job.snapshot(
                        "workflow-output",
                        "流程结果文件保存",
                        true,
                    ),
                    instance
                        .state
                        .text_flow
                        .job
                        .snapshot("text-flow", "工具流程与文件", true),
                ]
                .into_iter()
                .flatten()
                .map(|mut row| {
                    row.instance = Some(instance.id.clone());
                    row.instance_name = Some(instance.name.clone());
                    row
                })
            })
            .collect()
    }
    pub fn cancel(&mut self, id: &str, generation: u64) {
        if let Some(i) = self.instances.iter_mut().find(|i| i.id == id)
            && i.state
                .join
                .job
                .snapshot("csv-merge", "", true)
                .is_some_and(|r| r.generation == generation)
        {
            i.state.cancel_join();
        }
    }
    pub fn cancel_sqlite(&mut self, id: &str, generation: u64) {
        if let Some(instance) = self.instances.iter_mut().find(|i| i.id == id)
            && instance
                .state
                .sqlite_export
                .job
                .snapshot("sqlite-export", "", true)
                .is_some_and(|r| r.generation == generation)
        {
            instance.state.sqlite_export.cancel();
        }
    }
    pub fn cancel_workflow(&mut self, id: &str, generation: u64) {
        if let Some(instance) = self.instances.iter_mut().find(|i| i.id == id)
            && instance
                .state
                .workflow
                .job
                .snapshot("pipeline", "", true)
                .is_some_and(|r| r.generation == generation)
        {
            instance.state.workflow.cancel();
        }
    }
    pub fn cancel_text_flow(&mut self, id: &str, generation: u64) {
        if let Some(instance) = self.instances.iter_mut().find(|i| i.id == id)
            && instance
                .state
                .text_flow
                .job
                .snapshot("text-flow", "", true)
                .is_some_and(|row| row.generation == generation)
        {
            instance.state.text_flow.cancel();
        }
    }
    pub fn cancel_workflow_output(&mut self, id: &str, generation: u64) {
        if let Some(instance) = self.instances.iter_mut().find(|i| i.id == id)
            && instance
                .state
                .workflow
                .output
                .job
                .snapshot("workflow-output", "", true)
                .is_some_and(|r| r.generation == generation)
        {
            instance.state.workflow.output.cancel();
        }
    }
    pub fn poll(&mut self) {
        for instance in &mut self.instances {
            instance.state.poll();
            if let Some(loaded) = instance.state.workflow.files.loaded.take() {
                self.workflow_loads.push((instance.id.clone(), loaded));
                if self.workflow_loads.len() > 32 {
                    self.workflow_loads.remove(0);
                }
            }
        }
        let reply = self.receiver.as_ref().and_then(|r| match r.try_recv() {
            Ok(reply) => Some(reply),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Reply::Listed(Err(anyhow!(
                "保存任务意外结束，请检查后重试"
            )))),
        });
        let Some(reply) = reply else {
            return;
        };
        self.receiver = None;
        let result: Result<()> = (|| {
            match reply {
                Reply::Listed(result) => {
                    self.entries = result?;
                    self.status.clear();
                }
                Reply::Saved {
                    live,
                    saved,
                    result,
                } => {
                    let revision = result?;
                    if let Some(i) = self.instances.iter_mut().find(|i| i.id == live) {
                        i.saved = Some((saved, revision));
                    }
                    self.status = format!("已保存第 {revision} 版快照；后续修改请再次保存");
                }
                Reply::Loaded(result) => {
                    let (entry, state) = result?;
                    if let Some(index) = self
                        .instances
                        .iter()
                        .position(|i| i.saved.as_ref().is_some_and(|(id, _)| id == &entry.id))
                    {
                        self.active = index;
                        self.status = "已回到打开的实例；保留其中的修改".into();
                    } else {
                        let id = self.create(&entry.name)?;
                        let i = self.instances.iter_mut().find(|i| i.id == id).unwrap();
                        i.state = *state;
                        i.saved = Some((entry.id, entry.revision));
                        self.status = "已恢复为独立实例，没有重新执行任务".into();
                    }
                    self.library_open = false;
                }
                Reply::Deleted { id, result } => {
                    result?;
                    self.entries.retain(|e| e.id != id);
                    for i in &mut self.instances {
                        if i.saved.as_ref().is_some_and(|(key, _)| key == &id) {
                            i.saved = None;
                        }
                    }
                    self.status = "已删除保存项；已打开的内存实例仍保留".into();
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.status = error.to_string();
        }
    }
    fn list_saved(&mut self) {
        let store = self.store.clone();
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(Reply::Listed(store.list()));
        });
    }
    fn save_active(&mut self, copy: bool) -> Result<()> {
        anyhow::ensure!(self.receiver.is_none(), "请等待当前保存操作结束");
        let i = &self.instances[self.active];
        validate_name(&i.name)?;
        let bytes = i.state.snapshot()?;
        let (saved, expected) = if !copy {
            i.saved
                .as_ref()
                .map(|(id, rev)| (id.clone(), Some(*rev)))
                .unwrap_or_else(|| (uuid::Uuid::new_v4().to_string(), None))
        } else {
            (uuid::Uuid::new_v4().to_string(), None)
        };
        let live = i.id.clone();
        let name = i.name.clone();
        let store = self.store.clone();
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        std::thread::spawn(move || {
            let result = DataState::restore(&bytes)
                .and_then(|_| store.save(&saved, &name, "data-v1", &bytes, expected));
            let _ = tx.send(Reply::Saved {
                live,
                saved,
                result,
            });
        });
        Ok(())
    }
    fn load_saved(&mut self, id: String) {
        let store = self.store.clone();
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        std::thread::spawn(move || {
            let result = store.load(&id).and_then(|(e, b)| {
                anyhow::ensure!(e.kind == "data-v1", "不支持此工作实例类型");
                Ok((e, Box::new(DataState::restore(&b)?)))
            });
            let _ = tx.send(Reply::Loaded(result));
        });
    }
}
impl Deref for Workspace {
    type Target = DataState;
    fn deref(&self) -> &DataState {
        &self.instances[self.active].state
    }
}
impl DerefMut for Workspace {
    fn deref_mut(&mut self) -> &mut DataState {
        &mut self.instances[self.active].state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn text_flow_handoff_creates_independent_draft_and_roundtrips_without_execution() {
        let mut work = Workspace::new(PathBuf::from("unused-text-flow.db"));
        work.input = "table original".into();
        let source = work.active_id().to_owned();
        work.import_text_flow("中文🦀".into(), "text flow").unwrap();
        assert_ne!(source, work.active_id());
        assert!(work.has_work());
        assert!(!work.busy());
        let snapshot = work.snapshot().unwrap();
        let restored = DataState::restore(&snapshot).unwrap();
        assert!(restored.text_flow.has_content());
        assert!(!restored.busy());
        assert!(
            work.import_text_flow("x".repeat(1024 * 1024 + 1), "too large")
                .is_err()
        );
        assert_eq!(work.instances.len(), 2);
        work.select(&source).unwrap();
        assert_eq!(work.input, "table original");
    }
    #[test]
    fn old_data_snapshot_without_text_flow_restores_its_existing_table() {
        let mut data = DataState::default();
        data.input = "id,name\n001,中文".into();
        data.dataset = Some(Dataset::parse(&data.input, DataFormat::Csv, b',').unwrap());
        let mut old = serde_json::to_value(&data).unwrap();
        old.as_object_mut().unwrap().remove("text_flow");
        let restored = DataState::restore(&serde_json::to_vec(&old).unwrap()).unwrap();
        assert_eq!(restored.dataset, data.dataset);
        assert!(!restored.text_flow.has_content());
        assert!(!restored.busy());
        assert_eq!(restored.active_tool_id(), "data");
        let mut restored = restored;
        for id in [
            "text-flow",
            "data-sqlite-export",
            "csv-merge",
            "data-transform",
            "pipeline",
            "workspace-sessions",
        ] {
            restored.set_active_tool(id);
            assert_eq!(restored.active_tool_id(), id);
        }
        restored.set_active_tool("unknown");
        assert_eq!(restored.active_tool_id(), "workspace-sessions");
    }
    #[test]
    fn typed_page_import_creates_independent_work_and_rejects_oversize_before_creation() {
        let mut workspace = Workspace::new(
            std::env::temp_dir().join(format!("zi-page-work-{}.db", uuid::Uuid::new_v4())),
        );
        workspace.input = "original draft".into();
        let source = workspace.active_id().to_owned();
        let data = Dataset::from_parts(
            vec!["id".into(), "quantity".into()],
            vec![vec![serde_json::json!("001"), serde_json::json!(2)]],
        )
        .unwrap();
        workspace
            .import_table(data.clone(), "SQLite current page")
            .unwrap();
        assert_ne!(workspace.active_id(), source);
        assert_eq!(workspace.instances[0].state.input, "original draft");
        assert_eq!(workspace.dataset.as_ref(), Some(&data));
        assert_eq!(workspace.visible, vec![0]);
        assert!(!workspace.busy());
        let count = workspace.instances.len();
        let huge = Dataset::from_parts(
            vec!["id".into()],
            vec![vec![serde_json::json!("x".repeat(INPUT_LIMIT + 1))]],
        )
        .unwrap();
        assert!(workspace.import_table(huge, "too large").is_err());
        assert_eq!(workspace.instances.len(), count);
        assert_eq!(workspace.dataset.as_ref(), Some(&data));
        let empty = Dataset::from_parts(vec!["id".into()], vec![]).unwrap();
        workspace.import_table(empty.clone(), "empty page").unwrap();
        assert_eq!(workspace.dataset.as_ref(), Some(&empty));
        assert!(workspace.visible.is_empty());
        while workspace.instances.len() < 16 {
            workspace.create("capacity fixture").unwrap();
        }
        let active = workspace.active_id().to_owned();
        assert!(workspace.import_table(data, "over capacity").is_err());
        assert_eq!(workspace.instances.len(), 16);
        assert_eq!(workspace.active_id(), active);
        assert_eq!(workspace.instances[0].state.input, "original draft");
    }
    fn wait(workspace: &mut Workspace) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while workspace.instances.iter().any(|i| i.state.busy()) || workspace.operation_pending() {
            workspace.poll();
            assert!(std::time::Instant::now() < deadline, "worker timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    #[test]
    fn empty_sqlite_page_reparses_after_snapshot_restore_without_losing_headers() {
        let mut workspace = Workspace::new(
            std::env::temp_dir().join(format!("zi-empty-page-{}.db", uuid::Uuid::new_v4())),
        );
        workspace.input = "original draft".into();
        let source = workspace.active_id().to_owned();
        let data = Dataset::from_parts(
            vec![
                "中文🦀".into(),
                "comma,name".into(),
                "quote\"name".into(),
                "line\nname".into(),
            ],
            vec![],
        )
        .unwrap();
        workspace
            .import_table(data.clone(), "empty SQLite page")
            .unwrap();
        assert!(workspace.format == DataFormat::Json);
        let snapshot = workspace.snapshot().unwrap();
        let mut restored = DataState::restore(&snapshot).unwrap();
        restored.parse();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while restored.busy() {
            restored.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(
            restored.parse_job.phase,
            Phase::Done,
            "{}",
            restored.message
        );
        assert_eq!(restored.dataset.as_ref(), Some(&data));
        assert!(restored.visible.is_empty());
        workspace.select(&source).unwrap();
        assert_eq!(workspace.input, "original draft");
        assert_eq!(workspace.instances.len(), 2);
    }
    #[test]
    fn structured_table_draft_reparse_preserves_wide_columns_and_value_types() {
        let mut workspace = Workspace::new(PathBuf::from("unused-structured-table.db"));
        workspace.input = "source draft".into();
        let data = Dataset::from_parts(
            vec![
                format!("z{}", "x".repeat(2048)),
                "a".into(),
                "null".into(),
                "list".into(),
            ],
            vec![
                vec![
                    serde_json::json!(7),
                    serde_json::json!(true),
                    serde_json::Value::Null,
                    serde_json::json!(["中文", 1])
                ];
                1024
            ],
        )
        .unwrap();
        assert!(
            data.export(&(0..1024).collect::<Vec<_>>(), DataFormat::Json, b',')
                .unwrap()
                .len()
                > INPUT_LIMIT
        );
        workspace
            .import_table(data.clone(), "typed material")
            .unwrap();
        assert!(workspace.input.len() < INPUT_LIMIT);
        let snapshot = workspace.snapshot().unwrap();
        let mut restored = DataState::restore(&snapshot).unwrap();
        restored.parse();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while restored.busy() {
            restored.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(
            restored.parse_job.phase,
            Phase::Done,
            "{}",
            restored.message
        );
        assert_eq!(restored.dataset.as_ref(), Some(&data));
        assert_eq!(restored.visible.len(), 1024);
        assert_eq!(workspace.instances[0].state.input, "source draft");
        for invalid in [
            r#"{"headers":["a"],"headers":["b"],"rows":[]}"#,
            r#"{"headers":["a"],"rows":[[1,2]]}"#,
            r#"{"headers":["a"],"rows":[],"target":"x"}"#,
        ] {
            assert!(Dataset::parse(invalid, DataFormat::Json, b',').is_err());
        }
    }
    #[test]
    fn independent_work_continues_and_saved_snapshot_restores_without_execution() {
        let root = std::env::temp_dir().join(format!("zi-instances-{}", uuid::Uuid::new_v4()));
        let mut w = Workspace::new(root.join("workspace.sqlite3"));
        let first = w.active_id().to_owned();
        w.instances[0].name = "第一份订单".into();
        w.import_text("id,name\n1,Alpha\n2,Beta".into(), DataFormat::Csv, false)
            .unwrap();
        assert!(w.snapshot().is_err());
        assert!(w.close(&first, true).is_err());
        w.import_new(
            "[{\"id\":9,\"name\":\"Other\"}]".into(),
            DataFormat::Json,
            false,
            "另一份",
        )
        .unwrap();
        let second = w.active_id().to_owned();
        wait(&mut w);
        assert_eq!(w.snapshots().len(), 2);
        assert!(w.snapshots().iter().all(|r| r.phase == Phase::Done));
        assert!(!root.exists(), "temporary work never creates a database");
        w.select(&first).unwrap();
        w.query = "Alpha".into();
        w.visible = w.dataset.as_ref().unwrap().view("Alpha", None, false);
        w.output = w
            .dataset
            .as_ref()
            .unwrap()
            .export(&w.visible, DataFormat::Json, b',')
            .unwrap();
        let saved_output = w.output.clone();
        assert!(w.close(&first, false).is_err());
        w.save_active(false).unwrap();
        wait(&mut w);
        assert!(w.status.contains("已保存"), "{}", w.status);
        let saved = w.instances[0].saved.clone().unwrap();
        w.input = "unsaved edit".into();
        w.select(&second).unwrap();
        assert!(w.input.contains("Other"));
        w.close(&first, true).unwrap();
        w.load_saved(saved.0.clone());
        wait(&mut w);
        assert_eq!(w.output, saved_output);
        assert_eq!(w.query, "Alpha");
        assert_eq!(w.visible.len(), 1);
        assert_eq!(w.parse_job.phase, Phase::Idle);
        assert!(w.receiver.is_none());
        assert_ne!(w.active_id(), second);
        assert_eq!(w.instances.len(), 2);
        let restored = w.active_id().to_owned();
        w.input = "keep live edit".into();
        w.load_saved(saved.0);
        wait(&mut w);
        assert_eq!(w.active_id(), restored);
        assert_eq!(w.input, "keep live edit");
        assert_eq!(w.instances.len(), 2);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn invalid_snapshot_is_rejected_and_instance_limits_preserve_existing_work() {
        let mut state = DataState::default();
        state.input = "a\n1".into();
        state.dataset = Some(Dataset::parse(&state.input, DataFormat::Csv, b',').unwrap());
        let valid = state.snapshot().unwrap();
        let mut value: Value = serde_json::from_slice(&valid).unwrap();
        value["sort"] = Value::from(99);
        assert!(DataState::restore(&serde_json::to_vec(&value).unwrap()).is_err());
        value["sort"] = Value::Null;
        value["dataset"]["rows"][0] = serde_json::json!([]);
        assert!(DataState::restore(&serde_json::to_vec(&value).unwrap()).is_err());
        assert!(DataState::restore(b"{\"version\":999}").is_err());
        let mut w = Workspace::new(PathBuf::from("unused.sqlite3"));
        w.input = "original".into();
        let first = w.active_id().to_owned();
        for n in 1..16 {
            w.create(&format!("instance {n}")).unwrap();
        }
        assert!(w.create("overflow").is_err());
        assert_eq!(w.instances.len(), 16);
        w.select(&first).unwrap();
        assert_eq!(w.input, "original");
        assert!(w.select("missing").is_err());
        assert_eq!(w.active_id(), first);
    }
}
