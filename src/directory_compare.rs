//! Bounded, read-only comparison of two explicitly selected directory trees.
use anyhow::{Context, Result, ensure};
use eframe::egui;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
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
const DISPLAY_ROWS: usize = 100;

#[derive(Clone, Copy)]
pub struct Limits {
    pub max_entries_per_side: u64,
    pub max_depth: usize,
    pub max_duration: Duration,
    pub max_hash_bytes: u64,
    pub max_pairs: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_entries_per_side: 50_000,
            max_depth: 24,
            max_duration: Duration::from_secs(45),
            max_hash_bytes: 2 * 1024 * 1024 * 1024,
            max_pairs: 10_000,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub left_entries: u64,
    pub right_entries: u64,
    pub pairs_seen: u64,
    pub attempted_hash_pairs: u64,
    pub hashed_pairs: u64,
    pub hash_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    File,
    Directory,
}

impl ItemKind {
    fn label(self) -> &'static str {
        match self {
            Self::File => "文件",
            Self::Directory => "目录",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Difference {
    OnlyLeft,
    OnlyRight,
    TypeMismatch,
    SizeMismatch,
    ContentMismatch,
    Unverified,
    Changed,
    ReadError,
}

impl Difference {
    fn label(self) -> &'static str {
        match self {
            Self::OnlyLeft => "仅左侧",
            Self::OnlyRight => "仅右侧",
            Self::TypeMismatch => "类型不同",
            Self::SizeMismatch => "大小不同",
            Self::ContentMismatch => "内容不同",
            Self::Unverified => "未校验内容",
            Self::Changed => "检查中变化",
            Self::ReadError => "读取失败",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Row {
    pub relative: PathBuf,
    pub difference: Difference,
    pub left_kind: Option<ItemKind>,
    pub right_kind: Option<ItemKind>,
    pub left_size: Option<u64>,
    pub right_size: Option<u64>,
    pub left_sha256: Option<String>,
    pub right_sha256: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Report {
    pub left_root: PathBuf,
    pub right_root: PathBuf,
    pub status: ScanStatus,
    pub progress: Progress,
    pub rows: Vec<Row>,
    pub same_files: u64,
    pub same_directories: u64,
    pub unassessed_one_side: u64,
    pub skipped_links: u64,
    pub skipped_depth: u64,
    pub read_errors: u64,
    pub changed_entries: u64,
    pub skipped_hash_pairs: u64,
    pub limit_reason: Option<&'static str>,
}

#[derive(Clone)]
struct Snapshot {
    path: PathBuf,
    kind: ItemKind,
    size: u64,
    modified: Option<SystemTime>,
}

struct SideScan {
    entries: BTreeMap<PathBuf, Snapshot>,
    complete: bool,
    skipped_links: u64,
    skipped_depth: u64,
    read_errors: u64,
    limit_reason: Option<&'static str>,
}

impl Default for SideScan {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            complete: true,
            skipped_links: 0,
            skipped_depth: 0,
            read_errors: 0,
            limit_reason: None,
        }
    }
}

fn validate_root(root: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(root).context("无法读取目录信息")?;
    ensure!(
        metadata.is_dir() && !is_link(&metadata),
        "请选择普通目录，不能是链接或重解析点"
    );
    Ok(())
}

fn scan_side(
    root: &Path,
    left: bool,
    cancel: &AtomicBool,
    started: Instant,
    limits: Limits,
    progress: &mut Progress,
    on_progress: &mut impl FnMut(&Progress),
) -> SideScan {
    let mut result = SideScan::default();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    'walk: while let Some((directory, depth)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            result.complete = false;
            result.limit_reason = Some("已取消");
            break;
        }
        if started.elapsed() >= limits.max_duration {
            result.complete = false;
            result.limit_reason = Some("超过检查时间上限");
            break;
        }
        match fs::symlink_metadata(&directory) {
            Ok(metadata) if metadata.is_dir() && !is_link(&metadata) => {}
            Ok(metadata) if is_link(&metadata) => {
                result.complete = false;
                result.skipped_links += 1;
                continue;
            }
            _ => {
                result.complete = false;
                result.read_errors += 1;
                continue;
            }
        }
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => {
                result.complete = false;
                result.read_errors += 1;
                continue;
            }
        };
        for entry in entries {
            if cancel.load(Ordering::Relaxed) {
                result.complete = false;
                result.limit_reason = Some("已取消");
                break 'walk;
            }
            if started.elapsed() >= limits.max_duration {
                result.complete = false;
                result.limit_reason = Some("超过检查时间上限");
                break 'walk;
            }
            let count = if left {
                &mut progress.left_entries
            } else {
                &mut progress.right_entries
            };
            if *count >= limits.max_entries_per_side {
                result.complete = false;
                result.limit_reason = Some("达到每侧目录条目上限");
                break 'walk;
            }
            *count += 1;
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    result.complete = false;
                    result.read_errors += 1;
                    continue;
                }
            };
            let path = entry.path();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(_) => {
                    result.complete = false;
                    result.read_errors += 1;
                    continue;
                }
            };
            if is_link(&metadata) {
                result.complete = false;
                result.skipped_links += 1;
                continue;
            }
            let kind = if metadata.is_dir() {
                if depth < limits.max_depth {
                    stack.push((path.clone(), depth + 1));
                } else {
                    result.complete = false;
                    result.skipped_depth += 1;
                }
                ItemKind::Directory
            } else if metadata.is_file() {
                ItemKind::File
            } else {
                result.complete = false;
                result.read_errors += 1;
                continue;
            };
            let modified = metadata.modified().ok();
            if kind == ItemKind::File && modified.is_none() {
                result.complete = false;
                result.read_errors += 1;
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .expect("descendant path")
                .to_path_buf();
            result.entries.insert(
                relative,
                Snapshot {
                    path,
                    kind,
                    size: metadata.len(),
                    modified,
                },
            );
            if (*count).is_multiple_of(128) {
                on_progress(progress);
            }
        }
    }
    on_progress(progress);
    result
}

fn same_snapshot(metadata: &Metadata, snap: &Snapshot) -> bool {
    snap.kind == ItemKind::File
        && metadata.is_file()
        && !is_link(metadata)
        && metadata.len() == snap.size
        && metadata.modified().ok() == snap.modified
}

fn still_current(snap: &Snapshot) -> bool {
    fs::symlink_metadata(&snap.path)
        .ok()
        .is_some_and(|metadata| {
            if snap.kind == ItemKind::File {
                same_snapshot(&metadata, snap)
            } else {
                metadata.is_dir() && !is_link(&metadata)
            }
        })
}

fn counterpart_absent(root: &Path, relative: &Path) -> std::io::Result<bool> {
    match fs::symlink_metadata(root.join(relative)) {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HashError {
    Cancelled,
    TimedOut,
    Changed,
    Read,
}

fn hash_file(
    snap: &Snapshot,
    cancel: &AtomicBool,
    started: Instant,
    limits: Limits,
    progress: &mut Progress,
    on_progress: &mut impl FnMut(&Progress),
) -> std::result::Result<String, HashError> {
    let before = fs::symlink_metadata(&snap.path).map_err(|_| HashError::Read)?;
    if !same_snapshot(&before, snap) {
        return Err(HashError::Changed);
    }
    let mut handle = File::open(&snap.path).map_err(|_| HashError::Read)?;
    if !same_snapshot(&handle.metadata().map_err(|_| HashError::Read)?, snap) {
        return Err(HashError::Changed);
    }
    let mut hasher = Sha256::new();
    let mut read_bytes = 0u64;
    let mut last_reported_mib = progress.hash_bytes / (1024 * 1024);
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
        progress.hash_bytes = progress.hash_bytes.saturating_add(count as u64);
        if read_bytes > snap.size || progress.hash_bytes > limits.max_hash_bytes {
            return Err(HashError::Changed);
        }
        hasher.update(&buffer[..count]);
        let mib = progress.hash_bytes / (1024 * 1024);
        if mib > last_reported_mib {
            last_reported_mib = mib;
            on_progress(progress);
        }
    }
    if read_bytes != snap.size {
        return Err(HashError::Changed);
    }
    let after = handle.metadata().map_err(|_| HashError::Read)?;
    let path_after = fs::symlink_metadata(&snap.path).map_err(|_| HashError::Read)?;
    if !same_snapshot(&after, snap) || !same_snapshot(&path_after, snap) {
        return Err(HashError::Changed);
    }
    on_progress(progress);
    Ok(format!("{:x}", hasher.finalize()))
}

fn row(
    relative: PathBuf,
    difference: Difference,
    left: Option<&Snapshot>,
    right: Option<&Snapshot>,
) -> Row {
    Row {
        relative,
        difference,
        left_kind: left.map(|snap| snap.kind),
        right_kind: right.map(|snap| snap.kind),
        left_size: left
            .filter(|snap| snap.kind == ItemKind::File)
            .map(|snap| snap.size),
        right_size: right
            .filter(|snap| snap.kind == ItemKind::File)
            .map(|snap| snap.size),
        left_sha256: None,
        right_sha256: None,
    }
}

pub fn compare(
    left_root: &Path,
    right_root: &Path,
    cancel: &AtomicBool,
    limits: Limits,
    mut on_progress: impl FnMut(&Progress),
) -> Result<Report> {
    validate_root(left_root).context("左侧目录无效")?;
    validate_root(right_root).context("右侧目录无效")?;
    ensure!(
        fs::canonicalize(left_root)? != fs::canonicalize(right_root)?,
        "请选择两个不同目录"
    );
    ensure!(
        limits.max_entries_per_side > 0
            && limits.max_pairs > 0
            && limits.max_duration > Duration::ZERO,
        "检查上限无效"
    );
    let started = Instant::now();
    let mut progress = Progress::default();
    let left = scan_side(
        left_root,
        true,
        cancel,
        started,
        limits,
        &mut progress,
        &mut on_progress,
    );
    let right = if cancel.load(Ordering::Relaxed) || started.elapsed() >= limits.max_duration {
        SideScan {
            complete: false,
            limit_reason: Some("未扫描右侧目录"),
            ..SideScan::default()
        }
    } else {
        scan_side(
            right_root,
            false,
            cancel,
            started,
            limits,
            &mut progress,
            &mut on_progress,
        )
    };
    let mut report = Report {
        left_root: left_root.to_path_buf(),
        right_root: right_root.to_path_buf(),
        status: ScanStatus::Complete,
        progress,
        rows: Vec::new(),
        same_files: 0,
        same_directories: 0,
        unassessed_one_side: 0,
        skipped_links: left.skipped_links + right.skipped_links,
        skipped_depth: left.skipped_depth + right.skipped_depth,
        read_errors: left.read_errors + right.read_errors,
        changed_entries: 0,
        skipped_hash_pairs: 0,
        limit_reason: left.limit_reason.or(right.limit_reason),
    };
    let all_paths: BTreeSet<_> = left
        .entries
        .keys()
        .chain(right.entries.keys())
        .cloned()
        .collect();
    let both_complete = left.complete && right.complete;
    let mut stopped = cancel.load(Ordering::Relaxed) || started.elapsed() >= limits.max_duration;
    for relative in all_paths {
        let a = left.entries.get(&relative);
        let b = right.entries.get(&relative);
        match (a, b) {
            (Some(a), Some(b)) => {
                report.progress.pairs_seen += 1;
                if !still_current(a) || !still_current(b) {
                    report.changed_entries += 1;
                    report
                        .rows
                        .push(row(relative, Difference::Changed, Some(a), Some(b)));
                } else if a.kind != b.kind {
                    report
                        .rows
                        .push(row(relative, Difference::TypeMismatch, Some(a), Some(b)));
                } else if a.kind == ItemKind::Directory {
                    report.same_directories += 1;
                } else if a.size != b.size {
                    report
                        .rows
                        .push(row(relative, Difference::SizeMismatch, Some(a), Some(b)));
                } else {
                    let mut entry = row(relative, Difference::Unverified, Some(a), Some(b));
                    let required = a.size.saturating_mul(2);
                    if stopped {
                        report.rows.push(entry);
                        continue;
                    }
                    if report.progress.attempted_hash_pairs >= limits.max_pairs
                        || required
                            > limits
                                .max_hash_bytes
                                .saturating_sub(report.progress.hash_bytes)
                    {
                        report.skipped_hash_pairs += 1;
                        report
                            .limit_reason
                            .get_or_insert("达到摘要配对数或读取字节预算");
                        report.rows.push(entry);
                        continue;
                    }
                    report.progress.attempted_hash_pairs += 1;
                    let left_hash = hash_file(
                        a,
                        cancel,
                        started,
                        limits,
                        &mut report.progress,
                        &mut on_progress,
                    );
                    match left_hash {
                        Ok(digest) => entry.left_sha256 = Some(digest),
                        Err(HashError::Changed) => {
                            entry.difference = Difference::Changed;
                            report.changed_entries += 1;
                        }
                        Err(HashError::Read) => {
                            entry.difference = Difference::ReadError;
                            report.read_errors += 1;
                        }
                        Err(HashError::Cancelled) => {
                            stopped = true;
                            report.status = ScanStatus::Cancelled;
                        }
                        Err(HashError::TimedOut) => {
                            stopped = true;
                            report.limit_reason = Some("超过检查时间上限");
                        }
                    }
                    if entry.left_sha256.is_none() {
                        report.rows.push(entry);
                        continue;
                    }
                    match hash_file(
                        b,
                        cancel,
                        started,
                        limits,
                        &mut report.progress,
                        &mut on_progress,
                    ) {
                        Ok(digest) => {
                            entry.right_sha256 = Some(digest);
                            report.progress.hashed_pairs += 1;
                            if entry.left_sha256 == entry.right_sha256 {
                                report.same_files += 1;
                                continue;
                            }
                            entry.difference = Difference::ContentMismatch;
                        }
                        Err(HashError::Changed) => {
                            entry.difference = Difference::Changed;
                            report.changed_entries += 1;
                        }
                        Err(HashError::Read) => {
                            entry.difference = Difference::ReadError;
                            report.read_errors += 1;
                        }
                        Err(HashError::Cancelled) => {
                            stopped = true;
                            report.status = ScanStatus::Cancelled;
                        }
                        Err(HashError::TimedOut) => {
                            stopped = true;
                            report.limit_reason = Some("超过检查时间上限");
                        }
                    }
                    report.rows.push(entry);
                }
            }
            (Some(a), None) => {
                if both_complete {
                    let difference = if !still_current(a) {
                        report.changed_entries += 1;
                        Difference::Changed
                    } else {
                        match counterpart_absent(right_root, &relative) {
                            Ok(true) => Difference::OnlyLeft,
                            Ok(false) => {
                                report.changed_entries += 1;
                                Difference::Changed
                            }
                            Err(_) => {
                                report.read_errors += 1;
                                Difference::ReadError
                            }
                        }
                    };
                    report.rows.push(row(relative, difference, Some(a), None));
                } else {
                    report.unassessed_one_side += 1;
                }
            }
            (None, Some(b)) => {
                if both_complete {
                    let difference = if !still_current(b) {
                        report.changed_entries += 1;
                        Difference::Changed
                    } else {
                        match counterpart_absent(left_root, &relative) {
                            Ok(true) => Difference::OnlyRight,
                            Ok(false) => {
                                report.changed_entries += 1;
                                Difference::Changed
                            }
                            Err(_) => {
                                report.read_errors += 1;
                                Difference::ReadError
                            }
                        }
                    };
                    report.rows.push(row(relative, difference, None, Some(b)));
                } else {
                    report.unassessed_one_side += 1;
                }
            }
            (None, None) => unreachable!(),
        }
    }
    if report.status != ScanStatus::Cancelled {
        if cancel.load(Ordering::Relaxed) {
            report.status = ScanStatus::Cancelled;
        } else if !both_complete
            || report.skipped_hash_pairs > 0
            || report.changed_entries > 0
            || report.read_errors > 0
            || stopped
        {
            report.status = ScanStatus::Partial;
        }
    }
    on_progress(&report.progress);
    Ok(report)
}

pub fn export_csv(report: &Report, destination: &Path) -> Result<()> {
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .context("无法创建新 CSV；已有文件不会被覆盖")?;
    let written = (|| -> Result<()> {
        let mut writer = csv::Writer::from_writer(file);
        writer.write_record([
            "relative_path",
            "difference",
            "left_type",
            "right_type",
            "left_size",
            "right_size",
            "left_sha256",
            "right_sha256",
            "coverage",
        ])?;
        for row in &report.rows {
            let relative = row.relative.to_string_lossy();
            let safe_relative = csv_safe_path(&relative);
            writer.write_record([
                safe_relative.as_str(),
                row.difference.label(),
                row.left_kind.map_or("", ItemKind::label),
                row.right_kind.map_or("", ItemKind::label),
                &row.left_size
                    .map_or(String::new(), |value| value.to_string()),
                &row.right_size
                    .map_or(String::new(), |value| value.to_string()),
                row.left_sha256.as_deref().unwrap_or(""),
                row.right_sha256.as_deref().unwrap_or(""),
                match report.status {
                    ScanStatus::Complete => "complete",
                    ScanStatus::Partial => "partial",
                    ScanStatus::Cancelled => "cancelled",
                },
            ])?;
        }
        writer.flush()?;
        Ok(())
    })();
    if written.is_err() {
        let _ = fs::remove_file(destination);
    }
    written
}

fn csv_safe_path(path: &str) -> String {
    if matches!(
        path.trim_start().chars().next(),
        Some('=' | '+' | '-' | '@')
    ) {
        format!("'{path}")
    } else {
        path.to_owned()
    }
}

pub struct State {
    left_path: String,
    right_path: String,
    filter: String,
    report: Option<Report>,
    message: String,
    receiver: Option<Receiver<Result<Report, String>>>,
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<Progress>>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            left_path: String::new(),
            right_path: String::new(),
            filter: String::new(),
            report: None,
            message: String::new(),
            receiver: None,
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(Mutex::new(Progress::default())),
        }
    }
}

impl State {
    fn poll(&mut self) {
        let Some(receiver) = &self.receiver else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(report)) => {
                self.message = match report.status {
                    ScanStatus::Complete => "对比完成".to_owned(),
                    ScanStatus::Partial => "对比未覆盖全部内容；请查看未判定项".to_owned(),
                    ScanStatus::Cancelled => "已取消；只显示已观察或已校验的结果".to_owned(),
                };
                self.report = Some(report);
                self.receiver = None;
            }
            Ok(Err(error)) => {
                self.message = error;
                self.receiver = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.message = "对比线程意外结束".into();
                self.receiver = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }

    fn start(&mut self) {
        let left = PathBuf::from(self.left_path.trim().trim_matches('"'));
        let right = PathBuf::from(self.right_path.trim().trim_matches('"'));
        self.report = None;
        self.message = "正在只读对比目录…".into();
        self.cancel = Arc::new(AtomicBool::new(false));
        self.progress = Arc::new(Mutex::new(Progress::default()));
        let cancel = self.cancel.clone();
        let progress = self.progress.clone();
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        std::thread::spawn(move || {
            let result = compare(&left, &right, &cancel, Limits::default(), |value| {
                if let Ok(mut current) = progress.lock() {
                    *current = value.clone();
                }
            })
            .map_err(|error| format!("对比失败：{error:#}"));
            let _ = sender.send(result);
        });
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self) {
        self.left_path = r"C:\Projects\sample-app".into();
        self.right_path = r"D:\Backups\sample-app".into();
        self.message = "合成目录预览，未读取本机文件".into();
        self.report = Some(Report {
            left_root: PathBuf::from(&self.left_path),
            right_root: PathBuf::from(&self.right_path),
            status: ScanStatus::Complete,
            progress: Progress {
                left_entries: 842,
                right_entries: 838,
                pairs_seen: 820,
                attempted_hash_pairs: 804,
                hashed_pairs: 804,
                hash_bytes: 640_000_000,
            },
            rows: vec![
                Row {
                    relative: PathBuf::from(r"src\config.toml"),
                    difference: Difference::ContentMismatch,
                    left_kind: Some(ItemKind::File),
                    right_kind: Some(ItemKind::File),
                    left_size: Some(1_850),
                    right_size: Some(1_850),
                    left_sha256: Some("a".repeat(64)),
                    right_sha256: Some("b".repeat(64)),
                },
                Row {
                    relative: PathBuf::from(r"assets\banner.png"),
                    difference: Difference::SizeMismatch,
                    left_kind: Some(ItemKind::File),
                    right_kind: Some(ItemKind::File),
                    left_size: Some(240_000),
                    right_size: Some(180_000),
                    left_sha256: None,
                    right_sha256: None,
                },
                Row {
                    relative: PathBuf::from(r"docs\release-notes.md"),
                    difference: Difference::OnlyLeft,
                    left_kind: Some(ItemKind::File),
                    right_kind: None,
                    left_size: Some(4_200),
                    right_size: None,
                    left_sha256: None,
                    right_sha256: None,
                },
                Row {
                    relative: PathBuf::from(r"archive\"),
                    difference: Difference::OnlyRight,
                    left_kind: None,
                    right_kind: Some(ItemKind::Directory),
                    left_size: None,
                    right_size: None,
                    left_sha256: None,
                    right_sha256: None,
                },
            ],
            same_files: 803,
            same_directories: 12,
            unassessed_one_side: 0,
            skipped_links: 0,
            skipped_depth: 0,
            read_errors: 0,
            changed_entries: 0,
            skipped_hash_pairs: 0,
            limit_reason: None,
        });
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.poll();
        ui.heading("目录内容对比");
        ui.label("选择两个本机目录，按相对路径、文件大小与 SHA-256 对照；只读，不同步或删除。");
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label("左侧");
            if ui
                .add(
                    egui::TextEdit::singleline(&mut self.left_path)
                        .hint_text("左侧目录")
                        .desired_width(540.0)
                        .interactive(self.receiver.is_none()),
                )
                .changed()
            {
                self.report = None;
            }
            if ui
                .add_enabled(self.receiver.is_none(), egui::Button::new("选择目录…"))
                .clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                self.left_path = path.display().to_string();
                self.report = None;
            }
        });
        ui.horizontal(|ui| {
            ui.label("右侧");
            if ui
                .add(
                    egui::TextEdit::singleline(&mut self.right_path)
                        .hint_text("右侧目录")
                        .desired_width(540.0)
                        .interactive(self.receiver.is_none()),
                )
                .changed()
            {
                self.report = None;
            }
            if ui
                .add_enabled(self.receiver.is_none(), egui::Button::new("选择目录…"))
                .clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                self.right_path = path.display().to_string();
                self.report = None;
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.receiver.is_none()
                        && !self.left_path.trim().is_empty()
                        && !self.right_path.trim().is_empty(),
                    egui::Button::new("开始对比"),
                )
                .clicked()
            {
                self.start();
            }
            if self.receiver.is_some() && ui.button("取消对比").clicked() {
                self.cancel.store(true, Ordering::Relaxed);
            }
            if self.receiver.is_some()
                && let Ok(progress) = self.progress.lock()
            {
                ui.label(format!(
                    "左 {} 项 · 右 {} 项 · 已校验 {} 对 / {} MiB",
                    progress.left_entries,
                    progress.right_entries,
                    progress.hashed_pairs,
                    progress.hash_bytes / (1024 * 1024)
                ));
                ui.ctx().request_repaint_after(Duration::from_millis(150));
            }
        });
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        let Some(report) = &self.report else { return };
        ui.add_space(10.0);
        ui.columns(3, |cols| {
            cols[0].group(|ui| {
                ui.weak("本次一致文件");
                ui.heading(report.same_files.to_string());
            });
            cols[1].group(|ui| {
                ui.weak("差异或未校验");
                ui.heading(report.rows.len().to_string());
            });
            cols[2].group(|ui| {
                ui.weak("未判定单侧项");
                ui.heading(report.unassessed_one_side.to_string());
            });
        });
        ui.label(format!(
            "左侧 {} 项、右侧 {} 项；另有 {} 个同路径目录。",
            report.progress.left_entries, report.progress.right_entries, report.same_directories
        ));
        if report.status != ScanStatus::Complete {
            ui.colored_label(ui.visuals().warn_fg_color, format!("部分结果：{}；链接 {}、深度跳过 {}、读取错误 {}、变化条目 {}、摘要预算跳过 {} 对。未完整扫描时，不断言单侧缺失。", report.limit_reason.unwrap_or("未覆盖全部内容"), report.skipped_links, report.skipped_depth, report.read_errors, report.changed_entries, report.skipped_hash_pairs));
        }
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("筛选相对路径或差异类型")
                    .desired_width(400.0),
            );
            if ui.button("导出差异 CSV…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_file_name("directory-differences.csv")
                    .save_file()
            {
                self.message = match export_csv(report, &path) {
                    Ok(()) => format!("差异清单已保存：{}", path.display()),
                    Err(error) => format!("导出失败：{error:#}"),
                };
            }
        });
        ui.separator();
        let query = self.filter.trim().to_lowercase();
        let mut matched = 0usize;
        for row in report.rows.iter().filter(|row| {
            query.is_empty()
                || format!("{} {}", row.relative.display(), row.difference.label())
                    .to_lowercase()
                    .contains(&query)
        }) {
            matched += 1;
            if matched > DISPLAY_ROWS {
                continue;
            }
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(row.difference.label()).strong());
                ui.monospace(row.relative.display().to_string());
                if let (Some(a), Some(b)) = (row.left_size, row.right_size) {
                    ui.weak(format!("{a} / {b} B"));
                }
            });
        }
        if matched == 0 {
            ui.weak("没有匹配的差异项。本次完全一致的文件不逐项显示。");
        } else if matched > DISPLAY_ROWS {
            ui.weak(format!(
                "当前筛选共 {matched} 项，仅显示前 {DISPLAY_ROWS} 项；导出包含全部已判定行。"
            ));
        }
        ui.add_space(8.0);
        ui.small("同大小与 SHA-256 相同只代表本次读取一致；文件之后仍可能变化。部分扫描、链接与读取错误会限制缺失判断。导出含相对路径与摘要，由你主动选择位置；工具不会改动任一目录。");
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

    fn roots() -> (PathBuf, PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("zi-dir-compare-{}", uuid::Uuid::new_v4()));
        let left = root.join("left");
        let right = root.join("right");
        fs::create_dir_all(&left).unwrap();
        fs::create_dir_all(&right).unwrap();
        (root, left, right)
    }

    #[test]
    fn distinguishes_content_size_type_and_one_sided_paths() {
        let (root, left, right) = roots();
        fs::write(left.join("same.txt"), b"same").unwrap();
        fs::write(right.join("same.txt"), b"same").unwrap();
        fs::write(left.join("content.txt"), b"one!").unwrap();
        fs::write(right.join("content.txt"), b"two!").unwrap();
        fs::write(left.join("size.txt"), b"longer").unwrap();
        fs::write(right.join("size.txt"), b"x").unwrap();
        fs::write(left.join("kind"), b"file").unwrap();
        fs::create_dir(right.join("kind")).unwrap();
        fs::create_dir(left.join("empty-left")).unwrap();
        fs::write(right.join("right-only"), b"r").unwrap();
        let report = compare(
            &left,
            &right,
            &AtomicBool::new(false),
            Limits::default(),
            |_| {},
        )
        .unwrap();
        assert_eq!(report.status, ScanStatus::Complete);
        assert_eq!(report.same_files, 1);
        assert_eq!(report.rows.len(), 5);
        for difference in [
            Difference::ContentMismatch,
            Difference::SizeMismatch,
            Difference::TypeMismatch,
            Difference::OnlyLeft,
            Difference::OnlyRight,
        ] {
            assert!(report.rows.iter().any(|row| row.difference == difference));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn partial_inventory_never_asserts_a_missing_counterpart() {
        let (root, left, right) = roots();
        fs::write(left.join("a"), b"a").unwrap();
        fs::write(left.join("b"), b"b").unwrap();
        fs::write(right.join("z"), b"z").unwrap();
        let report = compare(
            &left,
            &right,
            &AtomicBool::new(false),
            Limits {
                max_entries_per_side: 1,
                ..Limits::default()
            },
            |_| {},
        )
        .unwrap();
        assert_eq!(report.status, ScanStatus::Partial);
        assert!(report.unassessed_one_side > 0);
        assert!(
            !report
                .rows
                .iter()
                .any(|row| matches!(row.difference, Difference::OnlyLeft | Difference::OnlyRight))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn budget_change_cancel_and_csv_are_explicit() {
        let (root, left, right) = roots();
        fs::write(left.join("a.txt"), b"same").unwrap();
        fs::write(right.join("a.txt"), b"same").unwrap();
        let budgeted = compare(
            &left,
            &right,
            &AtomicBool::new(false),
            Limits {
                max_hash_bytes: 1,
                ..Limits::default()
            },
            |_| {},
        )
        .unwrap();
        assert_eq!(budgeted.status, ScanStatus::Partial);
        assert_eq!(budgeted.rows[0].difference, Difference::Unverified);
        let csv = root.join("diff.csv");
        export_csv(&budgeted, &csv).unwrap();
        assert!(export_csv(&budgeted, &csv).is_err());
        let mut reader = csv::Reader::from_path(&csv).unwrap();
        assert_eq!(reader.records().count(), 1);
        assert_eq!(csv_safe_path("=SUM(1,1).txt"), "'=SUM(1,1).txt");
        let mut changed = false;
        let modified = compare(
            &left,
            &right,
            &AtomicBool::new(false),
            Limits::default(),
            |progress| {
                if progress.pairs_seen == 0 && progress.right_entries > 0 && !changed {
                    fs::write(left.join("a.txt"), b"longer than before").unwrap();
                    changed = true;
                }
            },
        )
        .unwrap();
        assert_eq!(modified.status, ScanStatus::Partial);
        assert_eq!(modified.rows[0].difference, Difference::Changed);
        fs::write(left.join("a.txt"), vec![3u8; BUFFER_SIZE * 2]).unwrap();
        fs::write(right.join("a.txt"), vec![3u8; BUFFER_SIZE * 2]).unwrap();
        let cancel = AtomicBool::new(false);
        let cancelled = compare(&left, &right, &cancel, Limits::default(), |progress| {
            if progress.hash_bytes > 0 {
                cancel.store(true, Ordering::Relaxed);
            }
        })
        .unwrap();
        assert_eq!(cancelled.status, ScanStatus::Cancelled);
        fs::remove_dir_all(root).unwrap();
    }
}
