use super::*;

#[derive(Clone, Copy)]
pub(super) enum Action {
    Save,
    Discard,
}
impl State {
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
        let dirty = self.has_unsaved();
        let available = external_enabled && self.editor_actions_available();
        let status = if self.pending.is_some() {
            "正在读写…"
        } else if revision == 0 {
            "尚未保存"
        } else if dirty {
            "有未保存修改"
        } else {
            "已保存"
        };
        let mut save = false;
        let mut discard = false;
        ui.horizontal(|ui| {
            let response =
                ui.add_enabled(available, egui::Button::new("保存到本机").selected(dirty));
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
        if save {
            self.queue_editor_action(Action::Save);
        } else if discard {
            self.queue_editor_action(Action::Discard);
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
                Action::Discard => self.discard(),
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
