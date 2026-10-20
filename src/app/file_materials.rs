use super::*;
impl DevToolsApp {
    pub(super) fn route_file_material(&mut self) {
        if self.handoff.is_some() {
            return;
        }
        let Some(material) = self.file_state.file_transfer.take() else {
            return;
        };
        match handoff::Transfer::file("批量文件校验 · 原文件".into(), material) {
            Ok(transfer) => self.handoff = Some(transfer),
            Err(error) => self.toast = Some((error.to_string(), Instant::now())),
        }
    }
}

#[cfg(feature = "ui-preview")]
impl DevToolsApp {
    pub fn preview_file_material_prepare(
        &mut self,
        ctx: &egui::Context,
        light: bool,
        path: PathBuf,
    ) {
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.startup_warning = None;
        self.page = Page::Files;
        self.file_encoding.preview_fixture();
        self.file_state.preview(path);
    }
    pub fn preview_file_material_check(&self, phase: usize) {
        if phase == 0 {
            assert_eq!(self.file_state.job.phase, crate::tasks::Phase::Done);
            let report: serde_json::Value = serde_json::from_str(self.file_state.report()).unwrap();
            assert_eq!(report["bytes"], 3);
            assert!(!self.file_state.report().contains("Users"));
            assert!(self.handoff_source().is_some());
        } else if phase == 4 {
            assert!(self.page == Page::Files);
            assert!(self.file_state.file_transfer.is_none());
            assert!(self.handoff.as_ref().unwrap().preview_file_ready());
        } else if phase == 3 {
            assert!(self.page == Page::Files);
            assert!(self.handoff.is_none());
            assert!(self.file_state.file_transfer.is_none());
            assert!(!self.file_state.report().is_empty());
            self.file_encoding.preview_original_draft_check();
        } else {
            assert!(self.page == Page::FileEncoding);
            self.file_encoding.preview_material_check(phase == 2);
            assert!(self.file_state.file_transfer.is_none());
        }
    }
    pub fn preview_file_material_apply_position(&self) -> egui::Pos2 {
        self.handoff.as_ref().unwrap().preview_apply_position()
    }
}
