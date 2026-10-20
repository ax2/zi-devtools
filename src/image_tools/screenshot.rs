//! Explicit desktop capture and bounded even-odd alpha selection.
use anyhow::{Result, ensure};
use eframe::egui::{self, Color32, Pos2, Sense, Stroke};
use image::RgbaImage;
use std::{
    path::Path,
    sync::{Arc, mpsc},
};

#[cfg(windows)]
mod overlay;
#[cfg(all(windows, feature = "ui-preview"))]
mod overlay_preview;

const MAX_POINTS: usize = 2048;
const SELECTION_HISTORY_LIMIT: usize = 20;
const MAX_FEATHER_RADIUS: u8 = 32;
const REDACTION_HISTORY_LIMIT: usize = 100;
const MIN_PIXELATION_BLOCK: u8 = 4;
const MAX_PIXELATION_BLOCK: u8 = 64;
const MIN_BLUR_RADIUS: u8 = 4;
const MAX_BLUR_RADIUS: u8 = 32;

#[derive(Clone, Copy, Debug, PartialEq)]
struct RedactionRect {
    a: [f32; 2],
    b: [f32; 2],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum RedactionEffect {
    #[default]
    Black,
    Pixelate {
        block_size: u8,
    },
    Blur {
        radius: u8,
    },
    Gradient {
        direction: GradientDirection,
    },
    Arrow,
    Text,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum GradientDirection {
    #[default]
    Horizontal,
    Vertical,
    Diagonal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TextAnnotation {
    value: String,
    size: u8,
}

#[derive(Clone, Debug, PartialEq)]
struct Redaction {
    rect: RedactionRect,
    effect: RedactionEffect,
    text: Option<TextAnnotation>,
}

fn pinned_viewport_builder(image: &RgbaImage) -> egui::ViewportBuilder {
    let scale = (900.0 / image.width() as f32)
        .min(700.0 / image.height() as f32)
        .min(1.0);
    let initial = egui::vec2(
        (image.width() as f32 * scale).max(220.0),
        (image.height() as f32 * scale + 48.0).max(160.0),
    );
    egui::ViewportBuilder::default()
        .with_title("Zi DevTools · 置顶截图")
        .with_inner_size(initial)
        .with_min_inner_size([220.0, 160.0])
        .with_taskbar(true)
        .with_window_level(egui::WindowLevel::AlwaysOnTop)
}

impl RedactionRect {
    fn from_drag(a: [f32; 2], b: [f32; 2], width: u32, height: u32) -> Option<Self> {
        let left = a[0].min(b[0]);
        let top = a[1].min(b[1]);
        let right = a[0].max(b[0]);
        let bottom = a[1].max(b[1]);
        (((right - left) * width as f32) >= 2.0 && ((bottom - top) * height as f32) >= 2.0)
            .then_some(Self {
                a: [left, top],
                b: [right, bottom],
            })
    }

    fn bounds(self, width: u32, height: u32) -> (u32, u32, u32, u32) {
        (
            (self.a[0] * width as f32).floor().clamp(0.0, width as f32) as u32,
            (self.a[1] * height as f32)
                .floor()
                .clamp(0.0, height as f32) as u32,
            (self.b[0] * width as f32).ceil().clamp(0.0, width as f32) as u32,
            (self.b[1] * height as f32).ceil().clamp(0.0, height as f32) as u32,
        )
    }

    fn arrow_from_drag(a: [f32; 2], b: [f32; 2], width: u32, height: u32) -> Option<Self> {
        let dx = (b[0] - a[0]) * width as f32;
        let dy = (b[1] - a[1]) * height as f32;
        (dx.hypot(dy) >= 6.0).then_some(Self { a, b })
    }
}

fn add_weighted_pixel(sums: &mut [u64; 4], pixel: [u8; 4]) {
    let alpha = u64::from(pixel[3]);
    for (sum, channel) in sums[..3].iter_mut().zip(&pixel[..3]) {
        *sum += u64::from(*channel) * alpha;
    }
    sums[3] += alpha;
}

fn remove_weighted_pixel(sums: &mut [u64; 4], pixel: [u8; 4]) {
    let alpha = u64::from(pixel[3]);
    for (sum, channel) in sums[..3].iter_mut().zip(&pixel[..3]) {
        *sum -= u64::from(*channel) * alpha;
    }
    sums[3] -= alpha;
}

fn weighted_average(sums: [u64; 4], fallback: [u8; 3]) -> [u8; 3] {
    if sums[3] == 0 {
        fallback
    } else {
        [
            (sums[0] / sums[3]) as u8,
            (sums[1] / sums[3]) as u8,
            (sums[2] / sums[3]) as u8,
        ]
    }
}

fn blur_redaction_region(image: &mut RgbaImage, rect: RedactionRect, radius: u8) -> Result<()> {
    let (x0, y0, x1, y1) = rect.bounds(image.width(), image.height());
    let width = x1.saturating_sub(x0);
    let height = y1.saturating_sub(y0);
    if width == 0 || height == 0 {
        return Ok(());
    }
    let area = usize::try_from(width)
        .ok()
        .and_then(|w| usize::try_from(height).ok().and_then(|h| w.checked_mul(h)))
        .ok_or_else(|| anyhow::anyhow!("模糊区域尺寸溢出"))?;
    let mut horizontal = Vec::new();
    horizontal
        .try_reserve_exact(area)
        .map_err(|_| anyhow::anyhow!("模糊区域超出可用内存"))?;
    horizontal.resize(area, [0; 4]);
    let radius = u32::from(radius.clamp(MIN_BLUR_RADIUS, MAX_BLUR_RADIUS));

    for y in y0..y1 {
        let mut sums = [0_u64; 4];
        let initial_end = x0.saturating_add(radius).min(x1 - 1);
        let mut count = initial_end - x0 + 1;
        for sample_x in x0..=initial_end {
            add_weighted_pixel(&mut sums, image.get_pixel(sample_x, y).0);
        }
        for x in x0..x1 {
            let index = (y - y0) as usize * width as usize + (x - x0) as usize;
            let fallback = image.get_pixel(x, y).0;
            let color = weighted_average(sums, [fallback[0], fallback[1], fallback[2]]);
            horizontal[index] = [
                color[0],
                color[1],
                color[2],
                (sums[3] / u64::from(count)) as u8,
            ];
            if let Some(remove_x) = x.checked_sub(radius)
                && remove_x >= x0
            {
                remove_weighted_pixel(&mut sums, image.get_pixel(remove_x, y).0);
                count -= 1;
            }
            if let Some(add_x) = x.checked_add(radius).and_then(|value| value.checked_add(1))
                && add_x < x1
            {
                add_weighted_pixel(&mut sums, image.get_pixel(add_x, y).0);
                count += 1;
            }
        }
    }

    for x in x0..x1 {
        let mut sums = [0_u64; 4];
        let initial_end = y0.saturating_add(radius).min(y1 - 1);
        for sample_y in y0..=initial_end {
            let index = (sample_y - y0) as usize * width as usize + (x - x0) as usize;
            add_weighted_pixel(&mut sums, horizontal[index]);
        }
        for y in y0..y1 {
            let index = (y - y0) as usize * width as usize + (x - x0) as usize;
            let alpha = image.get_pixel(x, y)[3];
            if alpha != 0 {
                let color = weighted_average(
                    sums,
                    [
                        horizontal[index][0],
                        horizontal[index][1],
                        horizontal[index][2],
                    ],
                );
                image.get_pixel_mut(x, y).0[..3].copy_from_slice(&color);
            }
            if let Some(remove_y) = y.checked_sub(radius)
                && remove_y >= y0
            {
                let index = (remove_y - y0) as usize * width as usize + (x - x0) as usize;
                remove_weighted_pixel(&mut sums, horizontal[index]);
            }
            if let Some(add_y) = y.checked_add(radius).and_then(|value| value.checked_add(1))
                && add_y < y1
            {
                let index = (add_y - y0) as usize * width as usize + (x - x0) as usize;
                add_weighted_pixel(&mut sums, horizontal[index]);
            }
        }
    }
    Ok(())
}

fn gradient_redaction_region(
    image: &mut RgbaImage,
    rect: RedactionRect,
    direction: GradientDirection,
) {
    const STOPS: [[u8; 3]; 3] = [[28, 42, 88], [78, 46, 112], [150, 48, 74]];
    let (x0, y0, x1, y1) = rect.bounds(image.width(), image.height());
    let width = x1.saturating_sub(x0);
    let height = y1.saturating_sub(y0);
    if width == 0 || height == 0 {
        return;
    }

    for y in y0..y1 {
        for x in x0..x1 {
            let pixel = image.get_pixel_mut(x, y);
            if pixel.0[3] == 0 {
                continue;
            }
            let horizontal = (x - x0) as f32 / width.saturating_sub(1).max(1) as f32;
            let vertical = (y - y0) as f32 / height.saturating_sub(1).max(1) as f32;
            let position = match direction {
                GradientDirection::Horizontal => horizontal,
                GradientDirection::Vertical => vertical,
                GradientDirection::Diagonal => (horizontal + vertical) * 0.5,
            }
            .clamp(0.0, 1.0)
                * (STOPS.len() - 1) as f32;
            let from = position.floor() as usize;
            let to = (from + 1).min(STOPS.len() - 1);
            let blend = position - from as f32;
            for channel in 0..3 {
                pixel.0[channel] = (f32::from(STOPS[from][channel]) * (1.0 - blend)
                    + f32::from(STOPS[to][channel]) * blend)
                    .round() as u8;
            }
        }
    }
}

fn draw_segment(image: &mut RgbaImage, start: [f32; 2], end: [f32; 2], width: f32, color: [u8; 3]) {
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let length_squared = dx * dx + dy * dy;
    let radius = width * 0.5;
    let left = (start[0].min(end[0]) - radius - 1.0).floor().max(0.0) as u32;
    let top = (start[1].min(end[1]) - radius - 1.0).floor().max(0.0) as u32;
    let right = (start[0].max(end[0]) + radius + 1.0)
        .ceil()
        .min(image.width() as f32) as u32;
    let bottom = (start[1].max(end[1]) + radius + 1.0)
        .ceil()
        .min(image.height() as f32) as u32;
    for y in top..bottom {
        for x in left..right {
            let px = x as f32 + 0.5 - start[0];
            let py = y as f32 + 0.5 - start[1];
            let t = if length_squared <= f32::EPSILON {
                0.0
            } else {
                ((px * dx + py * dy) / length_squared).clamp(0.0, 1.0)
            };
            let distance = (px - t * dx).hypot(py - t * dy);
            let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            let pixel = image.get_pixel_mut(x, y);
            if pixel.0[3] == 0 {
                continue;
            }
            for (channel, new) in pixel.0[..3].iter_mut().zip(color) {
                *channel =
                    (*channel as f32 * (1.0 - coverage) + new as f32 * coverage).round() as u8;
            }
        }
    }
}

fn arrow_points(start: [f32; 2], end: [f32; 2]) -> Option<([f32; 2], [f32; 2])> {
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let length = dx.hypot(dy);
    if length < 1.0 {
        return None;
    }
    let ux = dx / length;
    let uy = dy / length;
    let head = (length * 0.24).clamp(7.0, 18.0).min(length * 0.7);
    let wing = head * 0.62;
    let base = [end[0] - ux * head, end[1] - uy * head];
    let perpendicular = [-uy * wing, ux * wing];
    Some((
        [base[0] + perpendicular[0], base[1] + perpendicular[1]],
        [base[0] - perpendicular[0], base[1] - perpendicular[1]],
    ))
}

fn draw_arrow(image: &mut RgbaImage, start: [f32; 2], end: [f32; 2]) {
    let start = [
        start[0] * image.width() as f32,
        start[1] * image.height() as f32,
    ];
    let end = [
        end[0] * image.width() as f32,
        end[1] * image.height() as f32,
    ];
    let Some((left_head, right_head)) = arrow_points(start, end) else {
        return;
    };
    let segments = [(start, end), (end, left_head), (end, right_head)];
    for (from, to) in segments {
        draw_segment(image, from, to, 7.0, [255, 255, 255]);
    }
    for (from, to) in segments {
        draw_segment(image, from, to, 3.5, [238, 48, 66]);
    }
}

fn paint_arrow(painter: &egui::Painter, start: Pos2, end: Pos2) {
    let Some((left_head, right_head)) = arrow_points([start.x, start.y], [end.x, end.y]) else {
        return;
    };
    let to_pos = |point: [f32; 2]| Pos2::new(point[0], point[1]);
    let segments = [
        (start, end),
        (end, to_pos(left_head)),
        (end, to_pos(right_head)),
    ];
    for (from, to) in segments {
        painter.line_segment([from, to], Stroke::new(7.0, Color32::WHITE));
    }
    for (from, to) in segments {
        painter.line_segment([from, to], Stroke::new(3.5, Color32::from_rgb(238, 48, 66)));
    }
}

fn apply_redactions(image: &mut RgbaImage, redactions: &[Redaction]) -> Result<()> {
    let font = if redactions
        .iter()
        .any(|redaction| redaction.effect == RedactionEffect::Text && redaction.text.is_some())
    {
        Some(super::editor::load_font("")?)
    } else {
        None
    };
    for redaction in redactions {
        match redaction.effect {
            RedactionEffect::Black => {
                let (x0, y0, x1, y1) = redaction.rect.bounds(image.width(), image.height());
                for y in y0..y1 {
                    for x in x0..x1 {
                        image.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
                    }
                }
            }
            RedactionEffect::Pixelate { block_size } => {
                let (x0, y0, x1, y1) = redaction.rect.bounds(image.width(), image.height());
                let block_size =
                    u32::from(block_size.clamp(MIN_PIXELATION_BLOCK, MAX_PIXELATION_BLOCK));
                let mut block_y = y0;
                while block_y < y1 {
                    let end_y = block_y.saturating_add(block_size).min(y1);
                    let mut block_x = x0;
                    while block_x < x1 {
                        let end_x = block_x.saturating_add(block_size).min(x1);
                        let mut alpha_sum = 0_u64;
                        let mut color_sum = [0_u64; 3];
                        for y in block_y..end_y {
                            for x in block_x..end_x {
                                let pixel = image.get_pixel(x, y).0;
                                let alpha = u64::from(pixel[3]);
                                alpha_sum += alpha;
                                for (sum, channel) in color_sum.iter_mut().zip(&pixel[..3]) {
                                    *sum += u64::from(*channel) * alpha;
                                }
                            }
                        }
                        let color = if alpha_sum == 0 {
                            [0, 0, 0]
                        } else {
                            color_sum.map(|sum| (sum / alpha_sum) as u8)
                        };
                        for y in block_y..end_y {
                            for x in block_x..end_x {
                                let pixel = image.get_pixel_mut(x, y);
                                pixel.0[..3].copy_from_slice(&color);
                            }
                        }
                        block_x = end_x;
                    }
                    block_y = end_y;
                }
            }
            RedactionEffect::Blur { radius } => {
                blur_redaction_region(image, redaction.rect, radius)?;
            }
            RedactionEffect::Gradient { direction } => {
                gradient_redaction_region(image, redaction.rect, direction);
            }
            RedactionEffect::Arrow => draw_arrow(image, redaction.rect.a, redaction.rect.b),
            RedactionEffect::Text => {
                if let (Some(text), Some(font)) = (&redaction.text, font.as_ref()) {
                    ensure!(
                        !text.value.trim().is_empty()
                            && text.value.chars().count() <= 80
                            && !text.value.chars().any(char::is_control),
                        "文字需为1–80个非控制字符"
                    );
                    let at = (
                        redaction.rect.a[0] * image.width() as f32,
                        redaction.rect.a[1] * image.height() as f32,
                    );
                    let size = u32::from(text.size.clamp(12, 96));
                    for dy in -2..=2 {
                        for dx in -2..=2 {
                            if dx * dx + dy * dy <= 4 {
                                super::editor::paint_text_preserving_transparency(
                                    image,
                                    (at.0 + dx as f32, at.1 + dy as f32),
                                    &text.value,
                                    [20, 20, 24, 255],
                                    size + 2,
                                    font,
                                )?;
                            }
                        }
                    }
                    super::editor::paint_text_preserving_transparency(
                        image,
                        at,
                        &text.value,
                        [238, 48, 66, 255],
                        size,
                        font,
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn crop_with_redactions(
    source: &RgbaImage,
    points: &[[f32; 2]],
    feather_radius: u8,
    redactions: &[Redaction],
) -> Result<RgbaImage> {
    let mut output = crop_with_feather(source, points, feather_radius)?;
    apply_redactions(&mut output, redactions)?;
    Ok(output)
}

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
    let mut encoded = super::encoding::Output::new(super::MAX_OUTPUT_BYTES);
    output.write_to(&mut encoded, image::ImageFormat::Png)?;
    let encoded = encoded.into_bytes()?;
    ensure!(
        image::load_from_memory(&encoded)?.to_rgba8() == output,
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

fn crop_with_feather(
    source: &RgbaImage,
    points: &[[f32; 2]],
    feather_radius: u8,
) -> Result<RgbaImage> {
    ensure!(feather_radius <= MAX_FEATHER_RADIUS, "羽化半径超出范围");
    let mut output = crop(source, points)?;
    if feather_radius == 0 {
        return Ok(output);
    }

    // A chamfer distance map uses 1/5-pixel units (orthogonal=5, diagonal=7).
    // The map is bounded by crop()'s 16 MP input limit and is discarded after applying.
    let width = output.width() as usize;
    let height = output.height() as usize;
    let limit = feather_radius.saturating_mul(5);
    let sentinel = limit.saturating_add(1);
    let mut distances = vec![sentinel; width * height];
    for y in 0..height {
        for x in 0..width {
            if x == 0
                || y == 0
                || x + 1 == width
                || y + 1 == height
                || output.get_pixel(x as u32, y as u32)[3] == 0
            {
                distances[y * width + x] = 0;
            }
        }
    }

    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            let mut distance = distances[index];
            if x > 0 {
                distance = distance.min(distances[index - 1].saturating_add(5));
                if y > 0 {
                    distance = distance.min(distances[index - width - 1].saturating_add(7));
                }
                if y + 1 < height {
                    distance = distance.min(distances[index + width - 1].saturating_add(7));
                }
            }
            if y > 0 {
                distance = distance.min(distances[index - width].saturating_add(5));
            }
            distances[index] = distance.min(sentinel);
        }
    }
    for y in (0..height).rev() {
        for x in (0..width).rev() {
            let index = y * width + x;
            let mut distance = distances[index];
            if x + 1 < width {
                distance = distance.min(distances[index + 1].saturating_add(5));
                if y > 0 {
                    distance = distance.min(distances[index - width + 1].saturating_add(7));
                }
                if y + 1 < height {
                    distance = distance.min(distances[index + width + 1].saturating_add(7));
                }
            }
            if y + 1 < height {
                distance = distance.min(distances[index + width].saturating_add(5));
            }
            distances[index] = distance.min(sentinel);
        }
    }

    let radius = f32::from(limit);
    for (index, pixel) in output.pixels_mut().enumerate() {
        if pixel[3] == 0 {
            continue;
        }
        let t = (f32::from(distances[index]) / radius).clamp(0., 1.);
        let smooth = t * t * (3. - 2. * t);
        pixel[3] = (f32::from(pixel[3]) * smooth).round() as u8;
    }
    Ok(output)
}

fn save_png(path: &Path, image: &RgbaImage) -> Result<usize> {
    let mut data = super::encoding::Output::new(super::MAX_OUTPUT_BYTES);
    image.write_to(&mut data, image::ImageFormat::Png)?;
    let data = data.into_bytes()?;
    super::save_image_new(path, &data, &std::sync::atomic::AtomicBool::new(false))?;
    Ok(data.len())
}

enum Job {
    #[cfg(windows)]
    Desktop(RgbaImage, crate::recorder::DisplayInfo),
    Output(RgbaImage),
    Copied,
    Saved(usize),
}
#[derive(Clone)]
struct SelectionSnapshot {
    points: Vec<[f32; 2]>,
    freehand: bool,
    feather_radius: u8,
    redactions: Vec<Redaction>,
}
#[derive(Default)]
pub struct State {
    source: Option<Arc<RgbaImage>>,
    output: Option<Arc<RgbaImage>>,
    texture: Option<egui::TextureHandle>,
    preview: Option<egui::TextureHandle>,
    pinned_image: Option<Arc<RgbaImage>>,
    pinned_preview: Option<egui::TextureHandle>,
    gesture: Gesture,
    freehand: bool,
    feather_radius: u8,
    feather_draft: u8,
    edit_boundary: bool,
    editing_point: Option<usize>,
    undo_selections: Vec<SelectionSnapshot>,
    redo_selections: Vec<SelectionSnapshot>,
    redactions: Vec<Redaction>,
    rendered_redactions: Vec<Redaction>,
    redo_redactions: Vec<Redaction>,
    redaction_mode: bool,
    redaction_effect: RedactionEffect,
    pixelation_block_size: u8,
    blur_radius: u8,
    annotation_text: String,
    annotation_size: u8,
    redaction_start: Option<[f32; 2]>,
    redaction_current: Option<[f32; 2]>,
    redaction_render_pending: bool,
    capture_requested: bool,
    capture_inflight: bool,
    annotation_requested: bool,
    restore_requested: bool,
    #[cfg(windows)]
    overlay: Option<overlay::Overlay>,
    next_overlay_id: u64,
    dirty: bool,
    message: String,
    pending: Option<mpsc::Receiver<Result<Job, String>>>,
    #[cfg(test)]
    canvas: Option<egui::Rect>,
    #[cfg(test)]
    redaction_canvas: Option<egui::Rect>,
}
impl State {
    fn pin_current_preview(&mut self) -> bool {
        let (Some(image), Some(preview)) = (&self.output, &self.preview) else {
            return false;
        };
        self.pinned_image = Some(image.clone());
        self.pinned_preview = Some(preview.clone());
        true
    }

    pub fn pinned_window_ui(&mut self, ctx: &egui::Context) {
        let (Some(image), Some(preview)) = (self.pinned_image.clone(), self.pinned_preview.clone())
        else {
            return;
        };
        let id = egui::ViewportId::from_hash_of("zi-pinned-screenshot");
        let builder = pinned_viewport_builder(&image);
        let mut open = true;
        ctx.show_viewport_immediate(id, builder, |panel, _| {
            if panel.input(|input| input.viewport().close_requested()) {
                open = false;
                return;
            }
            egui::CentralPanel::default().show(panel, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "置顶截图 · {} × {} px",
                        image.width(),
                        image.height()
                    ));
                    if ui.button("取消置顶").clicked() {
                        open = false;
                    }
                });
                ui.separator();
                let available = ui.available_size();
                let source_size = egui::vec2(image.width() as f32, image.height() as f32);
                let fit = (available.x / source_size.x)
                    .min(available.y / source_size.y)
                    .min(1.0);
                ui.vertical_centered(|ui| {
                    ui.add(
                        egui::Image::new(&preview).fit_to_exact_size(source_size * fit.max(0.01)),
                    );
                });
            });
        });
        if !open {
            self.pinned_image = None;
            self.pinned_preview = None;
        }
    }

    fn pixelation_block_size(&self) -> u8 {
        if self.pixelation_block_size == 0 {
            12
        } else {
            self.pixelation_block_size
                .clamp(MIN_PIXELATION_BLOCK, MAX_PIXELATION_BLOCK)
        }
    }

    fn set_pixelation_block_size(&mut self, block_size: u8) {
        let block_size = block_size.clamp(MIN_PIXELATION_BLOCK, MAX_PIXELATION_BLOCK);
        self.pixelation_block_size = block_size;
        if matches!(self.redaction_effect, RedactionEffect::Pixelate { .. }) {
            self.redaction_effect = RedactionEffect::Pixelate { block_size };
        }
    }

    fn blur_radius(&self) -> u8 {
        if self.blur_radius == 0 {
            12
        } else {
            self.blur_radius.clamp(MIN_BLUR_RADIUS, MAX_BLUR_RADIUS)
        }
    }

    fn set_blur_radius(&mut self, radius: u8) {
        let radius = radius.clamp(MIN_BLUR_RADIUS, MAX_BLUR_RADIUS);
        self.blur_radius = radius;
        if matches!(self.redaction_effect, RedactionEffect::Blur { .. }) {
            self.redaction_effect = RedactionEffect::Blur { radius };
        }
    }

    fn annotation_size(&self) -> u8 {
        if self.annotation_size == 0 {
            32
        } else {
            self.annotation_size.clamp(12, 96)
        }
    }

    fn current_selection(&self) -> Option<SelectionSnapshot> {
        let points = polygon(&self.gesture.points, self.freehand);
        (self.output.is_some() && points.len() >= 3).then_some(SelectionSnapshot {
            points,
            freehand: self.freehand,
            feather_radius: self.feather_radius,
            redactions: self.redactions.clone(),
        })
    }

    fn push_selection(stack: &mut Vec<SelectionSnapshot>, selection: SelectionSnapshot) {
        if stack.len() == SELECTION_HISTORY_LIMIT {
            stack.remove(0);
        }
        stack.push(selection);
    }

    fn restore_selection(&mut self, ctx: &egui::Context, undo: bool) {
        let current = self.current_selection();
        let selection = if undo {
            self.undo_selections.pop()
        } else {
            self.redo_selections.pop()
        };
        let Some(selection) = selection else {
            return;
        };
        if let Some(current) = current {
            let to = if undo {
                &mut self.redo_selections
            } else {
                &mut self.undo_selections
            };
            Self::push_selection(to, current);
        }
        let source = self.source.clone();
        let Some(source) = source else {
            return;
        };
        self.freehand = selection.freehand;
        self.feather_radius = selection.feather_radius;
        self.feather_draft = selection.feather_radius;
        self.redactions = selection.redactions;
        self.redo_redactions.clear();
        self.gesture = Gesture {
            points: selection.points.clone(),
            ..Default::default()
        };
        self.output = None;
        self.preview = None;
        self.dirty = true;
        self.message = if undo {
            "正在撤销选区…".into()
        } else {
            "正在重做选区…".into()
        };
        let points = selection.points;
        let feather_radius = selection.feather_radius;
        let redactions = self.redactions.clone();
        self.task(ctx, move || {
            crop_with_redactions(&source, &points, feather_radius, &redactions).map(Job::Output)
        });
    }

    fn render_current_output(&mut self, ctx: &egui::Context) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let points = polygon(&self.gesture.points, self.freehand);
        if points.len() < 3 {
            return;
        }
        let feather_radius = self.feather_radius;
        let redactions = self.redactions.clone();
        self.redaction_render_pending = true;
        self.message = "正在更新遮挡预览…".into();
        self.dirty = true;
        self.task(ctx, move || {
            crop_with_redactions(&source, &points, feather_radius, &redactions).map(Job::Output)
        });
    }

    fn undo_redaction(&mut self, ctx: &egui::Context, redo: bool) {
        let operation = if redo {
            self.redo_redactions.pop()
        } else {
            self.redactions.pop()
        };
        let Some(operation) = operation else {
            return;
        };
        let target = if redo {
            &mut self.redactions
        } else {
            &mut self.redo_redactions
        };
        if target.len() == REDACTION_HISTORY_LIMIT {
            target.remove(0);
        }
        target.push(operation);
        self.render_current_output(ctx);
    }

    fn reset_redactions(&mut self) {
        self.redactions.clear();
        self.rendered_redactions.clear();
        self.redo_redactions.clear();
        self.redaction_mode = false;
        self.redaction_start = None;
        self.redaction_current = None;
        self.redaction_render_pending = false;
    }

    fn update_boundary_edit(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        size: egui::Vec2,
        source: Arc<RgbaImage>,
    ) {
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            if self.editing_point.take().is_some() {
                self.edit_boundary = false;
                self.restore_selection(ui.ctx(), true);
            } else {
                self.edit_boundary = false;
                self.message = "已退出轮廓编辑。".into();
            }
            return;
        }
        if response.drag_started()
            && let Some(pointer) = ui.input(|input| input.pointer.press_origin())
        {
            let points = polygon(&self.gesture.points, self.freehand);
            if let Some(index) = nearest_vertex(&points, pointer, rect, size, 14.) {
                if let Some(selection) = self.current_selection() {
                    Self::push_selection(&mut self.undo_selections, selection);
                }
                self.redo_selections.clear();
                self.gesture.points = points;
                self.editing_point = Some(index);
                self.message = "正在调整轮廓点；松开鼠标后重新生成透明预览。".into();
            }
        }

        if let (Some(index), Some(pointer)) = (self.editing_point, response.interact_pointer_pos())
            && let Some(point) = self.gesture.points.get_mut(index)
        {
            point[0] = ((pointer.x - rect.left()) * size.x / rect.width()).clamp(0., size.x);
            point[1] = ((pointer.y - rect.top()) * size.y / rect.height()).clamp(0., size.y);
        }

        if response.drag_stopped() && self.editing_point.take().is_some() {
            let points = polygon(&self.gesture.points, self.freehand);
            let feather_radius = self.feather_radius;
            let redactions = self.redactions.clone();
            self.output = None;
            self.preview = None;
            self.dirty = true;
            self.message = "正在生成调整后的透明预览…".into();
            self.task(ui.ctx(), move || {
                crop_with_redactions(&source, &points, feather_radius, &redactions).map(Job::Output)
            });
        }
    }

    pub(super) fn relay_source(&self) -> Option<Arc<RgbaImage>> {
        self.output.clone()
    }

    pub fn capture_active(&self) -> bool {
        self.capture_requested || self.capture_inflight || self.selecting()
    }
    pub fn request_capture(&mut self) {
        if !self.busy() {
            self.capture_requested = true;
            self.message = "正在隐藏工作台并捕获桌面…".into();
        }
    }
    pub fn take_capture_request(&mut self) -> bool {
        std::mem::take(&mut self.capture_requested)
    }
    pub fn take_annotation_request(&mut self) -> bool {
        std::mem::take(&mut self.annotation_requested)
    }
    pub fn capture_failed(&mut self, message: &str) {
        self.message = message.into();
        self.restore_requested = true;
    }
    #[cfg(windows)]
    pub fn start_desktop_capture(&mut self, ctx: &egui::Context, root: Option<isize>) {
        self.capture_inflight = true;
        self.task(ctx, move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            let root = root.ok_or_else(|| anyhow::anyhow!("工作台窗口不可用"))?;
            ensure!(root_is_hidden(root), "工作台在捕获前已恢复，请重新截图");
            let display = crate::recorder::primary_display()?;
            crate::recorder::validate_display(&display)?;
            let captured = crate::recorder_ui::capture_display_snapshot(&display)?;
            ensure!(
                root_is_hidden(root),
                "工作台在捕获中恢复，本次画面未采用，请重试"
            );
            let bytes = captured.pixels.iter().flat_map(|p| p.to_array()).collect();
            let image = RgbaImage::from_raw(display.width, display.height, bytes)
                .ok_or_else(|| anyhow::anyhow!("捕获尺寸无效"))?;
            Ok(Job::Desktop(image, display))
        });
    }
    fn selecting(&self) -> bool {
        #[cfg(windows)]
        {
            self.overlay.is_some()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }
    pub fn overlay_ui(&mut self, ctx: &egui::Context) -> bool {
        #[cfg(windows)]
        if let Some(mut overlay) = self.overlay.take() {
            match overlay.ui(ctx) {
                Some(overlay::Outcome::Selected(points)) => {
                    self.texture = Some(texture(ctx, "screenshot-source", &overlay.source));
                    self.source = Some(overlay.source.clone());
                    self.undo_selections.clear();
                    self.redo_selections.clear();
                    self.reset_redactions();
                    self.output = None;
                    self.preview = None;
                    self.gesture = Gesture::default();
                    self.gesture.points = points.clone();
                    self.freehand = overlay.freehand();
                    self.edit_boundary = false;
                    self.editing_point = None;
                    self.feather_radius = 0;
                    self.feather_draft = 0;
                    let source = overlay.source.clone();
                    let redactions = self.redactions.clone();
                    self.message = "正在生成透明预览…".into();
                    self.task(ctx, move || {
                        crop_with_redactions(&source, &points, 0, &redactions).map(Job::Output)
                    });
                    self.restore_requested = true;
                }
                Some(overlay::Outcome::Cancelled(message)) => {
                    self.message = message;
                    self.restore_requested = true;
                }
                None => self.overlay = Some(overlay),
            }
        }
        std::mem::take(&mut self.restore_requested)
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
            || self.capture_requested
            || self.selecting()
            || self.editing_point.is_some()
    }
    pub fn has_work(&self) -> bool {
        self.busy() || (self.output.is_some() && self.dirty)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self, ctx: &egui::Context) {
        self.reset_redactions();
        let image = RgbaImage::from_fn(800, 360, |x, y| {
            if x > 90 && x < 710 && y > 70 && y < 300 {
                image::Rgba([40, 120, 190, 255])
            } else {
                image::Rgba([235, 240, 245, 255])
            }
        });
        self.source = Some(Arc::new(image.clone()));
        self.undo_selections.clear();
        self.redo_selections.clear();
        self.reset_redactions();
        self.texture = Some(texture(ctx, "screenshot-fixture-source", &image));
        self.gesture.points = vec![
            [100., 80.],
            [690., 80.],
            [690., 280.],
            [430., 280.],
            [430., 190.],
            [100., 190.],
        ];
        self.freehand = true;
        self.edit_boundary = false;
        self.editing_point = None;
        self.feather_radius = 8;
        self.feather_draft = 8;
        let mut output = crop_with_feather(&image, &self.gesture.points, self.feather_radius)
            .expect("synthetic screenshot feathered mask");
        self.redactions = vec![
            Redaction {
                rect: RedactionRect {
                    a: [0.24, 0.28],
                    b: [0.53, 0.53],
                },
                effect: RedactionEffect::Black,
                text: None,
            },
            Redaction {
                rect: RedactionRect {
                    a: [0.12, 0.18],
                    b: [0.50, 0.18],
                },
                effect: RedactionEffect::Arrow,
                text: None,
            },
        ];
        apply_redactions(&mut output, &self.redactions).expect("fixture annotations render");
        self.rendered_redactions = self.redactions.clone();
        self.preview = Some(texture(ctx, "screenshot-fixture-alpha", &output));
        self.output = Some(Arc::new(output));
        self.redaction_mode = true;
        self.redaction_effect = RedactionEffect::Arrow;
        self.edit_boundary = true;
        self.dirty = true;
        self.message =
            "合成画面用于布局验收，非真实桌面捕获；可在透明预览上拖出黑色遮挡、像素化或箭头。"
                .into();
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
        let was_capture = std::mem::take(&mut self.capture_inflight);
        match result {
            #[cfg(windows)]
            Ok(Job::Desktop(image, display)) => {
                if let Some(next) = self.next_overlay_id.checked_add(1) {
                    self.next_overlay_id = next;
                    self.overlay = Some(overlay::Overlay::new(
                        next,
                        display,
                        image,
                        self.freehand,
                        ctx,
                    ));
                } else {
                    self.capture_failed("选择层编号耗尽，请保存后重新打开程序。");
                }
            }
            Ok(Job::Output(image)) => {
                self.dirty = true;
                self.rendered_redactions = self.redactions.clone();
                self.redaction_render_pending = false;
                self.preview = Some(texture(ctx, "screenshot-alpha", &image));
                self.output = Some(Arc::new(image));
                self.message = "透明 PNG 预览已生成，可以另存。原始画面仍保留。".into();
            }
            Ok(Job::Saved(bytes)) => {
                self.dirty = false;
                self.message = format!("已保存 PNG：{:.1} KiB", bytes as f64 / 1024.)
            }
            Ok(Job::Copied) => self.message = "透明 PNG 已复制到系统剪贴板。".into(),
            Err(error) => {
                if self.redaction_render_pending {
                    self.redactions = self.rendered_redactions.clone();
                    self.redaction_render_pending = false;
                }
                self.message = format!("操作失败：{error}");
                self.restore_requested |= was_capture;
            }
        }
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.poll(ui.ctx());
        ui.heading("截图与透明套索 · 开发中");
        ui.label(
            "捕获后在全屏实景上拖动选区；取消保留旧工作，确认后在此预览透明 PNG。原始画面也可在下方重选。",
        );
        let busy = self.busy();
        let undo_selection_shortcut = !busy
            && !self.undo_selections.is_empty()
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::Z));
        let redo_shortcut = !busy
            && !self.redo_selections.is_empty()
            && (ui.input_mut(|input| {
                input.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::Z)
            }) || ui.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::Y)));
        if undo_selection_shortcut {
            self.restore_selection(ui.ctx(), true);
        } else if redo_shortcut {
            self.restore_selection(ui.ctx(), false);
        }
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal_wrapped(|ui| {
                #[cfg(windows)]
                if ui.button("捕获主屏幕").clicked() {
                    self.request_capture();
                }
                let selection_before_mode_change = self.current_selection();
                let changed = ui
                    .selectable_value(&mut self.freehand, false, "矩形")
                    .changed()
                    | ui.selectable_value(&mut self.freehand, true, "自由轮廓")
                        .changed();
                if changed {
                    if let Some(selection) = selection_before_mode_change {
                        Self::push_selection(&mut self.undo_selections, selection);
                    }
                    self.redo_selections.clear();
                    self.gesture.points.clear();
                    self.output = None;
                    self.preview = None;
                    self.gesture.drawing = false;
                    self.edit_boundary = false;
                    self.editing_point = None;
                    self.reset_redactions();
                }
                if ui
                    .add_enabled(
                        self.output.is_some(),
                        egui::Button::selectable(self.edit_boundary, "编辑轮廓"),
                    )
                    .on_hover_text("拖动边界上的橙色控制点微调选区；空白处不会启动新选区。")
                    .clicked()
                {
                    self.edit_boundary = !self.edit_boundary;
                    self.editing_point = None;
                }
                let has_output = self.output.is_some();
                ui.add_enabled(
                    has_output,
                    egui::Slider::new(&mut self.feather_draft, 0..=MAX_FEATHER_RADIUS)
                        .text("羽化(px)"),
                )
                .on_hover_text(
                    "从选区边缘向内渐隐；0 表示保持原有硬边缘。更改数值不会立即重算图像。",
                );
                if ui
                    .add_enabled(
                        has_output && self.feather_draft != self.feather_radius,
                        egui::Button::new("应用羽化"),
                    )
                    .on_hover_text("按当前半径从原始截图重新生成透明预览，可撤销和重做。")
                    .clicked()
                {
                    if let Some(selection) = self.current_selection() {
                        Self::push_selection(&mut self.undo_selections, selection);
                    }
                    self.redo_selections.clear();
                    self.feather_radius = self.feather_draft;
                    let feather_radius = self.feather_radius;
                    let points = polygon(&self.gesture.points, self.freehand);
                    if let Some(source) = self.source.clone() {
                        let redactions = self.redactions.clone();
                        self.output = None;
                        self.preview = None;
                        self.message = format!("正在从原始截图应用 {feather_radius}px 羽化…");
                        self.task(ui.ctx(), move || {
                            crop_with_redactions(&source, &points, feather_radius, &redactions)
                                .map(Job::Output)
                        });
                    }
                }
                if ui.button("重新选择").clicked() {
                    if let Some(selection) = self.current_selection() {
                        Self::push_selection(&mut self.undo_selections, selection);
                    }
                    self.redo_selections.clear();
                    self.gesture.points.clear();
                    self.output = None;
                    self.preview = None;
                    self.gesture.drawing = false;
                    self.edit_boundary = false;
                    self.editing_point = None;
                    self.reset_redactions();
                }
                if ui
                    .add_enabled(
                        has_output,
                        egui::Button::selectable(
                            self.redaction_mode && self.redaction_effect == RedactionEffect::Black,
                            "黑色遮挡",
                        ),
                    )
                    .on_hover_text("在下方透明 PNG 预览上拖出纯黑矩形；可撤销和重做。")
                    .clicked()
                {
                    let active =
                        self.redaction_mode && self.redaction_effect == RedactionEffect::Black;
                    self.redaction_effect = RedactionEffect::Black;
                    self.redaction_mode = !active;
                    self.redaction_start = None;
                    self.redaction_current = None;
                }
                if ui
                    .add_enabled(
                        has_output,
                        egui::Button::selectable(
                            self.redaction_mode
                                && matches!(
                                    self.redaction_effect,
                                    RedactionEffect::Gradient { .. }
                                ),
                            "渐变遮挡",
                        ),
                    )
                    .on_hover_text("在预览中拖出不透明三色渐变遮挡；可选横向、纵向或对角线方向。")
                    .clicked()
                {
                    let active = self.redaction_mode
                        && matches!(self.redaction_effect, RedactionEffect::Gradient { .. });
                    self.redaction_effect = RedactionEffect::Gradient {
                        direction: match self.redaction_effect {
                            RedactionEffect::Gradient { direction } => direction,
                            _ => GradientDirection::Horizontal,
                        },
                    };
                    self.redaction_mode = !active;
                    self.redaction_start = None;
                    self.redaction_current = None;
                }
                if let RedactionEffect::Gradient { direction } = self.redaction_effect {
                    let mut selected = direction;
                    egui::ComboBox::from_id_salt("screenshot-gradient-direction")
                        .selected_text(match selected {
                            GradientDirection::Horizontal => "横向",
                            GradientDirection::Vertical => "纵向",
                            GradientDirection::Diagonal => "对角线",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut selected,
                                GradientDirection::Horizontal,
                                "横向",
                            );
                            ui.selectable_value(&mut selected, GradientDirection::Vertical, "纵向");
                            ui.selectable_value(
                                &mut selected,
                                GradientDirection::Diagonal,
                                "对角线",
                            );
                        });
                    if selected != direction {
                        self.redaction_effect = RedactionEffect::Gradient {
                            direction: selected,
                        };
                    }
                }
                if ui
                    .add_enabled(
                        has_output,
                        egui::Button::selectable(
                            self.redaction_mode
                                && matches!(self.redaction_effect, RedactionEffect::Blur { .. }),
                            "模糊",
                        ),
                    )
                    .on_hover_text(
                        "在下方预览拖出模糊矩形；半径可调为4–32像素。高度敏感内容请使用黑色遮挡。",
                    )
                    .clicked()
                {
                    let active = self.redaction_mode
                        && matches!(self.redaction_effect, RedactionEffect::Blur { .. });
                    self.redaction_effect = RedactionEffect::Blur {
                        radius: self.blur_radius(),
                    };
                    self.redaction_mode = !active;
                    self.redaction_start = None;
                    self.redaction_current = None;
                }
                if ui
                    .add_enabled(
                        has_output,
                        egui::Button::selectable(
                            self.redaction_mode
                                && matches!(
                                    self.redaction_effect,
                                    RedactionEffect::Pixelate { .. }
                                ),
                            "像素化",
                        ),
                    )
                    .on_hover_text(
                        "在下方透明 PNG 预览上拖出像素化矩形；颜色按块平均，透明度边界保持不变。",
                    )
                    .clicked()
                {
                    let active = self.redaction_mode
                        && matches!(self.redaction_effect, RedactionEffect::Pixelate { .. });
                    self.redaction_mode = !active;
                    self.redaction_effect = RedactionEffect::Pixelate {
                        block_size: self.pixelation_block_size(),
                    };
                    self.redaction_start = None;
                    self.redaction_current = None;
                }
                if ui
                    .add_enabled(
                        has_output,
                        egui::Button::selectable(
                            self.redaction_mode && self.redaction_effect == RedactionEffect::Arrow,
                            "箭头",
                        ),
                    )
                    .on_hover_text("在下方透明 PNG 预览上拖出带白色描边的红色箭头。")
                    .clicked()
                {
                    let active =
                        self.redaction_mode && self.redaction_effect == RedactionEffect::Arrow;
                    self.redaction_effect = RedactionEffect::Arrow;
                    self.redaction_mode = !active;
                    self.redaction_start = None;
                    self.redaction_current = None;
                }
                if ui
                    .add_enabled(
                        has_output,
                        egui::Button::selectable(
                            self.redaction_mode && self.redaction_effect == RedactionEffect::Text,
                            "文字",
                        ),
                    )
                    .on_hover_text("输入一行文字，再点击透明 PNG 预览放置；文字使用本机字体。")
                    .clicked()
                {
                    let active =
                        self.redaction_mode && self.redaction_effect == RedactionEffect::Text;
                    self.redaction_effect = RedactionEffect::Text;
                    self.redaction_mode = !active;
                    self.redaction_start = None;
                    self.redaction_current = None;
                }
                if self.redaction_effect == RedactionEffect::Text {
                    ui.add_enabled(
                        has_output && self.redaction_mode,
                        egui::TextEdit::singleline(&mut self.annotation_text)
                            .hint_text("标注文字，最多80字")
                            .desired_width(180.0),
                    );
                    let mut size = self.annotation_size();
                    ui.add_enabled(
                        has_output && self.redaction_mode,
                        egui::Slider::new(&mut size, 12..=96).text("字号(px)"),
                    );
                    self.annotation_size = size;
                }
                if matches!(self.redaction_effect, RedactionEffect::Pixelate { .. }) {
                    let mut block_size = self.pixelation_block_size();
                    ui.add_enabled(
                        has_output && self.redaction_mode,
                        egui::Slider::new(
                            &mut block_size,
                            MIN_PIXELATION_BLOCK..=MAX_PIXELATION_BLOCK,
                        )
                        .text("块(px)"),
                    );
                    if block_size != self.pixelation_block_size() {
                        self.set_pixelation_block_size(block_size);
                    }
                }
                if matches!(self.redaction_effect, RedactionEffect::Blur { .. }) {
                    let mut radius = self.blur_radius();
                    ui.add_enabled(
                        has_output && self.redaction_mode,
                        egui::Slider::new(&mut radius, MIN_BLUR_RADIUS..=MAX_BLUR_RADIUS)
                            .text("模糊半径(px)"),
                    );
                    if radius != self.blur_radius() {
                        self.set_blur_radius(radius);
                    }
                }
                if ui
                    .add_enabled(!self.redactions.is_empty(), egui::Button::new("撤销标注"))
                    .on_hover_text("撤销最近一次遮挡或箭头；与选区撤销历史分开管理。")
                    .clicked()
                {
                    self.undo_redaction(ui.ctx(), false);
                }
                if ui
                    .add_enabled(
                        !self.redo_redactions.is_empty(),
                        egui::Button::new("重做标注"),
                    )
                    .on_hover_text("重做最近撤销的遮挡或箭头。")
                    .clicked()
                {
                    self.undo_redaction(ui.ctx(), true);
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
                if ui
                    .add_enabled(
                        self.output.is_some() && self.preview.is_some(),
                        egui::Button::new(if self.pinned_image.is_some() {
                            "更新置顶截图"
                        } else {
                            "置顶截图"
                        }),
                    )
                    .on_hover_text(
                        "把当前透明 PNG 预览固定为独立的置顶窗口；关闭该窗口即可取消置顶。",
                    )
                    .clicked()
                {
                    self.pin_current_preview();
                }
                if ui
                    .add_enabled(
                        has_output && self.source.is_some(),
                        egui::Button::new("箭头、文字、打码…"),
                    )
                    .on_hover_text(
                        "在图片编辑器中添加箭头/文字或遮挡区域；会先预览并确认内存接力。",
                    )
                    .clicked()
                {
                    self.annotation_requested = true;
                }
                if ui
                    .add_enabled(
                        !self.undo_selections.is_empty(),
                        egui::Button::new("撤销选区"),
                    )
                    .on_hover_text("撤销上一步选区（Ctrl+Z）")
                    .clicked()
                {
                    self.restore_selection(ui.ctx(), true);
                }
                if ui
                    .add_enabled(
                        !self.redo_selections.is_empty(),
                        egui::Button::new("重做选区"),
                    )
                    .on_hover_text("重做选区（Ctrl+Y 或 Ctrl+Shift+Z）")
                    .clicked()
                {
                    self.restore_selection(ui.ctx(), false);
                }
                #[cfg(windows)]
                if let Some(output) = self.output.clone() {
                    let pixels = u64::from(output.width()) * u64::from(output.height());
                    let copy =
                        ui.add_enabled(pixels <= 4_000_000, egui::Button::new("复制透明 PNG"));
                    let copy = copy.on_hover_text(if pixels <= 4_000_000 {
                        "复制时会替换系统剪贴板；保留透明通道，最多400万像素。"
                    } else {
                        "系统剪贴板图片通道最多支持400万像素。"
                    });
                    if copy.clicked() {
                        self.message = "正在复制透明 PNG…".into();
                        self.task(ui.ctx(), move || {
                            crate::clipboard::copy_screenshot_to_clipboard(&output)
                                .map_err(anyhow::Error::msg)?;
                            Ok(Job::Copied)
                        });
                    }
                }
            });
        });
        ui.add(egui::Label::new(&self.message).truncate());
        if self.redaction_mode {
            let hint = match self.redaction_effect {
                RedactionEffect::Black => "在下方透明 PNG 预览上拖出黑色遮挡；可撤销或重做。",
                RedactionEffect::Pixelate { .. } => {
                    "在下方透明 PNG 预览上拖出像素化区域；松开后显示实际结果。"
                }
                RedactionEffect::Blur { .. } => {
                    "在下方透明 PNG 预览上拖出模糊区域；松开后显示实际结果。"
                }
                RedactionEffect::Gradient { .. } => {
                    "在下方透明 PNG 预览上拖出不透明渐变遮挡；松开后显示实际结果。"
                }
                RedactionEffect::Arrow => {
                    "在下方透明 PNG 预览上从箭头尾部拖向箭头尖端；可撤销或重做。"
                }
                RedactionEffect::Text => {
                    "先输入一行文字，再点击下方透明 PNG 预览放置；可撤销或重做。"
                }
            };
            ui.small(hint);
        }
        if self.edit_boundary {
            ui.label(
                "轮廓编辑：将指针移到橙色控制点并拖动；按 Esc 退出，拖动中按 Esc 还原本次编辑。",
            );
        }
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
        if self.edit_boundary && (!busy || self.editing_point.is_some()) {
            self.update_boundary_edit(ui, &response, rect, size, source.clone());
        } else if !busy {
            if response.drag_started() {
                if let Some(selection) = self.current_selection() {
                    Self::push_selection(&mut self.undo_selections, selection);
                }
                self.redo_selections.clear();
                self.reset_redactions();
                self.output = None;
                self.preview = None;
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.gesture.points.clear();
                self.output = None;
                self.preview = None;
                self.gesture.drawing = false;
                self.reset_redactions();
            }
            if self
                .gesture
                .update(ui, &response, rect, size, self.freehand)
            {
                self.gesture.drawing = false;
                let points = polygon(&self.gesture.points, self.freehand);
                let feather_radius = self.feather_radius;
                let redactions = self.redactions.clone();
                if self.gesture.overflow {
                    self.message = "轮廓超过2048点，请缩短轮廓重新选择。".into();
                } else {
                    self.message = "正在生成透明预览…".into();
                    self.task(ui.ctx(), move || {
                        crop_with_redactions(&source, &points, feather_radius, &redactions)
                            .map(Job::Output)
                    });
                }
            }
        }
        let path: Vec<Pos2> = polygon(&self.gesture.points, self.freehand)
            .iter()
            .map(|p| rect.min + egui::vec2(p[0], p[1]) * scale)
            .collect();
        if path.len() >= 2 {
            ui.painter().add(egui::Shape::closed_line(
                path,
                Stroke::new(2., Color32::from_rgb(255, 185, 55)),
            ));
        }
        if self.edit_boundary {
            let points = polygon(&self.gesture.points, self.freehand);
            let hovered = response
                .hover_pos()
                .and_then(|pointer| nearest_vertex(&points, pointer, rect, size, 14.));
            for (index, point) in points.iter().enumerate() {
                if points.len() <= 32 || self.editing_point == Some(index) || hovered == Some(index)
                {
                    let position = rect.min + egui::vec2(point[0] * scale, point[1] * scale);
                    ui.painter()
                        .circle_filled(position, 6., Color32::from_rgb(255, 185, 55));
                    ui.painter()
                        .circle_stroke(position, 8., Stroke::new(1., Color32::BLACK));
                }
            }
        }
        if let (Some(output), Some(preview)) = (&self.output, &self.preview) {
            ui.label(format!(
                "输出 {} × {} px · 棋盘格代表透明",
                output.width(),
                output.height()
            ));
            let size = egui::vec2(output.width() as f32, output.height() as f32);
            let scale = (ui.available_width() / size.x).min(320. / size.y).min(1.);
            let (rect, _response) = ui.allocate_exact_size(size * scale, Sense::click_and_drag());
            #[cfg(test)]
            {
                self.redaction_canvas = Some(rect);
            }
            let painter = ui.painter_at(rect);
            let image_width = output.width();
            let image_height = output.height();
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
            if self.redaction_mode && !busy {
                let map_pointer = |pointer: Pos2| {
                    [
                        ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
                        ((pointer.y - rect.top()) / rect.height()).clamp(0.0, 1.0),
                    ]
                };
                let events = ui.input(|input| input.events.clone());
                for event in events {
                    match event {
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            ..
                        } if rect.contains(pos) => {
                            self.redaction_start = Some(map_pointer(pos));
                            self.redaction_current = self.redaction_start;
                        }
                        egui::Event::PointerMoved(pos) if self.redaction_start.is_some() => {
                            self.redaction_current = Some(map_pointer(pos));
                        }
                        egui::Event::PointerMoved(pos)
                            if self.redaction_effect == RedactionEffect::Text =>
                        {
                            self.redaction_current = rect.contains(pos).then(|| map_pointer(pos));
                        }
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            ..
                        } => {
                            if let Some(start) = self.redaction_start.take() {
                                let end = map_pointer(pos);
                                let redaction = match self.redaction_effect {
                                    RedactionEffect::Text => {
                                        let value = self.annotation_text.trim();
                                        if value.is_empty()
                                            || value.chars().count() > 80
                                            || value.chars().any(char::is_control)
                                        {
                                            self.message =
                                                "文字需为1–80个非控制字符后才能放置。".into();
                                            None
                                        } else {
                                            Some(Redaction {
                                                rect: RedactionRect { a: end, b: end },
                                                effect: RedactionEffect::Text,
                                                text: Some(TextAnnotation {
                                                    value: value.to_owned(),
                                                    size: self.annotation_size(),
                                                }),
                                            })
                                        }
                                    }
                                    RedactionEffect::Arrow => RedactionRect::arrow_from_drag(
                                        start,
                                        end,
                                        image_width,
                                        image_height,
                                    )
                                    .map(|rect| Redaction {
                                        rect,
                                        effect: self.redaction_effect,
                                        text: None,
                                    }),
                                    RedactionEffect::Black
                                    | RedactionEffect::Pixelate { .. }
                                    | RedactionEffect::Blur { .. }
                                    | RedactionEffect::Gradient { .. } => RedactionRect::from_drag(
                                        start,
                                        end,
                                        image_width,
                                        image_height,
                                    )
                                    .map(|rect| Redaction {
                                        rect,
                                        effect: self.redaction_effect,
                                        text: None,
                                    }),
                                };
                                if let Some(redaction) = redaction {
                                    if self.redactions.len() == REDACTION_HISTORY_LIMIT {
                                        self.redactions.remove(0);
                                    }
                                    self.redactions.push(redaction);
                                    self.redo_redactions.clear();
                                    self.render_current_output(ui.ctx());
                                } else {
                                    self.message = match self.redaction_effect {
                                        RedactionEffect::Arrow => {
                                            "箭头太短；请拖动至少 6 个输出像素。".into()
                                        }
                                        RedactionEffect::Text => {
                                            "请输入1–80个不含换行的字符，再点击预览放置文字。"
                                                .into()
                                        }
                                        RedactionEffect::Black
                                        | RedactionEffect::Pixelate { .. }
                                        | RedactionEffect::Blur { .. }
                                        | RedactionEffect::Gradient { .. } => {
                                            "遮挡区域太小；请拖出至少 2 × 2 px 的矩形。".into()
                                        }
                                    };
                                }
                                self.redaction_current = None;
                            }
                        }
                        _ => {}
                    }
                }
                if let (Some(start), Some(current)) = (self.redaction_start, self.redaction_current)
                {
                    let draft = match self.redaction_effect {
                        RedactionEffect::Arrow => RedactionRect::arrow_from_drag(
                            start,
                            current,
                            image_width,
                            image_height,
                        ),
                        RedactionEffect::Black
                        | RedactionEffect::Pixelate { .. }
                        | RedactionEffect::Blur { .. }
                        | RedactionEffect::Gradient { .. } => {
                            RedactionRect::from_drag(start, current, image_width, image_height)
                        }
                        RedactionEffect::Text => None,
                    };
                    if let Some(draft) = draft {
                        match self.redaction_effect {
                            RedactionEffect::Black => {
                                let min = rect.min
                                    + egui::vec2(
                                        draft.a[0] * rect.width(),
                                        draft.a[1] * rect.height(),
                                    );
                                let max = rect.min
                                    + egui::vec2(
                                        draft.b[0] * rect.width(),
                                        draft.b[1] * rect.height(),
                                    );
                                let draft_rect = egui::Rect::from_min_max(min, max);
                                painter.rect_filled(
                                    draft_rect,
                                    0.0,
                                    Color32::from_rgba_unmultiplied(0, 0, 0, 210),
                                );
                            }
                            RedactionEffect::Pixelate { .. } => {
                                let min = rect.min
                                    + egui::vec2(
                                        draft.a[0] * rect.width(),
                                        draft.a[1] * rect.height(),
                                    );
                                let max = rect.min
                                    + egui::vec2(
                                        draft.b[0] * rect.width(),
                                        draft.b[1] * rect.height(),
                                    );
                                let draft_rect = egui::Rect::from_min_max(min, max);
                                painter.rect_filled(
                                    draft_rect,
                                    0.0,
                                    Color32::from_rgba_unmultiplied(40, 90, 160, 110),
                                );
                                painter.rect_stroke(
                                    draft_rect,
                                    0.0,
                                    Stroke::new(2.0, Color32::from_rgb(40, 90, 220)),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            RedactionEffect::Blur { .. } => {
                                let min = rect.min
                                    + egui::vec2(
                                        draft.a[0] * rect.width(),
                                        draft.a[1] * rect.height(),
                                    );
                                let max = rect.min
                                    + egui::vec2(
                                        draft.b[0] * rect.width(),
                                        draft.b[1] * rect.height(),
                                    );
                                let draft_rect = egui::Rect::from_min_max(min, max);
                                painter.rect_filled(
                                    draft_rect,
                                    0.0,
                                    Color32::from_rgba_unmultiplied(110, 110, 130, 110),
                                );
                                painter.rect_stroke(
                                    draft_rect,
                                    0.0,
                                    Stroke::new(2.0, Color32::from_rgb(90, 90, 130)),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            RedactionEffect::Gradient { .. } => {
                                let min = rect.min
                                    + egui::vec2(
                                        draft.a[0] * rect.width(),
                                        draft.a[1] * rect.height(),
                                    );
                                let max = rect.min
                                    + egui::vec2(
                                        draft.b[0] * rect.width(),
                                        draft.b[1] * rect.height(),
                                    );
                                let draft_rect = egui::Rect::from_min_max(min, max);
                                painter.rect_filled(
                                    draft_rect,
                                    0.0,
                                    Color32::from_rgba_unmultiplied(78, 46, 112, 175),
                                );
                                painter.rect_stroke(
                                    draft_rect,
                                    0.0,
                                    Stroke::new(2.0, Color32::from_rgb(150, 48, 74)),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            RedactionEffect::Arrow => {
                                let start = rect.min
                                    + egui::vec2(
                                        draft.a[0] * rect.width(),
                                        draft.a[1] * rect.height(),
                                    );
                                let end = rect.min
                                    + egui::vec2(
                                        draft.b[0] * rect.width(),
                                        draft.b[1] * rect.height(),
                                    );
                                paint_arrow(&painter, start, end);
                            }
                            RedactionEffect::Text => {}
                        }
                    }
                }
                if self.redaction_effect == RedactionEffect::Text
                    && let Some(pointer) = self.redaction_current
                {
                    let value = self.annotation_text.trim();
                    if !value.is_empty() && value.chars().count() <= 80 {
                        let position = rect.min
                            + egui::vec2(pointer[0] * rect.width(), pointer[1] * rect.height());
                        let font_size = (self.annotation_size() as f32 * scale).clamp(10.0, 48.0);
                        let font = egui::FontId::proportional(font_size);
                        painter.text(
                            position + egui::vec2(1.0, 1.0),
                            egui::Align2::LEFT_TOP,
                            value,
                            font.clone(),
                            Color32::WHITE,
                        );
                        painter.text(
                            position,
                            egui::Align2::LEFT_TOP,
                            value,
                            font,
                            Color32::from_rgb(238, 48, 66),
                        );
                    }
                }
            }
        }
    }
}

#[cfg(windows)]
fn root_is_hidden(root: isize) -> bool {
    use windows_sys::Win32::{
        Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute},
        UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow},
    };
    let hwnd = root as windows_sys::Win32::Foundation::HWND;
    let mut owner = 0;
    let mut cloak = 0u32;
    unsafe {
        IsWindow(hwnd) != 0
            && GetWindowThreadProcessId(hwnd, &mut owner) != 0
            && owner == std::process::id()
            && DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED as u32, &mut cloak as *mut _ as _, 4) == 0
            && cloak != 0
    }
}

#[derive(Default)]
struct Gesture {
    points: Vec<[f32; 2]>,
    drawing: bool,
    overflow: bool,
}
impl Gesture {
    fn update(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        size: egui::Vec2,
        freehand: bool,
    ) -> bool {
        let scale_x = rect.width() / size.x;
        let scale_y = rect.height() / size.y;
        if response.drag_started() {
            self.points.clear();
            self.drawing = true;
            self.overflow = false;
            if let Some(pos) = ui.input(|input| input.pointer.press_origin()) {
                let p = [
                    ((pos.x - rect.left()) / scale_x).clamp(0., size.x),
                    ((pos.y - rect.top()) / scale_y).clamp(0., size.y),
                ];
                self.points.push(p);
                if !freehand {
                    self.points.push(p);
                }
            }
        }
        if self.drawing && freehand {
            // Keep queued movement between rendered frames; a fast curve must not become a chord.
            ui.input(|input| {
                let start = if response.drag_started() {
                    input
                        .events
                        .iter()
                        .position(|e| {
                            matches!(
                                e,
                                egui::Event::PointerButton {
                                    button: egui::PointerButton::Primary,
                                    pressed: true,
                                    ..
                                }
                            )
                        })
                        .unwrap_or(0)
                } else {
                    0
                };
                for event in &input.events[start..] {
                    let pos = match event {
                        egui::Event::PointerMoved(pos)
                        | egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            ..
                        } => Some(*pos),
                        _ => None,
                    };
                    if let Some(pos) = pos {
                        self.push_sample([
                            ((pos.x - rect.left()) / scale_x).clamp(0., size.x),
                            ((pos.y - rect.top()) / scale_y).clamp(0., size.y),
                        ]);
                    }
                    if self.overflow
                        || matches!(
                            event,
                            egui::Event::PointerButton {
                                button: egui::PointerButton::Primary,
                                pressed: false,
                                ..
                            }
                        )
                    {
                        break;
                    }
                }
            });
        }
        if self.drawing
            && let Some(pos) = response.interact_pointer_pos()
        {
            let p = [
                ((pos.x - rect.left()) / scale_x).clamp(0., size.x),
                ((pos.y - rect.top()) / scale_y).clamp(0., size.y),
            ];
            if freehand {
                self.push_sample(p);
            } else if self.points.is_empty() {
                self.points.push(p);
                self.points.push(p);
            } else {
                self.points[1] = p;
            }
        }

        if response.drag_stopped() && self.drawing {
            self.drawing = false;
            true
        } else {
            false
        }
    }
    fn push_sample(&mut self, p: [f32; 2]) {
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
fn nearest_vertex(
    points: &[[f32; 2]],
    pointer: Pos2,
    rect: egui::Rect,
    size: egui::Vec2,
    hit_radius: f32,
) -> Option<usize> {
    points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let position = rect.min
                + egui::vec2(
                    point[0] / size.x * rect.width(),
                    point[1] / size.y * rect.height(),
                );
            (index, position.distance(pointer))
        })
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .filter(|(_, distance)| *distance <= hit_radius)
        .map(|(index, _)| index)
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
    fn black_redaction_is_opaque_clipped_to_the_selected_rectangle() {
        let mut image = RgbaImage::from_pixel(12, 8, image::Rgba([70, 110, 150, 96]));
        apply_redactions(
            &mut image,
            &[Redaction {
                rect: RedactionRect {
                    a: [0.25, 0.25],
                    b: [0.75, 0.75],
                },
                effect: RedactionEffect::Black,
                text: None,
            }],
        )
        .unwrap();
        assert_eq!(image.get_pixel(3, 2).0, [0, 0, 0, 255]);
        assert_eq!(image.get_pixel(8, 5).0, [0, 0, 0, 255]);
        assert_eq!(image.get_pixel(2, 2).0, [70, 110, 150, 96]);
        assert_eq!(image.get_pixel(9, 5).0, [70, 110, 150, 96]);
    }

    #[test]
    fn pixelation_averages_each_block_and_preserves_alpha_edges() {
        let mut image = RgbaImage::from_fn(4, 2, |x, y| {
            let alpha = if x == 1 && y == 0 { 0 } else { 255 };
            let color = if x % 2 == 0 {
                [20, 40, 60]
            } else {
                [100, 120, 140]
            };
            image::Rgba([color[0], color[1], color[2], alpha])
        });
        apply_redactions(
            &mut image,
            &[Redaction {
                rect: RedactionRect {
                    a: [0., 0.],
                    b: [1., 1.],
                },
                effect: RedactionEffect::Pixelate { block_size: 4 },
                text: None,
            }],
        )
        .unwrap();
        assert_eq!(image.get_pixel(0, 0).0, [54, 74, 94, 255]);
        assert_eq!(image.get_pixel(1, 1).0, [54, 74, 94, 255]);
        assert_eq!(image.get_pixel(1, 0).0, [54, 74, 94, 0]);
        assert_eq!(image.get_pixel(2, 0).0, [54, 74, 94, 255]);
        assert_eq!(image.get_pixel(3, 1).0, [54, 74, 94, 255]);
    }

    #[test]
    fn blur_softens_only_the_selected_region_and_preserves_alpha() {
        let mut image = RgbaImage::from_fn(32, 16, |x, y| {
            let color = if x % 2 == 0 { 250 } else { 5 };
            let alpha = if (x + y) % 17 == 0 { 0 } else { 160 };
            image::Rgba([color, color, color, alpha])
        });
        let original = image.clone();
        blur_redaction_region(
            &mut image,
            RedactionRect {
                a: [0.25, 0.125],
                b: [0.75, 0.875],
            },
            6,
        )
        .unwrap();
        for (before, after) in original.pixels().zip(image.pixels()) {
            assert_eq!(before[3], after[3]);
        }
        for y in 0..16 {
            for x in 0..32 {
                if !(8..24).contains(&x) || !(2..14).contains(&y) {
                    assert_eq!(image.get_pixel(x, y), original.get_pixel(x, y));
                }
            }
        }
        assert_ne!(image.get_pixel(16, 8)[0], original.get_pixel(16, 8)[0]);
        assert!((5..250).contains(&image.get_pixel(16, 8)[0]));
    }

    #[test]
    fn gradient_redaction_covers_pixels_directionally_and_preserves_transparency() {
        let source = RgbaImage::from_fn(6, 6, |x, y| {
            if x == 3 && y == 3 {
                image::Rgba([9, 8, 7, 0])
            } else {
                image::Rgba([240, 230, 220, 96])
            }
        });
        let mut horizontal = source.clone();
        gradient_redaction_region(
            &mut horizontal,
            RedactionRect {
                a: [0.0, 0.0],
                b: [1.0, 1.0],
            },
            GradientDirection::Horizontal,
        );
        assert_eq!(horizontal.get_pixel(0, 2).0, [28, 42, 88, 96]);
        assert_eq!(horizontal.get_pixel(5, 2).0, [150, 48, 74, 96]);
        assert_eq!(horizontal.get_pixel(0, 5), horizontal.get_pixel(0, 0));
        assert_eq!(horizontal.get_pixel(3, 3).0, [9, 8, 7, 0]);

        let mut vertical = source.clone();
        gradient_redaction_region(
            &mut vertical,
            RedactionRect {
                a: [0.0, 0.0],
                b: [1.0, 1.0],
            },
            GradientDirection::Vertical,
        );
        assert_eq!(vertical.get_pixel(2, 0).0, [28, 42, 88, 96]);
        assert_eq!(vertical.get_pixel(2, 5).0, [150, 48, 74, 96]);

        let mut clipped = source.clone();
        gradient_redaction_region(
            &mut clipped,
            RedactionRect {
                a: [0.25, 0.25],
                b: [0.75, 0.75],
            },
            GradientDirection::Diagonal,
        );
        assert_eq!(clipped.get_pixel(0, 0), source.get_pixel(0, 0));
        assert_eq!(clipped.get_pixel(2, 2).0, [61, 45, 104, 96]);
        assert_eq!(clipped.get_pixel(4, 4).0, [150, 48, 74, 96]);
        assert_eq!(clipped.get_pixel(3, 3).0[3], 0);
    }

    #[test]
    fn pinned_screenshot_keeps_a_static_preview_until_replaced() {
        let ctx = egui::Context::default();
        let first = Arc::new(RgbaImage::from_pixel(3, 2, image::Rgba([12, 34, 56, 78])));
        let mut state = State {
            output: Some(first.clone()),
            preview: Some(texture(&ctx, "pinned-screenshot-first", &first)),
            ..Default::default()
        };
        assert!(state.pin_current_preview());
        assert_eq!(
            state.pinned_image.as_ref().unwrap().get_pixel(0, 0).0,
            [12, 34, 56, 78]
        );
        assert!(state.pinned_preview.is_some());

        let second = Arc::new(RgbaImage::from_pixel(3, 2, image::Rgba([90, 80, 70, 60])));
        state.output = Some(second.clone());
        state.preview = Some(texture(&ctx, "pinned-screenshot-second", &second));
        assert_eq!(
            state.pinned_image.as_ref().unwrap().get_pixel(0, 0).0,
            [12, 34, 56, 78]
        );

        assert!(state.pin_current_preview());
        assert_eq!(
            state.pinned_image.as_ref().unwrap().get_pixel(0, 0).0,
            [90, 80, 70, 60]
        );
    }

    #[test]
    fn pinned_viewport_is_topmost_taskbar_accessible_and_starts_at_a_safe_size() {
        let small = pinned_viewport_builder(&RgbaImage::new(600, 400));
        assert_eq!(small.title.as_deref(), Some("Zi DevTools · 置顶截图"));
        assert_eq!(small.window_level, Some(egui::WindowLevel::AlwaysOnTop));
        assert_eq!(small.taskbar, Some(true));
        assert_eq!(small.min_inner_size, Some(egui::vec2(220.0, 160.0)));
        assert_eq!(small.inner_size, Some(egui::vec2(600.0, 448.0)));

        let large = pinned_viewport_builder(&RgbaImage::new(3600, 2400));
        assert_eq!(large.inner_size, Some(egui::vec2(900.0, 648.0)));
    }

    #[test]
    fn arrow_rasterization_keeps_alpha_and_uses_drag_direction() {
        let mut image = RgbaImage::from_pixel(40, 24, image::Rgba([50, 60, 70, 128]));
        image.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
        let arrow = RedactionRect::arrow_from_drag([0.9, 0.5], [0.1, 0.5], 40, 24).unwrap();
        assert_eq!(arrow.a, [0.9, 0.5]);
        assert_eq!(arrow.b, [0.1, 0.5]);
        assert!(RedactionRect::arrow_from_drag([0.5, 0.5], [0.51, 0.5], 40, 24).is_none());
        draw_arrow(&mut image, arrow.a, arrow.b);
        assert_eq!(image.get_pixel(20, 12).0, [238, 48, 66, 128]);
        assert_eq!(image.get_pixel(0, 0).0, [0, 0, 0, 0]);
        assert_ne!(image.get_pixel(6, 12).0, [50, 60, 70, 128]);
    }

    #[cfg(windows)]
    #[test]
    fn native_text_annotation_uses_local_font_and_adds_antialiased_opaque_glyphs() {
        let mut image = RgbaImage::from_fn(100, 48, |x, _| {
            if x < 75 {
                image::Rgba([40, 80, 120, 96])
            } else {
                image::Rgba([0, 0, 0, 0])
            }
        });
        apply_redactions(
            &mut image,
            &[Redaction {
                rect: RedactionRect {
                    a: [0.08, 0.10],
                    b: [0.08, 0.10],
                },
                effect: RedactionEffect::Text,
                text: Some(TextAnnotation {
                    value: "M".into(),
                    size: 24,
                }),
            }],
        )
        .expect("Windows system font should render ASCII text");
        assert!(image.pixels().any(|pixel| pixel[3] > 0));
        assert!(image.pixels().any(|pixel| pixel[0] > pixel[1] * 2));
        assert!(image.pixels().any(|pixel| pixel[0] < 80 && pixel[3] > 0));
        assert_eq!(image.get_pixel(99, 47).0, [0, 0, 0, 0]);
    }

    #[test]
    fn black_redaction_history_rebuilds_from_original_and_supports_undo_redo() {
        let ctx = egui::Context::default();
        let source = RgbaImage::from_pixel(20, 12, image::Rgba([80, 120, 160, 255]));
        let points = vec![[0., 0.], [20., 0.], [20., 12.], [0., 12.]];
        let base = crop(&source, &points).unwrap();
        let mut state = State {
            source: Some(Arc::new(source.clone())),
            output: Some(Arc::new(base.clone())),
            gesture: Gesture {
                points: points.clone(),
                ..Default::default()
            },
            redactions: vec![Redaction {
                rect: RedactionRect {
                    a: [0.2, 0.25],
                    b: [0.6, 0.75],
                },
                effect: RedactionEffect::Black,
                text: None,
            }],
            dirty: true,
            ..Default::default()
        };
        let wait_for_output = |state: &mut State| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while state.pending.is_some() && std::time::Instant::now() < deadline {
                state.poll(&ctx);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            assert!(state.pending.is_none(), "redaction render did not finish");
        };

        state.render_current_output(&ctx);
        wait_for_output(&mut state);
        assert_eq!(
            state.output.as_ref().unwrap().get_pixel(5, 4).0,
            [0, 0, 0, 255]
        );
        assert_eq!(
            state.output.as_ref().unwrap().get_pixel(15, 4).0,
            [80, 120, 160, 255]
        );

        state.undo_redaction(&ctx, false);
        wait_for_output(&mut state);
        assert_eq!(state.output.as_deref().unwrap(), &base);
        assert_eq!(state.redo_redactions.len(), 1);

        state.undo_redaction(&ctx, true);
        wait_for_output(&mut state);
        assert_eq!(
            state.output.as_ref().unwrap().get_pixel(5, 4).0,
            [0, 0, 0, 255]
        );
        assert!(state.redactions.len() == 1);
    }

    #[test]
    fn pixelation_history_retains_its_block_size_for_undo_and_redo() {
        let ctx = egui::Context::default();
        let source = RgbaImage::from_fn(16, 8, |x, y| {
            image::Rgba([x as u8 * 12, y as u8 * 20, 90, 255])
        });
        let points = vec![[0., 0.], [16., 0.], [16., 8.], [0., 8.]];
        let base = crop(&source, &points).unwrap();
        let redaction = Redaction {
            rect: RedactionRect {
                a: [0., 0.],
                b: [1., 1.],
            },
            effect: RedactionEffect::Pixelate { block_size: 4 },
            text: None,
        };
        let mut state = State {
            source: Some(Arc::new(source)),
            output: Some(Arc::new(base.clone())),
            gesture: Gesture {
                points: points.clone(),
                ..Default::default()
            },
            redactions: vec![redaction.clone()],
            dirty: true,
            ..Default::default()
        };
        let wait_for_output = |state: &mut State| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while state.pending.is_some() && std::time::Instant::now() < deadline {
                state.poll(&ctx);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            assert!(state.pending.is_none(), "pixelation render did not finish");
        };

        state.render_current_output(&ctx);
        wait_for_output(&mut state);
        let rendered = state.output.as_deref().unwrap().clone();
        assert_eq!(rendered.get_pixel(0, 0).0, rendered.get_pixel(3, 3).0);
        assert_ne!(rendered, base);

        state.undo_redaction(&ctx, false);
        wait_for_output(&mut state);
        assert_eq!(state.output.as_deref().unwrap(), &base);

        state.undo_redaction(&ctx, true);
        wait_for_output(&mut state);
        assert_eq!(state.output.as_deref().unwrap(), &rendered);
        assert_eq!(state.redactions, vec![redaction]);
    }

    #[test]
    fn blur_history_retains_its_radius_for_undo_and_redo() {
        let ctx = egui::Context::default();
        let source = RgbaImage::from_fn(48, 24, |x, y| {
            let value = if x % 2 == 0 { 250 } else { 5 };
            image::Rgba([value, (y * 3) as u8, value, 255])
        });
        let points = vec![[0., 0.], [48., 0.], [48., 24.], [0., 24.]];
        let base = crop(&source, &points).unwrap();
        let blur = Redaction {
            rect: RedactionRect {
                a: [0.15, 0.1],
                b: [0.85, 0.9],
            },
            effect: RedactionEffect::Blur { radius: 8 },
            text: None,
        };
        let mut state = State {
            source: Some(Arc::new(source)),
            output: Some(Arc::new(base.clone())),
            gesture: Gesture {
                points,
                ..Default::default()
            },
            redactions: vec![blur.clone()],
            dirty: true,
            ..Default::default()
        };
        let wait_for_output = |state: &mut State| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while state.pending.is_some() && std::time::Instant::now() < deadline {
                state.poll(&ctx);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            assert!(state.pending.is_none(), "blur render did not finish");
        };

        state.render_current_output(&ctx);
        wait_for_output(&mut state);
        let rendered = state.output.as_deref().unwrap().clone();
        assert_ne!(rendered, base);
        assert_eq!(rendered.get_pixel(0, 0), base.get_pixel(0, 0));

        state.undo_redaction(&ctx, false);
        wait_for_output(&mut state);
        assert_eq!(state.output.as_deref().unwrap(), &base);

        state.undo_redaction(&ctx, true);
        wait_for_output(&mut state);
        assert_eq!(state.output.as_deref().unwrap(), &rendered);
        assert_eq!(state.redactions, vec![blur]);
    }

    #[test]
    fn arrow_history_rebuilds_from_source_and_supports_undo_redo() {
        let ctx = egui::Context::default();
        let source = RgbaImage::from_pixel(40, 24, image::Rgba([80, 120, 160, 255]));
        let points = vec![[0., 0.], [40., 0.], [40., 24.], [0., 24.]];
        let base = crop(&source, &points).unwrap();
        let arrow = Redaction {
            rect: RedactionRect {
                a: [0.9, 0.5],
                b: [0.1, 0.5],
            },
            effect: RedactionEffect::Arrow,
            text: None,
        };
        let mut state = State {
            source: Some(Arc::new(source)),
            output: Some(Arc::new(base.clone())),
            gesture: Gesture {
                points: points.clone(),
                ..Default::default()
            },
            redactions: vec![arrow.clone()],
            dirty: true,
            ..Default::default()
        };
        let wait_for_output = |state: &mut State| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while state.pending.is_some() && std::time::Instant::now() < deadline {
                state.poll(&ctx);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            assert!(state.pending.is_none(), "arrow render did not finish");
        };

        state.render_current_output(&ctx);
        wait_for_output(&mut state);
        let rendered = state.output.as_deref().unwrap().clone();
        assert_eq!(rendered.get_pixel(20, 12).0, [238, 48, 66, 255]);
        assert_ne!(rendered, base);

        state.undo_redaction(&ctx, false);
        wait_for_output(&mut state);
        assert_eq!(state.output.as_deref().unwrap(), &base);

        state.undo_redaction(&ctx, true);
        wait_for_output(&mut state);
        assert_eq!(state.output.as_deref().unwrap(), &rendered);
        assert_eq!(state.redactions, vec![arrow]);
    }

    #[cfg(windows)]
    #[test]
    fn clicking_the_png_preview_places_text_and_supports_undo_redo() {
        let ctx = egui::Context::default();
        let source = RgbaImage::from_pixel(800, 360, image::Rgba([40, 120, 190, 255]));
        let points = vec![[0., 0.], [800., 0.], [800., 360.], [0., 360.]];
        let output = crop(&source, &points).unwrap();
        let mut state = State {
            source: Some(Arc::new(source)),
            texture: Some(texture(&ctx, "text-annotation-source", &output)),
            preview: Some(texture(&ctx, "text-annotation-output", &output)),
            output: Some(Arc::new(output.clone())),
            gesture: Gesture {
                points,
                ..Default::default()
            },
            redaction_mode: true,
            redaction_effect: RedactionEffect::Text,
            annotation_text: "步骤 1".into(),
            annotation_size: 32,
            dirty: true,
            ..Default::default()
        };
        let frame = |state: &mut State, events: Vec<egui::Event>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        Pos2::ZERO,
                        egui::vec2(1200., 1500.),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| state.ui(ui));
                },
            );
        };
        let wait_for_output = |state: &mut State| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while state.pending.is_some() && std::time::Instant::now() < deadline {
                state.poll(&ctx);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            assert!(state.pending.is_none(), "text render did not finish");
        };

        frame(&mut state, vec![]);
        let rect = state.redaction_canvas.unwrap();
        let point = rect.min + egui::vec2(rect.width() * 0.2, rect.height() * 0.25);
        frame(
            &mut state,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        frame(
            &mut state,
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        wait_for_output(&mut state);
        assert_eq!(state.redactions.len(), 1);
        assert_eq!(state.redactions[0].text.as_ref().unwrap().value, "步骤 1");
        let annotated = state.output.as_deref().unwrap().clone();
        assert_ne!(annotated, output);

        state.undo_redaction(&ctx, false);
        wait_for_output(&mut state);
        assert_eq!(state.output.as_deref().unwrap(), &output);
        state.undo_redaction(&ctx, true);
        wait_for_output(&mut state);
        assert_eq!(state.output.as_deref().unwrap(), &annotated);
    }

    #[test]
    fn dragging_on_the_png_preview_creates_a_blur_rectangle() {
        let ctx = egui::Context::default();
        let source = RgbaImage::from_fn(800, 360, |x, _| {
            let value = if x % 2 == 0 { 240 } else { 8 };
            image::Rgba([value, 120, value, 255])
        });
        let points = vec![[0., 0.], [800., 0.], [800., 360.], [0., 360.]];
        let output = crop(&source, &points).unwrap();
        let mut state = State {
            source: Some(Arc::new(source)),
            texture: Some(texture(&ctx, "redaction-drag-source", &output)),
            preview: Some(texture(&ctx, "redaction-drag-output", &output)),
            output: Some(Arc::new(output)),
            gesture: Gesture {
                points,
                ..Default::default()
            },
            redaction_mode: true,
            redaction_effect: RedactionEffect::Blur { radius: 12 },
            dirty: true,
            ..Default::default()
        };
        let frame = |state: &mut State, events: Vec<egui::Event>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        Pos2::ZERO,
                        egui::vec2(1200., 1500.),
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
        let rect = state.redaction_canvas.unwrap();
        let start = rect.min + egui::vec2(rect.width() * 0.2, rect.height() * 0.2);
        let end = rect.min + egui::vec2(rect.width() * 0.6, rect.height() * 0.7);
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
        assert!(
            state.redaction_start.is_some(),
            "redaction pointer did not start inside preview {rect:?} at {start:?}"
        );
        frame(&mut state, vec![egui::Event::PointerMoved(end)]);
        assert_ne!(
            state.redaction_current, state.redaction_start,
            "pointer move did not update redaction draft: start={:?}, current={:?}",
            state.redaction_start, state.redaction_current
        );
        frame(
            &mut state,
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while state.pending.is_some() && std::time::Instant::now() < deadline {
            state.poll(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(state.pending.is_none(), "redaction render did not finish");
        assert_eq!(
            state.redactions.len(),
            1,
            "start={:?}, mode={}, message={}",
            state.redaction_start,
            state.redaction_mode,
            state.message
        );
        let output = state.output.as_deref().unwrap();
        assert_eq!(
            state.redactions[0].effect,
            RedactionEffect::Blur { radius: 12 }
        );
        assert!((8..240).contains(&output.get_pixel(300, 160)[0]));
        assert_eq!(output.get_pixel(100, 160).0, [240, 120, 240, 255]);
    }

    #[test]
    fn selection_undo_and_redo_recompute_from_the_source_without_copying_pixels() {
        let ctx = egui::Context::default();
        let source = RgbaImage::from_fn(6, 4, |x, y| {
            image::Rgba([x as u8 * 30, y as u8 * 40, 90, 255])
        });
        let left = vec![[0., 0.], [3., 0.], [3., 4.], [0., 4.]];
        let right = vec![[3., 0.], [6., 0.], [6., 4.], [3., 4.]];
        let right_output = crop(&source, &right).unwrap();
        let left_redactions = vec![Redaction {
            rect: RedactionRect {
                a: [0., 0.],
                b: [0.5, 1.],
            },
            effect: RedactionEffect::Black,
            text: None,
        }];
        let mut state = State {
            source: Some(Arc::new(source.clone())),
            output: Some(Arc::new(right_output.clone())),
            gesture: Gesture {
                points: right.clone(),
                ..Default::default()
            },
            undo_selections: vec![SelectionSnapshot {
                points: left.clone(),
                freehand: false,
                feather_radius: 0,
                redactions: left_redactions.clone(),
            }],
            dirty: true,
            ..Default::default()
        };

        let wait_for_output = |state: &mut State| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while state.pending.is_some() && std::time::Instant::now() < deadline {
                state.poll(&ctx);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            assert!(
                state.pending.is_none(),
                "selection history worker did not finish"
            );
        };

        state.restore_selection(&ctx, true);
        wait_for_output(&mut state);
        assert_eq!(
            state.output.as_deref().unwrap(),
            &crop_with_redactions(&source, &left, 0, &left_redactions).unwrap()
        );
        assert_eq!(state.redactions, left_redactions);
        assert_eq!(state.redo_selections.len(), 1);

        state.restore_selection(&ctx, false);
        wait_for_output(&mut state);
        assert_eq!(state.output.as_deref().unwrap(), &right_output);
        assert_eq!(state.undo_selections.len(), 1);
    }

    #[test]
    fn rectangle_selection_undo_and_redo_keep_both_two_point_rectangles() {
        let ctx = egui::Context::default();
        let source = RgbaImage::from_fn(8, 6, |x, y| {
            image::Rgba([x as u8 * 20, y as u8 * 30, 90, 255])
        });
        let left = vec![[0., 0.], [4., 0.], [4., 6.], [0., 6.]];
        let right = vec![[4., 0.], [8., 0.], [8., 6.], [4., 6.]];
        let right_output = crop(&source, &right).unwrap();
        let mut state = State {
            source: Some(Arc::new(source.clone())),
            output: Some(Arc::new(right_output.clone())),
            gesture: Gesture {
                points: vec![[4., 0.], [8., 6.]],
                ..Default::default()
            },
            freehand: false,
            undo_selections: vec![SelectionSnapshot {
                points: left.clone(),
                freehand: false,
                feather_radius: 0,
                redactions: Vec::new(),
            }],
            ..Default::default()
        };

        assert!(state.current_selection().is_some());
        let wait_for_output = |state: &mut State| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while state.pending.is_some() && std::time::Instant::now() < deadline {
                state.poll(&ctx);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            assert!(
                state.pending.is_none(),
                "selection history worker did not finish"
            );
        };

        state.restore_selection(&ctx, true);
        wait_for_output(&mut state);
        assert_eq!(
            state.output.as_deref().unwrap(),
            &crop(&source, &left).unwrap()
        );
        assert_eq!(state.redo_selections.len(), 1);
        state.restore_selection(&ctx, false);
        wait_for_output(&mut state);
        assert_eq!(state.output.as_deref().unwrap(), &right_output);
    }

    #[test]
    fn selection_history_is_bounded_to_twenty_coordinate_snapshots() {
        let mut history = Vec::new();
        for value in 0..=SELECTION_HISTORY_LIMIT {
            State::push_selection(
                &mut history,
                SelectionSnapshot {
                    points: vec![[value as f32, 0.], [1., 0.], [1., 1.]],
                    freehand: false,
                    feather_radius: 0,
                    redactions: Vec::new(),
                },
            );
        }
        assert_eq!(history.len(), SELECTION_HISTORY_LIMIT);
        assert_eq!(history[0].points[0][0], 1.);
    }

    #[test]
    fn boundary_editor_only_picks_nearby_vertices() {
        let rect = egui::Rect::from_min_max(Pos2::new(100., 100.), Pos2::new(200., 150.));
        let size = egui::vec2(100., 50.);
        let points = [[10., 10.], [90., 10.], [90., 40.]];
        assert_eq!(
            nearest_vertex(&points, Pos2::new(111., 110.), rect, size, 14.),
            Some(0)
        );
        assert_eq!(
            nearest_vertex(&points, Pos2::new(150., 125.), rect, size, 14.),
            None
        );
    }

    #[test]
    fn selection_history_shortcuts_undo_and_redo_in_the_workbench() {
        let ctx = egui::Context::default();
        let source = RgbaImage::from_fn(6, 4, |x, y| {
            image::Rgba([x as u8 * 30, y as u8 * 40, 90, 255])
        });
        let left = vec![[0., 0.], [3., 0.], [3., 4.], [0., 4.]];
        let right = vec![[3., 0.], [6., 0.], [6., 4.], [3., 4.]];
        let right_output = crop(&source, &right).unwrap();
        let mut state = State {
            source: Some(Arc::new(source.clone())),
            output: Some(Arc::new(right_output.clone())),
            gesture: Gesture {
                points: right,
                ..Default::default()
            },
            undo_selections: vec![SelectionSnapshot {
                points: left.clone(),
                freehand: false,
                feather_radius: 0,
                redactions: Vec::new(),
            }],
            ..Default::default()
        };
        let frame = |state: &mut State, key, modifiers| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        Pos2::ZERO,
                        egui::vec2(900., 800.),
                    )),
                    events: vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    }],
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| state.ui(ui));
                },
            );
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while state.pending.is_some() && std::time::Instant::now() < deadline {
                state.poll(&ctx);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            assert!(
                state.pending.is_none(),
                "selection history worker did not finish"
            );
        };

        frame(&mut state, egui::Key::Z, egui::Modifiers::CTRL);
        assert_eq!(
            state.output.as_deref().unwrap(),
            &crop(&source, &left).unwrap()
        );
        frame(
            &mut state,
            egui::Key::Z,
            egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
        );
        assert_eq!(state.output.as_deref().unwrap(), &right_output);
        frame(&mut state, egui::Key::Z, egui::Modifiers::CTRL);
        assert_eq!(
            state.output.as_deref().unwrap(),
            &crop(&source, &left).unwrap()
        );
        frame(&mut state, egui::Key::Y, egui::Modifiers::CTRL);
        assert_eq!(state.output.as_deref().unwrap(), &right_output);
    }

    #[test]
    fn dragging_a_boundary_handle_rebuilds_preview_and_keeps_undo_history() {
        let ctx = egui::Context::default();
        let source = RgbaImage::from_fn(300, 220, |x, y| image::Rgba([x as u8, y as u8, 90, 255]));
        let points = vec![[20., 20.], [260., 20.], [260., 180.], [20., 180.]];
        let output = crop(&source, &points).unwrap();
        let original_output = output.clone();
        let mut state = State {
            source: Some(Arc::new(source.clone())),
            output: Some(Arc::new(output)),
            texture: Some(texture(&ctx, "boundary-edit-source", &source)),
            gesture: Gesture {
                points: vec![[20., 20.], [260., 180.]],
                ..Default::default()
            },
            freehand: false,
            edit_boundary: true,
            dirty: true,
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
        let rect = state.canvas.unwrap();
        let screen_point = |x: f32, y: f32| {
            rect.min
                + egui::vec2(
                    x * rect.width() / source.width() as f32,
                    y * rect.height() / source.height() as f32,
                )
        };
        let start = screen_point(20., 20.);
        let end = screen_point(60., 60.);
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
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while state.pending.is_some() && std::time::Instant::now() < deadline {
            state.poll(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(
            state.pending.is_none(),
            "edited selection worker did not finish"
        );
        assert_eq!(state.gesture.points[0], [60., 60.]);
        let edited = state.output.as_deref().unwrap();
        assert_eq!(edited.dimensions(), (240, 160));
        assert_ne!(edited, &original_output);
        let edited_output = edited.clone();
        assert_eq!(state.undo_selections.len(), 1);
        assert_eq!(
            state.undo_selections[0].points,
            vec![[20., 20.], [260., 20.], [260., 180.], [20., 180.]]
        );
        state.restore_selection(&ctx, true);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while state.pending.is_some() && std::time::Instant::now() < deadline {
            state.poll(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(state.output.as_deref().unwrap(), &original_output);
        state.restore_selection(&ctx, false);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while state.pending.is_some() && std::time::Instant::now() < deadline {
            state.poll(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(state.output.as_deref().unwrap(), &edited_output);
    }

    #[test]
    fn screenshot_clipboard_preparation_preserves_alpha_and_enforces_pixel_limit() {
        let image = RgbaImage::from_fn(2, 2, |x, y| {
            image::Rgba([
                x as u8 * 100,
                y as u8 * 100,
                25,
                if x == y { 0 } else { 180 },
            ])
        });
        let (picture, _dib) = crate::clipboard::prepare_screenshot_picture(&image).unwrap();
        assert_eq!(picture.image().unwrap().to_rgba8(), image);
        assert!(crate::clipboard::validate_screenshot_copy_dimensions(2001, 2000).is_err());
        assert!(crate::clipboard::validate_screenshot_copy_dimensions(2000, 2000).is_ok());
    }

    #[test]
    fn queued_freehand_movement_keeps_concave_shape_between_frames() {
        let ctx = egui::Context::default();
        let image = RgbaImage::from_pixel(300, 200, image::Rgba([10, 20, 30, 255]));
        let mut state = State {
            texture: Some(texture(&ctx, "queued-pointer-test", &image)),
            source: Some(Arc::new(image)),
            freehand: true,
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
        let origin = state.canvas.unwrap().min;
        let start = origin + egui::vec2(10., 10.);
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
        let mut events: Vec<_> = [
            [100., 10.],
            [100., 120.],
            [50., 120.],
            [50., 60.],
            [10., 60.],
        ]
        .into_iter()
        .map(|p| egui::Event::PointerMoved(origin + egui::vec2(p[0], p[1])))
        .collect();
        events.push(egui::Event::PointerButton {
            pos: origin + egui::vec2(10., 60.),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        frame(&mut state, events);
        for _ in 0..100 {
            frame(&mut state, vec![]);
            if state.pending.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let output = state.output.as_ref().unwrap();
        assert_eq!(output.dimensions(), (90, 110));
        assert_eq!(output.get_pixel(15, 80)[3], 0);
        assert_eq!(output.get_pixel(80, 80)[3], 255);
        assert_eq!(state.gesture.points.len(), 6);
    }
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
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
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
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
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
            state.gesture.points
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
        assert!(state.output.is_none() && state.gesture.points.is_empty());
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
    fn feather_softens_inward_without_changing_crop_dimensions_or_rgb() {
        let image = RgbaImage::from_pixel(12, 12, image::Rgba([80, 150, 200, 255]));
        let points = [[1., 1.], [11., 1.], [11., 11.], [1., 11.]];
        let hard = crop(&image, &points).unwrap();
        assert_eq!(crop_with_feather(&image, &points, 0).unwrap(), hard);

        let feathered = crop_with_feather(&image, &points, 2).unwrap();
        assert_eq!(feathered.dimensions(), hard.dimensions());
        assert_eq!(feathered.get_pixel(0, 5)[3], 0);
        assert!(feathered.get_pixel(1, 5)[3] > 0);
        assert!(feathered.get_pixel(1, 5)[3] < 255);
        assert_eq!(feathered.get_pixel(5, 5)[3], 255);
        assert_eq!(&feathered.get_pixel(1, 5).0[..3], &[80, 150, 200]);
        assert!(crop_with_feather(&image, &points, MAX_FEATHER_RADIUS + 1).is_err());
    }

    #[test]
    fn feather_radius_is_restored_by_selection_undo_and_redo() {
        let ctx = egui::Context::default();
        let source = RgbaImage::from_pixel(16, 16, image::Rgba([80, 150, 200, 255]));
        let points = vec![[1., 1.], [15., 1.], [15., 15.], [1., 15.]];
        let feathered = crop_with_feather(&source, &points, 2).unwrap();
        let mut state = State {
            source: Some(Arc::new(source.clone())),
            output: Some(Arc::new(feathered.clone())),
            gesture: Gesture {
                points: points.clone(),
                ..Default::default()
            },
            feather_radius: 2,
            feather_draft: 2,
            undo_selections: vec![SelectionSnapshot {
                points: points.clone(),
                freehand: true,
                feather_radius: 0,
                redactions: Vec::new(),
            }],
            ..Default::default()
        };

        let wait_for_output = |state: &mut State| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while state.pending.is_some() && std::time::Instant::now() < deadline {
                state.poll(&ctx);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            assert!(
                state.pending.is_none(),
                "feather history worker did not finish"
            );
        };

        state.restore_selection(&ctx, true);
        wait_for_output(&mut state);
        assert_eq!(state.feather_radius, 0);
        assert_eq!(state.feather_draft, 0);
        assert_eq!(
            state.output.as_deref().unwrap(),
            &crop(&source, &points).unwrap()
        );

        state.restore_selection(&ctx, false);
        wait_for_output(&mut state);
        assert_eq!(state.feather_radius, 2);
        assert_eq!(state.feather_draft, 2);
        assert_eq!(state.output.as_deref().unwrap(), &feathered);
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
