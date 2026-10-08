use super::*;
fn source() -> Arc<DynamicImage> {
    Arc::new(DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        10,
        5,
        image::Rgba([200, 30, 10, 130]),
    )))
}
fn crop_def() -> Definition {
    Definition {
        steps: vec![
            Step::Crop {
                version: 1,
                x: 0,
                y: 0,
                width: 5000,
                height: 10000,
            },
            Step::Info { version: 1 },
        ],
        ..Definition::default()
    }
}
#[test]
fn real_execution_keeps_old_results_charged_rejects_new_allocation_and_reclaims_on_drop() {
    let pool = super::super::super::memory::Pool::new(300);
    let source = source();
    let def = crop_def();
    let first = execute_budgeted(&def, source.clone(), &AtomicBool::new(false), &pool).unwrap();
    assert!(first.failure.is_none());
    assert_eq!(first.outputs.len(), 2);
    assert_eq!(pool.snapshot().unwrap(), (300, 0, 300));
    assert!(Arc::ptr_eq(
        &first.outputs[0].image,
        &first.outputs[1].image
    ));
    let next = execute_budgeted(&def, source.clone(), &AtomicBool::new(false), &pool).unwrap();
    assert!(next.failure.as_deref().unwrap().contains("预算不足"));
    assert!(next.outputs.is_empty());
    assert_eq!(first.outputs[0].image.dimensions(), (5, 5));
    assert_eq!(pool.snapshot().unwrap(), (300, 0, 300));
    assert!(pool.set_limit(299).is_err());
    drop(first);
    assert_eq!(pool.snapshot().unwrap(), (200, 0, 300));
    let next = execute_budgeted(&def, source.clone(), &AtomicBool::new(false), &pool).unwrap();
    assert!(next.failure.is_none());
    drop(next);
    drop(source);
    assert_eq!(pool.snapshot().unwrap(), (0, 0, 300));
}
#[test]
fn actual_worker_receipt_not_polled_still_blocks_other_worker_and_drop_releases() {
    let pool = super::super::super::memory::Pool::new(300);
    let source = source();
    let (tx, rx) = mpsc::channel();
    let worker_pool = pool.clone();
    let worker_source = source.clone();
    std::thread::spawn(move || {
        let run = execute_budgeted(
            &crop_def(),
            worker_source,
            &AtomicBool::new(false),
            &worker_pool,
        )
        .unwrap();
        assert!(run.failure.is_none());
        tx.send(run).unwrap();
    })
    .join()
    .unwrap();
    assert_eq!(pool.snapshot().unwrap(), (300, 0, 300));
    let other_pool = pool.clone();
    let other_source = source.clone();
    let rejected = std::thread::spawn(move || {
        execute_budgeted(
            &crop_def(),
            other_source,
            &AtomicBool::new(false),
            &other_pool,
        )
        .unwrap()
    })
    .join()
    .unwrap();
    assert!(rejected.failure.is_some());
    assert_eq!(pool.snapshot().unwrap(), (300, 0, 300));
    drop(rx);
    assert_eq!(pool.snapshot().unwrap(), (200, 0, 300));
    let cancelled = execute_budgeted(&crop_def(), source, &AtomicBool::new(true), &pool).unwrap();
    assert!(cancelled.cancelled);
    assert_eq!(pool.snapshot().unwrap(), (0, 0, 300));
}
#[test]
fn real_encoding_growth_uses_remaining_capacity_and_shared_instances_use_same_pool() {
    let pool = super::super::super::memory::Pool::new(1000);
    let src = source();
    let run = execute_budgeted(
        &Definition::default(),
        src.clone(),
        &AtomicBool::new(false),
        &pool,
    )
    .unwrap();
    assert!(run.failure.is_none(), "{:?}", run.failure);
    let (retained, reserved, _) = pool.snapshot().unwrap();
    assert!(retained > 400 && retained <= 1000);
    assert_eq!(reserved, 0);
    let encoded = run.outputs[1].encoded.as_ref().unwrap();
    assert_eq!(
        image::load_from_memory(encoded).unwrap().to_rgba8(),
        src.to_rgba8()
    );
    drop(run);
    drop(src);
    assert_eq!(pool.snapshot().unwrap(), (0, 0, 1000));
    let mut work = Workspace::default();
    work.memory.set_limit(200).unwrap();
    let prepared =
        relay::prepare(relay::Source::Image(source()), vec![], "image-workflow").unwrap();
    work.receive(&prepared).unwrap();
    let from = work.active_id();
    work.receive_at(&Destination::New, Some(&from), false, &prepared)
        .unwrap();
    assert_eq!(work.memory.snapshot().unwrap().0, 200);
    let copy = relay::prepare(relay::Source::Image(source()), vec![], "image-workflow").unwrap();
    assert!(
        work.receive_at(&Destination::New, Some(&from), false, &copy)
            .is_err()
    );
    assert_eq!(work.instances.len(), 2);
    assert_eq!(work.memory.snapshot().unwrap(), (200, 0, 200));
}
