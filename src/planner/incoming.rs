use super::*;

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
    fn receive_snapshot(&mut self, source: &str, text: &str, event: bool) -> Result<()> {
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
        let mut item = Item::new(event.then_some(self.selected));
        item.title = format!("来自 {source}")
            .chars()
            .filter(|c| !c.is_control())
            .take(120)
            .collect();
        item.body = text.to_owned();
        if let Some(schedule) = &mut item.schedule {
            schedule.remind = false;
        }
        item.validate()?;
        self.edit(item);
        self.calendar = event;
        self.trash = false;
        self.query.clear();
        self.message = if event {
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
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
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
