//! Explicit, conflict-checked image batch processing. Each output is created once.
use super::{Format, decode_image, encode_image, inspect_image};
use anyhow::{Context, Result, ensure};
use eframe::egui;
use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

const MAX_BATCH: usize = 100;

#[derive(Clone)]
struct Item {
    source: PathBuf,
    target: PathBuf,
    dimensions: (u32, u32),
    original_bytes: u64,
    outcome: String,
    conflict: bool,
}

enum Event {
    Inspected(Result<Vec<Item>, String>),
    Saved(usize, Result<usize, String>),
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
        }
    }
}

impl State {
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
            })
            .collect();
        self.ready = true;
        self.message = "界面预览：3 张图片已检查，无命名冲突。".into();
    }
    fn invalidate(&mut self) {
        self.items.clear();
        self.ready = false;
        self.completed = 0;
        self.failed = 0;
        self.message = "规则已变化，请重新检查。".into();
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.receiver.as_ref() else {
            return;
        };
        let mut disconnected = false;
        loop {
            let event = match rx.try_recv() {
                Ok(event) => event,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            };
            match event {
                Event::Inspected(Ok(items)) => {
                    let conflicts = items.iter().filter(|item| item.conflict).count();
                    self.ready = conflicts == 0;
                    self.message = if conflicts == 0 {
                        format!("检查完成：{} 张图片，确认输出清单后开始。", items.len())
                    } else {
                        format!("检查完成：{conflicts} 项冲突；更改输入或输出目录后重新检查。")
                    };
                    self.items = items;
                }
                Event::Inspected(Err(error)) => self.message = format!("检查失败：{error}"),
                Event::Saved(index, result) => {
                    self.completed += 1;
                    if result.is_err() {
                        self.failed += 1;
                    }
                    if let Some(item) = self.items.get_mut(index) {
                        item.outcome = match result {
                            Ok(bytes) => format!("完成 · {:.2} MB", bytes as f64 / 1_000_000.0),
                            Err(error) => format!("失败：{error}"),
                        };
                    }
                }
                Event::Finished { cancelled } => {
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
                }
            }
        }
        if disconnected {
            self.receiver = None;
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }
    }

    fn inspect(&mut self) {
        if self.receiver.is_some() || self.running {
            return;
        }
        self.items.clear();
        self.ready = false;
        let sources = self.inputs.clone();
        let output_dir = PathBuf::from(self.output_dir.trim());
        let format = self.format;
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        self.message = "正在检查文件与输出冲突…".into();
        std::thread::spawn(move || {
            let result = inspect_batch(&sources, &output_dir, format).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Inspected(result));
        });
    }

    fn start(&mut self) {
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
        self.running = true;
        self.ready = false;
        self.completed = 0;
        self.failed = 0;
        self.message = "批量处理中…".into();
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        std::thread::spawn(move || run_batch(&items, max_width, format, quality, &cancel, &tx));
    }

    pub(super) fn ui(&mut self, ui: &mut egui::Ui) {
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
                self.inspect();
            }
            if ui
                .add_enabled(
                    !busy && self.ready && !self.items.is_empty(),
                    egui::Button::new("确认并开始批处理"),
                )
                .clicked()
            {
                self.start();
            }
            if self.running && ui.button("取消余下任务").clicked() {
                self.cancel.store(true, Ordering::Relaxed);
                self.message = "正在完成当前文件，随后停止…".into();
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
                    for item in &self.items {
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
                        });
                    }
                });
        }
        ui.small("一次最多 100 张；每张输入 ≤32 MiB、≤1600 万像素。完成项保留，失败项会单独说明。预检后若外部文件变化，执行时仍拒绝覆盖。");
    }
}

fn target_for(source: &Path, dir: &Path, format: Format) -> Result<PathBuf> {
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .context("文件名不是有效文本")?;
    Ok(dir.join(format!("{stem}-batch.{}", format.extension())))
}

fn inspect_batch(sources: &[PathBuf], dir: &Path, format: Format) -> Result<Vec<Item>> {
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
        let (width, height, size) =
            inspect_image(source).with_context(|| source.display().to_string())?;
        let target = target_for(source, dir, format)?;
        let target_key = target.to_string_lossy().to_lowercase();
        let repeated = !seen.insert(target_key);
        let conflict = repeated || target.exists() || canonical_sources.contains(&target);
        items.push(Item {
            source: source.clone(),
            target,
            dimensions: (width, height),
            original_bytes: size,
            outcome: if conflict {
                "目标重名或已存在".into()
            } else {
                String::new()
            },
            conflict,
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

fn process_one(item: &Item, max_width: u32, format: Format, quality: u8) -> Result<usize> {
    let image = decode_image(&item.source).with_context(|| item.source.display().to_string())?;
    let width = image.width().min(max_width.max(1));
    let (bytes, _, _) = encode_image(&image, width, format, quality)?;
    let mut created = false;
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&item.target)
            .with_context(|| format!("目标已存在或不可写：{}", item.target.display()))?;
        created = true;
        file.write_all(&bytes)?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(error) = result {
        if created {
            let _ = fs::remove_file(&item.target);
        }
        return Err(error);
    }
    Ok(bytes.len())
}

fn run_batch(
    items: &[Item],
    max_width: u32,
    format: Format,
    quality: u8,
    cancel: &AtomicBool,
    tx: &mpsc::Sender<Event>,
) {
    for (index, item) in items.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let result = process_one(item, max_width, format, quality).map_err(|e| format!("{e:#}"));
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
        assert!(process_one(&one[0], 40, Format::Jpeg, 75).unwrap() > 0);
        assert_eq!(image::open(&one[0].target).unwrap().dimensions(), (40, 20));
        assert!(process_one(&one[0], 40, Format::Jpeg, 75).is_err());
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
