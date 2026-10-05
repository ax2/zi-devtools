use super::*;

#[test]
fn windows_reuse_targets_bound_count_and_never_reuse_restored_viewport_ids() {
    let mut windows = Windows::default();
    let ctx = egui::Context::default();
    windows.open(Target::Timer(11), &ctx).unwrap();
    let first = windows.panes[0].viewport();
    windows.open(Target::Timer(11), &ctx).unwrap();
    assert_eq!(windows.panes.len(), 1);
    for id in 12..=18 {
        windows.open(Target::Timer(id), &ctx).unwrap();
    }
    assert!(windows.open(Target::Focus, &ctx).is_err());
    windows.clear();
    windows.open(Target::Timer(11), &ctx).unwrap();
    assert_ne!(first, windows.panes[0].viewport());
    assert!(!windows.panes[0].pinned);
    assert!(!windows.panes[0].fullscreen);
    windows.panes[0].pinned = true;
    assert_eq!(windows.panes[0].level(), egui::WindowLevel::AlwaysOnTop);
    windows.panes[0].fullscreen = true;
    assert_eq!(windows.panes[0].level(), egui::WindowLevel::Normal);
    windows.panes[0].fullscreen = false;
    assert_eq!(windows.panes[0].level(), egui::WindowLevel::AlwaysOnTop);
}

#[test]
fn stable_timer_actions_do_not_target_the_next_item_or_repeat_reminders() {
    let mut state = State {
        timer_seconds: 1,
        ..State::default()
    };
    state.add_timer().unwrap();
    state.add_timer().unwrap();
    let now = Instant::now();
    assert!(state.window_action(Target::Timer(2), Action::Toggle, now));
    state.timers.remove(0);
    assert!(!state.window_action(Target::Timer(1), Action::Reset, now));
    assert!(state.timers[0].running());
    assert!(state.poll(now + Duration::from_secs(2), Utc::now()));
    assert_eq!(state.notices.len(), 1);
    assert!(!state.window_action(Target::Timer(2), Action::Toggle, now));
    assert!(!state.poll(now + Duration::from_secs(3), Utc::now()));
    assert!(state.window_action(Target::Timer(2), Action::Reset, now));
    assert!(state.notices.is_empty());
    assert!(!state.timers[0].running());
    assert_eq!(state.timers[0].remaining(now), Duration::from_secs(1));
}

#[test]
fn shared_stopwatch_focus_and_window_closure_preserve_runtime_and_restore_closes_views() {
    let mut state = State::default();
    let now = Instant::now();
    let utc = Utc::now();
    state.window_action(Target::Stopwatch, Action::Toggle, now);
    state.window_action(Target::Stopwatch, Action::Lap, now + Duration::from_secs(2));
    state.window_action(
        Target::Stopwatch,
        Action::Toggle,
        now + Duration::from_secs(3),
    );
    assert_eq!(state.stopwatch.laps, [Duration::from_secs(2)]);
    assert_eq!(state.stopwatch.elapsed(now), Duration::from_secs(3));
    assert!(!state.window_action(Target::Focus, Action::Next, now));
    state.focus.timer.finished = true;
    assert!(state.window_action(Target::Focus, Action::Next, now));
    assert_eq!(state.focus.phase, 1);
    assert!(!state.focus.timer.running());
    state
        .windows
        .open(Target::Focus, &egui::Context::default())
        .unwrap();
    state.window_action(Target::Focus, Action::Toggle, now);
    state.windows.clear();
    assert!(state.focus.timer.running());
    let snapshot = persistence::Snapshot::capture(&state, now, utc, persistence::Policy::Pause);
    state
        .windows
        .open(Target::Stopwatch, &egui::Context::default())
        .unwrap();
    assert!(state.needs_clock());
    snapshot
        .restore(&mut state, now, utc, persistence::Policy::Pause)
        .unwrap();
    assert!(!state.windows.active());
    assert!(!state.focus.timer.running());
    assert_eq!(state.stopwatch.laps, [Duration::from_secs(2)]);
}
