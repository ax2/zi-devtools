use super::DevToolsApp;
use eframe::egui;

impl DevToolsApp {
    pub fn preview_file_rename_action_position(&self, undo: bool) -> egui::Pos2 {
        self.file_rename.preview_action_position(undo)
    }

    pub fn preview_file_rename_execution_assert(&self, phase: u8) {
        self.file_rename.preview_execution_assert(phase);
    }
}
