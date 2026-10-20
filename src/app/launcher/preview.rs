use super::*;
impl DevToolsApp {
    pub fn preview_launcher_keyboard_quick(&mut self, ctx: &egui::Context) {
        ctx.set_embed_viewports(true);
        self.open_quick(ctx);
        self.quick_tab = "全部".into();
        self.quick_category = "全部分类".into();
    }

    pub fn preview_launcher_keyboard_state(&mut self) -> (bool, usize, String, String) {
        let first = self
            .launcher_results(&self.launcher_query.clone())
            .tools
            .first()
            .map(|e| e.id.clone())
            .unwrap_or_default();
        (
            self.launcher_open || self.quick_open,
            self.launcher_index,
            self.launcher_query.clone(),
            first,
        )
    }
    pub fn preview_launcher_keyboard_opened(&self, id: &str, fingerprint: &str) {
        assert!(!self.launcher_open && !self.quick_open);
        let entry = self
            .entries("")
            .into_iter()
            .find(|entry| entry.id == id)
            .unwrap();
        assert_eq!(self.page, entry.page);
        if entry.page == Page::Images {
            assert_eq!(self.images.active_tool_id(), id);
        }
        assert_eq!(self.images.preview_entry_fingerprint(), fingerprint);
        assert_eq!(self.tool_state.input, "preserved text draft");
        assert_eq!(self.data_state.input, "preserved table draft");
    }
}
