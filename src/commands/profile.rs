//! Explicit portable bindings, excluding tool inputs, global permissions and credentials.
use super::Bindings;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
};
const LIMIT: usize = 256 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    version: u32,
    bindings: Vec<Binding>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    command: String,
    sequence: String,
}
fn validate_id(id: &str) -> Result<()> {
    ensure!(
        id.len() <= 512
            && (matches!(
                id,
                "search" | "recorder:start" | "recorder:pause" | "recorder:stop"
            ) || (id.starts_with("open:")
                && id.len() > 5
                && id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b":/._-".contains(&c)))),
        "配置含无效指令标识"
    );
    Ok(())
}
pub fn parse(bytes: &[u8]) -> Result<Bindings> {
    ensure!(bytes.len() <= LIMIT, "快捷配置最多 256 KiB");
    let profile: Profile = serde_json::from_slice(bytes).context("无法解析快捷配置 JSON")?;
    ensure!(profile.version == 1, "不支持的快捷配置版本");
    ensure!(profile.bindings.len() <= 4096, "快捷配置最多 4096 项");
    let mut result = Bindings::new();
    for binding in profile.bindings {
        validate_id(&binding.command)?;
        let sequence = super::normalize_sequence(&binding.sequence).map_err(anyhow::Error::msg)?;
        ensure!(
            binding.command != "search" || sequence == "K",
            "K 保留给搜索"
        );
        ensure!(
            result.insert(binding.command, sequence).is_none(),
            "配置含重复指令，拒绝覆盖"
        );
    }
    Ok(result)
}
pub fn encode(bindings: &Bindings) -> Result<Vec<u8>> {
    let profile = Profile {
        version: 1,
        bindings: bindings
            .iter()
            .map(|(id, s)| Binding {
                command: id.clone(),
                sequence: s.clone(),
            })
            .collect(),
    };
    let bytes = serde_json::to_vec_pretty(&profile)?;
    parse(&bytes)?;
    Ok(bytes)
}
pub fn candidate(current: &Bindings, imported: &Bindings, merge: bool) -> Bindings {
    let mut result = if merge {
        current.clone()
    } else {
        Bindings::new()
    };
    result.extend(imported.clone());
    result
}
pub fn load(path: &Path) -> Result<Bindings> {
    ensure!(path.is_absolute(), "请选择绝对路径");
    let file = File::open(path).context("无法打开快捷配置")?;
    ensure!(file.metadata()?.is_file(), "请选择普通文件");
    let mut bytes = Vec::new();
    file.take((LIMIT + 1) as u64).read_to_end(&mut bytes)?;
    parse(&bytes)
}
pub fn save_new(bindings: &Bindings, path: &Path) -> Result<()> {
    ensure!(path.is_absolute(), "请选择绝对路径");
    let bytes = encode(bindings)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .context("已有文件不覆盖，请选择新文件名")?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .context("保存失败；可能留下不完整文件，请检查后另选新文件名")?;
    Ok(())
}
enum Reply {
    Loaded(Bindings),
    Saved(PathBuf),
}
#[derive(Default)]
pub struct Files {
    receiver: Option<Receiver<std::result::Result<Reply, String>>>,
    pub review: Option<Bindings>,
    pub message: String,
}
impl Files {
    pub fn busy(&self) -> bool {
        self.receiver.is_some()
    }
    pub fn pending(&self) -> bool {
        self.busy() || self.review.is_some()
    }
    fn start(&mut self, work: impl FnOnce() -> Result<Reply> + Send + 'static) -> Result<()> {
        ensure!(!self.pending(), "请先完成当前配置操作");
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("shortcut-profile".into())
            .spawn(move || {
                let _ = tx.send(work().map_err(|e| format!("{e:#}")));
            })?;
        self.receiver = Some(rx);
        self.message.clear();
        Ok(())
    }
    pub fn read(&mut self, path: PathBuf) -> Result<()> {
        self.start(move || load(&path).map(Reply::Loaded))
    }
    pub fn save(&mut self, bindings: Bindings, path: PathBuf) -> Result<()> {
        encode(&bindings)?;
        self.start(move || {
            save_new(&bindings, &path)?;
            Ok(Reply::Saved(path))
        })
    }
    pub fn poll(&mut self) {
        let result = self.receiver.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err("快捷配置任务意外结束".into())),
        });
        let Some(result) = result else {
            return;
        };
        self.receiver = None;
        match result {
            Ok(Reply::Loaded(bindings)) => self.review = Some(bindings),
            Ok(Reply::Saved(path)) => {
                self.message = format!("配置已另存：{}；未改变当前绑定", path.display())
            }
            Err(error) => self.message = error,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn profiles_are_strict_bounded_and_preserve_disabled_and_dormant_bindings() {
        let original = Bindings::from([
            ("open:json".into(), String::new()),
            ("open:plugin:sample/tool".into(), "Q A".into()),
        ]);
        assert_eq!(parse(&encode(&original).unwrap()).unwrap(), original);
        for invalid in [
            r#"{"version":2,"bindings":[]}"#,
            r#"{"version":1,"bindings":[],"secret":"no"}"#,
            r#"{"version":1,"bindings":[{"command":"open:a","sequence":"Q","extra":true}]}"#,
            r#"{"version":1,"bindings":[{"command":"open:a","sequence":"Q"},{"command":"open:a","sequence":"R"}]}"#,
            r#"{"version":1,"bindings":[{"command":"search","sequence":"Q"}]}"#,
            r#"{"version":1,"bindings":[{"command":"open:a","sequence":"A B C D E"}]}"#,
            r#"{"version":1,"bindings":[{"command":"run-script","sequence":"Q"}]}"#,
        ] {
            assert!(parse(invalid.as_bytes()).is_err(), "{invalid}");
        }
        assert!(parse(&vec![b' '; LIMIT + 1]).is_err());
        let imported = Bindings::from([("open:json".into(), "Q J".into())]);
        assert_eq!(candidate(&original, &imported, true).len(), 2);
        assert_eq!(candidate(&original, &imported, false), imported);
        assert_eq!(original["open:json"], "");
    }
    #[test]
    fn file_roundtrip_never_overwrites_and_async_review_does_not_activate() {
        let path = std::env::temp_dir().join(format!("zi-shortcuts-{}.json", uuid::Uuid::new_v4()));
        let bindings = Bindings::from([("open:json".into(), "Q J".into())]);
        save_new(&bindings, &path).unwrap();
        let bytes = fs::read(&path).unwrap();
        assert!(save_new(&Bindings::new(), &path).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let mut files = Files::default();
        files.read(path.clone()).unwrap();
        assert!(files.read(path.clone()).is_err());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while files.busy() {
            files.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(files.review.as_ref(), Some(&bindings));
        assert!(files.save(bindings, path.clone()).is_err());
        fs::remove_file(path).unwrap();
    }
}
