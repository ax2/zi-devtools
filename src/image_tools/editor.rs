//! Local full-resolution crop, redaction, arrows and text with encoded preview.
use super::{Format, MAX_OUTPUT_BYTES, metadata, preview_image};
use ab_glyph::{Font, FontVec, PxScale, ScaleFont, point};
use anyhow::{Result, bail, ensure};
use eframe::egui::{self, Color32, Sense, Stroke};
use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Point {
    x: f32,
    y: f32,
}
impl Point {
    fn new(x: f32, y: f32) -> Self {
        Self {
            x: x.clamp(0.0, 1.0),
            y: y.clamp(0.0, 1.0),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    a: Point,
    b: Point,
}
impl Rect {
    fn sorted(self) -> Self {
        Self {
            a: Point::new(self.a.x.min(self.b.x), self.a.y.min(self.b.y)),
            b: Point::new(self.a.x.max(self.b.x), self.a.y.max(self.b.y)),
        }
    }
    fn pixel_bounds(self, width: u32, height: u32) -> (u32, u32, u32, u32) {
        let s = self.sorted();
        let x0 = (s.a.x * width as f32).floor().clamp(0.0, width as f32) as u32;
        let y0 = (s.a.y * height as f32).floor().clamp(0.0, height as f32) as u32;
        let x1 = (s.b.x * width as f32).ceil().clamp(0.0, width as f32) as u32;
        let y1 = (s.b.y * height as f32).ceil().clamp(0.0, height as f32) as u32;
        (x0, y0, x1, y1)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tool {
    #[default]
    Crop,
    Redact,
    Arrow,
    Text,
}

#[derive(Clone, Debug)]
enum Edit {
    Crop(Rect),
    Redact(Rect),
    Arrow {
        a: Point,
        b: Point,
        color: [u8; 4],
        width: u32,
    },
    Text {
        at: Point,
        value: String,
        color: [u8; 4],
        size: u32,
    },
}

enum Job {
    Loaded {
        image: Arc<DynamicImage>,
        preview: egui::ColorImage,
    },
    Rendered {
        encoded: Vec<u8>,
        preview: egui::ColorImage,
        width: u32,
        height: u32,
    },
}

pub(super) struct State {
    input: String,
    origins: Vec<super::relay::Origin>,
    output: String,
    source: Option<Arc<DynamicImage>>,
    source_texture: Option<egui::TextureHandle>,
    output_texture: Option<egui::TextureHandle>,
    encoded: Option<Arc<Vec<u8>>>,
    edits: Vec<Edit>,
    tool: Tool,
    drag_start: Option<Point>,
    annotation_text: String,
    color: [u8; 4],
    stroke_width: u32,
    font_size: u32,
    font_path: String,
    format: Format,
    jpeg_quality: u8,
    pending: Option<mpsc::Receiver<Result<Job, String>>>,
    message: String,
    error: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            input: String::new(),
            origins: Vec::new(),
            output: String::new(),
            source: None,
            source_texture: None,
            output_texture: None,
            encoded: None,
            edits: Vec::new(),
            tool: Tool::Crop,
            drag_start: None,
            annotation_text: "标注".into(),
            color: [244, 68, 68, 255],
            stroke_width: 6,
            font_size: 42,
            font_path: String::new(),
            format: Format::Png,
            jpeg_quality: 85,
            pending: None,
            message: String::new(),
            error: false,
        }
    }
}

impl State {
    pub(super) fn relay_source(
        &self,
    ) -> Option<(
        super::relay::Source,
        Vec<super::relay::Origin>,
        &'static str,
    )> {
        if self.pending.is_some() {
            return None;
        }
        // Pending edits must be rendered before transfer; never silently send the unedited source.
        if let Some(encoded) = &self.encoded {
            return Some((
                super::relay::Source::Encoded(encoded.clone()),
                self.origins.clone(),
                "image-crop-annotate",
            ));
        }
        if !self.edits.is_empty() {
            return None;
        }
        self.source.as_ref().map(|image| {
            (
                super::relay::Source::Image(image.clone()),
                self.origins.clone(),
                "image-crop-annotate",
            )
        })
    }
    pub(super) fn relay_target_state(&self) -> (bool, bool) {
        (
            self.pending.is_some(),
            self.source.is_some()
                || !self.edits.is_empty()
                || self.encoded.is_some()
                || !self.input.is_empty(),
        )
    }
    pub(super) fn receive_relay(&mut self, ctx: &egui::Context, value: &super::relay::Prepared) {
        self.source = Some(value.image.clone());
        self.origins = value.origins.clone();
        self.input.clear();
        self.output.clear();
        self.edits.clear();
        self.format = Format::Png;
        self.drag_start = None;
        self.source_texture = Some(ctx.load_texture(
            "editor-source",
            value.thumbnail.clone(),
            egui::TextureOptions::LINEAR,
        ));
        self.invalidate();
        self.message = "已接收内存图片；编辑后生成预览并选择路径另存。".into();
        self.error = false;
    }

    #[cfg(feature = "ui-preview")]
    pub(super) fn verify_relay_source(&self, source: &Arc<DynamicImage>, steps: usize) {
        assert!(Arc::ptr_eq(self.source.as_ref().unwrap(), source));
        assert_eq!(self.origins.len(), steps);
        assert!(self.edits.is_empty() && self.encoded.is_none());
        assert!(self.input.is_empty() && self.output.is_empty());
        assert_eq!(self.format, Format::Png);
    }

    #[cfg(feature = "ui-preview")]
    pub(super) fn preview_fixture(&mut self, ctx: &egui::Context) {
        let image = DynamicImage::ImageRgba8(image::ImageBuffer::from_fn(960, 540, |x, y| {
            image::Rgba([((x / 5) + 35) as u8, ((y / 3) + 35) as u8, 150, 255])
        }));
        self.input = "C:\\Users\\demo\\Pictures\\sample.png".into();
        self.output = "C:\\Users\\demo\\Pictures\\sample-annotated.png".into();
        self.source_texture = Some(ctx.load_texture(
            "editor-source",
            preview_image(&image),
            egui::TextureOptions::LINEAR,
        ));
        self.source = Some(Arc::new(image));
        self.edits = vec![
            Edit::Crop(Rect {
                a: Point::new(0.06, 0.08),
                b: Point::new(0.94, 0.92),
            }),
            Edit::Redact(Rect {
                a: Point::new(0.12, 0.23),
                b: Point::new(0.38, 0.37),
            }),
            Edit::Arrow {
                a: Point::new(0.46, 0.68),
                b: Point::new(0.78, 0.32),
                color: self.color,
                width: 8,
            },
            Edit::Text {
                at: Point::new(0.45, 0.20),
                value: "重点说明".into(),
                color: [255, 255, 255, 255],
                size: 42,
            },
        ];
        self.message = "合成界面预览：裁剪、遮挡、箭头和中文文字已放置。".into();
    }

    fn invalidate(&mut self) {
        self.encoded = None;
        self.output_texture = None;
    }

    pub(super) fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub(super) fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.pending else { return };
        let result = match rx.try_recv() {
            Ok(value) => value,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(80));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => Err("图片编辑线程意外结束".into()),
        };
        self.pending = None;
        match result {
            Ok(Job::Loaded { image, preview }) => {
                self.source_texture =
                    Some(ctx.load_texture("editor-source", preview, egui::TextureOptions::LINEAR));
                self.output = suggested_output(Path::new(&self.input), self.format)
                    .to_string_lossy()
                    .into_owned();
                self.source = Some(image);
                self.origins.clear();
                self.edits.clear();
                self.invalidate();
                self.message = "图片已读取。选择工具，在画布上拖动或点击。".into();
                self.error = false;
            }
            Ok(Job::Rendered {
                encoded,
                preview,
                width,
                height,
            }) => {
                self.message = format!(
                    "输出预览已生成：{width} × {height} · {:.2} MB；确认画面后另存。",
                    encoded.len() as f64 / 1_000_000.0
                );
                self.encoded = Some(Arc::new(encoded));
                self.output_texture =
                    Some(ctx.load_texture("editor-output", preview, egui::TextureOptions::LINEAR));
                self.error = false;
            }
            Err(error) => {
                self.message = error;
                self.error = true;
            }
        }
    }

    fn load(&mut self) {
        if self.pending.is_some() {
            return;
        }
        let path = PathBuf::from(self.input.trim());
        self.source = None;
        self.source_texture = None;
        self.edits.clear();
        self.invalidate();
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        self.message = "正在读取原图…".into();
        std::thread::spawn(move || {
            let result = metadata::oriented_static_image(&path)
                .map(|image| {
                    let preview = preview_image(&image);
                    Job::Loaded { image, preview }
                })
                .map_err(|e| format!("读取失败：{e:#}"));
            let _ = tx.send(result);
        });
    }

    fn render(&mut self) {
        if self.pending.is_some() {
            return;
        }
        let Some(source) = self.source.clone() else {
            return;
        };
        let edits = self.edits.clone();
        let format = self.format;
        let quality = self.jpeg_quality;
        let font_path = self.font_path.clone();
        self.invalidate();
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        self.message = "正在按原图分辨率生成输出预览…".into();
        std::thread::spawn(move || {
            let result = render_output(&source, &edits, format, quality, &font_path)
                .map_err(|e| format!("导出预览失败：{e:#}"));
            let _ = tx.send(result);
        });
    }

    fn save(&mut self) {
        let Some(bytes) = &self.encoded else { return };
        let target = Path::new(self.output.trim());
        let source = Path::new(self.input.trim());
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
            super::save_image_new(target, bytes, &std::sync::atomic::AtomicBool::new(false))?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.message = format!("已另存标注图片：{}", target.display());
                self.error = false;
            }
            Err(error) => {
                self.message = format!("保存失败：{error:#}");
                self.error = true;
            }
        }
    }

    fn on_drag(&mut self, a: Point, b: Point) {
        let pixels = self
            .source
            .as_ref()
            .map(|s| s.dimensions())
            .unwrap_or((1, 1));
        let rect = Rect { a, b };
        let (x0, y0, x1, y1) = rect.pixel_bounds(pixels.0, pixels.1);
        let too_small = if self.tool == Tool::Arrow {
            let dx = (b.x - a.x) * pixels.0 as f32;
            let dy = (b.y - a.y) * pixels.1 as f32;
            (dx * dx + dy * dy).sqrt() < 8.0
        } else {
            (x1 - x0) < 4 || (y1 - y0) < 4
        };
        if too_small {
            self.message = "拖动范围太小，请重新选择。".into();
            return;
        }
        let edit = match self.tool {
            Tool::Crop => Edit::Crop(rect),
            Tool::Redact => Edit::Redact(rect),
            Tool::Arrow => Edit::Arrow {
                a,
                b,
                color: self.color,
                width: self.stroke_width,
            },
            Tool::Text => return,
        };
        self.edits.push(edit);
        self.invalidate();
        self.message = format!(
            "已添加操作；当前 {} 步。生成输出预览后再保存。",
            self.edits.len()
        );
    }

    fn on_click(&mut self, at: Point) {
        if self.tool != Tool::Text {
            return;
        }
        let value = self.annotation_text.trim();
        if value.is_empty() || value.chars().count() > 80 {
            self.message = "文字需为 1–80 个字符。".into();
            self.error = true;
            return;
        }
        self.edits.push(Edit::Text {
            at,
            value: value.into(),
            color: self.color,
            size: self.font_size,
        });
        self.invalidate();
        self.error = false;
        self.message = "文字已放置；导出时会检查本机字体是否包含所有字符。".into();
    }

    pub(super) fn ui(&mut self, ui: &mut egui::Ui) {
        self.poll(ui.ctx());
        ui.heading("图片裁剪与标注");
        ui.label("在缩放画布上编辑；输出按原图像素渲染。先看真实编码预览，再另存新文件。");
        let busy = self.pending.is_some();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !busy,
                    egui::TextEdit::singleline(&mut self.input)
                        .hint_text("原图路径")
                        .desired_width((ui.available_width() - 180.0).max(180.0)),
                )
                .changed()
            {
                self.source = None;
                self.source_texture = None;
                self.edits.clear();
                self.invalidate();
            }
            if ui
                .add_enabled(!busy, egui::Button::new("选择图片…"))
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("图片", &["png", "jpg", "jpeg", "webp"])
                    .pick_file()
            {
                self.input = path.to_string_lossy().into_owned();
                self.load();
            }
            if ui.add_enabled(!busy, egui::Button::new("读取")).clicked() {
                self.load();
            }
        });
        if self.source.is_none() {
            ui.label(&self.message);
            return;
        }
        let source = self.source.as_ref().unwrap();
        let source_width = source.width();
        let source_height = source.height();
        ui.small(format!(
            "原图：{} × {} · {} 步编辑",
            source_width,
            source_height,
            self.edits.len()
        ));
        ui.horizontal_wrapped(|ui| {
            for (tool, label) in [
                (Tool::Crop, "裁剪"),
                (Tool::Redact, "黑色遮挡"),
                (Tool::Arrow, "箭头"),
                (Tool::Text, "文字"),
            ] {
                ui.selectable_value(&mut self.tool, tool, label);
            }
            if ui
                .add_enabled(
                    !busy && !self.edits.is_empty(),
                    egui::Button::new("撤销上一步"),
                )
                .clicked()
            {
                self.edits.pop();
                self.invalidate();
            }
            if ui
                .add_enabled(
                    !busy && !self.edits.is_empty(),
                    egui::Button::new("清空操作"),
                )
                .clicked()
            {
                self.edits.clear();
                self.invalidate();
            }
        });
        if matches!(self.tool, Tool::Arrow | Tool::Text) {
            ui.horizontal(|ui| {
                ui.label("颜色");
                for (name, color) in [
                    ("红", [244, 68, 68, 255]),
                    ("黄", [255, 213, 64, 255]),
                    ("白", [255, 255, 255, 255]),
                    ("黑", [0, 0, 0, 255]),
                ] {
                    if ui.selectable_label(self.color == color, name).clicked() {
                        self.color = color;
                    }
                }
                if self.tool == Tool::Arrow {
                    ui.add(egui::Slider::new(&mut self.stroke_width, 2..=24).text("线宽"));
                } else {
                    ui.add(egui::Slider::new(&mut self.font_size, 16..=100).text("字号"));
                }
            });
        }
        if self.tool == Tool::Text {
            ui.horizontal(|ui| {
                ui.label("文字");
                ui.add(egui::TextEdit::singleline(&mut self.annotation_text).desired_width(300.0));
                ui.small("填写后在画布点击放置，最多 80 字符");
            });
        }
        let Some(texture) = &self.source_texture else {
            return;
        };
        let dimensions = texture.size_vec2();
        let draw_size = dimensions * (720.0 / dimensions.x).min(440.0 / dimensions.y).min(1.0);
        let sense = if busy {
            Sense::hover()
        } else {
            Sense::click_and_drag()
        };
        let (rect, response) = ui.allocate_exact_size(draw_size, sense);
        ui.painter().image(
            texture.id(),
            rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        paint_edits(ui.painter(), rect, source_width, &self.edits);
        if !busy && let Some(at) = response.interact_pointer_pos() {
            let p = Point::new(
                (at.x - rect.left()) / rect.width(),
                (at.y - rect.top()) / rect.height(),
            );
            if self.tool == Tool::Text && response.clicked() {
                self.on_click(p);
            }
            if self.tool != Tool::Text {
                if response.drag_started() {
                    self.drag_start = Some(p);
                }
                if let Some(start) = self.drag_start {
                    if response.dragged() {
                        paint_drag(ui.painter(), rect, start, p, self.tool);
                    }
                    if response.drag_stopped() {
                        self.drag_start = None;
                        self.on_drag(start, p);
                    }
                }
            }
        }
        ui.horizontal(|ui| {
            ui.label("输出格式");
            egui::ComboBox::from_id_salt("editor-format")
                .selected_text(self.format.label())
                .show_ui(ui, |ui| {
                    for format in Format::ALL {
                        if ui
                            .selectable_value(&mut self.format, format, format.label())
                            .changed()
                        {
                            if !self.input.trim().is_empty() {
                                self.output = suggested_output(Path::new(&self.input), format)
                                    .to_string_lossy()
                                    .into_owned();
                            } else {
                                self.output.clear();
                            }
                            self.invalidate();
                        }
                    }
                });
            if self.format == Format::Jpeg
                && ui
                    .add(egui::Slider::new(&mut self.jpeg_quality, 35..=95).text("JPEG 质量"))
                    .changed()
            {
                self.invalidate();
            }
            if ui
                .add_enabled(!busy, egui::Button::new("生成输出预览"))
                .clicked()
            {
                self.render();
            }
        });
        if self
            .edits
            .iter()
            .any(|edit| matches!(edit, Edit::Text { .. }))
        {
            ui.horizontal(|ui| {
                ui.label("导出字体");
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.font_path)
                            .hint_text("自动选择 Windows 字体")
                            .desired_width(410.0),
                    )
                    .changed()
                {
                    self.invalidate();
                }
                if ui.button("选择字体…").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("字体", &["ttf", "ttc", "otf"])
                        .pick_file()
                {
                    self.font_path = path.to_string_lossy().into_owned();
                    self.invalidate();
                }
            });
        }
        if let Some(texture) = &self.output_texture {
            ui.small("最终编码输出预览");
            let size = texture.size_vec2();
            ui.image((
                texture.id(),
                size * (450.0 / size.x).min(250.0 / size.y).min(1.0),
            ));
        }
        ui.horizontal(|ui| {
            ui.label("另存路径");
            ui.add_enabled(
                self.encoded.is_some() && !busy,
                egui::TextEdit::singleline(&mut self.output).desired_width(500.0),
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
        });
        ui.label(egui::RichText::new(&self.message).color(if self.error {
            Color32::from_rgb(220, 70, 75)
        } else {
            ui.visuals().text_color()
        }));
        ui.small("黑色遮挡会写入输出像素；原图不修改。输入 ≤32 MiB/1600 万像素，输出 ≤128 MiB；拒绝覆盖已有文件。文字导出需本机字体支持字符。");
    }
}

fn suggested_output(source: &Path, format: Format) -> PathBuf {
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    source.with_file_name(format!("{stem}-annotated.{}", format.extension()))
}
fn screen_point(rect: egui::Rect, p: Point) -> egui::Pos2 {
    egui::pos2(
        rect.left() + p.x * rect.width(),
        rect.top() + p.y * rect.height(),
    )
}
fn paint_drag(painter: &egui::Painter, rect: egui::Rect, a: Point, b: Point, tool: Tool) {
    let a = screen_point(rect, a);
    let b = screen_point(rect, b);
    if tool == Tool::Arrow {
        painter.line_segment([a, b], Stroke::new(2.0, Color32::LIGHT_BLUE));
    } else {
        painter.rect_stroke(
            egui::Rect::from_two_pos(a, b),
            0.0,
            Stroke::new(2.0, Color32::LIGHT_BLUE),
            egui::StrokeKind::Inside,
        );
    }
}
fn paint_edits(painter: &egui::Painter, rect: egui::Rect, source_width: u32, edits: &[Edit]) {
    if let Some(crop) = edits.iter().rev().find_map(|e| {
        if let Edit::Crop(r) = e {
            Some(*r)
        } else {
            None
        }
    }) {
        let box_rect =
            egui::Rect::from_two_pos(screen_point(rect, crop.a), screen_point(rect, crop.b));
        painter.rect_stroke(
            box_rect,
            0.0,
            Stroke::new(2.0, Color32::LIGHT_BLUE),
            egui::StrokeKind::Inside,
        );
    }
    for edit in edits {
        match edit {
            Edit::Crop(_) => {}
            Edit::Redact(r) => {
                painter.rect_filled(
                    egui::Rect::from_two_pos(screen_point(rect, r.a), screen_point(rect, r.b)),
                    0.0,
                    Color32::BLACK,
                );
            }
            Edit::Arrow { a, b, color, width } => {
                let a = screen_point(rect, *a);
                let b = screen_point(rect, *b);
                let c = Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]);
                let w = (*width as f32 * rect.width() / source_width as f32).max(1.0);
                painter.arrow(a, b - a, Stroke::new(w, c));
            }
            Edit::Text {
                at,
                value,
                color,
                size,
            } => {
                let c = Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]);
                painter.text(
                    screen_point(rect, *at),
                    egui::Align2::LEFT_TOP,
                    value,
                    egui::FontId::proportional(
                        (*size as f32 * rect.width() / source_width as f32).max(8.0),
                    ),
                    c,
                );
            }
        }
    }
}

fn render_output(
    source: &DynamicImage,
    edits: &[Edit],
    format: Format,
    quality: u8,
    font_path: &str,
) -> Result<Job> {
    let (full_width, full_height) = source.dimensions();
    let crop = edits.iter().rev().find_map(|e| {
        if let Edit::Crop(r) = e {
            Some(*r)
        } else {
            None
        }
    });
    let (cx0, cy0, cx1, cy1) = crop
        .map(|r| r.pixel_bounds(full_width, full_height))
        .unwrap_or((0, 0, full_width, full_height));
    ensure!(cx1 > cx0 && cy1 > cy0, "裁剪区域不能为空");
    let mut pixels =
        image::imageops::crop_imm(&source.to_rgba8(), cx0, cy0, cx1 - cx0, cy1 - cy0).to_image();
    let font = if edits.iter().any(|e| matches!(e, Edit::Text { .. })) {
        Some(load_font(font_path)?)
    } else {
        None
    };
    for edit in edits {
        match edit {
            Edit::Crop(_) => {}
            Edit::Redact(rect) => {
                let (x0, y0, x1, y1) = rect.pixel_bounds(full_width, full_height);
                let x0 = x0.max(cx0);
                let y0 = y0.max(cy0);
                let x1 = x1.min(cx1);
                let y1 = y1.min(cy1);
                for y in y0..y1 {
                    for x in x0..x1 {
                        pixels.put_pixel(x - cx0, y - cy0, Rgba([0, 0, 0, 255]));
                    }
                }
            }
            Edit::Arrow { a, b, color, width } => {
                let a = (
                    (a.x * full_width as f32 - cx0 as f32),
                    (a.y * full_height as f32 - cy0 as f32),
                );
                let b = (
                    (b.x * full_width as f32 - cx0 as f32),
                    (b.y * full_height as f32 - cy0 as f32),
                );
                paint_arrow(&mut pixels, a, b, *color, *width);
            }
            Edit::Text {
                at,
                value,
                color,
                size,
            } => {
                let at = (
                    (at.x * full_width as f32 - cx0 as f32),
                    (at.y * full_height as f32 - cy0 as f32),
                );
                paint_text(
                    &mut pixels,
                    at,
                    value,
                    *color,
                    *size,
                    font.as_ref().unwrap(),
                )?;
            }
        }
    }
    let rendered = DynamicImage::ImageRgba8(pixels);
    let encoded = if format == Format::Jpeg {
        super::encoding::encode(
            &flatten_on_white(&rendered),
            format.image_format(),
            quality,
            MAX_OUTPUT_BYTES,
        )?
    } else {
        super::encoding::encode(&rendered, format.image_format(), quality, MAX_OUTPUT_BYTES)?
    };
    let decoded = image::load_from_memory_with_format(&encoded, format.image_format())?;
    Ok(Job::Rendered {
        encoded,
        preview: preview_image(&decoded),
        width: decoded.width(),
        height: decoded.height(),
    })
}

fn flatten_on_white(image: &DynamicImage) -> DynamicImage {
    let mut rgb = image::RgbImage::new(image.width(), image.height());
    for (x, y, p) in image.to_rgba8().enumerate_pixels() {
        let a = u16::from(p[3]);
        let channel = |v: u8| ((u16::from(v) * a + 255 * (255 - a) + 127) / 255) as u8;
        rgb.put_pixel(
            x,
            y,
            image::Rgb([channel(p[0]), channel(p[1]), channel(p[2])]),
        );
    }
    DynamicImage::ImageRgb8(rgb)
}

fn blend_pixel(image: &mut RgbaImage, x: i32, y: i32, color: [u8; 4], coverage: f32) {
    if x < 0 || y < 0 || x >= image.width() as i32 || y >= image.height() as i32 {
        return;
    }
    let pixel = image.get_pixel_mut(x as u32, y as u32);
    let alpha = (coverage.clamp(0.0, 1.0) * color[3] as f32 / 255.0).clamp(0.0, 1.0);
    let base_alpha = pixel[3] as f32 / 255.0;
    let out_alpha = alpha + base_alpha * (1.0 - alpha);
    if out_alpha <= 0.0 {
        return;
    }
    for channel in 0..3 {
        pixel[channel] = ((color[channel] as f32 * alpha
            + pixel[channel] as f32 * base_alpha * (1.0 - alpha))
            / out_alpha)
            .round() as u8;
    }
    pixel[3] = (out_alpha * 255.0).round() as u8;
}
fn paint_disc(image: &mut RgbaImage, x: f32, y: f32, radius: f32, color: [u8; 4]) {
    let r = radius.max(1.0);
    let left = (x - r).floor() as i32;
    let right = (x + r).ceil() as i32;
    let top = (y - r).floor() as i32;
    let bottom = (y + r).ceil() as i32;
    for py in top..=bottom {
        for px in left..=right {
            let d = ((px as f32 - x).powi(2) + (py as f32 - y).powi(2)).sqrt();
            if d <= r {
                blend_pixel(image, px, py, color, (r - d).clamp(0.0, 1.0));
            }
        }
    }
}
fn paint_line(image: &mut RgbaImage, a: (f32, f32), b: (f32, f32), color: [u8; 4], width: u32) {
    let steps = ((b.0 - a.0).abs().max((b.1 - a.1).abs()) * 2.0)
        .ceil()
        .max(1.0) as usize;
    for index in 0..=steps {
        let t = index as f32 / steps as f32;
        paint_disc(
            image,
            a.0 + (b.0 - a.0) * t,
            a.1 + (b.1 - a.1) * t,
            width as f32 / 2.0,
            color,
        );
    }
}
fn paint_arrow(image: &mut RgbaImage, a: (f32, f32), b: (f32, f32), color: [u8; 4], width: u32) {
    paint_line(image, a, b, color, width);
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return;
    }
    let angle = dy.atan2(dx);
    let head = (width * 4).max(14) as f32;
    for sign in [-1.0f32, 1.0] {
        let theta = angle + std::f32::consts::PI + sign * 0.55;
        paint_line(
            image,
            b,
            (b.0 + head * theta.cos(), b.1 + head * theta.sin()),
            color,
            width,
        );
    }
}

fn load_font(custom: &str) -> Result<FontVec> {
    let candidates: Vec<PathBuf> = if custom.trim().is_empty() {
        ["msyh.ttc", "simsun.ttc", "segoeui.ttf"]
            .iter()
            .map(|name| Path::new("C:\\Windows\\Fonts").join(name))
            .collect()
    } else {
        vec![PathBuf::from(custom.trim())]
    };
    for path in candidates {
        if !fs::metadata(&path).is_ok_and(|meta| meta.is_file() && meta.len() <= 32 * 1024 * 1024) {
            continue;
        }
        if let Ok(bytes) = fs::read(&path) {
            if let Ok(font) = FontVec::try_from_vec_and_index(bytes, 0) {
                return Ok(font);
            }
        }
    }
    bail!("找不到可用字体；请选择本机 TTF/TTC/OTF 字体文件")
}
fn paint_text(
    image: &mut RgbaImage,
    at: (f32, f32),
    value: &str,
    color: [u8; 4],
    size: u32,
    font: &FontVec,
) -> Result<()> {
    ensure!(
        !value.is_empty() && value.chars().count() <= 80,
        "文字需为 1–80 字符"
    );
    let scale = font.as_scaled(PxScale::from(size as f32));
    let baseline = at.1 + scale.ascent();
    let mut caret = at.0;
    let mut previous = None;
    for ch in value.chars() {
        let id = font.glyph_id(ch);
        ensure!(
            id.0 != 0 || ch.is_whitespace(),
            "所选字体缺少字符：{ch}；请选择其他字体"
        );
        if let Some(prev) = previous {
            caret += scale.kern(prev, id);
        }
        if let Some(outline) =
            font.outline_glyph(id.with_scale_and_position(size as f32, point(caret, baseline)))
        {
            let bounds = outline.px_bounds();
            outline.draw(|x, y, coverage| {
                blend_pixel(
                    image,
                    bounds.min.x as i32 + x as i32,
                    bounds.min.y as i32 + y as i32,
                    color,
                    coverage,
                )
            });
        }
        caret += scale.h_advance(id);
        previous = Some(id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageFormat;
    use std::io::Cursor;
    #[test]
    fn relay_refuses_unrendered_edits_and_invalidates_old_result() {
        let image = Arc::new(DynamicImage::new_rgba8(12, 8));
        let mut state = State {
            source: Some(image),
            ..Default::default()
        };
        assert!(state.relay_source().is_some());
        state.edits.push(Edit::Redact(Rect {
            a: Point::new(0.1, 0.1),
            b: Point::new(0.5, 0.5),
        }));
        assert!(state.relay_source().is_none());
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::new_rgba8(4, 3)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        state.encoded = Some(Arc::new(bytes.into_inner()));
        assert!(matches!(
            state.relay_source().unwrap().0,
            super::super::relay::Source::Encoded(_)
        ));
        state.invalidate();
        assert!(state.relay_source().is_none());
    }
    #[test]
    fn crop_redact_arrow_and_output_keep_original_unchanged() {
        let image =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(100, 80, Rgba([200, 200, 200, 255])));
        let edits = vec![
            Edit::Crop(Rect {
                a: Point::new(0.1, 0.1),
                b: Point::new(0.9, 0.9),
            }),
            Edit::Redact(Rect {
                a: Point::new(0.2, 0.2),
                b: Point::new(0.4, 0.4),
            }),
            Edit::Arrow {
                a: Point::new(0.5, 0.7),
                b: Point::new(0.8, 0.3),
                color: [255, 0, 0, 255],
                width: 4,
            },
        ];
        let Job::Rendered {
            encoded,
            width,
            height,
            ..
        } = render_output(&image, &edits, Format::Png, 85, "").unwrap()
        else {
            panic!("expected render")
        };
        assert_eq!((width, height), (80, 64));
        let output = image::load_from_memory_with_format(&encoded, ImageFormat::Png)
            .unwrap()
            .to_rgba8();
        assert_eq!(output.get_pixel(20, 15).0, [0, 0, 0, 255]);
        assert_eq!(image.to_rgba8().get_pixel(30, 25).0, [200, 200, 200, 255]);
        assert!(output.pixels().any(|p| p[0] > p[1]));
    }
    #[test]
    fn text_renders_and_missing_glyph_is_explicit() {
        let font = load_font("").unwrap();
        let mut image = RgbaImage::from_pixel(200, 80, Rgba([0, 0, 0, 255]));
        paint_text(
            &mut image,
            (10.0, 10.0),
            "Hello",
            [255, 255, 255, 255],
            32,
            &font,
        )
        .unwrap();
        assert!(image.pixels().any(|p| p[0] > 0));
        if Path::new("C:\\Windows\\Fonts\\msyh.ttc").exists() {
            let mut chinese = RgbaImage::from_pixel(200, 80, Rgba([0, 0, 0, 255]));
            paint_text(
                &mut chinese,
                (10.0, 10.0),
                "重点说明",
                [255, 255, 255, 255],
                32,
                &font,
            )
            .unwrap();
            assert!(chinese.pixels().any(|p| p[0] > 0));
        }
        assert!(
            paint_text(
                &mut image,
                (10.0, 10.0),
                "\u{10ffff}",
                [255, 255, 255, 255],
                32,
                &font
            )
            .is_err()
        );
    }
    #[test]
    fn no_overwrite_after_preview_and_changed_edits_invalidate() {
        let root = std::env::temp_dir().join(format!("zi-editor-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let input = root.join("input.png");
        let output = root.join("input-annotated.png");
        DynamicImage::new_rgba8(20, 20)
            .save_with_format(&input, ImageFormat::Png)
            .unwrap();
        let original = fs::read(&input).unwrap();
        let Job::Rendered { encoded, .. } =
            render_output(&image::open(&input).unwrap(), &[], Format::Png, 85, "").unwrap()
        else {
            panic!("expected render")
        };
        let mut state = State {
            input: input.to_string_lossy().into_owned(),
            output: output.to_string_lossy().into_owned(),
            encoded: Some(Arc::new(encoded)),
            ..Default::default()
        };
        state.save();
        assert!(!state.error);
        let saved = fs::read(&output).unwrap();
        state.save();
        assert!(state.error);
        assert_eq!(fs::read(&output).unwrap(), saved);
        assert_eq!(fs::read(&input).unwrap(), original);
        state.invalidate();
        assert!(state.encoded.is_none());
        fs::remove_dir_all(root).unwrap();
    }
}
