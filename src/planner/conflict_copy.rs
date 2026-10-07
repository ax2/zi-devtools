use super::*;

#[cfg(feature = "ui-preview")]
impl State {
    pub fn preview_conflict_copy(&mut self, phase: u8) -> bool {
        match phase {
            0 => {
                let path = std::env::temp_dir()
                    .join(format!("zi-conflict-native-{}", uuid::Uuid::new_v4()))
                    .join("planner.sqlite3");
                let mut item = Item::new(None);
                item.title = "多窗口冲突示例".into();
                item.body = "原来已保存的正文".into();
                store::save(&path, item).unwrap();
                *self = State::new(path);
            }
            1 => {
                if !self.loaded || self.pending.is_some() {
                    return false;
                }
                self.edit(self.items[0].clone());
                self.focus_editor = false;
                let mut other = self.items[0].clone();
                other.title = "其他窗口保存的记录".into();
                other.body = "另一窗口的新内容".into();
                store::save(&self.path, other).unwrap();
                self.draft.as_mut().unwrap().title = "保留下来的修改".into();
                self.draft.as_mut().unwrap().body = "我的未保存正文，另存后应完整保留。".into();
            }
            2 => {
                return self.shared_conflict();
            }
            3 => {
                if self.pending.is_some() {
                    return false;
                }
                assert!(
                    !self.error && !self.has_unsaved(),
                    "actual conflict-copy click did not save: {} / unsaved={}",
                    self.message,
                    self.has_unsaved()
                );
                let rows = store::load(&self.path).unwrap();
                assert_eq!(rows.len(), 2);
                assert!(rows.iter().any(|i| i.title == "其他窗口保存的记录"
                    && i.body == "另一窗口的新内容"
                    && i.revision == 2));
                assert_eq!(self.draft.as_ref().unwrap().title, "保留下来的修改");
                assert_eq!(
                    self.draft.as_ref().unwrap().body,
                    "我的未保存正文，另存后应完整保留。"
                );
                assert_eq!(self.draft.as_ref().unwrap().revision, 1);
                println!(
                    "PASS native conflict copy: normal shared polling detects external change, actual egui pointer click saves local modifications as a new record, remote record remains intact, saved copy selected with no unsaved state"
                );
            }
            4 => {
                let root = self.path.parent().unwrap();
                assert!(
                    root.starts_with(std::env::temp_dir())
                        && root
                            .file_name()
                            .unwrap()
                            .to_string_lossy()
                            .starts_with("zi-conflict-native-")
                );
                self.shared.synthetic = true;
                std::fs::remove_dir_all(root).unwrap();
            }
            _ => unreachable!(),
        }
        true
    }
}

impl State {
    pub(super) fn save_conflict_copy(&mut self) -> Result<()> {
        ensure!(
            self.loaded && self.pending.is_none(),
            "正在读写本地记录，请稍后重试"
        );
        ensure!(
            self.purge_review.is_none()
                && self.export_review.is_none()
                && self.backup_review.is_none()
                && self.ics_review.is_none()
                && self.conflict_review.is_none(),
            "请先关闭文件或删除预览"
        );
        ensure!(
            self.shared_conflict()
                && self
                    .draft
                    .as_ref()
                    .is_some_and(|i| i.revision > 0 && !i.trash),
            "仅可将冲突的未保存修改另存为新记录"
        );
        let copy = self.prepared_draft(true)?;
        self.saving = Some(copy.id.clone());
        let path = self.path.clone();
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        std::thread::spawn(move || {
            let result = store::save_returning(&path, copy)
                .map(|saved| {
                    Reply::SavedCopy(
                        Box::new(saved),
                        store::load(&path).map_err(|e| e.to_string()),
                    )
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        Ok(())
    }
    pub(super) fn accept_saved_copy(
        &mut self,
        saved: Item,
        rows: std::result::Result<Vec<Item>, String>,
    ) {
        self.saving = None;
        let is_event = saved.schedule.is_some();
        match rows {
            Ok(rows) => {
                let current = rows.iter().find(|i| i.id == saved.id).cloned();
                self.replace_items(rows);
                if let Some(current) = current {
                    self.edit(current);
                    self.error = false;
                    self.message = if is_event {
                        "已将修改另存为新日程，原记录未改变；新日程提醒已关闭。"
                    } else {
                        "已将修改另存为新备忘录，原记录未改变。"
                    }
                    .into();
                } else {
                    self.message = "新记录已写入，但随后已被其他操作删除；当前修改已保留。".into();
                    self.error = true;
                }
            }
            Err(error) => {
                // The transaction succeeded. Never report an overall save failure or
                // leave the old draft selected and invite duplicate copy retries.
                self.edit(saved);
                self.error = false;
                self.message = "新记录已保存，原记录未改变。".into();
                self.shared.notice = format!("列表刷新失败，将自动重试：{error}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::tests::{date, event, fixture, wait_state};
    fn conflict() -> (State, PathBuf, Item) {
        let path = fixture();
        let mut source = event();
        source.calendar_uid = Some("synthetic-original-uid".into());
        source.pinned = true;
        let schedule = source.schedule.as_mut().unwrap();
        schedule.repeat = Repeat::Weekly;
        schedule.repeat_until = Some(date("2026-12-01 00:00").date());
        schedule.end = Some(schedule.start + Duration::hours(1));
        schedule.done = true;
        schedule.handled = Some(schedule.start);
        schedule.snooze = Some((schedule.start, Local::now().timestamp() + 3600));
        store::save(&path, source).unwrap();
        let mut state = State::new(path.clone());
        wait_state(&mut state);
        state.edit(state.items[0].clone());
        state.draft.as_mut().unwrap().title = "Local title".into();
        state.draft.as_mut().unwrap().body = "Local content".into();
        state.date_text = "2026-10-09".into();
        state.time_text = "10:15".into();
        state.end_date_text = "2026-10-09".into();
        state.end_time_text = "11:30".into();
        state.repeat_until_text = "2026-12-31".into();
        let mut remote = state.items[0].clone();
        remote.title = "Remote title".into();
        remote.body = "Remote content".into();
        store::save(&path, remote).unwrap();
        wait_state(&mut state);
        assert!(state.shared_conflict());
        let remote = store::load(&path).unwrap().remove(0);
        (state, path, remote)
    }
    #[test]
    fn queued_conflict_copy_saves_final_input_and_preserves_remote_and_calendar_rules() {
        let (mut state, path, remote) = conflict();
        let id = state.draft.as_ref().unwrap().id.clone();
        state.editor_action = Some((id, actions::Action::SaveConflictCopy));
        state.draft.as_mut().unwrap().body.push_str(" / same frame");
        state.finish_editor_actions(false);
        assert!(state.saving());
        assert!(
            state.save_conflict_copy().is_err(),
            "duplicate submit while saving must be refused"
        );
        wait_state(&mut state);
        let rows = store::load(&path).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows.iter().find(|i| i.id == remote.id).unwrap(), &remote);
        let copy = rows.iter().find(|i| i.id != remote.id).unwrap();
        assert_eq!(copy.title, "Local title");
        assert_eq!(copy.body, "Local content / same frame");
        assert_eq!(copy.revision, 1);
        assert!(copy.calendar_uid.is_none() && !copy.pinned && !copy.trash);
        let schedule = copy.schedule.as_ref().unwrap();
        assert_eq!(schedule.start, date("2026-10-09 10:15"));
        assert_eq!(schedule.end, Some(date("2026-10-09 11:30")));
        assert_eq!(schedule.repeat, Repeat::Weekly);
        assert_eq!(schedule.repeat_until, Some(date("2026-12-31 00:00").date()));
        assert!(!schedule.remind && !schedule.done);
        assert!(schedule.handled.is_none() && schedule.snooze.is_none());
        assert_eq!(state.draft.as_ref().unwrap(), copy);
        assert!(!state.has_unsaved() && !state.shared_conflict());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn validation_storage_failure_and_modal_guard_preserve_original_input() {
        let (mut state, path, remote) = conflict();
        let draft = state.draft.clone();
        state.date_text = "bad date".into();
        assert!(state.save_conflict_copy().is_err());
        assert!(state.pending.is_none());
        assert_eq!(state.draft, draft);
        assert_eq!(state.date_text, "bad date");
        assert_eq!(store::load(&path).unwrap(), vec![remote]);
        state.date_text = "2026-10-09".into();
        state.purge_review = Some(Vec::new());
        assert!(state.save_conflict_copy().is_err());
        state.purge_review = None;
        std::fs::write(&path, b"broken synthetic database").unwrap();
        state.save_conflict_copy().unwrap();
        wait_state(&mut state);
        assert!(state.error);
        assert_eq!(state.draft, draft);
        assert_eq!(state.date_text, "2026-10-09");
        assert_eq!(state.end_time_text, "11:30");
        assert_eq!(state.repeat_until_text, "2026-12-31");
        assert!(state.has_unsaved());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn successful_write_receipt_survives_list_read_failure_without_duplicate_retry() {
        let (mut state, path, remote) = conflict();
        let saved = store::save_returning(&path, state.prepared_draft(true).unwrap()).unwrap();
        assert_eq!(
            store::load(&path)
                .unwrap()
                .iter()
                .find(|i| i.id == saved.id),
            Some(&saved)
        );
        state.accept_saved_copy(saved.clone(), Err("synthetic refresh failure".into()));
        assert!(state.message.starts_with("新记录已保存"));
        assert!(state.shared.notice.starts_with("列表刷新失败"));
        assert_eq!(state.draft.as_ref(), Some(&saved));
        assert!(!state.has_unsaved());
        state.draft.as_mut().unwrap().body.push_str(" further edit");
        assert!(!state.shared_conflict());
        assert!(state.save_conflict_copy().is_err());
        assert_eq!(store::load(&path).unwrap().len(), 2);
        assert_eq!(
            store::load(&path)
                .unwrap()
                .iter()
                .find(|i| i.id == remote.id),
            Some(&remote)
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn external_delete_conflict_can_be_copied_and_later_deleted_copy_keeps_old_draft() {
        let (mut state, path, remote) = conflict();
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute("DELETE FROM records WHERE id=?1", [&remote.id])
            .unwrap();
        wait_state(&mut state);
        assert!(state.shared_conflict());
        let old = state.draft.clone();
        let saved = store::save_returning(&path, state.prepared_draft(true).unwrap()).unwrap();
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute("DELETE FROM records WHERE id=?1", [&saved.id])
            .unwrap();
        state.accept_saved_copy(saved, Ok(store::load(&path).unwrap()));
        assert_eq!(state.draft, old);
        assert!(state.error && state.has_unsaved());
        assert!(state.message.contains("随后已被其他操作删除"));
        state.shared.expedite();
        wait_state(&mut state);
        state.save_conflict_copy().unwrap();
        wait_state(&mut state);
        assert_eq!(store::load(&path).unwrap().len(), 1);
        assert!(!state.has_unsaved());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
