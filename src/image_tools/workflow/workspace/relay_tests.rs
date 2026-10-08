use super::*;
fn prepared(value: u8) -> relay::Prepared {
    relay::prepare(
        relay::Source::Image(Arc::new(DynamicImage::ImageRgba8(
            image::RgbaImage::from_pixel(8, 6, image::Rgba([value, 2, 3, 120])),
        ))),
        vec![],
        "image-workflow",
    )
    .unwrap()
}
#[test]
fn existing_target_is_bound_to_id_preserves_definition_source_and_requires_fresh_consent() {
    let mut work = Workspace::default();
    let a = prepared(10);
    work.receive(&a);
    let source_id = work.active_id();
    work.create().unwrap();
    let b = prepared(20);
    work.receive(&b);
    work.definition.steps = vec![Step::Info { version: 1 }];
    let def = work.definition.clone();
    let target = work.relay_choices(Some(&source_id))[0].destination.clone();
    work.select(0).unwrap();
    assert!(
        work.receive_at(&target, Some(&source_id), false, &a)
            .is_err()
    );
    assert!(Arc::ptr_eq(
        work.instances[1].state.source.as_ref().unwrap(),
        &b.image
    ));
    work.receive_at(&target, Some(&source_id), true, &a)
        .unwrap();
    assert_eq!(work.active, 1);
    assert_eq!(work.definition, def);
    assert!(work.run.is_none());
    assert!(!work.busy());
    assert!(Arc::ptr_eq(work.source.as_ref().unwrap(), &a.image));
    assert!(Arc::ptr_eq(
        work.instances[0].state.source.as_ref().unwrap(),
        &a.image
    ));
    assert!(
        work.receive_at(&target, Some(&source_id), true, &b)
            .is_err()
    );
    assert!(Arc::ptr_eq(work.source.as_ref().unwrap(), &a.image));
    let own = work.relay_choices(None)[0].destination.clone();
    assert!(work.receive_at(&own, Some(&source_id), true, &b).is_err());
}
#[test]
fn new_only_creates_on_success_capacity_closed_busy_and_import_are_guarded() {
    let mut work = Workspace::default();
    let a = prepared(10);
    work.receive(&a);
    let source_id = work.active_id();
    work.receive_at(&Destination::New, Some(&source_id), false, &a)
        .unwrap();
    assert_eq!(work.instances.len(), 2);
    assert!(work.run.is_none());
    let target = work.relay_choices(Some(&source_id))[0].destination.clone();
    work.receive_definition(Definition::default()).unwrap();
    assert!(
        work.receive_at(&target, Some(&source_id), true, &a)
            .is_err()
    );
    work.import = None;
    let (tx, rx) = mpsc::channel();
    work.receiver = Some(rx);
    assert!(
        work.receive_at(&target, Some(&source_id), true, &a)
            .is_err()
    );
    drop(tx);
    work.receiver = None;
    work.request_close().unwrap();
    assert!(
        work.receive_at(&Destination::New, Some(&source_id), false, &a)
            .is_err()
    );
    work.close().unwrap();
    assert!(
        work.receive_at(&target, Some(&source_id), true, &a)
            .is_err()
    );
    while work.instances.len() < MAX_INSTANCES {
        work.create().unwrap();
    }
    let before = work.active;
    assert!(
        work.receive_at(&Destination::New, Some(&source_id), false, &a)
            .is_err()
    );
    assert_eq!(work.active, before);
    assert_eq!(work.instances.len(), MAX_INSTANCES);
}
#[test]
fn async_completion_invalidates_preselected_target_token() {
    let mut work = Workspace::default();
    let a = prepared(10);
    work.receive(&a);
    work.launch(&egui::Context::default(), Kind::Run, |_| {
        Ok(Reply::Run(Run::default()))
    });
    let target = work.relay_choices(None)[0].destination.clone();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while work.busy() {
        assert!(std::time::Instant::now() < deadline);
        work.poll(&egui::Context::default());
        std::thread::yield_now();
    }
    assert!(work.receive_at(&target, None, true, &a).is_err());
    assert!(work.run.is_some());
}
