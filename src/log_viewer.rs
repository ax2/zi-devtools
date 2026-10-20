//! Read-only, bounded-index log browsing. File contents are loaded only for the visible page.

use std::{
    fs::{self, File},
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::SystemTime,
};

use anyhow::{Context, Result, bail, ensure};
use chrono::NaiveDateTime;
use crossbeam_channel::{Receiver, unbounded};
use eframe::egui;

#[cfg(feature = "ui-preview")]
use std::io::Write;

const INDEX_LIMIT: usize = 500_000;
const PAGE_SIZE: usize = 100;
const LINE_PREFIX_LIMIT: usize = 4096;
const EVENT_PREVIEW_LIMIT: usize = 16 * 1024;
const SEARCH_QUERY_LIMIT: usize = 256;
const SEARCH_CHUNK_SIZE: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Level {
    Error,
    Fatal,
    Warning,
    Info,
    Debug,
    Trace,
    #[default]
    Unknown,
}

impl Level {
    fn label(self) -> &'static str {
        match self {
            Self::Error => "错误",
            Self::Fatal => "致命",
            Self::Warning => "警告",
            Self::Info => "信息",
            Self::Debug => "调试",
            Self::Trace => "跟踪",
            Self::Unknown => "未识别",
        }
    }

    fn from_first_line(bytes: &[u8]) -> Self {
        let line = String::from_utf8_lossy(bytes).to_ascii_uppercase();
        for (tokens, level) in [
            (&["FATAL", "CRITICAL"][..], Self::Fatal),
            (&["ERROR", "ERR"][..], Self::Error),
            (&["WARN", "WARNING"][..], Self::Warning),
            (&["INFO"][..], Self::Info),
            (&["DEBUG"][..], Self::Debug),
            (&["TRACE"][..], Self::Trace),
        ] {
            if tokens.iter().any(|token| contains_word(&line, token)) {
                return level;
            }
        }
        Self::Unknown
    }
}

fn contains_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(start, _)| {
        let end = start + word.len();
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        before.is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_')
            && after.is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_')
    })
}

#[derive(Clone, Debug)]
struct EventIndex {
    start: u64,
    end: u64,
    timestamp: Option<NaiveDateTime>,
    level: Level,
    lines: u32,
    first_line_truncated: bool,
}

struct Snapshot {
    path: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
    identity: same_file::Handle,
    events: Arc<[EventIndex]>,
    partial: bool,
    scanned_bytes: u64,
}

impl Snapshot {
    fn verify_unchanged(&self) -> Result<()> {
        let metadata = fs::symlink_metadata(&self.path)
            .with_context(|| format!("日志文件不可用：{}", self.path.display()))?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "选择的对象不再是普通文件"
        );
        ensure!(
            metadata.len() == self.size && metadata.modified().ok() == self.modified,
            "日志文件在索引后发生变化，请重新读取"
        );
        let identity = same_file::Handle::from_path(&self.path).context("无法复核日志文件身份")?;
        ensure!(self.identity == identity, "日志文件身份在索引后发生变化");
        Ok(())
    }

    fn load_page(&self, indices: &[usize]) -> Result<Vec<LoadedEvent>> {
        self.verify_unchanged()?;
        let mut file = File::open(&self.path).context("重新打开日志文件失败")?;
        let mut page = Vec::with_capacity(indices.len());
        for &index in indices {
            let event = self.events.get(index).context("日志事件索引越界")?;
            let length = event.end.saturating_sub(event.start);
            let read_length = length.min(EVENT_PREVIEW_LIMIT as u64);
            file.seek(SeekFrom::Start(event.start))
                .context("定位日志事件失败")?;
            let mut bytes = Vec::with_capacity(read_length as usize);
            (&mut file)
                .take(read_length)
                .read_to_end(&mut bytes)
                .context("读取日志事件失败")?;
            let truncated = event.first_line_truncated || length > read_length;
            page.push(LoadedEvent {
                timestamp: event.timestamp,
                level: event.level,
                lines: event.lines,
                text: String::from_utf8_lossy(&bytes).into_owned(),
                truncated,
            });
        }
        self.verify_unchanged()?;
        Ok(page)
    }
}

#[derive(Clone, Debug)]
struct LoadedEvent {
    timestamp: Option<NaiveDateTime>,
    level: Level,
    lines: u32,
    text: String,
    truncated: bool,
}

enum ScanMessage {
    Progress {
        bytes: u64,
        events: usize,
    },
    Finished(Result<Snapshot, String>),
    SearchProgress {
        bytes: u64,
        matches: usize,
    },
    SearchFinished {
        query: String,
        result: Result<Vec<usize>, String>,
    },
}

#[derive(Default)]
pub struct State {
    path: Option<PathBuf>,
    snapshot: Option<Snapshot>,
    visible: Vec<usize>,
    loaded: Vec<LoadedEvent>,
    level_filter: Option<Level>,
    seek_time: String,
    search_query: String,
    applied_search: Option<String>,
    search_results: Option<Vec<usize>>,
    searching: bool,
    page: usize,
    progress_bytes: u64,
    progress_events: usize,
    pending: bool,
    cancel: Option<Arc<AtomicBool>>,
    receiver: Option<Receiver<ScanMessage>>,
    message: String,
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Release);
        }
    }
}

impl State {
    pub fn ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.poll(ctx);
        ui.heading("日志工作台");
        ui.label("只读浏览本机日志；普通翻页仅读当前页正文，显式正文搜索会流式扫描已索引事件。不会修改源文件或保存日志内容。");

        ui.horizontal_wrapped(|ui| {
            if ui.button("打开日志文件…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_title("选择要查看的日志")
                    .add_filter("日志与文本", &["log", "txt", "out"])
                    .pick_file()
            {
                self.start_scan(path, ctx.clone());
            }
            if self.pending && ui.button("取消").clicked() {
                self.cancel_scan();
            }
            if self.snapshot.is_some() && !self.pending && ui.button("重新索引").clicked() {
                if let Some(path) = self.path.clone() {
                    self.start_scan(path, ctx.clone());
                }
            }
        });

        if let Some(path) = &self.path {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_else(|| "日志文件".into());
            ui.small(format!("当前文件：{name}"));
        }
        if self.pending {
            ui.horizontal(|ui| {
                ui.spinner();
                if self.searching {
                    ui.label(format!(
                        "正在搜索 {} 字节，已找到 {} 项…",
                        self.progress_bytes, self.progress_events
                    ));
                } else {
                    ui.label(format!(
                        "正在扫描 {} 字节，已索引 {} 个事件…",
                        self.progress_bytes, self.progress_events
                    ));
                }
            });
        }
        if !self.message.is_empty() {
            ui.colored_label(
                if self.pending || self.snapshot.is_some() {
                    ui.visuals().weak_text_color()
                } else {
                    ui.visuals().error_fg_color
                },
                &self.message,
            );
        }

        if let Some(snapshot) = &self.snapshot {
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                ui.strong(format!("{} 个事件", snapshot.events.len()));
                ui.label(format!("源文件 {} 字节", snapshot.size));
                if snapshot.partial {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        format!(
                            "已达到 {} 事件索引上限；只显示文件前 {} 字节",
                            INDEX_LIMIT, snapshot.scanned_bytes
                        ),
                    );
                }
            });

            ui.horizontal_wrapped(|ui| {
                ui.label("正文关键词");
                ui.add(
                    egui::TextEdit::singleline(&mut self.search_query)
                        .desired_width(250.0)
                        .hint_text("区分大小写；最多 256 字节"),
                );
                if ui
                    .add_enabled(!self.pending, egui::Button::new("搜索正文"))
                    .clicked()
                {
                    self.start_search(ctx.clone());
                }
                if let Some(query) = &self.applied_search {
                    ui.weak(format!(
                        "匹配“{query}” · {} 项",
                        self.search_results.as_ref().map_or(0, Vec::len)
                    ));
                }
            });

            let previous_filter = self.level_filter;
            egui::ComboBox::from_id_salt("log-viewer-level")
                .selected_text(self.level_filter.map_or("全部级别", Level::label))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.level_filter, None, "全部级别");
                    for level in [
                        Level::Fatal,
                        Level::Error,
                        Level::Warning,
                        Level::Info,
                        Level::Debug,
                        Level::Trace,
                        Level::Unknown,
                    ] {
                        ui.selectable_value(&mut self.level_filter, Some(level), level.label());
                    }
                });
            if previous_filter != self.level_filter {
                self.apply_filter();
                self.refresh_page();
            }

            ui.horizontal_wrapped(|ui| {
                ui.label("定位到时间及之后");
                ui.add(
                    egui::TextEdit::singleline(&mut self.seek_time)
                        .desired_width(190.0)
                        .hint_text("YYYY-MM-DD HH:MM:SS"),
                );
                if ui.button("定位").clicked() {
                    self.seek_to_time();
                    self.refresh_page();
                }
                let page_count = self.visible.len().div_ceil(PAGE_SIZE);
                ui.separator();
                if ui
                    .add_enabled(self.page > 0, egui::Button::new("上一页"))
                    .clicked()
                {
                    self.page -= 1;
                    self.refresh_page();
                }
                ui.label(format!(
                    "第 {} / {} 页 · 当前筛选 {} 项",
                    self.page + 1,
                    page_count.max(1),
                    self.visible.len()
                ));
                if ui
                    .add_enabled(self.page + 1 < page_count, egui::Button::new("下一页"))
                    .clicked()
                {
                    self.page += 1;
                    self.refresh_page();
                }
            });

            egui::ScrollArea::vertical()
                .id_salt("log-viewer-events")
                .max_height(620.0)
                .show(ui, |ui| {
                    for event in &self.loaded {
                        ui.group(|ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.strong(event.level.label());
                                if let Some(timestamp) = event.timestamp {
                                    ui.monospace(timestamp.format("%Y-%m-%d %H:%M:%S").to_string());
                                } else {
                                    ui.weak("无可识别时间戳");
                                }
                                ui.weak(format!("{} 行", event.lines));
                            });
                            ui.monospace(&event.text);
                            if event.truncated {
                                ui.weak("此事件预览已截断；源日志未修改。");
                            }
                        });
                    }
                    if self.loaded.is_empty() {
                        ui.weak("当前筛选没有事件");
                    }
                });
        } else if !self.pending {
            ui.separator();
            ui.weak("选择一个日志文件开始。支持 UTF-8 文本；无效 UTF-8 字节以替换字符显示。");
        }
    }

    fn start_scan(&mut self, path: PathBuf, ctx: egui::Context) {
        self.cancel_scan();
        self.snapshot = None;
        self.visible.clear();
        self.loaded.clear();
        self.page = 0;
        self.level_filter = None;
        self.seek_time.clear();
        self.applied_search = None;
        self.search_results = None;
        self.searching = false;
        self.progress_bytes = 0;
        self.progress_events = 0;
        self.path = Some(path.clone());
        self.message.clear();
        self.pending = true;

        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel.clone());
        let (tx, rx) = unbounded();
        self.receiver = Some(rx);
        thread::spawn(move || {
            let progress_tx = tx.clone();
            let result = scan_file(&path, &cancel, |bytes, events| {
                let _ = progress_tx.send(ScanMessage::Progress { bytes, events });
                ctx.request_repaint();
            })
            .map_err(|error| format!("{error:#}"));
            let _ = tx.send(ScanMessage::Finished(result));
            ctx.request_repaint();
        });
    }

    fn start_search(&mut self, ctx: egui::Context) {
        let query = self.search_query.clone();
        if let Err(error) = validate_search_query(&query) {
            self.message = error.to_string();
            return;
        }
        let Some(snapshot) = &self.snapshot else {
            self.message = "请先打开日志文件".into();
            return;
        };
        let prepared = (|| -> Result<(File, same_file::Handle)> {
            snapshot.verify_unchanged()?;
            let file = File::open(&snapshot.path).context("打开日志文件进行搜索失败")?;
            let metadata = file.metadata().context("读取搜索文件属性失败")?;
            ensure!(
                metadata.len() == snapshot.size && metadata.modified().ok() == snapshot.modified,
                "日志文件在搜索前发生变化，请重新索引"
            );
            let identity =
                same_file::Handle::from_file(file.try_clone().context("复制只读文件句柄失败")?)
                    .context("无法确认搜索文件身份")?;
            ensure!(identity == snapshot.identity, "日志文件在搜索前已被替换");
            Ok((file, identity))
        })();
        let (file, identity) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                self.message = format!("无法开始搜索：{error:#}");
                return;
            }
        };
        let path = snapshot.path.clone();
        let size = snapshot.size;
        let modified = snapshot.modified;
        let events = Arc::clone(&snapshot.events);
        self.searching = true;
        self.pending = true;
        self.progress_bytes = 0;
        self.progress_events = 0;
        self.search_results = None;
        self.applied_search = None;
        self.page = 0;
        self.apply_filter();
        self.message.clear();

        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel.clone());
        let (tx, rx) = unbounded();
        self.receiver = Some(rx);
        thread::spawn(move || {
            let progress_tx = tx.clone();
            let result = search_events(
                SearchSource {
                    path,
                    file,
                    identity,
                    expected_size: size,
                    expected_modified: modified,
                    events,
                },
                query.as_bytes(),
                &cancel,
                |bytes, matches| {
                    let _ = progress_tx.send(ScanMessage::SearchProgress { bytes, matches });
                    ctx.request_repaint();
                },
            )
            .map_err(|error| format!("{error:#}"));
            let _ = tx.send(ScanMessage::SearchFinished { query, result });
            ctx.request_repaint();
        });
    }

    fn cancel_scan(&mut self) {
        let was_searching = self.searching;
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        if self.pending {
            self.pending = false;
            self.searching = false;
            self.receiver = None;
            if was_searching {
                self.search_results = None;
                self.applied_search = None;
                self.apply_filter();
                self.refresh_page();
                self.message = "已取消正文搜索；源文件未修改。".into();
            } else {
                self.message = "已取消日志扫描；源文件未修改。".into();
            }
        }
    }

    fn poll(&mut self, _ctx: &egui::Context) {
        loop {
            let message = self
                .receiver
                .as_ref()
                .and_then(|receiver| receiver.try_recv().ok());
            let Some(message) = message else { break };
            match message {
                ScanMessage::Progress { bytes, events } => {
                    self.progress_bytes = bytes;
                    self.progress_events = events;
                }
                ScanMessage::SearchProgress { bytes, matches } => {
                    self.progress_bytes = bytes;
                    self.progress_events = matches;
                }
                ScanMessage::Finished(Ok(snapshot)) => {
                    self.pending = false;
                    self.searching = false;
                    self.cancel = None;
                    self.snapshot = Some(snapshot);
                    self.apply_filter();
                    self.refresh_page();
                    if self
                        .snapshot
                        .as_ref()
                        .is_some_and(|snapshot| snapshot.partial)
                    {
                        self.message = format!(
                            "索引已达到 {INDEX_LIMIT} 个事件的内存上限；文件剩余部分未索引。"
                        );
                    } else {
                        self.message = "索引完成；日志以只读方式打开。".into();
                    }
                    self.receiver = None;
                    break;
                }
                ScanMessage::Finished(Err(error)) => {
                    self.pending = false;
                    self.searching = false;
                    self.cancel = None;
                    self.snapshot = None;
                    self.visible.clear();
                    self.loaded.clear();
                    self.message = format!("日志索引失败：{error}");
                    self.receiver = None;
                    break;
                }
                ScanMessage::SearchFinished { query, result } => {
                    self.pending = false;
                    self.searching = false;
                    self.cancel = None;
                    self.receiver = None;
                    match result {
                        Ok(matches) => {
                            let count = matches.len();
                            self.search_results = Some(matches);
                            self.applied_search = Some(query);
                            self.page = 0;
                            self.apply_filter();
                            self.refresh_page();
                            self.message = format!("正文搜索完成，匹配 {count} 个事件。 ");
                        }
                        Err(error) => {
                            self.search_results = None;
                            self.applied_search = None;
                            self.apply_filter();
                            self.refresh_page();
                            self.message = format!("正文搜索失败：{error}");
                        }
                    }
                    break;
                }
            }
        }
    }

    fn apply_filter(&mut self) {
        let Some(snapshot) = &self.snapshot else {
            self.visible.clear();
            return;
        };
        self.visible = if let Some(matches) = &self.search_results {
            matches
                .iter()
                .copied()
                .filter(|&index| {
                    snapshot.events.get(index).is_some_and(|event| {
                        self.level_filter.is_none_or(|level| event.level == level)
                    })
                })
                .collect()
        } else {
            snapshot
                .events
                .iter()
                .enumerate()
                .filter_map(|(index, event)| {
                    self.level_filter
                        .is_none_or(|level| event.level == level)
                        .then_some(index)
                })
                .collect()
        };
        self.page = self
            .page
            .min(self.visible.len().saturating_sub(1) / PAGE_SIZE);
    }

    fn seek_to_time(&mut self) {
        let target = match NaiveDateTime::parse_from_str(&self.seek_time, "%Y-%m-%d %H:%M:%S") {
            Ok(target) => target,
            Err(_) => {
                self.message = "请输入 YYYY-MM-DD HH:MM:SS 格式的时间".into();
                return;
            }
        };
        let Some(position) = self.visible.iter().position(|&index| {
            self.snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.events.get(index))
                .and_then(|event| event.timestamp)
                .is_some_and(|timestamp| timestamp >= target)
        }) else {
            self.message = "筛选结果中没有该时间及之后的带时间戳事件".into();
            return;
        };
        self.page = position / PAGE_SIZE;
        self.message = format!("已定位到第 {} 项", position + 1);
    }

    fn refresh_page(&mut self) {
        let start = self.page.saturating_mul(PAGE_SIZE);
        let end = (start + PAGE_SIZE).min(self.visible.len());
        let indices = self.visible.get(start..end).unwrap_or_default();
        let Some(snapshot) = &self.snapshot else {
            self.loaded.clear();
            return;
        };
        match snapshot.load_page(indices) {
            Ok(page) => self.loaded = page,
            Err(error) => {
                self.loaded.clear();
                self.message = format!("读取当前页失败：{error:#}");
            }
        }
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self, root: &Path) {
        let path = root.join(format!("log-viewer-fixture-{}.log", uuid::Uuid::new_v4()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .expect("create unique log viewer fixture");
        file.write_all(
            b"2026-10-17 09:00:00 INFO service started\n\
              2026-10-17 09:00:01 ERROR request failed\n\
              java.lang.IllegalStateException: synthetic fixture\n\
              \tat example.Service.run(Service.java:42)\n\
              Caused by: java.io.IOException: example only\n\
              2026-10-17 09:00:02 WARN retrying request\n",
        )
        .expect("write synthetic log viewer fixture");
        self.load_fixture(path);
    }

    #[cfg(feature = "ui-preview")]
    fn load_fixture(&mut self, path: PathBuf) {
        let cancel = AtomicBool::new(false);
        let snapshot = scan_file(&path, &cancel, |_, _| {}).expect("scan synthetic log fixture");
        let file = File::open(&path).expect("open synthetic log fixture for search");
        let identity = same_file::Handle::from_file(
            file.try_clone()
                .expect("clone synthetic log fixture handle"),
        )
        .expect("identify synthetic log fixture");
        let query = "IllegalStateException";
        let matches = search_events(
            SearchSource {
                path: path.clone(),
                file,
                identity,
                expected_size: snapshot.size,
                expected_modified: snapshot.modified,
                events: Arc::clone(&snapshot.events),
            },
            query.as_bytes(),
            &cancel,
            |_, _| {},
        )
        .expect("search synthetic log fixture");
        self.path = Some(path);
        self.snapshot = Some(snapshot);
        self.search_query = query.into();
        self.applied_search = Some(query.into());
        self.search_results = Some(matches);
        self.apply_filter();
        self.refresh_page();
        self.message = "合成示例：只读索引，不修改日志文件。".into();
    }
}

fn scan_file(
    path: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, usize),
) -> Result<Snapshot> {
    let metadata = fs::symlink_metadata(path).context("无法读取日志文件属性")?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "只允许打开普通日志文件"
    );
    let identity = same_file::Handle::from_path(path).context("无法确认日志文件身份")?;
    let mut file = File::open(path).context("打开日志文件失败")?;
    let opened = file.metadata().context("读取已打开日志文件属性失败")?;
    ensure!(
        same_file::Handle::from_path(path).is_ok_and(|current| current == identity),
        "日志文件在打开时已被替换"
    );
    ensure!(
        opened.len() == metadata.len() && opened.modified().ok() == metadata.modified().ok(),
        "日志文件在读取时发生变化"
    );

    file.seek(SeekFrom::Start(0)).context("定位日志开头失败")?;
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    let mut events = Vec::with_capacity(4096);
    let mut current: Option<EventIndex> = None;
    let mut offset = 0u64;
    let mut partial = false;
    let mut last_progress = 0u64;

    loop {
        if cancel.load(Ordering::Acquire) {
            bail!("日志扫描已取消");
        }
        let Some(line) = read_bounded_line(&mut reader, cancel)? else {
            break;
        };
        let start = offset;
        offset = offset.saturating_add(line.bytes_read);
        let timestamp = parse_timestamp(&line.prefix);
        let continuation = is_continuation(&line.prefix);
        if timestamp.is_some() || !continuation {
            if let Some(previous) = current.take() {
                events.push(previous);
            }
            if events.len() >= INDEX_LIMIT {
                partial = true;
                break;
            }
            current = Some(EventIndex {
                start,
                end: offset,
                timestamp,
                level: Level::from_first_line(&line.prefix),
                lines: 1,
                first_line_truncated: line.truncated,
            });
        } else if let Some(event) = &mut current {
            event.end = offset;
            event.lines = event.lines.saturating_add(1);
        } else {
            current = Some(EventIndex {
                start,
                end: offset,
                timestamp: None,
                level: Level::from_first_line(&line.prefix),
                lines: 1,
                first_line_truncated: line.truncated,
            });
        }
        if offset.saturating_sub(last_progress) >= 4 * 1024 * 1024 {
            last_progress = offset;
            progress(offset, events.len() + usize::from(current.is_some()));
        }
    }
    if let Some(event) = current {
        events.push(event);
    }
    let file = reader.into_inner();
    let after = file.metadata().context("复核日志文件属性失败")?;
    ensure!(
        after.len() == metadata.len() && after.modified().ok() == metadata.modified().ok(),
        "日志文件在索引过程中发生变化，请重新读取"
    );
    let after_identity = same_file::Handle::from_path(path).context("复核日志文件身份失败")?;
    ensure!(after_identity == identity, "日志文件在索引过程中被替换");
    progress(offset, events.len());
    Ok(Snapshot {
        path: path.to_owned(),
        size: metadata.len(),
        modified: metadata.modified().ok(),
        identity,
        events: events.into(),
        partial,
        scanned_bytes: offset,
    })
}

fn validate_search_query(query: &str) -> Result<()> {
    ensure!(!query.is_empty(), "请输入要搜索的正文关键词");
    ensure!(
        query.len() <= SEARCH_QUERY_LIMIT,
        "搜索词不能超过 {SEARCH_QUERY_LIMIT} 字节"
    );
    Ok(())
}

struct SearchSource {
    path: PathBuf,
    file: File,
    identity: same_file::Handle,
    expected_size: u64,
    expected_modified: Option<SystemTime>,
    events: Arc<[EventIndex]>,
}

fn search_events(
    source: SearchSource,
    query: &[u8],
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, usize),
) -> Result<Vec<usize>> {
    let SearchSource {
        path,
        mut file,
        identity,
        expected_size,
        expected_modified,
        events,
    } = source;
    ensure!(
        !query.is_empty() && query.len() <= SEARCH_QUERY_LIMIT,
        "搜索词长度无效"
    );
    ensure!(
        file.metadata()?.len() == expected_size
            && file.metadata()?.modified().ok() == expected_modified,
        "日志文件在搜索开始时发生变化"
    );
    ensure!(
        same_file::Handle::from_path(&path).is_ok_and(|current| current == identity),
        "日志文件在搜索开始时已被替换"
    );

    let prefix = kmp_prefix(query);
    let mut buffer = vec![0u8; SEARCH_CHUNK_SIZE];
    let mut matches = Vec::new();
    let mut scanned = 0u64;
    let mut last_progress = 0u64;
    for (index, event) in events.iter().enumerate() {
        if cancel.load(Ordering::Acquire) {
            bail!("正文搜索已取消");
        }
        ensure!(
            event.end <= expected_size && event.start <= event.end,
            "日志事件索引无效"
        );
        file.seek(SeekFrom::Start(event.start))
            .context("定位日志事件失败")?;
        let mut remaining = event.end - event.start;
        let mut state = 0usize;
        let mut found = false;
        while remaining > 0 && !found {
            if cancel.load(Ordering::Acquire) {
                bail!("正文搜索已取消");
            }
            let requested = remaining.min(buffer.len() as u64) as usize;
            let read = file
                .read(&mut buffer[..requested])
                .context("读取日志正文进行搜索失败")?;
            ensure!(read > 0, "日志文件在搜索时提前结束");
            remaining -= read as u64;
            scanned = scanned.saturating_add(read as u64);
            found = kmp_feed(query, &prefix, &buffer[..read], &mut state);
            if scanned.saturating_sub(last_progress) >= 4 * 1024 * 1024 {
                last_progress = scanned;
                progress(scanned, matches.len() + usize::from(found));
            }
        }
        if found {
            matches.push(index);
        }
    }

    let metadata = file.metadata().context("复核搜索文件属性失败")?;
    ensure!(
        metadata.len() == expected_size && metadata.modified().ok() == expected_modified,
        "日志文件在搜索期间发生变化，请重新索引"
    );
    ensure!(
        same_file::Handle::from_path(&path).is_ok_and(|current| current == identity),
        "日志文件在搜索期间被替换"
    );
    progress(scanned, matches.len());
    Ok(matches)
}

fn kmp_prefix(pattern: &[u8]) -> Vec<usize> {
    let mut prefix = vec![0; pattern.len()];
    let mut matched = 0usize;
    for index in 1..pattern.len() {
        while matched > 0 && pattern[index] != pattern[matched] {
            matched = prefix[matched - 1];
        }
        if pattern[index] == pattern[matched] {
            matched += 1;
        }
        prefix[index] = matched;
    }
    prefix
}

fn kmp_feed(pattern: &[u8], prefix: &[usize], bytes: &[u8], matched: &mut usize) -> bool {
    for &byte in bytes {
        while *matched > 0 && byte != pattern[*matched] {
            *matched = prefix[*matched - 1];
        }
        if byte == pattern[*matched] {
            *matched += 1;
        }
        if *matched == pattern.len() {
            return true;
        }
    }
    false
}

struct BoundedLine {
    prefix: Vec<u8>,
    bytes_read: u64,
    truncated: bool,
}

fn read_bounded_line(
    reader: &mut impl BufRead,
    cancel: &AtomicBool,
) -> io::Result<Option<BoundedLine>> {
    let mut prefix = Vec::with_capacity(LINE_PREFIX_LIMIT);
    let mut bytes_read = 0u64;
    let mut content_bytes = 0u64;
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if bytes_read == 0 {
                return Ok(None);
            }
            break;
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |position| position + 1);
        let content = newline.unwrap_or(available.len());
        let retained = LINE_PREFIX_LIMIT.saturating_sub(prefix.len()).min(content);
        prefix.extend_from_slice(&available[..retained]);
        content_bytes = content_bytes.saturating_add(content as u64);
        bytes_read = bytes_read.saturating_add(consumed as u64);
        reader.consume(consumed);
        if newline.is_some() {
            break;
        }
    }
    if prefix.last() == Some(&b'\r') {
        prefix.pop();
    }
    Ok(Some(BoundedLine {
        prefix,
        bytes_read,
        truncated: content_bytes > LINE_PREFIX_LIMIT as u64,
    }))
}

fn parse_timestamp(bytes: &[u8]) -> Option<NaiveDateTime> {
    let line = String::from_utf8_lossy(bytes);
    let line = line.trim_start();
    let line = line.strip_prefix('[').unwrap_or(line);
    let raw = line.get(..19)?;
    let normalized = raw.replacen('T', " ", 1);
    NaiveDateTime::parse_from_str(&normalized, "%Y-%m-%d %H:%M:%S").ok()
}

fn is_continuation(bytes: &[u8]) -> bool {
    let line = String::from_utf8_lossy(bytes);
    let trimmed = line.trim_start();
    trimmed.is_empty()
        || line.starts_with(' ')
        || line.starts_with('\t')
        || trimmed.starts_with("at ")
        || trimmed.starts_with("Caused by:")
        || trimmed.starts_with("Suppressed:")
        || trimmed.starts_with("...")
        || trimmed.starts_with('^')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let folder =
                std::env::temp_dir().join(format!("zi-log-viewer-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&folder).unwrap();
            Self(folder)
        }

        fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            file.write_all(bytes).unwrap();
            path
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn scans_timestamped_multiline_events_and_levels_without_loading_full_text() {
        let fixture = Fixture::new();
        let path = fixture.file(
            "app.log",
            b"2026-10-17T09:00:00 INFO ready\n2026-10-17 09:00:01 ERROR failed\n  at app.Main.run(Main.java:5)\nCaused by: timeout\n2026-10-17 09:00:02 WARN retry\n",
        );
        let cancel = AtomicBool::new(false);
        let snapshot = scan_file(&path, &cancel, |_, _| {}).unwrap();
        assert_eq!(snapshot.events.len(), 3);
        assert_eq!(snapshot.events[0].level, Level::Info);
        assert_eq!(snapshot.events[1].level, Level::Error);
        assert_eq!(snapshot.events[1].lines, 3);
        assert_eq!(snapshot.events[1].timestamp.unwrap().second(), 1);
        assert_eq!(snapshot.events[2].level, Level::Warning);
        let page = snapshot.load_page(&[1]).unwrap();
        assert!(page[0].text.contains("Main.run"));
        assert!(page[0].text.contains("Caused by: timeout"));
    }

    #[test]
    fn filters_levels_and_locates_first_visible_event_at_or_after_time() {
        let fixture = Fixture::new();
        let path = fixture.file(
            "app.log",
            b"2026-10-17 09:00:00 INFO ready\n2026-10-17 09:00:01 ERROR failed\n2026-10-17 09:00:02 WARN retry\n",
        );
        let cancel = AtomicBool::new(false);
        let snapshot = scan_file(&path, &cancel, |_, _| {}).unwrap();
        let filtered = snapshot
            .events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| (event.level == Level::Warning).then_some(index))
            .collect::<Vec<_>>();
        assert_eq!(filtered, vec![2]);
        let target =
            NaiveDateTime::parse_from_str("2026-10-17 09:00:01", "%Y-%m-%d %H:%M:%S").unwrap();
        let selected = snapshot
            .events
            .iter()
            .position(|event| event.timestamp.is_some_and(|time| time >= target));
        assert_eq!(selected, Some(1));
    }

    #[test]
    fn large_lines_and_pages_have_explicit_memory_bounds() {
        let fixture = Fixture::new();
        let path = fixture.0.join("large.log");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.write_all(b"2026-10-17 09:00:00 INFO ").unwrap();
        file.write_all(&vec![b'x'; 2 * 1024 * 1024]).unwrap();
        file.write_all(b"\n2026-10-17 09:00:01 INFO end\n").unwrap();
        let cancel = AtomicBool::new(false);
        let snapshot = scan_file(&path, &cancel, |_, _| {}).unwrap();
        assert_eq!(snapshot.events.len(), 2);
        assert!(snapshot.events[0].first_line_truncated);
        let page = snapshot.load_page(&[0]).unwrap();
        assert!(page[0].truncated);
        assert!(page[0].text.len() <= EVENT_PREVIEW_LIMIT + 4);
    }

    #[test]
    fn scan_rejects_non_files_and_detects_source_changes_before_page_read() {
        let fixture = Fixture::new();
        let path = fixture.file("app.log", b"2026-10-17 09:00:00 INFO ready\n");
        let cancel = AtomicBool::new(false);
        let snapshot = scan_file(&path, &cancel, |_, _| {}).unwrap();
        fs::write(&path, b"replacement content with a different size").unwrap();
        assert!(snapshot.load_page(&[0]).is_err());
        let directory = fixture.0.clone();
        assert!(scan_file(&directory, &cancel, |_, _| {}).is_err());
    }

    #[test]
    fn cancellation_is_observed_during_long_line_streaming() {
        let fixture = Fixture::new();
        let path = fixture.file("app.log", &vec![b'x'; 2 * 1024 * 1024]);
        let cancel = AtomicBool::new(true);
        assert!(scan_file(&path, &cancel, |_, _| {}).is_err());
    }

    #[test]
    fn search_cancellation_is_observed_before_reading_events() {
        let fixture = Fixture::new();
        let path = fixture.file("app.log", b"2026-10-17 09:00:00 INFO ready\n");
        let scan_cancel = AtomicBool::new(false);
        let snapshot = scan_file(&path, &scan_cancel, |_, _| {}).unwrap();
        let file = File::open(&path).unwrap();
        let identity = same_file::Handle::from_file(file.try_clone().unwrap()).unwrap();
        let cancel = AtomicBool::new(true);
        assert!(
            search_events(
                SearchSource {
                    path: path.clone(),
                    file,
                    identity,
                    expected_size: snapshot.size,
                    expected_modified: snapshot.modified,
                    events: Arc::clone(&snapshot.events),
                },
                b"ready",
                &cancel,
                |_, _| {},
            )
            .is_err()
        );
    }

    #[test]
    fn severity_tokens_do_not_match_inside_words() {
        assert_eq!(
            Level::from_first_line(b"[WARN] slow request"),
            Level::Warning
        );
        assert_eq!(Level::from_first_line(b"INFO information"), Level::Info);
        assert_eq!(
            Level::from_first_line(b"an errorless response"),
            Level::Unknown
        );
        assert_eq!(
            Level::from_first_line(b"FATAL startup failed"),
            Level::Fatal
        );
    }

    #[test]
    fn searches_full_event_text_across_chunks_and_keeps_case_sensitive_semantics() {
        let fixture = Fixture::new();
        let mut content =
            b"2026-10-17 09:00:00 INFO first\n2026-10-17 09:00:01 ERROR failed\n  detail ".to_vec();
        content.extend(vec![b'x'; 2 * 1024 * 1024]);
        content.extend_from_slice(
            b"needle-matches-here\n2026-10-17 09:00:02 INFO Needle differs by case\n",
        );
        let path = fixture.file("app.log", &content);
        let cancel = AtomicBool::new(false);
        let snapshot = scan_file(&path, &cancel, |_, _| {}).unwrap();
        let file = File::open(&path).unwrap();
        let identity = same_file::Handle::from_file(file.try_clone().unwrap()).unwrap();
        let found = search_events(
            SearchSource {
                path: path.clone(),
                file,
                identity,
                expected_size: snapshot.size,
                expected_modified: snapshot.modified,
                events: Arc::clone(&snapshot.events),
            },
            b"needle",
            &cancel,
            |_, _| {},
        )
        .unwrap();
        assert_eq!(found, vec![1]);

        let mut state = 0;
        let prefix = kmp_prefix(b"needle");
        assert!(!kmp_feed(b"needle", &prefix, b"need", &mut state));
        assert!(kmp_feed(b"needle", &prefix, b"le", &mut state));
    }

    #[test]
    fn search_queries_are_nonempty_and_bounded_by_utf8_bytes() {
        assert!(validate_search_query("").is_err());
        assert!(validate_search_query(&"a".repeat(SEARCH_QUERY_LIMIT)).is_ok());
        assert!(validate_search_query(&"你".repeat(SEARCH_QUERY_LIMIT / 3 + 1)).is_err());
    }

    use chrono::Timelike;
}
