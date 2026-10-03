use super::*;
fn copy_as_draft(source: &Item) -> Item {
    let mut copy = source.clone();
    copy.id = uuid::Uuid::new_v4().to_string();
    copy.revision = 0;
    copy.updated = 0;
    copy.pinned = false;
    copy.trash = false;
    copy.calendar_uid = None;
    let suffix = " · 副本";
    copy.title = source
        .title
        .chars()
        .take(120 - suffix.chars().count())
        .collect::<String>()
        + suffix;
    if let Some(s) = &mut copy.schedule {
        s.remind = false;
        s.done = false;
        s.handled = None;
        s.snooze = None;
    }
    copy
}
impl State {
    pub(super) fn saved_copy_source(&self) -> Result<&Item> {
        ensure!(
            self.loaded && self.pending.is_none(),
            "正在读写本地记录，请稍后重试"
        );
        ensure!(
            self.purge_review.is_none()
                && self.export_review.is_none()
                && self.backup_review.is_none()
                && self.ics_review.is_none(),
            "请先关闭文件或删除预览"
        );
        ensure!(!self.has_unsaved(), "请先保存或放弃当前修改，再创建副本");
        let source = self
            .draft
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("请先打开一条已保存记录"))?;
        ensure!(
            source.revision > 0 && !source.trash,
            "请先保存或从回收站恢复这条记录"
        );
        Ok(source)
    }
    pub(super) fn duplicate_draft(&mut self) -> Result<()> {
        let copy = copy_as_draft(self.saved_copy_source()?);
        copy.validate()?;
        let event = copy.schedule.is_some();
        self.edit(copy);
        self.message = if event {
            "已创建日程副本草稿，提醒已关闭；修改后保存才会保留。"
        } else {
            "已创建备忘副本草稿；修改后保存才会保留。"
        }
        .into();
        self.error = false;
        Ok(())
    }
}
#[cfg(feature = "ui-preview")]
impl State {
    pub fn preview_duplicate(&mut self, calendar: bool) {
        self.preview(calendar, false);
        self.duplicate_draft().unwrap();
    }
    pub fn preview_duplicate_smoke(&mut self, phase: u8) -> bool {
        match phase {
            0 => {
                self.preview(true, false);
                self.path = std::env::temp_dir()
                    .join(format!("zi-copy-{}", uuid::Uuid::new_v4()))
                    .join("planner.sqlite3");
                let mut item = self.draft.clone().unwrap();
                item.revision = 0;
                item.calendar_uid = Some("duplicate-fixture-uid".into());
                let s = item.schedule.as_mut().unwrap();
                s.handled = Some(s.start);
                s.snooze = Some((s.start, Local::now().timestamp() + 600));
                store::save(&self.path, item).unwrap();
                self.replace_items(store::load(&self.path).unwrap());
                let source = self.items[0].clone();
                self.preview_duplicate_source = Some(source.clone());
                self.edit(source);
                self.focus_editor = false;
            }
            1 => {
                let source = self.preview_duplicate_source.as_ref().unwrap();
                let copy = self.draft.as_ref().unwrap();
                assert_ne!(copy.id, source.id);
                assert_eq!(copy.revision, 0);
                assert!(self.has_unsaved() && self.pending.is_none());
                assert_eq!(copy.body, source.body);
                assert!(copy.calendar_uid.is_none());
                let s = copy.schedule.as_ref().unwrap();
                assert!(!s.remind && s.handled.is_none() && s.snooze.is_none());
                assert_eq!(store::load(&self.path).unwrap(), vec![source.clone()]);
            }
            2 => {
                if self.pending.is_some() {
                    return false;
                }
                assert!(!self.error && !self.has_unsaved());
                let rows = store::load(&self.path).unwrap();
                assert_eq!(rows.len(), 2);
                assert!(rows.contains(self.preview_duplicate_source.as_ref().unwrap()));
                let copy = self.draft.as_ref().unwrap();
                assert!(rows.contains(copy));
                assert!(!copy.schedule.as_ref().unwrap().remind);
                std::fs::remove_dir_all(self.path.parent().unwrap()).unwrap();
                println!(
                    "PASS duplicate UI: actual copy click creates unsaved fresh identity, UID/reminder state cleared; Ctrl S persists second record with original untouched"
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
    use crate::planner::tests::{event, fixture, wait_state};
    #[test]
    fn calendar_copy_preserves_rules_and_content_but_clears_identity_and_reminder_state() {
        let mut item = event();
        item.title = "题".repeat(120);
        item.body = "完整正文\n第二行".into();
        item.revision = 7;
        item.pinned = true;
        item.trash = true;
        item.updated = 123;
        item.calendar_uid = Some("external-calendar-uid".into());
        let s = item.schedule.as_mut().unwrap();
        s.repeat = Repeat::Monthly;
        s.repeat_until = Some(s.start.date() + Duration::days(60));
        s.end = Some(s.start + Duration::hours(2));
        s.clamp_missing_day = true;
        s.handled = Some(s.start);
        s.snooze = Some((s.start, 123));
        s.done = true;
        let copy = copy_as_draft(&item);
        copy.validate().unwrap();
        assert_ne!(copy.id, item.id);
        assert_eq!(copy.title.chars().count(), 120);
        assert!(copy.title.ends_with("副本"));
        assert_eq!(copy.body, item.body);
        assert_eq!(copy.revision, 0);
        assert_eq!(copy.updated, 0);
        assert!(!copy.pinned && !copy.trash && copy.calendar_uid.is_none());
        let a = copy.schedule.unwrap();
        let b = item.schedule.unwrap();
        assert_eq!(a.start, b.start);
        assert_eq!(a.end, b.end);
        assert_eq!(a.repeat, b.repeat);
        assert_eq!(a.repeat_until, b.repeat_until);
        assert_eq!(a.clamp_missing_day, b.clamp_missing_day);
        assert!(!a.remind && !a.done && a.handled.is_none() && a.snooze.is_none());
    }
    #[test]
    fn duplicate_is_unsaved_and_original_survives_discard_or_save() {
        let path = fixture();
        let mut item = Item::new(None);
        item.title = "原始备忘".into();
        item.body = "正文".into();
        store::save(&path, item).unwrap();
        let original = store::load(&path).unwrap()[0].clone();
        let mut state = State::new(path.clone());
        wait_state(&mut state);
        state.edit(original.clone());
        state.duplicate_draft().unwrap();
        assert!(state.has_unsaved());
        assert_eq!(store::load(&path).unwrap(), vec![original.clone()]);
        state.discard();
        assert!(state.draft.is_none());
        state.edit(original.clone());
        state.duplicate_draft().unwrap();
        state.save_draft();
        wait_state(&mut state);
        let rows = store::load(&path).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.contains(&original));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn unsaved_edits_and_trash_never_get_replaced_by_copy() {
        let path = fixture();
        let mut state = State::new(path);
        wait_state(&mut state);
        let mut item = event();
        item.revision = 1;
        state.edit(item.clone());
        state.draft.as_mut().unwrap().body = "未保存".into();
        assert!(state.duplicate_draft().is_err());
        assert_eq!(state.draft.as_ref().unwrap().body, "未保存");
        state.discard();
        item.trash = true;
        state.edit(item.clone());
        assert!(state.duplicate_draft().is_err());
        assert_eq!(state.draft, Some(item));
        assert!(state.pending.is_none());
    }
}
