//! Explicit files contain definitions, never source tables or grants.
use super::{workflow::Definition, *};
const LIMIT: usize = 256 * 1024;
fn save_new(definition: &Definition, path: &std::path::Path) -> Result<()> {
    definition.validate()?;
    anyhow::ensure!(path.is_absolute(), "请选择绝对路径");
    let bytes = serde_json::to_vec_pretty(definition)?;
    anyhow::ensure!(bytes.len() <= LIMIT, "流程文件最多256 KiB");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .context("无法创建流程文件；已有文件不覆盖，请另选新文件名")?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .context("流程保存失败，目标可能留下不完整文件；请检查后另选新文件名")?;
    Ok(())
}
fn load(path: &std::path::Path) -> Result<Definition> {
    anyhow::ensure!(path.is_absolute(), "请选择绝对路径");
    let file = File::open(path).context("无法打开流程文件")?;
    anyhow::ensure!(file.metadata()?.is_file(), "请选择普通流程文件");
    let mut bytes = Vec::new();
    file.take((LIMIT + 1) as u64).read_to_end(&mut bytes)?;
    Definition::parse(&bytes)
}
enum Reply {
    Saved(PathBuf),
    Loaded(Definition),
}
#[derive(Default)]
pub(super) struct State {
    receiver: Option<Receiver<std::result::Result<Reply, String>>>,
    pub(super) job: Job,
    pub(super) review: Option<Definition>,
    pub(super) message: String,
}
impl State {
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
            let _ = tx.send(load(&path).map(Reply::Loaded).map_err(|e| format!("{e:#}")));
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
            Ok(Reply::Loaded(definition)) => {
                self.job
                    .finish(Phase::Done, "流程文件已读取，等待确认；未运行");
                self.review = Some(definition);
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
    fn definition() -> Definition {
        Definition {
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
}
