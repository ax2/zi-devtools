//! Native checks of the shared tool-entry route, including retained image drafts.
use super::*;

impl DevToolsApp {
    pub fn preview_image_entry_prepare(&mut self, ctx: &egui::Context, light: bool) -> String {
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.startup_warning = None;
        self.images.preview_fixture(ctx);
        self.tool_state.input = "preserved text draft".into();
        self.data_state.input = "preserved table draft".into();
        self.images.preview_entry_fingerprint()
    }

    pub fn preview_image_entry_check(&self, fingerprint: &str) -> String {
        assert_eq!(self.page, Page::Images);
        assert!(!self.launcher_open);
        assert_eq!(self.images.preview_entry_fingerprint(), fingerprint);
        assert!(!self.images.background_active());
        assert_eq!(self.tool_state.input, "preserved text draft");
        assert_eq!(self.data_state.input, "preserved table draft");
        self.images.active_tool_id().into()
    }
}
