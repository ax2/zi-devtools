//! Bound output growth before writing, independent of codec-internal scratch memory.
use anyhow::{Result, bail};
use image::{DynamicImage, ImageFormat};
use std::io::{self, Seek, SeekFrom, Write};
pub(super) struct Output {
    bytes: Vec<u8>,
    position: usize,
    limit: usize,
    failed: bool,
    reservation: Option<super::memory::Reservation>,
}
impl Output {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            position: 0,
            limit,
            failed: false,
            reservation: None,
        }
    }
    fn budgeted(limit: usize, pool: &super::memory::Pool) -> Result<Self> {
        let mut value = Self::new(limit);
        value.reservation = Some(pool.reserve(0).map_err(anyhow::Error::msg)?);
        Ok(value)
    }
    fn reject(&mut self) -> io::Error {
        self.failed = true;
        io::Error::new(
            io::ErrorKind::InvalidData,
            "图片编码输出超限，未生成可保存结果",
        )
    }
    pub(super) fn into_bytes(self) -> io::Result<Vec<u8>> {
        if self.failed || self.reservation.is_some() {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "图片编码输出失败，未生成可保存结果",
            ))
        } else {
            Ok(self.bytes)
        }
    }
}
impl Write for Output {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.failed {
            return Err(self.reject());
        }
        let Some(end) = self
            .position
            .checked_add(buffer.len())
            .filter(|n| *n <= self.limit)
        else {
            return Err(self.reject());
        };
        if buffer.is_empty() {
            return Ok(0);
        }
        if end > self.bytes.capacity() {
            let mut target = self
                .bytes
                .capacity()
                .max(4096)
                .saturating_mul(2)
                .min(self.limit)
                .max(end);
            if let Some(reservation) = &mut self.reservation {
                if reservation.grow(target).is_err() {
                    target = end;
                    if let Err(message) = reservation.grow(target) {
                        self.failed = true;
                        return Err(io::Error::new(io::ErrorKind::InvalidData, message));
                    }
                }
            }
            if self
                .bytes
                .try_reserve_exact(target - self.bytes.len())
                .is_err()
            {
                return Err(self.reject());
            }
        }
        if end > self.bytes.len() {
            self.bytes.resize(end, 0)
        }
        self.bytes[self.position..end].copy_from_slice(buffer);
        self.position = end;
        Ok(buffer.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.failed {
            Err(self.reject())
        } else {
            Ok(())
        }
    }
}
impl Seek for Output {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        if self.failed {
            return Err(self.reject());
        }
        let next = match position {
            SeekFrom::Start(v) => i128::from(v),
            SeekFrom::End(v) => self.bytes.len() as i128 + i128::from(v),
            SeekFrom::Current(v) => self.position as i128 + i128::from(v),
        };
        if next < 0 || next > self.limit as i128 {
            return Err(self.reject());
        }
        self.position = next as usize;
        Ok(self.position as u64)
    }
}
pub(super) fn encode(
    image: &DynamicImage,
    format: ImageFormat,
    quality: u8,
    limit: usize,
) -> Result<Vec<u8>> {
    let mut output = Output::new(limit);
    match format {
        ImageFormat::Jpeg => {
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, quality.clamp(35, 95))
                .encode_image(image)?
        }
        ImageFormat::Png | ImageFormat::WebP => image.write_to(&mut output, format)?,
        _ => bail!("不支持该图片格式"),
    }
    Ok(output.into_bytes()?)
}
pub(super) fn encode_tracked(
    image: &DynamicImage,
    format: ImageFormat,
    quality: u8,
    limit: usize,
    pool: &super::memory::Pool,
) -> Result<std::sync::Arc<Vec<u8>>> {
    let mut output = Output::budgeted(limit, pool)?;
    match format {
        ImageFormat::Jpeg => {
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, quality.clamp(35, 95))
                .encode_image(image)?
        }
        ImageFormat::Png | ImageFormat::WebP => image.write_to(&mut output, format)?,
        _ => bail!("不支持该图片格式"),
    }
    if output.failed {
        bail!("图片输出失败")
    }
    let bytes = std::sync::Arc::new(output.bytes);
    output
        .reservation
        .take()
        .ok_or_else(|| anyhow::anyhow!("缺少编码预留"))?
        .promote(&bytes, bytes.capacity())
        .map_err(anyhow::Error::msg)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_growth_before_allocation_and_never_returns_partial_success() {
        let mut output = Output::new(16);
        assert!(output.write_all(&[0; 17]).is_err());
        assert_eq!(output.bytes.capacity(), 0);
        assert!(output.into_bytes().is_err());
        let mut output = Output::new(8);
        output.write_all(b"keep").unwrap();
        let old = output.bytes.clone();
        let cap = output.bytes.capacity();
        assert!(output.write_all(b"oversized").is_err());
        assert_eq!(output.bytes, old);
        assert_eq!(output.bytes.capacity(), cap);
        assert!(output.into_bytes().is_err());
    }
    #[test]
    fn seek_overwrite_zero_fill_and_boundaries_are_checked_without_growing_on_seek() {
        let mut output = Output::new(8);
        output.seek(SeekFrom::Start(7)).unwrap();
        assert_eq!(output.bytes.capacity(), 0);
        output.write_all(b"x").unwrap();
        output.seek(SeekFrom::Start(0)).unwrap();
        output.write_all(b"a").unwrap();
        assert_eq!(output.bytes, b"a\0\0\0\0\0\0x");
        assert!(output.bytes.capacity() <= 8);
        assert!(output.seek(SeekFrom::End(1)).is_err());
        assert_eq!(output.bytes.len(), 8);
        assert!(output.into_bytes().is_err());
        let mut negative = Output::new(8);
        assert!(negative.seek(SeekFrom::Current(-1)).is_err());
        assert!(negative.into_bytes().is_err());
    }
    #[test]
    fn actual_codecs_match_unbounded_bytes_accept_exact_size_and_refuse_smaller_limit() {
        let source = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            12,
            8,
            image::Rgba([200, 40, 10, 130]),
        ));
        let pixels = source.to_rgba8();
        for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP] {
            let mut baseline = std::io::Cursor::new(Vec::new());
            if format == ImageFormat::Jpeg {
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut baseline, 80)
                    .encode_image(&source)
                    .unwrap();
            } else {
                source.write_to(&mut baseline, format).unwrap();
            }
            let bytes = baseline.into_inner();
            let bounded = encode(&source, format, 80, bytes.len()).unwrap();
            assert_eq!(bounded, bytes);
            let decoded = image::load_from_memory_with_format(&bounded, format).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (12, 8));
            if format != ImageFormat::Jpeg {
                assert_eq!(decoded.to_rgba8(), pixels)
            }
            assert!(encode(&source, format, 80, bytes.len() - 1).is_err());
            assert_eq!(source.to_rgba8(), pixels);
        }
    }
}
