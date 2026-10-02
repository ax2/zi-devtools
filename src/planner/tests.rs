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
