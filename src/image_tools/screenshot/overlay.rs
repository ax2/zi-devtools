use super::*;

pub(super) enum Outcome {
    Selected(Vec<[f32; 2]>),
    Cancelled(String),
}
pub(super) struct Overlay {
    pub id: u64,
    pub display: crate::recorder::DisplayInfo,
    pub source: Arc<RgbaImage>,
    pub texture: egui::TextureHandle,
    gesture: Gesture,
    freehand: bool,
    magnifier: bool,
    hint: String,
    created: std::time::Instant,
    placed: bool,
    #[cfg(feature = "ui-preview")]
    fixture_pointer: Option<Pos2>,
}
impl Overlay {
    pub fn freehand(&self) -> bool {
        self.freehand
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self, index: usize) {
        let width = self.source.width() as f32;
        let height = self.source.height() as f32;
        self.gesture.points = vec![
            [width * 0.1, height * 0.25],
            [width * 0.7, height * 0.25],
            [width * 0.7, height * 0.75],
            [width * 0.45, height * 0.75],
            [width * 0.45, height * 0.55],
            [width * 0.1, height * 0.55],
        ];
        self.fixture_pointer = Some(if index >= 2 {
            egui::pos2(2035., 1268.)
        } else {
            egui::pos2(800., 520.)
        });
    }
    pub fn title(&self) -> String {
        format!("Zi DevTools · 截图选择 · {}", self.id)
    }
    pub fn new(
        id: u64,
        display: crate::recorder::DisplayInfo,
        image: RgbaImage,
        freehand: bool,
        ctx: &egui::Context,
    ) -> Self {
        Self {
            id,
            display,
            texture: ctx.load_texture(
                "screenshot-desktop-overlay",
                egui::ColorImage::from_rgba_unmultiplied(
                    [image.width() as usize, image.height() as usize],
                    image.as_raw(),
                ),
                egui::TextureOptions::NEAREST,
            ),
            source: Arc::new(image),
            gesture: Gesture::default(),
            freehand,
            magnifier: true,
            hint: String::new(),
            created: std::time::Instant::now(),
            placed: false,
            #[cfg(feature = "ui-preview")]
            fixture_pointer: None,
        }
    }
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<Outcome> {
        let title = self.title();
        let placement = crate::recorder_ui::place_capture_overlay(&self.display, &title);
        let id = egui::ViewportId::from_hash_of(("zi-screenshot-overlay", self.id));
        self.placed |= matches!(placement, Ok(true));
        if !self.placed && self.created.elapsed() > std::time::Duration::from_secs(5) {
            ctx.send_viewport_cmd_to(id, egui::ViewportCommand::Close);
            return Some(Outcome::Cancelled(
                "选择层创建超时，已返回工作台，请重试。".into(),
            ));
        }
        let builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_fullscreen(true)
            .with_position([self.display.x as f32, self.display.y as f32])
            .with_decorations(false)
            .with_taskbar(false)
            .with_window_level(egui::WindowLevel::AlwaysOnTop);
        let mut outcome = None;
        ctx.show_viewport_immediate(id, builder, |panel, _| {
            if let Err(error) = &placement {
                outcome = Some(Outcome::Cancelled(format!("选择层定位失败：{error:#}")));
            } else if panel
                .input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape))
            {
                outcome = Some(Outcome::Cancelled("已取消截图，原工作台内容保留。".into()));
            }
            if outcome.is_some() {
                panel.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            if panel.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space)) {
                self.magnifier = !self.magnifier;
            }
            let mode = if panel.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::R)) {
                Some(false)
            } else if panel.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F)) {
                Some(true)
            } else {
                None
            };
            if let Some(mode) = mode {
                self.freehand = mode;
                self.gesture = Gesture::default();
            }
            if panel.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)) {
                self.gesture = Gesture::default();
            }
            if panel.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                let width = self.source.width() as f32;
                let height = self.source.height() as f32;
                outcome = Some(Outcome::Selected(vec![
                    [0., 0.],
                    [width, 0.],
                    [width, height],
                    [0., height],
                ]));
            }
            if outcome.is_some() {
                panel.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(Color32::BLACK))
                .show(panel, |ui| {
                    let (rect, response) =
                        ui.allocate_exact_size(ui.available_size(), Sense::drag());
                    let size = egui::vec2(self.source.width() as f32, self.source.height() as f32);
                    ui.painter().image(
                        self.texture.id(),
                        rect,
                        egui::Rect::from_min_max(Pos2::ZERO, egui::pos2(1., 1.)),
                        Color32::WHITE,
                    );
                    ui.painter()
                        .rect_filled(rect, 0., Color32::from_black_alpha(28));
                    let finished = self
                        .gesture
                        .update(ui, &response, rect, size, self.freehand);
                    #[cfg(feature="ui-preview")]
                    if std::env::args().nth(3).as_deref()==Some("screenshot-overlay-smoke") && panel.input(|input|input.events.iter().any(|event|matches!(event,egui::Event::PointerButton{..}))) {
                        eprintln!("overlay input: rect={rect:?}, pointer={:?}, started={}, finished={finished}, points={}",response.interact_pointer_pos(),response.drag_started(),self.gesture.points.len());
                    }
                    if response.drag_started() {
                        self.hint.clear();
                    }
                    let points = polygon(&self.gesture.points, self.freehand);
                    if points.len() >= 2 {
                        let path = points
                            .iter()
                            .map(|p| {
                                rect.min
                                    + egui::vec2(
                                        p[0] / size.x * rect.width(),
                                        p[1] / size.y * rect.height(),
                                    )
                            })
                            .collect();
                        ui.painter().add(egui::Shape::closed_line(
                            path,
                            Stroke::new(2., Color32::from_rgb(255, 185, 55)),
                        ));
                    }
                    if finished {
                        if self.gesture.overflow {
                            self.gesture = Gesture::default();
                            self.hint = "轮廓超过2048点，请重新拖动；Esc取消".into();
                        } else if points.len() >= 3 {
                            let bounds = points.iter().fold(egui::Rect::NOTHING, |rect, p| {
                                rect.union(egui::Rect::from_min_max(
                                    egui::pos2(p[0], p[1]),
                                    egui::pos2(p[0], p[1]),
                                ))
                            });
                            if bounds.width() >= 1. && bounds.height() >= 1. {
                                outcome = Some(Outcome::Selected(points));
                            } else {
                                self.hint = "选区太小，请重新拖动；Esc取消".into();
                                self.gesture = Gesture::default();
                            }
                        } else {
                            self.hint = "自由轮廓至少需要三个点，请重新拖动；Esc取消".into();
                            self.gesture = Gesture::default();
                        }
                    }
                    let help = egui::Rect::from_center_size(
                        rect.center_top() + egui::vec2(0., 62.),
                        egui::vec2(rect.width().min(780.) - 24., 92.),
                    );
                    ui.painter()
                        .rect_filled(help, 12., Color32::from_black_alpha(220));
                    ui.painter().text(
                        help.center_top() + egui::vec2(0., 15.),
                        egui::Align2::CENTER_TOP,
                        format!(
                            "{} · {} × {} px · {}",
                            self.display.name,
                            self.source.width(),
                            self.source.height(),
                            if self.freehand {
                                "自由轮廓"
                            } else {
                                "矩形"
                            }
                        ),
                        egui::FontId::proportional(18.),
                        Color32::WHITE,
                    );
                    ui.painter().text(
                        help.center_bottom() - egui::vec2(0., 15.),
                        egui::Align2::CENTER_BOTTOM,
                        if self.hint.is_empty() {
                            "拖动后松开截取 · R 矩形 / F 轮廓 · Enter 全屏 · 空格 放大镜 · Esc 取消"
                        } else {
                            &self.hint
                        },
                        egui::FontId::proportional(15.),
                        Color32::from_gray(230),
                    );
                    let pointer = response.hover_pos().or(response.interact_pointer_pos());
                    #[cfg(feature = "ui-preview")]
                    let pointer = self.fixture_pointer.or(pointer);
                    if self.magnifier
                        && let Some(pos) = pointer
                    {
                        self.paint_magnifier(ui, rect, pos);
                    }
                });
            if outcome.is_some() {
                panel.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(30));
        outcome
    }
    fn paint_magnifier(&self, ui: &egui::Ui, canvas: egui::Rect, pos: Pos2) {
        let (px, py) = pixel_at(canvas, pos, self.source.dimensions());
        let left = px
            .saturating_sub(12)
            .min(self.source.width().saturating_sub(24));
        let top = py
            .saturating_sub(12)
            .min(self.source.height().saturating_sub(24));
        let right = (left + 24).min(self.source.width());
        let bottom = (top + 24).min(self.source.height());
        let x = (pos.x + 28.).min(canvas.right() - 154.).max(canvas.left());
        let y = (pos.y + 28.).min(canvas.bottom() - 184.).max(canvas.top());
        let body = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(154., 184.));
        let p = ui.painter_at(canvas);
        p.rect_filled(body, 8., Color32::from_gray(18));
        let sample =
            egui::Rect::from_min_size(body.min + egui::vec2(9., 9.), egui::vec2(136., 136.));
        let uv = egui::Rect::from_min_max(
            egui::pos2(
                left as f32 / self.source.width() as f32,
                top as f32 / self.source.height() as f32,
            ),
            egui::pos2(
                right as f32 / self.source.width() as f32,
                bottom as f32 / self.source.height() as f32,
            ),
        );
        p.image(self.texture.id(), sample, uv, Color32::WHITE);
        let center = sample.min
            + egui::vec2((px - left) as f32 + 0.5, (py - top) as f32 + 0.5)
                / egui::vec2((right - left) as f32, (bottom - top) as f32)
                * sample.size();
        p.line_segment(
            [
                egui::pos2(sample.left(), center.y),
                egui::pos2(sample.right(), center.y),
            ],
            Stroke::new(1., Color32::WHITE),
        );
        p.line_segment(
            [
                egui::pos2(center.x, sample.top()),
                egui::pos2(center.x, sample.bottom()),
            ],
            Stroke::new(1., Color32::WHITE),
        );
        let c = self.source.get_pixel(px, py);
        p.text(
            body.center_bottom() - egui::vec2(0., 9.),
            egui::Align2::CENTER_BOTTOM,
            format!("{px}, {py}\n#{:02X}{:02X}{:02X}", c[0], c[1], c[2]),
            egui::FontId::monospace(13.),
            Color32::WHITE,
        );
    }
}
fn pixel_at(rect: egui::Rect, pos: Pos2, size: (u32, u32)) -> (u32, u32) {
    (
        ((pos.x - rect.left()) / rect.width() * size.0 as f32)
            .floor()
            .clamp(0., size.0.saturating_sub(1) as f32) as u32,
        ((pos.y - rect.top()) / rect.height() * size.1 as f32)
            .floor()
            .clamp(0., size.1.saturating_sub(1) as f32) as u32,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scaled_negative_origin_and_edges_map_to_physical_source_pixels() {
        let rect = egui::Rect::from_min_size(egui::pos2(-300., -100.), egui::vec2(2048., 1280.));
        assert_eq!(pixel_at(rect, rect.min, (2560, 1600)), (0, 0));
        assert_eq!(pixel_at(rect, rect.max, (2560, 1600)), (2559, 1599));
        assert_eq!(
            pixel_at(rect, rect.min + egui::vec2(800., 400.), (2560, 1600)),
            (1000, 500)
        );
        assert_eq!(
            pixel_at(rect, rect.min - egui::vec2(50., 50.), (2560, 1600)),
            (0, 0)
        );
    }
}
