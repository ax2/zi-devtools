use super::*;
use crate::tasks::{Center, Phase};
fn wait(work: &mut Workspace) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while work.instances.iter().any(|i| i.state.busy()) {
        assert!(std::time::Instant::now() < deadline);
        work.poll(&egui::Context::default());
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}
#[test]
fn cancel_open_generation_and_closed_terminal_rows_keep_exact_owner() {
    let mut work = Workspace::default();
    let ctx = egui::Context::default();
    let id = work.instances[0].id.clone();
    work.launch(&ctx, Kind::Run, |cancel| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !cancel.load(Ordering::Relaxed) {
            ensure!(std::time::Instant::now() < deadline, "deadline");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        Ok(Reply::Run(Run {
            cancelled: true,
            ..Default::default()
        }))
    });
    let first = work.snapshots()[0].clone();
    work.create().unwrap();
    let active = work.instances[work.active].id.clone();
    assert!(work.cancel_task(&id, first.generation + 1).is_err());
    assert!(!work.instances[0].state.cancel.load(Ordering::Relaxed));
    work.cancel_task(&id, first.generation).unwrap();
    wait(&mut work);
    assert_eq!(work.instances[work.active].id, active);
    assert_eq!(work.snapshots()[0].phase, Phase::Cancelled);
    work.open_task(&id, first.generation).unwrap();
    work.launch(&ctx, Kind::Input, |_| {
        Ok(Reply::Input(
            Arc::new(DynamicImage::new_rgba8(2, 2)),
            "private-filename.png".into(),
        ))
    });
    wait(&mut work);
    work.select(1).unwrap();
    assert!(work.open_task(&id, first.generation).is_err());
    assert!(work.cancel_task(&id, first.generation).is_err());
    assert_eq!(work.instances[work.active].id, active);
    let latest = work.snapshots()[0].clone();
    assert_eq!(latest.phase, Phase::Done);
    assert!(!latest.summary.contains("private-filename"));
    work.open_task(&id, latest.generation).unwrap();
    work.request_close().unwrap();
    work.close().unwrap();
    let closed = work.take_task_receipts();
    assert_eq!(closed.len(), 2);
    assert_eq!(closed[0].phase, Phase::Cancelled);
    assert_eq!(closed[1].generation, latest.generation);
    assert_eq!(closed[0].instance.as_deref(), Some(id.as_str()));
    assert!(work.take_task_receipts().is_empty());
    assert!(work.open_task(&id, latest.generation).is_err());
    let mut center = Center::default();
    center.observe(Some(first));
    for row in closed {
        center.observe(Some(row));
    }
    assert_eq!(center.rows.last().unwrap().phase, Phase::Done);
    center.clear_finished();
    for r in work.snapshots() {
        center.observe(Some(r));
    }
    assert!(center.rows.is_empty());
}
#[test]
fn actual_definition_publication_survives_late_cancel_and_metadata_is_safe() {
    let directory = std::env::temp_dir().join(format!("zi-image-task-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("private-name.json");
    let target = path.clone();
    let def = Definition::default();
    let expected = def.bytes().unwrap();
    let (published_tx, published_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut work = Workspace::default();
    let ctx = egui::Context::default();
    work.launch(&ctx, Kind::SaveDefinition, move |cancel| {
        crate::local_files::save_new_moved(&target, &def.bytes()?, cancel)?;
        published_tx.send(()).unwrap();
        release_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        Ok(Reply::SavedDefinition(
            crate::workflow_document::Document::Image(def).metadata(&target),
        ))
    });
    published_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    let row = work.snapshots()[0].clone();
    work.cancel_task(row.instance.as_ref().unwrap(), row.generation)
        .unwrap();
    release_tx.send(()).unwrap();
    wait(&mut work);
    assert_eq!(std::fs::read(&path).unwrap(), expected);
    let final_row = work.snapshots()[0].clone();
    assert_eq!(final_row.phase, Phase::Done);
    assert!(!final_row.summary.contains("private-name"));
    assert!(!final_row.summary.contains(directory.to_str().unwrap()));
    assert_eq!(work.take_loaded().unwrap().path, path);
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(&directory).unwrap();
}

#[test]
fn cancelled_load_receipt_survives_following_operation_without_replacing_input() {
    let mut work = Workspace::default();
    let ctx = egui::Context::default();
    let original = Arc::new(DynamicImage::new_rgba8(10, 6));
    work.source = Some(original.clone());
    let (ready_tx, ready_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    work.launch(&ctx, Kind::Input, move |_| {
        let image = Arc::new(DynamicImage::new_rgba8(20, 10));
        ready_tx.send(()).unwrap();
        release_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        Ok(Reply::Input(image, "private-file.png".into()))
    });
    ready_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    let row = work.snapshots()[0].clone();
    work.cancel_task(row.instance.as_ref().unwrap(), row.generation)
        .unwrap();
    release_tx.send(()).unwrap();
    wait(&mut work);
    assert!(Arc::ptr_eq(work.source.as_ref().unwrap(), &original));
    work.launch(&ctx, Kind::Run, |_| Ok(Reply::Run(Run::default())));
    wait(&mut work);
    let receipts = work.take_task_receipts();
    assert_eq!(receipts.len(), 2);
    assert_eq!(receipts[0].phase, Phase::Cancelled);
    assert_eq!(receipts[1].phase, Phase::Done);
    assert!(receipts[0].generation < receipts[1].generation);
    assert!(work.take_task_receipts().is_empty());
}
