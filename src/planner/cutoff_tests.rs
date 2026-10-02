use super::tests::{date, event, fixture, now, wait_state};
use super::*;

#[test]
fn cutoff_bounds_every_rule_and_latest_without_changing_earlier_occurrences() {
    for repeat in [
        Repeat::Daily,
        Repeat::Weekly,
        Repeat::Monthly,
        Repeat::Yearly,
    ] {
        for clamp in [false, true] {
            if clamp && !matches!(repeat, Repeat::Monthly | Repeat::Yearly) {
                continue;
            }
            let mut s = event().schedule.unwrap();
            s.start = date("2024-02-29 09:00");
            s.repeat = repeat;
            s.clamp_missing_day = clamp;
            let unlimited = s.clone();
            let cutoff = date("2027-03-01 00:00").date();
            s.repeat_until = Some(cutoff);
            let mut last = None;
            for offset in 0..2200 {
                let day = s.start.date() + Days::new(offset);
                let expected = day <= cutoff && unlimited.on_day(day);
                assert_eq!(s.on_day(day), expected, "{repeat:?} {clamp} {day}");
                if expected {
                    last = Some(day.and_time(s.start.time()));
                }
                assert_eq!(s.latest(day.and_hms_opt(23, 59, 59).unwrap()), last);
            }
        }
    }
}

#[test]
fn cutoff_keeps_last_cross_day_event_and_carry_in_but_stops_future_starts() {
    let mut item = event();
    let s = item.schedule.as_mut().unwrap();
    s.start = date("2026-10-02 21:00");
    s.end = Some(date("2026-10-04 09:00"));
    s.repeat = Repeat::Weekly;
    s.repeat_until = Some(date("2026-10-09 00:00").date());
    let last = date("2026-10-09 21:00");
    assert_eq!(s.covering(date("2026-10-11 00:00").date()), Some(last));
    assert_eq!(s.covering(date("2026-10-12 00:00").date()), None);
    let rows = agenda::rows(
        &[item.clone()],
        date("2026-10-10 00:00").date(),
        30,
        "",
        false,
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].at, last);
    let s = item.schedule.as_mut().unwrap();
    s.start = date("2026-10-02 00:00");
    s.end = Some(date("2026-10-05 00:00"));
    s.all_day = true;
    s.reminder_time = Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap());
    assert_eq!(
        s.covering(date("2026-10-11 00:00").date()),
        Some(date("2026-10-09 00:00"))
    );
    assert_eq!(s.covering(date("2026-10-12 00:00").date()), None);
}

#[test]
fn cutoff_reminders_catch_up_last_only_and_keep_snooze_and_acknowledgment() {
    let mut s = event().schedule.unwrap();
    s.repeat = Repeat::Daily;
    s.minutes = 10;
    s.repeat_until = Some(date("2026-10-03 00:00").date());
    let last = date("2026-10-03 09:00");
    assert_eq!(s.due(now("2026-10-03 08:50")), Some(last));
    assert_eq!(s.due(now("2026-10-30 09:00")), Some(last));
    s.snooze = Some((last, now("2026-10-30 09:10").timestamp()));
    assert_eq!(s.due(now("2026-10-30 09:09")), None);
    assert_eq!(s.due(now("2026-10-30 09:10")), Some(last));
    s.handled = Some(last);
    assert_eq!(s.due(now("2026-11-30 09:00")), None);
    s.repeat_until = Some(date("2026-10-04 00:00").date());
    assert_eq!(
        s.due(now("2026-11-30 09:00")),
        Some(date("2026-10-04 09:00"))
    );
}

#[test]
fn cutoff_validates_and_round_trips_store_backup_without_touching_legacy_json() {
    let mut item = event();
    let json = serde_json::to_string(&item).unwrap();
    assert!(!json.contains("repeat_until"));
    assert!(
        serde_json::from_str::<Item>(&json)
            .unwrap()
            .schedule
            .unwrap()
            .repeat_until
            .is_none()
    );
    item.schedule.as_mut().unwrap().repeat_until = Some(date("2026-10-09 00:00").date());
    assert!(item.validate().is_err());
    item.schedule.as_mut().unwrap().repeat = Repeat::Weekly;
    assert!(item.validate().is_ok());
    for day in ["2026-10-01 00:00", "2100-01-01 00:00"] {
        let mut invalid = item.clone();
        invalid.schedule.as_mut().unwrap().repeat_until = Some(date(day).date());
        assert!(invalid.validate().is_err());
    }
    let path = fixture();
    store::save(&path, item).unwrap();
    let saved = store::load(&path).unwrap();
    let file = path.with_extension("json");
    backup::write(&file, &backup::Document::new(saved.clone()).unwrap()).unwrap();
    let restored = backup::read(&file).unwrap();
    assert_eq!(restored.records, saved);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn cutoff_editor_rejects_invalid_text_retains_draft_and_preserves_handled_on_save() {
    let path = fixture();
    let mut item = event();
    let s = item.schedule.as_mut().unwrap();
    s.repeat = Repeat::Daily;
    s.handled = Some(s.start);
    s.remind = false;
    store::save(&path, item).unwrap();
    let mut state = State::new(path.clone());
    wait_state(&mut state);
    state.edit(state.items[0].clone());
    let before = store::load(&path).unwrap();
    state
        .draft
        .as_mut()
        .unwrap()
        .schedule
        .as_mut()
        .unwrap()
        .repeat_until = Some(date("2026-10-09 00:00").date());
    for text in ["invalid", "2026-10-01"] {
        state.repeat_until_text = text.into();
        assert!(state.has_unsaved());
        state.save_draft();
        assert!(state.error && state.pending.is_none());
        assert_eq!(store::load(&path).unwrap(), before);
        assert_eq!(state.repeat_until_text, text);
    }
    state.repeat_until_text = "2026-10-09".into();
    state.save_draft();
    wait_state(&mut state);
    assert!(!state.error && !state.has_unsaved());
    let saved = store::load(&path).unwrap();
    let s = saved[0].schedule.as_ref().unwrap();
    assert_eq!(s.repeat_until, Some(date("2026-10-09 00:00").date()));
    assert_eq!(s.handled, Some(s.start));
    state.repeat_until_text = "2026-10-10".into();
    assert!(state.has_unsaved());
    state.discard();
    assert_eq!(state.repeat_until_text, "2026-10-09");
    assert!(!state.has_unsaved());
    state
        .draft
        .as_mut()
        .unwrap()
        .schedule
        .as_mut()
        .unwrap()
        .repeat = Repeat::Once;
    state.save_draft();
    wait_state(&mut state);
    assert!(!state.error);
    assert!(
        store::load(&path).unwrap()[0]
            .schedule
            .as_ref()
            .unwrap()
            .repeat_until
            .is_none()
    );
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

fn calendar(start: &str, rule: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:cutoff@test.invalid\r\nSUMMARY:截止样例\r\nDTSTART{start}\r\nRRULE:{rule}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    )
}
#[test]
fn cutoff_ics_until_matches_time_type_and_inclusive_start_boundary() {
    for (until, last) in [
        ("20261009T090000", "2026-10-09 09:00"),
        ("20261009T085959", "2026-10-08 09:00"),
        ("20261009T235959", "2026-10-09 09:00"),
    ] {
        let parsed = ics::parse(&calendar(
            ":20261002T090000",
            &format!("FREQ=DAILY;UNTIL={until}"),
        ))
        .unwrap();
        let s = parsed.records[0].schedule.as_ref().unwrap();
        assert_eq!(s.latest(date("2027-01-01 00:00")), Some(date(last)));
        let exported = ics::export(&parsed.records, false).unwrap();
        let again = ics::parse(&exported).unwrap();
        for offset in 0..20 {
            let day = date("2026-10-01 00:00").date() + Days::new(offset);
            assert_eq!(
                s.on_day(day),
                again.records[0].schedule.as_ref().unwrap().on_day(day)
            );
        }
    }
    let all_day = ics::parse(&calendar(
        ";VALUE=DATE:20261002",
        "FREQ=DAILY;UNTIL=20261009",
    ))
    .unwrap();
    assert_eq!(
        all_day.records[0].schedule.as_ref().unwrap().repeat_until,
        Some(date("2026-10-09 00:00").date())
    );
    assert!(
        ics::export(&all_day.records, false)
            .unwrap()
            .contains("UNTIL=20261009\r\n")
    );
    for (start, until) in [
        (":20261002T090000", "20261009"),
        (":20261002T090000", "20261009T090000Z"),
        (";VALUE=DATE:20261002", "20261009T090000"),
        (":20261002T090000", "20261002T085959"),
        (";VALUE=DATE:20261002", "20261001"),
        (":20261002T090000", "21001009T090000"),
    ] {
        assert!(ics::parse(&calendar(start, &format!("FREQ=DAILY;UNTIL={until}"))).is_err());
    }
    assert!(
        ics::parse(&calendar(
            ":20261002T090000",
            "FREQ=DAILY;UNTIL=20261009T090000;COUNT=2"
        ))
        .is_err()
    );
}

#[test]
fn cutoff_ics_changed_end_is_reviewed_and_default_ceiling_is_equivalent() {
    let original = ics::parse(&calendar(":20261002T090000", "FREQ=DAILY"))
        .unwrap()
        .records;
    let finite = ics::parse(&calendar(
        ":20261002T090000",
        "FREQ=DAILY;UNTIL=20261009T090000",
    ))
    .unwrap()
    .records;
    assert_eq!(ics_ui::plan(&original, &finite, false).unwrap().skipped, 1);
    let applied = ics_ui::plan(&original, &finite, true).unwrap();
    assert_eq!(applied.updated, 1);
    assert_eq!(
        ics_ui::plan(&applied.records, &finite, true)
            .unwrap()
            .unchanged,
        1
    );
    let mut ceiling = original.clone();
    ceiling[0].schedule.as_mut().unwrap().repeat_until = Some(date("2099-12-31 00:00").date());
    assert_eq!(
        ics_ui::plan(&original, &ceiling, true).unwrap().unchanged,
        1
    );
}
