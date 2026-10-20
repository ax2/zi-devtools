//! Portable, UI-free image operations shared by the desktop app and future adapters.
use anyhow::{Result, ensure};
use image::{DynamicImage, ImageFormat, ImageReader, Limits, imageops::FilterType};
use std::io::{Cursor, Seek, Write};

pub const MAX_EDGE: u32 = 12_000;
pub const MAX_PIXELS: u64 = 16_000_000;
pub const MAX_DECODED_BYTES: usize = 192 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CropRegion {
    /// Horizontal origin in ten-thousandths of the input width.
    pub x: u32,
    /// Vertical origin in ten-thousandths of the input height.
    pub y: u32,
    /// Width in ten-thousandths of the input width.
    pub width: u32,
    /// Height in ten-thousandths of the input height.
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CropBounds {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

pub fn validate_dimensions(width: u32, height: u32) -> Result<()> {
    validate_dimensions_with_limits(width, height, MAX_EDGE, MAX_PIXELS)
}

/// Validates encoded image dimensions against an adapter's narrower profile
/// before decoding allocates the pixel buffer.
pub fn validate_dimensions_with_limits(
    width: u32,
    height: u32,
    max_edge: u32,
    max_pixels: u64,
) -> Result<()> {
    let pixels = u64::from(width) * u64::from(height);
    ensure!(
        max_edge > 0
            && max_pixels > 0
            && width > 0
            && height > 0
            && width <= max_edge
            && height <= max_edge
            && pixels <= max_pixels,
        "图片尺寸超出限制"
    );
    Ok(())
}

/// Reads dimensions from the encoded header and checks the core safety ceiling
/// without decoding pixel data. Resource adapters can apply tighter limits next.
pub fn encoded_dimensions(bytes: &[u8]) -> Result<(u32, u32)> {
    ensure!(!bytes.is_empty(), "图片数据为空");
    let dimensions = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()?
        .into_dimensions()?;
    validate_dimensions(dimensions.0, dimensions.1)?;
    Ok(dimensions)
}

/// Decode only after reading and validating the image header. This keeps oversized
/// dimensions from allocating their full decoded pixel buffer in adapters.
pub fn decode(bytes: &[u8]) -> Result<DynamicImage> {
    encoded_dimensions(bytes)?;

    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_EDGE);
    limits.max_image_height = Some(MAX_EDGE);
    limits.max_alloc = Some(MAX_DECODED_BYTES as u64);
    reader.limits(limits);
    let image = reader.decode()?;
    validate_image(&image)?;
    Ok(image)
}

pub fn validate_image(image: &DynamicImage) -> Result<()> {
    validate_dimensions(image.width(), image.height())?;
    ensure!(
        image.as_bytes().len() <= MAX_DECODED_BYTES,
        "图片解码内存超限"
    );
    Ok(())
}

pub fn crop_bounds(width: u32, height: u32, region: CropRegion) -> Result<CropBounds> {
    ensure!(
        width > 0
            && height > 0
            && region.width > 0
            && region.height > 0
            && u64::from(region.x) + u64::from(region.width) <= 10_000
            && u64::from(region.y) + u64::from(region.height) <= 10_000,
        "裁剪比例必须在0..10000内且不能为空"
    );
    let w = u64::from(width);
    let h = u64::from(height);
    let x = w * u64::from(region.x) / 10_000;
    let y = h * u64::from(region.y) / 10_000;
    let right = (w * (u64::from(region.x) + u64::from(region.width))).div_ceil(10_000);
    let bottom = (h * (u64::from(region.y) + u64::from(region.height))).div_ceil(10_000);
    ensure!(right > x && bottom > y, "裁剪区域为空");
    Ok(CropBounds {
        x: x as u32,
        y: y as u32,
        width: (right - x) as u32,
        height: (bottom - y) as u32,
    })
}

pub fn crop_percent(image: &DynamicImage, region: CropRegion) -> Result<DynamicImage> {
    validate_image(image)?;
    let bounds = crop_bounds(image.width(), image.height(), region)?;
    let output = image.crop_imm(bounds.x, bounds.y, bounds.width, bounds.height);
    validate_image(&output)?;
    Ok(output)
}

pub fn resize_dimensions(width: u32, height: u32, max_width: u32) -> Result<(u32, u32)> {
    ensure!(
        width > 0 && height > 0 && (1..=MAX_EDGE).contains(&max_width),
        "缩放尺寸超出限制"
    );
    let output_width = width.min(max_width);
    let output_height = if output_width == width {
        height
    } else {
        ((u64::from(height) * u64::from(output_width) + u64::from(width) / 2) / u64::from(width))
            .max(1) as u32
    };
    ensure!(
        u64::from(output_width) * u64::from(output_height) <= MAX_PIXELS,
        "缩放结果超出像素限制"
    );
    Ok((output_width, output_height))
}

pub fn resize_to_width(image: &DynamicImage, max_width: u32) -> Result<DynamicImage> {
    validate_image(image)?;
    let (width, height) = resize_dimensions(image.width(), image.height(), max_width)?;
    let output = if width == image.width() {
        image.clone()
    } else {
        // Preserve the desktop Lanczos quality profile in the shared adapter.
        image.resize_exact(width, height, FilterType::Lanczos3)
    };
    validate_image(&output)?;
    Ok(output)
}

/// Encode using the desktop and WASI adapters' bounded writer. The adapter owns
/// output allocation policy; this function owns format and codec behavior.
pub fn encode<W: Write + Seek>(
    image: &DynamicImage,
    format: ImageFormat,
    quality: u8,
    writer: &mut W,
) -> Result<()> {
    validate_image(image)?;
    match format {
        ImageFormat::Jpeg => {
            image::codecs::jpeg::JpegEncoder::new_with_quality(writer, quality.clamp(35, 95))
                .encode_image(image)?
        }
        ImageFormat::Png | ImageFormat::WebP => image.write_to(writer, format)?,
        _ => anyhow::bail!("不支持该图片格式"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn numbered() -> DynamicImage {
        DynamicImage::ImageRgba8(image::RgbaImage::from_fn(4, 3, |x, y| {
            Rgba([
                x as u8,
                y as u8,
                (x + y * 4) as u8,
                if x == 0 { 0 } else { 255 },
            ])
        }))
    }

    #[test]
    fn crop_uses_the_existing_floor_origin_and_ceil_far_edge_rule() {
        let bounds = crop_bounds(
            4,
            3,
            CropRegion {
                x: 2_500,
                y: 3_334,
                width: 5_000,
                height: 3_332,
            },
        )
        .unwrap();
        assert_eq!(
            bounds,
            CropBounds {
                x: 1,
                y: 1,
                width: 2,
                height: 1
            }
        );
        let cropped = crop_percent(
            &numbered(),
            CropRegion {
                x: 2_500,
                y: 3_334,
                width: 5_000,
                height: 3_332,
            },
        )
        .unwrap()
        .to_rgba8();
        assert_eq!((cropped.width(), cropped.height()), (2, 1));
        assert_eq!(cropped.get_pixel(0, 0), &Rgba([1, 1, 5, 255]));
    }

    #[test]
    fn crop_rejects_empty_or_out_of_range_regions() {
        for region in [
            CropRegion {
                x: 0,
                y: 0,
                width: 0,
                height: 10_000,
            },
            CropRegion {
                x: 9_999,
                y: 0,
                width: 2,
                height: 10_000,
            },
        ] {
            assert!(crop_bounds(4, 3, region).is_err());
        }
    }

    #[test]
    fn resize_preserves_aspect_and_does_not_upscale() {
        assert_eq!(resize_dimensions(400, 201, 100).unwrap(), (100, 50));
        assert_eq!(resize_dimensions(4, 3, 100).unwrap(), (4, 3));
        let output = resize_to_width(&numbered(), 2).unwrap();
        assert_eq!((output.width(), output.height()), (2, 2));
    }

    #[test]
    fn encode_formats_write_decodable_images_through_the_bounded_writer() {
        let source = numbered();
        for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP] {
            let mut output = std::io::Cursor::new(Vec::new());
            encode(&source, format, 80, &mut output).unwrap();
            let bytes = output.into_inner();
            let decoded = image::load_from_memory_with_format(&bytes, format).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (4, 3));
        }
    }

    #[test]
    fn decode_supports_only_bounded_shared_formats() {
        let source = numbered();
        for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP] {
            let mut output = std::io::Cursor::new(Vec::new());
            encode(&source, format, 80, &mut output).unwrap();
            let decoded = decode(output.get_ref()).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (4, 3));
        }
        assert!(decode(&[]).is_err());
        assert!(validate_dimensions(MAX_EDGE + 1, 1).is_err());
        assert!(validate_dimensions(4_097, 4_097).is_err());
    }

    #[test]
    fn adapter_dimensions_can_be_narrower_than_the_shared_core() {
        const PROFILE_EDGE: u32 = 4096;
        const PROFILE_PIXELS: u64 = 16_000_000;
        assert!(validate_dimensions_with_limits(4000, 4000, PROFILE_EDGE, PROFILE_PIXELS).is_ok());
        assert!(validate_dimensions_with_limits(4096, 4096, PROFILE_EDGE, PROFILE_PIXELS).is_err());
        assert!(validate_dimensions_with_limits(5000, 2000, PROFILE_EDGE, PROFILE_PIXELS).is_err());
        assert!(validate_dimensions_with_limits(0, 10, PROFILE_EDGE, PROFILE_PIXELS).is_err());
    }
}
