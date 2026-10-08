//! Bounded image-container metadata inspection and explicit clean-copy export.
use super::{MAX_INPUT_BYTES, MAX_OUTPUT_BYTES, MAX_PIXELS, image_limits, preview_image};
use anyhow::{Context, Result, bail, ensure};
use eframe::egui;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use std::{
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Summary {
    format: ImageFormat,
    exif: u32,
    gps: bool,
    icc: u32,
    xmp: u32,
    text: u32,
    iptc: u32,
    other: u32,
    animation: bool,
}

impl Default for Summary {
    fn default() -> Self {
        Self {
            format: ImageFormat::Png,
            exif: 0,
            gps: false,
            icc: 0,
            xmp: 0,
            text: 0,
            iptc: 0,
            other: 0,
            animation: false,
        }
    }
}

impl Summary {
    fn has_source_metadata(&self) -> bool {
        self.exif + self.icc + self.xmp + self.text + self.iptc + self.other > 0
    }
    fn label(&self) -> &'static str {
        match self.format {
            ImageFormat::Jpeg => "JPEG",
            ImageFormat::Png => "PNG",
            ImageFormat::WebP => "WebP",
            _ => "未知",
        }
    }
    fn extension(&self) -> &'static str {
        match self.format {
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Png => "png",
            ImageFormat::WebP => "webp",
            _ => "bin",
        }
    }
}

fn be16(data: &[u8]) -> Result<usize> {
    ensure!(data.len() >= 2, "元数据长度截断");
    Ok(u16::from_be_bytes([data[0], data[1]]) as usize)
}
fn be32(data: &[u8]) -> Result<usize> {
    ensure!(data.len() >= 4, "元数据长度截断");
    Ok(u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize)
}
fn le32(data: &[u8]) -> Result<usize> {
    ensure!(data.len() >= 4, "元数据长度截断");
    Ok(u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize)
}

fn has_gps_ifd(tiff: &[u8]) -> bool {
    gps_ifd(tiff).unwrap_or(false)
}
fn gps_ifd(tiff: &[u8]) -> Option<bool> {
    if tiff.len() < 8 {
        return Some(false);
    }
    let little = match &tiff[..4] {
        b"II*\0" => true,
        b"MM\0*" => false,
        _ => return Some(false),
    };
    let read16 = |bytes: &[u8]| -> Option<u16> {
        let array: [u8; 2] = bytes.get(..2)?.try_into().ok()?;
        Some(if little {
            u16::from_le_bytes(array)
        } else {
            u16::from_be_bytes(array)
        })
    };
    let read32 = |bytes: &[u8]| -> Option<u32> {
        let array: [u8; 4] = bytes.get(..4)?.try_into().ok()?;
        Some(if little {
            u32::from_le_bytes(array)
        } else {
            u32::from_be_bytes(array)
        })
    };
    let offset = read32(tiff.get(4..8)?)? as usize;
    let count = read16(tiff.get(offset..offset.checked_add(2)?)?)? as usize;
    if count > 4096 {
        return Some(false);
    }
    for index in 0..count {
        let start = offset.checked_add(2)?.checked_add(index.checked_mul(12)?)?;
        let entry = tiff.get(start..start.checked_add(12)?)?;
        if read16(entry)? == 0x8825 {
            let gps_offset = read32(&entry[8..12])? as usize;
            if gps_offset != 0 && tiff.get(gps_offset..gps_offset.checked_add(2)?).is_some() {
                return Some(true);
            }
        }
    }
    Some(false)
}

fn scan_metadata(bytes: &[u8]) -> Result<Summary> {
    if bytes.starts_with(&[0xff, 0xd8]) {
        scan_jpeg(bytes)
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        scan_png(bytes)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        scan_webp(bytes)
    } else {
        bail!("仅支持 PNG、JPEG、WebP")
    }
}

fn scan_jpeg(bytes: &[u8]) -> Result<Summary> {
    let mut summary = Summary {
        format: ImageFormat::Jpeg,
        ..Default::default()
    };
    let mut pos = 2usize;
    loop {
        ensure!(pos < bytes.len(), "JPEG 元数据结构截断");
        ensure!(bytes[pos] == 0xff, "JPEG marker 无效");
        while pos < bytes.len() && bytes[pos] == 0xff {
            pos += 1;
        }
        ensure!(pos < bytes.len(), "JPEG marker 截断");
        let marker = bytes[pos];
        pos += 1;
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        let length = be16(bytes.get(pos..).context("JPEG 长度截断")?)?;
        ensure!(length >= 2, "JPEG 段长度无效");
        let end = pos.checked_add(length).context("JPEG 段长度溢出")?;
        ensure!(end <= bytes.len(), "JPEG 段超出文件");
        let data = &bytes[pos + 2..end];
        match marker {
            0xe1 if data.starts_with(b"Exif\0\0") => {
                summary.exif += 1;
                summary.gps |= has_gps_ifd(&data[6..]);
            }
            0xe1 if data.starts_with(b"http://ns.adobe.com/xap/1.0/\0") => summary.xmp += 1,
            0xe2 if data.starts_with(b"ICC_PROFILE\0") => summary.icc += 1,
            0xed => summary.iptc += 1,
            0xfe => summary.text += 1,
            0xe1..=0xef if marker != 0xee => summary.other += 1,
            _ => {}
        }
        pos = end;
    }
    if let Some(end) = bytes.windows(2).rposition(|pair| pair == [0xff, 0xd9]) {
        if end + 2 < bytes.len() {
            summary.other += 1;
        }
    }
    Ok(summary)
}

fn scan_png(bytes: &[u8]) -> Result<Summary> {
    let mut summary = Summary {
        format: ImageFormat::Png,
        ..Default::default()
    };
    let mut pos = 8usize;
    let mut ended = false;
    while pos < bytes.len() {
        ensure!(pos + 12 <= bytes.len(), "PNG 块头截断");
        let length = be32(&bytes[pos..pos + 4])?;
        let kind = &bytes[pos + 4..pos + 8];
        let end = pos
            .checked_add(12)
            .and_then(|v| v.checked_add(length))
            .context("PNG 块长度溢出")?;
        ensure!(end <= bytes.len(), "PNG 块超出文件");
        match kind {
            b"eXIf" => {
                summary.exif += 1;
                summary.gps |= has_gps_ifd(&bytes[pos + 8..pos + 8 + length]);
            }
            b"iCCP" => summary.icc += 1,
            b"iTXt" | b"tEXt" | b"zTXt" => summary.text += 1,
            b"tIME" => summary.other += 1,
            b"acTL" | b"fcTL" | b"fdAT" => summary.animation = true,
            b"IEND" => {
                ended = true;
                if end < bytes.len() {
                    summary.other += 1;
                }
                break;
            }
            _ if kind[0].is_ascii_lowercase() => summary.other += 1,
            _ => {}
        }
        pos = end;
    }
    ensure!(ended, "PNG 缺少 IEND 块");
    Ok(summary)
}

fn scan_webp(bytes: &[u8]) -> Result<Summary> {
    let expected = le32(&bytes[4..8])?
        .checked_add(8)
        .context("WebP 长度溢出")?;
    ensure!(expected <= bytes.len(), "WebP RIFF 长度超出文件");
    let mut summary = Summary {
        format: ImageFormat::WebP,
        ..Default::default()
    };
    let mut pos = 12usize;
    while pos < expected {
        ensure!(pos + 8 <= expected, "WebP 块头截断");
        let kind = &bytes[pos..pos + 4];
        let length = le32(&bytes[pos + 4..pos + 8])?;
        let end = pos
            .checked_add(8)
            .and_then(|v| v.checked_add(length))
            .context("WebP 块长度溢出")?;
        ensure!(end <= expected, "WebP 块超出文件");
        match kind {
            b"EXIF" => {
                summary.exif += 1;
                let payload = &bytes[pos + 8..end];
                let tiff = payload.strip_prefix(b"Exif\0\0").unwrap_or(payload);
                summary.gps |= has_gps_ifd(tiff);
            }
            b"ICCP" => summary.icc += 1,
            b"XMP " => summary.xmp += 1,
            b"ANIM" | b"ANMF" => summary.animation = true,
            b"VP8 " | b"VP8L" | b"VP8X" | b"ALPH" => {}
            _ => summary.other += 1,
        }
        pos = end + (length % 2);
    }
    ensure!(pos == expected, "WebP 块对齐或长度无效");
    if expected < bytes.len() {
        summary.other += 1;
    }
    Ok(summary)
}

enum Job {
    Inspected {
        summary: Summary,
        image: Arc<DynamicImage>,
        original_bytes: u64,
        preview: egui::ColorImage,
    },
    Cleaned {
        encoded: Vec<u8>,
        summary: Summary,
        preview: egui::ColorImage,
    },
}

#[derive(Default)]
pub(super) struct State {
    input: String,
    output: String,
    original: Option<Arc<DynamicImage>>,
    source_summary: Option<Summary>,
    source_bytes: u64,
    jpeg_quality: u8,
    encoded: Option<Vec<u8>>,
    texture: Option<egui::TextureHandle>,
    message: String,
    error: bool,
    pending: Option<mpsc::Receiver<Result<Job, String>>>,
}

impl State {
    #[cfg(feature = "ui-preview")]
    pub(super) fn preview_fixture(&mut self, ctx: &egui::Context) {
        let image = DynamicImage::ImageRgb8(image::ImageBuffer::from_fn(960, 540, |x, y| {
            image::Rgb([(x / 4) as u8, (y / 3) as u8, 160])
        }));
        self.input = "C:\\Users\\demo\\Pictures\\trip.jpg".into();
        self.output = "C:\\Users\\demo\\Pictures\\trip-cleaned.jpg".into();
        let summary = Summary {
            format: ImageFormat::Jpeg,
            exif: 1,
            gps: true,
            icc: 1,
            xmp: 1,
            ..Default::default()
        };
        let Job::Cleaned {
            encoded, preview, ..
        } = clean_preview(&image, &summary, 85).expect("metadata preview fixture")
        else {
            unreachable!()
        };
        self.source_summary = Some(summary);
        self.source_bytes = 2_400_000;
        self.jpeg_quality = 85;
        self.original = Some(Arc::new(image.clone()));
        self.encoded = Some(encoded);
        self.texture =
            Some(ctx.load_texture("metadata-preview", preview, egui::TextureOptions::LINEAR));
        self.message = "界面预览：输出复检无已识别元数据块；确认画面后另存副本。".into();
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.pending else { return };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(80));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => Err("元数据处理线程意外结束".into()),
        };
        self.pending = None;
        match result {
            Ok(Job::Inspected {
                summary,
                image,
                original_bytes,
                preview,
            }) => {
                self.output = suggested_output(Path::new(&self.input), &summary)
                    .to_string_lossy()
                    .into_owned();
                self.source_summary = Some(summary);
                self.original = Some(image);
                self.source_bytes = original_bytes;
                self.encoded = None;
                self.texture = Some(ctx.load_texture(
                    "metadata-preview",
                    preview,
                    egui::TextureOptions::LINEAR,
                ));
                self.message = "检查完成。生成清理预览后可明确另存副本。".into();
                self.error = false;
            }
            Ok(Job::Cleaned {
                encoded,
                summary,
                preview,
            }) => {
                self.message = format!(
                    "清理预览已生成：{:.2} MB；输出复检无已识别元数据块。",
                    encoded.len() as f64 / 1_000_000.0
                );
                self.encoded = Some(encoded);
                self.texture = Some(ctx.load_texture(
                    "metadata-preview",
                    preview,
                    egui::TextureOptions::LINEAR,
                ));
                self.error = summary.has_source_metadata();
            }
            Err(error) => {
                self.message = error;
                self.error = true;
            }
        }
    }

    fn inspect(&mut self) {
        if self.pending.is_some() {
            return;
        }
        self.original = None;
        self.source_summary = None;
        self.encoded = None;
        self.texture = None;
        let path = PathBuf::from(self.input.trim());
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        self.message = "正在检查图片容器和元数据…".into();
        std::thread::spawn(move || {
            let _ = tx.send(inspect_file(&path).map_err(|e| format!("检查失败：{e:#}")));
        });
    }

    fn clean(&mut self) {
        if self.pending.is_some() {
            return;
        }
        let (Some(image), Some(summary)) = (self.original.clone(), self.source_summary.clone())
        else {
            return;
        };
        let quality = self.jpeg_quality;
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        self.encoded = None;
        self.message = "正在重新编码并复检输出…".into();
        std::thread::spawn(move || {
            let _ = tx.send(
                clean_preview(&image, &summary, quality).map_err(|e| format!("清理失败：{e:#}")),
            );
        });
    }

    fn save(&mut self) {
        let (Some(bytes), Some(summary)) = (&self.encoded, &self.source_summary) else {
            return;
        };
        let target = Path::new(self.output.trim());
        let source = Path::new(self.input.trim());
        let result = (|| -> Result<()> {
            ensure!(
                !target.as_os_str().is_empty() && target != source,
                "请指定不同于原图的输出路径"
            );
            ensure!(
                target.extension().is_some_and(|ext| ext
                    .to_string_lossy()
                    .eq_ignore_ascii_case(summary.extension())),
                "输出扩展名需与源格式一致"
            );
            super::save_image_new(target, bytes, &std::sync::atomic::AtomicBool::new(false))?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.message = format!("已保存清理副本：{}", target.display());
                self.error = false;
            }
            Err(error) => {
                self.message = format!("保存失败：{error:#}");
                self.error = true;
            }
        }
    }

    pub(super) fn ui(&mut self, ui: &mut egui::Ui) {
        self.poll(ui.ctx());
        ui.heading("图片元数据检查与清理");
        ui.label("在本机识别常见元数据块；确认预览后另存清理副本，原图保持不变。");
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
                self.original = None;
                self.source_summary = None;
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
                self.inspect();
            }
            if ui.add_enabled(!busy, egui::Button::new("检查")).clicked() {
                self.inspect();
            }
        });
        if let Some(summary) = &self.source_summary {
            if let Some(image) = &self.original {
                ui.small(format!(
                    "{} · {} × {} · {:.2} MB",
                    summary.label(),
                    image.width(),
                    image.height(),
                    self.source_bytes as f64 / 1_000_000.0
                ));
            }
            ui.horizontal_wrapped(|ui| {
                for (name, count) in [
                    ("EXIF", summary.exif),
                    ("ICC 色彩配置", summary.icc),
                    ("XMP", summary.xmp),
                    ("文本", summary.text),
                    ("IPTC", summary.iptc),
                    ("其他附加块", summary.other),
                ] {
                    ui.label(format!("{name}：{count}"));
                }
            });
            if summary.gps {
                ui.colored_label(
                    egui::Color32::from_rgb(230, 120, 60),
                    "检测到 EXIF GPS 目录，原图可能包含位置数据。",
                );
            } else if summary.exif > 0 || summary.xmp > 0 || summary.text > 0 || summary.iptc > 0 {
                ui.small(
                    "EXIF、XMP、文本或 IPTC 可能包含位置与设备信息；未发现标准 EXIF GPS 目录不等于没有位置数据。",
                );
            }
            if !summary.has_source_metadata() {
                ui.small("未发现上述常见元数据块；仍可能存在未识别的私有内容。");
            }
            ui.small("清理会重新编码。ICC 色彩配置与 EXIF 方向移除后，不同看图软件中的颜色或方向可能变化；请检查输出预览。");
            if summary.format == ImageFormat::Jpeg {
                if ui
                    .add_enabled(
                        !busy,
                        egui::Slider::new(&mut self.jpeg_quality, 35..=95).text("JPEG 质量"),
                    )
                    .changed()
                {
                    self.encoded = None;
                }
                if self.jpeg_quality == 0 {
                    self.jpeg_quality = 85;
                }
            }
            if ui
                .add_enabled(!busy, egui::Button::new("生成清理预览并复检"))
                .clicked()
            {
                self.clean();
            }
            if let Some(texture) = &self.texture {
                ui.small(if self.encoded.is_some() {
                    "清理副本预览"
                } else {
                    "原图预览（按 EXIF 方向显示）"
                });
                let size = texture.size_vec2();
                ui.image((
                    texture.id(),
                    size * (520.0 / size.x).min(280.0 / size.y).min(1.0),
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
                    egui::Button::new("确认另存清理副本"),
                )
                .clicked()
            {
                self.save();
            }
        }
        ui.label(egui::RichText::new(&self.message).color(if self.error {
            egui::Color32::from_rgb(220, 70, 75)
        } else {
            ui.visuals().text_color()
        }));
        ui.small("支持 PNG/JPEG/WebP 静态图片，输入 ≤32 MiB、≤1600 万像素；不覆盖现有文件。仅识别常见标准块，不保证清除所有私有格式数据。");
    }
}

fn suggested_output(source: &Path, summary: &Summary) -> PathBuf {
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    source.with_file_name(format!("{stem}-cleaned.{}", summary.extension()))
}

fn inspect_file(path: &Path) -> Result<Job> {
    let size = fs::metadata(path)?.len();
    ensure!(
        size > 0 && size <= MAX_INPUT_BYTES,
        "图片需为非空且不超过 32 MiB"
    );
    let mut bytes = Vec::with_capacity(size as usize);
    fs::File::open(path)?
        .take(MAX_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_INPUT_BYTES,
        "图片读取期间超过 32 MiB"
    );
    let summary = scan_metadata(&bytes)?;
    ensure!(!summary.animation, "动画图片暂不支持清理，以免丢失帧");
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    ensure!(
        reader.format() == Some(summary.format),
        "图片容器与解码格式不一致"
    );
    reader.limits(image_limits());
    let mut decoder = reader.into_decoder()?;
    let (width, height) = decoder.dimensions();
    ensure!(
        u64::from(width) * u64::from(height) <= MAX_PIXELS,
        "图片像素数超过 1600 万"
    );
    let orientation = decoder.orientation().context("无法读取图片方向")?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    let preview = preview_image(&image);
    Ok(Job::Inspected {
        summary,
        image: Arc::new(image),
        original_bytes: size,
        preview,
    })
}

pub(super) fn oriented_static_image(path: &Path) -> Result<Arc<DynamicImage>> {
    let Job::Inspected { image, .. } = inspect_file(path)? else {
        bail!("图片读取结果无效")
    };
    Ok(image)
}

fn clean_preview(image: &DynamicImage, source: &Summary, quality: u8) -> Result<Job> {
    let mut encoded = Vec::new();
    match source.format {
        ImageFormat::Jpeg => {
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut encoded,
                quality.clamp(35, 95),
            );
            encoder.encode_image(image)?;
        }
        ImageFormat::Png | ImageFormat::WebP => {
            image.write_to(&mut Cursor::new(&mut encoded), source.format)?
        }
        _ => bail!("不支持该图片格式"),
    }
    ensure!(encoded.len() <= MAX_OUTPUT_BYTES, "输出超过 128 MiB");
    let output_summary = scan_metadata(&encoded)?;
    ensure!(
        !output_summary.has_source_metadata() && !output_summary.animation,
        "重新编码的文件仍含可识别元数据，已拒绝保存"
    );
    let decoded = image::load_from_memory_with_format(&encoded, source.format)?;
    let preview = preview_image(&decoded);
    Ok(Job::Cleaned {
        encoded,
        summary: output_summary,
        preview,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;

    fn jpeg_with_exif_gps() -> Vec<u8> {
        let mut raw = Vec::new();
        DynamicImage::new_rgb8(32, 16)
            .write_to(&mut Cursor::new(&mut raw), ImageFormat::Jpeg)
            .unwrap();
        let mut tiff = b"II*\0\x08\0\0\0\x01\0".to_vec();
        tiff.extend_from_slice(&[0x25, 0x88, 4, 0, 1, 0, 0, 0, 26, 0, 0, 0]);
        tiff.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        let mut payload = b"Exif\0\0".to_vec();
        payload.extend_from_slice(&tiff);
        let length = (payload.len() + 2) as u16;
        let mut result = vec![0xff, 0xd8, 0xff, 0xe1];
        result.extend_from_slice(&length.to_be_bytes());
        result.extend_from_slice(&payload);
        result.extend_from_slice(&raw[2..]);
        result
    }

    fn jpeg_with_orientation() -> Vec<u8> {
        let mut raw = Vec::new();
        DynamicImage::new_rgb8(32, 16)
            .write_to(&mut Cursor::new(&mut raw), ImageFormat::Jpeg)
            .unwrap();
        let tiff = b"II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
        let mut payload = b"Exif\0\0".to_vec();
        payload.extend_from_slice(tiff);
        let mut result = vec![0xff, 0xd8, 0xff, 0xe1];
        result.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        result.extend_from_slice(&payload);
        result.extend_from_slice(&raw[2..]);
        result
    }

    #[test]
    fn scans_exif_gps_and_clean_copy_rechecks_without_overwriting() {
        let root = std::env::temp_dir().join(format!("zi-metadata-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let original = root.join("photo.jpg");
        let target = root.join("photo-cleaned.jpg");
        let bytes = jpeg_with_exif_gps();
        fs::write(&original, &bytes).unwrap();
        let summary = scan_metadata(&bytes).unwrap();
        assert_eq!(summary.exif, 1);
        assert!(summary.gps);
        let Job::Inspected { image, .. } = inspect_file(&original).unwrap() else {
            panic!("expected inspected image")
        };
        let Job::Cleaned {
            encoded,
            summary: clean,
            ..
        } = clean_preview(&image, &summary, 85).unwrap()
        else {
            panic!("expected clean image")
        };
        assert!(!clean.has_source_metadata());
        assert_eq!(
            image::load_from_memory(&encoded).unwrap().dimensions(),
            (32, 16)
        );
        let mut state = State {
            input: original.to_string_lossy().into_owned(),
            output: target.to_string_lossy().into_owned(),
            source_summary: Some(summary),
            encoded: Some(encoded),
            ..Default::default()
        };
        state.save();
        assert!(!state.error);
        state.save();
        assert!(state.error);
        assert_eq!(fs::read(original).unwrap(), bytes);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reencoded_png_and_webp_have_no_source_metadata() {
        let image = DynamicImage::new_rgba8(24, 12);
        for format in [ImageFormat::Png, ImageFormat::WebP] {
            let source = Summary {
                format,
                text: 1,
                icc: 1,
                ..Default::default()
            };
            let Job::Cleaned {
                encoded, summary, ..
            } = clean_preview(&image, &source, 80).unwrap()
            else {
                panic!("expected clean image")
            };
            assert!(!summary.has_source_metadata());
            assert_eq!(
                image::load_from_memory_with_format(&encoded, format)
                    .unwrap()
                    .dimensions(),
                (24, 12)
            );
            let root = std::env::temp_dir().join(format!("zi-metadata-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            let input = root.join(if format == ImageFormat::Png {
                "source.png"
            } else {
                "source.webp"
            });
            fs::write(&input, &encoded).unwrap();
            let Job::Inspected { image: loaded, .. } = inspect_file(&input).unwrap() else {
                panic!("expected inspected image")
            };
            assert_eq!(loaded.dimensions(), (24, 12));
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn clean_copy_applies_exif_orientation_before_dropping_it() {
        let root = std::env::temp_dir().join(format!("zi-metadata-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let source = root.join("rotated.jpg");
        fs::write(&source, jpeg_with_orientation()).unwrap();
        let Job::Inspected { summary, image, .. } = inspect_file(&source).unwrap() else {
            panic!("expected inspected image")
        };
        assert_eq!(summary.exif, 1);
        assert_eq!(image.dimensions(), (16, 32));
        let Job::Cleaned { encoded, .. } = clean_preview(&image, &summary, 85).unwrap() else {
            panic!("expected clean image")
        };
        assert_eq!(
            image::load_from_memory(&encoded).unwrap().dimensions(),
            (16, 32)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_truncated_containers_and_animation_markers() {
        assert!(scan_metadata(b"\xff\xd8\xff\xe1\0\x10short").is_err());
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&0u32.to_be_bytes());
        png.extend_from_slice(b"acTL");
        png.extend_from_slice(&[0; 4]);
        png.extend_from_slice(&0u32.to_be_bytes());
        png.extend_from_slice(b"IEND");
        png.extend_from_slice(&[0; 4]);
        assert!(scan_metadata(&png).unwrap().animation);
    }

    #[test]
    fn identifies_png_text_and_webp_xmp_without_showing_values() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&4u32.to_be_bytes());
        png.extend_from_slice(b"tEXt");
        png.extend_from_slice(b"GPS!");
        png.extend_from_slice(&[0; 4]);
        png.extend_from_slice(&0u32.to_be_bytes());
        png.extend_from_slice(b"IEND");
        png.extend_from_slice(&[0; 4]);
        let summary = scan_metadata(&png).unwrap();
        assert_eq!(summary.text, 1);
        assert!(summary.has_source_metadata());

        let mut webp = b"RIFF\0\0\0\0WEBP".to_vec();
        webp.extend_from_slice(b"XMP ");
        webp.extend_from_slice(&4u32.to_le_bytes());
        webp.extend_from_slice(b"GPS!");
        let riff_len = (webp.len() - 8) as u32;
        webp[4..8].copy_from_slice(&riff_len.to_le_bytes());
        let summary = scan_metadata(&webp).unwrap();
        assert_eq!(summary.xmp, 1);
        assert!(summary.has_source_metadata());
    }
}
