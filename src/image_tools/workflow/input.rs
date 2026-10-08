//! Selected-file input: reserve managed bytes before reading and decoding pixels.
use super::*;
use image::ImageDecoder;
use std::io::Read;

pub(in crate::image_tools) struct SelectedBytes {
    bytes: Vec<u8>,
    file: std::fs::File,
    material: crate::material_files::FileMaterial,
    _reservation: super::super::memory::Reservation,
}
impl SelectedBytes {
    pub(in crate::image_tools) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub(in crate::image_tools) fn verify(&self) -> Result<()> {
        self.material.verify(&self.file, MAX_INPUT_BYTES as usize)
    }
}

pub(in crate::image_tools) fn load(
    material: &crate::material_files::FileMaterial,
    cancel: &AtomicBool,
    memory: &super::super::memory::Pool,
) -> Result<Arc<DynamicImage>> {
    let selected = read_selected(material, cancel, memory)?;
    let image = decode(selected.bytes(), cancel, memory)?;
    selected.verify()?;
    Ok(image)
}

pub(in crate::image_tools) fn read_selected(
    material: &crate::material_files::FileMaterial,
    cancel: &AtomicBool,
    memory: &super::super::memory::Pool,
) -> Result<SelectedBytes> {
    ensure!(!cancel.load(Ordering::Relaxed), "读取已取消");
    let size = usize::try_from(material.bytes())?;
    ensure!(
        size > 0 && size <= MAX_INPUT_BYTES as usize,
        "输入图片需为非空且不超过32MiB"
    );
    let input_guard = memory.reserve(size).map_err(anyhow::Error::msg)?;
    let mut file = material.open(MAX_INPUT_BYTES as usize)?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(size)?;
    ensure!(bytes.capacity() <= size, "输入缓冲分配超出预留");
    bytes.resize(size, 0);
    for part in bytes.chunks_mut(65536) {
        ensure!(!cancel.load(Ordering::Relaxed), "读取已取消");
        file.read_exact(part)?;
    }
    material.verify(&file, MAX_INPUT_BYTES as usize)?;
    ensure!(!cancel.load(Ordering::Relaxed), "读取已取消");
    Ok(SelectedBytes {
        bytes,
        file,
        material: material.clone(),
        _reservation: input_guard,
    })
}

pub(in crate::image_tools) fn decode(
    bytes: &[u8],
    cancel: &AtomicBool,
    memory: &super::super::memory::Pool,
) -> Result<Arc<DynamicImage>> {
    ensure!(!cancel.load(Ordering::Relaxed), "读取已取消");
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_OUTPUT_BYTES,
        "编码输入超限"
    );
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    ensure!(
        matches!(
            reader.format(),
            Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP)
        ),
        "仅支持PNG/JPEG/WebP"
    );
    reader.limits(image_limits());
    let decoder = reader.into_decoder()?;
    let (width, height) = decoder.dimensions();
    ensure!(
        width > 0
            && height > 0
            && width <= 12000
            && height <= 12000
            && u64::from(width) * u64::from(height) <= MAX_PIXELS,
        "输入图片尺寸超限"
    );
    let pixel_bytes = usize::try_from(decoder.total_bytes())?;
    ensure!(pixel_bytes <= 192 * 1024 * 1024, "输入图片解码超限");
    let pixels = memory.reserve(pixel_bytes).map_err(anyhow::Error::msg)?;
    ensure!(!cancel.load(Ordering::Relaxed), "读取已取消");
    let image = Arc::new(DynamicImage::from_decoder(decoder)?);
    ensure!(!cancel.load(Ordering::Relaxed), "读取已取消");
    pixels
        .promote(&image, super::super::memory::pixels(&image))
        .map_err(anyhow::Error::msg)?;
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new(bytes: &[u8]) -> Self {
            let folder =
                std::env::temp_dir().join(format!("zi-image-budget-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&folder).unwrap();
            let path = folder.join("input.image");
            std::fs::write(&path, bytes).unwrap();
            Self(path)
        }
        fn material(&self) -> crate::material_files::FileMaterial {
            crate::material_files::FileMaterial::selected(&self.0, MAX_INPUT_BYTES as usize)
                .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
        }
    }
    #[test]
    fn actual_formats_and_sixteen_bit_pixels_fit_exact_joint_input_budget() {
        let sources = [
            DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                10,
                5,
                image::Rgba([200, 40, 20, 150]),
            )),
            DynamicImage::ImageRgba16(image::ImageBuffer::from_pixel(
                10,
                5,
                image::Rgba([50000u16, 10000, 3000, 40000]),
            )),
        ];
        for (source, formats) in [
            (
                &sources[0],
                vec![ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP],
            ),
            (&sources[1], vec![ImageFormat::Png]),
        ] {
            for format in formats {
                let bytes =
                    super::super::super::encoding::encode(source, format, 80, MAX_OUTPUT_BYTES)
                        .unwrap();
                let expected = image::load_from_memory(&bytes).unwrap();
                let pixels = super::super::super::memory::pixels(&expected);
                let total = bytes.len() + pixels;
                let fixture = Fixture::new(&bytes);
                let material = fixture.material();
                let too_small = super::super::super::memory::Pool::new(total - 1);
                assert!(load(&material, &AtomicBool::new(false), &too_small).is_err());
                assert_eq!(too_small.snapshot().unwrap(), (0, 0, total - 1));
                let exact = super::super::super::memory::Pool::new(total);
                let actual = load(&material, &AtomicBool::new(false), &exact).unwrap();
                assert_eq!(actual.as_bytes(), expected.as_bytes());
                assert_eq!(actual.color(), expected.color());
                assert_eq!(exact.snapshot().unwrap(), (pixels, 0, total));
                drop(actual);
                assert_eq!(exact.snapshot().unwrap(), (0, 0, total));
                assert_eq!(std::fs::read(&fixture.0).unwrap(), bytes);
            }
        }
    }
    #[test]
    fn real_input_job_keeps_old_work_on_budget_error_and_replaces_only_after_success() {
        let image = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            10,
            5,
            image::Rgba([50, 70, 80, 150]),
        ));
        let bytes =
            super::super::super::encoding::encode(&image, ImageFormat::Png, 80, MAX_OUTPUT_BYTES)
                .unwrap();
        let fixture = Fixture::new(&bytes);
        let pool = super::super::super::memory::Pool::new(bytes.len() + 200);
        let mut state = State::with_memory(pool.clone());
        let old = Arc::new(DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([1, 2, 3, 4]),
        )));
        pool.share(&old, 4).unwrap();
        state.source = Some(old.clone());
        state.definition.steps = vec![Step::Info { version: 1 }];
        state.run = Some(
            execute_budgeted(
                &state.definition,
                old.clone(),
                &AtomicBool::new(false),
                &pool,
            )
            .unwrap(),
        );
        let ctx = egui::Context::default();
        let settle = |state: &mut State| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while state.busy() {
                state.poll(&ctx);
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
        };
        let material = fixture.material();
        let worker_pool = pool.clone();
        state.launch(&ctx, Kind::Input, move |cancel| {
            Ok(Reply::Input(
                load(&material, cancel, &worker_pool)?,
                "fixture".into(),
            ))
        });
        settle(&mut state);
        assert!(Arc::ptr_eq(state.source.as_ref().unwrap(), &old));
        assert!(state.run.is_some());
        assert_eq!(pool.snapshot().unwrap().0, 4);
        pool.set_limit(bytes.len() + 204).unwrap();
        let material = fixture.material();
        let worker_pool = pool.clone();
        state.launch(&ctx, Kind::Input, move |cancel| {
            Ok(Reply::Input(
                load(&material, cancel, &worker_pool)?,
                "fixture".into(),
            ))
        });
        settle(&mut state);
        assert_eq!(state.source.as_ref().unwrap().as_bytes(), image.as_bytes());
        assert!(state.run.is_none());
        assert_eq!(state.definition.steps, vec![Step::Info { version: 1 }]);
        assert_eq!(pool.snapshot().unwrap().0, 204);
        drop(state);
        drop(old);
        assert_eq!(pool.snapshot().unwrap().0, 0);
    }
    #[test]
    fn cancellation_bad_format_and_changed_selected_file_refund_without_touching_old_source() {
        let pool = super::super::super::memory::Pool::new(1024);
        let old = Arc::new(DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([1, 2, 3, 4]),
        )));
        pool.share(&old, 4).unwrap();
        let fixture = Fixture::new(b"not an image");
        let material = fixture.material();
        assert!(load(&material, &AtomicBool::new(true), &pool).is_err());
        assert!(load(&material, &AtomicBool::new(false), &pool).is_err());
        std::fs::write(&fixture.0, b"changed").unwrap();
        assert!(load(&material, &AtomicBool::new(false), &pool).is_err());
        assert_eq!(pool.snapshot().unwrap(), (4, 0, 1024));
        assert_eq!(old.as_bytes(), [1, 2, 3, 4]);
    }
}
