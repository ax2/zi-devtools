//! Explicit files contain definitions, never source tables or grants.
use super::{workflow::Definition, *};
pub(super) mod library;
const LIMIT: usize = 256 * 1024;
fn save_new(definition: &Definition, path: &std::path::Path) -> Result<()> {
    definition.validate()?;
    anyhow::ensure!(path.is_absolute(), "请选择绝对路径");
    let bytes = serde_json::to_vec_pretty(definition)?;
    anyhow::ensure!(bytes.len() <= LIMIT, "流程文件最多256 KiB");
    crate::local_files::save_new_moved(path, &bytes, &AtomicBool::new(false))
        .context("流程保存失败；已有文件不覆盖，当前步骤保留，请另选文件名")?;
    Ok(())
}
#[cfg(test)]
fn load(path: &std::path::Path) -> Result<Definition> {
    anyhow::ensure!(path.is_absolute(), "请选择绝对路径");
    let file = crate::local_files::open_regular(path, LIMIT).context("无法打开普通流程文件")?;
    let mut bytes = Vec::new();
    file.take((LIMIT + 1) as u64).read_to_end(&mut bytes)?;
    Definition::parse(&bytes)
}
enum Reply {
    Saved(PathBuf),
    Loaded(PathBuf, crate::workflow_document::Document),
    Listed(PathBuf, library::Listing),
}
#[derive(Default)]
pub(super) struct State {
    receiver: Option<Receiver<std::result::Result<Reply, String>>>,
    pub(super) job: Job,
    pub(super) review: Option<Definition>,
    pub(super) tool_review: Option<crate::text_flow::Definition>,
    pub(super) loaded_tool: Option<&'static str>,
    pub(super) message: String,
    pub(super) folder: Option<PathBuf>,
    pub(super) listing: Option<library::Listing>,
    pub(super) query: String,
    pub(super) remembered_folder: Option<PathBuf>,
    pub(super) folder_hydrated: bool,
    pub(super) folder_request: Option<Option<PathBuf>>,
    pub(super) memory_message: String,
    pub(super) listing_revision: u64,
    pub(super) loaded: Option<crate::preferences::SavedWorkflow>,
    #[cfg(feature = "ui-preview")]
    pub(super) memory_buttons: [Option<egui::Rect>; 2],
}
impl State {
    pub(super) fn list(&mut self, folder: PathBuf) -> Result<()> {
        anyhow::ensure!(!self.job.phase.active(), "请等待当前流程文件操作结束");
        anyhow::ensure!(self.review.is_none(), "请先确认或取消已读取的流程");
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        self.message.clear();
        self.listing = None;
        self.listing_revision = self.listing_revision.wrapping_add(1);
        self.folder = Some(folder.clone());
        self.job.begin();
        std::thread::spawn(move || {
            let _ = tx.send(
                library::scan(&folder)
                    .map(|listing| Reply::Listed(folder, listing))
                    .map_err(|e| format!("{e:#}")),
            );
        });
        Ok(())
    }
    pub(super) fn save(&mut self, definition: Definition, path: PathBuf) -> Result<()> {
        anyhow::ensure!(!self.job.phase.active(), "请等待当前流程文件操作结束");
        anyhow::ensure!(self.review.is_none(), "请先确认或取消已读取的流程");
        definition.validate()?;
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        self.message.clear();
        self.job.begin();
        std::thread::spawn(move || {
            let _ = tx.send(
                save_new(&definition, &path)
                    .map(|_| Reply::Saved(path))
                    .map_err(|e| format!("{e:#}")),
            );
        });
        Ok(())
    }
    pub(super) fn read(&mut self, path: PathBuf) -> Result<()> {
        anyhow::ensure!(!self.job.phase.active(), "请等待当前流程文件操作结束");
        anyhow::ensure!(self.review.is_none(), "请先确认或取消已读取的流程");
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        self.review = None;
        self.message.clear();
        self.job.begin();
        std::thread::spawn(move || {
            let _ = tx.send(
                crate::workflow_document::Document::load(&path)
                    .map(|definition| Reply::Loaded(path, definition))
                    .map_err(|e| format!("{e:#}")),
            );
        });
        Ok(())
    }
    pub(super) fn poll(&mut self) {
        let result = self.receiver.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err("流程文件任务意外结束".into())),
        });
        let Some(result) = result else {
            return;
        };
        self.receiver = None;
        match result {
            Ok(Reply::Saved(path)) => {
                self.job.finish(Phase::Done, "流程定义已保存；未运行");
                self.message = format!("流程已保存：{}；只含步骤与参数", path.display());
            }
            Ok(Reply::Loaded(path, document)) => {
                self.loaded = Some(document.metadata(&path));
                self.job
                    .finish(Phase::Done, "流程文件已读取，等待确认；未运行");
                match document {
                    crate::workflow_document::Document::Table(definition) => {
                        self.review = Some(definition);
                        self.loaded_tool = Some("pipeline");
                    }
                    crate::workflow_document::Document::Tool(definition) => {
                        self.tool_review = Some(definition);
                        self.loaded_tool = Some("text-flow");
                    }
                }
            }
            Ok(Reply::Listed(folder, listing)) => {
                self.listing_revision = self.listing_revision.wrapping_add(1);
                self.job.finish(Phase::Done, "流程文件夹已检查；未运行");
                self.folder = Some(folder);
                self.listing = Some(listing);
            }
            Err(error) => {
                self.job.finish(Phase::Failed, "流程文件操作失败");
                self.message = error;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::super::workflow::{ColumnOperation, Step};
    use super::*;
    #[test]
    fn saved_output_declaration_survives_real_file_roundtrip_without_execution() {
        let root = std::env::temp_dir().join(format!("zi-output-recipe-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("recipe.json");
        let mut def = definition();
        def.version = 3;
        def.output = Some(super::super::workflow::Output::Sqlite {
            version: 1,
            table: "教程资料".into(),
        });
        save_new(&def, &path).unwrap();
        let imported = load(&path).unwrap();
        assert_eq!(imported, def);
        assert_eq!(
            std::fs::read_dir(&root).unwrap().count(),
            1,
            "loading does not write a database"
        );
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    fn definition() -> Definition {
        Definition {
            output: None,
            version: 1,
            name: "清洗".into(),
            steps: vec![Step::Column {
                column: "名称".into(),
                operation: ColumnOperation::Trim,
                value: String::new(),
            }],
        }
    }
    #[test]
    fn background_listing_and_load_wait_for_review_without_running() {
        let folder = std::env::temp_dir().join(format!("zi-flow-list-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&folder).unwrap();
        let path = folder.join("recipe.json");
        save_new(&definition(), &path).unwrap();
        let mut state = State::default();
        state.list(folder.clone()).unwrap();
        assert!(state.listing.is_none());
        assert!(state.review.is_none());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while state.job.phase.active() {
            assert!(std::time::Instant::now() < deadline);
            state.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(state.listing.as_ref().unwrap().entries.len(), 1);
        state.read(path).unwrap();
        while state.job.phase.active() {
            assert!(std::time::Instant::now() < deadline);
            state.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(state.review.as_ref().unwrap(), &definition());
        assert!(state.list(folder.clone()).is_err());
        assert!(state.read(folder.join("recipe.json")).is_err());
        state.review = None;
        assert_eq!(load(&folder.join("recipe.json")).unwrap(), definition());
        std::fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn round_trip_reuses_new_inputs_without_overwriting() {
        let path = std::env::temp_dir().join(format!("zi-flow-{}.json", uuid::Uuid::new_v4()));
        let def = definition();
        save_new(&def, &path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded, def);
        assert!(save_new(&def, &path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        for name in ["first", "second"] {
            let data = Dataset::parse(&format!("名称\n {name} "), DataFormat::Csv, b',').unwrap();
            assert_eq!(
                loaded
                    .preview(&data, &AtomicBool::new(false))
                    .unwrap()
                    .result
                    .rows[0][0],
                name
            );
            assert_eq!(data.rows[0][0], format!(" {name} "));
        }
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn corrupt_oversize_and_relative_files_are_rejected() {
        let path =
            std::env::temp_dir().join(format!("zi-flow-invalid-{}.json", uuid::Uuid::new_v4()));
        for bytes in [b"not json".to_vec(), vec![b' '; LIMIT + 1]] {
            std::fs::write(&path, &bytes).unwrap();
            assert!(load(&path).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        assert!(save_new(&definition(), std::path::Path::new("relative.json")).is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn row_recipe_disk_roundtrip_reuses_new_tables_and_keeps_source() {
        use super::super::workflow::{Predicate, SortKey, Step};
        let path = std::env::temp_dir().join(format!("zi-row-flow-{}.json", uuid::Uuid::new_v4()));
        let definition = Definition {
            output: None,
            version: 2,
            name: "行处理".into(),
            steps: vec![
                Step::Filter {
                    column: "name".into(),
                    predicate: Predicate::IsNotNull,
                    value: String::new(),
                    case_sensitive: true,
                },
                Step::Sort {
                    keys: vec![SortKey {
                        column: "n".into(),
                        descending: true,
                    }],
                },
                Step::Deduplicate {
                    columns: vec!["name".into()],
                },
            ],
        };
        save_new(&definition, &path).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded, definition);
        for n in [3, 9] {
            let input = format!(
                r#"[{{"name":"Zi","n":1}},{{"name":"Zi","n":{n}}},{{"name":null,"n":99}}]"#
            );
            let source =
                super::super::Dataset::parse(&input, super::super::DataFormat::Json, b',').unwrap();
            let original = source.clone();
            let result = loaded
                .preview(&source, &std::sync::atomic::AtomicBool::new(false))
                .unwrap();
            assert_eq!(
                result.result.rows,
                vec![vec![serde_json::json!(n), serde_json::json!("Zi")]]
            );
            assert_eq!(source, original);
        }
        std::fs::remove_file(path).unwrap();
    }
}
