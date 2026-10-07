use super::plot::{Output, Saved, valid_range};
use eframe::egui::{self, Color32, Pos2, Rect, Stroke};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Bounds {
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
}
impl Bounds {
    pub fn fit(output: &Output, saved: &Saved) -> Self {
        let (y_min, y_max) = if saved.auto_y {
            output.y_range().unwrap_or((saved.y_min, saved.y_max))
        } else {
            (saved.y_min, saved.y_max)
        };
        let (y_min, y_max) = if valid_range(y_min, y_max) {
            (y_min, y_max)
        } else {
            (-2.0, 2.0)
        };
        Self {
            x_min: output.key.x_min,
            x_max: output.key.x_max,
            y_min,
            y_max,
        }
    }
    fn valid(self) -> bool {
        valid_range(self.x_min, self.x_max) && valid_range(self.y_min, self.y_max)
    }
    pub fn pan(self, dx: f64, dy: f64) -> Self {
        let candidate = Self {
            x_min: self.x_min + dx,
            x_max: self.x_max + dx,
            y_min: self.y_min + dy,
            y_max: self.y_max + dy,
        };
        if candidate.valid() { candidate } else { self }
    }
    pub fn zoom(self, x: f64, y: f64, factor: f64) -> Self {
        let candidate = Self {
            x_min: x + (self.x_min - x) * factor,
            x_max: x + (self.x_max - x) * factor,
            y_min: y + (self.y_min - y) * factor,
            y_max: y + (self.y_max - y) * factor,
        };
        if candidate.valid() { candidate } else { self }
    }
    pub fn at(self, rect: Rect, point: Pos2) -> (f64, f64) {
        (
            self.x_min
                + (point.x - rect.left()) as f64 / rect.width() as f64 * (self.x_max - self.x_min),
            self.y_max
                - (point.y - rect.top()) as f64 / rect.height() as f64 * (self.y_max - self.y_min),
        )
    }
    /// Clip before converting to f32 so outlying finite values cannot poison painting.
    pub fn segment(self, rect: Rect, a: (f64, f64), b: (f64, f64)) -> Option<[Pos2; 2]> {
        let normalize = |(x, y): (f64, f64)| {
            (
                (x - self.x_min) / (self.x_max - self.x_min),
                (y - self.y_min) / (self.y_max - self.y_min),
            )
        };
        let (ax, ay) = normalize(a);
        let (bx, by) = normalize(b);
        if ![ax, ay, bx, by].iter().all(|v| v.is_finite()) {
            return None;
        }
        let dx = bx - ax;
        let dy = by - ay;
        if !dx.is_finite() || !dy.is_finite() {
            return None;
        }
        let mut low: f64 = 0.0;
        let mut high: f64 = 1.0;
        for (p, q) in [(-dx, ax), (dx, 1.0 - ax), (-dy, ay), (dy, 1.0 - ay)] {
            if p == 0.0 {
                if q < 0.0 {
                    return None;
                }
            } else {
                let t = q / p;
                if p < 0.0 {
                    low = low.max(t);
                } else {
                    high = high.min(t);
                }
                if low > high {
                    return None;
                }
            }
        }
        let at = |t: f64| {
            egui::pos2(
                rect.left() + ((ax + dx * t).clamp(0.0, 1.0) as f32) * rect.width(),
                rect.bottom() - ((ay + dy * t).clamp(0.0, 1.0) as f32) * rect.height(),
            )
        };
        Some([at(low), at(high)])
    }
}
pub(super) fn color(index: usize, dark: bool) -> Color32 {
    if dark {
        [
            Color32::from_rgb(93, 174, 255),
            Color32::from_rgb(70, 218, 174),
            Color32::from_rgb(255, 183, 83),
            Color32::from_rgb(224, 148, 255),
        ][index % 4]
    } else {
        [
            Color32::from_rgb(31, 98, 193),
            Color32::from_rgb(0, 120, 91),
            Color32::from_rgb(172, 85, 0),
            Color32::from_rgb(136, 62, 190),
        ][index % 4]
    }
}
pub(super) fn chart(ui: &mut egui::Ui, output: &Output, view: &mut Bounds) -> (Rect, Rect) {
    let screen_height = ui.ctx().screen_rect().height();
    let height = if screen_height <= 800.0 {
        (160.0 + (screen_height - 640.0) * 0.25).clamp(160.0, 200.0)
    } else {
        (screen_height * 0.34).clamp(210.0, 340.0)
    };
    let (outer, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().max(240.0), height),
        egui::Sense::click_and_drag(),
    );
    let rect = Rect::from_min_max(
        outer.min + egui::vec2(66.0, 14.0),
        outer.max - egui::vec2(20.0, 32.0),
    );
    let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(outer));
    painter.rect_filled(outer, 8.0, ui.visuals().extreme_bg_color);
    if response.dragged() {
        let delta = response.drag_delta();
        *view = view.pan(
            -(delta.x as f64) / rect.width() as f64 * (view.x_max - view.x_min),
            (delta.y as f64) / rect.height() as f64 * (view.y_max - view.y_min),
        );
    }
    if let Some(pos) = response.hover_pos().filter(|p| rect.contains(*p)) {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            let (x, y) = view.at(rect, pos);
            *view = view.zoom(x, y, (-scroll as f64 * 0.004).clamp(-0.7, 0.7).exp());
            ui.input_mut(|i| {
                i.smooth_scroll_delta = egui::Vec2::ZERO;
                i.raw_scroll_delta = egui::Vec2::ZERO;
            });
        }
    }
    let grid = Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color);
    let text = ui.visuals().text_color();
    for i in 0..=5 {
        let t = i as f32 / 5.0;
        let x = rect.left() + rect.width() * t;
        let y = rect.bottom() - rect.height() * t;
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            grid,
        );
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            grid,
        );
        painter.text(
            egui::pos2(x, rect.bottom() + 7.0),
            match i {
                0 => egui::Align2::LEFT_TOP,
                5 => egui::Align2::RIGHT_TOP,
                _ => egui::Align2::CENTER_TOP,
            },
            format!("{:.3e}", view.x_min + (view.x_max - view.x_min) * t as f64),
            egui::FontId::monospace(10.0),
            text,
        );
        painter.text(
            egui::pos2(rect.left() - 7.0, y),
            egui::Align2::RIGHT_CENTER,
            format!("{:.2e}", view.y_min + (view.y_max - view.y_min) * t as f64),
            egui::FontId::monospace(10.0),
            text,
        );
    }
    let zero = Stroke::new(1.5, ui.visuals().weak_text_color());
    for (a, b) in [
        ((view.x_min, 0.0), (view.x_max, 0.0)),
        ((0.0, view.y_min), (0.0, view.y_max)),
    ] {
        if let Some(line) = view.segment(rect, a, b) {
            painter.line_segment(line, zero);
        }
    }
    for series in &output.series {
        for i in 0..output.x.len() - 1 {
            if series.connect[i]
                && let (Some(a), Some(b)) = (series.y[i], series.y[i + 1])
                && let Some(line) = view.segment(rect, (output.x[i], a), (output.x[i + 1], b))
            {
                painter.line_segment(
                    line,
                    Stroke::new(2.0, color(series.index, ui.visuals().dark_mode)),
                );
            }
        }
    }
    if let Some(pos) = response.hover_pos().filter(|p| rect.contains(*p)) {
        painter.line_segment(
            [
                egui::pos2(pos.x, rect.top()),
                egui::pos2(pos.x, rect.bottom()),
            ],
            grid,
        );
        let (x, _) = view.at(rect, pos);
        let index = output.x.partition_point(|v| *v < x).min(output.x.len() - 1);
        let index = if index > 0 && (x - output.x[index - 1]).abs() < (x - output.x[index]).abs() {
            index - 1
        } else {
            index
        };
        response.clone().on_hover_ui(|ui| {
            ui.monospace(format!("最近采样 x = {:.12e}", output.x[index]));
            for s in &output.series {
                ui.colored_label(
                    color(s.index, ui.visuals().dark_mode),
                    format!(
                        "y{} = {}",
                        s.index + 1,
                        s.y[index].map_or_else(|| "定义域无效".into(), |v| format!("{v:.12e}"))
                    ),
                );
            }
            ui.small("读数是最近采样值，非实时精确求值");
        });
    }
    (rect, ui.clip_rect())
}
fn xml(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
pub(super) fn svg(output: &Output, view: Bounds) -> String {
    let rect = Rect::from_min_max(egui::pos2(76.0, 32.0), egui::pos2(974.0, 490.0));
    let mut svg = String::from(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1000\" height=\"640\" viewBox=\"0 0 1000 640\"><rect width=\"1000\" height=\"640\" fill=\"white\"/><g font-family=\"sans-serif\" font-size=\"11\" fill=\"#273244\">",
    );
    for i in 0..=5 {
        let t = i as f64 / 5.0;
        let x = rect.left() as f64 + rect.width() as f64 * t;
        let y = rect.bottom() as f64 - rect.height() as f64 * t;
        svg.push_str(&format!("<path d=\"M {x:.3} 32 V 490 M 76 {y:.3} H 974\" stroke=\"#dce3ec\"/><text x=\"{x:.3}\" y=\"512\" text-anchor=\"middle\">{:.3e}</text><text x=\"70\" y=\"{y:.3}\" text-anchor=\"end\">{:.3e}</text>",view.x_min+(view.x_max-view.x_min)*t,view.y_min+(view.y_max-view.y_min)*t));
    }
    for s in &output.series {
        let color = color(s.index, false);
        let hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
        svg.push_str(&format!(
            "<path fill=\"none\" stroke=\"{hex}\" stroke-width=\"2\" d=\""
        ));
        for i in 0..output.x.len() - 1 {
            if s.connect[i]
                && let (Some(a), Some(b)) = (s.y[i], s.y[i + 1])
                && let Some([a, b]) = view.segment(rect, (output.x[i], a), (output.x[i + 1], b))
            {
                svg.push_str(&format!("M {:.3} {:.3} L {:.3} {:.3} ", a.x, a.y, b.x, b.y));
            }
        }
        svg.push_str(&format!(
            "\"/><text x=\"76\" y=\"{}\" fill=\"{hex}\"><title>{}</title>y{} = {}</text>",
            540 + s.index * 20,
            xml(&s.expression),
            s.index + 1,
            xml(&s.expression.chars().take(100).collect::<String>())
        ));
    }
    svg.push_str("<text x=\"76\" y=\"632\">Approximate samples; missing domain values and detected jumps are not joined.</text></g></svg>");
    svg
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipped_segments_and_bounded_view_changes() {
        let view = Bounds {
            x_min: -1.0,
            x_max: 1.0,
            y_min: -1.0,
            y_max: 1.0,
        };
        let rect = Rect::from_min_max(Pos2::ZERO, egui::pos2(100.0, 100.0));
        let [a, b] = view.segment(rect, (-2.0, -2.0), (2.0, 2.0)).unwrap();
        assert!(rect.contains(a) && rect.contains(b));
        assert!(view.segment(rect, (-2.0, 2.0), (2.0, 2.0)).is_none());
        assert!(view.segment(rect, (0.0, f64::NAN), (1.0, 1.0)).is_none());
        assert_eq!(view.zoom(0.0, 0.0, 0.0), view);
        assert_eq!(view.pan(f64::INFINITY, 0.0), view);
        assert_eq!(view.zoom(0.0, 0.0, 0.5).x_max, 0.5);
        assert_eq!(view.pan(1.0, 0.0).x_min, 0.0);
    }
}
