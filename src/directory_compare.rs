//! Bounded, read-only comparison of two explicitly selected directory trees.
use anyhow::{Context, Result, ensure};
use eframe::egui;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, Metadata},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant, SystemTime},
};

use crate::disk_inspector::{ScanStatus, is_link};

mod copy_progress;
mod pagination;
mod result_review;
mod retry;
mod tasks;
mod transfer;
use crate::tasks::Phase;
use copy_progress::{CopyProgress, Outcome};
use tasks::Operation;

const BUFFER_SIZE: usize = 64 * 1024;
const COPY_MAX_FILES: usize = 1_000;
const COPY_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopySide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyTargetState {
    Available,
    Exists,
    Unavailable,
}

#[derive(Clone, Debug)]
pub struct CopyPlanItem {
    pub relative: PathBuf,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub target_state: CopyTargetState,
}

#[derive(Clone, Debug)]
pub struct CopyPlan {
    pub side: CopySide,
    pub source_root: PathBuf,
    pub destination_root: PathBuf,
    pub items: Vec<CopyPlanItem>,
    /// Relative directories that must be created before publishing the selected files.
    pub new_directories: Vec<PathBuf>,
    pub total_bytes: u64,
    source_identity: DirectoryIdentity,
    destination_identity: DirectoryIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DirectoryIdentity(Arc<same_file::Handle>);

fn directory_identity(path: &Path) -> Result<DirectoryIdentity> {
    let metadata = fs::symlink_metadata(path).context("无法读取目标目录身份")?;
    ensure!(
        metadata.is_dir() && !is_link(&metadata),
        "目标目录不是普通目录"
    );
    Ok(DirectoryIdentity(Arc::new(
        same_file::Handle::from_path(path).context("无法打开目标目录以记录身份")?,
    )))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CopyOutcome {
    Copied(PathBuf),
    Conflict(PathBuf),
    Unavailable(PathBuf),
    Failed(PathBuf, String),
    Cancelled,
}

#[derive(Clone, Debug, Default)]
pub struct CopyExecution {
    pub outcomes: Vec<CopyOutcome>,
    pub copied_bytes: u64,
    pub cancelled: bool,
}

enum CopyTaskResult {
    Prepared(Result<CopyPlan, String>),
    Executed(CopyExecution),
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

fn validate_relative_file(relative: &Path) -> Result<()> {
    use std::path::Component;
    ensure!(!relative.as_os_str().is_empty(), "相对路径不能为空");
    for component in relative.components() {
        match component {
            Component::Normal(name) => crate::local_files::leaf(Path::new(name))?,
            _ => anyhow::bail!("只允许目录内的普通相对路径"),
        }
    }
    Ok(())
}

fn copyable_from(row: &Row, side: CopySide) -> bool {
    matches!(
        row.difference,
        Difference::OnlyLeft
            | Difference::OnlyRight
            | Difference::TypeMismatch
            | Difference::SizeMismatch
            | Difference::ContentMismatch
    ) && match side {
        CopySide::Left => row.left_kind == Some(ItemKind::File),
        CopySide::Right => row.right_kind == Some(ItemKind::File),
    }
}

fn select_visible_copyable_rows(
    selected: &mut BTreeSet<PathBuf>,
    visible_rows: &[&Row],
    side: CopySide,
    limit: usize,
) {
    for row in visible_rows {
        if copyable_from(row, side) && selected.len() < limit {
            selected.insert(row.relative.clone());
        }
    }
}

fn invalidate_copy_preview(plan: &mut Option<CopyPlan>, result: &mut Option<CopyExecution>) {
    *plan = None;
    *result = None;
}

fn can_execute_copy(plan: Option<&CopyPlan>, result: Option<&CopyExecution>, busy: bool) -> bool {
    !busy
        && result.is_none()
        && plan.is_some_and(|plan| {
            plan.items
                .iter()
                .any(|item| item.target_state == CopyTargetState::Available)
        })
}

fn change_copy_side(
    current: &mut CopySide,
    selected: &mut BTreeSet<PathBuf>,
    plan: &mut Option<CopyPlan>,
    result: &mut Option<CopyExecution>,
    next: CopySide,
) {
    if *current != next {
        *current = next;
        selected.clear();
        invalidate_copy_preview(plan, result);
    }
}

fn change_copy_destination(
    current: &mut String,
    plan: &mut Option<CopyPlan>,
    result: &mut Option<CopyExecution>,
    next: String,
) {
    if *current != next {
        *current = next;
        invalidate_copy_preview(plan, result);
    }
}

fn display_path_tail(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let parent = path
        .parent()
        .and_then(Path::file_name)
        .map(|value| value.to_string_lossy().into_owned());
    parent.map_or_else(|| name.clone(), |parent| format!("…\\{parent}\\{name}"))
}

fn inspect_regular_under(root: &Path, relative: &Path) -> Result<(PathBuf, Metadata)> {
    validate_relative_file(relative)?;
    let mut current = root.to_path_buf();
    let components: Vec<_> = relative.components().collect();
    for (index, component) in components.iter().enumerate() {
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current).context("源文件路径无法核实")?;
        ensure!(!is_link(&metadata), "源路径包含链接或重解析点");
        if index + 1 == components.len() {
            ensure!(metadata.is_file(), "所选来源不是普通文件");
            return Ok((current, metadata));
        }
        ensure!(metadata.is_dir(), "源路径中间项不是目录");
    }
    anyhow::bail!("源文件路径无效")
}

fn hash_path(path: &Path, expected: &Metadata, cancel: &AtomicBool) -> Result<String> {
    let before = fs::symlink_metadata(path)?;
    ensure!(
        same_file_metadata(&before, expected),
        "源文件在校验前已变化"
    );
    let mut file = File::open(path)?;
    ensure!(
        same_file_metadata(&file.metadata()?, expected),
        "源文件在打开时已变化"
    );
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; BUFFER_SIZE];
    loop {
        ensure!(!cancel.load(Ordering::Relaxed), "操作已取消");
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes = bytes.saturating_add(count as u64);
        ensure!(bytes <= expected.len(), "源文件读取期间大小发生变化");
        hash.update(&buffer[..count]);
    }
    ensure!(bytes == expected.len(), "源文件读取不完整");
    ensure!(
        same_file_metadata(&file.metadata()?, expected)
            && same_file_metadata(&fs::symlink_metadata(path)?, expected),
        "源文件在校验期间已变化"
    );
    Ok(format!("{:x}", hash.finalize()))
}

fn same_file_metadata(actual: &Metadata, expected: &Metadata) -> bool {
    actual.is_file()
        && !is_link(actual)
        && actual.len() == expected.len()
        && actual.modified().ok() == expected.modified().ok()
}

fn target_state(
    destination_root: &Path,
    relative: &Path,
) -> Result<(CopyTargetState, Vec<PathBuf>)> {
    let mut parent = destination_root.to_path_buf();
    let mut parent_relative = PathBuf::new();
    let mut missing = Vec::new();
    let mut missing_parent = false;
    let components: Vec<_> = relative.components().collect();
    for component in components.iter().take(components.len().saturating_sub(1)) {
        parent.push(component.as_os_str());
        parent_relative.push(component.as_os_str());
        if missing_parent {
            missing.push(parent_relative.clone());
            continue;
        }
        match fs::symlink_metadata(&parent) {
            Ok(metadata) if is_link(&metadata) => anyhow::bail!("目标路径包含链接或重解析点"),
            Ok(metadata) if !metadata.is_dir() => {
                return Ok((CopyTargetState::Unavailable, Vec::new()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing_parent = true;
                missing.push(parent_relative.clone());
            }
            Err(error) => return Err(error.into()),
        }
    }
    let target = destination_root.join(relative);
    match fs::symlink_metadata(target) {
        Ok(_) => Ok((CopyTargetState::Exists, missing)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok((CopyTargetState::Available, missing))
        }
        Err(error) => Err(error.into()),
    }
}

/// Prepare a bounded, immutable copy preview from selected rows in a complete report.
pub fn prepare_copy_plan(
    report: &Report,
    side: CopySide,
    selected: &[PathBuf],
    destination_root: &Path,
) -> Result<CopyPlan> {
    prepare_copy_plan_cancellable(
        report,
        side,
        selected,
        destination_root,
        &AtomicBool::new(false),
    )
}

pub fn prepare_copy_plan_cancellable(
    report: &Report,
    side: CopySide,
    selected: &[PathBuf],
    destination_root: &Path,
    cancel: &AtomicBool,
) -> Result<CopyPlan> {
    ensure!(
        report.status == ScanStatus::Complete,
        "仅完整比较结果可生成复制计划"
    );
    ensure!(
        !selected.is_empty() && selected.len() <= COPY_MAX_FILES,
        "选择文件数必须在 1 到 {COPY_MAX_FILES} 之间"
    );
    validate_root(destination_root).context("目标目录无效")?;
    let destination_root = fs::canonicalize(destination_root)?;
    let destination_identity = directory_identity(&destination_root)?;
    let source_root = fs::canonicalize(match side {
        CopySide::Left => &report.left_root,
        CopySide::Right => &report.right_root,
    })?;
    let source_identity = directory_identity(&source_root)?;
    ensure!(
        !destination_root.starts_with(&source_root),
        "目标目录不能位于来源目录内"
    );
    let mut unique = BTreeSet::new();
    let mut items = Vec::with_capacity(selected.len());
    let mut new_directories = BTreeSet::new();
    let mut total_bytes = 0u64;
    for relative in selected {
        ensure!(!cancel.load(Ordering::Relaxed), "操作已取消");
        validate_relative_file(relative)?;
        ensure!(unique.insert(relative.clone()), "选择列表中存在重复路径");
        let row = report
            .rows
            .iter()
            .find(|row| row.relative == *relative)
            .context("所选路径不属于本次比较报告")?;
        let eligible = matches!(
            row.difference,
            Difference::OnlyLeft
                | Difference::OnlyRight
                | Difference::TypeMismatch
                | Difference::SizeMismatch
                | Difference::ContentMismatch
        );
        let kind = match side {
            CopySide::Left => row.left_kind,
            CopySide::Right => row.right_kind,
        };
        ensure!(
            eligible && kind == Some(ItemKind::File),
            "该差异项在所选来源侧不是可复制的普通文件"
        );
        let (source, metadata) = inspect_regular_under(&source_root, relative)?;
        total_bytes = total_bytes
            .checked_add(metadata.len())
            .context("复制计划大小溢出")?;
        ensure!(total_bytes <= COPY_MAX_BYTES, "复制计划超过 2 GiB 上限");
        let sha256 = hash_path(&source, &metadata, cancel)?;
        let (target_state, missing) = target_state(&destination_root, relative)?;
        new_directories.extend(missing);
        items.push(CopyPlanItem {
            relative: relative.clone(),
            source,
            destination: destination_root.join(relative),
            bytes: metadata.len(),
            sha256,
            target_state,
        });
    }
    Ok(CopyPlan {
        side,
        source_root,
        destination_root,
        items,
        new_directories: new_directories.into_iter().collect(),
        total_bytes,
        source_identity,
        destination_identity,
    })
}

fn ensure_destination_parent(root: &Path, relative: &Path) -> Result<PathBuf> {
    validate_relative_file(relative)?;
    let mut parent = root.to_path_buf();
    let components: Vec<_> = relative.components().collect();
    for component in components.iter().take(components.len().saturating_sub(1)) {
        parent.push(component.as_os_str());
        match fs::symlink_metadata(&parent) {
            Ok(metadata) => ensure!(
                metadata.is_dir() && !is_link(&metadata),
                "目标父路径不是普通目录"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&parent).context("无法创建目标子目录")?
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(root.join(relative))
}

/// Execute a previewed plan with per-file atomic no-overwrite publication.
pub fn execute_copy_plan(plan: &CopyPlan, cancel: &AtomicBool) -> CopyExecution {
    execute_copy_plan_with_progress(plan, cancel, |_| {})
}

fn execute_copy_plan_with_progress(
    plan: &CopyPlan,
    cancel: &AtomicBool,
    mut notify: impl FnMut(CopyProgress),
) -> CopyExecution {
    let mut execution = CopyExecution::default();
    let mut progress = CopyProgress::new(plan.items.len());
    notify(progress);
    for item in &plan.items {
        if cancel.load(Ordering::Relaxed) {
            execution.cancelled = true;
            execution.outcomes.push(CopyOutcome::Cancelled);
            break;
        }
        progress.start_file(item.bytes);
        notify(progress);
        if item.target_state == CopyTargetState::Unavailable {
            execution
                .outcomes
                .push(CopyOutcome::Unavailable(item.relative.clone()));
            progress.finish_file(Outcome::Failed, item.bytes);
            notify(progress);
            continue;
        }
        if item.target_state == CopyTargetState::Exists
            || fs::symlink_metadata(&item.destination).is_ok()
        {
            execution
                .outcomes
                .push(CopyOutcome::Conflict(item.relative.clone()));
            progress.finish_file(Outcome::Conflict, item.bytes);
            notify(progress);
            continue;
        }
        let result = (|| -> Result<PathBuf> {
            validate_root(&plan.destination_root).context("预览后的目标目录已变化")?;
            ensure!(
                fs::canonicalize(&plan.destination_root)? == plan.destination_root,
                "预览后的目标目录路径已变化"
            );
            ensure!(
                directory_identity(&plan.destination_root)? == plan.destination_identity,
                "预览后的目标目录对象已变化"
            );
            validate_root(&plan.source_root).context("预览后的来源目录已变化")?;
            ensure!(
                fs::canonicalize(&plan.source_root)? == plan.source_root,
                "预览后的来源目录路径已变化"
            );
            ensure!(
                directory_identity(&plan.source_root)? == plan.source_identity,
                "预览后的来源目录对象已变化"
            );
            let destination = ensure_destination_parent(&plan.destination_root, &item.relative)?;
            let (source, before) = inspect_regular_under(&plan.source_root, &item.relative)?;
            ensure!(
                source == item.source && before.len() == item.bytes,
                "来源文件身份或大小已变化"
            );
            let mut copied = 0u64;
            let published = crate::local_files::save_new_with(&destination, cancel, |out| {
                let mut input = File::open(&source)?;
                ensure!(
                    same_file_metadata(&input.metadata()?, &before),
                    "来源文件在打开时已变化"
                );
                let mut hash = Sha256::new();
                let mut buffer = [0u8; BUFFER_SIZE];
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        anyhow::bail!("操作已取消");
                    }
                    let count = input.read(&mut buffer)?;
                    if count == 0 {
                        break;
                    }
                    copied = copied.saturating_add(count as u64);
                    ensure!(copied <= item.bytes, "来源文件复制时大小发生变化");
                    hash.update(&buffer[..count]);
                    out.write_all(&buffer[..count])?;
                    progress.staged(copied);
                    notify(progress);
                }
                ensure!(
                    copied == item.bytes && format!("{:x}", hash.finalize()) == item.sha256,
                    "来源内容与预览摘要不一致"
                );
                ensure!(
                    same_file_metadata(&input.metadata()?, &before)
                        && same_file_metadata(&fs::symlink_metadata(&source)?, &before),
                    "来源文件在复制期间已变化"
                );
                Ok(())
            })?;
            Ok(published)
        })();
        let outcome = match result {
            Ok(path) => {
                execution.copied_bytes = execution.copied_bytes.saturating_add(item.bytes);
                execution.outcomes.push(CopyOutcome::Copied(path));
                Outcome::Copied
            }
            Err(_error) if cancel.load(Ordering::Relaxed) => {
                execution.cancelled = true;
                execution.outcomes.push(CopyOutcome::Cancelled);
                break;
            }
            Err(_error) if fs::symlink_metadata(&item.destination).is_ok() => {
                execution
                    .outcomes
                    .push(CopyOutcome::Conflict(item.relative.clone()));
                Outcome::Conflict
            }
            Err(error) => {
                execution.outcomes.push(CopyOutcome::Failed(
                    item.relative.clone(),
                    format!("{error:#}"),
                ));
                Outcome::Failed
            }
        };
        progress.finish_file(outcome, item.bytes);
        notify(progress);
    }
    progress.clear_staging();
    notify(progress);
    execution
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
    job: crate::tasks::Job,
    operation: Operation,
    left_path: String,
    right_path: String,
    filter: String,
    result_page: usize,
    copy_preview_page: usize,
    directory_preview_page: usize,
    report: Option<Report>,
    message: String,
    receiver: Option<Receiver<Result<Report, String>>>,
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<Progress>>,
    selected: BTreeSet<PathBuf>,
    copy_side: CopySide,
    destination_path: String,
    copy_plan: Option<CopyPlan>,
    copy_result: Option<CopyExecution>,
    copy_review: Option<result_review::Review>,
    transfer: transfer::State,
    copy_receiver: Option<Receiver<CopyTaskResult>>,
    copy_cancel: Arc<AtomicBool>,
    copy_progress: Arc<Mutex<CopyProgress>>,
    #[cfg(feature = "ui-preview")]
    copy_preview_pause: bool,
    #[cfg(feature = "ui-preview")]
    copy_ui_positions: [Option<egui::Pos2>; 4],
}

impl Default for State {
    fn default() -> Self {
        Self {
            job: Default::default(),
            operation: Operation::Scan,
            left_path: String::new(),
            right_path: String::new(),
            filter: String::new(),
            result_page: 0,
            copy_preview_page: 0,
            directory_preview_page: 0,
            report: None,
            message: String::new(),
            receiver: None,
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(Mutex::new(Progress::default())),
            selected: BTreeSet::new(),
            copy_side: CopySide::Left,
            destination_path: String::new(),
            copy_plan: None,
            copy_result: None,
            copy_review: None,
            transfer: Default::default(),
            copy_receiver: None,
            copy_cancel: Arc::new(AtomicBool::new(false)),
            copy_progress: Arc::new(Mutex::new(CopyProgress::default())),
            #[cfg(feature = "ui-preview")]
            copy_preview_pause: false,
            #[cfg(feature = "ui-preview")]
            copy_ui_positions: [None; 4],
        }
    }
}

impl State {
    #[cfg(feature = "ui-preview")]
    pub(crate) fn preview_transfer_review_rows(&self) -> Option<usize> {
        self.transfer.review_rows()
    }
    pub(crate) fn take_workbench_transfer(
        &mut self,
    ) -> Option<(String, crate::workbench::Dataset)> {
        self.transfer.take()
    }
    pub(crate) fn complete_workbench_transfer(&mut self, result: Result<()>) {
        self.transfer.completed(result);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_copy_fixture(
        &mut self,
        left: &Path,
        right: &Path,
        destination: &Path,
    ) -> Result<()> {
        let report = compare(
            left,
            right,
            &AtomicBool::new(false),
            Limits::default(),
            |_| {},
        )?;
        self.left_path = left.display().to_string();
        self.right_path = right.display().to_string();
        self.destination_path = destination.display().to_string();
        self.report = Some(report);
        self.result_page = 0;
        self.selected.clear();
        self.copy_plan = None;
        self.copy_result = None;
        self.message = "临时目录交互验收；复制尚未执行".into();
        Ok(())
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_copy_position(&self, index: usize) -> Option<egui::Pos2> {
        self.copy_ui_positions.get(index).copied().flatten()
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_copy_state(&self, relative: &Path) -> (bool, bool, bool, bool) {
        let selected = self.selected.contains(relative);
        let plan_ready = self.copy_plan.is_some();
        let busy = self.copy_receiver.is_some();
        let copied = self.copy_result.as_ref().is_some_and(|result| {
            result
                .outcomes
                .iter()
                .any(|outcome| matches!(outcome, CopyOutcome::Copied(_)))
        });
        (selected, plan_ready, busy, copied)
    }

    pub(crate) fn background_active(&self) -> bool {
        self.receiver.is_some() || self.copy_receiver.is_some()
    }

    fn poll(&mut self) {
        let Some(receiver) = &self.receiver else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(report)) => {
                self.finish_task(
                    match report.status {
                        ScanStatus::Complete => Phase::Done,
                        ScanStatus::Partial => Phase::Failed,
                        ScanStatus::Cancelled => Phase::Cancelled,
                    },
                    match report.status {
                        ScanStatus::Complete => {
                            format!("比较完成：{} 个差异或未校验项", report.rows.len())
                        }
                        ScanStatus::Partial => "扫描覆盖不完整；打开目录工具查看限制原因".into(),
                        ScanStatus::Cancelled => "扫描已取消，保留已观察结果".into(),
                    },
                );
                self.message = match report.status {
                    ScanStatus::Complete => "对比完成".to_owned(),
                    ScanStatus::Partial => "对比未覆盖全部内容；请查看未判定项".to_owned(),
                    ScanStatus::Cancelled => "已取消；只显示已观察或已校验的结果".to_owned(),
                };
                self.report = Some(report);
                self.receiver = None;
            }
            Ok(Err(error)) => {
                self.finish_task(Phase::Failed, "目录对比失败；打开工具查看详情");
                self.message = error;
                self.receiver = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.finish_task(Phase::Failed, "目录对比线程意外结束");
                self.message = "对比线程意外结束".into();
                self.receiver = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }

    fn poll_copy(&mut self) {
        let Some(receiver) = &self.copy_receiver else {
            return;
        };
        match receiver.try_recv() {
            Ok(CopyTaskResult::Prepared(Ok(plan))) => {
                if self.copy_cancel.load(Ordering::Relaxed) {
                    self.finish_task(Phase::Cancelled, "预览已取消，未写入目标文件");
                    self.message = "复制预览已取消；未写入目标文件。".into();
                    self.copy_plan = None;
                    self.copy_receiver = None;
                    return;
                }
                self.finish_task(
                    Phase::Done,
                    format!(
                        "{} 个文件 · {} B，等待明确执行",
                        plan.items.len(),
                        plan.total_bytes
                    ),
                );
                self.message = format!(
                    "复制预览就绪：{} 个文件，共 {} B；尚未写入文件。",
                    plan.items.len(),
                    plan.total_bytes
                );
                self.copy_preview_page = 0;
                self.directory_preview_page = 0;
                self.copy_plan = Some(plan);
                self.copy_receiver = None;
            }
            Ok(CopyTaskResult::Prepared(Err(error))) => {
                let cancelled = self.copy_cancel.load(Ordering::Relaxed);
                self.finish_task(
                    if cancelled {
                        Phase::Cancelled
                    } else {
                        Phase::Failed
                    },
                    if cancelled {
                        "预览已取消，未写入目标文件"
                    } else {
                        "生成预览失败；打开工具查看详情"
                    },
                );
                self.message = if cancelled {
                    "复制预览已取消；未写入目标文件。".into()
                } else {
                    format!("生成复制预览失败：{error}")
                };
                self.copy_plan = None;
                self.copy_receiver = None;
            }
            Ok(CopyTaskResult::Executed(result)) => {
                let copied = result
                    .outcomes
                    .iter()
                    .filter(|outcome| matches!(outcome, CopyOutcome::Copied(_)))
                    .count();
                let conflicts = result
                    .outcomes
                    .iter()
                    .filter(|outcome| matches!(outcome, CopyOutcome::Conflict(_)))
                    .count();
                let failed = result
                    .outcomes
                    .iter()
                    .filter(|outcome| {
                        matches!(
                            outcome,
                            CopyOutcome::Failed(_, _) | CopyOutcome::Unavailable(_)
                        )
                    })
                    .count();
                self.message = format!(
                    "复制结束：成功 {copied}，冲突 {conflicts}，失败 {failed}，写入 {} B{}。",
                    result.copied_bytes,
                    if result.cancelled { "；已取消" } else { "" }
                );
                self.finish_task(
                    if result.cancelled {
                        Phase::Cancelled
                    } else if conflicts > 0 || failed > 0 {
                        Phase::Failed
                    } else {
                        Phase::Done
                    },
                    self.message.clone(),
                );
                self.copy_review = self
                    .copy_plan
                    .as_ref()
                    .map(|plan| result_review::Review::new(plan, &result));
                self.copy_result = Some(result);
                self.copy_receiver = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.finish_task(Phase::Failed, "复制线程意外结束；打开工具重新检查目标");
                self.copy_plan = None;
                self.message = "复制任务线程意外结束".into();
                self.copy_receiver = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }

    fn prepare_copy(&mut self, side: CopySide, selected: Vec<PathBuf>, destination: PathBuf) {
        if self.background_active() {
            return;
        }
        let Some(report) = self.report.clone() else {
            return;
        };
        self.copy_plan = None;
        self.copy_result = None;
        self.begin_task(Operation::Preview);
        self.message = "正在校验所选来源并生成复制预览…".into();
        self.copy_cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.copy_cancel.clone();
        let (sender, receiver) = mpsc::channel();
        self.copy_receiver = Some(receiver);
        std::thread::spawn(move || {
            let result =
                prepare_copy_plan_cancellable(&report, side, &selected, &destination, &cancel)
                    .map_err(|error| format!("{error:#}"));
            let _ = sender.send(CopyTaskResult::Prepared(result));
        });
    }

    fn execute_copy(&mut self) {
        if !can_execute_copy(
            self.copy_plan.as_ref(),
            self.copy_result.as_ref(),
            self.copy_receiver.is_some() || self.receiver.is_some(),
        ) {
            return;
        }
        let Some(plan) = self.copy_plan.clone() else {
            return;
        };
        self.begin_task(Operation::Copy);
        self.copy_progress = Arc::new(Mutex::new(CopyProgress::new(plan.items.len())));
        let progress = self.copy_progress.clone();
        #[cfg(feature = "ui-preview")]
        let pause_after_chunk = std::mem::take(&mut self.copy_preview_pause);
        self.copy_cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.copy_cancel.clone();
        let (sender, receiver) = mpsc::channel();
        self.copy_receiver = Some(receiver);
        self.message = "正在复制；每个文件会单独原子提交，已完成的文件会保留。".into();
        std::thread::spawn(move || {
            #[cfg(feature = "ui-preview")]
            let mut paused = false;
            let result = execute_copy_plan_with_progress(&plan, &cancel, |update| {
                if let Ok(mut current) = progress.lock() {
                    *current = update;
                }
                #[cfg(feature = "ui-preview")]
                if pause_after_chunk && !paused && update.current_bytes > 0 {
                    paused = true;
                    let end = Instant::now() + Duration::from_secs(20);
                    while !cancel.load(Ordering::Relaxed) && Instant::now() < end {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
            });
            let _ = sender.send(CopyTaskResult::Executed(result));
        });
    }

    fn start(&mut self) {
        if self.background_active() {
            return;
        }
        self.result_page = 0;
        self.begin_task(Operation::Scan);
        let left = PathBuf::from(self.left_path.trim().trim_matches('"'));
        let right = PathBuf::from(self.right_path.trim().trim_matches('"'));
        self.report = None;
        self.selected.clear();
        self.copy_plan = None;
        self.copy_result = None;
        self.message = "正在扫描并对比目录…".into();
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
        // The application loop owns polling so completion transitions are
        // observed by task notifications/history before this page renders.
        #[cfg(feature = "ui-preview")]
        {
            self.copy_ui_positions = [None; 4];
        }
        // ScrollArea can remember the previous content width. Keep this page
        // bounded by the visible viewport; rows must wrap instead of expanding it.
        let visible_width = (ui.clip_rect().right() - ui.next_widget_position().x - 4.0)
            .min(ui.available_width())
            .max(1.0);
        ui.set_width(visible_width);
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        ui.heading("目录内容对比");
        ui.label("选择两个本机目录，按相对路径、文件大小与 SHA-256 对照；文件变更仅在单独预览并确认后复制。");
        ui.add_space(10.0);
        let source_controls_enabled = self.receiver.is_none() && self.copy_receiver.is_none();
        ui.horizontal_wrapped(|ui| {
            ui.label("左侧");
            if ui
                .add(
                    egui::TextEdit::singleline(&mut self.left_path)
                        .hint_text("左侧目录")
                        .desired_width(
                            (ui.available_size_before_wrap().x - 120.0).clamp(80.0, 540.0),
                        )
                        .interactive(source_controls_enabled),
                )
                .on_hover_text(&self.left_path)
                .changed()
            {
                self.report = None;
                self.selected.clear();
                self.copy_plan = None;
            }
            if ui
                .add_enabled(source_controls_enabled, egui::Button::new("选择目录…"))
                .clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                self.left_path = path.display().to_string();
                self.report = None;
                self.selected.clear();
                self.copy_plan = None;
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("右侧");
            if ui
                .add(
                    egui::TextEdit::singleline(&mut self.right_path)
                        .hint_text("右侧目录")
                        .desired_width(
                            (ui.available_size_before_wrap().x - 120.0).clamp(80.0, 540.0),
                        )
                        .interactive(source_controls_enabled),
                )
                .on_hover_text(&self.right_path)
                .changed()
            {
                self.report = None;
                self.selected.clear();
                self.copy_plan = None;
            }
            if ui
                .add_enabled(source_controls_enabled, egui::Button::new("选择目录…"))
                .clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                self.right_path = path.display().to_string();
                self.report = None;
                self.selected.clear();
                self.copy_plan = None;
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    source_controls_enabled
                        && !self.left_path.trim().is_empty()
                        && !self.right_path.trim().is_empty(),
                    egui::Button::new("开始对比"),
                )
                .clicked()
            {
                self.start();
            }
            if self.receiver.is_some() && ui.button("取消对比").clicked() {
                self.cancel_current();
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
        if self.copy_receiver.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(150));
            ui.weak("复制预览或文件复制正在后台运行…");
            if matches!(self.operation, Operation::Copy)
                && let Ok(progress) = self.copy_progress.lock()
            {
                ui.add(egui::ProgressBar::new(progress.fraction()).text(format!(
                    "已处理 {} / {} 项",
                    progress.processed_files, progress.total_files
                )));
                ui.label(progress.summary());
            }
            let cancel_button = ui.button("取消任务");
            #[cfg(feature = "ui-preview")]
            ui.ctx().data_mut(|d| {
                d.insert_temp(
                    egui::Id::new("directory-copy-cancel"),
                    cancel_button.rect.intersect(ui.clip_rect()),
                )
            });
            if cancel_button.clicked() {
                self.cancel_current();
            }
        }
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        let Some(report) = &self.report else { return };
        ui.add_space(10.0);
        let metrics = [
            ("本次一致文件", report.same_files),
            ("差异或未校验", report.rows.len() as u64),
            ("未判定单侧项", report.unassessed_one_side),
        ];
        let columns = if ui.available_width() >= 550.0 { 3 } else { 1 };
        ui.columns(columns, |cols| {
            for (index, (label, value)) in metrics.into_iter().enumerate() {
                cols[index % columns].group(|ui| {
                    ui.weak(label);
                    ui.heading(value.to_string());
                });
            }
        });
        ui.label(format!(
            "左侧 {} 项、右侧 {} 项；另有 {} 个同路径目录。",
            report.progress.left_entries, report.progress.right_entries, report.same_directories
        ));
        if report.status != ScanStatus::Complete {
            ui.colored_label(ui.visuals().warn_fg_color, format!("部分结果：{}；链接 {}、深度跳过 {}、读取错误 {}、变化条目 {}、摘要预算跳过 {} 对。未完整扫描时，不断言单侧缺失。", report.limit_reason.unwrap_or("未覆盖全部内容"), report.skipped_links, report.skipped_depth, report.read_errors, report.changed_entries, report.skipped_hash_pairs));
        }
        ui.horizontal_wrapped(|ui| {
            let filter_response = ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("筛选相对路径或差异类型")
                    .desired_width((ui.available_width() - 175.0).clamp(80.0, 400.0)),
            );
            if filter_response.changed() {
                self.result_page = 0;
            }
            #[cfg(feature = "ui-preview")]
            ui.ctx().data_mut(|d| {
                d.insert_temp(egui::Id::new("directory-filter"), filter_response.rect)
            });
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
        let query = self.filter.trim().to_lowercase();
        let is_match = |row: &&Row| transfer::matches(row, &query);
        let matched = report.rows.iter().filter(is_match).count();
        let send = ui
            .push_id("directory-report-transfer-action", |ui| {
                ui.button(format!("发送筛选差异到数据工作台…（{matched}项）"))
            })
            .inner;
        #[cfg(feature = "ui-preview")]
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new("directory-transfer-report"), send.rect));
        if send.clicked() {
            self.transfer
                .prepare(transfer::Review::report(report, &query));
        }
        let result_range = pagination::controls(
            ui,
            &mut self.result_page,
            matched,
            "directory-results",
            "筛选结果",
        );
        let visible_rows: Vec<&Row> = report
            .rows
            .iter()
            .filter(is_match)
            .skip(result_range.start)
            .take(result_range.len())
            .collect();
        let copy_busy = self.copy_receiver.is_some();
        let mut prepare_request = None;
        let mut execute_request = false;
        let mut select_uncopied_request = false;
        let mut changed_destination = None;
        let mut edited_destination = false;
        ui.group(|ui| {
            ui.label("从本次差异中复制文件");
            ui.horizontal_wrapped(|ui| {
                let mut side = self.copy_side;
                ui.add_enabled_ui(!copy_busy, |ui| {
                    egui::ComboBox::from_id_salt("directory-copy-side")
                        .selected_text(match side { CopySide::Left => "来源：左侧", CopySide::Right => "来源：右侧" })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut side, CopySide::Left, "从左侧复制");
                            ui.selectable_value(&mut side, CopySide::Right, "从右侧复制");
                        });
                });
                change_copy_side(
                    &mut self.copy_side,
                    &mut self.selected,
                    &mut self.copy_plan,
                    &mut self.copy_result,
                    side,
                );
                let destination_response = ui.add_enabled(!copy_busy,
                    egui::TextEdit::singleline(&mut self.destination_path).hint_text("目标目录").desired_width((ui.available_size_before_wrap().x - 260.0).clamp(100.0, 420.0)));
                destination_response.clone().on_hover_text(&self.destination_path);
                if destination_response.changed() {
                    edited_destination = true;
                }
                if ui.add_enabled(!copy_busy, egui::Button::new("选择目标…")).clicked()
                    && let Some(path) = rfd::FileDialog::new().pick_folder()
                {
                    changed_destination = Some(path.display().to_string());
                }
                let preview_button = ui.add_enabled(report.status == ScanStatus::Complete && !self.selected.is_empty() && !self.destination_path.trim().is_empty() && self.copy_receiver.is_none(), egui::Button::new("预览复制计划…"));
                #[cfg(feature = "ui-preview")]
                { self.copy_ui_positions[1] = Some(preview_button.rect.center()); }
                #[cfg(feature = "ui-preview")]
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("directory-preview-plan"), preview_button.rect));
                if preview_button.clicked() {
                    prepare_request = Some((self.copy_side, self.selected.iter().cloned().collect::<Vec<_>>(), PathBuf::from(self.destination_path.trim().trim_matches('"'))));
                }
            });
            ui.small("仅完整比较报告可操作。支持差异中的普通文件；不会覆盖现有文件，也不会移动或删除来源。复制上限 1,000 个文件 / 2 GiB。勾选下方文件后，先生成预览，再明确执行。");
            let selectable_visible = visible_rows.iter().filter(|row| copyable_from(row, self.copy_side)).count();
            let selected_bytes = self.selected.iter().filter_map(|relative| {
                report.rows.iter().find(|row| row.relative == *relative).and_then(|row| match self.copy_side {
                    CopySide::Left => row.left_size,
                    CopySide::Right => row.right_size,
                })
            }).fold(0u64, u64::saturating_add);
            ui.horizontal_wrapped(|ui| {
                let select_button = ui.add_enabled(!copy_busy && selectable_visible > 0 && self.selected.len() < COPY_MAX_FILES,
                    egui::Button::new(format!("选择本页可复制项（{selectable_visible}）")));
                #[cfg(feature = "ui-preview")]
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("directory-select-visible"), select_button.rect));
                if select_button.clicked() {
                    select_visible_copyable_rows(&mut self.selected, &visible_rows, self.copy_side, COPY_MAX_FILES);
                    self.copy_plan = None;
                    self.copy_result = None;
                }
                if ui.add_enabled(!copy_busy && !self.selected.is_empty(), egui::Button::new("清空选择")).clicked() {
                    self.selected.clear();
                    self.copy_plan = None;
                    self.copy_result = None;
                }
                ui.weak(format!("已选 {} / {} 项 · {} B", self.selected.len(), COPY_MAX_FILES, selected_bytes));
                if selected_bytes > COPY_MAX_BYTES {
                    ui.colored_label(ui.visuals().warn_fg_color, "超过 2 GiB 计划上限");
                }
            });
            if let Some(plan) = &self.copy_plan {
                let consumed = self.copy_result.is_some();
                let plan_panel = egui::CollapsingHeader::new(if consumed { "上次执行计划（展开查看）" } else { "复制计划详情" })
                    .id_salt(("directory-copy-plan", consumed))
                    .default_open(!consumed)
                    .show(ui, |ui| {
                let side_label = match plan.side { CopySide::Left => "左侧", CopySide::Right => "右侧" };
                let plan_label = if self.copy_result.is_some() { "上次执行计划（需重新预览）" } else { "预览" };
                ui.strong(format!("{plan_label}：{} 个文件 · {} B · 来源{}", plan.items.len(), plan.total_bytes, side_label));
                ui.label(format!("目标目录：{}", display_path_tail(&plan.destination_root)))
                    .on_hover_text(plan.destination_root.display().to_string());
                if !plan.new_directories.is_empty() {
                    ui.small(format!("将新建 {} 个目标子目录：", plan.new_directories.len()));
                    let directories = pagination::controls(ui, &mut self.directory_preview_page, plan.new_directories.len(), "directory-copy-directories", "待建目录");
                    for relative in &plan.new_directories[directories] {
                        ui.monospace(format!("目录 · {}", relative.display()));
                    }
                } else {
                    ui.small("目标文件所需的目录均已存在；不会新建子目录。");
                }
                let files = pagination::controls(ui, &mut self.copy_preview_page, plan.items.len(), "directory-copy-files", "待复制文件");
                for item in &plan.items[files] {
                    let state = match item.target_state { CopyTargetState::Available => "可新建", CopyTargetState::Exists => "目标已存在，跳过", CopyTargetState::Unavailable => "目标路径不可用" };
                    ui.monospace(format!("{} · {} B · {} · SHA-256 {}", item.relative.display(), item.bytes, state, &item.sha256[..12]));
                }
                let execute_button = ui.add_enabled(
                    can_execute_copy(
                        self.copy_plan.as_ref(),
                        self.copy_result.as_ref(),
                        self.copy_receiver.is_some(),
                    ),
                    egui::Button::new("执行复制…"),
                ).on_hover_text("每份预览只能执行一次；如需重试，请重新生成复制预览。已成功复制的目标会显示为冲突并跳过。");
                #[cfg(feature = "ui-preview")]
                { self.copy_ui_positions[2] = Some(execute_button.rect.center()); }
                #[cfg(feature = "ui-preview")]
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("directory-execute-copy"), execute_button.rect));
                if execute_button.clicked() { execute_request = true; }
                    });
                #[cfg(feature = "ui-preview")]
                if consumed {
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("directory-consumed-plan-expanded"), plan_panel.body_response.is_some()));
                }
                #[cfg(not(feature = "ui-preview"))]
                let _ = plan_panel;
            }
            if let Some(result) = &self.copy_result {
                if let Some(plan) = &self.copy_plan {
                    let remaining = retry::remaining(plan, result).len();
                    let button = ui.add_enabled(!copy_busy && remaining > 0,
                        egui::Button::new(format!("仅选择未复制项（{remaining}）")))
                        .on_hover_text("排除已成功复制项，保留冲突、失败及取消后尚未处理的项目。只修改勾选；需重新预览并明确执行。");
                    #[cfg(feature = "ui-preview")]
                    { self.copy_ui_positions[3] = Some(button.rect.center()); }
                    select_uncopied_request = button.clicked();
                }
                if let Some(review) = &mut self.copy_review {
                    review.ui(ui, &mut self.transfer);
                } else {
                    ui.weak("执行结果缺少对应计划，请重新检查目标后生成新预览。");
                }
            }
        });
        if edited_destination {
            invalidate_copy_preview(&mut self.copy_plan, &mut self.copy_result);
        }
        if let Some(destination) = changed_destination {
            change_copy_destination(
                &mut self.destination_path,
                &mut self.copy_plan,
                &mut self.copy_result,
                destination,
            );
        }
        ui.separator();
        for row in &visible_rows {
            ui.horizontal_wrapped(|ui| {
                let selectable = copyable_from(row, self.copy_side);
                if selectable {
                    let mut selected = self.selected.contains(&row.relative);
                    let checkbox = ui
                        .add_enabled(
                            !copy_busy && (selected || self.selected.len() < COPY_MAX_FILES),
                            egui::Checkbox::new(&mut selected, ""),
                        )
                        .on_hover_text("最多选择 1,000 个文件；达到上限后仍可取消已有选择。");
                    #[cfg(feature = "ui-preview")]
                    {
                        self.copy_ui_positions[0].get_or_insert(checkbox.rect.center());
                    }
                    if checkbox.changed() {
                        if selected {
                            self.selected.insert(row.relative.clone());
                        } else {
                            self.selected.remove(&row.relative);
                        }
                        self.copy_plan = None;
                        self.copy_result = None;
                    }
                } else {
                    ui.add_space(20.0);
                }
                ui.label(egui::RichText::new(row.difference.label()).strong());
                if let (Some(a), Some(b)) = (row.left_size, row.right_size) {
                    ui.weak(format!("{a} / {b} B"));
                }
            });
            // A multiline path gets its own vertical row. Wrapping a long
            // monospace label inside a horizontal flow can shift continuation
            // lines left of the viewport in egui.
            ui.add(
                egui::Label::new(
                    egui::RichText::new(row.relative.display().to_string()).monospace(),
                )
                .wrap(),
            )
            .on_hover_text(row.relative.display().to_string());
            ui.add_space(4.0);
        }
        if matched == 0 {
            ui.weak("没有匹配的差异项。本次完全一致的文件不逐项显示。");
        } else {
            ui.weak("翻页与筛选保留已选项目；批量选择只加入当前页。导出包含全部已判定行。");
        }
        if let Some((side, selected, destination)) = prepare_request {
            self.prepare_copy(side, selected, destination);
        }
        if select_uncopied_request {
            self.select_uncopied();
        }
        if execute_request {
            self.execute_copy();
        }
        ui.add_space(8.0);
        ui.small("同大小与 SHA-256 相同只代表本次读取一致；文件之后仍可能变化。部分扫描、链接与读取错误会限制缺失判断。导出含相对路径与摘要，由你主动选择位置；仅在你确认执行复制后写入目标目录，不覆盖现有文件。");
        self.transfer.ui(ui);
        #[cfg(feature = "ui-preview")]
        ui.ctx().data_mut(|d| {
            d.insert_temp(
                egui::Id::new("directory-layout-bounds"),
                (ui.min_rect(), ui.clip_rect()),
            )
        });
    }
}

impl Drop for State {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.copy_cancel.store(true, Ordering::Relaxed);
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

    #[test]
    fn copy_plan_is_bounded_verified_and_never_overwrites() {
        let (root, left, right) = roots();
        fs::write(left.join("diff.txt"), b"left-content").unwrap();
        fs::write(right.join("diff.txt"), b"right-content").unwrap();
        fs::write(left.join("left-only.txt"), b"only-left").unwrap();
        let destination = root.join("destination");
        fs::create_dir(&destination).unwrap();
        let report = compare(
            &left,
            &right,
            &AtomicBool::new(false),
            Limits::default(),
            |_| {},
        )
        .unwrap();
        let diff = PathBuf::from("diff.txt");
        let only = PathBuf::from("left-only.txt");
        let plan = prepare_copy_plan(
            &report,
            CopySide::Left,
            &[diff.clone(), only.clone()],
            &destination,
        )
        .unwrap();
        assert_eq!(plan.items.len(), 2);
        assert_eq!(
            plan.total_bytes,
            b"left-content".len() as u64 + b"only-left".len() as u64
        );
        assert!(
            plan.items
                .iter()
                .all(|item| item.target_state == CopyTargetState::Available)
        );
        assert!(plan.new_directories.is_empty());
        let execution = execute_copy_plan(&plan, &AtomicBool::new(false));
        assert_eq!(
            execution
                .outcomes
                .iter()
                .filter(|outcome| matches!(outcome, CopyOutcome::Copied(_)))
                .count(),
            2
        );
        assert_eq!(
            fs::read(destination.join("diff.txt")).unwrap(),
            b"left-content"
        );
        assert_eq!(
            fs::read(destination.join("left-only.txt")).unwrap(),
            b"only-left"
        );
        assert!(
            prepare_copy_plan(
                &report,
                CopySide::Left,
                &[PathBuf::from("../escape")],
                &destination
            )
            .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn copy_preview_lists_deduplicated_destination_directories_that_execution_will_create() {
        let (root, left, right) = roots();
        fs::create_dir_all(left.join("nested/deeper")).unwrap();
        fs::create_dir_all(right.join("nested/deeper")).unwrap();
        fs::write(left.join("nested/deeper/a.txt"), b"a").unwrap();
        fs::write(left.join("nested/deeper/b.txt"), b"b").unwrap();
        fs::write(right.join("nested/deeper/a.txt"), b"different a").unwrap();
        fs::write(right.join("nested/deeper/b.txt"), b"different b").unwrap();
        let destination = root.join("destination");
        fs::create_dir(&destination).unwrap();
        let report = compare(
            &left,
            &right,
            &AtomicBool::new(false),
            Limits::default(),
            |_| {},
        )
        .unwrap();
        let plan = prepare_copy_plan(
            &report,
            CopySide::Left,
            &[
                PathBuf::from("nested/deeper/a.txt"),
                PathBuf::from("nested/deeper/b.txt"),
            ],
            &destination,
        )
        .unwrap();
        assert_eq!(
            plan.new_directories,
            [PathBuf::from("nested"), PathBuf::from("nested/deeper")]
        );
        assert!(!destination.join("nested").exists());
        let execution = execute_copy_plan(&plan, &AtomicBool::new(false));
        assert_eq!(
            execution
                .outcomes
                .iter()
                .filter(|outcome| matches!(outcome, CopyOutcome::Copied(_)))
                .count(),
            2
        );
        assert_eq!(
            fs::read(destination.join("nested/deeper/a.txt")).unwrap(),
            b"a"
        );
        assert_eq!(
            fs::read(destination.join("nested/deeper/b.txt")).unwrap(),
            b"b"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn changed_sources_and_late_conflicts_are_reported_per_file() {
        let (root, left, right) = roots();
        fs::write(left.join("a.txt"), b"a-left").unwrap();
        fs::write(right.join("a.txt"), b"a-right").unwrap();
        fs::write(left.join("b.txt"), b"b-left").unwrap();
        fs::write(right.join("b.txt"), b"b-right").unwrap();
        let destination = root.join("destination");
        fs::create_dir(&destination).unwrap();
        let report = compare(
            &left,
            &right,
            &AtomicBool::new(false),
            Limits::default(),
            |_| {},
        )
        .unwrap();
        let a = PathBuf::from("a.txt");
        let b = PathBuf::from("b.txt");
        let plan = prepare_copy_plan(
            &report,
            CopySide::Left,
            &[a.clone(), b.clone()],
            &destination,
        )
        .unwrap();
        fs::write(left.join("a.txt"), b"changed-after-preview").unwrap();
        fs::write(destination.join("b.txt"), b"another user's file").unwrap();
        let execution = execute_copy_plan(&plan, &AtomicBool::new(false));
        assert_eq!(execution.outcomes.len(), 2);
        assert!(
            execution
                .outcomes
                .iter()
                .any(|outcome| matches!(outcome, CopyOutcome::Failed(path, _) if path == &a))
        );
        assert!(
            execution
                .outcomes
                .iter()
                .any(|outcome| matches!(outcome, CopyOutcome::Conflict(path) if path == &b))
        );
        assert_eq!(
            fs::read(destination.join("b.txt")).unwrap(),
            b"another user's file"
        );
        assert!(!destination.join("a.txt").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn copy_rejects_destination_object_replaced_after_preview() {
        let (root, left, right) = roots();
        fs::write(left.join("a.txt"), b"left").unwrap();
        fs::write(right.join("a.txt"), b"right").unwrap();
        let destination = root.join("destination");
        fs::create_dir(&destination).unwrap();
        let report = compare(
            &left,
            &right,
            &AtomicBool::new(false),
            Limits::default(),
            |_| {},
        )
        .unwrap();
        let plan = prepare_copy_plan(
            &report,
            CopySide::Left,
            &[PathBuf::from("a.txt")],
            &destination,
        )
        .unwrap();

        fs::remove_dir(&destination).unwrap();
        fs::create_dir(&destination).unwrap();
        fs::write(
            destination.join("replacement-marker.txt"),
            b"replacement marker",
        )
        .unwrap();
        let execution = execute_copy_plan(&plan, &AtomicBool::new(false));

        assert!(matches!(
            execution.outcomes.as_slice(),
            [CopyOutcome::Failed(path, _)] if path == Path::new("a.txt")
        ));
        assert_eq!(
            fs::read(destination.join("replacement-marker.txt")).unwrap(),
            b"replacement marker"
        );
        assert!(!destination.join("a.txt").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn copy_rejects_source_object_replaced_after_preview() {
        let (root, left, right) = roots();
        fs::write(left.join("a.txt"), b"left version").unwrap();
        fs::write(right.join("a.txt"), b"right version").unwrap();
        let destination = root.join("destination");
        fs::create_dir(&destination).unwrap();
        let report = compare(
            &left,
            &right,
            &AtomicBool::new(false),
            Limits::default(),
            |_| {},
        )
        .unwrap();
        let plan = prepare_copy_plan(
            &report,
            CopySide::Left,
            &[PathBuf::from("a.txt")],
            &destination,
        )
        .unwrap();

        fs::remove_dir_all(&left).unwrap();
        fs::create_dir(&left).unwrap();
        fs::write(left.join("a.txt"), b"left version").unwrap();
        let execution = execute_copy_plan(&plan, &AtomicBool::new(false));

        assert!(matches!(
            execution.outcomes.as_slice(),
            [CopyOutcome::Failed(path, reason)]
                if path == Path::new("a.txt") && reason.contains("来源目录对象已变化")
        ));
        assert!(!destination.join("a.txt").exists());
        assert_eq!(fs::read(left.join("a.txt")).unwrap(), b"left version");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn copy_plan_rejects_partial_reports_and_destination_inside_source() {
        let (root, left, right) = roots();
        fs::write(left.join("a.txt"), b"a-left").unwrap();
        fs::write(right.join("a.txt"), b"a-right").unwrap();
        let report = compare(
            &left,
            &right,
            &AtomicBool::new(false),
            Limits::default(),
            |_| {},
        )
        .unwrap();
        let nested = left.join("nested");
        fs::create_dir(&nested).unwrap();
        assert!(
            prepare_copy_plan(&report, CopySide::Left, &[PathBuf::from("a.txt")], &nested).is_err()
        );
        let partial = compare(
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
        assert_eq!(partial.status, ScanStatus::Partial);
        let destination = root.join("destination");
        fs::create_dir(&destination).unwrap();
        assert!(
            prepare_copy_plan(
                &partial,
                CopySide::Left,
                &[PathBuf::from("a.txt")],
                &destination
            )
            .is_err()
        );
        let cancel = AtomicBool::new(true);
        let plan = prepare_copy_plan(
            &report,
            CopySide::Left,
            &[PathBuf::from("a.txt")],
            &destination,
        )
        .unwrap();
        let execution = execute_copy_plan(&plan, &cancel);
        assert!(execution.cancelled);
        assert!(matches!(
            execution.outcomes.as_slice(),
            [CopyOutcome::Cancelled]
        ));
        assert!(
            prepare_copy_plan_cancellable(
                &report,
                CopySide::Left,
                &[PathBuf::from("a.txt")],
                &destination,
                &cancel,
            )
            .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ui_copy_state_keeps_preview_read_only_until_explicit_execution() {
        let (root, left, right) = roots();
        fs::write(left.join("chosen.txt"), b"selected source").unwrap();
        fs::write(right.join("chosen.txt"), b"other version").unwrap();
        let destination = root.join("output");
        fs::create_dir(&destination).unwrap();
        let report = compare(
            &left,
            &right,
            &AtomicBool::new(false),
            Limits::default(),
            |_| {},
        )
        .unwrap();
        let relative = PathBuf::from("chosen.txt");
        let mut state = State::default();
        state.report = Some(report);
        state.selected.insert(relative.clone());
        state.destination_path = destination.display().to_string();

        state.prepare_copy(CopySide::Left, vec![relative.clone()], destination.clone());
        let deadline = Instant::now() + Duration::from_secs(5);
        while state.copy_receiver.is_some() && Instant::now() < deadline {
            state.poll_copy();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            state.copy_receiver.is_none(),
            "copy preview worker did not finish"
        );
        assert!(
            state.copy_plan.is_some(),
            "valid preview should be available"
        );
        assert!(can_execute_copy(
            state.copy_plan.as_ref(),
            state.copy_result.as_ref(),
            state.copy_receiver.is_some(),
        ));
        assert!(
            !destination.join(&relative).exists(),
            "preview must never write files"
        );

        state.execute_copy();
        let deadline = Instant::now() + Duration::from_secs(5);
        while state.copy_receiver.is_some() && Instant::now() < deadline {
            state.poll_copy();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(state.copy_receiver.is_none(), "copy worker did not finish");
        let copied = destination.join(&relative);
        assert_eq!(fs::read(&copied).unwrap(), b"selected source");
        assert!(state.copy_result.as_ref().is_some_and(|result| {
            result
                .outcomes
                .iter()
                .any(|outcome| matches!(outcome, CopyOutcome::Copied(_)))
        }));
        assert!(!can_execute_copy(
            state.copy_plan.as_ref(),
            state.copy_result.as_ref(),
            state.copy_receiver.is_some(),
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn changing_copy_destination_or_source_side_discards_the_old_preview() {
        let (root, left, right) = roots();
        fs::write(left.join("selected.txt"), b"left").unwrap();
        fs::write(right.join("selected.txt"), b"right").unwrap();
        let destination = root.join("output");
        fs::create_dir(&destination).unwrap();
        let report = compare(
            &left,
            &right,
            &AtomicBool::new(false),
            Limits::default(),
            |_| {},
        )
        .unwrap();
        let relative = PathBuf::from("selected.txt");
        let plan = prepare_copy_plan(
            &report,
            CopySide::Left,
            std::slice::from_ref(&relative),
            &destination,
        )
        .unwrap();
        let mut state = State::default();
        state.report = Some(report);
        state.selected.insert(relative.clone());
        state.copy_plan = Some(plan.clone());

        change_copy_destination(
            &mut state.destination_path,
            &mut state.copy_plan,
            &mut state.copy_result,
            root.join("other-output").display().to_string(),
        );
        assert!(state.copy_plan.is_none());
        state.copy_plan = Some(plan);
        change_copy_side(
            &mut state.copy_side,
            &mut state.selected,
            &mut state.copy_plan,
            &mut state.copy_result,
            CopySide::Right,
        );
        assert!(state.copy_plan.is_none());
        assert!(state.selected.is_empty());
        state.execute_copy();
        assert!(!destination.join(relative).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dropping_state_cancels_background_copy_work() {
        let copy_cancel = Arc::new(AtomicBool::new(false));
        let mut state = State::default();
        state.copy_cancel = copy_cancel.clone();

        drop(state);

        assert!(copy_cancel.load(Ordering::Relaxed));
    }

    #[test]
    fn bulk_selection_only_adds_visible_eligible_files_and_respects_limit() {
        let rows = vec![
            Row {
                relative: PathBuf::from("a-left.txt"),
                difference: Difference::OnlyLeft,
                left_kind: Some(ItemKind::File),
                right_kind: None,
                left_size: Some(4),
                right_size: None,
                left_sha256: None,
                right_sha256: None,
            },
            Row {
                relative: PathBuf::from("b-right.txt"),
                difference: Difference::OnlyRight,
                left_kind: None,
                right_kind: Some(ItemKind::File),
                left_size: None,
                right_size: Some(5),
                left_sha256: None,
                right_sha256: None,
            },
            Row {
                relative: PathBuf::from("folder"),
                difference: Difference::OnlyLeft,
                left_kind: Some(ItemKind::Directory),
                right_kind: None,
                left_size: None,
                right_size: None,
                left_sha256: None,
                right_sha256: None,
            },
            Row {
                relative: PathBuf::from("unverified.txt"),
                difference: Difference::Unverified,
                left_kind: Some(ItemKind::File),
                right_kind: Some(ItemKind::File),
                left_size: Some(1),
                right_size: Some(1),
                left_sha256: None,
                right_sha256: None,
            },
        ];
        let visible: Vec<_> = rows.iter().collect();
        let mut selected = BTreeSet::new();
        select_visible_copyable_rows(&mut selected, &visible, CopySide::Left, 1);
        assert_eq!(selected, BTreeSet::from([PathBuf::from("a-left.txt")]));
        select_visible_copyable_rows(&mut selected, &visible, CopySide::Right, 2);
        assert_eq!(
            selected,
            BTreeSet::from([PathBuf::from("a-left.txt"), PathBuf::from("b-right.txt")])
        );
        select_visible_copyable_rows(&mut selected, &visible, CopySide::Left, 1);
        assert_eq!(
            selected.len(),
            2,
            "a full selection must not grow past its limit"
        );
    }
}
