//! Single-image workers use the same managed payload pool as workflow instances.
use super::*;
use std::sync::atomic::AtomicBool;

pub(super) fn load(path: &Path, memory: &memory::Pool) -> Result<Job> {
    let path = std::path::absolute(path)?;
    let material = crate::material_files::FileMaterial::selected(&path, MAX_INPUT_BYTES as usize)?;
    let image = workflow::input::load(&material, &AtomicBool::new(false), memory)?;
    Ok(Job::Loaded {
        preview: preview_image(&image),
        image,
        bytes: material.bytes(),
        path: material.path().to_path_buf(),
    })
}

pub(super) fn preview(
    source: Arc<DynamicImage>,
    width: u32,
    format: Format,
    quality: u8,
    memory: &memory::Pool,
) -> Result<Job> {
    let (encoded, width, height) = encode(source, width, format, quality, memory)?;
    let decoded = workflow::input::decode(&encoded, &AtomicBool::new(false), memory)?;
    Ok(Job::Preview {
        encoded,
        preview: preview_image(&decoded),
        width,
        height,
    })
}

pub(super) fn encode(
    source: Arc<DynamicImage>,
    width: u32,
    format: Format,
    quality: u8,
    memory: &memory::Pool,
) -> Result<(Arc<Vec<u8>>, u32, u32)> {
    ensure!(
        width >= 1 && width <= source.width(),
        "输出宽度超出原图范围"
    );
    ensure!(
        source.width() <= 12000
            && source.height() <= 12000
            && u64::from(source.width()) * u64::from(source.height()) <= MAX_PIXELS
            && memory::pixels(&source) <= 192 * 1024 * 1024,
        "输入图片尺寸超限"
    );
    memory
        .share(&source, memory::pixels(&source))
        .map_err(anyhow::Error::msg)?;
    let height = ((u64::from(source.height()) * u64::from(width) + u64::from(source.width()) / 2)
        / u64::from(source.width()))
    .max(1) as u32;
    let pixels = if width == source.width() {
        source
    } else {
        let guard = memory
            .reserve(
                width as usize * height as usize * usize::from(source.color().bytes_per_pixel()),
            )
            .map_err(anyhow::Error::msg)?;
        let pixels = Arc::new(source.resize_exact(width, height, FilterType::Lanczos3));
        guard
            .promote(&pixels, memory::pixels(&pixels))
            .map_err(anyhow::Error::msg)?;
        pixels
    };
    let encoded = encoding::encode_tracked(
        &pixels,
        format.image_format(),
        quality,
        MAX_OUTPUT_BYTES,
        memory,
    )?;
    Ok((encoded, width, height))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("zi-single-budget-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn settle(state: &mut State, ctx: &egui::Context) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while state.pending.is_some() {
            state.poll(ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
    #[test]
    fn actual_codecs_queued_output_and_workflow_share_one_pool_without_double_charging() {
        for format in Format::ALL {
            let mut state = State::default();
            let pool = state.workflow.memory_pool();
            pool.set_limit(10000).unwrap();
            let source = Arc::new(DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                10,
                5,
                image::Rgba([10, 30, 90, 180]),
            )));
            pool.share(&source, memory::pixels(&source)).unwrap();
            state.source = Some(source.clone());
            let prepared =
                relay::prepare(relay::Source::Image(source.clone()), vec![], "image-tools")
                    .unwrap();
            state.workflow.receive(&prepared).unwrap();
            assert_eq!(pool.snapshot().unwrap().0, 200);
            let worker_pool = pool.clone();
            let worker_source = source.clone();
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                tx.send(preview(worker_source, 10, format, 80, &worker_pool).unwrap())
                    .unwrap()
            })
            .join()
            .unwrap();
            let retained = pool.snapshot().unwrap().0;
            assert!(retained > 200);
            assert_eq!(pool.snapshot().unwrap().1, 0);
            pool.set_limit(retained).unwrap();
            assert!(preview(source.clone(), 10, format, 80, &pool).is_err());
            let copy = relay::prepare(
                relay::Source::Image(Arc::new((*source).clone())),
                vec![],
                "image-tools",
            )
            .unwrap();
            assert!(
                state
                    .workflow
                    .receive_at(&workflow::Destination::New, None, false, &copy)
                    .is_err()
            );
            assert_eq!(pool.snapshot().unwrap().0, retained);
            let Job::Preview {
                encoded,
                width,
                height,
                ..
            } = rx.recv().unwrap()
            else {
                unreachable!()
            };
            assert_eq!((width, height), (10, 5));
            assert_eq!(
                image::load_from_memory(&encoded).unwrap().dimensions(),
                (10, 5)
            );
            assert_eq!(&source.as_bytes()[..4], &[10, 30, 90, 180]);
            drop(encoded);
            assert_eq!(pool.snapshot().unwrap().0, 200);
            drop(prepared);
            drop(state);
            drop(source);
            assert_eq!(pool.snapshot().unwrap().0, 0);
        }
    }
    #[test]
    fn real_workers_keep_old_results_on_error_and_save_uses_loaded_path_not_new_draft() {
        let fixture = Fixture::new();
        let original = fixture.0.join("original.png");
        DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            10,
            5,
            image::Rgba([70, 20, 150, 100]),
        ))
        .save(&original)
        .unwrap();
        let original_bytes = fs::read(&original).unwrap();
        let ctx = egui::Context::default();
        let mut state = State {
            input: original.to_string_lossy().into_owned(),
            format: Format::WebP,
            jpeg_quality: 80,
            ..Default::default()
        };
        state.start_load();
        settle(&mut state, &ctx);
        assert!(!state.error);
        let old_source = state.source.clone().unwrap();
        let old_path = state.loaded_source_path.clone().unwrap();
        state.start_preview();
        settle(&mut state, &ctx);
        assert!(!state.error);
        let old_encoded = state.encoded.clone().unwrap();
        let pool = state.workflow.memory_pool();
        pool.set_limit(pool.snapshot().unwrap().0).unwrap();
        state.start_preview();
        settle(&mut state, &ctx);
        assert!(state.error);
        assert!(Arc::ptr_eq(state.encoded.as_ref().unwrap(), &old_encoded));
        let draft = fixture.0.join("draft.webp");
        state.input = draft.to_string_lossy().into_owned();
        state.start_load();
        settle(&mut state, &ctx);
        assert!(state.error);
        assert!(Arc::ptr_eq(state.source.as_ref().unwrap(), &old_source));
        assert!(Arc::ptr_eq(state.encoded.as_ref().unwrap(), &old_encoded));
        assert_eq!(state.loaded_source_path.as_ref().unwrap(), &old_path);
        state.output = draft.to_string_lossy().into_owned();
        state.save();
        assert!(!state.error, "{}", state.message);
        assert_eq!(fs::read(&draft).unwrap(), *old_encoded);
        assert_eq!(fs::read(&original).unwrap(), original_bytes);
        let new_path = fixture.0.join("next.png");
        DynamicImage::new_rgba8(2, 3).save(&new_path).unwrap();
        pool.set_limit(20000).unwrap();
        state.input = new_path.to_string_lossy().into_owned();
        state.start_load();
        settle(&mut state, &ctx);
        assert!(!state.error);
        assert_eq!(state.source.as_ref().unwrap().dimensions(), (2, 3));
        assert!(state.encoded.is_none());
        assert_eq!(
            state.loaded_source_path.as_ref().unwrap(),
            &new_path.canonicalize().unwrap()
        );
        drop(old_encoded);
        drop(old_source);
        assert_eq!(pool.snapshot().unwrap().0, 24);
        drop(state);
        assert_eq!(pool.snapshot().unwrap().0, 0);
    }
}
