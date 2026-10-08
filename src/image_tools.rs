//! Bounded, local image inspection and explicit preview-before-save transforms.
mod batch;
mod editor;
mod metadata;
mod relay;
mod report;
mod screenshot;
mod workflow;
pub(crate) use workflow::Definition as WorkflowDefinition;
#[cfg(all(windows, feature = "ui-preview"))]
pub fn verify_screenshot_capture() -> anyhow::Result<(u32, u32)> {
    screenshot::verify_capture()
}
use anyhow::{Result, bail, ensure};
use eframe::egui;
use image::{
    DynamicImage, GenericImageView, ImageFormat, ImageReader, Limits, imageops::FilterType,
};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
};

const MAX_INPUT_BYTES: u64 = 32 * 1024 * 1024;
const MAX_PIXELS: u64 = 16_000_000;
const MAX_OUTPUT_BYTES: usize = 128 * 1024 * 1024;

fn save_image_new(
    path: &Path,
    bytes: &[u8],
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    crate::local_files::save_new_moved(&path, bytes, cancel)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Format {
    #[default]
    Png,
    Jpeg,
    WebP,
}
impl Format {
    pub(crate) const ALL: [Self; 3] = [Self::Png, Self::Jpeg, Self::WebP];
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG（无损）",
            Self::Jpeg => "JPEG（可调质量）",
            Self::WebP => "WebP（无损）",
        }
    }
    pub(crate) fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::WebP => "webp",
        }
    }
    pub(crate) fn image_format(self) -> ImageFormat {
        match self {
            Self::Png => ImageFormat::Png,
            Self::Jpeg => ImageFormat::Jpeg,
            Self::WebP => ImageFormat::WebP,
        }
    }
}

enum Job {
    Loaded {
        image: Arc<DynamicImage>,
        bytes: u64,
        preview: egui::ColorImage,
    },
    Preview {
        encoded: Vec<u8>,
        preview: egui::ColorImage,
        width: u32,
        height: u32,
    },
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    #[default]
    Single,
    Batch,
    Metadata,
    Editor,
    Screenshot,
    Workflow,
}

#[derive(Default)]
pub struct State {
    mode: Mode,
    origins: Vec<relay::Origin>,
    relay: Option<relay::Transfer>,
    relay_route: Option<&'static str>,
    reports: report::State,
    batch: batch::State,
    metadata: metadata::State,
    editor: editor::State,
    screenshot: screenshot::State,
    workflow: workflow::Workspace,
    input: String,
    output: String,
    source: Option<Arc<DynamicImage>>,
    source_bytes: u64,
    width: u32,
    format: Format,
    jpeg_quality: u8,
    encoded: Option<Arc<Vec<u8>>>,
    texture: Option<egui::TextureHandle>,
    message: String,
    error: bool,
    pending: Option<mpsc::Receiver<Result<Job, String>>>,
}
impl State {
    pub(crate) fn background_active(&self) -> bool {
        self.pending.is_some()
            || self.reports.busy()
            || self.batch.busy()
            || self.metadata.busy()
            || self.editor.busy()
            || self.screenshot.busy()
            || self.workflow.background_active()
            || self.relay.as_ref().is_some_and(|transfer| transfer.busy())
    }

    pub fn screenshot_capture_active(&self) -> bool {
        self.screenshot.capture_active()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_overlay_fixture(&mut self, ctx: &egui::Context, index: usize) {
        self.screenshot.preview_overlay_fixture(ctx, index);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_overlay_active(&self) -> bool {
        self.screenshot.preview_overlay_active()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_overlay_dimensions(&self) -> [i32; 2] {
        self.screenshot.preview_overlay_dimensions()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_overlay_key(&self, key: u32, scan: u32) {
        self.screenshot.preview_overlay_key(key, scan);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_overlay_pointer(&self, x: i32, y: i32, kind: u8) {
        self.screenshot.preview_overlay_pointer(x, y, kind);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_overlay_check(&self, phase: u8) {
        self.screenshot.preview_overlay_check(phase);
    }
    pub fn request_screenshot_capture(&mut self) {
        self.screenshot.request_capture();
    }
    pub fn active_tool_id(&self) -> &'static str {
        match self.mode {
            Mode::Single => "image-tools",
            Mode::Batch => "image-batch",
            Mode::Metadata => "image-metadata",
            Mode::Editor => "image-crop-annotate",
            Mode::Screenshot => "screenshot-workbench",
            Mode::Workflow => "image-workflow",
        }
    }
    pub fn take_relay_route(&mut self) -> Option<&'static str> {
        self.relay_route.take()
    }
    pub fn poll_screenshot(&mut self, ctx: &egui::Context) {
        self.poll_image_report(ctx);
        if let Some(transfer) = &mut self.relay {
            transfer.poll(ctx);
        }
        if self
            .relay
            .as_ref()
            .is_some_and(|transfer| transfer.cancelled && !transfer.busy())
        {
            self.relay = None;
        }

        self.poll(ctx);
        self.batch.poll(ctx);
        self.metadata.poll(ctx);
        self.editor.poll(ctx);
        self.screenshot.poll(ctx);
        self.workflow.poll(ctx);
    }
    pub fn take_screenshot_capture_request(&mut self) -> bool {
        self.screenshot.take_capture_request()
    }
    pub fn start_screenshot_capture(&mut self, ctx: &egui::Context, root: Option<isize>) {
        self.screenshot.start_desktop_capture(ctx, root);
    }
    pub fn screenshot_capture_failed(&mut self) {
        self.screenshot
            .capture_failed("无法隐藏工作台，未读取桌面；请重试。");
    }
    pub fn screenshot_overlay_ui(&mut self, ctx: &egui::Context) -> bool {
        self.screenshot.overlay_ui(ctx)
    }
    pub fn screenshot_busy(&self) -> bool {
        self.screenshot.busy()
    }
    pub fn screenshot_has_work(&self) -> bool {
        self.screenshot.has_work()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_screenshot_fixture(&mut self, ctx: &egui::Context) {
        self.mode = Mode::Screenshot;
        self.screenshot.preview_fixture(ctx);
    }
    pub fn show_screenshot(&mut self) {
        self.mode = Mode::Screenshot;
    }
    pub fn show_batch(&mut self) {
        self.mode = Mode::Batch;
    }
    pub(crate) fn can_receive_workflow(&self) -> bool {
        self.workflow.can_receive() && !self.relay_active()
    }
    pub(crate) fn receive_workflow(&mut self, definition: WorkflowDefinition) -> Result<()> {
        ensure!(
            self.can_receive_workflow(),
            "请先完成或取消图片流程当前任务和导入"
        );
        self.workflow.receive_definition(definition)?;
        self.show_workflow();
        Ok(())
    }
    pub(crate) fn take_workflow_loaded(&mut self) -> Option<crate::preferences::SavedWorkflow> {
        self.workflow.take_loaded()
    }
    pub fn show_workflow(&mut self) {
        self.mode = Mode::Workflow;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_image_workflow_fixture(&mut self) {
        self.mode = Mode::Workflow;
        self.workflow.preview_fixture();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_image_workflow_definition(&self) -> Vec<u8> {
        self.workflow.preview_definition()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_image_workflow_import_pending(&self) -> bool {
        self.workflow.pending_import()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_image_instances_check(&self, phase: u8) {
        self.workflow.preview_instance_check(phase);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_image_workflow_ready(&self) -> bool {
        self.workflow.preview_ready()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_image_workflow_check(&self, phase: u8) {
        self.workflow.preview_check(phase);
    }
    pub fn show_metadata(&mut self) {
        self.mode = Mode::Metadata;
    }
    pub fn show_editor(&mut self) {
        self.mode = Mode::Editor;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_batch_fixture(&mut self) {
        self.mode = Mode::Batch;
        self.batch.preview_fixture();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_metadata_fixture(&mut self, ctx: &egui::Context) {
        self.mode = Mode::Metadata;
        self.metadata.preview_fixture(ctx);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_editor_fixture(&mut self, ctx: &egui::Context) {
        self.mode = Mode::Editor;
        self.editor.preview_fixture(ctx);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self, ctx: &egui::Context) {
        let pixels = image::ImageBuffer::from_fn(960, 540, |x, y| {
            image::Rgba([(x / 4) as u8, (y / 3) as u8, 160, 255])
        });
        let source = DynamicImage::ImageRgba8(pixels);
        let Job::Preview {
            encoded, preview, ..
        } = encode_preview(&source, 720, Format::Jpeg, 78).expect("image preview fixture")
        else {
            unreachable!()
        };
        self.input = "C:\\Users\\demo\\Pictures\\sample.png".into();
        self.output = "C:\\Users\\demo\\Pictures\\sample-edited.jpg".into();
        self.source_bytes = 1_250_000;
        self.width = 720;
        self.format = Format::Jpeg;
        self.jpeg_quality = 78;
        self.source = Some(Arc::new(source));
        self.message = format!(
            "编码预览已生成：720 × 405 · {:.2} MB；可确认后另存新文件",
            encoded.len() as f64 / 1_000_000.0
        );
        self.encoded = Some(Arc::new(encoded));
        self.texture = Some(ctx.load_texture(
            "image-workbench-preview",
            preview,
            egui::TextureOptions::LINEAR,
        ));
    }
    fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.pending else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(80));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => Err("图片处理线程意外结束".into()),
        };
        self.pending = None;
        match result {
            Ok(Job::Loaded {
                image,
                bytes,
                preview,
            }) => {
                self.width = image.width();
                self.source_bytes = bytes;
                self.source = Some(image);
                self.origins.clear();
                self.encoded = None;
                self.texture = Some(ctx.load_texture(
                    "image-workbench-preview",
                    preview,
                    egui::TextureOptions::LINEAR,
                ));
                self.output = suggested_output(Path::new(&self.input), self.format)
                    .to_string_lossy()
                    .into_owned();
                self.message = "原图已读取。调整宽度或格式后生成最终编码预览。".into();
                self.error = false;
            }
            Ok(Job::Preview {
                encoded,
                preview,
                width,
                height,
            }) => {
                let size = encoded.len();
                self.encoded = Some(Arc::new(encoded));
                self.texture = Some(ctx.load_texture(
                    "image-workbench-preview",
                    preview,
                    egui::TextureOptions::LINEAR,
                ));
                self.message = format!(
                    "编码预览已生成：{width} × {height} · {:.2} MB；确认后另存为新文件",
                    size as f64 / 1_000_000.0
                );
                self.error = false;
            }
            Err(error) => {
                self.message = error;
                self.error = true;
            }
        }
    }
    fn start_load(&mut self) {
        if self.pending.is_some() {
            return;
        }
        let path = PathBuf::from(self.input.trim());
        self.source = None;
        self.encoded = None;
        self.texture = None;
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        self.message = "正在读取图片…".into();
        std::thread::spawn(move || {
            let result = load_image(&path).map_err(|error| format!("读取失败：{error:#}"));
            let _ = tx.send(result);
        });
    }
    fn start_preview(&mut self) {
        if self.pending.is_some() {
            return;
        }
        let Some(image) = self.source.clone() else {
            return;
        };
        let width = self.width;
        let format = self.format;
        let quality = self.jpeg_quality;
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        self.encoded = None;
        self.message = "正在生成最终编码预览…".into();
        std::thread::spawn(move || {
            let result = encode_preview(&image, width, format, quality)
                .map_err(|error| format!("预览失败：{error:#}"));
            let _ = tx.send(result);
        });
    }
    fn save(&mut self) {
        let Some(bytes) = &self.encoded else {
            return;
        };
        let source = Path::new(self.input.trim());
        let target = Path::new(self.output.trim());
        let result = (|| -> Result<()> {
            ensure!(
                !target.as_os_str().is_empty() && target != source,
                "请选择不同于原图的输出路径"
            );
            ensure!(
                target.extension().is_some_and(|ext| ext
                    .to_string_lossy()
                    .eq_ignore_ascii_case(self.format.extension())),
                "输出扩展名需与选择的格式一致"
            );
            save_image_new(target, bytes, &std::sync::atomic::AtomicBool::new(false))?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.message = format!("已另存：{}", target.display());
                self.error = false;
            }
            Err(error) => {
                self.message = format!("保存失败：{error:#}");
                self.error = true;
            }
        }
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.mode, Mode::Single, "单张图片");
            ui.selectable_value(&mut self.mode, Mode::Batch, "批量处理");
            ui.selectable_value(&mut self.mode, Mode::Metadata, "元数据检查");
            ui.selectable_value(&mut self.mode, Mode::Editor, "裁剪与标注");
            ui.selectable_value(&mut self.mode, Mode::Screenshot, "截图与透明套索");
            ui.selectable_value(&mut self.mode, Mode::Workflow, "图片流程");
        });
        self.relay_ui(ui);
        self.image_report_ui(ui);
        ui.add_space(10.0);
        match self.mode {
            Mode::Batch => {
                if let Some(source) = self.batch.ui(ui, self.relay.is_none()) {
                    self.relay = Some(relay::Transfer::start(
                        ui.ctx(),
                        source,
                        Vec::new(),
                        "image-batch",
                    ));
                }
                return;
            }
            Mode::Metadata => return self.metadata.ui(ui),
            Mode::Editor => return self.editor.ui(ui),
            Mode::Screenshot => return self.screenshot.ui(ui),
            Mode::Workflow => {
                let unlocked = !self.relay_active();
                return self.workflow.ui(ui, unlocked);
            }
            Mode::Single => {}
        }
        self.poll(ui.ctx());
        ui.heading("图片工作台");
        ui.label("在本机查看图片、缩小尺寸、转换格式并预览编码后的文件大小；原图不会被覆盖。");
        ui.add_space(12.0);
        let busy = self.pending.is_some();
        if !self.origins.is_empty() {
            ui.label("当前原图来自内存接力；输入路径后可切换文件。另存需手动选择输出路径。");
        }
        ui.label("原图路径");
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !busy,
                    egui::TextEdit::singleline(&mut self.input)
                        .desired_width((ui.available_width() - 190.0).max(180.0)),
                )
                .changed()
            {
                self.source = None;
                self.encoded = None;
                self.texture = None;
            }
            if ui
                .add_enabled(!busy, egui::Button::new("选择图片…"))
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("图片", &["png", "jpg", "jpeg", "webp"])
                    .pick_file()
            {
                self.input = path.to_string_lossy().into_owned();
                self.start_load();
            }
            if ui.add_enabled(!busy, egui::Button::new("读取")).clicked() {
                self.start_load();
            }
        });
        let Some(source) = &self.source else {
            ui.label(&self.message);
            return;
        };
        let (original_width, original_height) = source.dimensions();
        ui.small(format!(
            "原图：{original_width} × {original_height} · {:.2} MB",
            self.source_bytes as f64 / 1_000_000.0
        ));
        ui.horizontal(|ui| {
            ui.label("输出宽度");
            if ui
                .add_enabled(
                    !busy,
                    egui::Slider::new(&mut self.width, 32.min(original_width)..=original_width)
                        .suffix(" px"),
                )
                .changed()
            {
                self.encoded = None;
            }
            let height = (u64::from(original_height) * u64::from(self.width)
                / u64::from(original_width))
            .max(1);
            ui.label(format!("约 {height} px 高"));
        });
        ui.horizontal(|ui| {
            ui.label("输出格式");
            ui.add_enabled_ui(!busy, |ui| {
                egui::ComboBox::from_id_salt("image-format")
                    .selected_text(self.format.label())
                    .show_ui(ui, |ui| {
                        for format in Format::ALL {
                            if ui
                                .selectable_value(&mut self.format, format, format.label())
                                .changed()
                            {
                                self.encoded = None;
                                if !self.input.trim().is_empty() {
                                    self.output = suggested_output(Path::new(&self.input), format)
                                        .to_string_lossy()
                                        .into_owned();
                                } else {
                                    self.output.clear();
                                }
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
                self.encoded = None;
            }
        });
        if self.jpeg_quality == 0 {
            self.jpeg_quality = 80;
        }
        if ui
            .add_enabled(!busy, egui::Button::new("生成编码预览"))
            .clicked()
        {
            self.start_preview();
        }
        if let Some(texture) = &self.texture {
            let size = texture.size_vec2();
            ui.image((
                texture.id(),
                size * (700.0 / size.x).min(450.0 / size.y).min(1.0),
            ));
        }
        ui.label("另存路径");
        ui.add_enabled(
            self.encoded.is_some() && !busy,
            egui::TextEdit::singleline(&mut self.output)
                .desired_width(ui.available_width().min(680.0)),
        );
        if ui
            .add_enabled(
                self.encoded.is_some() && !busy,
                egui::Button::new("确认另存新文件"),
            )
            .clicked()
        {
            self.save();
        }
        ui.label(egui::RichText::new(&self.message).color(if self.error {
            egui::Color32::from_rgb(220, 70, 75)
        } else {
            ui.visuals().text_color()
        }));
        ui.small(
            "支持 PNG、JPEG、WebP。单文件 ≤32 MiB、解码图像 ≤1600 万像素；保存时拒绝覆盖现有文件。",
        );
    }
}

fn suggested_output(source: &Path, format: Format) -> PathBuf {
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    source.with_file_name(format!("{stem}-edited.{}", format.extension()))
}
fn preview_image(image: &DynamicImage) -> egui::ColorImage {
    let small = image.thumbnail(960, 600).to_rgba8();
    egui::ColorImage::from_rgba_unmultiplied(
        [small.width() as usize, small.height() as usize],
        small.as_raw(),
    )
}
fn load_image(path: &Path) -> Result<Job> {
    let (_, _, size) = inspect_image(path)?;
    let mut reader = ImageReader::open(path)?.with_guessed_format()?;
    reader.limits(image_limits());
    let image = reader.decode()?;
    let preview = preview_image(&image);
    Ok(Job::Loaded {
        image: Arc::new(image),
        bytes: size,
        preview,
    })
}
pub(crate) fn inspect_image(path: &Path) -> Result<(u32, u32, u64)> {
    let size = fs::metadata(path)?.len();
    ensure!(
        size > 0 && size <= MAX_INPUT_BYTES,
        "图片需为非空且不超过 32 MiB"
    );
    let bytes = fs::read(path)?;
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    ensure!(
        matches!(
            reader.format(),
            Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP)
        ),
        "仅支持 PNG、JPEG、WebP"
    );
    reader.limits(image_limits());
    let (width, height) = reader.into_dimensions()?;
    ensure!(
        u64::from(width) * u64::from(height) <= MAX_PIXELS,
        "图片像素数超过 1600 万"
    );
    Ok((width, height, size))
}
pub(crate) fn decode_image(path: &Path) -> Result<DynamicImage> {
    inspect_image(path)?;
    let mut reader = ImageReader::open(path)?.with_guessed_format()?;
    reader.limits(image_limits());
    Ok(reader.decode()?)
}
fn image_limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(12_000);
    limits.max_image_height = Some(12_000);
    limits.max_alloc = Some(192 * 1024 * 1024);
    limits
}
fn encode_preview(source: &DynamicImage, width: u32, format: Format, quality: u8) -> Result<Job> {
    let (encoded, width, height) = encode_image(source, width, format, quality)?;
    let decoded = image::load_from_memory_with_format(&encoded, format.image_format())?;
    Ok(Job::Preview {
        encoded,
        preview: preview_image(&decoded),
        width,
        height,
    })
}
pub(crate) fn encode_image(
    source: &DynamicImage,
    width: u32,
    format: Format,
    quality: u8,
) -> Result<(Vec<u8>, u32, u32)> {
    ensure!(
        width >= 1 && width <= source.width(),
        "输出宽度超出原图范围"
    );
    let height = ((u64::from(source.height()) * u64::from(width) + u64::from(source.width()) / 2)
        / u64::from(source.width()))
    .max(1) as u32;
    let resized = source.resize_exact(width, height, FilterType::Lanczos3);
    let mut encoded = Vec::new();
    match format {
        Format::Jpeg => {
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut encoded,
                quality.clamp(35, 95),
            );
            encoder.encode_image(&resized)?;
        }
        _ => resized.write_to(&mut Cursor::new(&mut encoded), format.image_format())?,
    }
    if encoded.len() > MAX_OUTPUT_BYTES {
        bail!("输出超过 128 MiB，请减小尺寸");
    }
    Ok((encoded, width, height))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_outputs_actual_encoded_image_and_preserves_source() {
        let source = DynamicImage::new_rgba8(64, 32);
        for format in Format::ALL {
            let Job::Preview {
                encoded,
                width,
                height,
                ..
            } = encode_preview(&source, 32, format, 75).unwrap()
            else {
                unreachable!()
            };
            assert_eq!((width, height), (32, 16));
            assert_eq!(
                image::load_from_memory_with_format(&encoded, format.image_format())
                    .unwrap()
                    .dimensions(),
                (32, 16)
            );
        }
        assert_eq!(source.dimensions(), (64, 32));
    }

    #[test]
    fn loads_local_image_and_never_overwrites_an_existing_output() {
        let folder = std::env::temp_dir().join(format!("zi-images-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&folder).unwrap();
        let original = folder.join("source.png");
        DynamicImage::new_rgba8(64, 32)
            .save_with_format(&original, ImageFormat::Png)
            .unwrap();
        let Job::Loaded { image, .. } = load_image(&original).unwrap() else {
            unreachable!()
        };
        assert_eq!(image.dimensions(), (64, 32));
        let Job::Preview { encoded, .. } = encode_preview(&image, 32, Format::Jpeg, 75).unwrap()
        else {
            unreachable!()
        };
        let target = folder.join("source-edited.jpg");
        let mut state = State {
            input: original.to_string_lossy().into_owned(),
            output: target.to_string_lossy().into_owned(),
            format: Format::Jpeg,
            encoded: Some(Arc::new(encoded)),
            ..Default::default()
        };
        state.save();
        assert!(!state.error && target.exists());
        let first = fs::read(&target).unwrap();
        state.save();
        assert!(state.error);
        assert_eq!(fs::read(&target).unwrap(), first);
        assert_eq!(image::open(&original).unwrap().dimensions(), (64, 32));
        fs::remove_file(&target).unwrap();
        fs::remove_file(&original).unwrap();
        fs::remove_dir(&folder).unwrap();
    }
}
