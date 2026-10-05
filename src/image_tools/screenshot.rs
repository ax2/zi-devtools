//! Explicit desktop capture and bounded even-odd alpha selection.
use anyhow::{Result, ensure};
use eframe::egui::{self, Color32, Pos2, Sense, Stroke};
use image::RgbaImage;
use std::{
    io::Write,
    path::Path,
    sync::{Arc, mpsc},
};

const MAX_POINTS: usize = 2048;

#[cfg(all(windows, feature = "ui-preview"))]
pub fn verify_capture() -> Result<(u32, u32)> {
    let display = crate::recorder::primary_display()?;
    crate::recorder::validate_display(&display)?;
    let captured = crate::recorder_ui::capture_display_snapshot(&display)?;
    let bytes: Vec<u8> = captured.pixels.iter().flat_map(|p| p.to_array()).collect();
    let source = RgbaImage::from_raw(display.width, display.height, bytes)
        .ok_or_else(|| anyhow::anyhow!("捕获尺寸无效"))?;
    let output = crop(
        &source,
        &[
            [1., 1.],
            [33., 1.],
            [33., 17.],
            [17., 17.],
            [17., 33.],
            [1., 33.],
        ],
    )?;
    ensure!(
        output.get_pixel(25, 25)[3] == 0 && output.get_pixel(4, 4)[3] == 255,
        "真实捕获透明裁剪验证失败"
    );
    let mut encoded = std::io::Cursor::new(Vec::new());
    output.write_to(&mut encoded, image::ImageFormat::Png)?;
    ensure!(
        image::load_from_memory(encoded.get_ref())?.to_rgba8() == output,
        "PNG重读不一致"
    );
    Ok((display.width, display.height))
}

fn crop(source: &RgbaImage, points: &[[f32; 2]]) -> Result<RgbaImage> {
    ensure!(
        (3..=MAX_POINTS).contains(&points.len()),
        "轮廓需要至少三个点，最多2048个点"
    );
    ensure!(
        points.iter().flatten().all(|v| v.is_finite()),
        "轮廓坐标无效"
    );
    ensure!(
        source.width() > 0
            && source.height() > 0
            && u64::from(source.width()) * u64::from(source.height()) <= 16_000_000,
        "图片尺寸超限"
    );
    let mut left = source.width() as f32;
    let mut top = source.height() as f32;
    let mut right = 0f32;
    let mut bottom = 0f32;
    let points: Vec<_> = points
        .iter()
        .map(|p| {
            [
                p[0].clamp(0., source.width() as f32),
                p[1].clamp(0., source.height() as f32),
            ]
        })
        .collect();
    for p in &points {
        left = left.min(p[0]);
        right = right.max(p[0]);
        top = top.min(p[1]);
        bottom = bottom.max(p[1]);
    }
    let (left, top, right, bottom) = (
        left.floor() as u32,
        top.floor() as u32,
        right.ceil() as u32,
        bottom.ceil() as u32,
    );
    ensure!(right > left && bottom > top, "选区太小");
    let mut output = RgbaImage::new(right - left, bottom - top);
    let mut covered = false;
    // Two horizontal subpixels at two scanlines: sorted intersections implement even-odd filling.
    for y in top..bottom {
        let mut coverage = vec![0u8; (right - left) as usize];
        for dy in [0.25, 0.75] {
            let sy = y as f32 + dy;
            let mut crossings = Vec::new();
            for i in 0..points.len() {
                let (a, b) = (points[i], points[(i + 1) % points.len()]);
                if (a[1] > sy) != (b[1] > sy) {
                    crossings.push(a[0] + (sy - a[1]) * (b[0] - a[0]) / (b[1] - a[1]));
                }
            }
            crossings.sort_by(f32::total_cmp);
            for pair in crossings.chunks_exact(2) {
                for x in (pair[0].floor().max(left as f32) as u32)
                    ..(pair[1].ceil().min(right as f32) as u32)
                {
                    for dx in [0.25, 0.75] {
                        if x as f32 + dx >= pair[0] && x as f32 + dx < pair[1] {
                            coverage[(x - left) as usize] += 1;
                        }
                    }
                }
            }
        }
        for (index, count) in coverage.into_iter().enumerate() {
            if count > 0 {
                let mut pixel = *source.get_pixel(left + index as u32, y);
                pixel[3] = ((u16::from(pixel[3]) * u16::from(count) + 2) / 4) as u8;
                output.put_pixel(index as u32, y - top, pixel);
                covered = true;
            }
        }
    }
    ensure!(covered, "轮廓没有可见面积，请重新选择");
    Ok(output)
}

fn save_png(path: &Path, image: &RgbaImage) -> Result<usize> {
    let mut data = std::io::Cursor::new(Vec::new());
    image.write_to(&mut data, image::ImageFormat::Png)?;
    let data = data.into_inner();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    let result = file.write_all(&data).and_then(|_| file.sync_all());
    drop(file);
    if let Err(error) = result {
        let _ = std::fs::remove_file(path);
        return Err(error.into());
    }
    Ok(data.len())
}

enum Job {
    Source(RgbaImage),
    Output(RgbaImage),
    Saved(usize),
}
#[derive(Default)]
pub struct State {
    source: Option<Arc<RgbaImage>>,
    output: Option<Arc<RgbaImage>>,
    texture: Option<egui::TextureHandle>,
    preview: Option<egui::TextureHandle>,
    points: Vec<[f32; 2]>,
    freehand: bool,
    drawing: bool,
    overflow: bool,
    dirty: bool,
    message: String,
    pending: Option<mpsc::Receiver<Result<Job, String>>>,
    #[cfg(test)]
    canvas: Option<egui::Rect>,
}
impl State {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn has_work(&self) -> bool {
        self.busy() || (self.output.is_some() && self.dirty)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self, ctx: &egui::Context) {
        let image = RgbaImage::from_fn(800, 360, |x, y| {
            if x > 90 && x < 710 && y > 70 && y < 300 {
                image::Rgba([40, 120, 190, 255])
            } else {
                image::Rgba([235, 240, 245, 255])
            }
        });
        self.source = Some(Arc::new(image.clone()));
        self.texture = Some(texture(ctx, "screenshot-fixture-source", &image));
        self.points = vec![
            [100., 80.],
            [690., 80.],
            [690., 280.],
            [430., 280.],
            [430., 190.],
            [100., 190.],
        ];
        self.freehand = true;
        let output = crop(&image, &self.points).expect("synthetic screenshot mask");
        self.preview = Some(texture(ctx, "screenshot-fixture-alpha", &output));
        self.output = Some(Arc::new(output));
        self.dirty = true;
        self.message = "合成画面用于布局验收，非真实桌面捕获；透明 PNG 预览就绪。".into();
    }
    fn task(&mut self, ctx: &egui::Context, task: impl FnOnce() -> Result<Job> + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(task().map_err(|e| format!("{e:#}")));
            ctx.request_repaint();
        });
    }
    pub fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.pending else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }
            Err(_) => Err("后台任务异常结束".into()),
        };
        self.pending = None;
        match result {
            Ok(Job::Source(image)) => {
                self.texture = Some(texture(ctx, "screenshot-source", &image));
                self.source = Some(Arc::new(image));
                self.output = None;
                self.preview = None;
                self.points.clear();
                self.message = "桌面静态画面已捕获。拖动选择；松开闭合。Esc 清除选区。".into();
            }
            Ok(Job::Output(image)) => {
                self.dirty = true;
                self.preview = Some(texture(ctx, "screenshot-alpha", &image));
                self.output = Some(Arc::new(image));
                self.message = "透明 PNG 预览已生成，可以另存。原始画面仍保留。".into();
            }
            Ok(Job::Saved(bytes)) => {
                self.dirty = false;
                self.message = format!("已保存 PNG：{:.1} KiB", bytes as f64 / 1024.)
            }
            Err(error) => self.message = format!("操作失败：{error}"),
        }
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.poll(ui.ctx());
        ui.heading("截图与透明套索 · 开发中");
        ui.label(
            "主动捕获屏幕后，在下方真实静态画面上拖动选区；自由轮廓外透明，另存不会覆盖已有文件。",
        );
        let busy = self.pending.is_some();
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal_wrapped(|ui| {
                #[cfg(windows)]
                if ui.button("捕获主屏幕").clicked() {
                    self.message = "正在捕获当前桌面…".into();
                    self.task(ui.ctx(), || {
                        let display = crate::recorder::primary_display()?;
                        crate::recorder::validate_display(&display)?;
                        let captured = crate::recorder_ui::capture_display_snapshot(&display)?;
                        let bytes: Vec<u8> = captured
                            .pixels
                            .iter()
                            .flat_map(|pixel| pixel.to_array())
                            .collect();
                        let image = RgbaImage::from_raw(
                            captured.size[0] as u32,
                            captured.size[1] as u32,
                            bytes,
                        )
                        .ok_or_else(|| anyhow::anyhow!("捕获尺寸无效"))?;
                        Ok(Job::Source(image))
                    });
                }
                let changed = ui
                    .selectable_value(&mut self.freehand, false, "矩形")
                    .changed()
                    | ui.selectable_value(&mut self.freehand, true, "自由轮廓")
                        .changed();
                if changed {
                    self.points.clear();
                    self.output = None;
                    self.preview = None;
                    self.drawing = false;
                }
                if ui.button("重新选择").clicked() {
                    self.points.clear();
                    self.output = None;
                    self.preview = None;
                    self.drawing = false;
                }
                if let Some(output) = self.output.clone()
                    && ui.button("另存透明 PNG…").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("PNG", &["png"])
                        .set_file_name("Zi-Screenshot.png")
                        .save_file()
                {
                    self.message = "正在保存…".into();
                    self.task(ui.ctx(), move || save_png(&path, &output).map(Job::Saved));
                }
            });
        });
        ui.label(&self.message);
        let (Some(source), Some(texture)) = (self.source.clone(), self.texture.as_ref()) else {
            return;
        };
        let size = egui::vec2(source.width() as f32, source.height() as f32);
        let scale = (ui.available_width() / size.x).min(560. / size.y).min(1.);
        let response = ui.add(
            egui::Image::new(texture)
                .fit_to_exact_size(size * scale)
                .sense(Sense::drag()),
        );
        let rect = response.rect;
        #[cfg(test)]
        {
            self.canvas = Some(rect);
        }
        if !busy {
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.points.clear();
                self.output = None;
                self.preview = None;
                self.drawing = false;
            }
            if response.drag_started() {
                self.points.clear();
                self.output = None;
                self.preview = None;
                self.drawing = true;
                self.overflow = false;
                if let Some(pos) = ui.input(|input| input.pointer.press_origin()) {
                    let p = [
                        ((pos.x - rect.left()) / scale).clamp(0., size.x),
                        ((pos.y - rect.top()) / scale).clamp(0., size.y),
                    ];
                    self.points.push(p);
                    if !self.freehand {
                        self.points.push(p);
                    }
                }
            }
            if self.drawing
                && let Some(pos) = response.interact_pointer_pos()
            {
                let p = [
                    ((pos.x - rect.left()) / scale).clamp(0., size.x),
                    ((pos.y - rect.top()) / scale).clamp(0., size.y),
                ];
                if self.freehand {
                    if self
                        .points
                        .last()
                        .is_none_or(|last| (last[0] - p[0]).hypot(last[1] - p[1]) >= 2.)
                    {
                        if self.points.len() < MAX_POINTS {
                            self.points.push(p);
                        } else {
                            self.overflow = true;
                        }
                    }
                } else if self.points.is_empty() {
                    self.points.push(p);
                    self.points.push(p);
                } else {
                    self.points[1] = p;
                }
            }
            if response.drag_stopped() && self.drawing {
                self.drawing = false;
                let points = polygon(&self.points, self.freehand);
                if self.overflow {
                    self.message = "轮廓超过2048点，请缩短轮廓重新选择。".into();
                } else {
                    self.message = "正在生成透明预览…".into();
                    self.task(ui.ctx(), move || crop(&source, &points).map(Job::Output));
                }
            }
        }
        let path: Vec<Pos2> = polygon(&self.points, self.freehand)
            .iter()
            .map(|p| rect.min + egui::vec2(p[0], p[1]) * scale)
            .collect();
        if path.len() >= 2 {
            ui.painter().add(egui::Shape::closed_line(
                path,
                Stroke::new(2., Color32::from_rgb(255, 185, 55)),
            ));
        }
        if let (Some(output), Some(preview)) = (&self.output, &self.preview) {
            ui.label(format!(
                "输出 {} × {} px · 棋盘格代表透明",
                output.width(),
                output.height()
            ));
            let size = egui::vec2(output.width() as f32, output.height() as f32);
            let scale = (ui.available_width() / size.x).min(320. / size.y).min(1.);
            let (rect, _) = ui.allocate_exact_size(size * scale, Sense::hover());
            let painter = ui.painter_at(rect);
            for y in 0..(rect.height() / 12.).ceil() as usize {
                for x in 0..(rect.width() / 12.).ceil() as usize {
                    painter.rect_filled(
                        egui::Rect::from_min_size(
                            rect.min + egui::vec2(x as f32 * 12., y as f32 * 12.),
                            egui::vec2(12., 12.),
                        ),
                        0.,
                        if (x + y) % 2 == 0 {
                            Color32::from_gray(180)
                        } else {
                            Color32::from_gray(230)
                        },
                    );
                }
            }
            painter.image(
                preview.id(),
                rect,
                egui::Rect::from_min_max(Pos2::ZERO, egui::pos2(1., 1.)),
                Color32::WHITE,
            );
        }
    }
}
fn polygon(points: &[[f32; 2]], freehand: bool) -> Vec<[f32; 2]> {
    if !freehand && points.len() == 2 {
        let (a, b) = (points[0], points[1]);
        vec![a, [b[0], a[1]], b, [a[0], b[1]]]
    } else {
        points.to_vec()
    }
}
fn texture(ctx: &egui::Context, name: &str, image: &RgbaImage) -> egui::TextureHandle {
    ctx.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        ),
        egui::TextureOptions::LINEAR,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inactive_save_completion_and_failure_preserve_exit_protection() {
        let ctx = egui::Context::default();
        let output = Arc::new(RgbaImage::from_pixel(4, 4, image::Rgba([1, 2, 3, 255])));
        let mut state = State {
            output: Some(output.clone()),
            dirty: true,
            ..Default::default()
        };
        let path = std::env::temp_dir().join(format!(
            "zi-screenshot-background-{}.png",
            uuid::Uuid::new_v4()
        ));
        let target = path.clone();
        state.task(&ctx, move || save_png(&target, &output).map(Job::Saved));
        assert!(state.busy() && state.has_work());
        for _ in 0..100 {
            state.poll(&ctx);
            if !state.busy() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(!state.busy() && !state.has_work() && state.output.is_some());
        let output = state.output.clone().unwrap();
        let target = path.clone();
        state.dirty = true;
        state.task(&ctx, move || save_png(&target, &output).map(Job::Saved));
        for _ in 0..100 {
            state.poll(&ctx);
            if !state.busy() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(!state.busy() && state.has_work() && state.message.starts_with("操作失败"));
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn pointer_drag_generates_output_and_escape_clears_it() {
        let ctx = egui::Context::default();
        let image = RgbaImage::from_pixel(300, 200, image::Rgba([10, 20, 30, 255]));
        let mut state = State {
            texture: Some(texture(&ctx, "pointer-test", &image)),
            source: Some(Arc::new(image)),
            ..Default::default()
        };
        let frame = |state: &mut State, events: Vec<egui::Event>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        Pos2::ZERO,
                        egui::vec2(900., 800.),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| state.ui(ui));
                },
            );
        };
        frame(&mut state, vec![]);
        let start = state.canvas.unwrap().min + egui::vec2(10., 10.);
        let end = start + egui::vec2(80., 60.);
        frame(
            &mut state,
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        frame(&mut state, vec![egui::Event::PointerMoved(end)]);
        frame(
            &mut state,
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        for _ in 0..100 {
            frame(&mut state, vec![]);
            if state.pending.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(
            state.output.is_some(),
            "{} points {:?}",
            state.message,
            state.points
        );
        assert_eq!(state.output.as_ref().unwrap().dimensions(), (80, 60));
        frame(
            &mut state,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(state.output.is_none() && state.points.is_empty());
    }
    #[test]
    fn concave_alpha_and_png_roundtrip() {
        let image = RgbaImage::from_pixel(8, 8, image::Rgba([80, 150, 200, 255]));
        let output = crop(
            &image,
            &[[0., 0.], [8., 0.], [8., 3.], [3., 3.], [3., 8.], [0., 8.]],
        )
        .unwrap();
        assert_eq!(output.get_pixel(1, 6)[3], 255);
        assert_eq!(output.get_pixel(6, 6)[3], 0);
        let path = std::env::temp_dir().join(format!("zi-screenshot-{}.png", uuid::Uuid::new_v4()));
        save_png(&path, &output).unwrap();
        assert!(save_png(&path, &image).is_err());
        assert_eq!(image::open(&path).unwrap().to_rgba8(), output);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn clipping_self_intersections_and_antialias() {
        let image = RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255]));
        let clipped = crop(
            &image,
            &[[-20., -20.], [20., -20.], [20., 20.], [-20., 20.]],
        )
        .unwrap();
        assert_eq!(clipped, image);
        let bow = crop(&image, &[[0., 0.], [8., 8.], [0., 8.], [8., 0.]]).unwrap();
        assert_eq!(bow.get_pixel(0, 3)[3], 0);
        assert_eq!(bow.get_pixel(4, 1)[3], 255);
        assert!(bow.pixels().any(|p| p[3] > 0 && p[3] < 255));
        assert!(crop(&image, &[[f32::NAN, 0.], [1., 1.], [2., 2.]]).is_err());
        assert!(crop(&image, &[[0., 0.]; 3]).is_err());
        assert!(crop(&image, &vec![[1., 1.]; MAX_POINTS + 1]).is_err());
    }
}
