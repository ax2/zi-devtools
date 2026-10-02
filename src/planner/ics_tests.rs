use super::tests::{date, event, fixture, wait_state};
use super::*;

fn calendar(event: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Fixture//Calendar//EN\r\nBEGIN:VEVENT\r\nUID:fixture@calendar.test\r\nSUMMARY:日程\r\n{event}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    )
}

#[test]
fn ics_export_import_preserves_unicode_escaping_whitespace_uid_and_ranges() {
    let mut item = event();
    item.body = format!(
        "正文含,分号;反斜杠\\与换行\n{}\n末尾空格  ",
        "中文内容 ".repeat(80)
    );
    item.schedule.as_mut().unwrap().end = Some(date("2026-10-03 10:00"));
    let text = ics::export(&[item.clone()], false).unwrap();
    assert!(text.contains("\r\n "));
    for line in text.split("\r\n") {
        assert!(line.len() <= 75);
    }
    let parsed = ics::parse(&text).unwrap().records.remove(0);
    assert_eq!(parsed.id, item.id);
    assert_eq!(parsed.title, item.title);
    assert_eq!(parsed.body, item.body);
    assert_eq!(
        parsed.schedule.as_ref().unwrap().end,
        item.schedule.as_ref().unwrap().end
    );
    assert!(!parsed.schedule.as_ref().unwrap().remind);
    let again = ics::parse(&ics::export(&[parsed.clone()], false).unwrap())
        .unwrap()
        .records
        .remove(0);
    assert_eq!(again.id, parsed.id);
    assert_eq!(again.calendar_uid, parsed.calendar_uid);
}

#[test]
fn ics_external_uid_is_stable_and_text_properties_preserve_folded_spaces() {
    let text = calendar(
        "DTSTART:20261002T090000\r\nDESCRIPTION:line one  \r\n continuation\\nnext  \r\nLOCATION:一楼\\,会议室\r\nURL:https://example.invalid/meeting",
    );
    let a = ics::parse(&text).unwrap();
    let b = ics::parse(&text).unwrap();
    assert_eq!(a.records[0].id, b.records[0].id);
    assert!(
        a.records[0]
            .body
            .starts_with("line one  continuation\nnext  \n\n地点：一楼,会议室")
    );
    assert!(
        a.records[0]
            .body
            .ends_with("链接：https://example.invalid/meeting")
    );
    let exported = ics::export(&a.records, false).unwrap();
    assert!(exported.contains("UID:fixture@calendar.test\r\n"));
}

#[test]
fn ics_all_day_end_is_exclusive_and_missing_end_means_one_day() {
    let parsed = ics::parse(&calendar("DTSTART;VALUE=DATE:20261002")).unwrap();
    let s = parsed.records[0].schedule.as_ref().unwrap();
    assert!(s.all_day);
    assert_eq!(s.end, Some(date("2026-10-03 00:00")));
    assert_eq!(
        s.reminder_time,
        Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap())
    );
    assert!(s.covering(date("2026-10-03 00:00").date()).is_none());
    let text = ics::export(&parsed.records, false).unwrap();
    assert!(text.contains("DTEND;VALUE=DATE:20261003\r\n"));
    assert!(
        ics::parse(&calendar(
            "DTSTART;VALUE=DATE:20261002\r\nDTEND;VALUE=DATE:20261002"
        ))
        .is_err()
    );
    assert!(
        ics::parse(&calendar(
            "DTSTART;VALUE=DATE:20261002\r\nDTEND:20261003T000000"
        ))
        .is_err()
    );
}

#[test]
fn ics_repeat_export_round_trips_dates_for_supported_rules() {
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
            let mut item = event();
            let s = item.schedule.as_mut().unwrap();
            s.start = if repeat == Repeat::Yearly {
                date("2024-02-29 09:00")
            } else {
                date("2026-01-31 09:00")
            };
            s.repeat = repeat;
            s.clamp_missing_day = clamp;
            let parsed = ics::parse(&ics::export(&[item.clone()], false).unwrap())
                .unwrap()
                .records
                .remove(0);
            let expected = item.schedule.unwrap();
            let actual = parsed.schedule.unwrap();
            for offset in 0..1500 {
                let day = expected.start.date() + Days::new(offset);
                assert_eq!(
                    actual.on_day(day),
                    expected.on_day(day),
                    "{repeat:?} {clamp} {day}"
                );
            }
        }
    }
    assert!(
        ics::parse(&calendar(
            "DTSTART:20261002T090000\r\nRRULE:FREQ=YEARLY;BYMONTHDAY=2"
        ))
        .is_err()
    );
    assert!(
        ics::parse(&calendar(
            "DTSTART:20261002T090000\r\nRRULE:FREQ=WEEKLY;BYDAY=FR"
        ))
        .is_ok()
    );
}

#[test]
fn ics_rejects_unsupported_or_incomplete_semantics_without_partial_import() {
    for extra in [
        "RRULE:FREQ=DAILY;COUNT=3",
        "RRULE:FREQ=DAILY;UNTIL=20261009T090000",
        "RRULE:FREQ=WEEKLY;BYDAY=MO,WE",
        "RRULE:FREQ=MONTHLY;BYDAY=1MO",
        "EXDATE:20261009T090000",
        "RECURRENCE-ID:20261009T090000",
        "DURATION:PT1H",
        "STATUS:CANCELLED",
        "DTSTART:20261003T090000",
    ] {
        assert!(
            ics::parse(&calendar(&format!("DTSTART:20261002T090000\r\n{extra}"))).is_err(),
            "{extra}"
        );
    }
    let valid = calendar("DTSTART:20261002T090000");
    for text in [
        valid.replace("END:VEVENT\r\n", ""),
        valid.replace("END:VCALENDAR\r\n", ""),
        format!("{valid}{valid}"),
        format!("{valid}trailing"),
        valid.replace("VERSION:2.0", "VERSION:1.0"),
        valid.replace("VERSION:2.0", "VERSION:2.0\r\nMETHOD:REQUEST"),
        valid.replace("BEGIN:VEVENT", "BEGIN:VTIMEZONE"),
        valid.replace("UID:fixture@calendar.test\r\n", ""),
        valid.replace("SUMMARY:日程", "SUMMARY:bad\\q"),
    ] {
        assert!(ics::parse(&text).is_err(), "{text}");
    }
    assert!(ics::parse(&calendar("DTSTART;TZID=Asia/Shanghai:20261002T090000")).is_err());
    assert!(
        ics::parse(
            &calendar("DTSTART:20261002T090000")
                .replace("VERSION:2.0", "VERSION:2.0\r\nX-WR-TIMEZONE:Asia/Shanghai")
        )
        .is_err()
    );
}

#[test]
fn ics_utc_single_event_converts_and_utc_repetition_is_rejected() {
    let input = calendar("DTSTART:20261002T010000Z\r\nDTEND:20261002T020000Z");
    let parsed = ics::parse(&input).unwrap();
    let expected = date("2026-10-02 01:00")
        .and_utc()
        .with_timezone(&Local)
        .naive_local();
    assert_eq!(parsed.records[0].schedule.as_ref().unwrap().start, expected);
    assert!(parsed.notices.iter().any(|n| n.contains("UTC")));
    assert!(ics::parse(&input.replace("END:VEVENT", "RRULE:FREQ=DAILY\r\nEND:VEVENT")).is_err());
    assert!(ics::parse(&calendar("DTSTART:20261002T010030Z")).is_err());
}

#[test]
fn ics_reminder_export_is_opt_in_and_import_never_enables_alarms() {
    let mut item = event();
    assert!(
        !ics::export(&[item.clone()], false)
            .unwrap()
            .contains("VALARM")
    );
    let s = item.schedule.as_mut().unwrap();
    s.start = date("2026-10-02 00:00");
    s.end = Some(date("2026-10-03 00:00"));
    s.all_day = true;
    s.reminder_time = Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap());
    let exported = ics::export(&[item], true).unwrap();
    assert!(exported.contains("TRIGGER:PT31800S"));
    let parsed = ics::parse(&exported).unwrap();
    assert!(!parsed.records[0].schedule.as_ref().unwrap().remind);
    assert!(parsed.notices.iter().any(|s| s.contains("提醒")));
}

#[test]
fn ics_plan_deduplicates_uid_and_preserves_unrelated_notes_and_local_flags() {
    let incoming = ics::parse(&calendar("DTSTART:20261002T090000"))
        .unwrap()
        .records;
    let mut old = incoming[0].clone();
    old.title = "本机修改".into();
    old.pinned = true;
    old.trash = true;
    old.schedule.as_mut().unwrap().done = true;
    let mut note = Item::new(None);
    note.title = "保留备忘".into();
    note.revision = 1;
    let current = vec![old.clone(), note.clone()];
    let keep = ics_ui::plan(&current, &incoming, false).unwrap();
    assert_eq!(keep.skipped, 1);
    assert_eq!(keep.records, current);
    let update = ics_ui::plan(&current, &incoming, true).unwrap();
    assert_eq!(update.updated, 1);
    assert_eq!(update.records[1], note);
    let changed = &update.records[0];
    assert!(changed.pinned && changed.trash && changed.schedule.as_ref().unwrap().done);
    let repeated = ics_ui::plan(&update.records, &incoming, true).unwrap();
    assert_eq!(repeated.unchanged, 1);
    assert_eq!(repeated.updated, 0);
    assert_eq!(repeated.records, update.records);
    let mut collision = note;
    collision.id = incoming[0].id.clone();
    assert!(ics_ui::plan(&[collision], &incoming, true).is_err());
}

#[test]
fn ics_import_requires_preview_confirmation_and_transaction_recheck() {
    let path = fixture();
    store::save(&path, event()).unwrap();
    let saved = store::load(&path).unwrap();
    let mut changed = saved[0].clone();
    changed.title = "来自 ICS 的修改".into();
    let file = path.with_extension("ics");
    ics::write(&file, &ics::export(&[changed], false).unwrap()).unwrap();
    let mut state = State::new(path.clone());
    wait_state(&mut state);
    assert!(state.import_ics().is_err());
    state.read_ics(file.clone()).unwrap();
    wait_state(&mut state);
    assert_eq!(store::load(&path).unwrap(), saved);
    state.ics_review = None;
    assert_eq!(store::load(&path).unwrap(), saved);
    state.read_ics(file.clone()).unwrap();
    wait_state(&mut state);
    let Some(ics_ui::Review::Import(review)) = state.ics_review.as_mut() else {
        panic!()
    };
    review.update = true;
    review.rebuild();
    assert!(state.import_ics().is_err());
    let Some(ics_ui::Review::Import(review)) = state.ics_review.as_mut() else {
        panic!()
    };
    review.confirmed = true;
    let mut newer = saved[0].clone();
    newer.body = "并发编辑".into();
    store::save(&path, newer).unwrap();
    state.import_ics().unwrap();
    wait_state(&mut state);
    assert!(state.error);
    assert_eq!(store::load(&path).unwrap()[0].title, saved[0].title);
    state.read_ics(file).unwrap();
    wait_state(&mut state);
    let Some(ics_ui::Review::Import(review)) = state.ics_review.as_mut() else {
        panic!()
    };
    review.update = true;
    review.rebuild();
    review.confirmed = true;
    state.import_ics().unwrap();
    wait_state(&mut state);
    assert!(!state.error);
    assert_eq!(state.items[0].title, "来自 ICS 的修改");
    assert!(!state.items[0].schedule.as_ref().unwrap().remind);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn ics_files_are_bounded_utf8_and_never_overwrite() {
    let path = fixture();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = path.with_extension("ics");
    let data = calendar("DTSTART:20261002T090000");
    ics::write(&file, &data).unwrap();
    assert!(ics::write(&file, "replacement").is_err());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), data);
    assert_eq!(ics::read(&file).unwrap().records.len(), 1);
    assert!(ics::read(&path.with_extension("txt")).is_err());
    std::fs::write(&file, b"\xff\xfe\0").unwrap();
    assert!(ics::read(&file).is_err());
    assert!(ics::parse(&"a".repeat(16 * 1024 * 1024 + 1)).is_err());
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn ics_export_review_snapshots_saved_scope_and_guards_unsaved_edits() {
    let path = fixture();
    store::save(&path, event()).unwrap();
    let mut extra = event();
    extra.title = "已完成安排".into();
    extra.schedule.as_mut().unwrap().done = true;
    store::save(&path, extra).unwrap();
    let mut state = State::new(path.clone());
    wait_state(&mut state);
    state.edit(state.items[0].clone());
    state.draft.as_mut().unwrap().body = "未保存".into();
    assert!(state.prepare_ics().is_err());
    state.discard();
    state.prepare_ics().unwrap();
    wait_state(&mut state);
    assert!(state.prepare_backup().is_err());
    assert!(state.review_export().is_err());
    let Some(ics_ui::Review::Export(review)) = state.ics_review.as_mut() else {
        panic!()
    };
    review.selected_only = false;
    review.completed = true;
    review.rebuild();
    assert_eq!(review.count, 2);
    let file = path.with_extension("ics");
    state.save_ics(file.clone()).unwrap();
    assert!(state.saving());
    wait_state(&mut state);
    assert!(!state.error);
    let records = ics::read(&file).unwrap().records;
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|i| i.body != "未保存"));
    let uid = records[0].calendar_uid.clone();
    let backup = backup::Document::new(records).unwrap();
    let json = path.with_extension("json");
    backup::write(&json, &backup).unwrap();
    assert_eq!(backup::read(&json).unwrap().records[0].calendar_uid, uid);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn ics_typed_text_parameters_cannot_silently_change_content() {
    let input = calendar("DTSTART:20261002T090000\r\nDESCRIPTION;ENCODING=BASE64:SGVsbG8=");
    assert!(ics::parse(&input).is_err());
    assert!(ics::parse(&input.replace("ENCODING=BASE64", "VALUE=BINARY")).is_err());
    assert!(ics::parse(&input.replace("ENCODING=BASE64", "LANGUAGE=zh-CN")).is_ok());
    let duplicate=calendar("DTSTART:20261002T090000").replace("END:VCALENDAR","BEGIN:VEVENT\r\nUID:fixture@calendar.test\r\nSUMMARY:同 UID\r\nDTSTART:20261003T090000\r\nEND:VEVENT\r\nEND:VCALENDAR");
    assert!(ics::parse(&duplicate).is_err());
}
