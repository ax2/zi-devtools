use super::*;

#[derive(Clone, Copy)]
pub(super) enum Action {
    Save,
    SaveConflictCopy,
    CompareConflict,
    Discard,
    Duplicate,
    Convert,
}
impl State {
    /// Consume a page navigation request before rendering the outer scroll area.
    pub fn take_page_navigation(&mut self) -> Option<f32> {
        if std::mem::take(&mut self.focus_overview) {
            self.page_scroll_offset = None;
            Some(0.0)
        } else {
            self.page_scroll_offset.take()
        }
    }
    pub fn editor_open(&self) -> bool {
        self.draft.is_some()
    }
    fn editor_actions_available(&self) -> bool {
        self.loaded
            && self.pending.is_none()
            && self.draft.is_some()
            && self.purge_review.is_none()
            && self.export_review.is_none()
            && self.backup_review.is_none()
            && self.ics_review.is_none()
            && self.conflict_review.is_none()
    }
    fn queue_editor_action(&mut self, action: Action) {
        if self.editor_actions_available() {
            self.editor_action = Some((self.draft.as_ref().unwrap().id.clone(), action));
        }
    }
    /// Render outside the page scroll area; defer mutation until editor input lands.
    pub fn editor_actions_ui(&mut self, ui: &mut egui::Ui, external_enabled: bool) {
        let Some(item) = self.draft.as_ref() else {
            return;
        };
        let title = item.title.clone();
        let is_event = item.schedule.is_some();
        let revision = item.revision;
        let in_trash = item.trash;
        let dirty = self.has_unsaved();
        let available = external_enabled && self.editor_actions_available();
        let conflict = self.shared_conflict();
        let status = if self.pending.is_some() {
            "正在读写…"
        } else if conflict {
            "编辑冲突 · 修改已保留"
        } else if revision == 0 {
            "尚未保存"
        } else if dirty {
            "有未保存修改"
        } else {
            "已保存"
        };
        let mut save = false;
        let mut discard = false;
        let mut duplicate = false;
        let mut convert = false;
        let mut save_copy = false;
        let mut compare = false;
        if self.calendar {
            ui.horizontal(|ui| {
                for (index, label) in ["返回日程视图", "查看当前编辑"].into_iter().enumerate()
                {
                    let response = ui.add_enabled(external_enabled, egui::Button::new(label));
                    #[cfg(feature = "ui-preview")]
                    {
                        self.preview_navigation_rects[index] =
                            Some((response.rect, ui.clip_rect()));
                    }
                    if response.clicked() {
                        self.focus_overview = index == 0;
                        self.focus_editor = index == 1;
                    }
                    response.on_hover_text(
                        "只移动页面位置，保留草稿、筛选、日期和提醒状态；不会自动保存。",
                    );
                }
            });
        }
        if conflict {
            ui.push_id("planner-conflict-copy-actions", |ui| {
            ui.horizontal_wrapped(|ui| {
                let review = ui.add_enabled(available, egui::Button::new("对比冲突内容…"));
                compare = review.clicked();
                #[cfg(feature = "ui-preview")]
                {
                    self.preview_conflict_review_rects[0] = Some((review.rect, ui.clip_rect()));
                }
                let response = ui.add_enabled(available && revision > 0 && !in_trash,
                    egui::Button::new("将修改另存为新记录").selected(true));
                save_copy = response.clicked();
                #[cfg(feature = "ui-preview")]
                {
                    self.preview_conflict_copy_rect = Some((response.rect, ui.clip_rect()));

                }
                response.on_hover_text("保存当前标题、正文和有效日程为独立新记录，原记录保留；新日程提醒关闭。失败时保留当前修改。");
                ui.small("保留双方内容；日程副本不会自动开启提醒。");
            });
            });
        }
        ui.horizontal(|ui| {
            let response = ui.add_enabled(
                available,
                egui::Button::new("保存到本机").selected(dirty && !conflict),
            );
            save = response.clicked();
            #[cfg(feature = "ui-preview")]
            {
                self.preview_recurrence_rects[1] = Some((response.rect, ui.clip_rect()));
            }
            response.on_hover_text("保存当前编辑到本机 · Ctrl S");
            let response = ui.add_enabled(
                available && (dirty || revision == 0),
                egui::Button::new("放弃编辑"),
            );
            discard = response.clicked();
            #[cfg(feature = "ui-preview")]
            {
                self.preview_discard_rect = Some((response.rect, ui.clip_rect()));
            }
            response.on_hover_text("丢弃尚未保存的修改，恢复上次保存内容；新草稿会关闭。");
            let copy = ui.add_enabled(
                available && !dirty && revision > 0 && !in_trash,
                egui::Button::new("创建副本"),
            );
            duplicate = copy.clicked();
            #[cfg(feature = "ui-preview")]
            {
                self.preview_duplicate_rect = Some((copy.rect, ui.clip_rect()));
            }
            copy.on_hover_text("复制已保存内容为新草稿，不自动保存；日程副本默认关闭提醒。");
            let response = ui.add_enabled(
                available && !dirty && revision > 0 && !in_trash,
                egui::Button::new(if is_event {
                    "复制为备忘"
                } else {
                    "复制为日程"
                }),
            );
            convert = response.clicked();
            #[cfg(feature = "ui-preview")]
            {
                self.preview_convert_rect = Some((response.rect, ui.clip_rect()));
            }
            response
                .on_hover_text("只复制标题和正文为新草稿，原记录保留；不会自动保存或迁移提醒。");
            ui.separator();
            ui.label(status);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!(
                        "{} · {}",
                        if is_event { "日程" } else { "备忘" },
                        if title.is_empty() {
                            "未命名"
                        } else {
                            &title
                        }
                    ))
                    .weak(),
                )
                .truncate(),
            )
            .on_hover_text(&title);
        });
        if self.error && !self.message.is_empty() {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(&self.message).color(ui.visuals().error_fg_color),
                )
                .truncate(),
            )
            .on_hover_text(&self.message);
        }
        if compare {
            self.queue_editor_action(Action::CompareConflict);
        } else if save_copy {
            self.queue_editor_action(Action::SaveConflictCopy);
        } else if save {
            self.queue_editor_action(Action::Save);
        } else if discard {
            self.queue_editor_action(Action::Discard);
        } else if duplicate {
            self.queue_editor_action(Action::Duplicate);
        } else if convert {
            self.queue_editor_action(Action::Convert);
        }
    }
    pub fn finish_editor_actions(&mut self, save_shortcut: bool) {
        if save_shortcut && self.editor_action.is_none() {
            self.queue_editor_action(Action::Save);
        }
        if let Some((id, action)) = self.editor_action.take()
            && self.editor_actions_available()
            && self.draft.as_ref().is_some_and(|item| item.id == id)
        {
            match action {
                Action::Save => self.save_draft(),
                Action::CompareConflict => {
                    if self.shared_conflict() {
                        self.conflict_review = Some(id);
                    }
                }
                Action::SaveConflictCopy => {
                    if let Err(error) = self.save_conflict_copy() {
                        self.message = error.to_string();
                        self.error = true;
                    }
                }
                Action::Discard => self.discard(),
                Action::Convert => {
                    if let Err(error) = self.convert_draft() {
                        self.message = error.to_string();
                        self.error = true;
                    }
                }
                Action::Duplicate => {
                    if let Err(error) = self.duplicate_draft() {
                        self.message = error.to_string();
                        self.error = true;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::tests::{event, fixture, wait_state};
    #[test]
    fn queued_save_captures_later_input_and_discard_restores_saved_snapshot() {
        let path = fixture();
        let mut state = State::new(path.clone());
        wait_state(&mut state);
        state.edit(event());
        state.queue_editor_action(Action::Save);
        state.draft.as_mut().unwrap().body = "保存点击后本帧输入的正文".into();
        state.finish_editor_actions(false);
        wait_state(&mut state);
        assert_eq!(
            store::load(&path).unwrap()[0].body,
            "保存点击后本帧输入的正文"
        );
        state.draft.as_mut().unwrap().body = "未保存".into();
        state.queue_editor_action(Action::Discard);
        state.finish_editor_actions(false);
        assert_eq!(
            state.draft.as_ref().unwrap().body,
            "保存点击后本帧输入的正文"
        );
        assert!(!state.has_unsaved());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn duplicate_action_rechecks_same_frame_input_and_record_identity() {
        let path = fixture();
        let mut state = State::new(path.clone());
        wait_state(&mut state);
        state.edit(event());
        state.save_draft();
        wait_state(&mut state);
        let id = state.draft.as_ref().unwrap().id.clone();
        state.queue_editor_action(Action::Duplicate);
        state.draft.as_mut().unwrap().body = "本帧修改".into();
        state.finish_editor_actions(false);
        assert_eq!(state.draft.as_ref().unwrap().id, id);
        assert!(state.has_unsaved());
        state.discard();
        state.queue_editor_action(Action::Duplicate);
        state.edit(event());
        let replacement_id = state.draft.as_ref().unwrap().id.clone();
        state.finish_editor_actions(false);
        assert_eq!(state.draft.as_ref().unwrap().id, replacement_id);
        assert_eq!(store::load(&path).unwrap().len(), 1);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn conversion_action_rechecks_input_and_modal_after_queue() {
        let path = fixture();
        let mut state = State::new(path.clone());
        wait_state(&mut state);
        state.edit(event());
        state.save_draft();
        wait_state(&mut state);
        let id = state.draft.as_ref().unwrap().id.clone();
        state.queue_editor_action(Action::Convert);
        state.draft.as_mut().unwrap().title = "同帧修改".into();
        state.finish_editor_actions(false);
        assert_eq!(state.draft.as_ref().unwrap().id, id);
        assert!(state.has_unsaved());
        state.discard();
        state.queue_editor_action(Action::Convert);
        state.purge_review = Some(vec![event()]);
        state.finish_editor_actions(false);
        assert_eq!(state.draft.as_ref().unwrap().id, id);
        assert_eq!(store::load(&path).unwrap().len(), 1);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn queued_actions_bind_record_and_modal_blocks_shortcut_and_pending_request() {
        let path = fixture();
        let mut state = State::new(path.clone());
        wait_state(&mut state);
        state.edit(event());
        state.queue_editor_action(Action::Save);
        let mut replacement = event();
        replacement.title = "另一条".into();
        state.edit(replacement);
        state.finish_editor_actions(false);
        assert!(state.pending.is_none() && !path.exists());
        state.queue_editor_action(Action::Save);
        state.purge_review = Some(vec![event()]);
        state.finish_editor_actions(true);
        assert!(state.pending.is_none() && !path.exists() && state.editor_action.is_none());
        state.finish_editor_actions(true);
        assert!(state.pending.is_none());
    }
}
