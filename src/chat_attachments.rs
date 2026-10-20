//! Explicit, bounded text and document snapshots for model chat.
use anyhow::{Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use eframe::egui;
use serde_json::{Value, json};
use std::{
    io::{Cursor, Read},
    path::{Path, PathBuf},
};
#[cfg(windows)]
use std::{
    sync::mpsc::{self, Receiver, TryRecvError},
    time::Duration,
};

const FILE_LIMIT: usize = 64 * 1024;
const TOTAL_LIMIT: usize = 128 * 1024;
const DOCUMENT_FILE_LIMIT: usize = 4 * 1024 * 1024;
const IMAGE_SOURCE_LIMIT: usize = 8 * 1024 * 1024;
const IMAGE_ITEM_LIMIT: usize = 512 * 1024;
const IMAGE_TOTAL_LIMIT: usize = 768 * 1024;
const IMAGE_EDGE_LIMIT: u32 = 2048;
const MAX_ATTACHMENTS: usize = 4;
struct Attachment {
    name: String,
    content: String,
}
struct ImageAttachment {
    id: uuid::Uuid,
    name: String,
    width: u32,
    height: u32,
    bytes: Vec<u8>,
    preview_size: [usize; 2],
    preview_rgba: Vec<u8>,
    texture: Option<egui::TextureHandle>,
    ocr_text: Option<String>,
    ocr_error: String,
}
#[cfg(windows)]
struct OcrJob {
    image_id: uuid::Uuid,
    receiver: Receiver<std::result::Result<String, String>>,
}
#[derive(Default)]
pub struct State {
    files: Vec<Attachment>,
    images: Vec<ImageAttachment>,
    path: String,
    message: String,
    #[cfg(windows)]
    ocr_job: Option<OcrJob>,
    #[cfg(feature = "ui-preview")]
    preview_scroll: bool,
}
impl State {
    #[cfg(feature = "ui-preview")]
    pub fn preview(&mut self) {
        let preview_image =
            image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(320, 180, |x, y| {
                if x < 8 || y < 8 || x >= 312 || y >= 172 {
                    image::Rgb([47, 93, 145])
                } else if y < 54 {
                    image::Rgb([222, 235, 250])
                } else {
                    image::Rgb([
                        86u8.saturating_add((x % 48) as u8),
                        132u8.saturating_add((y % 60) as u8),
                        91,
                    ])
                }
            }));
        let mut image_bytes = Cursor::new(Vec::new());
        zi_image_core::encode(
            &preview_image,
            image::ImageFormat::Jpeg,
            84,
            &mut image_bytes,
        )
        .expect("synthetic preview image encodes");
        let thumbnail = preview_image.to_rgba8();
        self.files = vec![
            Attachment {
                name: "示例配置.json".into(),
                content: "{\n  \"name\": \"demo\",\n  \"enabled\": true\n}".into(),
            },
            Attachment {
                name: "产品说明.pdf".into(),
                content: "[第 1 页]\n本机提取的 PDF 正文保留页码，扫描型页面需要 OCR。".into(),
            },
        ];
        self.images = vec![ImageAttachment {
            id: uuid::Uuid::new_v4(),
            name: "界面示例.png".into(),
            width: 320,
            height: 180,
            bytes: image_bytes.into_inner(),
            preview_size: [thumbnail.width() as usize, thumbnail.height() as usize],
            preview_rgba: thumbnail.into_raw(),
            texture: None,
            ocr_text: Some("示例 OCR 结果：界面示例（截图用合成内容）".into()),
            ocr_error: String::new(),
        }];
        self.message = "示例文本 / PDF 摘录、图片与 OCR 结果 · 截图未读取用户文件或发送请求".into();
        self.preview_scroll = true;
    }
    pub fn clear(&mut self) {
        #[cfg(windows)]
        let ocr_running = self.ocr_job.is_some();
        self.files.clear();
        self.images.clear();
        #[cfg(windows)]
        {
            self.ocr_job = None;
        }
        self.path.clear();
        self.message.clear();
        #[cfg(windows)]
        if ocr_running {
            self.message = "附件已清除；已开始的 Windows OCR 无法中途取消，结果会丢弃".into();
        }
    }
    fn attachment_count(&self) -> usize {
        self.files.len() + self.images.len()
    }
    pub fn has_images(&self) -> bool {
        !self.images.is_empty()
    }
    fn load(&mut self) -> Result<()> {
        let path = PathBuf::from(self.path.trim());
        self.load_path(&path)
    }
    fn load_path(&mut self, path: &Path) -> Result<()> {
        ensure!(
            self.attachment_count() < MAX_ATTACHMENTS,
            "最多添加 4 个附件"
        );
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        ensure!(
            matches!(
                extension.as_str(),
                "txt"
                    | "md"
                    | "json"
                    | "yaml"
                    | "yml"
                    | "csv"
                    | "tsv"
                    | "log"
                    | "py"
                    | "java"
                    | "xml"
                    | "toml"
                    | "properties"
                    | "sql"
                    | "js"
                    | "ts"
                    | "html"
                    | "css"
                    | "rs"
                    | "ini"
                    | "pdf"
                    | "docx"
                    | "png"
                    | "jpg"
                    | "jpeg"
                    | "webp"
            ),
            "支持 UTF-8 文本/代码、PDF、DOCX 及 PNG/JPEG/WebP 图片"
        );
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow::anyhow!("文件名无效"))?
            .chars()
            .filter(|ch| !ch.is_control())
            .take(128)
            .collect::<String>();
        ensure!(
            !self.files.iter().any(|f| f.name == name)
                && !self.images.iter().any(|f| f.name == name),
            "已有同名附件，请先移除后重新读取"
        );
        ensure!(std::fs::metadata(path)?.is_file(), "请选择普通文件");
        let document = matches!(extension.as_str(), "pdf" | "docx");
        let image = matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp");
        let input_limit = if document {
            DOCUMENT_FILE_LIMIT
        } else if image {
            IMAGE_SOURCE_LIMIT
        } else {
            FILE_LIMIT
        };
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take((input_limit + 1) as u64)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= input_limit,
            "附件原文件超过当前格式的大小限制"
        );
        if image {
            let decoded = zi_image_core::decode(&bytes)?;
            let width = decoded.width();
            let height = decoded.height();
            let resized = if width > IMAGE_EDGE_LIMIT || height > IMAGE_EDGE_LIMIT {
                decoded.resize(
                    IMAGE_EDGE_LIMIT,
                    IMAGE_EDGE_LIMIT,
                    image::imageops::FilterType::Lanczos3,
                )
            } else {
                decoded.clone()
            };
            let mut rgb = resized.to_rgba8();
            for pixel in rgb.pixels_mut() {
                let alpha = u32::from(pixel[3]);
                for channel in 0..3 {
                    pixel[channel] =
                        ((u32::from(pixel[channel]) * alpha + 255 * (255 - alpha) + 127) / 255)
                            as u8;
                }
                pixel[3] = 255;
            }
            let mut cursor = Cursor::new(Vec::new());
            zi_image_core::encode(
                &image::DynamicImage::ImageRgba8(rgb),
                image::ImageFormat::Jpeg,
                84,
                &mut cursor,
            )?;
            let normalized = cursor.into_inner();
            ensure!(
                normalized.len() <= IMAGE_ITEM_LIMIT,
                "图片压缩后仍超过 512 KiB；请先用图片工具缩小尺寸或压缩"
            );
            ensure!(
                self.images.iter().map(|f| f.bytes.len()).sum::<usize>() + normalized.len()
                    <= IMAGE_TOTAL_LIMIT,
                "图片附件总量超过 768 KiB；请减少图片或压缩后再添加"
            );
            let thumb_scale = (256.0 / resized.width() as f32)
                .min(160.0 / resized.height() as f32)
                .min(1.0);
            let thumb_width = (resized.width() as f32 * thumb_scale).round().max(1.0) as u32;
            let thumb_height = (resized.height() as f32 * thumb_scale).round().max(1.0) as u32;
            let thumbnail = resized
                .resize_exact(
                    thumb_width,
                    thumb_height,
                    image::imageops::FilterType::Lanczos3,
                )
                .to_rgba8();
            self.images.push(ImageAttachment {
                id: uuid::Uuid::new_v4(),
                name,
                width,
                height,
                bytes: normalized,
                preview_size: [thumbnail.width() as usize, thumbnail.height() as usize],
                preview_rgba: thumbnail.into_raw(),
                texture: None,
                ocr_text: None,
                ocr_error: String::new(),
            });
            self.path.clear();
            return Ok(());
        }
        let text = if document {
            crate::document_ingestion::extract_chat_attachment(&bytes, &extension, FILE_LIMIT)?
        } else {
            std::str::from_utf8(&bytes)
                .map_err(|_| anyhow::anyhow!("附件必须为 UTF-8 文本"))?
                .trim_start_matches('\u{feff}')
                .to_owned()
        };
        ensure!(!text.trim().is_empty(), "附件内容为空");
        ensure!(
            !text
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')),
            "附件包含二进制控制字符"
        );
        ensure!(
            self.files.iter().map(|f| f.content.len()).sum::<usize>() + text.len() <= TOTAL_LIMIT,
            "附件正文总量超过 128 KiB"
        );
        self.files.push(Attachment {
            name,
            content: text,
        });
        self.path.clear();
        Ok(())
    }
    fn add_ocr_text(&mut self, image_index: usize) -> Result<()> {
        ensure!(
            self.attachment_count() < MAX_ATTACHMENTS,
            "附件数量已达 4 个上限，移除一个附件后再添加 OCR 文本"
        );
        let image = self
            .images
            .get(image_index)
            .ok_or_else(|| anyhow::anyhow!("图片附件已移除"))?;
        let content = image
            .ocr_text
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("请先完成本机 OCR"))?;
        ensure!(!content.trim().is_empty(), "OCR 识别结果为空");
        ensure!(
            content.len() <= FILE_LIMIT,
            "OCR 结果超过 64 KiB，无法作为模型文本附件添加"
        );
        ensure!(
            !content
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')),
            "OCR 结果包含二进制控制字符"
        );
        ensure!(
            self.files
                .iter()
                .map(|file| file.content.len())
                .sum::<usize>()
                + content.len()
                <= TOTAL_LIMIT,
            "OCR 文本加入后会超过 128 KiB 附件正文总量"
        );
        let name = format!("{} [OCR]", image.name)
            .chars()
            .take(128)
            .collect::<String>();
        ensure!(
            !self.files.iter().any(|file| file.name == name)
                && !self.images.iter().any(|file| file.name == name),
            "已有同名 OCR 附件，请先移除"
        );
        self.files.push(Attachment {
            name,
            content: content.clone(),
        });
        Ok(())
    }
    #[cfg(windows)]
    fn poll_ocr(&mut self, ctx: &egui::Context) {
        let completed = self
            .ocr_job
            .as_ref()
            .and_then(|job| match job.receiver.try_recv() {
                Ok(result) => Some((job.image_id, result)),
                Err(TryRecvError::Disconnected) => {
                    Some((job.image_id, Err("本机 OCR 后台任务意外结束".to_owned())))
                }
                Err(TryRecvError::Empty) => None,
            });
        if let Some((image_id, result)) = completed {
            self.ocr_job = None;
            if let Some(image) = self.images.iter_mut().find(|image| image.id == image_id) {
                match result {
                    Ok(text) => {
                        image.ocr_error.clear();
                        image.ocr_text = Some(text);
                        self.message =
                            "本机 OCR 完成；结果仅在当前工作台预览，尚未加入模型消息".into();
                    }
                    Err(error) => {
                        image.ocr_text = None;
                        image.ocr_error = error;
                        self.message = "本机 OCR 未完成，请查看图片附件的错误信息".into();
                    }
                }
            }
        }
        if self.ocr_job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
    #[cfg(windows)]
    fn start_ocr(&mut self, image_index: usize, ctx: &egui::Context) -> Result<()> {
        ensure!(self.ocr_job.is_none(), "已有一项本机 OCR 正在运行");
        let image = self
            .images
            .get_mut(image_index)
            .ok_or_else(|| anyhow::anyhow!("图片附件已移除"))?;
        image.ocr_text = None;
        image.ocr_error.clear();
        let image_id = image.id;
        let bytes = image.bytes.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("zi-chat-image-ocr".into())
            .spawn(move || {
                let result = zi_image_core::decode(&bytes)
                    .map_err(|error| format!("无法解码图片供本机 OCR 使用：{error:#}"))
                    .and_then(|image| crate::clipboard::recognize_image(&image));
                let _ = sender.send(result);
            })
            .map_err(|error| anyhow::anyhow!("无法启动本机 OCR 任务：{error}"))?;
        self.ocr_job = Some(OcrJob { image_id, receiver });
        self.message =
            "正在使用 Windows 系统首选语言进行本机 OCR；识别完成前不会发送图片或文本".into();
        ctx.request_repaint();
        Ok(())
    }
    pub fn compose(&self, input: &str) -> Result<String> {
        if self.files.is_empty() && self.images.is_empty() {
            return Ok(input.to_owned());
        }
        ensure!(!input.trim().is_empty(), "请填写对附件的处理要求");
        let mut text = input.to_owned();
        if !self.files.is_empty() {
            let files: Vec<_> = self
                .files
                .iter()
                .map(|file| json!({"name":file.name,"content":file.content}))
                .collect();
            text.push_str("\n\n以下为用户选择的文本附件（JSON 中仅含文件名及正文）：\n");
            text.push_str(&serde_json::to_string_pretty(&files)?);
        }
        if !self.images.is_empty() {
            let images: Vec<_> = self
                .images
                .iter()
                .map(|file| format!("{} · {}×{}", file.name, file.width, file.height))
                .collect();
            text.push_str("\n\n本轮还附带了以下图片（实际图像数据随请求发送）：\n");
            text.push_str(&images.join("\n"));
            text.push_str("\n会话文本导出只保留图片名称与尺寸，不包含图像数据。后续轮次如需再次分析图片，请重新附加。");
        }
        ensure!(
            text.len() <= crate::conversation::MAX_BYTES,
            "输入与附件编码后超过 256 KiB，请减少附件或缩短输入"
        );
        Ok(text)
    }
    pub fn attach_to_messages(
        &self,
        messages: &mut Vec<Value>,
        input: &str,
        ollama: bool,
    ) -> Result<()> {
        if self.images.is_empty() {
            return Ok(());
        }
        if messages.is_empty() {
            messages.push(json!({"role":"user","content":input}));
        }
        let last = messages.last_mut().expect("message was added");
        ensure!(last["role"] == "user", "图片只能附加到最后一条用户消息");
        let content = last
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("最后一条用户消息正文格式无效"))?
            .to_owned();
        if ollama {
            last["images"] = Value::Array(
                self.images
                    .iter()
                    .map(|file| Value::String(STANDARD.encode(&file.bytes)))
                    .collect(),
            );
        } else {
            let mut parts = vec![json!({"type":"text","text":content})];
            parts.extend(self.images.iter().map(|file| {
                json!({
                    "type":"image_url",
                    "image_url":{"url":format!("data:image/jpeg;base64,{}", STANDARD.encode(&file.bytes)),"detail":"auto"}
                })
            }));
            last["content"] = Value::Array(parts);
        }
        Ok(())
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, enabled: bool) {
        #[cfg(windows)]
        self.poll_ocr(ui.ctx());
        ui.add_enabled_ui(enabled, |ui| {
            let response = egui::CollapsingHeader::new(format!("模型附件 · {} / 4", self.attachment_count()))
                .id_salt("chat-attachments")
                .default_open(self.attachment_count() > 0)
                .show(ui, |ui| {
                ui.weak("点击运行后，所选正文和图片会发送到当前模型服务（可能是远程服务）；只读取你明确选择的文件，不发送路径。支持文本/代码、PDF/DOCX 和 PNG/JPEG/WebP。图片先在本机解码、缩小至最长边2048并转为JPEG预览；单张8 MiB原文件、512 KiB发送数据，图片合计768 KiB。Windows 可在本机识别图片文字；识别结果需手动添加为文本附件，之后仍需点击运行才会发送。图片数据只用于当前请求，不进入会话导出；后续轮次需重新附加。扫描型PDF当前不做OCR。");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.path).hint_text("支持文本、PDF、DOCX、PNG、JPEG、WebP 文件路径").desired_width(ui.available_width() - 104.0));
                    #[cfg(windows)]
                    if ui.button("选择文件").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("文本、文档和图片", &["txt", "md", "json", "yaml", "yml", "csv", "tsv", "log", "py", "java", "xml", "toml", "properties", "sql", "js", "ts", "html", "css", "rs", "ini", "pdf", "docx", "png", "jpg", "jpeg", "webp"])
                            .pick_file()
                        {
                            self.message = match self.load_path(&path) {
                                Ok(()) => "附件已读取并预览，尚未发送；文件变化需移除后重新添加".into(),
                                Err(e) => e.to_string(),
                            };
                        }
                    }
                });
                if ui.button("读取附件并预览").clicked() {
                    self.message = match self.load() { Ok(()) => "附件已读取，尚未发送；文件变化需移除后重新添加".into(), Err(e) => e.to_string() };
                }
                let mut remove = None;
                for (index, file) in self.files.iter().enumerate() {
                    ui.horizontal(|ui| { ui.strong(format!("{} · {} 字节", file.name, file.content.len())); if ui.button("移除").clicked() { remove = Some(index); } });
                    egui::ScrollArea::vertical().id_salt(("attachment", index)).max_height(100.0).show(ui, |ui| { ui.monospace(&file.content); });
                }
                if let Some(index) = remove { self.files.remove(index); self.message = "附件已移除".into(); }
                let mut remove_image = None;
                #[cfg(windows)]
                let mut start_ocr = None;
                let mut add_ocr = None;
                #[cfg(windows)]
                let ocr_busy = self.ocr_job.is_some();
                #[cfg(windows)]
                let ocr_image_id = self.ocr_job.as_ref().map(|job| job.image_id);
                #[cfg(windows)]
                let attachment_count = self.attachment_count();
                #[cfg(windows)]
                let text_total_bytes = self.files.iter().map(|file| file.content.len()).sum::<usize>();
                for (index, file) in self.images.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.strong(format!("{} · {}×{} · {} KiB", file.name, file.width, file.height, file.bytes.len().div_ceil(1024)));
                        if ui.button("移除").clicked() { remove_image = Some(index); }
                    });
                    if file.texture.is_none() {
                        let color = egui::ColorImage::from_rgba_unmultiplied(file.preview_size, &file.preview_rgba);
                        file.texture = Some(ui.ctx().load_texture(format!("chat-image-preview-{index}-{}", file.name), color, egui::TextureOptions::LINEAR));
                    }
                    if let Some(texture) = &file.texture {
                        ui.add(egui::Image::new(texture).max_size(egui::vec2(256.0, 160.0)));
                    }
                    #[cfg(windows)]
                    {
                        ui.horizontal(|ui| {
                            if ocr_image_id == Some(file.id) {
                                ui.spinner();
                                ui.label("本机 OCR 识别中…");
                            } else if ui
                                .add_enabled(!ocr_busy, egui::Button::new("本机识别图片文字"))
                                .clicked()
                            {
                                start_ocr = Some(index);
                            }
                            if let Some(text) = &file.ocr_text {
                                let text_fits = text.len() <= FILE_LIMIT
                                    && text_total_bytes + text.len() <= TOTAL_LIMIT;
                                let can_add = text_fits && attachment_count < MAX_ATTACHMENTS;
                                if ui
                                    .add_enabled(can_add, egui::Button::new("添加为文本附件"))
                                    .clicked()
                                {
                                    add_ocr = Some(index);
                                }
                            }
                        });
                        if file.ocr_text.is_some() {
                            let text_len = file.ocr_text.as_ref().map_or(0, String::len);
                            if text_len > FILE_LIMIT {
                                ui.colored_label(egui::Color32::YELLOW, "识别结果超过 64 KiB，不能作为模型文本附件添加");
                            } else if attachment_count >= MAX_ATTACHMENTS {
                                ui.weak("附件数量已达 4 个上限；移除一个附件后可添加识别文本。");
                            }
                            egui::ScrollArea::vertical()
                                .id_salt(("chat-image-ocr", index))
                                .max_height(100.0)
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::TextEdit::multiline(file.ocr_text.as_mut().unwrap())
                                            .desired_rows(3)
                                            .desired_width(ui.available_width()),
                                    );
                                });
                        }
                        if !file.ocr_error.is_empty() {
                            ui.colored_label(egui::Color32::LIGHT_RED, &file.ocr_error);
                        }
                    }
                    #[cfg(not(windows))]
                    ui.weak("图片本机 OCR 仅在 Windows 版本提供。");
                }
                #[cfg(windows)]
                let ctx = ui.ctx().clone();
                if let Some(index) = remove_image {
                    #[cfg(windows)]
                    let removed_id = self.images.get(index).map(|image| image.id);
                    self.images.remove(index);
                    #[cfg(windows)]
                    if removed_id.is_some() && removed_id == ocr_image_id {
                        self.ocr_job = None;
                        self.message = "图片已移除；正在进行的 Windows OCR 无法中途取消，识别结果会丢弃".into();
                    } else {
                        self.message = "图片附件已移除".into();
                    }
                    #[cfg(not(windows))]
                    { self.message = "图片附件已移除".into(); }
                } else {
                    #[cfg(windows)]
                    if let Some(index) = add_ocr {
                        self.message = match self.add_ocr_text(index) {
                            Ok(()) => "OCR 文字已作为本机文本附件加入预览；尚未发送".into(),
                            Err(error) => error.to_string(),
                        };
                    } else if let Some(index) = start_ocr {
                        if let Err(error) = self.start_ocr(index, &ctx) {
                            self.message = error.to_string();
                        }
                    }
                }
                if !self.message.is_empty() { ui.label(&self.message); }
            });
            #[cfg(feature = "ui-preview")]
            if self.preview_scroll {
                response.header_response.scroll_to_me(Some(egui::Align::TOP));
                self.preview_scroll = false;
            }
            #[cfg(not(feature = "ui-preview"))]
            let _ = response;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;
    fn png_fixture() -> Vec<u8> {
        let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(48, 32, |x, y| {
            image::Rgba([x as u8 * 3, y as u8 * 5, 90, if x < 12 { 0 } else { 255 }])
        }));
        let mut cursor = Cursor::new(Vec::new());
        image
            .write_to(&mut cursor, image::ImageFormat::Png)
            .unwrap();
        cursor.into_inner()
    }

    #[test]
    fn image_is_decoded_bounded_previewed_and_added_to_provider_specific_message_shapes() {
        let root = std::env::temp_dir().join(format!("zi-chat-image-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("界面.png");
        std::fs::write(&path, png_fixture()).unwrap();
        let mut state = State {
            path: path.to_string_lossy().into_owned(),
            ..Default::default()
        };
        state.load().unwrap();
        assert_eq!(state.images.len(), 1);
        assert_eq!((state.images[0].width, state.images[0].height), (48, 32));
        assert_eq!(state.images[0].preview_size, [48, 32]);
        assert!(state.images[0].bytes.len() <= IMAGE_ITEM_LIMIT);
        let normalized = zi_image_core::decode(&state.images[0].bytes).unwrap();
        assert_eq!(normalized.color(), image::ColorType::Rgb8);
        let transparent = normalized.get_pixel(0, 0);
        assert!(transparent.0.iter().all(|channel| *channel >= 250));
        let composed = state.compose("请描述图片").unwrap();
        assert!(composed.contains("界面.png · 48×32"));
        assert!(!composed.contains(root.to_string_lossy().as_ref()));

        let mut openai = vec![
            json!({"role":"user","content":"此前问题"}),
            json!({"role":"assistant","content":"此前回答"}),
            json!({"role":"user","content":composed}),
        ];
        state
            .attach_to_messages(&mut openai, &composed, false)
            .unwrap();
        assert_eq!(openai[0]["content"], "此前问题");
        assert_eq!(
            openai[2]["content"][0],
            json!({"type":"text","text":composed})
        );
        let url = openai[2]["content"][1]["image_url"]["url"]
            .as_str()
            .unwrap();
        assert!(url.starts_with("data:image/jpeg;base64,"));
        let decoded = STANDARD
            .decode(url.trim_start_matches("data:image/jpeg;base64,"))
            .unwrap();
        assert_eq!(decoded, state.images[0].bytes);
        let body = json!({"model":"$model","messages":[{"role":"user","content":"$input"}],"stream":false});
        let payload: Value = serde_json::from_slice(
            &crate::plugins::request_body(&body, &composed, "vision-model", Some(&openai)).unwrap(),
        )
        .unwrap();
        assert_eq!(payload["messages"][2]["content"][1]["type"], "image_url");
        assert!(
            !payload
                .to_string()
                .contains(root.to_string_lossy().as_ref())
        );

        let mut ollama = vec![json!({"role":"user","content":composed})];
        state
            .attach_to_messages(&mut ollama, &composed, true)
            .unwrap();
        assert_eq!(ollama[0]["content"], composed);
        assert_eq!(
            STANDARD
                .decode(ollama[0]["images"][0].as_str().unwrap())
                .unwrap(),
            state.images[0].bytes
        );
        assert_eq!(ollama[0].as_object().unwrap().len(), 3);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn image_limits_and_invalid_inputs_fail_without_partial_attachment() {
        let root =
            std::env::temp_dir().join(format!("zi-chat-image-limit-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("image.png");
        std::fs::write(&path, png_fixture()).unwrap();
        let mut state = State::default();
        state.images.push(ImageAttachment {
            id: uuid::Uuid::new_v4(),
            name: "existing.jpg".into(),
            width: 1,
            height: 1,
            bytes: vec![0; IMAGE_TOTAL_LIMIT],
            preview_size: [1, 1],
            preview_rgba: vec![0; 4],
            texture: None,
            ocr_text: None,
            ocr_error: String::new(),
        });
        state.path = path.to_string_lossy().into_owned();
        assert!(state.load().is_err());
        assert_eq!(state.images.len(), 1);
        state.images.clear();
        std::fs::write(&path, b"not an image").unwrap();
        assert!(state.load().is_err());
        assert!(state.images.is_empty());
        std::fs::write(&path, vec![0u8; IMAGE_SOURCE_LIMIT + 1]).unwrap();
        assert!(state.load().is_err());
        assert!(state.images.is_empty());
        state.files = (0..MAX_ATTACHMENTS)
            .map(|index| Attachment {
                name: format!("{index}.txt"),
                content: "x".into(),
            })
            .collect();
        assert!(state.load().is_err());
        assert!(state.images.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recognized_text_stays_out_of_the_message_until_explicitly_added() {
        let mut state = State::default();
        state.images.push(ImageAttachment {
            id: uuid::Uuid::new_v4(),
            name: "receipt.png".into(),
            width: 8,
            height: 8,
            bytes: vec![1, 2, 3],
            preview_size: [1, 1],
            preview_rgba: vec![0, 0, 0, 255],
            texture: None,
            ocr_text: Some("Total: 42".into()),
            ocr_error: String::new(),
        });

        let before = state.compose("请分析附件").unwrap();
        assert!(!before.contains("Total: 42"));
        state.add_ocr_text(0).unwrap();
        assert_eq!(state.files[0].name, "receipt.png [OCR]");
        assert_eq!(state.files[0].content, "Total: 42");
        assert!(state.compose("请分析附件").unwrap().contains("Total: 42"));
    }

    #[test]
    fn recognized_text_respects_per_file_total_and_attachment_limits_atomically() {
        let make_image = |text: String| ImageAttachment {
            id: uuid::Uuid::new_v4(),
            name: "scan.png".into(),
            width: 8,
            height: 8,
            bytes: vec![1, 2, 3],
            preview_size: [1, 1],
            preview_rgba: vec![0, 0, 0, 255],
            texture: None,
            ocr_text: Some(text),
            ocr_error: String::new(),
        };
        let mut state = State::default();
        state.images.push(make_image("x".repeat(FILE_LIMIT + 1)));
        assert!(state.add_ocr_text(0).is_err());
        assert!(state.files.is_empty());

        state.images[0].ocr_text = Some("short".into());
        state.files.push(Attachment {
            name: "existing.txt".into(),
            content: "x".repeat(TOTAL_LIMIT - 4),
        });
        assert!(state.add_ocr_text(0).is_err());
        assert_eq!(state.files.len(), 1);

        state.files[0].content.clear();
        state.files.extend((0..2).map(|index| Attachment {
            name: format!("existing-{index}.txt"),
            content: String::new(),
        }));
        assert_eq!(state.attachment_count(), MAX_ATTACHMENTS);
        assert!(state.add_ocr_text(0).is_err());
        assert_eq!(state.files.len(), 3);
    }

    #[test]
    fn snapshots_are_bounded_literal_and_do_not_expose_paths() {
        let root = std::env::temp_dir().join(format!("zi-attachment-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("示例.json");
        std::fs::write(&path, "\u{feff}{\"key\":\"$input🙂\"}").unwrap();
        let mut state = State {
            path: path.to_str().unwrap().into(),
            ..Default::default()
        };
        state.load().unwrap();
        std::fs::write(&path, "changed").unwrap();
        let composed = state.compose("解释").unwrap();
        assert!(composed.contains("$input🙂"));
        assert!(!composed.contains("changed"));
        assert!(!composed.contains(root.to_str().unwrap()));
        let body =
            serde_json::json!({"model":"$model","messages":[{"role":"user","content":"$input"}]});
        let payload: serde_json::Value = serde_json::from_slice(
            &crate::plugins::request_body(&body, &composed, "example", None).unwrap(),
        )
        .unwrap();
        assert_eq!(payload["messages"][0]["content"], composed);
        assert!(state.compose("").is_err());
        assert!(
            state
                .compose(&"x".repeat(crate::conversation::MAX_BYTES))
                .is_err()
        );
        state.path = path.to_str().unwrap().into();
        assert!(state.load().is_err());
        state.clear();
        state.path = path.to_str().unwrap().into();
        for bytes in [vec![0, 1], vec![255], vec![b'x'; FILE_LIMIT + 1]] {
            std::fs::write(&path, bytes).unwrap();
            assert!(state.load().is_err());
            assert!(state.files.is_empty());
        }
        state.files = (0..4)
            .map(|i| Attachment {
                name: i.to_string(),
                content: "text".into(),
            })
            .collect();
        assert!(state.load().is_err());
        state.files = vec![Attachment {
            name: "other".into(),
            content: "x".repeat(TOTAL_LIMIT),
        }];
        std::fs::write(&path, "a").unwrap();
        assert!(state.load().is_err());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn docx_is_extracted_locally_with_bounded_paragraph_preview() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("zi-chat-docx-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("计划.docx");
        let buffer = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(buffer);
        writer
            .start_file(
                "word/document.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer
            .write_all(
                r#"<w:document xmlns:w="x"><w:body><w:p><w:r><w:t>第一段</w:t></w:r></w:p><w:p><w:r><w:t>第二段</w:t></w:r></w:p></w:body></w:document>"#
                    .as_bytes(),
            )
            .unwrap();
        std::fs::write(&path, writer.finish().unwrap().into_inner()).unwrap();

        let mut state = State {
            path: path.to_string_lossy().into_owned(),
            ..Default::default()
        };
        state.load().unwrap();
        let composed = state.compose("总结这份计划").unwrap();
        assert!(composed.contains("计划.docx"));
        assert!(composed.contains("段落 1"));
        assert!(composed.contains("第一段"));
        assert!(composed.contains("段落 2"));
        assert!(composed.contains("第二段"));
        assert!(!composed.contains(root.to_string_lossy().as_ref()));

        let too_long = root.join("过长.docx");
        let buffer = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(buffer);
        writer
            .start_file(
                "word/document.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        let xml = format!(
            "<w:document xmlns:w=\"x\"><w:body><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:body></w:document>",
            "字".repeat(FILE_LIMIT)
        );
        writer.write_all(xml.as_bytes()).unwrap();
        std::fs::write(&too_long, writer.finish().unwrap().into_inner()).unwrap();
        state.path = too_long.to_string_lossy().into_owned();
        assert!(state.load().is_err());
        assert_eq!(state.files.len(), 1);

        let old_doc = root.join("旧版.doc");
        std::fs::write(&old_doc, b"unsupported").unwrap();
        state.path = old_doc.to_string_lossy().into_owned();
        assert!(state.load().is_err());
        assert_eq!(state.files.len(), 1);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pdf_attachment_keeps_page_provenance_and_never_includes_its_path() {
        use lopdf::{
            Document, Object, Stream,
            content::{Content, Operation},
            dictionary,
        };
        let root = std::env::temp_dir().join(format!("zi-chat-pdf-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("说明.pdf");
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(
            dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier" },
        );
        let resources_id =
            doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Tj", vec![Object::string_literal("PDF attachment text")]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page_id = doc.add_object(
            dictionary! { "Type" => "Page", "Parent" => pages_id, "Contents" => content_id },
        );
        doc.objects.insert(
            pages_id,
            Object::Dictionary(
                dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1, "Resources" => resources_id, "MediaBox" => vec![0.into(),0.into(),595.into(),842.into()] },
            ),
        );
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        std::fs::write(&path, bytes).unwrap();

        let mut state = State {
            path: path.to_string_lossy().into_owned(),
            ..Default::default()
        };
        state.load().unwrap();
        let composed = state.compose("概括文件").unwrap();
        assert!(composed.contains("说明.pdf"));
        assert!(composed.contains("第 1 页"));
        assert!(composed.contains("PDF attachment text"));
        assert!(!composed.contains(root.to_string_lossy().as_ref()));

        std::fs::remove_dir_all(root).unwrap();
    }
}
