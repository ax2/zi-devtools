mod transform;

use anyhow::{Context, Result, anyhow, bail};
use eframe::egui::{self, RichText};
use serde_json::Value;
use sha2::{Digest, Sha256, Sha512};
use std::{
    collections::BTreeSet,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
};

const INPUT_LIMIT: usize = 2 * 1024 * 1024;
const ROW_LIMIT: usize = 10_000;
const COLUMN_LIMIT: usize = 128;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum DataFormat {
    #[default]
    Csv,
    Json,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Dataset {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}
impl Dataset {
    pub fn parse(input: &str, format: DataFormat, delimiter: u8) -> Result<Self> {
        if input.len() > INPUT_LIMIT {
            bail!("输入最多 2 MiB");
        }
        if input.trim().is_empty() {
            bail!("请先粘贴内容或载入文件");
        }
        let data = match format {
            DataFormat::Csv => {
                let mut reader = csv::ReaderBuilder::new()
                    .delimiter(delimiter)
                    .from_reader(input.trim_start_matches('\u{feff}').as_bytes());
                let headers = reader
                    .headers()?
                    .iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                if headers.iter().any(|s| s.trim().is_empty())
                    || headers.iter().collect::<BTreeSet<_>>().len() != headers.len()
                {
                    bail!("CSV 表头不能为空或重复；请先修正列名");
                }
                if headers.len() > COLUMN_LIMIT {
                    bail!("最多 128 列");
                }
                let mut rows = Vec::new();
                for (i, record) in reader.records().enumerate() {
                    if i >= ROW_LIMIT {
                        bail!("最多 10000 行数据");
                    }
                    let record =
                        record.with_context(|| format!("CSV 第 {} 条数据记录无效", i + 1))?;
                    rows.push(record.iter().map(|s| Value::String(s.to_owned())).collect());
                }
                Self { headers, rows }
            }
            DataFormat::Json => {
                let value: Value = serde_json::from_str(input.trim_start_matches('\u{feff}'))?;
                let items = value
                    .as_array()
                    .ok_or_else(|| anyhow!("JSON 须为对象数组，例如 [{{\"name\":\"Zi\"}}]"))?;
                if items.is_empty() {
                    bail!("对象数组为空，无法推断列名");
                }
                if items.len() > ROW_LIMIT {
                    bail!("最多 10000 行数据");
                }
                let mut names = BTreeSet::new();
                for (i, item) in items.iter().enumerate() {
                    let object = item
                        .as_object()
                        .ok_or_else(|| anyhow!("第 {} 项不是对象", i + 1))?;
                    names.extend(object.keys().cloned());
                }
                let headers = names.into_iter().collect::<Vec<_>>();
                let rows = items
                    .iter()
                    .map(|item| {
                        headers
                            .iter()
                            .map(|h| item.get(h).cloned().unwrap_or(Value::Null))
                            .collect()
                    })
                    .collect();
                Self { headers, rows }
            }
        };
        if data.headers.is_empty() || data.headers.len() > COLUMN_LIMIT {
            bail!("数据须包含 1–128 列");
        }
        Ok(data)
    }
    pub fn export(&self, indices: &[usize], format: DataFormat, delimiter: u8) -> Result<String> {
        let out = match format {
            DataFormat::Json => {
                let rows = indices
                    .iter()
                    .map(|&i| {
                        self.headers
                            .iter()
                            .cloned()
                            .zip(self.rows[i].iter().cloned())
                            .collect::<serde_json::Map<_, _>>()
                    })
                    .collect::<Vec<_>>();
                serde_json::to_string_pretty(&rows)?
            }
            DataFormat::Csv => {
                let mut w = csv::WriterBuilder::new()
                    .delimiter(delimiter)
                    .from_writer(Vec::new());
                w.write_record(&self.headers)?;
                for &i in indices {
                    w.write_record(self.rows[i].iter().map(cell_text))?;
                }
                String::from_utf8(w.into_inner()?)?
            }
        };
        if out.len() > 8 * 1024 * 1024 {
            bail!("导出超过 8 MiB，请筛选后再试");
        }
        Ok(out)
    }
    pub fn view(&self, query: &str, sort: Option<usize>, descending: bool) -> Vec<usize> {
        let query = query.to_lowercase();
        let mut rows = (0..self.rows.len())
            .filter(|&i| {
                query.is_empty()
                    || self.rows[i]
                        .iter()
                        .any(|v| cell_text(v).to_lowercase().contains(&query))
            })
            .collect::<Vec<_>>();
        if let Some(col) = sort {
            rows.sort_by(|&a, &b| {
                let order = match (self.rows[a][col].as_f64(), self.rows[b][col].as_f64()) {
                    (Some(a), Some(b)) => a.total_cmp(&b),
                    _ => cell_text(&self.rows[a][col]).cmp(&cell_text(&self.rows[b][col])),
                };
                if descending { order.reverse() } else { order }
            });
        }
        rows
    }
}
fn cell_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}

pub fn read_text_file(path: &str) -> Result<String> {
    let mut bytes = Vec::new();
    File::open(path.trim())?
        .take((INPUT_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > INPUT_LIMIT {
        bail!("文件超过 2 MiB，请先缩小范围");
    }
    String::from_utf8(bytes).map_err(|_| anyhow!("文件不是 UTF-8，请转换编码后再导入"))
}
pub fn save_new_file(path: &str, content: &str) -> Result<()> {
    if path.trim().is_empty() {
        bail!("请输入导出文件的完整路径");
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path.trim())
        .context("无法创建文件：请检查父目录与路径；已有文件不会被覆盖")?;
    file.write_all(content.as_bytes())?;
    Ok(())
}

#[derive(Default)]
pub struct DataState {
    pub input: String,
    pub format: DataFormat,
    pub output: String,
    pub message: String,
    path: String,
    export_path: String,
    tab_delimiter: bool,
    dataset: Option<Dataset>,
    transform: transform::State,
    query: String,
    visible: Vec<usize>,
    sort: Option<usize>,
    descending: bool,
    receiver: Option<Receiver<std::result::Result<Dataset, String>>>,
}
impl DataState {
    pub fn import_text(&mut self, text: String, format: DataFormat, tsv: bool) -> Result<()> {
        if self.receiver.is_some() {
            bail!("数据工作台正在解析，请稍后重试");
        }
        self.input = text;
        self.format = format;
        self.tab_delimiter = tsv;
        self.parse();
        Ok(())
    }

    pub fn sample(&mut self) {
        self.format = DataFormat::Csv;
        self.input="name,language,stars\nZi Tools,Rust,120\nData Studio,Python,85\nLocal Notes,Markdown,64\n".into();
        self.parse();
    }
    fn parse(&mut self) {
        let input = self.input.clone();
        let format = self.format;
        let delimiter = if self.tab_delimiter { b'\t' } else { b',' };
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        self.message.clear();
        self.dataset = None;
        self.transform = Default::default();
        self.output.clear();
        std::thread::spawn(move || {
            let _ = tx.send(Dataset::parse(&input, format, delimiter).map_err(|e| e.to_string()));
        });
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(result) = self.receiver.as_ref().and_then(|r| r.try_recv().ok()) {
            self.receiver = None;
            match result {
                Ok(data) => {
                    self.visible = (0..data.rows.len()).collect();
                    self.dataset = Some(data);
                    self.query.clear();
                    self.sort = None;
                    self.output.clear();
                }
                Err(e) => self.message = e,
            }
        }
        heading(
            ui,
            "数据工作台",
            "CSV / TSV 与 JSON 对象数组 · 筛选、排序、预览和导出",
        );
        card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut self.format, DataFormat::Csv, "CSV / TSV");
                ui.selectable_value(&mut self.format, DataFormat::Json, "JSON 数组");
                if self.format == DataFormat::Csv {
                    ui.checkbox(&mut self.tab_delimiter, "Tab 分隔");
                }
                if ui
                    .add_enabled(
                        self.input.is_empty() && self.receiver.is_none(),
                        egui::Button::new("载入示例"),
                    )
                    .on_hover_text("仅在输入为空时可用，避免覆盖当前草稿")
                    .clicked()
                {
                    self.sample();
                }
            });
            ui.add_space(8.0);
            ui.add_sized(
                [ui.available_width(), 150.0],
                egui::TextEdit::multiline(&mut self.input)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("粘贴 CSV（首行为表头）或 JSON 对象数组…"),
            );
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.path)
                        .hint_text("UTF-8 文件路径")
                        .desired_width(340.0),
                );
                if ui
                    .add_enabled(
                        self.input.is_empty() && self.receiver.is_none(),
                        egui::Button::new("载入文件"),
                    )
                    .on_hover_text("输入有内容时请先清空；最多 2 MiB")
                    .clicked()
                {
                    match read_text_file(&self.path) {
                        Ok(s) => self.input = s,
                        Err(e) => self.message = e.to_string(),
                    }
                }
                if ui
                    .add_enabled(
                        self.receiver.is_none() && !self.input.trim().is_empty(),
                        primary(ui, "解析数据"),
                    )
                    .clicked()
                {
                    self.parse();
                }
                if self.receiver.is_some() {
                    ui.spinner();
                    ui.label("解析中…");
                }
            });
            ui.label(
                RichText::new(
                    "CSV 值保留为字符串；JSON 类型保留，缺失字段补 null。预览是上次解析的快照。",
                )
                .small()
                .weak(),
            );
        });
        self.transform_ui(ui);
        if let Some(data) = &self.dataset {
            ui.add_space(14.0);
            let mut changed = false;
            ui.horizontal_wrapped(|ui| {
                ui.strong(format!(
                    "{} 行 · {} 列",
                    data.rows.len(),
                    data.headers.len()
                ));
                changed |= ui
                    .add(
                        egui::TextEdit::singleline(&mut self.query)
                            .hint_text("筛选所有列…")
                            .desired_width(220.0),
                    )
                    .changed();
                egui::ComboBox::from_id_salt("data-sort")
                    .selected_text(
                        self.sort
                            .map(|i| data.headers[i].as_str())
                            .unwrap_or("原始顺序"),
                    )
                    .show_ui(ui, |ui| {
                        changed |= ui
                            .selectable_value(&mut self.sort, None, "原始顺序")
                            .changed();
                        for (i, h) in data.headers.iter().enumerate() {
                            changed |= ui.selectable_value(&mut self.sort, Some(i), h).changed();
                        }
                    });
                changed |= ui.checkbox(&mut self.descending, "降序").changed();
            });
            if changed {
                self.output.clear();
                self.visible = data.view(&self.query, self.sort, self.descending);
            }
            ui.label(
                RichText::new(format!(
                    "筛选后 {} 行；下表最多预览 200 行，导出包含全部筛选结果",
                    self.visible.len()
                ))
                .small()
                .weak(),
            );
            egui::ScrollArea::both()
                .id_salt("data-preview")
                .max_height(270.0)
                .show(ui, |ui| {
                    egui::Grid::new("data-grid")
                        .striped(true)
                        .min_col_width(110.0)
                        .max_col_width(220.0)
                        .show(ui, |ui| {
                            for h in &data.headers {
                                ui.strong(h);
                            }
                            ui.end_row();
                            for &i in self.visible.iter().take(200) {
                                for value in &data.rows[i] {
                                    let text = cell_text(value);
                                    let short = text.chars().take(100).collect::<String>();
                                    ui.label(short).on_hover_text(text);
                                }
                                ui.end_row();
                            }
                        });
                });
            ui.horizontal_wrapped(|ui| {
                for (format, label) in [
                    (DataFormat::Json, "生成 JSON"),
                    (DataFormat::Csv, "生成 CSV / TSV"),
                ] {
                    if ui.button(label).clicked() {
                        match data.export(
                            &self.visible,
                            format,
                            if self.tab_delimiter { b'\t' } else { b',' },
                        ) {
                            Ok(s) => {
                                self.output = s;
                                self.message = "结果已生成，可复制或另存为新文件".into();
                            }
                            Err(e) => self.message = e.to_string(),
                        }
                    }
                }
            });
        }
        if !self.output.is_empty() {
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button("复制导出结果").clicked() {
                    ctx.copy_text(self.output.clone());
                    self.message = "已复制导出结果".into();
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.export_path)
                        .hint_text("导出到新文件的完整路径")
                        .desired_width(300.0),
                );
                if ui.button("另存为新文件").clicked() {
                    self.message = match save_new_file(&self.export_path, &self.output) {
                        Ok(()) => "已写入新文件".into(),
                        Err(e) => e.to_string(),
                    };
                }
            });
            egui::ScrollArea::vertical()
                .id_salt("data-output")
                .max_height(180.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.output)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .interactive(false),
                    );
                });
        }
        if !self.message.is_empty() {
            ui.add_space(8.0);
            ui.label(&self.message);
        }
    }
}

pub struct FileDigest {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub sha512: String,
}
pub fn hash_file(
    path: PathBuf,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> Result<FileDigest> {
    let mut file = File::open(&path).context("无法读取文件")?;
    let before = file.metadata()?;
    if !before.is_file() {
        bail!("仅支持普通文件");
    }
    let mut sha256 = Sha256::new();
    let mut sha512 = Sha512::new();
    let mut buffer = [0u8; 65536];
    let mut bytes = 0u64;
    let mut last = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            bail!("已取消");
        }
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        sha256.update(&buffer[..read]);
        sha512.update(&buffer[..read]);
        bytes += read as u64;
        if bytes - last >= 4 * 1024 * 1024 {
            progress(bytes, before.len());
            last = bytes;
        }
    }
    let after = file.metadata()?;
    if bytes != before.len()
        || after.len() != before.len()
        || after.modified().ok() != before.modified().ok()
    {
        bail!("文件在读取过程中发生变化，请重试");
    }
    progress(bytes, bytes);
    Ok(FileDigest {
        path,
        bytes,
        sha256: format!("{:x}", sha256.finalize()),
        sha512: format!("{:x}", sha512.finalize()),
    })
}
enum HashEvent {
    Progress(usize, u64, u64),
    Result(PathBuf, std::result::Result<FileDigest, String>),
    Done,
}
#[derive(Default)]
pub struct FileState {
    pub paths: String,
    pub expected: String,
    pub message: String,
    results: Vec<(PathBuf, std::result::Result<FileDigest, String>)>,
    receiver: Option<Receiver<HashEvent>>,
    cancel: Arc<AtomicBool>,
    progress: f32,
    index: usize,
    total: usize,
}
impl FileState {
    pub fn append_paths(&mut self, paths: &[PathBuf]) -> Result<()> {
        if self.receiver.is_some() {
            bail!("文件校验正在运行，请稍后重试");
        }
        let mut all: Vec<String> = self
            .paths
            .lines()
            .filter(|s| !s.trim().is_empty())
            .map(str::to_owned)
            .collect();
        for path in paths {
            let name = path.to_string_lossy().to_string();
            if !all.contains(&name) {
                all.push(name);
            }
        }
        if all.len() > 64 {
            bail!("追加后超过 64 个文件，请先整理校验列表");
        }
        self.paths = all.join("\n");
        Ok(())
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview(&mut self, path: PathBuf) {
        self.paths = path.display().to_string();
        self.expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into();
        self.start();
    }
    fn start(&mut self) {
        let paths = self
            .paths
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| PathBuf::from(s.trim_matches('"')))
            .collect::<Vec<_>>();
        if paths.is_empty() || paths.len() > 64 {
            self.message = "请输入 1–64 个文件路径，每行一个".into();
            return;
        }
        self.total = paths.len();
        self.index = 0;
        self.progress = 0.0;
        self.results.clear();
        self.message.clear();
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        std::thread::spawn(move || {
            for (i, path) in paths.into_iter().enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let result = hash_file(path.clone(), &cancel, |done, total| {
                    let _ = tx.send(HashEvent::Progress(i, done, total));
                })
                .map_err(|e| e.to_string());
                if tx.send(HashEvent::Result(path, result)).is_err() {
                    return;
                }
            }
            let _ = tx.send(HashEvent::Done);
        });
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let events = self
            .receiver
            .as_ref()
            .map(|r| r.try_iter().collect::<Vec<_>>())
            .unwrap_or_default();
        for event in events {
            match event {
                HashEvent::Progress(i, done, total) => {
                    self.index = i;
                    self.progress = if total == 0 {
                        1.0
                    } else {
                        done as f32 / total as f32
                    };
                }
                HashEvent::Result(path, result) => self.results.push((path, result)),
                HashEvent::Done => {
                    self.receiver = None;
                    let failed = self.results.iter().filter(|(_, r)| r.is_err()).count();
                    self.message = if self.cancel.load(Ordering::Relaxed) {
                        "已取消，保留已完成的结果".into()
                    } else {
                        format!("完成 {} 个文件，{} 个失败", self.results.len(), failed)
                    };
                }
            }
        }
        heading(
            ui,
            "文件校验",
            "批量 SHA-256 / SHA-512 · 流式读取 · 只读文件 · 支持取消",
        );
        if self.receiver.is_none() {
            for f in ctx.input(|i| i.raw.dropped_files.clone()) {
                if let Some(path) = f.path {
                    if !self.paths.is_empty() {
                        self.paths.push('\n');
                    }
                    self.paths.push_str(&path.to_string_lossy());
                }
            }
        }
        card(ui, |ui| {
            ui.label("文件路径 · 每行一个，也可以将文件拖入窗口");
            ui.add_sized(
                [ui.available_width(), 110.0],
                egui::TextEdit::multiline(&mut self.paths)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("C:\\Downloads\\release.exe")
                    .interactive(self.receiver.is_none()),
            );
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        self.receiver.is_none() && !self.paths.trim().is_empty(),
                        primary(ui, "开始校验"),
                    )
                    .clicked()
                {
                    self.start();
                }
                if ui
                    .add_enabled(self.receiver.is_some(), egui::Button::new("取消任务"))
                    .clicked()
                {
                    self.cancel.store(true, Ordering::Relaxed);
                }
                ui.label(
                    RichText::new("每批最多 64 个文件；文件内容不会上传")
                        .small()
                        .weak(),
                );
            });
            if self.receiver.is_some() {
                ui.add(
                    egui::ProgressBar::new(
                        (self.index as f32 + self.progress) / self.total.max(1) as f32,
                    )
                    .text(format!(
                        "正在读取第 {} / {} 个文件",
                        self.index + 1,
                        self.total
                    )),
                );
            }
            ui.add_space(6.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.expected)
                    .hint_text("可选：粘贴期望的 SHA-256（64 位）或 SHA-512（128 位）")
                    .desired_width(f32::INFINITY),
            );
        });
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        let expected = self.expected.trim();
        let valid_expected = (expected.len() == 64 || expected.len() == 128)
            && expected.bytes().all(|b| b.is_ascii_hexdigit());
        if !expected.is_empty() && !valid_expected {
            ui.colored_label(
                ui.visuals().error_fg_color,
                "期望值须为 64 或 128 位十六进制摘要",
            );
        }
        for (i, (path, result)) in self.results.iter().enumerate() {
            ui.push_id(i, |ui| {
                ui.add_space(12.0);
                card(ui, |ui| {
                    ui.strong(path.file_name().unwrap_or_default().to_string_lossy())
                        .on_hover_text(path.display().to_string());
                    match result {
                        Ok(d) => {
                            ui.label(RichText::new(format!("{} 字节", d.bytes)).small().weak());
                            if valid_expected {
                                let matches = if expected.len() == 64 {
                                    expected.eq_ignore_ascii_case(&d.sha256)
                                } else {
                                    expected.eq_ignore_ascii_case(&d.sha512)
                                };
                                ui.colored_label(
                                    if matches {
                                        ui.visuals().hyperlink_color
                                    } else {
                                        ui.visuals().error_fg_color
                                    },
                                    if matches {
                                        "摘要匹配"
                                    } else {
                                        "摘要不匹配"
                                    },
                                );
                            }
                            for (name, digest) in [("SHA-256", &d.sha256), ("SHA-512", &d.sha512)] {
                                ui.horizontal(|ui| {
                                    ui.strong(name);
                                    if ui.small_button("复制").clicked() {
                                        ctx.copy_text(digest.clone());
                                        self.message = format!("已复制 {name}");
                                    }
                                });
                                ui.add(egui::Label::new(RichText::new(digest).monospace()).wrap());
                            }
                        }
                        Err(e) => {
                            ui.colored_label(ui.visuals().error_fg_color, e);
                        }
                    }
                });
            });
        }
    }
}
impl Drop for FileState {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub fn heading(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.heading(RichText::new(title).size(28.0));
    ui.label(RichText::new(subtitle).weak());
    ui.add_space(18.0);
}
pub fn card(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(ui.visuals().window_fill)
        .stroke(egui::Stroke::new(
            1.0,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ))
        .corner_radius(12)
        .inner_margin(16.0)
        .show(ui, body);
}
fn primary(ui: &egui::Ui, text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_owned()).color(egui::Color32::WHITE))
        .fill(ui.visuals().selection.bg_fill)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn intake_preserves_busy_drafts_and_parses_tsv() {
        let mut data = DataState::default();
        data.import_text("name\tcount\n样例\t3\n".into(), DataFormat::Csv, true)
            .unwrap();
        assert!(
            data.import_text("replacement".into(), DataFormat::Json, false)
                .is_err()
        );
        assert!(data.input.contains("样例"));
        let parsed = data
            .receiver
            .as_ref()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap()
            .unwrap();
        assert_eq!(parsed.headers, vec!["name", "count"]);
        assert_eq!(parsed.rows.len(), 1);
        let mut files = FileState::default();
        files
            .append_paths(&[PathBuf::from("a.txt"), PathBuf::from("a.txt")])
            .unwrap();
        assert_eq!(files.paths, "a.txt");
        assert!(
            files
                .append_paths(
                    &(0..65)
                        .map(|i| PathBuf::from(format!("{i}.txt")))
                        .collect::<Vec<_>>()
                )
                .is_err()
        );
        assert_eq!(files.paths, "a.txt");
    }

    #[test]
    fn csv_quotes_unicode_and_shape() {
        let d = Dataset::parse(
            "name,note\r\n字与码,\"one,two\nthree\"\r\n",
            DataFormat::Csv,
            b',',
        )
        .unwrap();
        assert_eq!(d.rows[0][1], "one,two\nthree");
        let csv = d.export(&[0], DataFormat::Csv, b',').unwrap();
        assert_eq!(
            Dataset::parse(&csv, DataFormat::Csv, b',').unwrap().rows,
            d.rows
        );
        for s in ["a,a\n1,2", "a,b\n1,2,3", ",b\n1,2"] {
            assert!(Dataset::parse(s, DataFormat::Csv, b',').is_err());
        }
    }
    #[test]
    fn json_types_filter_sort_and_empty_export() {
        let d = Dataset::parse(
            r#"[{"name":"ten","n":10},{"name":"two","n":2,"enabled":true}]"#,
            DataFormat::Json,
            b',',
        )
        .unwrap();
        let n = d.headers.iter().position(|h| h == "n").unwrap();
        assert_eq!(d.view("", Some(n), false), vec![1, 0]);
        assert_eq!(d.view("two", None, false), vec![1]);
        let out: Value =
            serde_json::from_str(&d.export(&[1], DataFormat::Json, b',').unwrap()).unwrap();
        assert_eq!(out[0]["enabled"], true);
        assert_eq!(out[0]["n"], 2);
        assert_eq!(d.export(&[], DataFormat::Json, b',').unwrap(), "[]");
    }
    #[test]
    fn limits_and_utf8_import() {
        assert!(Dataset::parse("[]", DataFormat::Json, b',').is_err());
        assert!(Dataset::parse("[1]", DataFormat::Json, b',').is_err());
        assert!(Dataset::parse(&"x".repeat(INPUT_LIMIT + 1), DataFormat::Csv, b',').is_err());
    }
    #[test]
    fn file_hash_matches_known_digest_and_cancel_and_no_overwrite() {
        let root = std::env::temp_dir().join(format!("zi-hash-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let p = root.join("abc.txt");
        fs::write(&p, b"abc").unwrap();
        let d = hash_file(p.clone(), &AtomicBool::new(false), |_, _| {}).unwrap();
        assert_eq!(d.bytes, 3);
        assert_eq!(
            d.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(hash_file(p.clone(), &AtomicBool::new(true), |_, _| {}).is_err());
        assert!(save_new_file(p.to_str().unwrap(), "replace").is_err());
        assert_eq!(fs::read(&p).unwrap(), b"abc");
        fs::remove_dir_all(root).unwrap();
    }
}
