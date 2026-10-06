//! Read-only duplicate-content candidates, bounded by directory and hash budgets.
use anyhow::{Context, Result, ensure};
use eframe::egui;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, Metadata},
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant, SystemTime},
};

use crate::disk_inspector::{ScanStatus, is_link};

const BUFFER_SIZE: usize = 64 * 1024;
const DISPLAY_GROUPS: usize = 20;
const DISPLAY_FILES: usize = 12;
const COPY_FILES: usize = 256;

#[derive(Clone, Copy)]
pub struct Limits {
    pub max_entries: u64,
    pub max_depth: usize,
    pub max_duration: Duration,
    pub max_hash_bytes: u64,
    pub max_candidates: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_entries: 100_000,
            max_depth: 24,
            max_duration: Duration::from_secs(30),
            max_hash_bytes: 2 * 1024 * 1024 * 1024,
            max_candidates: 20_000,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub entries: u64,
    pub files: u64,
    pub candidates: u64,
    pub hashed_files: u64,
    pub hashed_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct DuplicateGroup {
    pub size: u64,
    pub sha256: String,
    pub files: Vec<PathBuf>,
}

impl DuplicateGroup {
    pub fn redundant_bytes(&self) -> u64 {
        self.size
            .saturating_mul(self.files.len().saturating_sub(1) as u64)
    }
}

#[derive(Clone, Debug)]
pub struct Report {
    pub root: PathBuf,
    pub status: ScanStatus,
    pub progress: Progress,
    pub groups: Vec<DuplicateGroup>,
    pub group_count: usize,
    pub redundant_bytes: u64,
    pub skipped_links: u64,
    pub skipped_depth: u64,
    pub skipped_budget: u64,
    pub read_errors: u64,
    pub changed_files: u64,
    pub limit_reason: Option<&'static str>,
}

#[derive(Clone)]
struct FileSnap {
    path: PathBuf,
    size: u64,
    modified: SystemTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HashError {
    Cancelled,
    TimedOut,
    Changed,
    Read,
}

fn same_file(metadata: &Metadata, file: &FileSnap) -> bool {
    metadata.is_file()
        && !is_link(metadata)
        && metadata.len() == file.size
        && metadata.modified().ok() == Some(file.modified)
}

fn hash_one(
    file: &FileSnap,
    cancel: &AtomicBool,
    started: Instant,
    limits: Limits,
    progress: &mut Progress,
    on_progress: &mut impl FnMut(&Progress),
) -> std::result::Result<[u8; 32], HashError> {
    let before = fs::symlink_metadata(&file.path).map_err(|_| HashError::Read)?;
    if !same_file(&before, file) {
        return Err(HashError::Changed);
    }
    let mut handle = File::open(&file.path).map_err(|_| HashError::Read)?;
    let opened = handle.metadata().map_err(|_| HashError::Read)?;
    if !same_file(&opened, file) {
        return Err(HashError::Changed);
    }
    let mut digest = Sha256::new();
    let mut read_bytes = 0u64;
    let mut buffer = [0u8; BUFFER_SIZE];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(HashError::Cancelled);
        }
        if started.elapsed() >= limits.max_duration {
            return Err(HashError::TimedOut);
        }
        let count = handle.read(&mut buffer).map_err(|_| HashError::Read)?;
        if count == 0 {
            break;
        }
        read_bytes = read_bytes.saturating_add(count as u64);
        progress.hashed_bytes = progress.hashed_bytes.saturating_add(count as u64);
        if read_bytes > file.size || progress.hashed_bytes > limits.max_hash_bytes {
            return Err(HashError::Changed);
        }
        digest.update(&buffer[..count]);
        if progress.hashed_bytes.is_multiple_of(1024 * 1024) {
            on_progress(progress);
        }
    }
    if read_bytes != file.size {
        return Err(HashError::Changed);
    }
    let after = handle.metadata().map_err(|_| HashError::Read)?;
    let path_after = fs::symlink_metadata(&file.path).map_err(|_| HashError::Read)?;
    if !same_file(&after, file) || !same_file(&path_after, file) {
        return Err(HashError::Changed);
    }
    progress.hashed_files += 1;
    on_progress(progress);
    Ok(digest.finalize().into())
}

pub fn find(
    root: &Path,
    cancel: &AtomicBool,
    limits: Limits,
    mut on_progress: impl FnMut(&Progress),
) -> Result<Report> {
    let root_meta = fs::symlink_metadata(root).context("无法读取目录信息")?;
    ensure!(
        root_meta.is_dir() && !is_link(&root_meta),
        "请选择普通目录，不能是链接或重解析点"
    );
    ensure!(
        limits.max_entries > 0 && limits.max_candidates > 0 && limits.max_duration > Duration::ZERO,
        "检查上限无效"
    );
    let started = Instant::now();
    let mut report = Report {
        root: root.to_path_buf(),
        status: ScanStatus::Complete,
        progress: Progress::default(),
        groups: Vec::new(),
        group_count: 0,
        redundant_bytes: 0,
        skipped_links: 0,
        skipped_depth: 0,
        skipped_budget: 0,
        read_errors: 0,
        changed_files: 0,
        limit_reason: None,
    };
    let mut by_size: BTreeMap<u64, Vec<FileSnap>> = BTreeMap::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    'inventory: while let Some((directory, depth)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            report.status = ScanStatus::Cancelled;
            break;
        }
        if started.elapsed() >= limits.max_duration {
            report.limit_reason = Some("超过检查时间上限");
            break;
        }
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => {
                report.read_errors += 1;
                continue;
            }
        };
        for entry in entries {
            if cancel.load(Ordering::Relaxed) {
                report.status = ScanStatus::Cancelled;
                break 'inventory;
            }
            if started.elapsed() >= limits.max_duration {
                report.limit_reason = Some("超过检查时间上限");
                break 'inventory;
            }
            if report.progress.entries >= limits.max_entries {
                report.limit_reason = Some("达到目录条目上限");
                break 'inventory;
            }
            report.progress.entries += 1;
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    report.read_errors += 1;
                    continue;
                }
            };
            let path = entry.path();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(_) => {
                    report.read_errors += 1;
                    continue;
                }
            };
            if is_link(&metadata) {
                report.skipped_links += 1;
                continue;
            }
            if metadata.is_dir() {
                if depth < limits.max_depth {
                    stack.push((path, depth + 1));
                } else {
                    report.skipped_depth += 1;
                }
            } else if metadata.is_file() {
                report.progress.files += 1;
                if let Ok(modified) = metadata.modified() {
                    by_size.entry(metadata.len()).or_default().push(FileSnap {
                        path,
                        size: metadata.len(),
                        modified,
                    });
                } else {
                    report.read_errors += 1;
                }
            } else {
                report.read_errors += 1;
            }
            if report.progress.entries.is_multiple_of(128) {
                on_progress(&report.progress);
            }
        }
    }
    report.progress.candidates = by_size
        .values()
        .filter(|files| files.len() > 1)
        .map(|files| files.len() as u64)
        .sum();
    on_progress(&report.progress);
    let mut hashed: BTreeMap<(u64, [u8; 32]), Vec<PathBuf>> = BTreeMap::new();
    let mut selected_candidates = 0u64;
    if report.status != ScanStatus::Cancelled && report.limit_reason != Some("超过检查时间上限")
    {
        for (size, files) in by_size.into_iter().rev() {
            if files.len() < 2 {
                continue;
            }
            if cancel.load(Ordering::Relaxed) {
                report.status = ScanStatus::Cancelled;
                break;
            }
            if started.elapsed() >= limits.max_duration {
                report.limit_reason = Some("超过检查时间上限");
                break;
            }
            let count = files.len() as u64;
            let required_bytes = size.saturating_mul(count);
            if selected_candidates.saturating_add(count) > limits.max_candidates
                || required_bytes
                    > limits
                        .max_hash_bytes
                        .saturating_sub(report.progress.hashed_bytes)
            {
                report.skipped_budget += count;
                report
                    .limit_reason
                    .get_or_insert("候选数量或读取字节预算不足");
                continue;
            }
            selected_candidates += count;
            for file in files {
                match hash_one(
                    &file,
                    cancel,
                    started,
                    limits,
                    &mut report.progress,
                    &mut on_progress,
                ) {
                    Ok(digest) => hashed.entry((size, digest)).or_default().push(file.path),
                    Err(HashError::Changed) => report.changed_files += 1,
                    Err(HashError::Read) => report.read_errors += 1,
                    Err(HashError::Cancelled) => {
                        report.status = ScanStatus::Cancelled;
                        break;
                    }
                    Err(HashError::TimedOut) => {
                        report.limit_reason = Some("超过检查时间上限");
                        break;
                    }
                }
            }
            if report.status == ScanStatus::Cancelled
                || report.limit_reason == Some("超过检查时间上限")
            {
                break;
            }
        }
    }
    report.groups = hashed
        .into_iter()
        .filter(|(_, files)| files.len() > 1)
        .map(|((size, digest), mut files)| {
            files.sort();
            DuplicateGroup {
                size,
                sha256: digest.iter().map(|byte| format!("{byte:02x}")).collect(),
                files,
            }
        })
        .collect();
    report.groups.sort_by(|a, b| {
        b.redundant_bytes()
            .cmp(&a.redundant_bytes())
            .then_with(|| a.sha256.cmp(&b.sha256))
    });
    report.group_count = report.groups.len();
    report.redundant_bytes = report.groups.iter().fold(0u64, |sum, group| {
        sum.saturating_add(group.redundant_bytes())
    });
    report.groups.truncate(DISPLAY_GROUPS);
    if report.status != ScanStatus::Cancelled
        && (report.limit_reason.is_some()
            || report.skipped_links > 0
            || report.skipped_depth > 0
            || report.skipped_budget > 0
            || report.read_errors > 0
            || report.changed_files > 0)
    {
        report.status = ScanStatus::Partial;
    }
    on_progress(&report.progress);
    Ok(report)
}

fn size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    for unit in ["B", "KiB", "MiB", "GiB", "TiB"] {
        if value < 1024.0 || unit == "TiB" {
            return format!("{value:.2} {unit}");
        }
        value /= 1024.0;
    }
    unreachable!()
}

pub struct State {
    path: String,
    report: Option<Report>,
    message: String,
    receiver: Option<Receiver<Result<Report, String>>>,
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<Progress>>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            path: String::new(),
            report: None,
            message: String::new(),
            receiver: None,
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(Mutex::new(Progress::default())),
        }
    }
}

impl State {
    pub(crate) fn background_active(&self) -> bool {
        self.receiver.is_some()
    }

    fn poll(&mut self) {
        let Some(receiver) = &self.receiver else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(report)) => {
                self.message = match report.status {
                    ScanStatus::Complete => "检查完成".to_owned(),
                    ScanStatus::Partial => "检查未覆盖全部内容；下方仅为已确认的候选".to_owned(),
                    ScanStatus::Cancelled => "已取消；下方仅为已确认的候选".to_owned(),
                };
                self.report = Some(report);
                self.receiver = None;
            }
            Ok(Err(error)) => {
                self.message = error;
                self.receiver = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.message = "检查线程意外结束".to_owned();
                self.receiver = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }

    fn start(&mut self) {
        let root = PathBuf::from(self.path.trim().trim_matches('"'));
        self.report = None;
        self.message = "正在检查文件…".to_owned();
        self.cancel = Arc::new(AtomicBool::new(false));
        self.progress = Arc::new(Mutex::new(Progress::default()));
        let cancel = self.cancel.clone();
        let progress = self.progress.clone();
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        std::thread::spawn(move || {
            let result = find(&root, &cancel, Limits::default(), |value| {
                if let Ok(mut current) = progress.lock() {
                    *current = value.clone();
                }
            })
            .map_err(|error| format!("检查失败：{error:#}"));
            let _ = sender.send(result);
        });
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self) {
        self.path = r"C:\Projects\sample-app".to_owned();
        self.message = "检查完成 · 合成预览，未读取本机文件".to_owned();
        self.report = Some(Report {
            root: PathBuf::from(&self.path),
            status: ScanStatus::Complete,
            progress: Progress {
                entries: 1_240,
                files: 1_200,
                candidates: 24,
                hashed_files: 24,
                hashed_bytes: 420_000_000,
            },
            groups: vec![
                DuplicateGroup {
                    size: 85_000_000,
                    sha256: "a".repeat(64),
                    files: vec![
                        PathBuf::from(r"assets\release-demo.mp4"),
                        PathBuf::from(r"backups\release-demo-copy.mp4"),
                        PathBuf::from(r"exports\release-demo.mp4"),
                    ],
                },
                DuplicateGroup {
                    size: 12_000_000,
                    sha256: "b".repeat(64),
                    files: vec![
                        PathBuf::from(r"docs\screenshots\cover.png"),
                        PathBuf::from(r"backups\cover.png"),
                    ],
                },
            ],
            group_count: 2,
            redundant_bytes: 182_000_000,
            skipped_links: 0,
            skipped_depth: 0,
            skipped_budget: 0,
            read_errors: 0,
            changed_files: 0,
            limit_reason: None,
        });
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.poll();
        ui.heading("重复文件检查");
        ui.label(
            "选择本机目录，先按大小筛选，再只读取候选文件计算 SHA-256；结果仅供判断，不自动删除。",
        );
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::TextEdit::singleline(&mut self.path)
                        .hint_text("本机目录路径")
                        .desired_width(580.0)
                        .interactive(self.receiver.is_none()),
                )
                .changed()
            {
                self.report = None;
                self.message.clear();
            }
            if ui
                .add_enabled(self.receiver.is_none(), egui::Button::new("选择目录…"))
                .clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                self.path = path.display().to_string();
                self.report = None;
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.receiver.is_none() && !self.path.trim().is_empty(),
                    egui::Button::new("开始检查"),
                )
                .clicked()
            {
                self.start();
            }
            if self.receiver.is_some() && ui.button("取消检查").clicked() {
                self.cancel.store(true, Ordering::Relaxed);
            }
            if self.receiver.is_some()
                && let Ok(progress) = self.progress.lock()
            {
                ui.label(format!(
                    "已扫描 {} 项 · 已计算 {} 个文件 / {}",
                    progress.entries,
                    progress.hashed_files,
                    size(progress.hashed_bytes)
                ));
                ui.ctx().request_repaint_after(Duration::from_millis(150));
            }
        });
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        let Some(report) = &self.report else { return };
        ui.add_space(12.0);
        ui.columns(3, |columns| {
            columns[0].group(|ui| {
                ui.weak("同大小候选");
                ui.heading(report.progress.candidates.to_string());
            });
            columns[1].group(|ui| {
                ui.weak("已读取内容");
                ui.heading(size(report.progress.hashed_bytes));
            });
            columns[2].group(|ui| {
                ui.weak("理论重复逻辑大小");
                ui.heading(size(report.redundant_bytes));
            });
        });
        ui.label(format!(
            "已扫描 {} 项、{} 个普通文件；确认 {} 组，显示前 {} 组。",
            report.progress.entries,
            report.progress.files,
            report.group_count,
            report.groups.len()
        ));
        if report.status != ScanStatus::Complete {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!(
                    "部分结果：{}；链接 {}、深度跳过 {}、预算跳过 {}、读取错误 {}、变化文件 {}。",
                    report.limit_reason.unwrap_or("未覆盖全部条目"),
                    report.skipped_links,
                    report.skipped_depth,
                    report.skipped_budget,
                    report.read_errors,
                    report.changed_files
                ),
            );
        }
        ui.separator();
        if report.groups.is_empty() {
            ui.weak("已检查范围内没有找到相同大小且 SHA-256 相同的文件组。");
        }
        for (index, group) in report.groups.iter().enumerate() {
            egui::CollapsingHeader::new(format!(
                "{}. {} 个文件 · 单个 {} · 理论重复 {}",
                index + 1,
                group.files.len(),
                size(group.size),
                size(group.redundant_bytes())
            ))
            .default_open(index == 0)
            .show(ui, |ui| {
                ui.monospace(format!("SHA-256  {}", group.sha256));
                for path in group.files.iter().take(DISPLAY_FILES) {
                    ui.label(
                        path.strip_prefix(&report.root)
                            .unwrap_or(path)
                            .display()
                            .to_string(),
                    );
                }
                if group.files.len() > DISPLAY_FILES {
                    ui.weak(format!(
                        "另有 {} 个路径未展开",
                        group.files.len() - DISPLAY_FILES
                    ));
                }
                if ui
                    .button(format!("复制路径（最多 {COPY_FILES} 条）"))
                    .clicked()
                {
                    ui.ctx().copy_text(
                        group
                            .files
                            .iter()
                            .take(COPY_FILES)
                            .map(|path| path.display().to_string())
                            .collect::<Vec<_>>()
                            .join("\n"),
                    );
                }
            });
        }
        ui.add_space(8.0);
        ui.small("大小与 SHA-256 相同只说明本次读取的内容候选一致；文件之后仍可能变化。理论重复逻辑大小不是实际可回收空间，硬链接可能共享存储。请自行核对用途，工具不会删除文件。");
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
    fn groups_equal_content_not_just_equal_size() {
        let root = std::env::temp_dir().join(format!("zi-duplicates-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("one")).unwrap();
        fs::create_dir_all(root.join("two")).unwrap();
        fs::write(root.join("one/a.txt"), b"same data").unwrap();
        fs::write(root.join("two/b.txt"), b"same data").unwrap();
        fs::write(root.join("two/c.txt"), b"different").unwrap();
        fs::write(root.join("zero-a"), b"").unwrap();
        fs::write(root.join("zero-b"), b"").unwrap();
        let report = find(&root, &AtomicBool::new(false), Limits::default(), |_| {}).unwrap();
        assert_eq!(report.status, ScanStatus::Complete);
        assert_eq!(report.progress.files, 5);
        assert_eq!(report.progress.candidates, 5);
        assert_eq!(report.group_count, 2);
        assert_eq!(report.redundant_bytes, 9);
        assert_eq!(report.groups[0].size, 9);
        assert_eq!(report.groups[0].files.len(), 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn caps_work_and_interrupts_an_active_hash() {
        let root =
            std::env::temp_dir().join(format!("zi-duplicate-bounds-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("a.bin"), vec![3u8; 2 * BUFFER_SIZE]).unwrap();
        fs::write(root.join("b.bin"), vec![3u8; 2 * BUFFER_SIZE]).unwrap();
        let budgeted = find(
            &root,
            &AtomicBool::new(false),
            Limits {
                max_hash_bytes: 1,
                ..Limits::default()
            },
            |_| {},
        )
        .unwrap();
        assert_eq!(budgeted.status, ScanStatus::Partial);
        assert_eq!(budgeted.skipped_budget, 2);
        let limited = find(
            &root,
            &AtomicBool::new(false),
            Limits {
                max_entries: 1,
                ..Limits::default()
            },
            |_| {},
        )
        .unwrap();
        assert_eq!(limited.status, ScanStatus::Partial);
        assert_eq!(limited.limit_reason, Some("达到目录条目上限"));
        let cancelled = AtomicBool::new(false);
        let interrupted = find(&root, &cancelled, Limits::default(), |progress| {
            if progress.hashed_bytes > 0 {
                cancelled.store(true, Ordering::Relaxed);
            }
        })
        .unwrap();
        assert_eq!(interrupted.status, ScanStatus::Cancelled);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn changed_candidate_is_excluded_and_result_is_partial() {
        let root =
            std::env::temp_dir().join(format!("zi-duplicate-change-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let changed = root.join("a.txt");
        fs::write(&changed, b"original").unwrap();
        fs::write(root.join("b.txt"), b"original").unwrap();
        let mut changed_once = false;
        let report = find(
            &root,
            &AtomicBool::new(false),
            Limits::default(),
            |progress| {
                if progress.candidates == 2 && progress.hashed_files == 0 && !changed_once {
                    fs::write(&changed, b"replacement longer than original").unwrap();
                    changed_once = true;
                }
            },
        )
        .unwrap();
        assert_eq!(report.status, ScanStatus::Partial);
        assert_eq!(report.changed_files, 1);
        assert_eq!(report.group_count, 0);
        fs::remove_dir_all(root).unwrap();
    }
}
