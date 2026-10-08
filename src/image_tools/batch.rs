//! Explicit, conflict-checked image batch processing. Each output is created once.
use super::{Format, memory, single, workflow};
use anyhow::{Context, Result, ensure};
use eframe::egui;
use image::GenericImageView;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

const MAX_BATCH: usize = 100;

#[derive(Clone, Copy)]
struct SavedOutput {
    bytes: usize,
    sha256: [u8; 32],
}

#[derive(Clone)]
struct Item {
    source: PathBuf,
    selected: Option<crate::material_files::FileMaterial>,
    source_sha256: Option<[u8; 32]>,
    target: PathBuf,
    dimensions: (u32, u32),
    original_bytes: u64,
    outcome: String,
    conflict: bool,
    saved_bytes: Option<usize>,
    saved_sha256: Option<[u8; 32]>,
    failed: bool,
}

enum Event {
    Inspected(Result<Vec<Item>, String>),
    Saved(usize, Result<SavedOutput, String>),
    Finished { cancelled: bool },
}

pub(super) struct State {
    inputs: Vec<PathBuf>,
    output_dir: String,
    max_width: u32,
    format: Format,
    jpeg_quality: u8,
    items: Vec<Item>,
    receiver: Option<mpsc::Receiver<Event>>,
    cancel: Arc<AtomicBool>,
    running: bool,
    ready: bool,
    completed: usize,
    failed: usize,
    message: String,
    phase: &'static str,
    report: Option<String>,
    report_current: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            inputs: Vec::new(),
            output_dir: String::new(),
            max_width: 1920,
            format: Format::Jpeg,
            jpeg_quality: 80,
            items: Vec::new(),
            receiver: None,
            cancel: Arc::new(AtomicBool::new(false)),
            running: false,
            ready: false,
            completed: 0,
            failed: 0,
            message: String::new(),
            phase: "empty",
            report: None,
            report_current: false,
        }
    }
}

impl State {
    #[cfg(feature = "ui-preview")]
    pub(super) fn preview_relay_fixture(&mut self, root: &Path) -> Arc<Vec<u8>> {
        fs::create_dir_all(root).unwrap();
        let source = root.join("批量示例.png");
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            320,
            180,
            image::Rgba([36, 108, 180, 160]),
        ))
        .save_with_format(&source, image::ImageFormat::Png)
        .unwrap();
        self.inputs = vec![source.clone()];
        self.output_dir = root.to_string_lossy().into_owned();
        self.format = Format::WebP;
        self.max_width = 240;
        self.items = inspect_batch(&[source], root, Format::WebP).unwrap();
        let (tx, rx) = mpsc::channel();
        run_batch(
            &self.items,
            240,
            Format::WebP,
            80,
            &AtomicBool::new(false),
            &tx,
        );
        self.receiver = Some(rx);
        self.running = true;
        self.poll(&egui::Context::default());
        self.preview_output_bytes()
    }
    #[cfg(feature = "ui-preview")]
    pub(super) fn preview_output_bytes(&self) -> Arc<Vec<u8>> {
        assert_eq!(self.completed, 1);
        assert_eq!(self.failed, 0);
        assert!(self.relay_source(0).is_some());
        Arc::new(fs::read(&self.items[0].target).unwrap())
    }
    #[cfg(feature = "ui-preview")]
    pub(super) fn preview_fixture(&mut self) {
        self.inputs = vec![
            PathBuf::from("C:\\Users\\demo\\Pictures\\hero.png"),
            PathBuf::from("C:\\Users\\demo\\Pictures\\diagram.webp"),
            PathBuf::from("C:\\Users\\demo\\Pictures\\cover.jpg"),
        ];
        self.output_dir = "C:\\Users\\demo\\Pictures\\export".into();
        self.max_width = 1600;
        self.format = Format::WebP;
        self.items = self
            .inputs
            .iter()
            .enumerate()
            .map(|(index, source)| Item {
                source: source.clone(),
                target: PathBuf::from(format!(
                    "C:\\Users\\demo\\Pictures\\export\\{}-batch.webp",
                    source.file_stem().unwrap().to_string_lossy()
                )),
                dimensions: [(2400, 1350), (1920, 1080), (1200, 1600)][index],
                original_bytes: [2_750_000, 1_180_000, 900_000][index],
                outcome: String::new(),
                conflict: false,
                saved_bytes: None,
                saved_sha256: None,
                selected: None,
                source_sha256: None,
                failed: false,
            })
            .collect();
        self.phase = "preflight";
        self.report_current = false;
        self.ready = true;
        self.message = "界面预览：3 张图片已检查，无命名冲突。".into();
    }
    fn invalidate(&mut self) {
        self.report_current = false;
        self.phase = "empty";
        self.items.clear();
        self.ready = false;
        self.completed = 0;
        self.failed = 0;
        self.message = "规则已变化，请重新检查。".into();
    }

    pub(super) fn busy(&self) -> bool {
        self.receiver.is_some() || self.running
    }

    pub(super) fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.receiver.as_ref() else {
            return;
        };
        let mut disconnected = false;
        loop {
            let event = match rx.try_recv() {
                Ok(event) => {
                    self.report_current = false;
                    event
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.report_current = false;
                    self.phase = "interrupted";
                    if self.running {
                        self.running = false;
                        self.ready = false;
                        self.message = format!(
                            "批处理线程意外结束；已处理 {} / {} 项。已完成文件保留，请检查后重新开始。",
                            self.completed,
                            self.items.len()
                        );
                    } else if self.items.is_empty() && self.message == "正在检查文件与输出冲突…"
                    {
                        self.message = "图片检查线程意外结束，请重新检查。".into();
                    }
                    disconnected = true;
                    break;
                }
            };
            match event {
                Event::Inspected(Ok(items)) => {
                    self.phase = "preflight";
                    let conflicts = items.iter().filter(|item| item.conflict).count();
                    self.ready = conflicts == 0;
                    self.message = if conflicts == 0 {
                        format!("检查完成：{} 张图片，确认输出清单后开始。", items.len())
                    } else {
                        format!("检查完成：{conflicts} 项冲突；更改输入或输出目录后重新检查。")
                    };
                    self.items = items;
                    disconnected = true;
                    break;
                }
                Event::Inspected(Err(error)) => {
                    self.phase = "inspection_failed";
                    self.message = format!("检查失败：{error}");
                    disconnected = true;
                    break;
                }
                Event::Saved(index, result) => {
                    self.completed += 1;
                    if result.is_err() {
                        self.failed += 1;
                    }
                    if let Some(item) = self.items.get_mut(index) {
                        item.saved_bytes = result.as_ref().ok().map(|saved| saved.bytes);
                        item.saved_sha256 = result.as_ref().ok().map(|saved| saved.sha256);
                        item.failed = result.is_err();
                        item.outcome = match result {
                            Ok(saved) => {
                                format!("完成 · {:.2} MB", saved.bytes as f64 / 1_000_000.0)
                            }
                            Err(error) => format!("失败：{error}"),
                        };
                    }
                }
                Event::Finished { cancelled } => {
                    self.phase = if cancelled { "cancelled" } else { "completed" };
                    self.running = false;
                    self.message = if cancelled {
                        format!(
                            "已取消；已处理 {} / {} 项，成功 {}、失败 {}。已完成文件保留。",
                            self.completed,
                            self.items.len(),
                            self.completed - self.failed,
                            self.failed
                        )
                    } else {
                        format!(
                            "批处理结束：已处理 {} / {} 项，成功 {}、失败 {}。",
                            self.completed,
                            self.items.len(),
                            self.completed - self.failed,
                            self.failed
                        )
                    };
                    disconnected = true;
                    break;
                }
            }
        }
        if disconnected {
            self.receiver = None;
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }
    }

    fn inspect(&mut self, memory: memory::Pool) {
        if self.receiver.is_some() || self.running {
            return;
        }
        self.report_current = false;
        self.phase = "inspecting";
        self.completed = 0;
        self.failed = 0;
        self.items.clear();
        self.ready = false;
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        let sources = self.inputs.clone();
        let output_dir = PathBuf::from(self.output_dir.trim());
        let format = self.format;
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        self.message = "正在检查文件与输出冲突…".into();
        std::thread::spawn(move || {
            let result = inspect_batch_budgeted(&sources, &output_dir, format, &cancel, &memory)
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Inspected(result));
        });
    }

    fn start(&mut self, memory: memory::Pool) {
        if self.receiver.is_some()
            || self.running
            || !self.ready
            || self.items.is_empty()
            || self.items.iter().any(|item| item.conflict)
        {
            return;
        }
        let items = self.items.clone();
        let max_width = self.max_width;
        let format = self.format;
        let quality = self.jpeg_quality;
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        self.report_current = false;
        self.phase = "processing";
        for item in &mut self.items {
            item.saved_bytes = None;
            item.saved_sha256 = None;
            item.failed = false;
            item.outcome.clear();
        }
        self.running = true;
        self.ready = false;
        self.completed = 0;
        self.failed = 0;
        self.message = "批量处理中…".into();
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        std::thread::spawn(move || {
            run_batch_budgeted(&items, max_width, format, quality, &cancel, &tx, &memory)
        });
    }

    pub(super) fn report(&self) -> Option<&str> {
        if self.report_current && !self.busy() {
            self.report.as_deref()
        } else {
            None
        }
    }

    fn build_report(&self) -> Result<String> {
        ensure!(!self.busy() && !self.items.is_empty(), "请先完成检查或处理");
        ensure!(self.items.len() <= MAX_BATCH, "报告条目超限");
        let origin = super::relay::origin("image-batch")?;
        let items = self.items.iter().map(|item| serde_json::json!({
            "sourceName":item.source.file_name().map(|v|v.to_string_lossy()),
            "outputName":item.target.file_name().map(|v|v.to_string_lossy()),
            "sourceWidth":item.dimensions.0,"sourceHeight":item.dimensions.1,
            "sourceBytes":item.original_bytes,"outputBytes":item.saved_bytes,
            "status": if item.conflict {"conflict"} else if item.saved_bytes.is_some() {"saved"} else if item.failed {"failed"} else {"not_processed"}
        })).collect::<Vec<_>>();
        let text = serde_json::to_string_pretty(&serde_json::json!({
            "schemaVersion":1,"material":"image-batch-report","phase":self.phase,
            "origin":{"toolId":origin.id,"version":origin.version,"utc":origin.utc},
            "settings":{"format":self.format.extension(),"maxWidth":self.max_width,"jpegQuality":if self.format==Format::Jpeg {Some(self.jpeg_quality)} else {None}},
            "counts":{"total":items.len(),"saved":self.items.iter().filter(|i|i.saved_bytes.is_some()).count(),"failed":self.items.iter().filter(|i|i.failed).count(),"conflict":self.items.iter().filter(|i|i.conflict).count(),"notProcessed":self.items.iter().filter(|i|!i.conflict && !i.failed && i.saved_bytes.is_none()).count()},
            "items":items,
            "notes":"冻结清单：预检不代表已输出；outputBytes 仅来自实际成功保存结果。仅保留文件名，不含完整路径或原始错误；名称不是可执行文件引用。不重新读取或保存文件，不自动运行接收工具。"
        }))?;
        ensure!(
            text.len() <= 256 * 1024,
            "报告超过 256 KiB；原工作和旧报告保留"
        );
        Ok(text)
    }

    pub(super) fn report_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let response = ui.add_enabled(
                !self.busy() && !self.items.is_empty(),
                egui::Button::new("生成批量图片报告"),
            );
            #[cfg(feature = "ui-preview")]
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::new("image-report-generate"), response.rect));
            if response.clicked() {
                match self.build_report() {
                    Ok(text) => {
                        self.report = Some(text);
                        self.report_current = true;
                    }
                    Err(error) => {
                        self.message = error.to_string();
                    }
                }
            }
            ui.small("预检 / 实际处理结果 → JSON、备忘；不重新读写图片。");
        });
        if let Some(text) = &self.report {
            if !self.report_current {
                ui.small("清单、规则或任务已改变，旧报告不可发送；请重新生成。");
            }
            egui::CollapsingHeader::new(if self.report_current {
                "批量图片 JSON 报告"
            } else {
                "旧批量图片 JSON 报告"
            })
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(170.0)
                    .show(ui, |ui| {
                        ui.monospace(text);
                    });
            });
        }
    }

    fn relay_source(&self, index: usize) -> Option<super::relay::Source> {
        if self.busy() {
            return None;
        }
        let item = self.items.get(index)?;
        if item.conflict || item.failed {
            return None;
        }
        Some(super::relay::Source::Published {
            path: item.target.clone(),
            bytes: item.saved_bytes?,
            sha256: item.saved_sha256?,
        })
    }

    pub(super) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        allow_relay: bool,
        memory: &memory::Pool,
    ) -> Option<super::relay::Source> {
        let mut selected = None;
        self.poll(ui.ctx());
        ui.heading("图片批处理");
        ui.label("先检查目标名称和冲突，再逐张转换。原图和已有输出不会被覆盖。");
        let busy = self.receiver.is_some() || self.running;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("选择多张图片…"))
                .clicked()
                && let Some(paths) = rfd::FileDialog::new()
                    .add_filter("图片", &["png", "jpg", "jpeg", "webp"])
                    .pick_files()
            {
                self.inputs = paths;
                if self.output_dir.is_empty()
                    && let Some(parent) = self.inputs.first().and_then(|p| p.parent())
                {
                    self.output_dir = parent.to_string_lossy().into_owned();
                }
                self.invalidate();
            }
            ui.label(format!(
                "已选择 {} 张（最多 {MAX_BATCH} 张）",
                self.inputs.len()
            ));
        });
        ui.horizontal(|ui| {
            ui.label("输出目录");
            if ui
                .add_enabled(
                    !busy,
                    egui::TextEdit::singleline(&mut self.output_dir).desired_width(440.0),
                )
                .changed()
            {
                self.invalidate();
            }
            if ui
                .add_enabled(!busy, egui::Button::new("选择目录…"))
                .clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                self.output_dir = path.to_string_lossy().into_owned();
                self.invalidate();
            }
        });
        ui.horizontal(|ui| {
            ui.label("最大宽度");
            if ui
                .add_enabled(
                    !busy,
                    egui::Slider::new(&mut self.max_width, 32..=12000).suffix(" px"),
                )
                .changed()
            {
                self.invalidate();
            }
            ui.label("较窄图片保持原宽");
        });
        ui.horizontal(|ui| {
            ui.label("输出格式");
            ui.add_enabled_ui(!busy, |ui| {
                egui::ComboBox::from_id_salt("batch-image-format")
                    .selected_text(self.format.label())
                    .show_ui(ui, |ui| {
                        for format in Format::ALL {
                            if ui
                                .selectable_value(&mut self.format, format, format.label())
                                .changed()
                            {
                                self.invalidate();
                            }
                        }
                    });
            });
            if self.format == Format::Jpeg
                && ui
                    .add_enabled(
                        !busy,
                        egui::Slider::new(&mut self.jpeg_quality, 35..=95).text("JPEG 质量"),
                    )
                    .changed()
            {
                self.invalidate();
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !busy && !self.inputs.is_empty(),
                    egui::Button::new("检查并预览输出清单"),
                )
                .clicked()
            {
                self.inspect(memory.clone());
            }
            if ui
                .add_enabled(
                    !busy && self.ready && !self.items.is_empty(),
                    egui::Button::new("确认并开始批处理"),
                )
                .clicked()
            {
                self.start(memory.clone());
            }
            if busy && ui.button("取消余下任务").clicked() {
                self.cancel.store(true, Ordering::Relaxed);
                self.message = "正在取消；已保存的文件会保留…".into();
            }
        });
        if self.running {
            ui.add(
                egui::ProgressBar::new(self.completed as f32 / self.items.len().max(1) as f32)
                    .text(format!("{} / {}", self.completed, self.items.len())),
            );
        }
        ui.label(&self.message);
        if !self.items.is_empty() {
            ui.separator();
            ui.label("输出清单");
            egui::ScrollArea::vertical()
                .max_height(340.0)
                .show(ui, |ui| {
                    for (index, item) in self.items.iter().enumerate() {
                        let name = item
                            .source
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy();
                        let target = item
                            .target
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy();
                        ui.horizontal_wrapped(|ui| {
                            ui.label(format!(
                                "{name} · {}×{} · {:.2} MB → {target}",
                                item.dimensions.0,
                                item.dimensions.1,
                                item.original_bytes as f64 / 1_000_000.0
                            ));
                            ui.label(if item.outcome.is_empty() {
                                "待处理"
                            } else {
                                &item.outcome
                            });
                            if item.saved_bytes.is_some() {
                                let response = ui
                                    .add_enabled(
                                        allow_relay && self.relay_source(index).is_some(),
                                        egui::Button::new("发送此图片…"),
                                    )
                                    .on_hover_text(
                                        "只读取这一项，核对生成时的内容摘要，再预览并选择目标工具",
                                    );
                                #[cfg(feature = "ui-preview")]
                                ui.ctx().data_mut(|d| {
                                    d.insert_temp(
                                        egui::Id::new(format!("image-batch-relay-{index}")),
                                        response.rect,
                                    )
                                });
                                if response.clicked() {
                                    selected = self.relay_source(index);
                                }
                            }
                        });
                    }
                });
        }
        ui.small("一次最多 100 张；每张输入 ≤32 MiB、≤1600 万像素。完成项保留，失败项会单独说明。预检后若外部文件变化，执行时仍拒绝覆盖。");
        ui.small("发送此图片：明确选择一项后后台读取（≤128 MiB），核对生成时SHA-256；确认后只接收内存图片，不自动保存。文件身份从本次选择时开始核对。");
        selected
    }
}

fn target_for(source: &Path, dir: &Path, format: Format) -> Result<PathBuf> {
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .context("文件名不是有效文本")?;
    Ok(dir.join(format!("{stem}-batch.{}", format.extension())))
}

#[cfg(any(test, feature = "ui-preview"))]
fn inspect_batch(sources: &[PathBuf], dir: &Path, format: Format) -> Result<Vec<Item>> {
    inspect_batch_budgeted(
        sources,
        dir,
        format,
        &AtomicBool::new(false),
        &memory::Pool::default(),
    )
}

fn inspect_batch_budgeted(
    sources: &[PathBuf],
    dir: &Path,
    format: Format,
    cancel: &AtomicBool,
    memory: &memory::Pool,
) -> Result<Vec<Item>> {
    ensure!(
        !sources.is_empty() && sources.len() <= MAX_BATCH,
        "请选择 1–100 张图片"
    );
    ensure!(dir.is_dir(), "输出目录不存在");
    let mut seen = HashSet::new();
    let canonical_sources: HashSet<_> = sources
        .iter()
        .map(fs::canonicalize)
        .collect::<Result<_, _>>()?;
    ensure!(
        canonical_sources.len() == sources.len(),
        "输入列表包含重复文件"
    );
    let mut items = Vec::with_capacity(sources.len());
    for source in sources {
        ensure!(!cancel.load(Ordering::Relaxed), "检查已取消");
        let path = std::path::absolute(source)?;
        let material =
            crate::material_files::FileMaterial::selected(&path, super::MAX_INPUT_BYTES as usize)?;
        let selected = workflow::input::read_selected(&material, cancel, memory)
            .with_context(|| source.display().to_string())?;
        let mut reader = image::ImageReader::new(std::io::Cursor::new(selected.bytes()))
            .with_guessed_format()?;
        ensure!(
            matches!(
                reader.format(),
                Some(image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP)
            ),
            "仅支持PNG/JPEG/WebP"
        );
        reader.limits(super::image_limits());
        let (width, height) = reader.into_dimensions()?;
        ensure!(
            width > 0
                && height > 0
                && width <= 12000
                && height <= 12000
                && u64::from(width) * u64::from(height) <= super::MAX_PIXELS,
            "输入图片尺寸超限"
        );
        let digest = Sha256::digest(selected.bytes()).into();
        selected.verify()?;
        let size = material.bytes();
        let target = target_for(source, dir, format)?;
        let target_key = target.to_string_lossy().to_lowercase();
        let repeated = !seen.insert(target_key);
        let conflict = repeated || target.exists() || canonical_sources.contains(&target);
        items.push(Item {
            source: source.clone(),
            selected: Some(material),
            source_sha256: Some(digest),
            target,
            dimensions: (width, height),
            original_bytes: size,
            outcome: if conflict {
                "目标重名或已存在".into()
            } else {
                String::new()
            },
            conflict,
            saved_bytes: None,
            saved_sha256: None,
            failed: false,
        });
    }
    if items.iter().any(|item| item.conflict) {
        // The first of two generated duplicates must also be marked.
        let mut counts = std::collections::HashMap::new();
        for item in &items {
            *counts
                .entry(item.target.to_string_lossy().to_lowercase())
                .or_insert(0usize) += 1;
        }
        for item in &mut items {
            if counts[&item.target.to_string_lossy().to_lowercase()] > 1 {
                item.conflict = true;
                item.outcome = "批次内目标重名".into();
            }
        }
    }
    Ok(items)
}

#[cfg(test)]
fn process_one(
    item: &Item,
    max_width: u32,
    format: Format,
    quality: u8,
    cancel: &AtomicBool,
) -> Result<SavedOutput> {
    process_one_budgeted(
        item,
        max_width,
        format,
        quality,
        cancel,
        &memory::Pool::default(),
    )
}

fn process_one_budgeted(
    item: &Item,
    max_width: u32,
    format: Format,
    quality: u8,
    cancel: &AtomicBool,
    memory: &memory::Pool,
) -> Result<SavedOutput> {
    ensure!(!cancel.load(Ordering::Relaxed), "操作已取消");
    let material = item.selected.as_ref().context("请重新检查输入文件")?;
    let expected = item.source_sha256.context("请重新检查输入内容")?;
    let selected = workflow::input::read_selected(material, cancel, memory)?;
    let actual: [u8; 32] = Sha256::digest(selected.bytes()).into();
    ensure!(actual == expected, "输入内容在检查后发生变化，请重新检查");
    let image = workflow::input::decode(selected.bytes(), cancel, memory)?;
    selected.verify()?;
    ensure!(
        image.dimensions() == item.dimensions,
        "输入尺寸在检查后发生变化"
    );
    drop(selected);
    ensure!(!cancel.load(Ordering::Relaxed), "操作已取消");
    let width = image.width().min(max_width.max(1));
    let (bytes, _, _) = single::encode(image, width, format, quality, memory)?;
    ensure!(!cancel.load(Ordering::Relaxed), "操作已取消");
    let saved = SavedOutput {
        bytes: bytes.len(),
        sha256: Sha256::digest(bytes.as_slice()).into(),
    };
    super::save_image_new(&item.target, &bytes, cancel)?;
    Ok(saved)
}

#[cfg(any(test, feature = "ui-preview"))]
fn run_batch(
    items: &[Item],
    max_width: u32,
    format: Format,
    quality: u8,
    cancel: &AtomicBool,
    tx: &mpsc::Sender<Event>,
) {
    run_batch_budgeted(
        items,
        max_width,
        format,
        quality,
        cancel,
        tx,
        &memory::Pool::default(),
    );
}

fn run_batch_budgeted(
    items: &[Item],
    max_width: u32,
    format: Format,
    quality: u8,
    cancel: &AtomicBool,
    tx: &mpsc::Sender<Event>,
    memory: &memory::Pool,
) {
    for (index, item) in items.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let result = process_one_budgeted(item, max_width, format, quality, cancel, memory)
            .map_err(|e| format!("{e:#}"));
        if tx.send(Event::Saved(index, result)).is_err() {
            return;
        }
    }
    let _ = tx.send(Event::Finished {
        cancelled: cancel.load(Ordering::Relaxed),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, GenericImageView, ImageFormat};

    struct BudgetFixture(PathBuf);
    impl BudgetFixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("zi-batch-budget-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn image(&self, name: &str, side: u32) -> PathBuf {
            let path = self.0.join(name);
            DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                side,
                side,
                image::Rgba([20, 70, 120, 180]),
            ))
            .save_with_format(&path, ImageFormat::Png)
            .unwrap();
            path
        }
    }
    impl Drop for BudgetFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn shared_budget_partial_failure_preserves_published_file_and_retained_source() {
        let fixture = BudgetFixture::new();
        let pool = memory::Pool::new(1024);
        let retained = Arc::new(DynamicImage::new_rgba8(10, 5));
        pool.share(&retained, memory::pixels(&retained)).unwrap();
        let inputs = [
            fixture.image("small.png", 2),
            fixture.image("large.png", 20),
        ];
        let cancel = AtomicBool::new(false);
        let items =
            inspect_batch_budgeted(&inputs, &fixture.0, Format::WebP, &cancel, &pool).unwrap();
        assert_eq!(pool.snapshot().unwrap(), (200, 0, 1024));
        let (tx, rx) = mpsc::channel();
        run_batch_budgeted(&items, 20, Format::WebP, 80, &cancel, &tx, &pool);
        let Event::Saved(0, Ok(receipt)) = rx.recv().unwrap() else {
            panic!("small output must publish")
        };
        let saved = fs::read(&items[0].target).unwrap();
        assert_eq!(receipt.sha256, <[u8; 32]>::from(Sha256::digest(&saved)));
        assert_eq!(receipt.bytes, saved.len());
        assert_eq!(
            image::load_from_memory(&saved).unwrap().dimensions(),
            (2, 2)
        );
        let Event::Saved(1, Err(error)) = rx.recv().unwrap() else {
            panic!("large image must exceed shared quota")
        };
        assert!(error.contains("预算"), "{error}");
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Finished { cancelled: false }
        ));
        assert!(!items[1].target.exists());
        assert_eq!(pool.snapshot().unwrap(), (200, 0, 1024));
        cancel.store(true, Ordering::Relaxed);
        assert!(inspect_batch_budgeted(&inputs, &fixture.0, Format::WebP, &cancel, &pool).is_err());
        run_batch_budgeted(&items, 20, Format::WebP, 80, &cancel, &tx, &pool);
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Finished { cancelled: true }
        ));
        assert_eq!(fs::read(&items[0].target).unwrap(), saved);
        assert!(retained.as_bytes().iter().all(|b| *b == 0));
        drop(retained);
        assert_eq!(pool.snapshot().unwrap(), (0, 0, 1024));
    }

    #[test]
    fn preflight_content_change_with_same_size_and_timestamp_rejected_before_codec() {
        let fixture = BudgetFixture::new();
        let path = fixture.image("selected.png", 10);
        let pool = memory::Pool::new(10000);
        let cancel = AtomicBool::new(false);
        let items = inspect_batch_budgeted(
            std::slice::from_ref(&path),
            &fixture.0,
            Format::WebP,
            &cancel,
            &pool,
        )
        .unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let mut changed = fs::read(&path).unwrap();
        changed[0] ^= 1;
        fs::write(&path, &changed).unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
        let error = process_one_budgeted(&items[0], 10, Format::WebP, 80, &cancel, &pool)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("输入内容在检查后发生变化"), "{error}");
        assert!(!items[0].target.exists());
        assert_eq!(fs::read(&path).unwrap(), changed);
        assert_eq!(pool.snapshot().unwrap(), (0, 0, 10000));
    }

    #[test]
    fn published_outputs_relay_exact_pixels_and_preserve_batch_and_target_drafts() {
        let root = std::env::temp_dir().join(format!("zi-batch-relay-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let source = root.join("input.png");
        DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            40,
            20,
            image::Rgba([30, 60, 90, 120]),
        ))
        .save_with_format(&source, ImageFormat::Png)
        .unwrap();
        let original = fs::read(&source).unwrap();
        for format in Format::ALL {
            let items = inspect_batch(&[source.clone()], &root, format).unwrap();
            let (tx, rx) = mpsc::channel();
            run_batch(&items, 32, format, 80, &AtomicBool::new(false), &tx);
            drop(tx);
            let mut batch = State {
                items,
                receiver: Some(rx),
                running: true,
                ..State::default()
            };
            assert!(batch.relay_source(0).is_none());
            batch.poll(&egui::Context::default());
            let output = fs::read(&batch.items[0].target).unwrap();
            let expected = image::load_from_memory(&output).unwrap();
            let prepared = super::super::relay::prepare(
                batch.relay_source(0).unwrap(),
                Vec::new(),
                "image-batch",
            )
            .unwrap();
            assert_eq!(prepared.image.to_rgba8(), expected.to_rgba8());
            assert_eq!(prepared.origins[0].id, "image-batch");
            assert_eq!(prepared.kind, "所选批量输出 · 内容已核对");
            assert_eq!(batch.completed, 1);
            assert!(!batch.busy());
            assert_eq!(fs::read(&batch.items[0].target).unwrap(), output);
            batch.items[0].failed = true;
            assert!(batch.relay_source(0).is_none());
            batch.items[0].failed = false;
            batch.items[0].saved_sha256 = None;
            assert!(batch.relay_source(0).is_none());
        }
        assert_eq!(fs::read(&source).unwrap(), original);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn published_hash_rejects_same_length_same_mtime_change_and_missing_file() {
        let root = std::env::temp_dir().join(format!("zi-batch-changed-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let source = root.join("input.png");
        DynamicImage::new_rgb8(32, 32)
            .save_with_format(&source, ImageFormat::Png)
            .unwrap();
        let mut items = inspect_batch(&[source], &root, Format::WebP).unwrap();
        let receipt =
            process_one(&items[0], 32, Format::WebP, 80, &AtomicBool::new(false)).unwrap();
        items[0].saved_bytes = Some(receipt.bytes);
        items[0].saved_sha256 = Some(receipt.sha256);
        let state = State {
            items,
            ..State::default()
        };
        let path = &state.items[0].target;
        let mtime = fs::metadata(path).unwrap().modified().unwrap();
        let mut changed = fs::read(path).unwrap();
        changed[0] ^= 1;
        fs::write(path, &changed).unwrap();
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(mtime))
            .unwrap();
        assert!(
            super::super::relay::prepare(state.relay_source(0).unwrap(), Vec::new(), "image-batch")
                .is_err()
        );
        assert_eq!(fs::read(path).unwrap(), changed);
        assert_eq!(state.items[0].saved_bytes, Some(receipt.bytes));
        fs::remove_file(path).unwrap();
        assert!(
            super::super::relay::prepare(state.relay_source(0).unwrap(), Vec::new(), "image-batch")
                .is_err()
        );
        assert_eq!(state.items[0].saved_bytes, Some(receipt.bytes));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reports_distinguish_preflight_real_partial_results_and_stale_rules() {
        let root = std::env::temp_dir().join(format!("zi-batch-report-{}", uuid::Uuid::new_v4()));
        let output = root.join("output");
        fs::create_dir_all(&output).unwrap();
        let good = root.join("图.png");
        let missing = root.join("missing.png");
        for file in [&good, &missing] {
            DynamicImage::new_rgb8(40, 20)
                .save_with_format(file, ImageFormat::Png)
                .unwrap();
        }
        let items = inspect_batch(&[good.clone(), missing.clone()], &output, Format::WebP).unwrap();
        let mut state = State {
            items,
            phase: "preflight",
            format: Format::WebP,
            ..State::default()
        };
        let preflight = state.build_report().unwrap();
        let value: serde_json::Value = serde_json::from_str(&preflight).unwrap();
        assert_eq!(value["phase"], "preflight");
        assert_eq!(value["counts"]["saved"], 0);
        assert!(value["items"][0]["outputBytes"].is_null());
        assert!(!preflight.contains(&root.to_string_lossy().to_string()));
        state.report = Some(preflight.clone());
        state.report_current = true;
        assert!(state.report().is_some());
        fs::remove_file(&missing).unwrap();
        let (tx, rx) = mpsc::channel();
        run_batch(
            &state.items,
            32,
            Format::WebP,
            80,
            &AtomicBool::new(false),
            &tx,
        );
        drop(tx);
        state.receiver = Some(rx);
        state.running = true;
        state.report_current = false;
        state.poll(&egui::Context::default());
        let report = state.build_report().unwrap();
        let value: serde_json::Value = serde_json::from_str(&report).unwrap();
        assert_eq!(value["phase"], "completed");
        assert_eq!(value["counts"]["saved"], 1);
        assert_eq!(value["counts"]["failed"], 1);
        assert_eq!(
            value["items"][0]["outputBytes"],
            fs::metadata(&state.items[0].target).unwrap().len()
        );
        assert_eq!(value["items"][1]["status"], "failed");
        assert!(value["items"][1]["outputBytes"].is_null());
        assert!(!report.contains(&root.to_string_lossy().to_string()));
        state.report = Some(report.clone());
        state.report_current = true;
        state.invalidate();
        assert!(state.report().is_none());
        assert_eq!(state.report.as_deref(), Some(report.as_str()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hundred_item_report_keeps_every_name_without_truncation() {
        let items = (0..100)
            .map(|i| Item {
                source: PathBuf::from(format!("{}-{i}.png", "图".repeat(60))),
                target: PathBuf::from(format!("result-{i}.webp")),
                dimensions: (400, 300),
                original_bytes: 1000,
                outcome: String::new(),
                conflict: false,
                saved_bytes: None,
                saved_sha256: None,
                selected: None,
                source_sha256: None,
                failed: false,
            })
            .collect();
        let state = State {
            items,
            phase: "preflight",
            ..State::default()
        };
        let report = state.build_report().unwrap();
        let value: serde_json::Value = serde_json::from_str(&report).unwrap();
        assert_eq!(value["items"].as_array().unwrap().len(), 100);
        assert_eq!(
            value["items"][99]["sourceName"],
            format!("{}-99.png", "图".repeat(60))
        );
        assert!(report.len() <= 256 * 1024);
    }

    #[test]
    fn cancelled_report_does_not_claim_unprocessed_files_were_saved() {
        let mut state = State {
            phase: "cancelled",
            items: vec![Item {
                source: PathBuf::from("a.png"),
                target: PathBuf::from("a.webp"),
                dimensions: (2, 2),
                original_bytes: 10,
                outcome: String::new(),
                conflict: false,
                saved_bytes: None,
                saved_sha256: None,
                selected: None,
                source_sha256: None,
                failed: false,
            }],
            ..State::default()
        };
        let value: serde_json::Value =
            serde_json::from_str(&state.build_report().unwrap()).unwrap();
        assert_eq!(value["counts"]["notProcessed"], 1);
        assert_eq!(value["counts"]["saved"], 0);
        state.items[0].source = PathBuf::from("a".repeat(300_000));
        assert!(state.build_report().is_err());
    }

    #[test]
    fn batch_finishes_from_another_tool_and_exit_guard_tracks_work() {
        let (tx, rx) = mpsc::channel();
        let mut images = super::super::State::default();
        images.batch.receiver = Some(rx);
        images.batch.running = true;
        assert_eq!(images.active_tool_id(), "image-tools");
        assert!(images.background_active());
        tx.send(Event::Finished { cancelled: false }).unwrap();
        images.poll_screenshot(&egui::Context::default());
        assert!(!images.background_active());
        assert!(images.batch.message.starts_with("批处理结束"));
    }

    #[test]
    fn disconnected_worker_releases_busy_and_preserves_completed_count() {
        let (tx, rx) = mpsc::channel();
        let mut state = State {
            receiver: Some(rx),
            running: true,
            completed: 2,
            ..State::default()
        };
        drop(tx);
        state.poll(&egui::Context::default());
        assert!(!state.busy());
        assert_eq!(state.completed, 2);
        assert!(state.message.contains("意外结束"));
        assert!(!state.ready);
    }

    #[test]
    fn terminal_completion_is_not_replaced_by_channel_disconnect() {
        let (tx, rx) = mpsc::channel();
        let mut state = State {
            receiver: Some(rx),
            running: true,
            ..State::default()
        };
        tx.send(Event::Finished { cancelled: true }).unwrap();
        drop(tx);
        state.poll(&egui::Context::default());
        assert!(!state.busy());
        assert!(state.message.starts_with("已取消"));
    }

    #[test]
    fn batch_checks_collisions_and_preserves_every_input() {
        let root = std::env::temp_dir().join(format!("zi-image-batch-{}", uuid::Uuid::new_v4()));
        let left = root.join("left");
        let right = root.join("right");
        let output = root.join("output");
        for dir in [&left, &right, &output] {
            fs::create_dir_all(dir).unwrap();
        }
        let a = left.join("same.png");
        let b = right.join("same.png");
        for path in [&a, &b] {
            DynamicImage::new_rgb8(80, 40)
                .save_with_format(path, ImageFormat::Png)
                .unwrap();
        }
        let originals = [fs::read(&a).unwrap(), fs::read(&b).unwrap()];
        let duplicates = inspect_batch(&[a.clone(), b.clone()], &output, Format::Jpeg).unwrap();
        assert!(duplicates.iter().all(|item| item.conflict));
        let one = inspect_batch(&[a.clone()], &output, Format::Jpeg).unwrap();
        assert!(!one[0].conflict);
        assert!(
            process_one(&one[0], 40, Format::Jpeg, 75, &AtomicBool::new(false))
                .unwrap()
                .bytes
                > 0
        );
        assert_eq!(image::open(&one[0].target).unwrap().dimensions(), (40, 20));
        assert!(process_one(&one[0], 40, Format::Jpeg, 75, &AtomicBool::new(false)).is_err());
        assert_eq!(fs::read(&a).unwrap(), originals[0]);
        assert_eq!(fs::read(&b).unwrap(), originals[1]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_existing_outputs_and_more_than_one_hundred_inputs() {
        let root = std::env::temp_dir().join(format!("zi-image-batch-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let source = root.join("source.png");
        DynamicImage::new_rgb8(32, 32)
            .save_with_format(&source, ImageFormat::Png)
            .unwrap();
        let target = root.join("source-batch.jpg");
        fs::write(&target, b"keep").unwrap();
        assert!(inspect_batch(&[source.clone()], &root, Format::Jpeg).unwrap()[0].conflict);
        assert!(inspect_batch(&vec![source; 101], &root, Format::Jpeg).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"keep");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancellation_before_first_file_writes_nothing() {
        let root = std::env::temp_dir().join(format!("zi-image-batch-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let source = root.join("source.png");
        DynamicImage::new_rgb8(32, 32)
            .save_with_format(&source, ImageFormat::Png)
            .unwrap();
        let items = inspect_batch(&[source.clone()], &root, Format::WebP).unwrap();
        let (tx, rx) = mpsc::channel();
        let cancelled = AtomicBool::new(true);
        run_batch(&items, 32, Format::WebP, 80, &cancelled, &tx);
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Finished { cancelled: true }
        ));
        assert!(!items[0].target.exists());
        assert!(rx.try_recv().is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
