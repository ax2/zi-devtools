use super::*;
use chrono::{FixedOffset, Timelike};

pub(super) fn date(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
}
pub(super) fn now(s: &str) -> DateTime<FixedOffset> {
    FixedOffset::east_opt(8 * 3600)
        .unwrap()
        .from_local_datetime(&date(s))
        .unwrap()
}
pub(super) fn event() -> Item {
    let mut item = Item::new(Some(NaiveDate::from_ymd_opt(2026, 10, 2).unwrap()));
    item.title = "合成日程".into();
    item
}
pub(super) fn fixture() -> PathBuf {
    std::env::temp_dir()
        .join(format!("zi-planner-{}", uuid::Uuid::new_v4()))
        .join("planner.sqlite3")
}

#[test]
fn monthly_repeat_skips_or_clamps_without_losing_original_day() {
    let mut s = event().schedule.unwrap();
    s.start = date("2027-01-31 09:00");
    s.repeat = Repeat::Monthly;
    s.minutes = 0;
    assert!(!s.on_day(date("2027-02-28 00:00").date()));
    assert_eq!(s.latest(date("2027-03-01 00:00")), Some(s.start));
    assert_eq!(s.latest(date("2027-03-31 08:59")), Some(s.start));
    assert_eq!(
        s.latest(date("2027-03-31 09:00")),
        Some(date("2027-03-31 09:00"))
    );
    s.clamp_missing_day = true;
    assert!(s.on_day(date("2027-02-28 00:00").date()));
    assert_eq!(
        s.latest(date("2027-03-30 23:59")),
        Some(date("2027-02-28 09:00"))
    );
    assert!(s.on_day(date("2027-03-31 00:00").date()));
    assert!(!s.on_day(date("2027-03-28 00:00").date()));
    assert!(s.on_day(date("2028-02-29 00:00").date()));
    assert!(!s.on_day(date("2026-12-31 00:00").date()));
    assert!(!s.on_day(date("2100-01-31 00:00").date()));
    assert_eq!(
        s.latest(date("2100-01-31 09:00")),
        Some(date("2099-12-31 09:00"))
    );
}

#[test]
fn yearly_leap_day_policy_survives_years_and_does_not_drift() {
    let mut s = event().schedule.unwrap();
    s.start = date("2024-02-29 10:00");
    s.repeat = Repeat::Yearly;
    s.minutes = 0;
    assert!(!s.on_day(date("2025-02-28 00:00").date()));
    assert_eq!(s.latest(date("2028-02-29 09:59")), Some(s.start));
    assert_eq!(
        s.latest(date("2028-02-29 10:00")),
        Some(date("2028-02-29 10:00"))
    );
    s.clamp_missing_day = true;
    assert!(s.on_day(date("2025-02-28 00:00").date()));
    assert!(!s.on_day(date("2028-02-28 00:00").date()));
    assert_eq!(
        s.latest(date("2028-02-29 09:59")),
        Some(date("2027-02-28 10:00"))
    );
    assert_eq!(
        s.latest(date("2028-02-29 10:00")),
        Some(date("2028-02-29 10:00"))
    );
    s.start = date("1996-02-29 10:00");
    s.clamp_missing_day = false;
    assert!(s.on_day(date("2000-02-29 00:00").date()));
    assert_eq!(
        s.latest(date("2000-03-01 10:00")),
        Some(date("2000-02-29 10:00"))
    );
}

#[test]
fn calendar_repeat_reminders_cross_periods_and_preserve_ack_and_snooze() {
    let mut s = event().schedule.unwrap();
    s.start = date("2026-12-01 09:00");
    s.repeat = Repeat::Monthly;
    s.minutes = 7 * 1440;
    s.handled = Some(s.start);
    assert_eq!(s.due(now("2026-12-25 08:59")), None);
    let next = date("2027-01-01 09:00");
    assert_eq!(s.due(now("2026-12-25 09:00")), Some(next));
    s.snooze = Some((next, now("2026-12-25 09:10").timestamp()));
    assert_eq!(s.due(now("2026-12-25 09:09")), None);
    assert_eq!(s.due(now("2026-12-25 09:10")), Some(next));
    s.handled = Some(next);
    assert_eq!(s.due(now("2027-01-02 09:00")), None);
    s.repeat = Repeat::Yearly;
    s.start = date("2024-01-01 09:00");
    s.handled = Some(date("2026-01-01 09:00"));
    s.snooze = None;
    assert_eq!(s.due(now("2026-12-25 09:00")), Some(next));
    let tz = chrono_tz::America::New_York;
    s.start = date("2026-02-08 02:30");
    s.repeat = Repeat::Monthly;
    s.minutes = 0;
    s.handled = None;
    assert_eq!(
        s.due(tz.with_ymd_and_hms(2026, 3, 8, 10, 0, 0).unwrap()),
        None
    );
    assert_eq!(
        s.due(tz.with_ymd_and_hms(2026, 4, 8, 10, 0, 0).unwrap()),
        Some(date("2026-04-08 02:30"))
    );
}

#[test]
fn recurrence_latest_agrees_with_calendar_for_every_day_in_four_years() {
    let mut s = event().schedule.unwrap();
    s.start = date("2024-02-29 09:00");
    for repeat in [Repeat::Monthly, Repeat::Yearly] {
        for clamp in [false, true] {
            s.repeat = repeat;
            s.clamp_missing_day = clamp;
            let mut last = None;
            for offset in 0..1462 {
                let day = s.start.date().checked_add_days(Days::new(offset)).unwrap();
                if s.on_day(day) {
                    last = Some(day.and_hms_opt(9, 0, 0).unwrap());
                }
                assert_eq!(
                    s.latest(day.and_hms_opt(10, 0, 0).unwrap()),
                    last,
                    "{repeat:?} {clamp} {day}"
                );
            }
        }
    }
}

#[test]
fn monthly_rules_persist_backup_restore_and_keep_legacy_json_compatible() {
    let path = fixture();
    let mut item = event();
    let legacy = serde_json::to_string(&item).unwrap();
    assert!(!legacy.contains("clamp_missing_day"));
    assert!(
        !serde_json::from_str::<Item>(&legacy)
            .unwrap()
            .schedule
            .unwrap()
            .clamp_missing_day
    );
    let s = item.schedule.as_mut().unwrap();
    s.start = date("2026-01-31 09:00");
    s.repeat = Repeat::Monthly;
    s.clamp_missing_day = true;
    store::save(&path, item).unwrap();
    let saved = store::load(&path).unwrap();
    let backup_path = path.with_extension("json");
    backup::write(&backup_path, &backup::Document::new(saved.clone()).unwrap()).unwrap();
    let doc = backup::read(&backup_path).unwrap();
    assert_eq!(doc.records, saved);
    let plan = backup::plan(&[], &doc, backup::Mode::ReplaceAll, true).unwrap();
    let restored_path = path.with_file_name("restored.sqlite3");
    let restored = store::restore(&restored_path, &[], &plan.records).unwrap();
    let rows = agenda::rows(&restored, date("2026-02-01 00:00").date(), 30, "", false);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].at, date("2026-02-28 09:00"));
    let mut state = State::new(path.clone());
    wait_state(&mut state);
    state.edit(saved[0].clone());
    state
        .draft
        .as_mut()
        .unwrap()
        .schedule
        .as_mut()
        .unwrap()
        .clamp_missing_day = false;
    assert!(state.has_unsaved());
    state.discard();
    assert!(!state.has_unsaved());
    state
        .draft
        .as_mut()
        .unwrap()
        .schedule
        .as_mut()
        .unwrap()
        .handled = Some(date("2026-02-28 09:00"));
    state.save_draft();
    wait_state(&mut state);
    state
        .draft
        .as_mut()
        .unwrap()
        .schedule
        .as_mut()
        .unwrap()
        .clamp_missing_day = false;
    state.save_draft();
    wait_state(&mut state);
    assert!(!state.error);
    assert!(
        state
            .draft
            .as_ref()
            .unwrap()
            .schedule
            .as_ref()
            .unwrap()
            .handled
            .is_none()
    );
    for file in [path.clone(), restored_path, backup_path] {
        std::fs::remove_file(file).unwrap();
    }
    std::fs::remove_dir(path.parent().unwrap()).unwrap();
}

#[test]
fn agenda_expands_actual_occurrences_across_leap_day_and_year() {
    let mut daily = event();
    let s = daily.schedule.as_mut().unwrap();
    s.start = date("2028-02-28 09:00");
    s.repeat = Repeat::Daily;
    let mut weekly = daily.clone();
    weekly.id = uuid::Uuid::new_v4().to_string();
    weekly.schedule.as_mut().unwrap().repeat = Repeat::Weekly;
    let items = vec![weekly, daily];
    let rows = agenda::rows(&items, date("2028-02-29 00:00").date(), 7, "", false);
    assert_eq!(rows.len(), 8);
    assert_eq!(rows[0].at, date("2028-02-29 09:00"));
    assert_eq!(rows.last().unwrap().at, date("2028-03-06 09:00"));
    assert_eq!(rows.iter().filter(|r| r.index == 0).count(), 1);
    let rows = agenda::rows(&items, date("2028-12-30 00:00").date(), 7, "", false);
    assert!(rows.iter().any(|r| r.at == date("2029-01-01 09:00")));
    let rows = agenda::rows(&items, date("2099-12-31 00:00").date(), 30, "", false);
    assert_eq!(rows.len(), 1);
    assert!(agenda::rows(&items, date("2028-02-20 00:00").date(), 7, "", false).is_empty());
}

#[test]
fn agenda_filters_saved_series_without_treating_ack_as_completion() {
    let mut active = event();
    active.body = "Project ALPHA".into();
    active.schedule.as_mut().unwrap().handled = Some(active.schedule.as_ref().unwrap().start);
    let mut completed = active.clone();
    completed.id = uuid::Uuid::new_v4().to_string();
    completed.schedule.as_mut().unwrap().done = true;
    let mut trashed = active.clone();
    trashed.id = uuid::Uuid::new_v4().to_string();
    trashed.trash = true;
    let items = vec![active, completed, trashed, Item::new(None)];
    let day = date("2026-10-02 00:00").date();
    assert_eq!(agenda::rows(&items, day, 7, " alpha ", false).len(), 1);
    assert_eq!(agenda::rows(&items, day, 30, "alpha", true).len(), 2);
    assert!(agenda::rows(&items, day, 7, "absent", true).is_empty());
    assert!(agenda::rows(&items, day.succ_opt().unwrap(), 7, "", true).is_empty());
}

#[test]
fn agenda_cache_invalidates_for_revision_order_range_and_filters() {
    let mut a = event();
    a.revision = 1;
    let mut b = a.clone();
    b.id = uuid::Uuid::new_v4().to_string();
    b.title = "Earlier".into();
    b.schedule.as_mut().unwrap().start = date("2026-10-02 08:00");
    let mut items = vec![a, b];
    let day = date("2026-10-02 00:00").date();
    let mut cache = agenda::Cache::default();
    assert!(cache.refresh(&items, day, 7, "", false));
    assert_eq!(cache.rows[0].index, 1);
    assert!(!cache.refresh(&items, day, 7, "  ", false));
    items.swap(0, 1);
    assert!(cache.refresh(&items, day, 7, "", false));
    assert_eq!(cache.rows[0].index, 0);
    items[0].trash = true;
    items[0].revision += 1;
    assert!(cache.refresh(&items, day, 7, "", false));
    assert_eq!(cache.rows.len(), 1);
    assert!(cache.refresh(&items, day, 30, "", false));
    assert!(cache.refresh(&items, day, 30, "", true));
    assert!(cache.refresh(&items, day, 30, "not found", true));
    assert!(cache.rows.is_empty());
    assert!(cache.refresh(&items, day.succ_opt().unwrap(), 30, "", false));
    assert!(cache.rows.is_empty());
}

#[test]
fn agenda_large_collection_is_bounded_and_opening_preserves_dirty_editor() {
    let mut item = event();
    item.revision = 1;
    item.schedule.as_mut().unwrap().repeat = Repeat::Daily;
    let mut items = Vec::new();
    for _ in 0..MAX_ITEMS {
        let mut copy = item.clone();
        copy.id = uuid::Uuid::new_v4().to_string();
        items.push(copy);
    }
    let day = item.schedule.as_ref().unwrap().start.date();
    let rows = agenda::rows(&items, day, 365, "", false);
    assert_eq!(rows.len(), MAX_ITEMS * 30);
    assert!(rows.windows(2).all(|r| r[0].at <= r[1].at));
    let mut state = State::new(fixture());
    wait_state(&mut state);
    state.items = items;
    let first = state.items[0].clone();
    state.edit(first.clone());
    state.draft.as_mut().unwrap().body = "unsaved".into();
    state.agenda_days = 30;
    assert!(
        state
            .open_event(
                &state.items[1].id.clone(),
                day.and_hms_opt(9, 0, 0).unwrap()
            )
            .is_err()
    );
    assert_eq!(state.agenda_days, 30);
    assert_eq!(state.draft.as_ref().unwrap().body, "unsaved");
    let later = day.succ_opt().unwrap().and_hms_opt(9, 0, 0).unwrap();
    state.open_event(&first.id, later).unwrap();
    assert_eq!(state.selected, later.date());
    assert_eq!(state.agenda_days, 1);
    assert_eq!(state.draft.as_ref().unwrap().body, "unsaved");
    assert_eq!(
        state
            .draft
            .as_ref()
            .unwrap()
            .schedule
            .as_ref()
            .unwrap()
            .start
            .date(),
        day
    );
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

pub(super) fn wait_state(state: &mut State) {
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

#[test]
fn incoming_result_is_explicit_unsaved_bounded_and_never_overwrites_edits() {
    let path = fixture();
    let mut state = State::new(path.clone());
    assert!(state.receive_text("report", "loading").is_err());
    wait_state(&mut state);
    assert!(
        state
            .receive_text("report", &"x".repeat(MAX_BODY + 1))
            .is_err()
    );
    assert!(state.receive_text("report", "").is_err());
    assert!(state.draft.is_none());
    let content = "# 中文结果\n\nKeep **markup** and JSON: {\"ok\":true}";
    state.receive_text(&"title\n".repeat(100), content).unwrap();
    assert!(!path.exists());
    assert!(state.has_unsaved());
    assert_eq!(state.transfer_text().unwrap().1, content);
    assert_eq!(state.draft.as_ref().unwrap().title.chars().count(), 120);
    assert!(state.receive_text("another", "overwrite").is_err());
    assert_eq!(state.transfer_text().unwrap().1, content);
    state.save_draft();
    assert!(state.receive_text("another", "while saving").is_err());
    wait_state(&mut state);
    assert_eq!(store::load(&path).unwrap()[0].body, content);
    state.receive_text("next", "new draft").unwrap();
    assert_eq!(store::load(&path).unwrap().len(), 1);
    state.discard();
    assert!(state.transfer_text().is_none());
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(path.parent().unwrap()).unwrap();
}

#[test]
fn open_reminder_preserves_unsaved_draft_and_does_not_acknowledge() {
    let path = fixture();
    let mut state = State::new(path.clone());
    wait_state(&mut state);
    let item = event();
    let at = item.schedule.as_ref().unwrap().start;
    state.items.push(item.clone());
    state.receive_text("another tool", "unsaved").unwrap();
    assert!(state.open_event(&item.id, at).is_err());
    assert_eq!(state.transfer_text().unwrap().1, "unsaved");
    state.discard();
    state.alarm_open = true;
    state.open_event(&item.id, at).unwrap();
    assert!(state.calendar);
    assert!(!state.alarm_open);
    assert_eq!(state.selected, at.date());
    assert!(
        state
            .draft
            .as_ref()
            .unwrap()
            .schedule
            .as_ref()
            .unwrap()
            .handled
            .is_none()
    );
    state.draft.as_mut().unwrap().body = "currently editing this event".into();
    state.open_event(&item.id, at).unwrap();
    assert_eq!(
        state.transfer_text().unwrap().1,
        "currently editing this event"
    );
    assert!(state.open_event("missing", at).is_err());
    assert!(!path.exists());
}

#[test]
fn purge_snapshot_is_atomic_and_excludes_live_and_later_records() {
    let path = fixture();
    let mut first = event();
    first.trash = true;
    let mut second = Item::new(None);
    second.title = "trashed memo".into();
    second.trash = true;
    let live = event();
    for item in [first, second, live.clone()] {
        store::save(&path, item).unwrap();
    }
    let reviewed: Vec<_> = store::load(&path)
        .unwrap()
        .into_iter()
        .filter(|i| i.trash)
        .collect();
    let mut restored = reviewed[1].clone();
    restored.trash = false;
    store::save(&path, restored.clone()).unwrap();
    assert!(store::purge(&path, &reviewed).is_err());
    let after = store::load(&path).unwrap();
    assert_eq!(after.len(), 3); // First delete was rolled back when second conflicted.
    assert!(after.iter().any(|i| i.id == reviewed[0].id));
    assert!(
        store::purge(
            &path,
            std::slice::from_ref(after.iter().find(|i| i.id == live.id).unwrap())
        )
        .is_err()
    );
    let fresh: Vec<_> = after.into_iter().filter(|i| i.trash).collect();
    let mut later = event();
    later.trash = true;
    let later_id = later.id.clone();
    store::save(&path, later).unwrap();
    assert!(store::purge(&path, &[fresh[0].clone(), fresh[0].clone()]).is_err());
    assert_eq!(store::load(&path).unwrap().len(), 4);
    let remaining = store::purge(&path, &fresh).unwrap();
    assert_eq!(remaining.len(), 3);
    assert!(remaining.iter().any(|i| i.id == later_id && i.trash));
    assert!(remaining.iter().any(|i| i.id == restored.id && !i.trash));
    assert_eq!(store::load(&path).unwrap(), remaining);
    assert!(store::purge(&path, &fresh).is_err());
    assert_eq!(store::load(&path).unwrap(), remaining);
    std::fs::remove_file(&path).unwrap();
    assert!(store::purge(&path, &fresh).is_err());
    assert!(!path.exists());
    std::fs::remove_dir(path.parent().unwrap()).unwrap();
}

#[test]
fn purge_requires_confirmation_protects_edits_and_clears_deleted_selection() {
    let path = fixture();
    let mut memo = Item::new(None);
    memo.title = "memo".into();
    memo.trash = true;
    let mut scheduled = event();
    scheduled.trash = true;
    for item in [memo.clone(), scheduled] {
        store::save(&path, item).unwrap();
    }
    let mut state = State::new(path.clone());
    wait_state(&mut state);
    state.edit(
        state
            .items
            .iter()
            .find(|i| i.id == memo.id)
            .unwrap()
            .clone(),
    );
    state.query = "no matches".into();
    state.calendar = true;
    let before = std::fs::read(&path).unwrap();
    assert!(state.confirm_purge().is_err());
    state.request_purge(true).unwrap();
    assert_eq!(state.purge_review.as_ref().unwrap().len(), 2);
    state.purge_review = None; // Cancel has no storage effect.
    assert_eq!(std::fs::read(&path).unwrap(), before);
    state.draft.as_mut().unwrap().body = "unsaved".into();
    assert!(state.request_purge(true).is_err());
    state.discard();
    state.request_purge(false).unwrap();
    assert_eq!(state.purge_review.as_ref().unwrap().len(), 1);
    state.confirm_purge().unwrap();
    assert!(state.saving() && state.has_unsaved());
    assert!(state.request_purge(true).is_err());
    wait_state(&mut state);
    assert!(!state.error && !state.saving() && !state.has_unsaved());
    assert!(state.draft.is_none() && state.original.is_none());
    assert_eq!(state.items.len(), 1);
    state.request_purge(true).unwrap();
    state.confirm_purge().unwrap();
    wait_state(&mut state);
    assert!(state.items.is_empty());
    state.new_draft(None);
    state.draft.as_mut().unwrap().title = "new".into();
    state.save_draft();
    wait_state(&mut state);
    assert_eq!(store::load(&path).unwrap().len(), 1);
    assert!(state.request_purge(true).is_err());
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(path.parent().unwrap()).unwrap();
}

#[test]
fn purge_failure_retains_selection_and_database_without_partial_deletion() {
    let path = fixture();
    let mut item = event();
    item.trash = true;
    store::save(&path, item).unwrap();
    let mut state = State::new(path.clone());
    wait_state(&mut state);
    state.edit(state.items[0].clone());
    state.request_purge(true).unwrap();
    let mut changed = state.items[0].clone();
    changed.body = "edited elsewhere".into();
    store::save(&path, changed).unwrap();
    let before = std::fs::read(&path).unwrap();
    state.confirm_purge().unwrap();
    wait_state(&mut state);
    assert!(state.error && state.deleting.is_none());
    assert!(state.draft.is_some());
    assert_eq!(state.items.len(), 1);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(path.parent().unwrap()).unwrap();
}

#[test]
fn text_file_import_preserves_markdown_unicode_and_line_endings() {
    let path = fixture();
    let root = path.parent().unwrap();
    std::fs::create_dir_all(root).unwrap();
    let source = root.join("项目计划.MD");
    let body =
        "---\r\ntags: [one]\r\n---\r\n# 标题\r\n\r\n<script>do not execute</script>\r\n中文 😀\n";
    let bytes = [vec![0xef, 0xbb, 0xbf], body.as_bytes().to_vec()].concat();
    std::fs::write(&source, &bytes).unwrap();
    let item = files::read_note(&source).unwrap();
    assert_eq!(item.title, "项目计划");
    assert_eq!(item.body, body);
    assert_eq!(item.revision, 0);
    assert!(item.schedule.is_none() && !item.trash);
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
    std::fs::write(&source, []).unwrap();
    assert!(files::read_note(&source).unwrap().body.is_empty());
    let max = [vec![0xef, 0xbb, 0xbf], vec![b'x'; MAX_BODY]].concat();
    std::fs::write(&source, &max).unwrap();
    assert_eq!(files::read_note(&source).unwrap().body.len(), MAX_BODY);
    std::fs::write(&source, vec![b'x'; MAX_BODY + 1]).unwrap();
    assert!(files::read_note(&source).is_err());
    std::fs::write(&source, [0xff, 0xfe, 0x41, 0]).unwrap();
    assert!(files::read_note(&source).is_err());
    std::fs::write(&source, b"text\0binary").unwrap();
    assert!(files::read_note(&source).is_err());
    assert!(files::read_note(root).is_err());
    let unsupported = root.join("document.pdf");
    std::fs::write(&unsupported, b"not markdown").unwrap();
    assert!(files::read_note(&unsupported).is_err());
    std::fs::remove_file(source).unwrap();
    std::fs::remove_file(unsupported).unwrap();
    std::fs::remove_dir(root).unwrap();
}

#[test]
fn text_file_export_is_exact_and_never_overwrites() {
    let path = fixture();
    let root = path.parent().unwrap();
    std::fs::create_dir_all(root).unwrap();
    let target = root.join("正文.md");
    let body = "# UTF-8 😀\r\n原文\n";
    assert_eq!(files::write_note(&target, body).unwrap(), body.len());
    assert_eq!(std::fs::read(&target).unwrap(), body.as_bytes());
    assert!(files::write_note(&target, "replace").is_err());
    assert_eq!(std::fs::read(&target).unwrap(), body.as_bytes());
    let oversized = root.join("large.txt");
    assert!(files::write_note(&oversized, &"x".repeat(MAX_BODY + 1)).is_err());
    assert!(!oversized.exists());
    let invalid = root.join("bad.exe");
    assert!(files::write_note(&invalid, body).is_err());
    assert!(!invalid.exists());
    let empty = root.join("empty.markdown");
    assert_eq!(files::write_note(&empty, "").unwrap(), 0);
    assert_eq!(std::fs::metadata(&empty).unwrap().len(), 0);
    assert!(files::write_note(&root.join("missing/child.md"), body).is_err());
    std::fs::remove_file(target).unwrap();
    std::fs::remove_file(empty).unwrap();
    std::fs::remove_dir(root).unwrap();
}

#[test]
fn import_file_is_a_guarded_unsaved_draft_and_failure_keeps_current_note() {
    let path = fixture();
    let root = path.parent().unwrap();
    std::fs::create_dir_all(root).unwrap();
    let source = root.join("导入.txt");
    std::fs::write(&source, "正文\r\n").unwrap();
    let mut state = State::new(path.clone());
    assert!(state.import_file(source.clone()).is_err());
    wait_state(&mut state);
    state.calendar = true;
    state.trash = true;
    state.query = "old filter".into();
    state.import_file(source.clone()).unwrap();
    assert!(state.saving() && state.has_unsaved());
    assert!(state.receive_text("tool", "do not overwrite").is_err());
    wait_state(&mut state);
    assert!(!state.error && state.has_unsaved() && !state.saving());
    assert!(!state.calendar && !state.trash && state.query.is_empty());
    assert!(!path.exists());
    assert_eq!(state.draft.as_ref().unwrap().title, "导入");
    assert_eq!(state.draft.as_ref().unwrap().body, "正文\r\n");
    assert!(state.import_file(source.clone()).is_err());
    state.save_draft();
    wait_state(&mut state);
    let saved = state.draft.clone();
    state.import_file(root.join("missing.md")).unwrap();
    wait_state(&mut state);
    assert!(state.error && !state.saving());
    assert_eq!(state.draft, saved);
    assert_eq!(std::fs::read_to_string(&source).unwrap(), "正文\r\n");
    std::fs::remove_file(source).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(root).unwrap();
}

#[test]
fn export_snapshots_current_unsaved_body_without_saving_or_event_metadata() {
    let path = fixture();
    let root = path.parent().unwrap();
    std::fs::create_dir_all(root).unwrap();
    let target = root.join("export.md");
    let mut state = State::new(path.clone());
    wait_state(&mut state);
    assert!(state.export_file(target.clone()).is_err());
    let mut item = event();
    item.title = "CON: test / title?".into();
    item.body = "未保存的正文\r\n".into();
    state.edit(item);
    state.review_export().unwrap();
    assert_eq!(
        state.export_review.as_ref().unwrap().filename,
        "CON_ test _ title_-正文.md"
    );
    assert!(!target.exists());
    state.export_review = None;
    assert!(!target.exists()); // Cancel preview.
    state.review_export().unwrap();
    state.draft.as_mut().unwrap().body = "changed after preview".into();
    state.export_file(target.clone()).unwrap();
    assert!(state.saving());
    wait_state(&mut state);
    assert!(!state.error && state.has_unsaved());
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "未保存的正文\r\n"
    );
    assert_eq!(state.draft.as_ref().unwrap().body, "changed after preview");
    assert!(!path.exists());
    state.review_export().unwrap();
    state.export_file(target.clone()).unwrap();
    wait_state(&mut state);
    assert!(state.error && state.has_unsaved());
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "未保存的正文\r\n"
    );
    state.draft.as_mut().unwrap().trash = true;
    assert!(state.review_export().is_err());
    std::fs::remove_file(target).unwrap();
    std::fs::remove_dir(root).unwrap();
}

#[test]
fn escaped_body_can_save_load_and_backup_at_the_body_limit() {
    let path = fixture();
    let mut item = event();
    item.body = "\u{0001}".repeat(MAX_BODY);
    store::save(&path, item).unwrap();
    let records = store::load(&path).unwrap();
    assert_eq!(records[0].body.len(), MAX_BODY);
    let doc = backup::Document::new(records.clone()).unwrap();
    let file = path.with_file_name("backup.json");
    backup::write(&file, &doc).unwrap();
    assert_eq!(backup::read(&file).unwrap().records, records);
    std::fs::remove_file(file).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(path.parent().unwrap()).unwrap();
}

#[test]
fn backup_roundtrip_preserves_full_schedule_trash_and_rejects_malformed_data() {
    let path = fixture();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = path.with_file_name("backup.json");
    let mut item = event();
    item.revision = 8;
    item.trash = true;
    item.pinned = true;
    item.updated = Local::now().timestamp();
    item.body = "中文\nfull body".into();
    let schedule = item.schedule.as_mut().unwrap();
    schedule.repeat = Repeat::Weekly;
    schedule.handled = Some(schedule.start);
    schedule.snooze = Some((schedule.start, Local::now().timestamp() + 600));
    let doc = backup::Document::new(vec![item.clone()]).unwrap();
    backup::write(&file, &doc).unwrap();
    let bytes = std::fs::read(&file).unwrap();
    assert_eq!(backup::read(&file).unwrap().records, vec![item.clone()]);
    assert!(backup::write(&file, &doc).is_err());
    assert_eq!(std::fs::read(&file).unwrap(), bytes);
    let original: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for variant in 0..7 {
        let mut bad = original.clone();
        match variant {
            0 => bad["version"] = 2.into(),
            1 => bad["records"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::to_value(&item).unwrap()),
            2 => bad["records"][0]["revision"] = 0.into(),
            3 => bad["records"][0]["unknown-field"] = true.into(),
            4 => bad["created_at"] = i64::MAX.into(),
            5 => bad["records"][0]["updated"] = i64::MAX.into(),
            _ => bad["records"][0]["schedule"]["snooze"][1] = i64::MAX.into(),
        }
        std::fs::write(&file, serde_json::to_vec(&bad).unwrap()).unwrap();
        assert!(backup::read(&file).is_err(), "variant {variant}");
    }
    std::fs::remove_file(file).unwrap();
    std::fs::remove_dir(path.parent().unwrap()).unwrap();
}

#[test]
fn restore_modes_count_changes_and_reminders_require_choice() {
    let mut shared = event();
    shared.revision = 1;
    shared.title = "current".into();
    let mut local = shared.clone();
    local.id = uuid::Uuid::new_v4().to_string();
    local.title = "local only".into();
    let mut incoming = shared.clone();
    incoming.title = "backup content".into();
    let mut new = incoming.clone();
    new.id = uuid::Uuid::new_v4().to_string();
    new.trash = true;
    let current = vec![shared.clone(), local];
    let doc = backup::Document::new(vec![incoming, new]).unwrap();
    let keep = backup::plan(&current, &doc, backup::Mode::KeepCurrent, false).unwrap();
    assert_eq!(
        (keep.added, keep.updated, keep.removed, keep.kept),
        (1, 0, 0, 2)
    );
    assert_eq!(
        keep.records.iter().find(|i| i.id == shared.id).unwrap(),
        &shared
    );
    assert!(
        !keep
            .records
            .iter()
            .find(|i| i.trash)
            .unwrap()
            .schedule
            .as_ref()
            .unwrap()
            .remind
    );
    let merge = backup::plan(&current, &doc, backup::Mode::BackupWins, true).unwrap();
    assert_eq!(
        (merge.added, merge.updated, merge.removed, merge.kept),
        (1, 1, 0, 1)
    );
    assert!(
        merge
            .records
            .iter()
            .all(|i| i.schedule.as_ref().unwrap().remind)
    );
    let replace = backup::plan(&current, &doc, backup::Mode::ReplaceAll, true).unwrap();
    assert_eq!(
        (
            replace.added,
            replace.updated,
            replace.removed,
            replace.kept
        ),
        (1, 1, 1, 0)
    );
    let mut restored = doc.records.clone();
    for i in &mut restored {
        i.revision += 10;
    }
    let same = backup::plan(&restored, &doc, backup::Mode::ReplaceAll, true).unwrap();
    assert!(same.changes.is_empty());
    assert_eq!(same.records, restored);
}

#[test]
fn restore_transaction_rejects_stale_preview_and_stale_editor_versions() {
    let path = fixture();
    let mut original = event();
    original.body = "old".into();
    store::save(&path, original).unwrap();
    let before = store::load(&path).unwrap();
    let mut imported = before.clone();
    imported[0].body = "from backup".into();
    let mut concurrent = before[0].clone();
    concurrent.title = "concurrent".into();
    store::save(&path, concurrent).unwrap();
    let changed = store::load(&path).unwrap();
    assert!(store::restore(&path, &before, &imported).is_err());
    assert_eq!(store::load(&path).unwrap(), changed);
    let restored = store::restore(&path, &changed, &imported).unwrap();
    assert_eq!(restored[0].body, "from backup");
    assert_ne!(restored[0].revision, before[0].revision);
    assert_ne!(restored[0].revision, changed[0].revision);
    assert!(store::save(&path, changed[0].clone()).is_err());
    let mut edit = restored[0].clone();
    edit.title = "valid edit".into();
    store::save(&path, edit).unwrap();
    let current = store::load(&path).unwrap();
    assert_eq!(current[0].revision, restored[0].revision + 1);
    std::fs::remove_file(&path).unwrap();
    assert!(store::restore(&path, &current, &imported).is_err());
    assert!(!path.exists());
    std::fs::remove_dir(path.parent().unwrap()).unwrap();
}

#[test]
fn invalid_restore_rolls_back_and_empty_backup_can_explicitly_replace_all() {
    let path = fixture();
    store::save(&path, event()).unwrap();
    let before = store::load(&path).unwrap();
    let duplicate = vec![before[0].clone(), before[0].clone()];
    assert!(store::restore(&path, &before, &duplicate).is_err());
    assert_eq!(store::load(&path).unwrap(), before);
    let doc = backup::Document::new(Vec::new()).unwrap();
    let plan = backup::plan(&before, &doc, backup::Mode::ReplaceAll, false).unwrap();
    assert_eq!(plan.removed, 1);
    assert!(
        store::restore(&path, &before, &plan.records)
            .unwrap()
            .is_empty()
    );
    let recreated = store::restore(&path, &[], &before).unwrap();
    assert_ne!(recreated[0].revision, before[0].revision);
    assert!(store::save(&path, before[0].clone()).is_err());
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(path.parent().unwrap()).unwrap();
}

#[test]
fn backup_restore_workflow_requires_review_confirmation_and_preserves_failure_state() {
    let path = fixture();
    store::save(&path, event()).unwrap();
    let mut state = State::new(path.clone());
    wait_state(&mut state);
    state.prepare_backup().unwrap();
    wait_state(&mut state);
    let file = path.with_file_name("backup.json");
    assert!(!file.exists());
    state.backup_review = None;
    assert!(!file.exists());
    state.prepare_backup().unwrap();
    wait_state(&mut state);
    state.save_backup(file.clone()).unwrap();
    assert!(state.saving());
    wait_state(&mut state);
    assert!(!state.error);
    let mut changed = state.items[0].clone();
    changed.body = "new local content".into();
    store::save(&path, changed).unwrap();
    state.read_backup(file.clone()).unwrap();
    wait_state(&mut state);
    let Some(backup::Review::Restore(review)) = state.backup_review.as_mut() else {
        panic!()
    };
    review.mode = backup::Mode::BackupWins;
    review.reminders = true;
    review.rebuild();
    assert_eq!(review.plan.as_ref().unwrap().updated, 1);
    assert!(state.restore_backup().is_err());
    let Some(backup::Review::Restore(review)) = state.backup_review.as_mut() else {
        panic!()
    };
    review.confirmed = true;
    let mut newer = store::load(&path).unwrap().remove(0);
    newer.body = "another writer".into();
    store::save(&path, newer).unwrap();
    state.restore_backup().unwrap();
    wait_state(&mut state);
    assert!(state.error);
    assert_eq!(store::load(&path).unwrap()[0].body, "another writer");
    state.read_backup(file.clone()).unwrap();
    wait_state(&mut state);
    let Some(backup::Review::Restore(review)) = state.backup_review.as_mut() else {
        panic!()
    };
    review.mode = backup::Mode::BackupWins;
    review.reminders = true;
    review.rebuild();
    review.confirmed = true;
    state.restore_backup().unwrap();
    assert!(state.saving() && state.has_unsaved());
    wait_state(&mut state);
    assert!(!state.error && !state.has_unsaved());
    assert!(state.items[0].body.is_empty());
    assert!(state.draft.is_none());
    state.new_draft(None);
    state.draft.as_mut().unwrap().title = "unsaved".into();
    assert!(state.prepare_backup().is_err());
    assert!(state.read_backup(file.clone()).is_err());
    std::fs::remove_file(file).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(path.parent().unwrap()).unwrap();
}

#[test]
fn backup_and_restore_enforce_record_byte_and_file_limits() {
    let mut records = Vec::new();
    for _ in 0..MAX_ITEMS {
        let mut item = event();
        item.revision = 1;
        records.push(item);
    }
    let mut extra = event();
    extra.revision = 1;
    let doc = backup::Document::new(vec![extra.clone()]).unwrap();
    assert!(backup::plan(&records, &doc, backup::Mode::KeepCurrent, false).is_err());
    records.push(extra);
    assert!(backup::Document::new(records.clone()).is_err());
    let path = fixture();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = path.with_file_name("large.json");
    let raw = serde_json::json!({"format":"zi-devtools-planner","version":1,"created_at":0,"records":records});
    std::fs::write(&file, serde_json::to_vec(&raw).unwrap()).unwrap();
    assert!(backup::read(&file).is_err());
    std::fs::File::create(&file)
        .unwrap()
        .set_len(40 * 1024 * 1024 + 1)
        .unwrap();
    assert!(backup::read(&file).is_err());
    let mut large = Vec::new();
    for _ in 0..44 {
        let mut item = event();
        item.revision = 1;
        item.body = "\u{0001}".repeat(MAX_BODY);
        large.push(item);
    }
    assert!(backup::Document::new(large).is_err());
    std::fs::remove_file(file).unwrap();
    std::fs::remove_dir(path.parent().unwrap()).unwrap();
}

#[test]
fn restore_rolls_back_after_a_midtransaction_insert_failure() {
    let path = fixture();
    store::save(&path, event()).unwrap();
    let before = store::load(&path).unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_restore BEFORE INSERT ON records WHEN NEW.payload LIKE '%force_restore_abort%' BEGIN SELECT RAISE(ABORT, 'synthetic write failure'); END;").unwrap();
    drop(connection);
    let mut first = Item::new(None);
    first.revision = 1;
    first.title = "first insertion succeeds".into();
    let mut second = first.clone();
    second.id = uuid::Uuid::new_v4().to_string();
    second.title = "force_restore_abort".into();
    assert!(store::restore(&path, &before, &[first, second]).is_err());
    assert_eq!(store::load(&path).unwrap(), before);
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(path.parent().unwrap()).unwrap();
}
