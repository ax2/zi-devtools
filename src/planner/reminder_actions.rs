use super::*;

#[derive(Clone, Copy)]
pub(super) enum ReminderAction {
    Acknowledge,
    Snooze(u16),
}
impl State {
    pub(super) fn respond_reminder<T: TimeZone>(
        &mut self,
        id: &str,
        at: NaiveDateTime,
        action: ReminderAction,
        now: DateTime<T>,
    ) -> Result<()> {
        ensure!(
            self.loaded && self.pending.is_none(),
            "正在读写本地记录，请稍后重试"
        );
        ensure!(
            self.purge_review.is_none()
                && self.backup_review.is_none()
                && self.export_review.is_none()
                && self.ics_review.is_none(),
            "请先关闭文件或删除预览"
        );
        ensure!(
            !(self.draft.as_ref().is_some_and(|i| i.id == id) && self.has_unsaved()),
            "此日程正在编辑，请先保存或放弃编辑"
        );
        let mut item = self
            .items
            .iter()
            .find(|i| i.id == id && !i.trash)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("此日程已移除，请重新加载"))?;
        let s = item
            .schedule
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("此记录不是日程"))?;
        ensure!(
            s.due(now.clone()) == Some(at),
            "这次提醒已处理或已更新，请查看当前提醒"
        );
        match action {
            ReminderAction::Acknowledge => {
                s.handled = Some(at);
                s.snooze = None;
            }
            ReminderAction::Snooze(minutes) => {
                ensure!(
                    (1..=1440).contains(&minutes),
                    "稍后提醒需要在 1 分钟到 1 天之间"
                );
                let until = now
                    .timestamp()
                    .checked_add(i64::from(minutes) * 60)
                    .ok_or_else(|| anyhow::anyhow!("稍后提醒时间超出范围"))?;
                s.snooze = Some((at, until));
            }
        }
        self.launch(Some(item));
        Ok(())
    }
}

#[cfg(feature = "ui-preview")]
impl State {
    pub fn preview_snooze(&mut self, editing: bool) {
        self.preview(true, true);
        self.snooze_minutes = 30;
        if editing {
            self.draft
                .as_mut()
                .unwrap()
                .body
                .push_str("\n尚未保存的修改");
        }
    }
    pub fn preview_snooze_smoke(&mut self, phase: u8) -> bool {
        match phase {
            0 => {
                self.preview_snooze(false);
                self.snooze_minutes = 10;
                self.path = std::env::temp_dir()
                    .join(format!("zi-snooze-{}", uuid::Uuid::new_v4()))
                    .join("planner.sqlite3");
                let mut item = self.items[2].clone();
                item.revision = 0;
                let s = item.schedule.as_mut().unwrap();
                s.start = Local::now().naive_local() - Duration::minutes(1);
                s.remind = true;
                s.minutes = 0;
                s.repeat = Repeat::Once;
                store::save(&self.path, item).unwrap();
                self.items = store::load(&self.path).unwrap();
                self.edit(self.items[0].clone());
                self.alarms = vec![(
                    self.items[0].id.clone(),
                    self.items[0].schedule.as_ref().unwrap().start,
                )];
                self.last_tick = Instant::now() - std::time::Duration::from_secs(2);
            }
            1 => {
                assert_eq!(
                    self.snooze_minutes, 30,
                    "actual menu selection did not change delay"
                );
            }
            2 => {
                if self.pending.is_some() {
                    return false;
                }
                assert!(!self.error && !self.has_unsaved());
                let saved = store::load(&self.path).unwrap();
                let s = saved[0].schedule.as_ref().unwrap();
                let (at, until) = s.snooze.expect("actual snooze click persisted");
                assert_eq!(at, s.start);
                let remaining = until - Local::now().timestamp();
                assert!((1770..=1800).contains(&remaining));
                assert!(s.handled.is_none() && s.due(Local::now()).is_none());
                let reload = store::load(&self.path).unwrap();
                assert_eq!(reload, saved);
                assert!(self.alarms.is_empty());
                std::fs::remove_dir_all(self.path.parent().unwrap()).unwrap();
                println!(
                    "PASS snooze UI: actual menu chooses 30 minutes, actual action saves delay, reload suppresses reminder, original event preserved"
                );
            }
            _ => unreachable!(),
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::tests::{event, fixture, now, wait_state};
    #[test]
    fn delays_persist_survive_reload_and_acknowledgment_clears_delay() {
        for minutes in [5, 10, 30, 60, 120, 1440] {
            let path = fixture();
            let mut item = event();
            let s = item.schedule.as_mut().unwrap();
            s.start = now("2026-10-02 09:00").naive_local();
            s.remind = true;
            s.minutes = 0;
            let at = s.start;
            let id = item.id.clone();
            store::save(&path, item).unwrap();
            let mut state = State::new(path.clone());
            wait_state(&mut state);
            let clock = now("2026-10-02 09:01");
            state
                .respond_reminder(&id, at, ReminderAction::Snooze(minutes), clock)
                .unwrap();
            wait_state(&mut state);
            let saved = store::load(&path).unwrap();
            let s = saved[0].schedule.as_ref().unwrap();
            let until = clock.timestamp() + i64::from(minutes) * 60;
            assert_eq!(s.snooze, Some((at, until)));
            assert_eq!(s.due(clock), None);
            let end = clock + Duration::minutes(i64::from(minutes));
            assert_eq!(s.due(end), Some(at));
            let mut restart = State::new(path.clone());
            wait_state(&mut restart);
            assert_eq!(restart.items[0].schedule.as_ref().unwrap().snooze, s.snooze);
            restart
                .respond_reminder(&id, at, ReminderAction::Acknowledge, end)
                .unwrap();
            wait_state(&mut restart);
            let s = store::load(&path).unwrap()[0].schedule.clone().unwrap();
            assert_eq!(s.handled, Some(at));
            assert!(s.snooze.is_none());
            assert!(s.due(end).is_none());
            std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        }
    }
    #[test]
    fn stale_reminders_invalid_delays_and_unsaved_edits_never_write() {
        let path = fixture();
        let mut item = event();
        let s = item.schedule.as_mut().unwrap();
        s.remind = true;
        s.minutes = 0;
        s.repeat = Repeat::Daily;
        s.start = now("2026-10-02 09:00").naive_local();
        let at = s.start;
        let id = item.id.clone();
        store::save(&path, item).unwrap();
        let mut state = State::new(path.clone());
        wait_state(&mut state);
        let clock = now("2026-10-02 09:01");
        for minutes in [0, 1441, u16::MAX] {
            assert!(
                state
                    .respond_reminder(&id, at, ReminderAction::Snooze(minutes), clock)
                    .is_err()
            );
        }
        assert!(
            state
                .respond_reminder(
                    &id,
                    at,
                    ReminderAction::Acknowledge,
                    now("2026-10-03 09:01")
                )
                .is_err()
        );
        state.edit(state.items[0].clone());
        state.draft.as_mut().unwrap().body = "保留草稿".into();
        assert!(
            state
                .respond_reminder(&id, at, ReminderAction::Snooze(30), clock)
                .is_err()
        );
        assert!(state.pending.is_none());
        assert_eq!(state.draft.as_ref().unwrap().body, "保留草稿");
        assert!(
            store::load(&path).unwrap()[0]
                .schedule
                .as_ref()
                .unwrap()
                .snooze
                .is_none()
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn next_repeat_is_not_hidden_and_other_note_draft_survives_reminder_write() {
        let path = fixture();
        let mut item = event();
        let s = item.schedule.as_mut().unwrap();
        s.start = now("2026-10-02 09:00").naive_local();
        s.remind = true;
        s.minutes = 0;
        s.repeat = Repeat::Daily;
        let at = s.start;
        let id = item.id.clone();
        store::save(&path, item).unwrap();
        let mut note = Item::new(None);
        note.title = "其他备忘".into();
        store::save(&path, note).unwrap();
        let mut state = State::new(path.clone());
        wait_state(&mut state);
        state.edit(
            state
                .items
                .iter()
                .find(|i| i.schedule.is_none())
                .unwrap()
                .clone(),
        );
        state.draft.as_mut().unwrap().body = "未保存的其他备忘".into();
        state
            .respond_reminder(
                &id,
                at,
                ReminderAction::Snooze(1440),
                now("2026-10-02 09:01"),
            )
            .unwrap();
        wait_state(&mut state);
        assert!(state.has_unsaved());
        assert_eq!(state.draft.as_ref().unwrap().body, "未保存的其他备忘");
        let saved = store::load(&path).unwrap();
        let s = saved
            .iter()
            .find(|i| i.id == id)
            .unwrap()
            .schedule
            .as_ref()
            .unwrap();
        assert_eq!(
            s.due(now("2026-10-03 09:00")),
            Some(now("2026-10-03 09:00").naive_local())
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
