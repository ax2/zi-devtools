use super::*;
#[test]
fn policy_handles_midnight_boundaries_all_day_quiet_and_preview() {
    let mut s = Settings {
        enabled: true,
        quiet: true,
        ..Settings::default()
    };
    assert!(!s.permitted(1320, false));
    assert!(!s.permitted(0, false));
    assert!(!s.permitted(419, false));
    assert!(s.permitted(420, false));
    assert!(s.permitted(1319, false));
    assert!(s.permitted(0, true));
    s.quiet_start = 600;
    s.quiet_end = 720;
    assert!(s.permitted(599, false));
    assert!(!s.permitted(600, false));
    assert!(s.permitted(720, false));
    s.quiet_start = 600;
    s.quiet_end = 600;
    assert!(!s.permitted(900, false));
    s.muted = true;
    assert!(!s.permitted(900, true));
    s.muted = false;
    s.volume = 0;
    assert!(!s.permitted(900, true));
    s.volume = 101;
    assert!(!s.valid());
    assert_eq!(parse_minute("23:59"), Some(1439.0));
    assert_eq!(parse_minute(" 07:00 "), Some(420.0));
    assert_eq!(parse_minute("24:00"), None);
    assert_eq!(parse_minute("12:60"), None);
}
#[test]
fn generated_pcm_is_bounded_stereo_finite_and_volume_scaled() {
    for tone in [Tone::Chime, Tone::Double, Tone::Soft] {
        let settings = Settings {
            volume: 100,
            tone,
            ..Settings::default()
        };
        let high = samples(&settings);
        let low = samples(&Settings {
            volume: 25,
            ..settings.clone()
        });
        let zero = samples(&Settings {
            volume: 0,
            ..settings
        });
        assert!(high.len() < RATE * 8 && high.len() % 8 == 0);
        assert_eq!(high.len(), low.len());
        assert!(zero.iter().all(|b| *b == 0));
        let mut peak = 0.0f32;
        for (a, b) in high.chunks_exact(8).zip(low.chunks_exact(8)) {
            assert_eq!(&a[..4], &a[4..]);
            let v = f32::from_le_bytes(a[..4].try_into().unwrap());
            let w = f32::from_le_bytes(b[..4].try_into().unwrap());
            assert!(v.is_finite() && v.abs() <= 0.25);
            assert!((w - v * 0.25).abs() < 1e-6);
            peak = peak.max(v.abs());
        }
        assert!(peak > 0.1);
        assert_eq!(&high[..8], &[0; 8]);
    }
}
#[test]
fn worker_cancels_without_overlapping_and_reports_failure_without_retry() {
    fn wait(_: &[u8], cancel: &AtomicBool) -> Result<(), String> {
        let limit = Instant::now() + Duration::from_secs(2);
        while !cancel.load(Ordering::Acquire) {
            assert!(Instant::now() < limit);
            std::thread::yield_now();
        }
        Ok(())
    }
    let mut s = State::new(egui::Context::default());
    s.settings.enabled = true;
    s.backend = wait;
    assert!(s.alert(Utc::now(), false));
    assert!(!s.alert(Utc::now(), true));
    assert_eq!(s.attempts, 1);
    s.stop();
    let limit = Instant::now() + Duration::from_secs(3);
    while s.busy() {
        assert!(Instant::now() < limit);
        s.tick();
        std::thread::yield_now();
    }
    assert_eq!(s.status, "播放已停止");
    s.backend = |_, _| Err("synthetic device failure".into());
    assert!(s.alert(Utc::now(), false));
    while s.busy() {
        assert!(Instant::now() < limit);
        s.tick();
        std::thread::yield_now();
    }
    assert!(!s.error.is_empty());
    s.tick();
    assert_eq!(s.attempts, 2);
    assert!(!s.busy());
}
#[test]
fn simultaneous_reminders_make_one_sound_attempt_and_mute_never_drops_notices() {
    let mut s = super::super::State::default();
    let now = Instant::now();
    s.audio.settings.enabled = true;
    s.audio.backend = |_, _| Err("synthetic device failure".into());
    s.timer_seconds = 1;
    s.add_timer().unwrap();
    s.add_timer().unwrap();
    for t in &mut s.timers {
        t.toggle(now);
    }
    assert!(s.poll(now + Duration::from_secs(2), Utc::now()));
    assert_eq!(s.notices.len(), 2);
    assert_eq!(s.audio.attempts, 1);
    let limit = Instant::now() + Duration::from_secs(3);
    while s.audio.busy() {
        assert!(Instant::now() < limit);
        s.audio.tick();
        std::thread::yield_now();
    }
    assert!(!s.poll(now + Duration::from_secs(3), Utc::now()));
    assert_eq!(s.audio.attempts, 1);
    assert!(!s.audio.error.is_empty());
    s.audio.settings.muted = true;
    s.add_timer().unwrap();
    s.timers[2].toggle(now);
    assert!(s.poll(now + Duration::from_secs(4), Utc::now()));
    assert_eq!(s.notices.len(), 3);
    assert_eq!(s.audio.attempts, 1);
}
