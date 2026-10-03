use super::*;

impl State {
    pub(super) fn convert_draft(&mut self) -> Result<()> {
        let source = self.saved_copy_source()?;
        let to_event = source.schedule.is_none();
        let mut copy = Item::new(to_event.then_some(self.selected));
        copy.title = source.title.clone();
        copy.body = source.body.clone();
        if let Some(schedule) = &mut copy.schedule {
            schedule.remind = false;
        }
        copy.validate()?;
        self.calendar = to_event;
        self.edit(copy);
        self.message = if to_event {
            "已复制为日程草稿，原备忘保留；提醒默认关闭，设置日期和时间后保存。"
        } else {
            "已复制为备忘草稿，原日程及其提醒保留；仅复制标题与正文，保存后保留。"
        }
        .into();
        self.error = false;
        Ok(())
    }
}

#[cfg(feature = "ui-preview")]
impl State {
    pub fn preview_convert(&mut self, calendar: bool) {
        self.preview(calendar, false);
        self.convert_draft().unwrap();
    }
    pub fn preview_convert_smoke(&mut self, phase: u8, calendar: bool) -> bool {
        match phase {
            0 => {
                self.preview(calendar, false);
                self.path = std::env::temp_dir()
                    .join(format!("zi-convert-{}", uuid::Uuid::new_v4()))
                    .join("planner.sqlite3");
                let mut source = self.draft.clone().unwrap();
                source.revision = 0;
                if calendar {
                    source.calendar_uid = Some("external-fixture".into());
                }
                store::save(&self.path, source).unwrap();
                self.replace_items(store::load(&self.path).unwrap());
                let source = self.items[0].clone();
                self.preview_duplicate_source = Some(source.clone());
                self.edit(source);
                self.focus_editor = false;
            }
            1 => {
                let source = self.preview_duplicate_source.as_ref().unwrap();
                let copy = self.draft.as_ref().unwrap();
                assert_ne!(source.id, copy.id);
                assert_eq!(source.title, copy.title);
                assert_eq!(source.body, copy.body);
                assert_eq!(copy.schedule.is_some(), !calendar);
                assert_eq!(self.calendar, !calendar);
                assert!(copy.calendar_uid.is_none() && copy.revision == 0 && self.has_unsaved());
                if let Some(s) = &copy.schedule {
                    assert!(!s.remind);
                }
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
                assert!(rows.contains(self.draft.as_ref().unwrap()));
                std::fs::remove_dir_all(self.path.parent().unwrap()).unwrap();
                println!(
                    "PASS cross-type copy: actual fixed-footer click, target route, unsaved draft, Ctrl S persists independent record, source unchanged; calendar_source={calendar}"
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
    fn both_directions_create_independent_drafts_and_preserve_original_on_save() {
        for calendar in [false, true] {
            let path = fixture();
            let mut state = State::new(path.clone());
            wait_state(&mut state);
            let mut source = event();
            source.body = "会议复盘\n完整正文".into();
            source.pinned = true;
            if !calendar {
                source.schedule = None;
            } else {
                source.calendar_uid = Some("external-uid".into());
            }
            state.edit(source);
            state.save_draft();
            wait_state(&mut state);
            let original = store::load(&path).unwrap()[0].clone();
            state.convert_draft().unwrap();
            let copy = state.draft.as_ref().unwrap();
            assert_ne!(copy.id, original.id);
            assert_eq!((&copy.title, &copy.body), (&original.title, &original.body));
            assert_eq!(copy.schedule.is_some(), !calendar);
            assert_eq!(state.calendar, !calendar);
            assert!(!copy.pinned && !copy.trash && copy.calendar_uid.is_none());
            if let Some(s) = &copy.schedule {
                assert!(!s.remind && !s.done && s.handled.is_none() && s.snooze.is_none());
                assert_eq!(s.start.date(), state.selected);
                assert_eq!(s.repeat, Repeat::Once);
            }
            assert_eq!(copy.revision, 0);
            assert!(state.has_unsaved());
            assert_eq!(store::load(&path).unwrap(), vec![original.clone()]);
            state.save_draft();
            wait_state(&mut state);
            let rows = store::load(&path).unwrap();
            assert_eq!(rows.len(), 2);
            assert!(rows.contains(&original));
            std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        }
    }

    #[test]
    fn dirty_or_trashed_source_cannot_replace_draft() {
        let path = fixture();
        let mut state = State::new(path.clone());
        wait_state(&mut state);
        state.edit(event());
        assert!(state.convert_draft().is_err());
        state.save_draft();
        wait_state(&mut state);
        state.draft.as_mut().unwrap().body = "尚未保存".into();
        let before = state.draft.clone();
        assert!(state.convert_draft().is_err());
        assert_eq!(state.draft, before);
        state.discard();
        state.draft.as_mut().unwrap().trash = true;
        state.original = state.draft.clone();
        assert!(state.convert_draft().is_err());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
