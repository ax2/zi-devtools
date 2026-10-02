use super::*;
use chrono::{FixedOffset, Timelike};

fn date(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
}
fn now(s: &str) -> DateTime<FixedOffset> {
    FixedOffset::east_opt(8 * 3600)
        .unwrap()
        .from_local_datetime(&date(s))
        .unwrap()
}
fn event() -> Item {
    let mut item = Item::new(Some(NaiveDate::from_ymd_opt(2026, 10, 2).unwrap()));
    item.title = "合成日程".into();
    item
}
fn fixture() -> PathBuf {
    std::env::temp_dir()
        .join(format!("zi-planner-{}", uuid::Uuid::new_v4()))
        .join("planner.sqlite3")
}

#[test]
fn reminder_lead_ack_snooze_restart_and_disabled() {
    let mut s = event().schedule.unwrap();
    assert_eq!(s.due(now("2026-10-02 08:49")), None);
    assert_eq!(s.due(now("2026-10-02 08:50")), Some(s.start));
    s.snooze = Some((s.start, now("2026-10-02 09:10").timestamp()));
    let persisted = serde_json::to_string(&s).unwrap();
    s = serde_json::from_str(&persisted).unwrap();
    assert_eq!(s.due(now("2026-10-02 09:09")), None);
    assert_eq!(s.due(now("2026-10-02 09:10")), Some(s.start));
    s.handled = Some(s.start);
    assert_eq!(s.due(now("2026-10-05 09:10")), None);
    s.handled = None;
    s.done = true;
    assert_eq!(s.due(now("2026-10-05 09:10")), None);
    s.done = false;
    s.remind = false;
    assert_eq!(s.due(now("2026-10-05 09:10")), None);
}

#[test]
fn recurring_overdue_coalesces_and_crosses_year_leap_day() {
    let mut s = event().schedule.unwrap();
    s.repeat = Repeat::Daily;
    assert_eq!(
        s.due(now("2026-10-06 08:49")),
        Some(date("2026-10-05 09:00"))
    );
    assert_eq!(
        s.due(now("2026-10-06 08:50")),
        Some(date("2026-10-06 09:00"))
    );
    s.handled = Some(date("2026-10-06 09:00"));
    assert_eq!(s.due(now("2026-10-07 08:49")), None);
    assert_eq!(
        s.due(now("2027-01-01 09:00")),
        Some(date("2027-01-01 09:00"))
    );
    assert_eq!(
        s.due(now("2028-02-29 09:00")),
        Some(date("2028-02-29 09:00"))
    );
    s.repeat = Repeat::Weekly;
    s.handled = None;
    assert_eq!(
        s.due(now("2026-10-10 09:00")),
        Some(date("2026-10-09 09:00"))
    );
    assert!(s.on_day(date("2026-10-09 09:00").date()));
    assert!(!s.on_day(date("2026-10-08 09:00").date()));
    assert!(!s.on_day(date("2026-09-25 09:00").date()));
}

#[test]
fn save_restore_conflict_trash_and_schema_guard() {
    let path = fixture();
    assert!(store::load(&path).unwrap().is_empty());
    assert!(!path.parent().unwrap().exists());
    let item = event();
    store::save(&path, item).unwrap();
    let original = store::load(&path).unwrap().remove(0);
    assert_eq!(original.revision, 1);
    let mut changed = original.clone();
    changed.trash = true;
    store::save(&path, changed).unwrap();
    assert!(
        store::save(&path, original)
            .unwrap_err()
            .to_string()
            .contains("另一窗口")
    );
    let mut trashed = store::load(&path).unwrap().remove(0);
    assert!(trashed.trash);
    assert_eq!(trashed.revision, 2);
    trashed.trash = false;
    store::save(&path, trashed).unwrap();
    assert!(!store::load(&path).unwrap()[0].trash);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.pragma_update(None, "user_version", 9).unwrap();
    drop(connection);
    assert!(store::load(&path).is_err());
    assert!(store::save(&path, event()).is_err());
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(path.parent().unwrap()).unwrap();
}

#[test]
fn invalid_input_never_creates_database() {
    let path = fixture();
    let mut item = event();
    item.title.clear();
    assert!(store::save(&path, item).is_err());
    let mut item = event();
    item.body = "x".repeat(MAX_BODY + 1);
    assert!(store::save(&path, item).is_err());
    let mut item = event();
    item.schedule.as_mut().unwrap().start = date("2100-01-01 09:00");
    assert!(store::save(&path, item).is_err());
    assert!(!path.exists());
}

#[test]
fn daylight_saving_gaps_and_repeated_hour_do_not_duplicate() {
    let tz = chrono_tz::America::New_York;
    let mut s = event().schedule.unwrap();
    s.start = date("2026-03-07 02:30");
    s.repeat = Repeat::Daily;
    s.minutes = 0;
    let gap = tz.with_ymd_and_hms(2026, 3, 8, 10, 0, 0).unwrap();
    assert!(
        tz.from_local_datetime(&date("2026-03-08 02:30"))
            .single()
            .is_none()
    );
    assert_eq!(s.due(gap), None);
    let next = tz.with_ymd_and_hms(2026, 3, 9, 3, 0, 0).unwrap();
    assert_eq!(s.due(next), Some(date("2026-03-09 02:30")));
    s.start = date("2026-10-31 01:30");
    let repeated = tz.from_local_datetime(&date("2026-11-01 01:40"));
    let occurrence = s.due(repeated.earliest().unwrap()).unwrap();
    s.handled = Some(occurrence);
    assert_eq!(s.due(repeated.latest().unwrap()), None);
}

#[test]
fn repeat_lead_time_and_content_size_are_bounded() {
    let mut item = event();
    let s = item.schedule.as_mut().unwrap();
    s.repeat = Repeat::Daily;
    s.minutes = 1440;
    assert!(item.validate().is_err());
    let path = fixture();
    store::save(&path, event()).unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE records SET payload=?1",
            ["x".repeat(MAX_BODY + 8193)],
        )
        .unwrap();
    drop(connection);
    assert!(store::load(&path).is_err());
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(path.parent().unwrap()).unwrap();
}

#[test]
fn lunar_new_year_leap_month_terms_and_boundaries() {
    assert!(
        lunar_day(date("2024-02-10 00:00").date())
            .full
            .contains("正月初一")
    );
    assert!(
        lunar_day(date("2025-01-29 00:00").date())
            .full
            .contains("正月初一")
    );
    assert!(
        lunar_day(date("2023-03-22 00:00").date())
            .full
            .contains("闰二月初一")
    );
    assert_eq!(lunar_day(date("2024-04-04 00:00").date()).short, "清明");
    for d in ["1901-01-01 00:00", "2099-12-31 00:00", "2028-02-29 00:00"] {
        assert!(!lunar_day(date(d).date()).full.is_empty());
    }
}

fn wait_state(state: &mut State) {
    let ctx = egui::Context::default();
    let start = Instant::now();
    while state.pending.is_some() {
        state.poll(&ctx);
        assert!(start.elapsed().as_secs() < 8);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn editor_retains_draft_on_error_and_detects_unsaved_date() {
    let path = fixture();
    let mut state = State::new(path.clone());
    wait_state(&mut state);
    state.new_draft(Some(Local::now().date_naive()));
    state.draft.as_mut().unwrap().title = "测试".into();
    state.date_text = "invalid".into();
    state.save_draft();
    assert!(state.error);
    assert!(state.has_unsaved());
    assert!(!path.exists());
    state.date_text = Local::now().date_naive().to_string();
    state.save_draft();
    wait_state(&mut state);
    assert!(!state.has_unsaved());
    assert_eq!(state.draft.as_ref().unwrap().revision, 1);
    state.time_text = "10:13".into();
    assert!(state.has_unsaved());
    assert!(!state.may_leave());
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(path.parent().unwrap()).unwrap();
}

#[test]
fn copied_memo_event_is_unsaved_until_saved_or_explicitly_discarded() {
    let path = fixture();
    let mut state = State::new(path.clone());
    wait_state(&mut state);
    let mut copied = event();
    copied.body = "来自已保存备忘的内容".into();
    state.edit(copied.clone());
    assert!(state.has_unsaved());
    assert!(!state.may_leave());
    state.discard();
    assert!(!state.has_unsaved());
    assert!(state.draft.is_none());
    assert!(state.may_leave());
    state.edit(copied);
    state.save_draft();
    wait_state(&mut state);
    assert!(!state.has_unsaved());
    assert_eq!(store::load(&path).unwrap().len(), 1);
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(path.parent().unwrap()).unwrap();
}

#[test]
fn global_poll_delivers_once_suppresses_trash_and_recovers_after_restart() {
    let path = fixture();
    let mut item = event();
    let s = item.schedule.as_mut().unwrap();
    s.start = Local::now()
        .naive_local()
        .with_second(0)
        .unwrap()
        .with_nanosecond(0)
        .unwrap()
        - Duration::minutes(1);
    store::save(&path, item).unwrap();
    let mut state = State::new(path.clone());
    wait_state(&mut state);
    assert!(state.alarm_open);
    assert_eq!(state.alarms.len(), 1);
    state.alarm_open = false;
    state.last_tick -= std::time::Duration::from_secs(2);
    assert!(!state.poll(&egui::Context::default()));
    assert!(!state.alarm_open);
    let mut restart = State::new(path.clone());
    wait_state(&mut restart);
    assert!(restart.alarm_open);
    let mut item = restart.items[0].clone();
    item.trash = true;
    restart.launch(Some(item));
    wait_state(&mut restart);
    assert!(restart.alarms.is_empty());
    assert!(!restart.needs_clock());
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(path.parent().unwrap()).unwrap();
}
