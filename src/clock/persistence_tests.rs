use super::*;
fn fixture() -> (State, Instant, DateTime<Utc>) {
    let now = Instant::now();
    let utc = "2026-10-06T08:00:00Z".parse().unwrap();
    let mut s = State::default();
    s.add_timer().unwrap();
    s.timers[0].toggle(now);
    s.stopwatch.toggle(now);
    s.stopwatch.lap(now + Duration::from_secs(1));
    (s, now + Duration::from_secs(2), utc)
}
fn dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("zi-clock-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).unwrap();
    dir
}
#[test]
fn pause_continue_and_backwards_time_restore_without_replaying_notices() {
    let (s, now, utc) = fixture();
    let snap = Snapshot::capture(&s, now, utc, Policy::Pause);
    let mut restored = State::default();
    snap.restore(
        &mut restored,
        now,
        utc + chrono::Duration::seconds(60),
        Policy::Pause,
    )
    .unwrap();
    assert!(!restored.timers[0].running());
    assert_eq!(restored.timers[0].remaining(now), Duration::from_secs(298));
    assert!(!restored.stopwatch.running());
    assert_eq!(restored.stopwatch.elapsed(now), Duration::from_secs(2));
    assert_eq!(restored.stopwatch.laps.len(), 1);
    snap.restore(
        &mut restored,
        now,
        utc + chrono::Duration::seconds(60),
        Policy::Continue,
    )
    .unwrap();
    assert!(restored.timers[0].running());
    assert_eq!(restored.timers[0].remaining(now), Duration::from_secs(238));
    snap.restore(
        &mut restored,
        now,
        utc - chrono::Duration::hours(1),
        Policy::Continue,
    )
    .unwrap();
    assert_eq!(restored.timers[0].remaining(now), Duration::from_secs(298));
    snap.restore(
        &mut restored,
        now,
        utc + chrono::Duration::days(1),
        Policy::Continue,
    )
    .unwrap();
    assert!(restored.timers[0].finished);
    assert_eq!(restored.notices.len(), 1);
    assert!(!restored.poll(now, utc + chrono::Duration::days(1)));
    let saved = Snapshot::capture(
        &restored,
        now,
        utc + chrono::Duration::days(1),
        Policy::Continue,
    );
    saved
        .restore(
            &mut restored,
            now,
            utc + chrono::Duration::days(2),
            Policy::Continue,
        )
        .unwrap();
    assert_eq!(restored.notices.len(), 1);
    restored.notices.clear();
    let acknowledged = Snapshot::capture(
        &restored,
        now,
        utc + chrono::Duration::days(2),
        Policy::Continue,
    );
    acknowledged
        .restore(
            &mut restored,
            now,
            utc + chrono::Duration::days(3),
            Policy::Continue,
        )
        .unwrap();
    assert!(restored.notices.is_empty());
}
#[test]
fn alarms_snooze_and_focus_survive_roundtrip_and_catch_up_once() {
    let (mut s, now, utc) = fixture();
    s.alarm_text = "2026-10-07 09:00".into();
    s.daily = true;
    s.add_alarm(utc).unwrap();
    s.alarms[0].snooze = Some(utc + chrono::Duration::minutes(5));
    s.focus.timer.toggle(now);
    s.focus.completed = 3;
    let snap = Snapshot::capture(&s, now, utc, Policy::Continue);
    let bytes = serde_json::to_vec(&snap).unwrap();
    let snap: Snapshot = serde_json::from_slice(&bytes).unwrap();
    let mut r = State::default();
    let future = utc + chrono::Duration::days(3);
    snap.restore(&mut r, now, future, Policy::Continue).unwrap();
    assert_eq!(r.focus.completed, 3);
    assert!(r.focus.timer.finished);
    assert!(r.alarms[0].snooze.is_some());
    assert!(r.poll(now, future));
    assert!(r.alarms[0].at > future);
    assert!(r.alarms[0].snooze.is_none());
    assert!(!r.poll(now, future));
    assert_eq!(r.notices.len(), 3);
    let again = Snapshot::capture(&r, now, future, Policy::Continue);
    again
        .restore(&mut r, now, future, Policy::Continue)
        .unwrap();
    assert!(!r.poll(now, future));
    assert_eq!(r.notices.len(), 3);
}
#[test]
fn invalid_snapshots_do_not_modify_current_state() {
    let (s, now, utc) = fixture();
    let good = Snapshot::capture(&s, now, utc, Policy::Pause);
    let mut invalid = Vec::new();
    let mut x = good.clone();
    x.schema = 2;
    invalid.push(x);
    let mut x = good.clone();
    x.zones = vec!["Unknown/Zone".into()];
    invalid.push(x);
    let mut x = good.clone();
    x.timers.push(x.timers[0].clone());
    invalid.push(x);
    let mut x = good.clone();
    x.timers[0].remaining = u64::MAX;
    invalid.push(x);
    let mut x = good.clone();
    x.timers[0].id = u64::MAX;
    invalid.push(x);
    let mut x = good.clone();
    x.laps = vec![3000];
    invalid.push(x);
    let mut x = good.clone();
    x.phase = 9;
    invalid.push(x);
    let mut x = good.clone();
    x.notices.push(NoticeData {
        source: Source::Alarm(999),
        title: "bad".into(),
    });
    invalid.push(x);
    let mut x = good.clone();
    x.alarms = vec![AlarmData {
        id: 2,
        name: "alarm".into(),
        zone: "Unknown".into(),
        at: utc,
        daily: false,
        late: false,
        time: NaiveTime::MIN,
        enabled: true,
        snooze: None,
    }];
    invalid.push(x);
    for snapshot in invalid {
        let mut r = State {
            timer_name: "preserve".into(),
            ..State::default()
        };
        assert!(
            snapshot
                .restore(&mut r, now, utc, Policy::Continue)
                .is_err()
        );
        assert!(r.timers.is_empty());
        assert_eq!(r.timer_name, "preserve");
    }
}
#[test]
fn bounded_atomic_store_rejects_stale_writers_and_preserves_previous_bytes() {
    let dir = dir();
    let path = dir.join("clock.json");
    let (s, now, utc) = fixture();
    let mut snap = Snapshot::capture(&s, now, utc, Policy::Pause);
    assert!(read(&path).unwrap().is_none());
    let hash = write(&path, &snap, None).unwrap();
    let before = fs::read(&path).unwrap();
    assert!(write(&path, &snap, None).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    snap.meeting = 10;
    let next = write(&path, &snap, Some(hash)).unwrap();
    assert_ne!(hash, next);
    assert!(write(&path, &snap, Some(hash)).is_err());
    assert_eq!(read(&path).unwrap().unwrap().0, snap);
    let lock = WriteLock::acquire(&path).unwrap();
    assert!(write(&path, &snap, Some(next)).is_err());
    drop(lock);
    assert!(write(&path, &snap, Some(next)).is_ok());
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    fs::write(&path, b"corrupt original").unwrap();
    assert!(write(&path, &snap, Some(next)).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"corrupt original");
    fs::write(&path, vec![b' '; LIMIT as usize + 1]).unwrap();
    assert!(read(&path).is_err());
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn background_load_requires_confirmation_and_failure_keeps_live_state() {
    let dir = dir();
    let path = dir.join("clock.json");
    let (s, now, utc) = fixture();
    let snap = Snapshot::capture(&s, now, utc, Policy::Pause);
    write(&path, &snap, None).unwrap();
    let mut storage = Storage::new(path.clone(), egui::Context::default());
    let limit = Instant::now() + Duration::from_secs(5);
    while storage.busy() {
        assert!(Instant::now() < limit);
        storage.tick(&s, now, utc);
        std::thread::yield_now();
    }
    assert!(storage.pending.is_some());
    assert!(!storage.enabled);
    assert_eq!(s.timers.len(), 1);
    let before = fs::read(&path).unwrap();
    storage.dirty = true;
    storage.tick(&s, now, utc);
    assert_eq!(fs::read(&path).unwrap(), before);
    storage.pending = None;
    storage.enabled = true;
    fs::write(&path, b"external changed").unwrap();
    storage.tick(&s, now, utc);
    while storage.busy() {
        assert!(Instant::now() < limit);
        storage.tick(&s, now, utc);
        std::thread::yield_now();
    }
    assert!(!storage.error.is_empty());
    assert!(storage.dirty);
    assert!(s.timers[0].running());
    assert_eq!(fs::read(&path).unwrap(), b"external changed");
    fs::remove_dir_all(dir).unwrap();
}
