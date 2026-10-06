//! Canonical PNG payloads and checked common packed Windows DIB formats.
use anyhow::{Result, ensure};
use image::{DynamicImage, ImageFormat, ImageReader, Rgba, RgbaImage};
use sha2::{Digest, Sha256};
use std::{io::Cursor, sync::Arc};
pub(super) const RAW_LIMIT: usize = 32 * 1024 * 1024;
const PNG_LIMIT: usize = 16 * 1024 * 1024;
const PIXELS: u64 = 4_000_000;
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Picture {
    #[serde(with = "png_bytes")]
    pub png: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub sha256: String,
}
mod png_bytes {
    use super::*;
    use base64::{Engine, engine::general_purpose::STANDARD};
    pub fn serialize<S: serde::Serializer>(bytes: &Arc<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(bytes.as_slice()))
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Arc<Vec<u8>>, D::Error> {
        let text = <String as serde::Deserialize>::deserialize(d)?;
        if text.len() > PNG_LIMIT.div_ceil(3) * 4 {
            return Err(serde::de::Error::custom("图片编码超限"));
        }
        let bytes = STANDARD.decode(text).map_err(serde::de::Error::custom)?;
        if bytes.len() > PNG_LIMIT {
            return Err(serde::de::Error::custom("图片超限"));
        }
        Ok(Arc::new(bytes))
    }
}
fn dimensions(w: u32, h: u32) -> Result<()> {
    ensure!(
        w > 0 && h > 0 && w <= 12000 && h <= 12000 && u64::from(w) * u64::from(h) <= PIXELS,
        "剪贴板图片超过400万像素或尺寸无效"
    );
    Ok(())
}
fn decode_png(bytes: &[u8]) -> Result<DynamicImage> {
    ensure!(
        !bytes.is_empty() && bytes.len() <= PNG_LIMIT,
        "PNG数据超过16MiB"
    );
    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(12000);
    limits.max_image_height = Some(12000);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits.clone());
    let (w, h) = reader.into_dimensions()?;
    dimensions(w, h)?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
    reader.limits(limits);
    Ok(reader.decode()?)
}
impl Picture {
    pub fn from_png(bytes: &[u8]) -> Result<Self> {
        Self::from_image(decode_png(bytes)?)
    }
    pub fn from_image(image: DynamicImage) -> Result<Self> {
        dimensions(image.width(), image.height())?;
        let mut png = Cursor::new(Vec::new());
        image.write_to(&mut png, ImageFormat::Png)?;
        let png = png.into_inner();
        ensure!(png.len() <= PNG_LIMIT, "规范PNG超过16MiB");
        let value = Self {
            width: image.width(),
            height: image.height(),
            sha256: format!("{:x}", Sha256::digest(&png)),
            png: Arc::new(png),
        };
        ensure!(value.cost() <= super::BYTE_LIMIT, "单张图片超过历史容量");
        Ok(value)
    }
    pub fn cost(&self) -> usize {
        let decoded = u64::from(self.width)
            .saturating_mul(u64::from(self.height))
            .saturating_mul(4)
            .min(usize::MAX as u64) as usize;
        self.png.len().saturating_add(decoded)
    }
    pub fn validate(&self) -> Result<()> {
        dimensions(self.width, self.height)?;
        ensure!(
            self.cost() <= super::BYTE_LIMIT
                && self.sha256 == format!("{:x}", Sha256::digest(self.png.as_slice())),
            "图片容量或摘要无效"
        );
        let image = decode_png(&self.png)?;
        ensure!(
            (image.width(), image.height()) == (self.width, self.height),
            "图片声明尺寸与PNG不一致"
        );
        Ok(())
    }
    pub fn image(&self) -> Result<DynamicImage> {
        decode_png(&self.png)
    }
    pub fn thumbnail(&self) -> Result<eframe::egui::ColorImage> {
        let rgba = self.image()?.thumbnail(480, 240).to_rgba8();
        Ok(eframe::egui::ColorImage::from_rgba_unmultiplied(
            [rgba.width() as usize, rgba.height() as usize],
            rgba.as_raw(),
        ))
    }
}
fn u32_at(data: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        data.get(offset..offset + 4)
            .ok_or_else(|| anyhow::anyhow!("位图头截断"))?
            .try_into()?,
    ))
}
fn u16_at(data: &[u8], offset: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        data.get(offset..offset + 2)
            .ok_or_else(|| anyhow::anyhow!("位图头截断"))?
            .try_into()?,
    ))
}
pub(super) fn from_dib(data: &[u8]) -> Result<Picture> {
    ensure!(data.len() <= RAW_LIMIT, "位图数据超限");
    let header = u32_at(data, 0)? as usize;
    ensure!(
        matches!(header, 40 | 108 | 124) && data.len() >= header,
        "不支持的位图头"
    );
    let w = u32_at(data, 4)? as i32;
    let h = u32_at(data, 8)? as i32;
    ensure!(w > 0 && h != i32::MIN, "位图尺寸无效");
    dimensions(w as u32, h.unsigned_abs())?;
    let bpp = u16_at(data, 14)?;
    let compression = u32_at(data, 16)?;
    ensure!(
        u16_at(data, 12)? == 1 && matches!(bpp, 24 | 32) && matches!(compression, 0 | 3),
        "只支持24/32位RGB或32位掩码位图"
    );
    ensure!(compression == 0 || bpp == 32, "掩码位图需要32位");
    let mut offset = header;
    let mut masks = [0x00ff0000u32, 0x0000ff00, 0x000000ff, 0];
    if compression == 3 {
        let pos = if header == 40 {
            offset += 12;
            40
        } else {
            40
        };
        masks = [
            u32_at(data, pos)?,
            u32_at(data, pos + 4)?,
            u32_at(data, pos + 8)?,
            if header >= 108 { u32_at(data, 52)? } else { 0 },
        ];
        ensure!(
            masks[..3] == [0x00ff0000, 0x0000ff00, 0x000000ff]
                && matches!(masks[3], 0 | 0xff000000),
            "位图通道掩码不支持"
        );
    }
    if header == 124 {
        ensure!(
            u32_at(data, 112)? == 0 && u32_at(data, 116)? == 0,
            "暂不支持内嵌或链接ICC配置位图"
        );
    }
    let colors = u32_at(data, 32)? as usize;
    ensure!(colors <= 256, "颜色表超限");
    offset = offset
        .checked_add(colors * 4)
        .ok_or_else(|| anyhow::anyhow!("位图偏移溢出"))?;
    let stride = (w as usize * bpp as usize).div_ceil(32) * 4;
    let height = h.unsigned_abs() as usize;
    let size = stride
        .checked_mul(height)
        .ok_or_else(|| anyhow::anyhow!("位图大小溢出"))?;
    ensure!(
        offset
            .checked_add(size)
            .is_some_and(|end| end <= data.len()),
        "位图像素截断"
    );
    let declared = u32_at(data, 20)? as usize;
    ensure!(declared == 0 || declared == size, "位图像素大小不一致");
    let mut rgba = RgbaImage::new(w as u32, height as u32);
    for y in 0..height {
        let row = if h < 0 { y } else { height - 1 - y };
        for x in 0..w as usize {
            let i = offset + row * stride + x * (bpp as usize / 8);
            rgba.put_pixel(
                x as u32,
                y as u32,
                Rgba([
                    data[i + 2],
                    data[i + 1],
                    data[i],
                    if masks[3] != 0 { data[i + 3] } else { 255 },
                ]),
            );
        }
    }
    Picture::from_image(DynamicImage::ImageRgba8(rgba))
}
pub(super) fn to_dib(value: &Picture) -> Result<Vec<u8>> {
    let rgba = value.image()?.to_rgba8();
    let mut data = vec![0u8; 124 + rgba.as_raw().len()];
    for (at, value) in [
        (0, 124),
        (4, rgba.width()),
        (8, (-(rgba.height() as i32)) as u32),
        (16, 3),
        (20, rgba.as_raw().len() as u32),
        (40, 0x00ff0000),
        (44, 0x0000ff00),
        (48, 0x000000ff),
        (52, 0xff000000),
        (56, 0x73524742),
        (108, 4),
    ] {
        data[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    data[12..14].copy_from_slice(&1u16.to_le_bytes());
    data[14..16].copy_from_slice(&32u16.to_le_bytes());
    for (pixel, output) in rgba
        .as_raw()
        .chunks_exact(4)
        .zip(data[124..].chunks_exact_mut(4))
    {
        output.copy_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    Ok(data)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn forged_maximum_dimensions_are_rejected_without_budget_overflow() {
        let mut picture = Picture::from_image(DynamicImage::new_rgba8(1, 1)).unwrap();
        picture.width = u32::MAX;
        picture.height = u32::MAX;
        assert_eq!(picture.cost(), usize::MAX);
        assert!(picture.validate().is_err());
        let mut history = super::super::History::default();
        assert!(
            history
                .insert_image(Arc::new(picture), "fixture".into())
                .is_err()
        );
        assert!(history.entries.is_empty());
    }
    #[test]
    fn transparent_png_and_v5_roundtrip_and_invalid_headers() {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_fn(3, 2, |x, y| {
            Rgba([x as u8, y as u8, 120, (x * 70) as u8])
        }));
        let picture = Picture::from_image(image.clone()).unwrap();
        picture.validate().unwrap();
        assert_eq!(
            Picture::from_png(&picture.png).unwrap().sha256,
            picture.sha256
        );
        let dib = to_dib(&picture).unwrap();
        assert_eq!(
            from_dib(&dib).unwrap().image().unwrap().to_rgba8(),
            image.to_rgba8()
        );
        for length in [0, 16, 123, dib.len() - 1] {
            assert!(from_dib(&dib[..length]).is_err());
        }
        let mut bad = dib.clone();
        bad[48..52].copy_from_slice(&1u32.to_le_bytes());
        assert!(from_dib(&bad).is_err());
        bad = dib.clone();
        bad[112..116].copy_from_slice(&124u32.to_le_bytes());
        assert!(from_dib(&bad).is_err());
        let mut wrong = picture;
        wrong.width += 1;
        assert!(wrong.validate().is_err());
    }
    #[test]
    fn padded_bottom_up_24_and_rgb32_ignore_undefined_alpha() {
        let mut dib = vec![0u8; 40 + 8 * 2];
        for (at, value) in [(0, 40), (4, 2), (8, 2)] {
            dib[at..at + 4].copy_from_slice(&(value as u32).to_le_bytes());
        }
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&24u16.to_le_bytes());
        dib[40..46].copy_from_slice(&[30, 20, 10, 60, 50, 40]);
        dib[48..54].copy_from_slice(&[90, 80, 70, 120, 110, 100]);
        let image = from_dib(&dib).unwrap().image().unwrap().to_rgba8();
        assert_eq!(image.get_pixel(0, 0).0, [70, 80, 90, 255]);
        assert_eq!(image.get_pixel(1, 1).0, [40, 50, 60, 255]);
        let mut raw = to_dib(&Picture::from_image(DynamicImage::new_rgba8(1, 1)).unwrap()).unwrap();
        raw[16..20].fill(0);
        raw[124..128].copy_from_slice(&[3, 2, 1, 0]);
        assert_eq!(
            from_dib(&raw)
                .unwrap()
                .image()
                .unwrap()
                .to_rgba8()
                .get_pixel(0, 0)
                .0,
            [1, 2, 3, 255]
        );
        raw[4..8].copy_from_slice(&12001u32.to_le_bytes());
        assert!(from_dib(&raw).is_err());
    }
}
