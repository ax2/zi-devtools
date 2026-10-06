//! Explicit capture policy, independent of encrypted history storage.
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapturePolicy {
    pub excluded_apps: Vec<String>,
    pub exclude_unknown: bool,
}
impl CapturePolicy {
    pub fn parse(input: &str, exclude_unknown: bool) -> Result<Self, String> {
        if input.len() > 32768 {
            return Err("规则输入超过32 KiB".into());
        }
        let mut apps = Vec::new();
        for line in input.lines().map(str::trim).filter(|s| !s.is_empty()) {
            if line.len() > 256
                || line.len() <= 4
                || !line.to_lowercase().ends_with(".exe")
                || line
                    .chars()
                    .any(|c| c.is_control() || c == ';' || "\\/:*?\"<>|".contains(c))
            {
                return Err("每行填写一个进程文件名（如example.exe），不填路径或通配符".into());
            }
            let name = line.to_lowercase();
            if name.len() > 256 {
                return Err("规范化后的进程文件名超过256字节".into());
            }
            if !apps.contains(&name) {
                apps.push(name);
            }
            if apps.len() > 64 {
                return Err("最多排除64个进程文件名".into());
            }
        }
        Ok(Self {
            excluded_apps: apps,
            exclude_unknown,
        })
    }
    pub fn normalized(&self) -> Result<Self, String> {
        if self.excluded_apps.len() > 64
            || self
                .excluded_apps
                .iter()
                .any(|name| name.trim().is_empty() || name.contains(['\n', '\r']))
        {
            return Err("规则文件中的每个进程项必须是单一文件名，最多64项".into());
        }
        Self::parse(&self.excluded_apps.join("\n"), self.exclude_unknown)
    }
    pub fn excludes(&self, source: &str) -> bool {
        if source == "来源未知" {
            self.exclude_unknown
        } else {
            self.excluded_apps.contains(&source.to_lowercase())
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    schema: u32,
    policy: CapturePolicy,
}
fn load(path: &Path) -> Result<CapturePolicy, String> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(CapturePolicy::default()),
        Err(_) => return Err("无法读取排除规则；采集尚未开启，请重试或明确应用新规则".into()),
    };
    let mut bytes = Vec::new();
    file.take(32769)
        .read_to_end(&mut bytes)
        .map_err(|_| "读取排除规则失败")?;
    if bytes.len() > 32768 {
        return Err("排除规则文件超过32 KiB，未应用".into());
    }
    let config: Config = serde_json::from_slice(&bytes).map_err(|_| "排除规则格式无效，未应用")?;
    if config.schema != 1 {
        return Err("排除规则版本不支持，未应用".into());
    }
    config.policy.normalized()
}
fn save(path: &Path, policy: &CapturePolicy) -> Result<(), String> {
    let next = policy.normalized()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| "无法创建规则目录")?;
    }
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let bytes = serde_json::to_vec_pretty(&Config {
            schema: 1,
            policy: next,
        })?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|_| "规则保存失败，原文件及当前采集规则保持不变".into())
}
pub(super) struct Editor {
    pub applied: CapturePolicy,
    pub input: String,
    pub unknown: bool,
    pub path: Option<PathBuf>,
    pub load_failed: bool,
    pub ready: bool,
    pub replace_confirm: bool,
    pub status: String,
}
impl Default for Editor {
    fn default() -> Self {
        Self {
            applied: CapturePolicy::default(),
            input: String::new(),
            unknown: false,
            path: None,
            load_failed: false,
            ready: true,
            replace_confirm: false,
            status: "当前无排除规则；保存须主动选择".into(),
        }
    }
}
impl Editor {
    pub fn new(path: Option<PathBuf>) -> Self {
        let mut editor = Self {
            path,
            ready: true,
            ..Default::default()
        };
        editor.reload();
        editor
    }
    pub fn reload(&mut self) {
        let result = self
            .path
            .as_deref()
            .map(load)
            .unwrap_or_else(|| Ok(CapturePolicy::default()));
        match result {
            Ok(policy) => {
                self.input = policy.excluded_apps.join("\n");
                self.unknown = policy.exclude_unknown;
                self.applied = policy;
                self.load_failed = false;
                self.ready = true;
                self.replace_confirm = false;
                self.status = "已读取规则；采集仍需手动开启".into();
            }
            Err(error) => {
                self.load_failed = true;
                self.ready = false;
                self.status = error;
            }
        }
    }
    pub fn dirty(&self) -> bool {
        self.input != self.applied.excluded_apps.join("\n")
            || self.unknown != self.applied.exclude_unknown
    }
    pub fn apply(&mut self, remember: bool) -> Result<(), String> {
        let next = CapturePolicy::parse(&self.input, self.unknown)?;
        if remember {
            if self.load_failed && !self.replace_confirm {
                return Err("请先确认替换无法读取的旧规则文件".into());
            }
            let path = self
                .path
                .as_deref()
                .ok_or("未配置规则保存路径，仅可本次应用")?;
            save(path, &next)?;
            self.load_failed = false;
            self.replace_confirm = false;
        }
        self.input = next.excluded_apps.join("\n");
        self.applied = next;
        self.ready = true;
        self.status = if remember {
            "规则已保存并应用，重启记住规则但不自动采集"
        } else {
            "规则仅本次应用；重启读取此前保存规则，未写入历史"
        }
        .into();
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filenames_are_normalized_bounded_and_unknown_is_explicit() {
        let p = CapturePolicy::parse(" Example.EXE\nexample.exe\n编辑器.exe\n", false).unwrap();
        assert_eq!(p.excluded_apps, ["example.exe", "编辑器.exe"]);
        assert!(p.excludes("EXAMPLE.exe"));
        assert!(!p.excludes("another.exe") && !p.excludes("来源未知"));
        assert!(CapturePolicy::parse("", true).unwrap().excludes("来源未知"));
        for bad in [
            "C:\\app.exe",
            "app*exe",
            "x.exe\0",
            "app.exe;other.exe",
            ".exe",
        ] {
            assert!(CapturePolicy::parse(bad, false).is_err(), "{bad:?}");
        }
        let names = (0..65)
            .map(|i| format!("fixture{i}.exe"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(CapturePolicy::parse(&names, false).is_err());
    }
    #[test]
    fn unsupported_schema_fields_and_malformed_entries_are_not_silently_applied() {
        let path = std::env::temp_dir().join(format!("zi-policy-{}.json", uuid::Uuid::new_v4()));
        for value in [
            serde_json::json!({"schema":99,"policy":{"excluded_apps":[],"exclude_unknown":false}}),
            serde_json::json!({"schema":1,"extra":true,"policy":{"excluded_apps":[],"exclude_unknown":false}}),
            serde_json::json!({"schema":1,"policy":{"excluded_apps":["a.exe\nb.exe"],"exclude_unknown":false}}),
            serde_json::json!({"schema":1,"policy":{"excluded_apps":[""],"exclude_unknown":false}}),
        ] {
            let original = serde_json::to_vec(&value).unwrap();
            std::fs::write(&path, &original).unwrap();
            let e = Editor::new(Some(path.clone()));
            assert!(!e.ready && e.load_failed);
            assert_eq!(std::fs::read(&path).unwrap(), original);
        }
        std::fs::remove_file(path).unwrap();
        let expanding = format!("{}{}.exe", "x".repeat(250), "İ");
        assert!(CapturePolicy::parse(&expanding, false).is_err());
    }
    #[test]
    fn session_apply_and_failed_save_keep_previous_file_and_policy() {
        let root = std::env::temp_dir().join(format!("zi-policy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("policy.json");
        let mut e = Editor::new(Some(path.clone()));
        e.input = "synthetic.exe".into();
        assert!(e.dirty());
        e.apply(false).unwrap();
        assert!(!path.exists() && !e.dirty());
        e.apply(true).unwrap();
        assert_eq!(Editor::new(Some(path.clone())).applied, e.applied);
        e.path = Some(root.clone());
        e.input = "next.exe".into();
        assert!(e.apply(true).is_err());
        assert!(e.applied.excludes("synthetic.exe") && e.dirty());
        assert!(load(&path).unwrap().excludes("synthetic.exe"));
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn corrupt_policy_blocks_start_and_requires_explicit_replacement() {
        let path = std::env::temp_dir().join(format!("zi-policy-{}.json", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"synthetic invalid config").unwrap();
        let original = std::fs::read(&path).unwrap();
        let mut e = Editor::new(Some(path.clone()));
        assert!(!e.ready && e.load_failed);
        e.input = "fixture.exe".into();
        assert!(e.apply(true).is_err());
        e.apply(false).unwrap();
        assert!(e.ready && e.load_failed);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        e.replace_confirm = true;
        e.apply(true).unwrap();
        assert!(!e.load_failed && load(&path).unwrap().excludes("fixture.exe"));
        std::fs::write(&path, vec![b'x'; 32769]).unwrap();
        assert!(load(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
