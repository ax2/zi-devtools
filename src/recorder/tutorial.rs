//! Tutorial effects on top-down BGRA pixels; shared by preview and recorded output.
use anyhow::{Result, bail};
use std::{collections::VecDeque, time::Duration};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ZoomMode {
    #[default]
    Off,
    Fixed,
    Follow,
    Inset,
}
impl ZoomMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "原始画面",
            Self::Fixed => "固定焦点放大",
            Self::Follow => "跟随鼠标放大",
            Self::Inset => "全景＋鼠标放大窗",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    #[default]
    TopRight,
    BottomLeft,
    BottomRight,
}
impl Corner {
    pub fn label(self) -> &'static str {
        match self {
            Self::TopLeft => "左上",
            Self::TopRight => "右上",
            Self::BottomLeft => "左下",
            Self::BottomRight => "右下",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrawingColor {
    #[default]
    Red,
    Yellow,
    Cyan,
    White,
}
impl DrawingColor {
    pub fn label(self) -> &'static str {
        match self {
            Self::Red => "红色",
            Self::Yellow => "黄色",
            Self::Cyan => "青色",
            Self::White => "白色",
        }
    }
    fn rgb(self) -> [u8; 3] {
        match self {
            Self::Red => [255, 55, 65],
            Self::Yellow => [255, 220, 55],
            Self::Cyan => [40, 225, 255],
            Self::White => [250, 250, 250],
        }
    }
}
const SHORTCUT_HINT_MAX: usize = 24;

/// A user-authored key chord to burn into tutorial frames. It never reads OS keyboard input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShortcutHint {
    bytes: [u8; SHORTCUT_HINT_MAX],
    len: u8,
    pub corner: Corner,
}
impl ShortcutHint {
    pub fn parse(value: &str, corner: Corner) -> Result<Self> {
        let normalized = value.trim().to_ascii_uppercase();
        if normalized.is_empty()
            || normalized.len() > SHORTCUT_HINT_MAX
            || !normalized.bytes().any(|byte| byte.is_ascii_alphanumeric())
            || !normalized.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'+' | b'-' | b'/')
            })
        {
            bail!("按键提示仅支持1–24个英文字母、数字、空格和 + - /，且必须包含字母或数字");
        }
        let mut bytes = [0; SHORTCUT_HINT_MAX];
        bytes[..normalized.len()].copy_from_slice(normalized.as_bytes());
        Ok(Self {
            bytes,
            len: normalized.len() as u8,
            corner,
        })
    }
    pub fn text(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or("")
    }
    fn valid(self) -> bool {
        let len = usize::from(self.len);
        len > 0
            && len <= SHORTCUT_HINT_MAX
            && self.bytes[..len]
                .iter()
                .any(|byte| byte.is_ascii_alphanumeric())
            && self.bytes[..len].iter().all(|byte| {
                byte.is_ascii_uppercase()
                    || byte.is_ascii_digit()
                    || matches!(*byte, b' ' | b'+' | b'-' | b'/')
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub mode: ZoomMode,
    pub scale: f32,
    pub focus: [f32; 2],
    pub smooth: bool,
    pub highlight: bool,
    pub clicks: bool,
    /// Burn a high-contrast direction arrow at the current pointer position.
    pub pointer_arrow: bool,
    pub inset_corner: Corner,
    pub spotlight: bool,
    /// Radius as fraction of the output's shorter side.
    pub spotlight_radius: f32,
    pub spotlight_dim: f32,
    /// Optional tutorial step number burned into the recorded frame.
    pub step_number: Option<u8>,
    pub step_corner: Corner,
    /// A manually entered key chord; no keyboard hook or input capture is used.
    pub shortcut_hint: Option<ShortcutHint>,
    /// Show a sampled modifier+single-key chord while recording, when explicitly enabled.
    pub show_pressed_shortcuts: bool,
    pub pressed_shortcut_corner: Corner,
    /// Accept left-button strokes as persistent tutorial annotations.
    pub draw_enabled: bool,
    pub draw_color: DrawingColor,
    /// Increment to clear retained annotation strokes.
    pub draw_clear_epoch: u64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: ZoomMode::Off,
            scale: 2.0,
            focus: [0.5, 0.5],
            smooth: true,
            highlight: false,
            clicks: false,
            pointer_arrow: false,
            inset_corner: Corner::TopRight,
            spotlight: false,
            spotlight_radius: 0.16,
            spotlight_dim: 0.55,
            step_number: None,
            step_corner: Corner::BottomLeft,
            shortcut_hint: None,
            show_pressed_shortcuts: false,
            pressed_shortcut_corner: Corner::TopRight,
            draw_enabled: false,
            draw_color: DrawingColor::default(),
            draw_clear_epoch: 0,
        }
    }
}
impl Settings {
    pub fn validate(self) -> Result<()> {
        if !self.scale.is_finite()
            || !(1.0..=4.0).contains(&self.scale)
            || self
                .focus
                .iter()
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            || !self.spotlight_radius.is_finite()
            || !(0.03..=0.45).contains(&self.spotlight_radius)
            || !self.spotlight_dim.is_finite()
            || !(0.0..=0.9).contains(&self.spotlight_dim)
            || self
                .step_number
                .is_some_and(|value| !(1..=99).contains(&value))
            || self.shortcut_hint.is_some_and(|hint| !hint.valid())
        {
            bail!("教程参数无效：放大1–4倍、焦点0–100%、聚光半径3–45%、压暗0–90%");
        }
        Ok(())
    }
    pub fn active(self) -> bool {
        self.mode != ZoomMode::Off
            || self.highlight
            || self.clicks
            || self.pointer_arrow
            || self.spotlight
            || self.step_number.is_some()
            || self.shortcut_hint.is_some()
            || self.show_pressed_shortcuts
            || self.draw_enabled
    }
}
pub struct Control {
    settings: parking_lot::Mutex<Settings>,
    allowed: bool,
    #[cfg(feature = "ui-preview")]
    fixture_pointer: parking_lot::Mutex<Option<Pointer>>,
    #[cfg(feature = "ui-preview")]
    fixture_source: parking_lot::Mutex<Option<Vec<u8>>>,
}
impl Default for Control {
    fn default() -> Self {
        Self::new(true)
    }
}
impl Control {
    pub(super) fn new(allowed: bool) -> Self {
        Self {
            settings: parking_lot::Mutex::new(Settings::default()),
            allowed,
            #[cfg(feature = "ui-preview")]
            fixture_pointer: parking_lot::Mutex::new(None),
            #[cfg(feature = "ui-preview")]
            fixture_source: parking_lot::Mutex::new(None),
        }
    }
    pub fn set(&self, settings: Settings) -> Result<()> {
        settings.validate()?;
        if settings.active() && !self.allowed {
            bail!("教程效果录制区域最多1600万像素");
        }
        *self.settings.lock() = settings;
        Ok(())
    }
    pub fn snapshot(&self) -> Settings {
        *self.settings.lock()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_pointer(&self, pointer: Pointer) {
        *self.fixture_pointer.lock() = Some(pointer);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_source(&self, pixels: Vec<u8>) {
        assert!(pixels.len() <= 16_000_000 * 4);
        *self.fixture_source.lock() = Some(pixels);
    }
    #[cfg(feature = "ui-preview")]
    pub(super) fn copy_preview_source(&self, source: &mut [u8]) {
        if let Some(pixels) = &*self.fixture_source.lock() {
            assert_eq!(source.len(), pixels.len());
            source.copy_from_slice(pixels);
        }
    }
    pub(super) fn pointer(&self, origin: [i32; 2], region: super::Region) -> Pointer {
        #[cfg(feature = "ui-preview")]
        if let Some(pointer) = *self.fixture_pointer.lock() {
            return pointer;
        }
        physical_pointer(origin, region)
    }
}
#[derive(Clone, Copy, Default)]
pub struct Pointer {
    /// Physical source-region coordinates, not logical UI coordinates.
    pub position: Option<[f32; 2]>,
    pub buttons: u8,
}
struct Click {
    position: [f32; 2],
    at: Duration,
    right: bool,
}
#[derive(Default)]
pub struct Compositor {
    center: Option<[f32; 2]>,
    scale: f32,
    last: Option<Duration>,
    buttons: Option<u8>,
    clicks: VecDeque<Click>,
    columns: Vec<usize>,
    drawings: VecDeque<Stroke>,
    drawing_active: bool,
    drawing_blocked_until_release: bool,
    draw_clear_epoch: u64,
}

#[derive(Clone, Copy)]
pub(super) struct ShortcutFrame {
    pub pressed: Option<ShortcutHint>,
    pub elapsed: Duration,
}
struct Stroke {
    points: Vec<[f32; 2]>,
    color: [u8; 3],
}
const MAX_DRAW_STROKES: usize = 24;
const MAX_DRAW_POINTS_PER_STROKE: usize = 256;
impl Compositor {
    pub fn idle(&mut self, elapsed: Duration) {
        self.center = None;
        self.scale = 1.0;
        self.last = Some(elapsed);
        self.buttons = None;
        self.clicks.clear();
        self.drawing_active = false;
    }
    pub fn suspend(&mut self) {
        self.buttons = None;
        self.clicks.clear();
        self.drawing_active = false;
        self.drawing_blocked_until_release = true;
        self.last = None;
    }
    pub fn has_drawings(&self) -> bool {
        !self.drawings.is_empty()
    }
    pub fn render(
        &mut self,
        source: &[u8],
        size: [u32; 2],
        settings: Settings,
        pointer: Pointer,
        elapsed: Duration,
        output: &mut Vec<u8>,
    ) -> Result<()> {
        self.render_with_shortcut(
            source,
            size,
            settings,
            pointer,
            ShortcutFrame {
                pressed: None,
                elapsed,
            },
            output,
        )
    }
    pub(super) fn render_with_shortcut(
        &mut self,
        source: &[u8],
        size: [u32; 2],
        settings: Settings,
        pointer: Pointer,
        frame: ShortcutFrame,
        output: &mut Vec<u8>,
    ) -> Result<()> {
        let elapsed = frame.elapsed;
        settings.validate()?;
        if settings.draw_clear_epoch != self.draw_clear_epoch {
            self.drawings.clear();
            self.drawing_active = false;
            self.draw_clear_epoch = settings.draw_clear_epoch;
        }
        let [width, height] = size;
        let pixels = u64::from(width) * u64::from(height);
        if width == 0 || height == 0 || pixels > 16_000_000 || source.len() as u64 != pixels * 4 {
            bail!("教程画面尺寸或像素缓冲无效（最多1600万像素）");
        }
        let point = pointer.position.filter(|p| {
            p.iter().all(|v| v.is_finite())
                && p[0] >= 0.0
                && p[1] >= 0.0
                && p[0] < width as f32
                && p[1] < height as f32
        });
        if settings.clicks {
            if let Some(old) = self.buttons {
                let pressed = pointer.buttons & !old;
                if let Some(position) = point {
                    for (mask, right) in [(1, false), (2, true)] {
                        if settings.draw_enabled && mask == 1 {
                            continue;
                        }
                        if pressed & mask != 0 {
                            if self.clicks.len() == 8 {
                                self.clicks.pop_front();
                            }
                            self.clicks.push_back(Click {
                                position,
                                at: elapsed,
                                right,
                            });
                        }
                    }
                }
            }
        } else {
            self.clicks.clear();
        }
        self.buttons = Some(pointer.buttons);
        self.clicks
            .retain(|c| elapsed.saturating_sub(c.at) < Duration::from_millis(500));
        let target_scale = if settings.mode == ZoomMode::Off {
            1.0
        } else {
            settings.scale
        };
        let target = match settings.mode {
            ZoomMode::Follow | ZoomMode::Inset => point
                .or(self.center)
                .unwrap_or([width as f32 / 2.0, height as f32 / 2.0]),
            _ => [
                settings.focus[0] * width as f32,
                settings.focus[1] * height as f32,
            ],
        };
        let factor = if settings.smooth && self.last.is_some() {
            1.0 - (-elapsed.saturating_sub(self.last.unwrap()).as_secs_f32() / 0.18).exp()
        } else {
            1.0
        };
        // Switching off is immediate, so Reset restores the unmodified viewport.
        self.scale = if settings.mode == ZoomMode::Off {
            1.0
        } else {
            self.scale.max(1.0) + (target_scale - self.scale.max(1.0)) * factor
        };
        let old = self.center.unwrap_or(target);
        let center = [
            old[0] + (target[0] - old[0]) * factor,
            old[1] + (target[1] - old[1]) * factor,
        ];
        let inset = [
            (width as f32 * 0.36).round().max(1.0) as u32,
            (height as f32 * 0.36).round().max(1.0) as u32,
        ];
        let viewport = if settings.mode == ZoomMode::Inset {
            inset
        } else {
            size
        };
        let span = [
            viewport[0] as f32 / self.scale,
            viewport[1] as f32 / self.scale,
        ];
        let left = (center[0] - span[0] / 2.0).clamp(0.0, width as f32 - span[0]);
        let top = (center[1] - span[1] / 2.0).clamp(0.0, height as f32 - span[1]);
        self.center = Some([left + span[0] / 2.0, top + span[1] / 2.0]);
        self.last = Some(elapsed);
        output.resize(source.len(), 0);
        if self.scale <= 1.0001 || settings.mode == ZoomMode::Inset {
            output.copy_from_slice(source);
        } else {
            self.columns.clear();
            self.columns.extend((0..width).map(|x| {
                ((left + (x as f32 + 0.5) / self.scale) as u32).min(width - 1) as usize * 4
            }));
            for y in 0..height {
                let sy = ((top + (y as f32 + 0.5) / self.scale) as u32).min(height - 1) as usize;
                let src_row = &source[sy * width as usize * 4..(sy + 1) * width as usize * 4];
                let dst_row = &mut output
                    [y as usize * width as usize * 4..(y as usize + 1) * width as usize * 4];
                for (pixel, &sx) in dst_row.chunks_exact_mut(4).zip(&self.columns) {
                    pixel.copy_from_slice(&src_row[sx..sx + 4]);
                }
            }
        }
        let mapped = |p: [f32; 2]| {
            if settings.mode == ZoomMode::Inset {
                p
            } else {
                [(p[0] - left) * self.scale, (p[1] - top) * self.scale]
            }
        };
        if pointer.buttons & 1 == 0 {
            self.drawing_blocked_until_release = false;
        }
        if settings.draw_enabled
            && !self.drawing_blocked_until_release
            && pointer.buttons & 1 != 0
            && let Some(point) = point.map(mapped)
            && point[0].is_finite()
            && point[1].is_finite()
            && point[0] >= 0.0
            && point[1] >= 0.0
            && point[0] < width as f32
            && point[1] < height as f32
        {
            if !self.drawing_active {
                if self.drawings.len() == MAX_DRAW_STROKES {
                    self.drawings.pop_front();
                }
                self.drawings.push_back(Stroke {
                    points: vec![point],
                    color: settings.draw_color.rgb(),
                });
                self.drawing_active = true;
            } else if let Some(stroke) = self.drawings.back_mut() {
                let last = stroke.points.last().copied().unwrap_or(point);
                if segment_distance(point, last, last) >= 2.0 {
                    if stroke.points.len() == MAX_DRAW_POINTS_PER_STROKE {
                        stroke.points = stroke.points.iter().step_by(2).copied().collect();
                    }
                    stroke.points.push(point);
                }
            }
        } else {
            self.drawing_active = false;
        }
        if settings.spotlight
            && let Some(point) = point
        {
            let point = mapped(point);
            if point[0] >= 0.0
                && point[1] >= 0.0
                && point[0] < width as f32
                && point[1] < height as f32
            {
                spotlight(
                    output,
                    size,
                    point,
                    width.min(height) as f32 * settings.spotlight_radius,
                    settings.spotlight_dim,
                );
            }
        }
        if settings.mode == ZoomMode::Inset {
            magnifier(
                source,
                output,
                size,
                inset,
                settings.inset_corner,
                [left, top],
                self.scale,
            );
        }
        if settings.highlight
            && let Some(p) = point
        {
            disk(output, size, mapped(p), 18.0, 0.0, [40, 220, 255], 0.30);
        }
        for click in &self.clicks {
            let t = elapsed.saturating_sub(click.at).as_secs_f32() / 0.5;
            disk(
                output,
                size,
                mapped(click.position),
                12.0 + t * 26.0,
                3.0,
                if click.right {
                    [255, 160, 70]
                } else {
                    [40, 220, 255]
                },
                (1.0 - t) * 0.9,
            );
        }
        if settings.pointer_arrow
            && let Some(point) = point.map(mapped)
            && point[0].is_finite()
            && point[1].is_finite()
            && point[0] >= 0.0
            && point[1] >= 0.0
            && point[0] < width as f32
            && point[1] < height as f32
        {
            pointer_arrow(output, size, point);
        }
        if let Some(number) = settings.step_number {
            step_marker(output, size, number, settings.step_corner);
        }
        if let Some(hint) = settings.shortcut_hint {
            shortcut_marker(output, size, hint);
        }
        if settings.show_pressed_shortcuts
            && let Some(hint) = frame.pressed
        {
            shortcut_marker(output, size, hint);
        }
        for stroke in &self.drawings {
            for segment in stroke.points.windows(2) {
                draw_line(output, size, segment[0], segment[1], stroke.color);
            }
        }
        Ok(())
    }
}

fn pointer_arrow(output: &mut [u8], [w, h]: [u32; 2], tip: [f32; 2]) {
    if w == 0 || h == 0 {
        return;
    }
    // A familiar upper-left pointer silhouette, scaled for large captures.
    let scale = (w.min(h) / 96).clamp(1, 12) as f32;
    let shape = [
        [0.0, 0.0],
        [0.0, 17.0],
        [4.5, 12.5],
        [8.0, 21.0],
        [12.0, 19.5],
        [8.5, 11.0],
        [16.0, 11.0],
    ]
    .map(|[x, y]| [tip[0] + x * scale, tip[1] + y * scale]);
    let left = shape
        .iter()
        .map(|point| point[0])
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0) as u32;
    let top = shape
        .iter()
        .map(|point| point[1])
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0) as u32;
    let right = shape
        .iter()
        .map(|point| point[0])
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min(w as f32) as u32;
    let bottom = shape
        .iter()
        .map(|point| point[1])
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min(h as f32) as u32;
    let outline = (1.2 * scale).max(1.0);
    for y in top..bottom {
        for x in left..right {
            let sample = [x as f32 + 0.5, y as f32 + 0.5];
            if !inside_polygon(sample, &shape) {
                continue;
            }
            let edge_distance = (0..shape.len())
                .map(|index| {
                    segment_distance(sample, shape[index], shape[(index + 1) % shape.len()])
                })
                .fold(f32::INFINITY, f32::min);
            let color = if edge_distance <= outline {
                [18, 18, 18, 255]
            } else {
                [255, 255, 255, 255]
            };
            let offset = ((y * w + x) * 4) as usize;
            output[offset..offset + 4].copy_from_slice(&color);
        }
    }
}

fn inside_polygon(point: [f32; 2], polygon: &[[f32; 2]]) -> bool {
    let mut inside = false;
    let mut previous = polygon.len() - 1;
    for current in 0..polygon.len() {
        let [x, y] = polygon[current];
        let [previous_x, previous_y] = polygon[previous];
        if (y > point[1]) != (previous_y > point[1])
            && point[0] < (previous_x - x) * (point[1] - y) / (previous_y - y) + x
        {
            inside = !inside;
        }
        previous = current;
    }
    inside
}

fn segment_distance(point: [f32; 2], start: [f32; 2], end: [f32; 2]) -> f32 {
    let segment = [end[0] - start[0], end[1] - start[1]];
    let length_squared = segment[0] * segment[0] + segment[1] * segment[1];
    let t = if length_squared > f32::EPSILON {
        (((point[0] - start[0]) * segment[0] + (point[1] - start[1]) * segment[1]) / length_squared)
            .clamp(0.0, 1.0)
    } else {
        0.0
    };
    let dx = point[0] - (start[0] + t * segment[0]);
    let dy = point[1] - (start[1] + t * segment[1]);
    (dx * dx + dy * dy).sqrt()
}

fn step_marker(output: &mut [u8], [w, h]: [u32; 2], number: u8, corner: Corner) {
    if w == 0 || h == 0 {
        return;
    }
    let scale = (w.min(h) / 96).clamp(1, 12);
    let margin = (scale * 3)
        .min(w.saturating_sub(1) / 2)
        .min(h.saturating_sub(1) / 2);
    let digits = if number >= 10 { 2usize } else { 1usize };
    let glyph_width = 5 * scale;
    let glyph_height = 7 * scale;
    let padding = 2 * scale;
    let box_width = digits as u32 * glyph_width + (digits - 1) as u32 * scale + 2 * padding;
    let box_height = glyph_height + 2 * padding;
    let left = if matches!(corner, Corner::TopLeft | Corner::BottomLeft) {
        margin
    } else {
        w.saturating_sub(box_width + margin)
    };
    let top = if matches!(corner, Corner::TopLeft | Corner::TopRight) {
        margin
    } else {
        h.saturating_sub(box_height + margin)
    };
    let right = left.saturating_add(box_width).min(w);
    let bottom = top.saturating_add(box_height).min(h);
    for y in top..bottom {
        for x in left..right {
            let pixel = &mut output[((y * w + x) * 4) as usize..][..4];
            pixel.copy_from_slice(&[34, 116, 226, 255]);
        }
    }
    let values = if digits == 2 {
        [number / 10, number % 10]
    } else {
        [number % 10, 0]
    };
    for (index, value) in values.into_iter().enumerate().take(digits) {
        let origin_x = left + padding + index as u32 * (glyph_width + scale);
        let origin_y = top + padding;
        for (row, bits) in DIGITS[value as usize].iter().copied().enumerate() {
            for column in 0..5 {
                if bits & (1 << (4 - column)) == 0 {
                    continue;
                }
                for dy in 0..scale {
                    for dx in 0..scale {
                        let x = origin_x + column * scale + dx;
                        let y = origin_y + row as u32 * scale + dy;
                        if x < w && y < h {
                            let pixel = &mut output[((y * w + x) * 4) as usize..][..4];
                            pixel.copy_from_slice(&[255, 255, 255, 255]);
                        }
                    }
                }
            }
        }
    }
}

const DIGITS: [[u8; 7]; 10] = [
    [
        0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
    ],
    [
        0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
    ],
    [
        0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
    ],
    [
        0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
    ],
    [
        0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
    ],
    [
        0b11111, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b11110,
    ],
    [
        0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
    ],
    [
        0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
    ],
    [
        0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
    ],
    [
        0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110,
    ],
];

fn shortcut_marker(output: &mut [u8], [w, h]: [u32; 2], hint: ShortcutHint) {
    if w == 0 || h == 0 || !hint.valid() {
        return;
    }
    // Compact 3x5 uppercase glyphs keep the overlay legible without a font dependency.
    let scale = (w.min(h) / 120).clamp(1, 10);
    let gap = scale;
    let glyph_width = 3 * scale;
    let glyph_height = 5 * scale;
    let padding = 3 * scale;
    let chars = &hint.bytes[..usize::from(hint.len)];
    let content_width = chars.len() as u32 * (glyph_width + gap) - gap;
    let box_width = content_width.saturating_add(padding * 2).min(w);
    let box_height = (glyph_height + padding * 2).min(h);
    let margin = (scale * 3)
        .min(w.saturating_sub(box_width) / 2)
        .min(h.saturating_sub(box_height) / 2);
    let left = if matches!(hint.corner, Corner::TopLeft | Corner::BottomLeft) {
        margin
    } else {
        w.saturating_sub(box_width + margin)
    };
    let top = if matches!(hint.corner, Corner::TopLeft | Corner::TopRight) {
        margin
    } else {
        h.saturating_sub(box_height + margin)
    };
    for y in top..top.saturating_add(box_height).min(h) {
        for x in left..left.saturating_add(box_width).min(w) {
            let pixel = &mut output[((y * w + x) * 4) as usize..][..4];
            pixel.copy_from_slice(&[28, 35, 48, 255]);
        }
    }
    for (index, byte) in chars.iter().copied().enumerate() {
        let Some(rows) = shortcut_glyph(byte) else {
            continue;
        };
        let origin_x = left
            .saturating_add(padding)
            .saturating_add(index as u32 * (glyph_width + gap));
        let origin_y = top.saturating_add(padding);
        for (row, bits) in rows.iter().copied().enumerate() {
            for column in 0..3 {
                if bits & (1 << (2 - column)) == 0 {
                    continue;
                }
                for dy in 0..scale {
                    for dx in 0..scale {
                        let x = origin_x + column * scale + dx;
                        let y = origin_y + row as u32 * scale + dy;
                        if x < w && y < h {
                            let pixel = &mut output[((y * w + x) * 4) as usize..][..4];
                            pixel.copy_from_slice(&[245, 248, 255, 255]);
                        }
                    }
                }
            }
        }
    }
}

fn shortcut_glyph(byte: u8) -> Option<[u8; 5]> {
    Some(match byte {
        b'A' => [2, 5, 7, 5, 5],
        b'B' => [6, 5, 6, 5, 6],
        b'C' => [3, 4, 4, 4, 3],
        b'D' => [6, 5, 5, 5, 6],
        b'E' => [7, 4, 6, 4, 7],
        b'F' => [7, 4, 6, 4, 4],
        b'G' => [3, 4, 5, 5, 3],
        b'H' => [5, 5, 7, 5, 5],
        b'I' => [7, 2, 2, 2, 7],
        b'J' => [1, 1, 1, 5, 2],
        b'K' => [5, 5, 6, 5, 5],
        b'L' => [4, 4, 4, 4, 7],
        b'M' => [5, 7, 7, 5, 5],
        b'N' => [5, 7, 7, 7, 5],
        b'O' => [2, 5, 5, 5, 2],
        b'P' => [6, 5, 6, 4, 4],
        b'Q' => [2, 5, 5, 3, 1],
        b'R' => [6, 5, 6, 5, 5],
        b'S' => [3, 4, 2, 1, 6],
        b'T' => [7, 2, 2, 2, 2],
        b'U' => [5, 5, 5, 5, 7],
        b'V' => [5, 5, 5, 5, 2],
        b'W' => [5, 5, 7, 7, 5],
        b'X' => [5, 5, 2, 5, 5],
        b'Y' => [5, 5, 2, 2, 2],
        b'Z' => [7, 1, 2, 4, 7],
        b'0' => [7, 5, 5, 5, 7],
        b'1' => [2, 6, 2, 2, 7],
        b'2' => [6, 1, 2, 4, 7],
        b'3' => [6, 1, 2, 1, 6],
        b'4' => [5, 5, 7, 1, 1],
        b'5' => [7, 4, 6, 1, 6],
        b'6' => [3, 4, 6, 5, 2],
        b'7' => [7, 1, 2, 2, 2],
        b'8' => [2, 5, 2, 5, 2],
        b'9' => [2, 5, 3, 1, 6],
        b'+' => [0, 2, 7, 2, 0],
        b'-' => [0, 0, 7, 0, 0],
        b'/' => [1, 1, 2, 4, 4],
        b' ' => [0; 5],
        _ => return None,
    })
}
fn draw_line(output: &mut [u8], [w, h]: [u32; 2], from: [f32; 2], to: [f32; 2], color: [u8; 3]) {
    let (mut x0, mut y0) = (from[0].round() as i32, from[1].round() as i32);
    let (x1, y1) = (to[0].round() as i32, to[1].round() as i32);
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;
    let radius = (w.min(h) / 360).clamp(1, 5) as i32;
    loop {
        for oy in -radius..=radius {
            for ox in -radius..=radius {
                let (x, y) = (x0 + ox, y0 + oy);
                if x >= 0 && y >= 0 && x < w as i32 && y < h as i32 {
                    let pixel = &mut output[((y as u32 * w + x as u32) * 4) as usize..][..4];
                    pixel.copy_from_slice(&[color[2], color[1], color[0], 255]);
                }
            }
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let twice = error * 2;
        if twice >= dy {
            error += dy;
            x0 += sx;
        }
        if twice <= dx {
            error += dx;
            y0 += sy;
        }
    }
}
fn spotlight(output: &mut [u8], [w, h]: [u32; 2], point: [f32; 2], radius: f32, dim: f32) {
    let inner = radius * 0.9;
    for y in 0..h {
        let dy = y as f32 + 0.5 - point[1];
        for x in 0..w {
            let dx = x as f32 + 0.5 - point[0];
            let distance = dx * dx + dy * dy;
            if distance <= inner * inner {
                continue;
            }
            let darkness = if distance >= radius * radius {
                dim
            } else {
                dim * (distance.sqrt() - inner) / (radius - inner)
            };
            let pixel = &mut output[((y * w + x) * 4) as usize..][..4];
            for channel in &mut pixel[..3] {
                *channel = (f32::from(*channel) * (1.0 - darkness)).round() as u8;
            }
        }
    }
}
fn magnifier(
    source: &[u8],
    output: &mut [u8],
    size: [u32; 2],
    inset: [u32; 2],
    corner: Corner,
    origin: [f32; 2],
    scale: f32,
) {
    let [w, h] = size;
    let [iw, ih] = inset;
    let margin = ((w.min(h) as f32 * 0.02).round() as u32).min((w - iw).min(h - ih));
    let x0 = if matches!(corner, Corner::TopLeft | Corner::BottomLeft) {
        margin
    } else {
        w - iw - margin
    };
    let y0 = if matches!(corner, Corner::TopLeft | Corner::TopRight) {
        margin
    } else {
        h - ih - margin
    };
    for y in 0..ih {
        let sy = ((origin[1] + (y as f32 + 0.5) / scale) as u32).min(h - 1);
        for x in 0..iw {
            let sx = ((origin[0] + (x as f32 + 0.5) / scale) as u32).min(w - 1);
            let pixel = &mut output[(((y0 + y) * w + x0 + x) * 4) as usize..][..4];
            if x < 2 || y < 2 || x + 2 >= iw || y + 2 >= ih {
                pixel.copy_from_slice(&[230, 150, 60, 255]);
            } else {
                pixel.copy_from_slice(&source[((sy * w + sx) * 4) as usize..][..4]);
            }
        }
    }
}
fn disk(
    output: &mut [u8],
    [w, h]: [u32; 2],
    p: [f32; 2],
    radius: f32,
    thickness: f32,
    color: [u8; 3],
    alpha: f32,
) {
    let min_x = (p[0] - radius).floor().max(0.0) as u32;
    let max_x = (p[0] + radius).ceil().clamp(0.0, w as f32) as u32;
    let min_y = (p[1] - radius).floor().max(0.0) as u32;
    let max_y = (p[1] + radius).ceil().clamp(0.0, h as f32) as u32;
    let inner = (radius - thickness).max(0.0).powi(2);
    for y in min_y..max_y {
        for x in min_x..max_x {
            let distance = (x as f32 + 0.5 - p[0]).powi(2) + (y as f32 + 0.5 - p[1]).powi(2);
            if distance <= radius * radius && (thickness == 0.0 || distance >= inner) {
                let pixel = &mut output[((y * w + x) * 4) as usize..][..4];
                for c in 0..3 {
                    pixel[c] = (f32::from(pixel[c]) * (1.0 - alpha) + f32::from(color[c]) * alpha)
                        .round() as u8;
                }
            }
        }
    }
}

pub(super) fn physical_pointer(origin: [i32; 2], region: super::Region) -> Pointer {
    use windows_sys::Win32::{
        Foundation::POINT,
        UI::{
            Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON},
            WindowsAndMessaging::GetPhysicalCursorPos,
        },
    };
    let mut point = POINT { x: 0, y: 0 };
    let position = (unsafe { GetPhysicalCursorPos(&mut point) } != 0).then_some([
        (i64::from(point.x) - i64::from(origin[0]) - i64::from(region.x)) as f32,
        (i64::from(point.y) - i64::from(origin[1]) - i64::from(region.y)) as f32,
    ]);
    Pointer {
        position,
        buttons: u8::from(unsafe { GetAsyncKeyState(VK_LBUTTON as i32) } < 0)
            | (u8::from(unsafe { GetAsyncKeyState(VK_RBUTTON as i32) } < 0) << 1),
    }
}

pub(super) fn physical_shortcut(corner: Corner) -> Option<ShortcutHint> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    shortcut_from_key_state(
        |key| unsafe { GetAsyncKeyState(i32::from(key)) } < 0,
        corner,
    )
}

fn shortcut_from_key_state(
    mut is_down: impl FnMut(u16) -> bool,
    corner: Corner,
) -> Option<ShortcutHint> {
    let control = is_down(0x11) || is_down(0xA2) || is_down(0xA3);
    let alt = is_down(0x12) || is_down(0xA4) || is_down(0xA5);
    let shift = is_down(0x10) || is_down(0xA0) || is_down(0xA1);
    let win = is_down(0x5B) || is_down(0x5C);
    // Shift-only typing is ordinary text, so it is never shown as a shortcut.
    if !control && !alt && !win {
        return None;
    }
    for code in 0x07..=0xFF {
        let modifier = matches!(code, 0x10..=0x12 | 0x5B..=0x5C | 0xA0..=0xA5);
        let mouse_button = matches!(code, 0x01..=0x06);
        if is_down(code) && !modifier && !mouse_button && !supported_shortcut_key(code) {
            return None;
        }
    }
    let mut key: Option<String> = None;
    let mut accept = |name: String| {
        if key.is_some() {
            return false;
        }
        key = Some(name);
        true
    };
    for code in b'A'..=b'Z' {
        if is_down(u16::from(code)) && !accept(char::from(code).to_string()) {
            return None;
        }
    }
    for code in b'0'..=b'9' {
        if is_down(u16::from(code)) && !accept(char::from(code).to_string()) {
            return None;
        }
    }
    for index in 0..12u16 {
        if is_down(0x70 + index) && !accept(format!("F{}", index + 1)) {
            return None;
        }
    }
    for (code, name) in [
        (0x08, "BACKSPACE"),
        (0x09, "TAB"),
        (0x0D, "ENTER"),
        (0x1B, "ESC"),
        (0x20, "SPACE"),
        (0x21, "PAGEUP"),
        (0x22, "PAGEDOWN"),
        (0x23, "END"),
        (0x24, "HOME"),
        (0x25, "LEFT"),
        (0x26, "UP"),
        (0x27, "RIGHT"),
        (0x28, "DOWN"),
        (0x2D, "INSERT"),
        (0x2E, "DELETE"),
    ] {
        if is_down(code) && !accept(name.to_owned()) {
            return None;
        }
    }
    let key = key?;
    let mut chord = String::new();
    for (enabled, label) in [
        (control, "CTRL"),
        (alt, "ALT"),
        (shift, "SHIFT"),
        (win, "WIN"),
    ] {
        if enabled {
            if !chord.is_empty() {
                chord.push('+');
            }
            chord.push_str(label);
        }
    }
    chord.push('+');
    chord.push_str(&key);
    ShortcutHint::parse(&chord, corner).ok()
}

fn supported_shortcut_key(code: u16) -> bool {
    matches!(code, 0x08..=0x09 | 0x0D | 0x1B | 0x20..=0x28 | 0x2D..=0x2E | 0x30..=0x39 | 0x41..=0x5A | 0x70..=0x7B)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_hint_accepts_only_bounded_user_authored_ascii_chords() {
        let hint = ShortcutHint::parse(" ctrl+shift+s ", Corner::TopLeft).unwrap();
        assert_eq!(hint.text(), "CTRL+SHIFT+S");
        assert_eq!(hint.corner, Corner::TopLeft);
        for invalid in ["", "密码", "CTRL+S!", "ABCDEFGHIJKLMNOPQRSTUVWXY"] {
            assert!(
                ShortcutHint::parse(invalid, Corner::BottomRight).is_err(),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn live_chord_sampling_requires_a_modifier_and_exactly_one_nonmodifier_key() {
        let chord = |keys: &[u16]| {
            shortcut_from_key_state(|key| keys.contains(&key), Corner::TopRight)
                .map(|hint| hint.text().to_owned())
        };
        assert_eq!(chord(&[0x11, 0x53]), Some("CTRL+S".into()));
        assert_eq!(chord(&[0x11, 0x10, 0x53]), Some("CTRL+SHIFT+S".into()));
        assert_eq!(chord(&[0x10, 0x53]), None, "shift-only text is suppressed");
        assert_eq!(chord(&[0x53]), None, "unmodified text is suppressed");
        assert_eq!(chord(&[0x11]), None, "modifier-only input is suppressed");
        assert_eq!(
            chord(&[0x11, 0x53, 0x44]),
            None,
            "multiple simultaneous keys are suppressed"
        );
        assert_eq!(
            chord(&[0x11, 0x53, 0xBA]),
            None,
            "an unsupported extra key suppresses the chord too"
        );
        assert_eq!(chord(&[0x12, 0x73]), Some("ALT+F4".into()));
    }

    #[test]
    fn live_chords_are_opt_in_and_render_only_when_enabled() {
        let source = [24, 32, 40, 255].repeat(320 * 120);
        let chord = ShortcutHint::parse("CTRL+S", Corner::TopRight).unwrap();
        let mut compositor = Compositor::default();
        let mut output = Vec::new();
        compositor
            .render_with_shortcut(
                &source,
                [320, 120],
                Settings::default(),
                Pointer::default(),
                ShortcutFrame {
                    pressed: Some(chord),
                    elapsed: Duration::ZERO,
                },
                &mut output,
            )
            .unwrap();
        assert_eq!(output, source);

        let settings = Settings {
            show_pressed_shortcuts: true,
            pressed_shortcut_corner: Corner::TopRight,
            ..Settings::default()
        };
        compositor
            .render_with_shortcut(
                &source,
                [320, 120],
                settings,
                Pointer::default(),
                ShortcutFrame {
                    pressed: Some(chord),
                    elapsed: Duration::from_millis(33),
                },
                &mut output,
            )
            .unwrap();
        assert_ne!(output, source);
    }

    #[test]
    fn shortcut_hint_is_off_by_default_and_burned_into_the_selected_corner() {
        let source = [24, 32, 40, 255].repeat(320 * 120);
        let mut compositor = Compositor::default();
        let mut output = Vec::new();
        compositor
            .render(
                &source,
                [320, 120],
                Settings::default(),
                Pointer::default(),
                Duration::ZERO,
                &mut output,
            )
            .unwrap();
        assert_eq!(output, source);

        let settings = Settings {
            shortcut_hint: Some(ShortcutHint::parse("CTRL+SHIFT+S", Corner::BottomRight).unwrap()),
            ..Settings::default()
        };
        assert!(settings.active());
        compositor
            .render(
                &source,
                [320, 120],
                settings,
                Pointer::default(),
                Duration::ZERO,
                &mut output,
            )
            .unwrap();
        let pixel = |x: u32, y: u32| &output[((y * 320 + x) * 4) as usize..][..4];
        assert_eq!(pixel(316, 116), [28, 35, 48, 255]);
        assert_eq!(pixel(3, 3), [24, 32, 40, 255]);
    }

    #[test]
    fn drawing_requires_explicit_enable_persists_until_cleared_and_uses_bgra_color() {
        let source = [12, 24, 36, 255].repeat(80 * 80);
        let mut compositor = Compositor::default();
        let mut output = Vec::new();
        let settings = Settings {
            draw_enabled: true,
            draw_color: DrawingColor::Red,
            ..Settings::default()
        };
        compositor
            .render(
                &source,
                [80, 80],
                settings,
                Pointer {
                    position: Some([10.0, 10.0]),
                    buttons: 1,
                },
                Duration::ZERO,
                &mut output,
            )
            .unwrap();
        compositor
            .render(
                &source,
                [80, 80],
                settings,
                Pointer {
                    position: Some([40.0, 40.0]),
                    buttons: 1,
                },
                Duration::from_millis(33),
                &mut output,
            )
            .unwrap();
        assert!(compositor.has_drawings());
        assert_eq!(
            &output[((25 * 80 + 25) * 4) as usize..][..4],
            &[65, 55, 255, 255]
        );

        let inactive = Settings::default();
        compositor
            .render(
                &source,
                [80, 80],
                inactive,
                Pointer::default(),
                Duration::from_millis(66),
                &mut output,
            )
            .unwrap();
        assert_ne!(output, source, "the completed annotation remains visible");
        assert!(compositor.has_drawings());

        let cleared = Settings {
            draw_clear_epoch: 1,
            ..inactive
        };
        compositor
            .render(
                &source,
                [80, 80],
                cleared,
                Pointer::default(),
                Duration::from_millis(99),
                &mut output,
            )
            .unwrap();
        assert_eq!(output, source);
        assert!(!compositor.has_drawings());
    }

    #[test]
    fn drawing_does_not_resume_a_held_drag_after_pause_until_release() {
        let source = [0, 0, 0, 255].repeat(80 * 80);
        let mut compositor = Compositor::default();
        let mut output = Vec::new();
        let settings = Settings {
            draw_enabled: true,
            ..Settings::default()
        };
        let pointer = |x| Pointer {
            position: Some([x, x]),
            buttons: 1,
        };
        compositor
            .render(
                &source,
                [80, 80],
                settings,
                pointer(10.0),
                Duration::ZERO,
                &mut output,
            )
            .unwrap();
        compositor
            .render(
                &source,
                [80, 80],
                settings,
                pointer(20.0),
                Duration::from_millis(33),
                &mut output,
            )
            .unwrap();
        compositor.suspend();
        compositor
            .render(
                &source,
                [80, 80],
                settings,
                pointer(50.0),
                Duration::from_secs(2),
                &mut output,
            )
            .unwrap();
        assert_eq!(
            &output[((35 * 80 + 35) * 4) as usize..][..4],
            &[0, 0, 0, 255]
        );
        compositor
            .render(
                &source,
                [80, 80],
                settings,
                Pointer {
                    position: Some([50.0, 50.0]),
                    buttons: 0,
                },
                Duration::from_secs(2),
                &mut output,
            )
            .unwrap();
        compositor
            .render(
                &source,
                [80, 80],
                settings,
                pointer(50.0),
                Duration::from_secs(2) + Duration::from_millis(33),
                &mut output,
            )
            .unwrap();
        compositor
            .render(
                &source,
                [80, 80],
                settings,
                pointer(60.0),
                Duration::from_secs(2) + Duration::from_millis(66),
                &mut output,
            )
            .unwrap();
        assert_ne!(
            &output[((55 * 80 + 55) * 4) as usize..][..4],
            &[0, 0, 0, 255]
        );
    }

    #[test]
    fn step_marker_is_optional_bounded_and_burned_into_the_selected_corner() {
        let src = [24, 32, 40, 173].repeat(64 * 64);
        let mut compositor = Compositor::default();
        let mut out = Vec::new();
        compositor
            .render(
                &src,
                [64, 64],
                Settings::default(),
                Pointer::default(),
                Duration::ZERO,
                &mut out,
            )
            .unwrap();
        assert_eq!(out, src);

        let settings = Settings {
            step_number: Some(12),
            step_corner: Corner::BottomLeft,
            ..Settings::default()
        };
        assert!(settings.active());
        compositor
            .render(
                &src,
                [64, 64],
                settings,
                Pointer::default(),
                Duration::ZERO,
                &mut out,
            )
            .unwrap();
        let pixel = |x: usize, y: usize| &out[(y * 64 + x) * 4..][..4];
        assert_eq!(pixel(3, 50), &[34, 116, 226, 255]);
        assert_eq!(pixel(7, 52), &[255, 255, 255, 255]);
        assert_eq!(pixel(12, 52), &[255, 255, 255, 255]);
        assert_eq!(pixel(20, 20), &[24, 32, 40, 173]);
    }

    #[test]
    fn step_number_rejects_zero_and_values_over_99() {
        for number in [0, 100, u8::MAX] {
            assert!(
                Settings {
                    step_number: Some(number),
                    ..Settings::default()
                }
                .validate()
                .is_err()
            );
        }
    }

    #[test]
    fn pointer_arrow_is_optional_high_contrast_clipped_and_follows_zoom_mapping() {
        let source = [90, 110, 130, 173].repeat(64 * 64);
        let pointer = Pointer {
            position: Some([20.0, 20.0]),
            buttons: 0,
        };
        let mut compositor = Compositor::default();
        let mut output = Vec::new();
        compositor
            .render(
                &source,
                [64, 64],
                Settings::default(),
                pointer,
                Duration::ZERO,
                &mut output,
            )
            .unwrap();
        assert_eq!(output, source);
        assert!(!Settings::default().active());

        let arrow = Settings {
            pointer_arrow: true,
            mode: ZoomMode::Fixed,
            focus: [0.5, 0.5],
            scale: 2.0,
            smooth: false,
            ..Settings::default()
        };
        assert!(arrow.active());
        compositor
            .render(
                &source,
                [64, 64],
                arrow,
                pointer,
                Duration::ZERO,
                &mut output,
            )
            .unwrap();
        // The fixed zoom maps (20,20) to (8,8); the arrow tip lands there.
        let tip = ((8 * 64 + 8) * 4) as usize;
        assert_eq!(&output[tip..tip + 4], &[18, 18, 18, 255]);
        assert_eq!(source, [90, 110, 130, 173].repeat(64 * 64));

        compositor
            .render(
                &source,
                [64, 64],
                Settings {
                    pointer_arrow: true,
                    ..Settings::default()
                },
                Pointer {
                    position: Some([0.0, 0.0]),
                    buttons: 0,
                },
                Duration::ZERO,
                &mut output,
            )
            .unwrap();
        assert_eq!(output.len(), source.len());
        assert_eq!(&output[..4], &[18, 18, 18, 255]);
    }

    #[test]
    fn spotlight_preserves_center_alpha_source_and_restores_when_pointer_leaves() {
        let src = [160, 200, 240, 173].repeat(64 * 64);
        let mut c = Compositor::default();
        let settings = Settings {
            spotlight: true,
            spotlight_dim: 0.5,
            ..Settings::default()
        };
        let mut out = Vec::new();
        c.render(
            &src,
            [64, 64],
            settings,
            Pointer {
                position: Some([32.0, 32.0]),
                buttons: 0,
            },
            Duration::ZERO,
            &mut out,
        )
        .unwrap();
        assert_eq!(&out[..4], &[80, 100, 120, 173]);
        let center = ((32 * 64 + 32) * 4) as usize;
        assert_eq!(&out[center..center + 4], &src[center..center + 4]);
        assert!(out.chunks_exact(4).all(|p| p[3] == 173));
        c.render(
            &src,
            [64, 64],
            settings,
            Pointer {
                position: Some([-1.0, 32.0]),
                buttons: 0,
            },
            Duration::from_millis(33),
            &mut out,
        )
        .unwrap();
        assert_eq!(src, out);
    }
    #[test]
    fn inset_retains_overview_at_each_corner_and_reset_restores_exact_source() {
        let src = source(128, 64);
        let mut out = Vec::new();
        for corner in [
            Corner::TopLeft,
            Corner::TopRight,
            Corner::BottomLeft,
            Corner::BottomRight,
        ] {
            let mut c = Compositor::default();
            c.render(
                &src,
                [128, 64],
                Settings {
                    mode: ZoomMode::Inset,
                    inset_corner: corner,
                    smooth: false,
                    ..Settings::default()
                },
                Pointer {
                    position: Some([127.0, 63.0]),
                    buttons: 0,
                },
                Duration::ZERO,
                &mut out,
            )
            .unwrap();
            let center = ((32 * 128 + 64) * 4) as usize;
            assert_eq!(&out[center..center + 4], &src[center..center + 4]);
            assert_ne!(src, out);
            c.render(
                &src,
                [128, 64],
                Settings::default(),
                Pointer::default(),
                Duration::from_millis(33),
                &mut out,
            )
            .unwrap();
            assert_eq!(src, out);
        }
        // Tiny legal regions remain bounded, even with a one-pixel lens.
        Compositor::default()
            .render(
                &[1, 2, 3, 255],
                [1, 1],
                Settings {
                    mode: ZoomMode::Inset,
                    ..Settings::default()
                },
                Pointer::default(),
                Duration::ZERO,
                &mut out,
            )
            .unwrap();
        assert_eq!(out.len(), 4);
    }
    #[test]
    fn spotlight_invalid_parameters_do_not_replace_current_control() {
        let control = Control::default();
        let valid = Settings {
            spotlight: true,
            ..Settings::default()
        };
        control.set(valid).unwrap();
        for settings in [
            Settings {
                spotlight_radius: f32::NAN,
                ..valid
            },
            Settings {
                spotlight_radius: 0.0,
                ..valid
            },
            Settings {
                spotlight_dim: f32::INFINITY,
                ..valid
            },
            Settings {
                spotlight_dim: 1.0,
                ..valid
            },
        ] {
            assert!(control.set(settings).is_err());
            assert_eq!(control.snapshot(), valid);
        }
    }
    #[test]
    fn fixed_zoom_with_pointer_outside_visible_crop_does_not_black_out_video() {
        let src = source(64, 64);
        let pointer = Pointer {
            position: Some([63.0, 63.0]),
            buttons: 0,
        };
        let settings = Settings {
            mode: ZoomMode::Fixed,
            focus: [0.0, 0.0],
            smooth: false,
            ..Settings::default()
        };
        let mut original = Vec::new();
        let mut spotlight = Vec::new();
        Compositor::default()
            .render(
                &src,
                [64, 64],
                settings,
                pointer,
                Duration::ZERO,
                &mut original,
            )
            .unwrap();
        Compositor::default()
            .render(
                &src,
                [64, 64],
                Settings {
                    spotlight: true,
                    ..settings
                },
                pointer,
                Duration::ZERO,
                &mut spotlight,
            )
            .unwrap();
        assert_eq!(original, spotlight);
    }
    #[test]
    fn enabling_zoom_after_idle_starts_a_smooth_transition() {
        let mut compositor = Compositor::default();
        let mut output = vec![];
        compositor.idle(Duration::ZERO);
        compositor
            .render(
                &source(128, 64),
                [128, 64],
                Settings {
                    mode: ZoomMode::Fixed,
                    ..Settings::default()
                },
                Pointer::default(),
                Duration::from_millis(33),
                &mut output,
            )
            .unwrap();
        assert!(compositor.scale > 1.0 && compositor.scale < 1.5);
    }
    #[test]
    fn rejected_control_change_keeps_previous_settings() {
        let control = Control::default();
        let good = Settings {
            mode: ZoomMode::Fixed,
            scale: 3.0,
            ..Settings::default()
        };
        control.set(good).unwrap();
        assert!(
            control
                .set(Settings {
                    focus: [f32::NAN, 0.5],
                    ..good
                })
                .is_err()
        );
        assert_eq!(control.snapshot(), good);
        let restricted = Control::new(false);
        assert!(restricted.set(good).is_err());
        assert_eq!(restricted.snapshot(), Settings::default());
    }
    #[test]
    fn smooth_follow_advances_without_jumping_or_mutating_source() {
        let src = source(128, 64);
        let original = src.clone();
        let mut dst = vec![];
        let mut c = Compositor::default();
        let s = Settings {
            mode: ZoomMode::Follow,
            ..Settings::default()
        };
        let pointer = |x| Pointer {
            position: Some([x, 32.0]),
            buttons: 0,
        };
        c.render(&src, [128, 64], s, pointer(32.0), Duration::ZERO, &mut dst)
            .unwrap();
        c.render(
            &src,
            [128, 64],
            s,
            pointer(96.0),
            Duration::from_millis(33),
            &mut dst,
        )
        .unwrap();
        let intermediate = dst[0];
        assert!(intermediate > 0 && intermediate < 64);
        c.render(
            &src,
            [128, 64],
            s,
            pointer(96.0),
            Duration::from_secs(2),
            &mut dst,
        )
        .unwrap();
        assert!(dst[0] > intermediate);
        assert_eq!(src, original);
    }
    fn source(w: u32, h: u32) -> Vec<u8> {
        (0..h)
            .flat_map(|y| (0..w).flat_map(move |x| [x as u8, y as u8, 100, 255]))
            .collect()
    }
    #[test]
    fn unmodified_and_reset_preserve_every_byte() {
        let src = source(64, 32);
        let mut dst = vec![];
        let mut c = Compositor::default();
        c.render(
            &src,
            [64, 32],
            Settings::default(),
            Pointer::default(),
            Duration::ZERO,
            &mut dst,
        )
        .unwrap();
        assert_eq!(src, dst);
        c.render(
            &src,
            [64, 32],
            Settings {
                mode: ZoomMode::Fixed,
                ..Settings::default()
            },
            Pointer::default(),
            Duration::from_secs(1),
            &mut dst,
        )
        .unwrap();
        assert_ne!(src, dst);
        c.render(
            &src,
            [64, 32],
            Settings::default(),
            Pointer::default(),
            Duration::from_secs(2),
            &mut dst,
        )
        .unwrap();
        assert_eq!(src, dst);
    }
    #[test]
    fn source_edges_and_top_down_rows_are_clamped() {
        let src = source(64, 32);
        let mut dst = vec![];
        for (focus, expected) in [([0.0, 0.0], [0, 0]), ([1.0, 1.0], [32, 16])] {
            Compositor::default()
                .render(
                    &src,
                    [64, 32],
                    Settings {
                        mode: ZoomMode::Fixed,
                        focus,
                        smooth: false,
                        ..Settings::default()
                    },
                    Pointer::default(),
                    Duration::ZERO,
                    &mut dst,
                )
                .unwrap();
            assert_eq!(&dst[..2], &expected);
            assert_eq!(dst.len(), src.len());
            assert_eq!(src[2], 100);
        }
    }
    #[test]
    fn clicks_fade_and_pause_does_not_replay_held_button() {
        let src = vec![0; 64 * 64 * 4];
        let mut dst = vec![];
        let mut c = Compositor::default();
        let s = Settings {
            clicks: true,
            ..Settings::default()
        };
        let pointer = |buttons| Pointer {
            position: Some([32.0, 32.0]),
            buttons,
        };
        c.render(&src, [64, 64], s, pointer(0), Duration::ZERO, &mut dst)
            .unwrap();
        c.render(
            &src,
            [64, 64],
            s,
            pointer(1),
            Duration::from_millis(10),
            &mut dst,
        )
        .unwrap();
        assert_ne!(src, dst);
        c.render(
            &src,
            [64, 64],
            s,
            pointer(0),
            Duration::from_millis(600),
            &mut dst,
        )
        .unwrap();
        assert_eq!(src, dst);
        c.suspend();
        c.render(
            &src,
            [64, 64],
            s,
            pointer(1),
            Duration::from_millis(610),
            &mut dst,
        )
        .unwrap();
        assert_eq!(src, dst);
    }
    #[test]
    fn invalid_settings_and_buffer_are_rejected() {
        for scale in [f32::NAN, f32::INFINITY, 0.0, 4.1] {
            assert!(
                Settings {
                    scale,
                    ..Settings::default()
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            Compositor::default()
                .render(
                    &[],
                    [u32::MAX, u32::MAX],
                    Settings::default(),
                    Pointer::default(),
                    Duration::ZERO,
                    &mut vec![]
                )
                .is_err()
        );
    }
    #[test]
    fn follow_maps_pointer_and_rejects_outside_source() {
        let src = source(64, 64);
        let mut dst = vec![];
        let s = Settings {
            mode: ZoomMode::Follow,
            smooth: false,
            ..Settings::default()
        };
        Compositor::default()
            .render(
                &src,
                [64, 64],
                s,
                Pointer {
                    position: Some([48.0, 48.0]),
                    buttons: 0,
                },
                Duration::ZERO,
                &mut dst,
            )
            .unwrap();
        assert_eq!(&dst[..2], &[32, 32]);
        Compositor::default()
            .render(
                &src,
                [64, 64],
                s,
                Pointer {
                    position: Some([-10.0, 80.0]),
                    buttons: 0,
                },
                Duration::ZERO,
                &mut dst,
            )
            .unwrap();
        assert_eq!(&dst[..2], &[16, 16]);
    }
}
