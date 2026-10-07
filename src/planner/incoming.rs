use super::*;
use anyhow::Context;

impl State {
    #[cfg(feature = "ui-preview")]
    pub fn preview_incoming_event_assert(&self, text: &str) {
        let draft = self.draft.as_ref().unwrap();
        let schedule = draft.schedule.as_ref().unwrap();
        assert!(
            self.calendar && self.has_unsaved() && self.pending.is_none() && !self.path.exists()
        );
        assert_eq!(draft.body, text);
        assert_eq!(schedule.start, self.selected.and_hms_opt(9, 0, 0).unwrap());
        assert!(!schedule.remind && schedule.repeat == Repeat::Once);
        assert_eq!(self.items.len(), 3);
        assert!(self.items[2].schedule.as_ref().unwrap().remind);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_date_received(&self, day: NaiveDate, text: &str) {
        let draft = self.draft.as_ref().expect("received draft");
        let schedule = draft.schedule.as_ref().unwrap();
        assert!(
            self.calendar && self.has_unsaved() && self.pending.is_none() && !self.path.exists()
        );
        assert_eq!(schedule.start, day.and_time(NaiveTime::MIN));
        assert_eq!(
            schedule.end,
            Some(day.succ_opt().unwrap().and_time(NaiveTime::MIN))
        );
        assert!(schedule.all_day && !schedule.remind && schedule.repeat == Repeat::Once);
        assert_eq!(self.selected, day);
        assert_eq!(draft.body, text);
        assert_eq!(self.items.len(), 3);
    }
    pub fn incoming_event_date(&self) -> NaiveDate {
        self.selected
    }
    /// Receive a snapshot in memory; persistence still requires explicit Save.
    pub fn receive_text(&mut self, source: &str, text: &str) -> Result<()> {
        self.receive_snapshot(source, text, false)
    }
    pub fn receive_event_text(&mut self, source: &str, text: &str) -> Result<()> {
        self.receive_snapshot(source, text, true)
    }
    pub fn receive_event_date(&mut self, source: &str, text: &str, date: NaiveDate) -> Result<()> {
        self.receive_snapshot_at(source, text, Some(date), true)
    }
    fn receive_snapshot(&mut self, source: &str, text: &str, event: bool) -> Result<()> {
        self.receive_snapshot_at(source, text, event.then_some(self.selected), false)
    }
    fn receive_snapshot_at(
        &mut self,
        source: &str,
        text: &str,
        date: Option<NaiveDate>,
        all_day: bool,
    ) -> Result<()> {
        let event = date.is_some();
        ensure!(
            self.loaded && self.pending.is_none(),
            "备忘 / 日程正在加载或保存，请稍后重试"
        );
        ensure!(
            self.purge_review.is_none()
                && self.export_review.is_none()
                && self.backup_review.is_none()
                && self.ics_review.is_none()
                && !self.file_operation,
            "请先完成或取消备忘 / 日程的文件或删除预览，再接收结果"
        );
        ensure!(
            !self.has_unsaved(),
            "请先保存或放弃备忘 / 日程的当前编辑，再接收结果"
        );
        ensure!(
            !text.is_empty() && text.len() <= MAX_BODY,
            "正文需要 1 字节至 128 KiB，请先缩小结果范围"
        );
        let mut item = Item::new(date);
        item.title = format!("来自 {source}")
            .chars()
            .filter(|c| !c.is_control())
            .take(120)
            .collect();
        item.body = text.to_owned();
        if let Some(schedule) = &mut item.schedule {
            schedule.remind = false;
            if all_day {
                let day = date.context("日期快照缺少日期")?;
                schedule.start = day.and_time(NaiveTime::MIN);
                schedule.end = Some(
                    day.succ_opt()
                        .context("全天结束日期超出范围")?
                        .and_time(NaiveTime::MIN),
                );
                schedule.all_day = true;
                schedule.reminder_time = Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap());
                schedule.minutes = 0;
            }
        }
        item.validate()?;
        self.edit(item);
        if all_day {
            self.select_date(date.unwrap());
        }
        self.calendar = event;
        self.trash = false;
        self.query.clear();
        self.message = if all_day {
            "计算日期已填入单次全天日程草稿，提醒关闭；检查后保存到本机。"
        } else if event {
            "结果已填入新日程草稿，提醒默认关闭；调整日期和时间后保存到本机。"
        } else {
            "结果已填入新备忘草稿；点击保存后才会保留到本机。"
        }
        .into();
        self.error = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::tests::{date, fixture, wait_state};

    #[test]
    fn typed_date_creates_all_day_without_write_and_rejects_invalid_or_dirty_targets() {
        let path = fixture();
        let mut state = State::new(path.clone());
        wait_state(&mut state);
        let before = state.selected;
        assert!(
            state
                .receive_event_date("calculator", "snapshot", date("2500-01-01 00:00").date())
                .is_err()
        );
        assert_eq!(state.selected, before);
        assert!(state.draft.is_none() && !path.exists());
        state.file_operation = true;
        assert!(
            state
                .receive_event_date("calculator", "snapshot", date("2028-02-29 00:00").date())
                .is_err()
        );
        state.file_operation = false;
        let day = date("2028-02-29 00:00").date();
        state
            .receive_event_date("calculator0.10", "full calculation", day)
            .unwrap();
        let draft = state.draft.clone().unwrap();
        let schedule = draft.schedule.as_ref().unwrap();
        assert_eq!(state.selected, day);
        assert_eq!(state.month, day.with_day(1).unwrap());
        assert_eq!(schedule.start, date("2028-02-29 00:00"));
        assert_eq!(schedule.end, Some(date("2028-03-01 00:00")));
        assert!(schedule.all_day && !schedule.remind && schedule.repeat == Repeat::Once);
        assert!(draft.calendar_uid.is_none() && state.has_unsaved() && !path.exists());
        assert!(
            state
                .receive_event_date("next", "do not replace", date("2029-01-01 00:00").date())
                .is_err()
        );
        assert_eq!(state.draft.as_ref(), Some(&draft));
        assert_eq!(state.selected, day);
        state.save_draft();
        wait_state(&mut state);
        let records = store::load(&path).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].schedule, draft.schedule);
        assert_eq!(records[0].body, draft.body);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn incoming_event_is_unsaved_with_visible_date_and_reminder_off_until_explicit_save() {
        let path = fixture();
        let mut state = State::new(path.clone());
        assert!(state.receive_event_text("Java", "loading").is_err());
        wait_state(&mut state);
        state.select_date(date("2027-01-12 00:00").date());
        assert_eq!(state.incoming_event_date(), date("2027-01-12 00:00").date());
        state
            .receive_event_text("Java\n诊断", "完整结果\n{\"ok\":true}")
            .unwrap();
        let draft = state.draft.clone().unwrap();
        let schedule = draft.schedule.as_ref().unwrap();
        assert_eq!(schedule.start, date("2027-01-12 09:00"));
        assert!(!schedule.remind && schedule.repeat == Repeat::Once && !schedule.done);
        assert!(schedule.handled.is_none() && schedule.snooze.is_none());
        assert!(state.calendar && state.has_unsaved() && !path.exists());
        assert_eq!(draft.title, "来自 Java诊断");
        assert!(state.receive_text("next", "must not overwrite").is_err());
        assert_eq!(state.draft.as_ref(), Some(&draft));
        state.save_draft();
        wait_state(&mut state);
        let records = store::load(&path).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].body, draft.body);
        assert_eq!(records[0].schedule, draft.schedule);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn receiving_rejects_empty_oversize_and_active_review_without_mutating_record() {
        let mut state = State::new(fixture());
        wait_state(&mut state);
        assert!(state.receive_event_text("tool", "").is_err());
        assert!(
            state
                .receive_event_text("tool", &"x".repeat(MAX_BODY + 1))
                .is_err()
        );
        state.file_operation = true;
        for event in [false, true] {
            assert!(state.receive_snapshot("tool", "blocked", event).is_err());
            assert!(state.draft.is_none());
        }
        state.file_operation = false;
        state.purge_review = Some(Vec::new());
        assert!(state.receive_event_text("tool", "blocked").is_err());
        assert!(state.draft.is_none() && !state.path.exists());
    }
}
