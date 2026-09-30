//! Explicit, bounded local document sources for future ingestion and indexing.
//! Only source paths and file fingerprints are persisted; document content is not.
use anyhow::{Context, Result, bail, ensure};
use eframe::egui;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::UNIX_EPOCH,
};

const MAX_SOURCES: usize = 32;
const MAX_FILES: usize = 500;
const MAX_VISITED: usize = 5000;
const MAX_DEPTH: usize = 8;
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 128 * 1024 * 1024;
const MAX_REGISTRY_BYTES: u64 = 512 * 1024;
const DEFAULT_EXCLUDES: &[&str] = &[
    ".git/",
    "node_modules/",
    "target/",
    ".venv/",
    "__pycache__/",
];

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    File,
    Directory,
}
impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::File => "文件",
            Self::Directory => "目录",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileVersion {
    pub relative: String,
    pub bytes: u64,
    pub modified_ms: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub scanned_at: String,
    pub files: Vec<FileVersion>,
    pub skipped: usize,
    pub total_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub kind: Kind,
    pub excludes: Vec<String>,
    pub added_at: String,
    pub snapshot: Option<Snapshot>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    schema: String,
    version: u32,
    sources: Vec<Source>,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            schema: "zi-devtools-knowledge-sources".into(),
            version: 1,
            sources: Vec::new(),
        }
    }
}

pub fn default_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".zi-devtools/knowledge-sources.json")
}

fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.chars().count() <= 80 && !name.chars().any(char::is_control)
}

fn parse_excludes(text: &str) -> Result<Vec<String>> {
    let mut rules = Vec::new();
    for raw in text.split([',', ';', '\n']) {
        let rule = raw.trim().replace('\\', "/");
        if rule.is_empty() {
            continue;
        }
        ensure!(
            rule.len() <= 120 && rules.len() < 32,
            "排除规则最多 32 条、每条 120 字符"
        );
        ensure!(
            !rule.starts_with('/') && !rule.contains(':') && !rule.chars().any(char::is_control),
            "排除规则必须是相对名称"
        );
        ensure!(
            !rule.split('/').any(|part| part == ".." || part == "."),
            "排除规则不能包含路径穿越"
        );
        ensure!(
            !rule
                .split('/')
                .enumerate()
                .any(|(i, part)| part.is_empty() && i > 0 && i + 1 < rule.split('/').count()),
            "排除规则不能包含空目录段"
        );
        ensure!(
            !rule.contains('*')
                || (rule.starts_with("*.")
                    && rule.len() > 2
                    && rule[2..].chars().all(|c| c.is_alphanumeric())),
            "仅支持 *.扩展名 通配规则"
        );
        if !rules.contains(&rule) {
            rules.push(rule);
        }
    }
    Ok(rules)
}

fn excluded(relative: &str, directory: bool, rules: &[String]) -> bool {
    let lower = relative.to_lowercase();
    let segments: Vec<_> = lower.split('/').collect();
    DEFAULT_EXCLUDES
        .iter()
        .map(|s| s.trim_end_matches('/'))
        .chain(
            rules
                .iter()
                .filter(|s| s.ends_with('/'))
                .map(|s| s.trim_end_matches('/')),
        )
        .any(|name| {
            let name = name.to_lowercase();
            if name.contains('/') {
                lower == name || lower.starts_with(&format!("{name}/"))
            } else {
                segments.contains(&name.as_str())
            }
        })
        || rules.iter().any(|rule| {
            let rule = rule.to_lowercase();
            if let Some(ext) = rule.strip_prefix("*.") {
                !directory && lower.ends_with(&format!(".{ext}"))
            } else if rule.ends_with('/') {
                false
            } else {
                lower == rule || lower.starts_with(&format!("{rule}/"))
            }
        })
}

fn supported(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        ["md", "markdown", "txt", "html", "htm", "pdf", "docx"]
            .contains(&e.to_ascii_lowercase().as_str())
    })
}

fn fingerprint(path: &Path, relative: String, cancel: &AtomicBool) -> Result<FileVersion> {
    let mut file = File::open(path)?;
    let before = file.metadata()?;
    ensure!(
        before.is_file() && before.len() <= MAX_FILE_BYTES,
        "文件超过 16 MiB 或不是普通文件"
    );
    let modified = before.modified().ok();
    let modified_ms = modified
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0);
    let mut hasher = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        ensure!(!cancel.load(Ordering::Relaxed), "扫描已取消");
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        ensure!(bytes <= MAX_FILE_BYTES, "文件在读取中超过 16 MiB");
        hasher.update(&buffer[..read]);
    }
    let after = file.metadata()?;
    ensure!(
        bytes == before.len() && after.len() == before.len() && after.modified().ok() == modified,
        "文件在扫描中发生变化，请重试"
    );
    Ok(FileVersion {
        relative,
        bytes,
        modified_ms,
        sha256: format!("{:x}", hasher.finalize()),
    })
}

fn validate_source(source: &Source) -> Result<()> {
    ensure!(uuid::Uuid::parse_str(&source.id).is_ok(), "知识源 ID 无效");
    ensure!(valid_name(&source.name), "知识源名称无效");
    ensure!(source.path.is_absolute(), "知识源路径必须是绝对路径");
    ensure!(source.excludes.len() <= 32, "排除规则太多");
    parse_excludes(&source.excludes.join("\n"))?;
    if let Some(snapshot) = &source.snapshot {
        ensure!(
            snapshot.files.len() <= MAX_FILES && snapshot.total_bytes <= MAX_TOTAL_BYTES,
            "扫描快照超限"
        );
        for file in &snapshot.files {
            ensure!(
                !file.relative.starts_with('/') && !file.relative.split('/').any(|s| s == ".."),
                "扫描快照路径无效"
            );
            ensure!(
                file.sha256.len() == 64 && file.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
                "扫描快照摘要无效"
            );
        }
    }
    Ok(())
}

pub fn add_source(
    registry: &mut Vec<Source>,
    path: &Path,
    name: &str,
    excludes: &str,
) -> Result<Source> {
    ensure!(registry.len() < MAX_SOURCES, "最多保存 32 个知识源");
    ensure!(valid_name(name), "名称应为 1–80 个可见字符");
    let rules = parse_excludes(excludes)?;
    let metadata = fs::symlink_metadata(path).context("无法读取所选路径")?;
    ensure!(!metadata.file_type().is_symlink(), "知识源不能是符号链接");
    let kind = if metadata.is_dir() {
        Kind::Directory
    } else if metadata.is_file() {
        Kind::File
    } else {
        bail!("仅支持普通文件或目录")
    };
    let canonical = path.canonicalize()?;
    ensure!(
        kind != Kind::Directory || canonical.parent().is_some(),
        "不能添加整盘根目录"
    );
    ensure!(
        kind != Kind::File || supported(&canonical),
        "仅支持 Markdown、TXT、HTML、PDF、DOCX 文件"
    );
    ensure!(
        !registry.iter().any(|s| s.path == canonical),
        "这个路径已添加"
    );
    let source = Source {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.trim().into(),
        path: canonical,
        kind,
        excludes: rules,
        added_at: chrono::Utc::now().to_rfc3339(),
        snapshot: None,
    };
    registry.push(source.clone());
    Ok(source)
}

pub fn scan(source: &Source, cancel: &AtomicBool) -> Result<Snapshot> {
    validate_source(source)?;
    let root_meta = fs::symlink_metadata(&source.path).context("知识源不存在或无法读取")?;
    ensure!(
        !root_meta.file_type().is_symlink(),
        "知识源变为符号链接，请重新选择"
    );
    ensure!(
        (source.kind == Kind::File && root_meta.is_file())
            || (source.kind == Kind::Directory && root_meta.is_dir()),
        "知识源类型已变化"
    );
    let canonical = source.path.canonicalize()?;
    ensure!(canonical == source.path, "知识源路径已变化，请重新添加");
    let mut files = Vec::new();
    let mut skipped = 0;
    let mut visited = 0;
    let mut total_bytes = 0_u64;
    let mut stack = vec![(source.path.clone(), 0_usize)];
    while let Some((path, depth)) = stack.pop() {
        ensure!(!cancel.load(Ordering::Relaxed), "扫描已取消");
        visited += 1;
        ensure!(
            visited <= MAX_VISITED,
            "目录条目超过 5000 个，请缩小范围或增加排除规则"
        );
        let meta = fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() {
            skipped += 1;
            continue;
        }
        let relative = if source.kind == Kind::File {
            path.file_name()
                .and_then(|n| n.to_str())
                .context("文件名不是 Unicode")?
                .to_owned()
        } else {
            path.strip_prefix(&source.path)?
                .to_str()
                .context("相对路径不是 Unicode")?
                .replace('\\', "/")
        };
        if path != source.path && excluded(&relative, meta.is_dir(), &source.excludes) {
            skipped += 1;
            continue;
        }
        if path != source.path && !path.canonicalize()?.starts_with(&source.path) {
            skipped += 1;
            continue;
        }
        if meta.is_dir() {
            ensure!(depth < MAX_DEPTH, "目录深度超过 8 层，请缩小范围");
            let mut children = Vec::new();
            for entry in fs::read_dir(&path)? {
                ensure!(!cancel.load(Ordering::Relaxed), "扫描已取消");
                ensure!(
                    visited + stack.len() + children.len() < MAX_VISITED,
                    "目录条目超过 5000 个，请缩小范围或增加排除规则"
                );
                children.push(entry?.path());
            }
            children.sort();
            for child in children.into_iter().rev() {
                stack.push((child, depth + 1));
            }
        } else if meta.is_file() && supported(&path) {
            ensure!(files.len() < MAX_FILES, "文档超过 500 个，请缩小范围");
            ensure!(meta.len() <= MAX_FILE_BYTES, "文档 {relative} 超过 16 MiB");
            total_bytes = total_bytes.saturating_add(meta.len());
            ensure!(
                total_bytes <= MAX_TOTAL_BYTES,
                "文档总量超过 128 MiB，请缩小范围"
            );
            files.push(fingerprint(&path, relative, cancel)?);
        } else {
            skipped += 1;
        }
    }
    files.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(Snapshot {
        scanned_at: chrono::Utc::now().to_rfc3339(),
        files,
        skipped,
        total_bytes,
    })
}

pub fn changes(previous: Option<&Snapshot>, current: &Snapshot) -> (usize, usize, usize) {
    let Some(previous) = previous else {
        return (current.files.len(), 0, 0);
    };
    let old: BTreeMap<&str, &str> = previous
        .files
        .iter()
        .map(|f| (f.relative.as_str(), f.sha256.as_str()))
        .collect();
    let new: BTreeMap<&str, &str> = current
        .files
        .iter()
        .map(|f| (f.relative.as_str(), f.sha256.as_str()))
        .collect();
    let added = current
        .files
        .iter()
        .filter(|f| !old.contains_key(f.relative.as_str()))
        .count();
    let changed = current
        .files
        .iter()
        .filter(|f| {
            old.get(f.relative.as_str())
                .is_some_and(|old_hash| *old_hash != f.sha256.as_str())
        })
        .count();
    let removed = previous
        .files
        .iter()
        .filter(|f| !new.contains_key(f.relative.as_str()))
        .count();
    (added, changed, removed)
}

fn load(path: &Path) -> Result<Registry> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Registry::default()),
        Err(e) => return Err(e.into()),
    };
    ensure!(
        meta.file_type().is_file() && meta.len() <= MAX_REGISTRY_BYTES,
        "知识源配置不是普通文件或超过 512 KiB"
    );
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_REGISTRY_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_REGISTRY_BYTES,
        "知识源配置超过 512 KiB"
    );
    let registry: Registry = serde_json::from_slice(&bytes).context("知识源配置格式无效")?;
    ensure!(
        registry.schema == "zi-devtools-knowledge-sources"
            && registry.version == 1
            && registry.sources.len() <= MAX_SOURCES,
        "知识源配置版本或数量无效"
    );
    let mut ids = HashSet::new();
    let mut paths = HashSet::new();
    for source in &registry.sources {
        validate_source(source)?;
        ensure!(
            ids.insert(&source.id) && paths.insert(&source.path),
            "知识源配置含重复项"
        );
    }
    Ok(registry)
}

fn save(path: &Path, sources: &[Source]) -> Result<()> {
    ensure!(sources.len() <= MAX_SOURCES, "知识源过多");
    match fs::symlink_metadata(path) {
        Ok(meta) => ensure!(
            meta.file_type().is_file(),
            "知识源配置不能是符号链接或特殊文件"
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let registry = Registry {
        sources: sources.to_vec(),
        ..Registry::default()
    };
    let bytes = serde_json::to_vec_pretty(&registry)?;
    ensure!(
        bytes.len() as u64 <= MAX_REGISTRY_BYTES,
        "知识源配置超过 512 KiB"
    );
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.context("无法保存知识源配置")
}

enum Event {
    Scanned(String, Result<Snapshot, String>),
}

pub struct State {
    path: PathBuf,
    sources: Vec<Source>,
    input_path: String,
    input_name: String,
    excludes: String,
    edit_excludes: String,
    selected: Option<String>,
    running: Option<Receiver<Event>>,
    cancel: Arc<AtomicBool>,
    confirm_remove: Option<String>,
    message: String,
    locked: bool,
}
impl State {
    pub fn sources(&self) -> &[Source] {
        &self.sources
    }

    pub fn is_locked(&self) -> bool {
        self.locked
    }

    pub fn ensure_inbox(&mut self) -> Result<String> {
        ensure!(
            !self.locked && self.running.is_none(),
            "知识源正在使用或配置不可写"
        );
        let path = self
            .path
            .parent()
            .context("无法确定知识源配置目录")?
            .join("knowledge-inbox");
        fs::create_dir_all(&path).context("无法创建本机收集箱")?;
        ensure!(
            fs::symlink_metadata(&path)?.file_type().is_dir(),
            "收集箱路径必须是普通目录，不能是链接"
        );
        let canonical = path.canonicalize()?;
        if let Some(source) = self.sources.iter().find(|source| source.path == canonical) {
            ensure!(
                source.kind == Kind::Directory,
                "收集箱路径已被文件知识源占用"
            );
            return Ok(source.id.clone());
        }
        let old_len = self.sources.len();
        let source = add_source(&mut self.sources, &canonical, "Zi 收集箱", "")?;
        if let Err(error) = self.persist() {
            self.sources.truncate(old_len);
            return Err(error);
        }
        self.selected = Some(source.id.clone());
        Ok(source.id)
    }
    pub fn new(path: PathBuf) -> Self {
        let (sources, message, locked) = match load(&path) {
            Ok(registry) => (registry.sources, String::new(), false),
            Err(e) => (
                Vec::new(),
                format!("无法读取知识源配置：{e}；为避免覆盖，请检查配置文件"),
                true,
            ),
        };
        Self {
            path,
            sources,
            input_path: String::new(),
            input_name: String::new(),
            excludes: String::new(),
            edit_excludes: String::new(),
            selected: None,
            running: None,
            cancel: Arc::new(AtomicBool::new(false)),
            confirm_remove: None,
            message,
            locked,
        }
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self) {
        self.sources = vec![Source {
            id: uuid::Uuid::new_v4().to_string(),
            name: "技术笔记".into(),
            path: PathBuf::from(r"C:\Demo\Notes"),
            kind: Kind::Directory,
            excludes: vec!["drafts/".into(), "*.log".into()],
            added_at: "2026-09-30T00:00:00Z".into(),
            snapshot: Some(Snapshot {
                scanned_at: "2026-09-30T00:00:00Z".into(),
                skipped: 3,
                total_bytes: 22400,
                files: vec![
                    FileVersion {
                        relative: "README.md".into(),
                        bytes: 5400,
                        modified_ms: 0,
                        sha256: "a3".repeat(32),
                    },
                    FileVersion {
                        relative: "guides/rust.md".into(),
                        bytes: 17000,
                        modified_ms: 0,
                        sha256: "b7".repeat(32),
                    },
                ],
            }),
        }];
        self.selected = self.sources.first().map(|s| s.id.clone());
        self.input_path = r"C:\Demo\Reference".into();
        self.input_name = "参考资料".into();
        self.excludes = "drafts/, *.log".into();
        self.edit_excludes = "drafts/, *.log".into();
        self.message = "上次扫描：2 个文档，3 项跳过。内容只在本机读取。".into();
    }

    fn persist(&mut self) -> Result<()> {
        ensure!(!self.locked, "配置文件不可写；请先修复读取错误");
        save(&self.path, &self.sources)
    }

    pub fn poll(&mut self) {
        let event = self.running.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(event) => Some(event),
            Err(mpsc::TryRecvError::Disconnected) => Some(Event::Scanned(
                String::new(),
                Err("扫描任务意外结束".into()),
            )),
            Err(mpsc::TryRecvError::Empty) => None,
        });
        if let Some(Event::Scanned(id, result)) = event {
            self.running = None;
            match result {
                Ok(snapshot) => {
                    if let Some(source) = self.sources.iter_mut().find(|s| s.id == id) {
                        let (added, changed, removed) =
                            changes(source.snapshot.as_ref(), &snapshot);
                        let count = snapshot.files.len();
                        source.snapshot = Some(snapshot);
                        self.message = match self.persist() {
                            Ok(()) => format!(
                                "扫描完成：{count} 个文档；新增 {added}、变化 {changed}、移除 {removed}"
                            ),
                            Err(e) => format!("扫描完成但保存失败：{e}"),
                        };
                    }
                }
                Err(e) => self.message = e,
            }
        }
    }

    fn start_scan(&mut self, source: Source) {
        self.message = format!("正在扫描 {}…", source.name);
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let (tx, rx) = mpsc::channel();
        self.running = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(Event::Scanned(
                source.id.clone(),
                scan(&source, &cancel).map_err(|e| e.to_string()),
            ));
        });
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.poll();
        ui.heading("本地知识源");
        ui.label("明确添加文件或目录，再手动扫描。这里只建立可复查的来源与版本清单；可在“文档解析与分块”预览内容，或在“增量知识索引”手动建立本机索引。");
        ui.add_space(10.0);
        ui.group(|ui| {
            ui.strong("添加知识源");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.input_path).hint_text("本机文件或目录完整路径").desired_width(500.0));
                if ui.button("选文件…").clicked() && let Some(path) = rfd::FileDialog::new().pick_file() {
                    self.input_path = path.display().to_string();
                    if self.input_name.is_empty() { self.input_name = path.file_stem().unwrap_or_default().to_string_lossy().into_owned(); }
                }
                if ui.button("选目录…").clicked() && let Some(path) = rfd::FileDialog::new().pick_folder() {
                    self.input_path = path.display().to_string();
                    if self.input_name.is_empty() { self.input_name = path.file_name().unwrap_or_default().to_string_lossy().into_owned(); }
                }
            });
            ui.horizontal(|ui| {
                ui.label("名称");
                ui.add(egui::TextEdit::singleline(&mut self.input_name).desired_width(180.0));
                ui.label("排除");
                ui.add(egui::TextEdit::singleline(&mut self.excludes).hint_text("drafts/, *.log").desired_width(330.0));
                if ui.add_enabled(!self.locked && self.running.is_none(), egui::Button::new("添加")).clicked() {
                    let old = self.sources.len();
                    self.message = match add_source(&mut self.sources, Path::new(self.input_path.trim().trim_matches('"')), &self.input_name, &self.excludes) {
                        Ok(source) => match self.persist() {
                            Ok(()) => { self.selected = Some(source.id); self.edit_excludes = source.excludes.join(", "); self.input_path.clear(); self.input_name.clear(); "已添加，点击扫描以建立版本清单".into() },
                            Err(e) => { self.sources.truncate(old); e.to_string() },
                        },
                        Err(e) => e.to_string(),
                    };
                }
            });
            ui.small("默认排除 .git、node_modules、target、.venv、__pycache__；自定义规则支持目录名/、*.扩展名及相对路径。最多 32 个源。");
        });
        if self.running.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("扫描正在后台进行");
                if ui.button("取消扫描").clicked() {
                    self.cancel.store(true, Ordering::Relaxed);
                }
            });
        }
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        ui.separator();
        ui.strong(format!("已添加 {} 个知识源", self.sources.len()));
        let mut remove = None;
        let mut scan_request = None;
        let mut update_rules = None;
        for source in self.sources.clone() {
            let selected = self.selected.as_deref() == Some(source.id.as_str());
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(
                            selected,
                            format!("{} · {}", source.name, source.kind.label()),
                        )
                        .clicked()
                    {
                        self.selected = Some(source.id.clone());
                        self.edit_excludes = source.excludes.join(", ");
                    }
                    if ui
                        .add_enabled(self.running.is_none(), egui::Button::new("扫描/刷新"))
                        .clicked()
                    {
                        scan_request = Some(source.clone());
                    }
                    if ui
                        .add_enabled(
                            self.running.is_none() && !self.locked,
                            egui::Button::new("移除"),
                        )
                        .clicked()
                    {
                        self.confirm_remove = Some(source.id.clone());
                    }
                });
                ui.small(source.path.display().to_string());
                if selected {
                    ui.horizontal(|ui| {
                        ui.label("排除规则");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.edit_excludes)
                                .hint_text("drafts/, *.log")
                                .desired_width(360.0),
                        );
                        if ui
                            .add_enabled(
                                self.running.is_none() && !self.locked,
                                egui::Button::new("保存规则"),
                            )
                            .clicked()
                        {
                            update_rules = Some((source.id.clone(), self.edit_excludes.clone()));
                        }
                    });
                    if let Some(snapshot) = &source.snapshot {
                        ui.label(format!(
                            "上次扫描：{} · {} 个文档 · {} 字节 · 跳过 {} 项",
                            snapshot.scanned_at,
                            snapshot.files.len(),
                            snapshot.total_bytes,
                            snapshot.skipped
                        ));
                        for file in snapshot.files.iter().take(40) {
                            ui.monospace(format!(
                                "{}  ·  {} B  ·  {}…",
                                file.relative,
                                file.bytes,
                                &file.sha256[..12]
                            ));
                        }
                        if snapshot.files.len() > 40 {
                            ui.small("仅显示前 40 项；完整快照保存在本机配置中");
                        }
                    } else {
                        ui.small("尚未扫描；没有读取目录内容");
                    }
                }
                if self.confirm_remove.as_deref() == Some(source.id.as_str()) {
                    ui.horizontal(|ui| {
                        ui.label("仅从列表移除，不删除源文件。确定？");
                        if ui.button("确认移除").clicked() {
                            remove = Some(source.id.clone());
                        }
                        if ui.button("取消").clicked() {
                            self.confirm_remove = None;
                        }
                    });
                }
            });
        }
        if let Some(source) = scan_request {
            self.start_scan(source);
        }
        if let Some((id, text)) = update_rules {
            self.message = match parse_excludes(&text) {
                Ok(rules) => {
                    let old = self.sources.clone();
                    if let Some(source) = self.sources.iter_mut().find(|s| s.id == id) {
                        source.excludes = rules;
                        source.snapshot = None;
                    }
                    match self.persist() {
                        Ok(()) => "排除规则已保存；旧快照已失效，请重新扫描".into(),
                        Err(e) => {
                            self.sources = old;
                            e.to_string()
                        }
                    }
                }
                Err(e) => e.to_string(),
            };
        }
        if let Some(id) = remove {
            let old = self.sources.clone();
            self.sources.retain(|s| s.id != id);
            self.message = match self.persist() {
                Ok(()) => {
                    if self.selected.as_deref() == Some(id.as_str()) {
                        self.selected = None;
                    }
                    self.confirm_remove = None;
                    "已从知识源列表移除，源文件未删除".into()
                }
                Err(e) => {
                    self.sources = old;
                    e.to_string()
                }
            };
        }
        ui.small("扫描限制：最多 500 个文档、5000 个条目、8 层、单文件 16 MiB、总量 128 MiB；不跟随符号链接。仅保存路径和哈希，不保存正文。" );
    }
}

impl Drop for State {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_sources_scan_excludes_and_track_versions() {
        let root = std::env::temp_dir().join(format!("zi-knowledge-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("drafts")).unwrap();
        fs::create_dir_all(root.join("guides")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join("README.md"), "first").unwrap();
        fs::write(root.join("guides/a.txt"), "hello").unwrap();
        fs::write(root.join("drafts/private.md"), "secret").unwrap();
        fs::write(root.join("skip.log"), "skip").unwrap();
        fs::write(root.join(".git/private.md"), "hidden").unwrap();
        let mut sources = Vec::new();
        let source = add_source(&mut sources, &root, "Docs", "drafts/, *.log").unwrap();
        let one = scan(&source, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            one.files
                .iter()
                .map(|f| f.relative.as_str())
                .collect::<Vec<_>>(),
            ["README.md", "guides/a.txt"]
        );
        let config = root.join("sources.json");
        sources[0].snapshot = Some(one.clone());
        save(&config, &sources).unwrap();
        assert_eq!(
            load(&config).unwrap().sources[0]
                .snapshot
                .as_ref()
                .unwrap()
                .files
                .len(),
            2
        );
        fs::write(root.join("README.md"), "second").unwrap();
        fs::remove_file(root.join("guides/a.txt")).unwrap();
        fs::write(root.join("new.html"), "new").unwrap();
        let two = scan(&source, &AtomicBool::new(false)).unwrap();
        assert_eq!(changes(Some(&one), &two), (1, 1, 1));
        sources[0].snapshot = Some(two);
        save(&config, &sources).unwrap();
        assert_eq!(
            load(&config).unwrap().sources[0]
                .snapshot
                .as_ref()
                .unwrap()
                .files
                .len(),
            2
        );
        fs::write(&config, "invalid JSON").unwrap();
        let mut state = State::new(config);
        assert!(state.locked);
        assert!(state.persist().is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rejects_root_duplicate_bad_rules_and_cancel() {
        let root = std::env::temp_dir().join(format!("zi-knowledge-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("a.md"), "hello").unwrap();
        let mut sources = Vec::new();
        assert!(add_source(&mut sources, &root, "Docs", "../secret").is_err());
        let source = add_source(&mut sources, &root, "Docs", "").unwrap();
        assert!(add_source(&mut sources, &root, "Again", "").is_err());
        assert!(scan(&source, &AtomicBool::new(true)).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
