//! Tutorial effects on top-down BGRA pixels; shared by preview and recorded output.
use anyhow::{Result, bail};
use std::{collections::VecDeque, time::Duration};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ZoomMode {
    #[default]
    Off,
    Fixed,
    Follow,
}
impl ZoomMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "原始画面",
            Self::Fixed => "固定焦点放大",
            Self::Follow => "跟随鼠标放大",
        }
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
        {
            bail!("教程放大需要1–4倍与0–100%的焦点坐标");
        }
        Ok(())
    }
    pub fn active(self) -> bool {
        self.mode != ZoomMode::Off || self.highlight || self.clicks
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
}
impl Compositor {
    pub fn idle(&mut self, elapsed: Duration) {
        self.center = None;
        self.scale = 1.0;
        self.last = Some(elapsed);
        self.buttons = None;
        self.clicks.clear();
    }
    pub fn suspend(&mut self) {
        self.buttons = None;
        self.clicks.clear();
        self.last = None;
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
        settings.validate()?;
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
            ZoomMode::Follow => point
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
        let span = [width as f32 / self.scale, height as f32 / self.scale];
        let left = (center[0] - span[0] / 2.0).clamp(0.0, width as f32 - span[0]);
        let top = (center[1] - span[1] / 2.0).clamp(0.0, height as f32 - span[1]);
        self.center = Some([left + span[0] / 2.0, top + span[1] / 2.0]);
        self.last = Some(elapsed);
        output.resize(source.len(), 0);
        if self.scale <= 1.0001 {
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
        let mapped = |p: [f32; 2]| [(p[0] - left) * self.scale, (p[1] - top) * self.scale];
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
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
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
