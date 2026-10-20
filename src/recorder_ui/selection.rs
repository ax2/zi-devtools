use super::{DisplayInfo, RecorderState, Region};
use eframe::egui::{self, Color32, Sense, Stroke, StrokeKind};

fn pixel_bounds(
    a: egui::Pos2,
    b: egui::Pos2,
    rect: egui::Rect,
    display: &DisplayInfo,
) -> Option<Region> {
    Region::selection_bounds(
        (a.x - rect.left(), a.y - rect.top()),
        (b.x - rect.left(), b.y - rect.top()),
        (display.width, display.height),
        (rect.width(), rect.height()),
    )
}

fn paint_hint(ui: &egui::Ui, rect: egui::Rect, text: &str, error: bool) -> egui::Rect {
    let color = if error {
        Color32::from_rgb(255, 145, 135)
    } else {
        Color32::WHITE
    };
    let galley = ui.painter().layout(
        text.to_owned(),
        egui::FontId::proportional(if rect.width() < 480.0 { 16.0 } else { 20.0 }),
        color,
        (rect.width() - 56.0).max(1.0),
    );
    let size = galley.size() + egui::vec2(24.0, 20.0);
    let top = rect.top() + 24.0_f32.min((rect.height() - size.y).max(0.0));
    let background =
        egui::Rect::from_min_size(egui::pos2(rect.center().x - size.x / 2.0, top), size);
    ui.painter().rect_filled(
        background,
        8.0,
        Color32::from_rgba_unmultiplied(12, 18, 28, 210),
    );
    ui.painter()
        .galley(background.min + egui::vec2(12.0, 10.0), galley, color);
    background
}

impl RecorderState {
    // Shared by the real selection viewport and the synthetic native UI fixture.
    // Display topology validation happens only when the returned crop is committed.
    pub(super) fn selection_canvas(
        &mut self,
        ui: &mut egui::Ui,
        display: &DisplayInfo,
    ) -> Option<Region> {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        if let Some(texture) = &self.selection_texture {
            ui.painter().image(
                texture.id(),
                rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        ui.painter()
            .rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(9, 17, 29, 58));
        let pointer = response.interact_pointer_pos();
        if response.drag_started() {
            // egui reports a drag after its movement threshold; retain the actual press point.
            self.drag_start = ui.input(|input| input.pointer.press_origin()).or(pointer);
            self.error = false;
            self.status = "正在选择录制区域…".into();
        }
        let crop = self
            .drag_start
            .zip(pointer)
            .and_then(|(a, b)| pixel_bounds(a, b, rect, display));
        if let (Some(a), Some(b)) = (self.drag_start, pointer) {
            let selection = egui::Rect::from_two_pos(a, b).intersect(rect);
            if let Some(texture) = &self.selection_texture {
                ui.painter().with_clip_rect(selection).image(
                    texture.id(),
                    rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            ui.painter().rect_filled(
                selection,
                2.0,
                Color32::from_rgba_unmultiplied(80, 160, 245, 18),
            );
            ui.painter().rect_stroke(
                selection,
                2.0,
                Stroke::new(2.0, Color32::from_rgb(120, 195, 255)),
                StrokeKind::Inside,
            );
            if let Some(region) = crop {
                let label = format!("{} × {} px", region.width, region.height);
                let galley = ui.painter().layout_no_wrap(
                    label,
                    egui::FontId::proportional(16.0),
                    Color32::WHITE,
                );
                let size = galley.size() + egui::vec2(16.0, 10.0);
                let label_rect = egui::Rect::from_min_size(
                    egui::pos2(
                        (selection.right() - size.x - 8.0).max(rect.left()),
                        (selection.bottom() - size.y - 8.0).max(rect.top()),
                    ),
                    size,
                );
                ui.painter().rect_filled(
                    label_rect,
                    4.0,
                    Color32::from_rgba_unmultiplied(12, 18, 28, 225),
                );
                ui.painter().galley(
                    label_rect.min + egui::vec2(8.0, 5.0),
                    galley,
                    Color32::WHITE,
                );
            }
        }
        let accepted = if response.drag_stopped() {
            self.drag_start = None;
            match crop.filter(|region| region.fits(display.width, display.height)) {
                Some(region) => Some(region),
                None => {
                    self.status = "最小选区：32 × 32 px\n请重新拖选 · Esc 取消".into();
                    self.error = true;
                    None
                }
            }
        } else {
            None
        };
        let instruction = if self.error {
            &self.status
        } else {
            "按住鼠标拖出录制区域  ·  Esc 取消"
        };
        paint_hint(ui, rect, instruction, self.error);
        accepted
    }

    pub(super) fn complete_selection(&mut self, region: Region, validation: anyhow::Result<()>) {
        self.selecting = false;
        self.selection_texture = None;
        self.drag_start = None;
        self.finish_selection_snapshot();
        match validation {
            Ok(()) => {
                self.region = Some(region);
                self.status = "区域已选好，点击“开始录制”".into();
                self.error = false;
            }
            Err(error) => {
                self.status =
                    format!("显示器状态已变化，区域未保存；请刷新屏幕列表后重新选择：{error:#}");
                self.error = true;
            }
        }
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_selection_canvas(
        &mut self,
        ui: &mut egui::Ui,
        display: &DisplayInfo,
    ) -> (Option<Region>, bool) {
        if self.selection_texture.is_none() {
            self.preview_begin_selection(ui.ctx());
            self.drag_start = None;
        }
        (self.selection_canvas(ui, display), self.error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display() -> DisplayInfo {
        DisplayInfo {
            device_name: "synthetic".into(),
            name: "Synthetic".into(),
            x: -1920,
            y: -120,
            width: 1920,
            height: 1080,
            primary: false,
        }
    }

    #[test]
    fn egui_gesture_keeps_press_origin_rejects_small_drag_then_retries() {
        let ctx = egui::Context::default();
        let mut state = RecorderState::default();
        let mut tick = 0;
        let mut frame = |state: &mut RecorderState, pos: egui::Pos2, pressed: Option<bool>| {
            let mut events = vec![egui::Event::PointerMoved(pos)];
            if let Some(pressed) = pressed {
                events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(960.0, 540.0),
                )),
                time: Some(tick as f64 / 30.0),
                events,
                ..Default::default()
            };
            tick += 1;
            let mut result = None;
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        result = state.selection_canvas(ui, &display());
                    });
            });
            result
        };
        let start = egui::pos2(192.0, 162.0);
        let small = start + egui::vec2(40.0, 12.0);
        frame(&mut state, start, None);
        frame(&mut state, start, Some(true));
        frame(&mut state, small, None);
        assert!(frame(&mut state, small, Some(false)).is_none());
        assert!(state.error);
        frame(&mut state, start, None);
        frame(&mut state, start, Some(true));
        let end = egui::pos2(672.0, 432.0);
        assert!(frame(&mut state, end, None).is_none());
        assert!(!state.error);
        assert_eq!(state.drag_start, Some(start));
        assert_eq!(
            frame(&mut state, end, Some(false)),
            Some(Region {
                x: 384,
                y: 324,
                width: 960,
                height: 540
            })
        );
        assert!(state.drag_start.is_none());
    }

    #[test]
    fn preview_and_crop_agree_at_scaled_viewports_with_nonzero_origin() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let rect = egui::Rect::from_min_size(
                egui::pos2(8.0, 16.0),
                egui::vec2(1920.0 / scale, 1080.0 / scale),
            );
            let a = rect.min + egui::vec2(101.0 / scale, 121.0 / scale);
            let b = rect.min + egui::vec2(904.0 / scale, 726.0 / scale);
            let preview = pixel_bounds(a, b, rect, &display()).unwrap();
            assert_eq!(
                preview,
                Region {
                    x: 101,
                    y: 121,
                    width: 802,
                    height: 604
                }
            );
            assert_eq!(pixel_bounds(b, a, rect, &display()), Some(preview));
            assert_eq!(
                Region::from_points(
                    (a.x - rect.left(), a.y - rect.top()),
                    (b.x - rect.left(), b.y - rect.top()),
                    (1920, 1080),
                    (rect.width(), rect.height())
                ),
                Some(preview)
            );
        }
    }

    #[test]
    fn display_change_restores_window_and_preserves_previous_crop() {
        let previous = Region {
            x: 0,
            y: 0,
            width: 640,
            height: 360,
        };
        let candidate = Region {
            x: 30,
            y: 40,
            width: 320,
            height: 180,
        };
        let mut state = RecorderState::default();
        state.region = Some(previous);
        state.request_selection_snapshot();
        assert!(state.take_minimize_request());
        state.selecting = true;
        state.complete_selection(candidate, Err(anyhow::anyhow!("topology changed")));
        assert_eq!(state.region, Some(previous));
        assert!(state.error && state.status.contains("topology changed"));
        assert!(!state.status.contains("区域太小"));
        assert!(!state.selecting && !state.selection_snapshot_pending);
        assert!(state.take_restore_request());
        assert!(!state.take_restore_request());
        state.complete_selection(candidate, Ok(()));
        assert_eq!(state.region, Some(candidate));
        assert!(!state.error);
    }

    #[test]
    fn narrow_hint_wraps_inside_the_canvas() {
        let ctx = egui::Context::default();
        let mut banner = None;
        let mut canvas = None;
        let _ = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320.0, 240.0))), ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let rect = ui.available_rect_before_wrap();
                canvas = Some(rect);
                banner = Some(paint_hint(ui, rect, "Selection is too small. Drag at least 32 x 32 pixels, or press Escape to cancel.", true));
            });
        });
        assert!(canvas.unwrap().contains_rect(banner.unwrap()));
        assert!(banner.unwrap().height() > 44.0);
    }
}
