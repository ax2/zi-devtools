//! Bounded, local image inspection and explicit preview-before-save transforms.
mod batch;
mod metadata;
use anyhow::{Context, Result, bail, ensure};
use eframe::egui;
use image::{
    DynamicImage, GenericImageView, ImageFormat, ImageReader, Limits, imageops::FilterType,
};
use std::{
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
};

const MAX_INPUT_BYTES: u64 = 32 * 1024 * 1024;
const MAX_PIXELS: u64 = 16_000_000;
const MAX_OUTPUT_BYTES: usize = 128 * 1024 * 1024;

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

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Mode {
    #[default]
    Single,
    Batch,
    Metadata,
}

#[derive(Default)]
pub struct State {
    mode: Mode,
    batch: batch::State,
    metadata: metadata::State,
    input: String,
    output: String,
    source: Option<Arc<DynamicImage>>,
    source_bytes: u64,
    width: u32,
    format: Format,
    jpeg_quality: u8,
    encoded: Option<Vec<u8>>,
    texture: Option<egui::TextureHandle>,
    message: String,
    error: bool,
    pending: Option<mpsc::Receiver<Result<Job, String>>>,
}
impl State {
    pub fn show_batch(&mut self) {
        self.mode = Mode::Batch;
    }
    pub fn show_metadata(&mut self) {
        self.mode = Mode::Metadata;
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
        self.encoded = Some(encoded);
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
                self.encoded = Some(encoded);
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
        let mut created = false;
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
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)
                .with_context(|| {
                    format!("无法创建新文件；如已存在请更换名称：{}", target.display())
                })?;
            created = true;
            file.write_all(bytes)?;
            file.sync_all()?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.message = format!("已另存：{}", target.display());
                self.error = false;
            }
            Err(error) => {
                if created {
                    let _ = fs::remove_file(target);
                }
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
        });
        ui.add_space(10.0);
        match self.mode {
            Mode::Batch => return self.batch.ui(ui),
            Mode::Metadata => return self.metadata.ui(ui),
            Mode::Single => {}
        }
        self.poll(ui.ctx());
        ui.heading("图片工作台");
        ui.label("在本机查看图片、缩小尺寸、转换格式并预览编码后的文件大小；原图不会被覆盖。");
        ui.add_space(12.0);
        let busy = self.pending.is_some();
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
                                self.output = suggested_output(Path::new(&self.input), format)
                                    .to_string_lossy()
                                    .into_owned();
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
            encoded: Some(encoded),
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
