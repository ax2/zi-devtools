//! Bounded, read-only directory size analysis. File contents are never opened.
use anyhow::{Context, Result, ensure};
use eframe::egui;
use std::{
    collections::BTreeMap,
    fs::{self, Metadata},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};

const LARGEST_LIMIT: usize = 20;

#[derive(Clone, Copy)]
pub struct ScanLimits {
    pub max_entries: u64,
    pub max_depth: usize,
    pub max_duration: Duration,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            max_entries: 250_000,
            max_depth: 24,
            max_duration: Duration::from_secs(20),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub entries: u64,
    pub files: u64,
    pub directories: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug)]
pub struct Usage {
    pub path: String,
    pub bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanStatus {
    Complete,
    Partial,
    Cancelled,
}

#[derive(Clone, Debug)]
pub struct ScanResult {
    pub root: PathBuf,
    pub progress: Progress,
    pub status: ScanStatus,
    pub groups: Vec<Usage>,
    pub group_count: usize,
    pub largest_files: Vec<Usage>,
    pub skipped_links: u64,
    pub skipped_depth: u64,
    pub errors: u64,
    pub limit_reason: Option<&'static str>,
}

#[cfg(windows)]
fn is_link(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_link(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn keep_largest(largest: &mut Vec<Usage>, item: Usage) {
    if largest.len() < LARGEST_LIMIT {
        largest.push(item);
    } else if let Some((index, minimum)) = largest
        .iter()
        .enumerate()
        .min_by_key(|(_, existing)| existing.bytes)
        && item.bytes > minimum.bytes
    {
        largest[index] = item;
    }
}

pub fn scan(
    root: &Path,
    cancel: &AtomicBool,
    limits: ScanLimits,
    mut on_progress: impl FnMut(&Progress),
) -> Result<ScanResult> {
    let metadata = fs::symlink_metadata(root).context("无法读取目录信息")?;
    ensure!(
        metadata.is_dir() && !is_link(&metadata),
        "请选择普通目录，不能是链接或重解析点"
    );
    ensure!(
        limits.max_entries > 0 && limits.max_duration > Duration::ZERO,
        "扫描上限无效"
    );
    let started = Instant::now();
    let mut result = ScanResult {
        root: root.to_path_buf(),
        progress: Progress {
            directories: 1,
            ..Progress::default()
        },
        status: ScanStatus::Complete,
        groups: Vec::new(),
        group_count: 0,
        largest_files: Vec::new(),
        skipped_links: 0,
        skipped_depth: 0,
        errors: 0,
        limit_reason: None,
    };
    let mut groups: BTreeMap<Option<String>, u64> = BTreeMap::new();
    let mut stack = vec![(root.to_path_buf(), 0usize, None::<String>)];
    'scan: while let Some((directory, depth, group)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            result.status = ScanStatus::Cancelled;
            break;
        }
        if started.elapsed() >= limits.max_duration {
            result.limit_reason = Some("超过扫描时间上限");
            break;
        }
        if depth > limits.max_depth {
            result.skipped_depth += 1;
            continue;
        }
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => {
                result.errors += 1;
                continue;
            }
        };
        for entry in entries {
            if cancel.load(Ordering::Relaxed) {
                result.status = ScanStatus::Cancelled;
                break 'scan;
            }
            if started.elapsed() >= limits.max_duration {
                result.limit_reason = Some("超过扫描时间上限");
                break 'scan;
            }
            if result.progress.entries >= limits.max_entries {
                result.limit_reason = Some("达到扫描条目上限");
                break 'scan;
            }
            result.progress.entries += 1;
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    result.errors += 1;
                    continue;
                }
            };
            let path = entry.path();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(_) => {
                    result.errors += 1;
                    continue;
                }
            };
            if is_link(&metadata) {
                result.skipped_links += 1;
                continue;
            }
            let item_group = group.clone().or_else(|| {
                if metadata.is_dir() {
                    Some(entry.file_name().to_string_lossy().into_owned())
                } else {
                    None
                }
            });
            if metadata.is_dir() {
                result.progress.directories += 1;
                if depth < limits.max_depth {
                    stack.push((path, depth + 1, item_group));
                } else {
                    result.skipped_depth += 1;
                }
            } else if metadata.is_file() {
                result.progress.files += 1;
                result.progress.bytes = result.progress.bytes.saturating_add(metadata.len());
                let group_bytes = groups.entry(item_group).or_default();
                *group_bytes = group_bytes.saturating_add(metadata.len());
                keep_largest(
                    &mut result.largest_files,
                    Usage {
                        path: path
                            .strip_prefix(root)
                            .unwrap_or(&path)
                            .display()
                            .to_string(),
                        bytes: metadata.len(),
                    },
                );
            }
            if result.progress.entries.is_multiple_of(128) {
                on_progress(&result.progress);
            }
        }
    }
    on_progress(&result.progress);
    result.groups = groups
        .into_iter()
        .map(|(path, bytes)| Usage {
            path: path.unwrap_or_else(|| "（目录根部的文件）".to_owned()),
            bytes,
        })
        .collect();
    result
        .groups
        .sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
    result.group_count = result.groups.len();
    result.groups.truncate(20);
    result
        .largest_files
        .sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
    if result.status != ScanStatus::Cancelled
        && (result.limit_reason.is_some()
            || result.skipped_links > 0
            || result.skipped_depth > 0
            || result.errors > 0)
    {
        result.status = ScanStatus::Partial;
    }
    Ok(result)
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
    result: Option<ScanResult>,
    message: String,
    receiver: Option<Receiver<Result<ScanResult, String>>>,
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<Progress>>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            path: String::new(),
            result: None,
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
            Ok(Ok(result)) => {
                self.message = match result.status {
                    ScanStatus::Complete => "扫描完成".to_owned(),
                    ScanStatus::Partial => "扫描未覆盖全部内容；下方仅为已统计部分".to_owned(),
                    ScanStatus::Cancelled => "已取消；下方仅为已统计部分".to_owned(),
                };
                self.result = Some(result);
                self.receiver = None;
            }
            Ok(Err(error)) => {
                self.message = error;
                self.receiver = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.message = "扫描线程意外结束".to_owned();
                self.receiver = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }

    fn start(&mut self) {
        let root = PathBuf::from(self.path.trim().trim_matches('"'));
        self.result = None;
        self.message = "正在分析目录…".to_owned();
        self.cancel = Arc::new(AtomicBool::new(false));
        self.progress = Arc::new(Mutex::new(Progress::default()));
        let cancel = self.cancel.clone();
        let progress = self.progress.clone();
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        std::thread::spawn(move || {
            let result = scan(&root, &cancel, ScanLimits::default(), |value| {
                if let Ok(mut current) = progress.lock() {
                    *current = value.clone();
                }
            })
            .map_err(|error| format!("扫描失败：{error:#}"));
            let _ = sender.send(result);
        });
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self) {
        self.path = r"C:\Projects\sample-app".to_owned();
        self.message = "扫描完成 · 合成预览，未读取本机文件".to_owned();
        self.result = Some(ScanResult {
            root: PathBuf::from(&self.path),
            progress: Progress {
                entries: 12_400,
                files: 11_950,
                directories: 451,
                bytes: 12_294_636_146,
            },
            status: ScanStatus::Complete,
            groups: vec![
                Usage {
                    path: "target".into(),
                    bytes: 10_816_000_000,
                },
                Usage {
                    path: "dist".into(),
                    bytes: 1_100_000_000,
                },
                Usage {
                    path: "release".into(),
                    bytes: 300_000_000,
                },
                Usage {
                    path: "assets".into(),
                    bytes: 70_000_000,
                },
                Usage {
                    path: "（目录根部的文件）".into(),
                    bytes: 8_636_146,
                },
            ],
            group_count: 5,
            largest_files: vec![
                Usage {
                    path: r"target\release\ZiDevTools.exe".into(),
                    bytes: 23_000_000,
                },
                Usage {
                    path: r"dist\Stage-70\Local\ZiDevTools.exe".into(),
                    bytes: 22_300_000,
                },
                Usage {
                    path: r"target\debug\examples\capture_ui.exe".into(),
                    bytes: 18_400_000,
                },
                Usage {
                    path: r"release\stage-70\ZiDevTools.msi".into(),
                    bytes: 8_970_000,
                },
            ],
            skipped_links: 0,
            skipped_depth: 0,
            errors: 0,
            limit_reason: None,
        });
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.poll();
        ui.heading("目录空间分析");
        ui.label(
            "选择本机目录，统计占用最大的直属子目录和文件。只读文件信息，不读取正文或删除文件。",
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
                self.result = None;
                self.message.clear();
            }
            if ui
                .add_enabled(self.receiver.is_none(), egui::Button::new("选择目录…"))
                .clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                self.path = path.display().to_string();
                self.result = None;
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.receiver.is_none() && !self.path.trim().is_empty(),
                    egui::Button::new("开始分析"),
                )
                .clicked()
            {
                self.start();
            }
            if self.receiver.is_some() && ui.button("取消扫描").clicked() {
                self.cancel.store(true, Ordering::Relaxed);
            }
            if self.receiver.is_some()
                && let Ok(progress) = self.progress.lock()
            {
                ui.label(format!(
                    "已检查 {} 项、{} 个文件 · 已统计 {}",
                    progress.entries,
                    progress.files,
                    size(progress.bytes)
                ));
                ui.ctx().request_repaint_after(Duration::from_millis(150));
            }
        });
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        let Some(result) = &self.result else { return };
        ui.add_space(12.0);
        ui.columns(3, |columns| {
            columns[0].group(|ui| {
                ui.weak("已统计大小");
                ui.heading(size(result.progress.bytes));
            });
            columns[1].group(|ui| {
                ui.weak("普通文件");
                ui.heading(result.progress.files.to_string());
            });
            columns[2].group(|ui| {
                ui.weak("目录 / 已检查条目");
                ui.heading(format!(
                    "{} / {}",
                    result.progress.directories, result.progress.entries
                ));
            });
        });
        if result.status != ScanStatus::Complete {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!(
                    "部分结果：{}；跳过链接 {}、深度上限目录 {}、读取错误 {}。",
                    result.limit_reason.unwrap_or("未覆盖全部条目"),
                    result.skipped_links,
                    result.skipped_depth,
                    result.errors
                ),
            );
        }
        ui.separator();
        ui.strong(format!(
            "直属子目录与根部文件 · 显示 {} / {} 组",
            result.groups.len(),
            result.group_count
        ));
        if result.groups.is_empty() {
            ui.weak("没有统计到普通文件");
        }
        for group in &result.groups {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("{:>12}", size(group.bytes))).monospace());
                ui.add_sized([190.0, 22.0], egui::Label::new(&group.path));
                let share = if result.progress.bytes == 0 {
                    0.0
                } else {
                    group.bytes as f32 / result.progress.bytes as f32
                };
                ui.add(egui::ProgressBar::new(share.clamp(0.0, 1.0)).desired_width(480.0));
            });
        }
        ui.add_space(8.0);
        ui.strong("最大的文件（最多 20 个）");
        for file in &result.largest_files {
            ui.horizontal(|ui| {
                ui.monospace(format!("{:>12}", size(file.bytes)));
                ui.label(&file.path);
            });
        }
        ui.add_space(8.0);
        ui.small("统计的是文件逻辑长度，不等于磁盘实际占用。硬链接可能重复计数；扫描期间变化的文件会影响结果。目录与文件不会自动清理。");
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
    fn aggregates_direct_children_and_reports_partial_limits() {
        let root = std::env::temp_dir().join(format!("zi-disk-inspector-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("alpha/deep")).unwrap();
        fs::create_dir_all(root.join("beta")).unwrap();
        fs::write(root.join("root.bin"), vec![0u8; 7]).unwrap();
        fs::write(root.join("alpha/a.bin"), vec![0u8; 11]).unwrap();
        fs::write(root.join("alpha/deep/b.bin"), vec![0u8; 13]).unwrap();
        fs::write(root.join("beta/c.bin"), vec![0u8; 5]).unwrap();
        let result = scan(
            &root,
            &AtomicBool::new(false),
            ScanLimits::default(),
            |_| {},
        )
        .unwrap();
        assert_eq!(result.status, ScanStatus::Complete);
        assert_eq!(result.progress.bytes, 36);
        assert_eq!(result.progress.files, 4);
        assert_eq!(result.groups[0].path, "alpha");
        assert_eq!(result.groups[0].bytes, 24);
        assert_eq!(result.groups[1].path, "（目录根部的文件）");
        assert_eq!(result.groups[1].bytes, 7);
        assert_eq!(result.largest_files[0].bytes, 13);
        let limited = scan(
            &root,
            &AtomicBool::new(false),
            ScanLimits {
                max_depth: 0,
                ..ScanLimits::default()
            },
            |_| {},
        )
        .unwrap();
        assert_eq!(limited.status, ScanStatus::Partial);
        assert!(limited.skipped_depth >= 2);
        let entry_limited = scan(
            &root,
            &AtomicBool::new(false),
            ScanLimits {
                max_entries: 1,
                ..ScanLimits::default()
            },
            |_| {},
        )
        .unwrap();
        assert_eq!(entry_limited.status, ScanStatus::Partial);
        assert_eq!(entry_limited.limit_reason, Some("达到扫描条目上限"));
        let cancelled = scan(&root, &AtomicBool::new(true), ScanLimits::default(), |_| {}).unwrap();
        assert_eq!(cancelled.status, ScanStatus::Cancelled);
        #[cfg(windows)]
        let linked =
            std::os::windows::fs::symlink_file(root.join("root.bin"), root.join("link.bin"));
        #[cfg(not(windows))]
        let linked = std::os::unix::fs::symlink(root.join("root.bin"), root.join("link.bin"));
        if linked.is_ok() {
            let with_link = scan(
                &root,
                &AtomicBool::new(false),
                ScanLimits::default(),
                |_| {},
            )
            .unwrap();
            assert_eq!(with_link.status, ScanStatus::Partial);
            assert_eq!(with_link.progress.bytes, 36);
            assert_eq!(with_link.skipped_links, 1);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancellation_interrupts_an_active_scan() {
        let root = std::env::temp_dir().join(format!("zi-disk-cancel-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        for index in 0..140 {
            fs::write(root.join(format!("{index:03}.txt")), b"x").unwrap();
        }
        let cancel = AtomicBool::new(false);
        let result = scan(&root, &cancel, ScanLimits::default(), |progress| {
            if progress.entries >= 128 {
                cancel.store(true, Ordering::Relaxed);
            }
        })
        .unwrap();
        assert_eq!(result.status, ScanStatus::Cancelled);
        assert_eq!(result.progress.entries, 128);
        fs::remove_dir_all(root).unwrap();
    }
}
