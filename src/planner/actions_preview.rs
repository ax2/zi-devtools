use super::*;
impl State {
    pub fn preview_actions(&mut self, calendar: bool, error: bool) {
        if calendar {
            self.preview_cutoff();
        } else {
            self.preview(false, false);
            let mut item = self
                .items
                .iter()
                .find(|i| i.schedule.is_none())
                .unwrap()
                .clone();
            item.title = "资料整理与长篇备忘 · 固定保存操作".into();
            item.body = "记录资料和想法，编辑完成后保存到本机。\n".repeat(1000);
            self.items = vec![item.clone()];
            self.edit(item);
            self.focus_editor = false;
        }
        self.editor_action = None;
        if error {
            self.date_text = "无效日期".into();
            self.save_draft();
        }
    }
    pub fn preview_actions_smoke(&mut self, phase: u8) -> bool {
        match phase {
            0 => {
                self.preview_actions(false, false);
                self.path = std::env::temp_dir()
                    .join(format!("zi-actions-{}", uuid::Uuid::new_v4()))
                    .join("planner.sqlite3");
                let mut item = self.items[0].clone();
                item.revision = 0;
                store::save(&self.path, item).unwrap();
                self.items = store::load(&self.path).unwrap();
                self.edit(self.items[0].clone());
                self.focus_editor = false;
            }
            1 => {
                if self.pending.is_some() {
                    return false;
                }
                assert!(!self.error && !self.has_unsaved());
                let saved = store::load(&self.path).unwrap();
                assert!(
                    saved[0].title.contains("新增"),
                    "same-frame input omitted from shortcut save"
                );
                self.preview_action_y = Some((
                    self.preview_recurrence_rects[1].unwrap().0.min.y,
                    self.preview_title_rect.unwrap().0.min.y,
                ));
                self.draft
                    .as_mut()
                    .unwrap()
                    .body
                    .push_str("\n放弃这次未保存的正文修改");
            }
            2 => {
                assert!(self.has_unsaved());
                let (rect, clip) = self.preview_recurrence_rects[1].unwrap();
                assert!(clip.contains_rect(rect));
                assert_eq!(
                    rect.min.y,
                    self.preview_action_y.unwrap().0,
                    "save moved with page scroll"
                );
                assert!(
                    self.preview_title_rect.unwrap().0.min.y < self.preview_action_y.unwrap().1,
                    "outer page did not scroll"
                );
                let (rect, clip) = self.preview_discard_rect.unwrap();
                assert!(clip.contains_rect(rect));
                assert!(
                    !store::load(&self.path).unwrap()[0]
                        .body
                        .contains("放弃这次")
                );
            }
            3 => {
                assert!(!self.has_unsaved() && !self.error);
                assert!(!self.draft.as_ref().unwrap().body.contains("放弃这次"));
                assert!(store::load(&self.path).unwrap()[0].title.contains("新增"));
                std::fs::remove_dir_all(self.path.parent().unwrap()).unwrap();
                println!(
                    "PASS fixed actions: real title input and Ctrl S in same frame, save/reload, unchanged button position across scroll, actual discard restores persisted body"
                );
            }
            _ => unreachable!(),
        }
        true
    }
}
