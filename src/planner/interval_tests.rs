use super::tests::{date, event, fixture, now, wait_state};
use super::*;

fn all_day() -> Item {
    let mut item = event();
    let s = item.schedule.as_mut().unwrap();
    s.start = date("2026-10-02 00:00");
    s.end = Some(date("2026-10-05 00:00"));
    s.all_day = true;
    s.reminder_time = Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap());
    item
}

#[test]
fn interval_boundaries_and_carry_in_are_consistent() {
    let mut item = event();
    let s = item.schedule.as_mut().unwrap();
    s.start = date("2026-10-02 23:00");
    s.end = Some(date("2026-10-04 00:00"));
    assert_eq!(s.covering(date("2026-10-03 00:00").date()), Some(s.start));
    assert_eq!(s.covering(date("2026-10-04 00:00").date()), None);
    assert_eq!(s.covering(date("2026-10-01 00:00").date()), None);
    let start = s.start;
    let items = [item];
    let rows = agenda::rows(&items, date("2026-10-03 00:00").date(), 7, "", false);
    assert_eq!(
        rows,
        vec![agenda::Row {
            index: 0,
            at: start
        }]
    );
    assert!(agenda::rows(&items, date("2026-10-03 00:00").date(), 0, "", false).is_empty());
    assert!(agenda::rows(&items, date("2026-10-04 00:00").date(), 7, "", false).is_empty());
}

#[test]
fn daily_spans_include_prior_and_new_occurrences_once() {
    let mut item = event();
    let s = item.schedule.as_mut().unwrap();
    s.repeat = Repeat::Daily;
    s.end = Some(s.start + Duration::days(1));
    let rows = agenda::rows(&[item], date("2026-10-03 00:00").date(), 2, "", false);
    assert_eq!(
        rows.iter().map(|r| r.at).collect::<Vec<_>>(),
        vec![
            date("2026-10-02 09:00"),
            date("2026-10-03 09:00"),
            date("2026-10-04 09:00")
        ]
    );
}

#[test]
fn all_day_reminders_use_clock_but_keep_occurrence_identity() {
    let mut s = all_day().schedule.unwrap();
    assert!(s.due(now("2026-10-02 08:49")).is_none());
    assert_eq!(s.due(now("2026-10-02 08:50")), Some(s.start));
    s.handled = Some(s.start);
    assert!(s.due(now("2026-10-03 18:00")).is_none());
    s.handled = None;
    s.minutes = 600;
    assert_eq!(s.due(now("2026-10-01 23:00")), Some(s.start));
    s.snooze = Some((s.start, now("2026-10-02 09:00").timestamp()));
    assert!(s.due(now("2026-10-02 08:59")).is_none());
    assert_eq!(s.due(now("2026-10-02 09:00")), Some(s.start));
    s.snooze = None;
    s.repeat = Repeat::Weekly;
    assert_eq!(
        s.due(now("2026-10-16 09:00")),
        Some(date("2026-10-16 00:00"))
    );
}

#[test]
fn interval_validation_rejects_invalid_and_overlapping_ranges() {
    let mut item = all_day();
    item.validate().unwrap();
    item.schedule.as_mut().unwrap().repeat = Repeat::Daily;
    assert!(item.validate().is_err());
    item.schedule.as_mut().unwrap().repeat = Repeat::Once;
    item.schedule.as_mut().unwrap().end = Some(date("2026-10-01 00:00"));
    assert!(item.validate().is_err());
    item.schedule.as_mut().unwrap().end = Some(date("2026-10-03 09:00"));
    assert!(item.validate().is_err());
    item.schedule.as_mut().unwrap().end = Some(date("2100-01-01 00:00"));
    item.validate().unwrap();
    item.schedule.as_mut().unwrap().end = Some(date("2100-01-02 00:00"));
    assert!(item.validate().is_err());
    item.schedule.as_mut().unwrap().end = None;
    assert!(item.validate().is_err());
}

#[test]
fn all_day_storage_backup_and_old_data_round_trip() {
    let legacy = serde_json::to_string(&event()).unwrap();
    for field in ["end", "all_day", "reminder_time"] {
        assert!(!legacy.contains(&format!("\"{field}\"")));
    }
    assert!(
        !serde_json::from_str::<Item>(&legacy)
            .unwrap()
            .schedule
            .unwrap()
            .all_day
    );
    let path = fixture();
    store::save(&path, all_day()).unwrap();
    let saved = store::load(&path).unwrap();
    let backup_path = path.with_extension("json");
    backup::write(&backup_path, &backup::Document::new(saved.clone()).unwrap()).unwrap();
    assert_eq!(backup::read(&backup_path).unwrap().records, saved);
    let doc = backup::read(&backup_path).unwrap();
    let plan = backup::plan(&[], &doc, backup::Mode::ReplaceAll, false).unwrap();
    let restored =
        store::restore(&path.with_file_name("restored.sqlite3"), &[], &plan.records).unwrap();
    let restored_schedule = restored[0].schedule.as_ref().unwrap();
    assert!(restored_schedule.all_day && !restored_schedule.remind);
    assert_eq!(
        restored_schedule.end,
        saved[0].schedule.as_ref().unwrap().end
    );
    assert_eq!(
        restored_schedule.reminder_time,
        saved[0].schedule.as_ref().unwrap().reminder_time
    );
    assert_eq!(
        saved[0]
            .schedule
            .as_ref()
            .unwrap()
            .range_label(date("2026-10-02 00:00")),
        "2026-10-02 至 2026-10-04 · 全天"
    );
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn interval_draft_save_discard_and_reminder_reset() {
    let path = fixture();
    let mut item = all_day();
    item.schedule.as_mut().unwrap().handled = Some(date("2026-10-02 00:00"));
    store::save(&path, item).unwrap();
    let mut state = State::new(path.clone());
    wait_state(&mut state);
    state.edit(state.items[0].clone());
    assert_eq!(state.time_text, "09:00");
    assert_eq!(state.end_date_text, "2026-10-04");
    assert!(!state.has_unsaved());
    state.end_date_text = "bad date".into();
    assert!(state.has_unsaved());
    state.save_draft();
    assert!(state.error && state.pending.is_none());
    assert_eq!(state.end_date_text, "bad date");
    state.discard();
    assert!(!state.has_unsaved());
    state.end_date_text = "2026-10-05".into();
    state.save_draft();
    wait_state(&mut state);
    let s = state.items[0].schedule.as_ref().unwrap();
    assert_eq!(s.end, Some(date("2026-10-06 00:00")));
    assert!(
        s.handled.is_some(),
        "end-only changes should not rearm a reminder"
    );
    state.time_text = "10:00".into();
    state.save_draft();
    wait_state(&mut state);
    assert!(state.items[0].schedule.as_ref().unwrap().handled.is_none());
    assert!(!state.has_unsaved());
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn timed_editor_end_is_optional_and_preserves_failed_input() {
    let path = fixture();
    let mut state = State::new(path.clone());
    wait_state(&mut state);
    state.edit(event());
    state.draft.as_mut().unwrap().schedule.as_mut().unwrap().end = Some(date("2026-10-02 10:00"));
    state.end_date_text = "2026-10-02".into();
    state.end_time_text = "08:00".into();
    state.save_draft();
    assert!(state.error && state.pending.is_none());
    assert!(!path.exists());
    state.end_time_text = "10:30".into();
    state.save_draft();
    wait_state(&mut state);
    assert!(!state.error);
    assert_eq!(
        state.items[0].schedule.as_ref().unwrap().end,
        Some(date("2026-10-02 10:30"))
    );
    state.draft.as_mut().unwrap().schedule.as_mut().unwrap().end = None;
    state.end_date_text = "unused invalid text".into();
    state.save_draft();
    wait_state(&mut state);
    assert!(!state.error && !state.has_unsaved());
    assert!(state.items[0].schedule.as_ref().unwrap().end.is_none());
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn repeating_ranges_keep_duration_and_cross_month_and_leap_year() {
    let mut item = all_day();
    let s = item.schedule.as_mut().unwrap();
    s.start = date("2028-01-31 00:00");
    s.end = Some(date("2028-02-03 00:00"));
    s.repeat = Repeat::Monthly;
    s.clamp_missing_day = true;
    assert_eq!(
        s.covering(date("2028-03-02 00:00").date()),
        Some(date("2028-02-29 00:00"))
    );
    assert_eq!(s.covering(date("2028-03-03 00:00").date()), None);
    s.repeat = Repeat::Yearly;
    s.start = date("2028-02-29 00:00");
    s.end = Some(date("2028-03-02 00:00"));
    assert_eq!(
        s.covering(date("2029-03-01 00:00").date()),
        Some(date("2029-02-28 00:00"))
    );
    assert_eq!(s.covering(date("2029-03-02 00:00").date()), None);
}

#[test]
fn all_day_dst_clock_and_year_ceiling_respect_reminder_identity() {
    let tz = chrono_tz::America::New_York;
    let mut s = all_day().schedule.unwrap();
    s.start = date("2026-03-07 00:00");
    s.end = Some(date("2026-03-08 00:00"));
    s.repeat = Repeat::Daily;
    s.minutes = 0;
    s.reminder_time = Some(NaiveTime::from_hms_opt(2, 30, 0).unwrap());
    assert!(
        s.due(tz.with_ymd_and_hms(2026, 3, 8, 10, 0, 0).unwrap())
            .is_none()
    );
    assert_eq!(
        s.due(tz.with_ymd_and_hms(2026, 3, 9, 3, 0, 0).unwrap()),
        Some(date("2026-03-09 00:00"))
    );
    s.reminder_time = Some(NaiveTime::from_hms_opt(1, 30, 0).unwrap());
    let repeated = tz.from_local_datetime(&date("2026-11-01 01:40"));
    let occurrence = s.due(repeated.earliest().unwrap()).unwrap();
    assert_eq!(occurrence, date("2026-11-01 00:00"));
    s.handled = Some(occurrence);
    assert!(s.due(repeated.latest().unwrap()).is_none());
    s.handled = None;
    s.start = date("2099-12-31 00:00");
    s.end = Some(date("2100-01-01 00:00"));
    let item = Item {
        schedule: Some(s.clone()),
        ..event()
    };
    item.validate().unwrap();
    assert_eq!(
        agenda::rows(&[item], s.start.date(), 30, "", false).len(),
        1
    );
    assert!(s.covering(date("2100-01-01 00:00").date()).is_none());
}
