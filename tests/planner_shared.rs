//! Production State, SQLite and normal elapsed-time polling; no preview bypass.
#![cfg(windows)]
use chrono::{Duration, Local};
use eframe::egui::{self, epaint::Shape};
use rusqlite::{Connection, params};
use serde_json::json;
use std::{path::PathBuf, thread, time::Instant};
use zi_devtools::planner::State;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("zi-planner-public-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("planner.sqlite3");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE records(id TEXT PRIMARY KEY, revision INTEGER NOT NULL, payload TEXT NOT NULL); PRAGMA user_version=1;").unwrap();
        Self(path)
    }
    fn write(&self, id: &str, revision: i64, title: &str, reminder: bool) {
        let schedule = reminder.then(|| {
            json!({
                "start": Local::now().naive_local()-Duration::minutes(1),
                "minutes": 0, "remind": true, "repeat":"Once", "done":false,
                "handled":null, "snooze":null
            })
        });
        let payload = json!({"id":id,"revision":revision,"title":title,"body":"external writer",
            "pinned":false,"trash":false,"updated":Local::now().timestamp(),"schedule":schedule});
        Connection::open(&self.0)
            .unwrap()
            .execute(
                "INSERT OR REPLACE INTO records(id,revision,payload) VALUES(?1,?2,?3)",
                params![id, revision, payload.to_string()],
            )
            .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}
fn text(state: &mut State, ctx: &egui::Context) -> String {
    let output = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| state.ui(ui));
    });
    fn collect(shape: &Shape, text: &mut String) {
        match shape {
            Shape::Text(shape) => {
                text.push_str(shape.galley.text());
                text.push('\n');
            }
            Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, text);
                }
            }
            _ => (),
        }
    }
    let mut result = String::new();
    for shape in output.shapes {
        collect(&shape.shape, &mut result);
    }
    result
}

#[test]
fn normal_polling_refreshes_rendered_rows_and_preserves_draft_after_external_delete() {
    let fixture = Fixture::new();
    let id = uuid::Uuid::new_v4().to_string();
    fixture.write(&id, 1, "Original shared memo", false);
    let mut state = State::new(fixture.0.clone());
    let ctx = egui::Context::default();
    let start = Instant::now();
    loop {
        state.poll(&ctx);
        if text(&mut state, &ctx).contains("Original shared memo") {
            break;
        }
        assert!(start.elapsed().as_secs() < 8);
        thread::sleep(std::time::Duration::from_millis(20));
    }
    state
        .receive_text("Local draft", "Uncommitted content")
        .unwrap();
    fixture.write(&id, 2, "Externally updated memo", false);
    let start = Instant::now();
    loop {
        state.poll(&ctx);
        if text(&mut state, &ctx).contains("Externally updated memo") {
            break;
        }
        assert!(start.elapsed().as_secs() < 8);
        thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(state.transfer_text().unwrap().1, "Uncommitted content");
    assert!(state.has_unsaved());
    Connection::open(&fixture.0)
        .unwrap()
        .execute("DELETE FROM records WHERE id=?1", [&id])
        .unwrap();
    let start = Instant::now();
    loop {
        state.poll(&ctx);
        if !text(&mut state, &ctx).contains("Externally updated memo") {
            break;
        }
        assert!(start.elapsed().as_secs() < 8);
        thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(state.transfer_text().unwrap().1, "Uncommitted content");
    assert!(state.has_unsaved());
}

#[test]
fn empty_windows_discover_new_due_event_once_and_take_over_without_accelerated_ticks() {
    let fixture = Fixture::new();
    let mut first = State::new(fixture.0.clone());
    let ctx = egui::Context::default();
    // Establish the first owner before starting a follower.
    let start = Instant::now();
    while !text(&mut first, &ctx).contains("此窗口负责到期提醒") {
        first.poll(&ctx);
        assert!(start.elapsed().as_secs() < 8);
        thread::sleep(std::time::Duration::from_millis(20));
    }
    let mut second = State::new(fixture.0.clone());
    let id = uuid::Uuid::new_v4().to_string();
    fixture.write(&id, 1, "New due reminder", true);
    let start = Instant::now();
    let mut delivered = false;
    while start.elapsed().as_secs() < 5 {
        if first.poll(&ctx) {
            assert!(!delivered, "same owner delivered twice");
            delivered = true;
        }
        assert!(!second.poll(&ctx), "follower delivered automatic reminder");
        thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(delivered, "empty owner failed to discover the new reminder");
    drop(first);
    let start = Instant::now();
    while !second.poll(&ctx) {
        assert!(
            start.elapsed().as_secs() < 8,
            "follower failed to take over"
        );
        thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(text(&mut second, &ctx).contains("此窗口负责到期提醒"));
}
