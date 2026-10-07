use super::*;

impl State {
    pub fn preview_conflict_review(&mut self, phase: u8, calendar: bool, deleted: bool) -> bool {
        match phase {
            0 => {
                let path = std::env::temp_dir()
                    .join(format!("zi-conflict-native-{}", uuid::Uuid::new_v4()))
                    .join("planner.sqlite3");
                let mut item =
                    Item::new(calendar.then(|| NaiveDate::from_ymd_opt(2030, 10, 7).unwrap()));
                item.title = "版本对比示例".into();
                item.body = "最初保存的正文".into();
                if let Some(s) = &mut item.schedule {
                    s.repeat = Repeat::Weekly;
                    s.remind = false;
                    s.end = Some(s.start + Duration::hours(1));
                    item.calendar_uid = Some("synthetic-comparison-calendar".into());
                }
                store::save(&path, item).unwrap();
                *self = State::new(path);
                self.calendar = calendar;
            }
            1 => {
                if !self.loaded || self.pending.is_some() {
                    return false;
                }
                self.edit(self.items[0].clone());
                self.focus_editor = false;
                let mut remote = self.items[0].clone();
                remote.title = "另一窗口已保存的内容".into();
                remote.body = "另一窗口整理的正文，当前保存版本为 2。".into();
                if deleted {
                    let conn = rusqlite::Connection::open(&self.path).unwrap();
                    conn.execute("DELETE FROM records WHERE id=?1", [&remote.id])
                        .unwrap();
                } else {
                    store::save(&self.path, remote).unwrap();
                }
                self.draft.as_mut().unwrap().title = "我正在整理的内容".into();
                self.draft.as_mut().unwrap().body = "我的未保存正文，需要对比后保留。".into();
                if calendar {
                    self.time_text = "无效时间".into();
                }
            }
            2 => return self.shared_conflict(),
            3 => {
                assert!(
                    self.conflict_review_open(),
                    "actual comparison button failed to open"
                );
                assert_eq!(
                    self.draft.as_ref().unwrap().body,
                    "我的未保存正文，需要对比后保留。"
                );
                assert_eq!(self.draft.as_ref().unwrap().revision, 1);
                let rows = store::load(&self.path).unwrap();
                assert_eq!(rows.len(), usize::from(!deleted));
                if !deleted {
                    assert_eq!(rows[0].body, "另一窗口整理的正文，当前保存版本为 2。");
                }
                if calendar {
                    assert_eq!(self.time_text, "无效时间");
                }
                println!(
                    "PASS native comparison opens read-only with raw draft preserved; calendar={calendar}"
                );
            }
            4 => {
                assert!(
                    !self.conflict_review_open(),
                    "actual return button failed to close comparison"
                );
                assert!(self.has_unsaved());
                assert_eq!(
                    store::load(&self.path).unwrap().len(),
                    usize::from(!deleted)
                );
                if calendar {
                    assert_eq!(self.time_text, "无效时间");
                    self.time_text = "09:30".into();
                }
                println!(
                    "PASS native comparison closes without writing or discarding; calendar={calendar}"
                );
            }
            5 => {
                if self.pending.is_some() {
                    return false;
                }
                assert!(
                    !self.error && !self.has_unsaved(),
                    "copy after comparison failed: {}",
                    self.message
                );
                let rows = store::load(&self.path).unwrap();
                assert_eq!(rows.len(), if deleted { 1 } else { 2 });
                if !deleted {
                    assert!(
                        rows.iter()
                            .any(|i| i.revision == 2 && i.title == "另一窗口已保存的内容")
                    );
                }
                let copy = self.draft.as_ref().unwrap();
                assert_eq!(copy.body, "我的未保存正文，需要对比后保留。");
                assert_eq!(copy.revision, 1);
                if calendar {
                    assert!(copy.calendar_uid.is_none());
                    let s = copy.schedule.as_ref().unwrap();
                    assert!(!s.remind);
                    assert_eq!(s.repeat, Repeat::Weekly);
                    assert_eq!(s.start.time(), NaiveTime::from_hms_opt(9, 30, 0).unwrap());
                }
                println!(
                    "PASS native explicit copy after comparison preserves remote and creates selected clean independent record; calendar={calendar}"
                );
            }
            6 => {
                self.preview_conflict_copy(4);
            }
            7 => {
                assert!(self.conflict_review_open());
                assert!(self.pending.is_none() && !self.error && self.has_unsaved());
                assert_eq!(
                    store::load(&self.path).unwrap().len(),
                    usize::from(!deleted)
                );
                println!(
                    "PASS native Ctrl S in comparison performs no foreground save and preserves both versions"
                );
            }
            _ => unreachable!(),
        }
        true
    }
}
