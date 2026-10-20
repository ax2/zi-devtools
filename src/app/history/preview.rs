use super::*;
impl DevToolsApp {
    pub fn preview_history_prepare(
        &mut self,
        ctx: &egui::Context,
        light: bool,
    ) -> (String, String) {
        let fingerprint = self.preview_image_entry_prepare(ctx, light);
        self.data_state.input = "history A".into();
        self.data_state.set_active_tool("data");
        self.page = Page::Data;
        self.navigation_history = State::default();
        (self.data_state.active_id().into(), fingerprint)
    }
    pub fn preview_history_step(&mut self, phase: u8, second: &str) -> String {
        match phase {
            1 => self.open_startup_tool("image-crop-annotate"),
            2 => {
                self.data_state.create("History B").unwrap();
                self.data_state.input = "history B".into();
                self.data_state.set_active_tool("data-transform");
                self.page = Page::Data;
                return self.data_state.active_id().into();
            }
            3 => {
                self.open_library("全部", "全部分类");
                self.library_query = "json".into();
            }
            4 => self.open_startup_tool("json"),
            5 => self.open_startup_tool("uuid"),
            6 => self.data_state.close(second, true).unwrap(),
            7 => self.clear_tool_confirm = true,
            8 => self.clear_tool_confirm = false,
            _ => unreachable!(),
        }
        String::new()
    }
    pub fn preview_history_check(&self, phase: u8, first: &str, second: &str, fingerprint: &str) {
        assert_eq!(self.images.preview_entry_fingerprint(), fingerprint);
        assert_eq!(
            self.tool_state.preview_draft_input(ToolKind::default()),
            Some("preserved text draft")
        );
        assert_eq!(
            self.data_state
                .instances
                .iter()
                .find(|i| i.id == first)
                .unwrap()
                .state
                .input,
            "history A"
        );
        if let Some(instance) = self.data_state.instances.iter().find(|i| i.id == second) {
            assert_eq!(instance.state.input, "history B");
        }
        match phase {
            1 => {
                assert_eq!(self.page, Page::Images);
                assert_eq!(self.images.active_tool_id(), "image-crop-annotate");
            }
            2 => {
                assert_eq!(self.page, Page::Data);
                assert_eq!(self.data_state.active_id(), first);
                assert_eq!(self.data_state.active_tool_id(), "data");
            }
            3 => {
                assert_eq!(self.page, Page::Data);
                assert_eq!(self.data_state.active_id(), second);
                assert_eq!(self.data_state.active_tool_id(), "data-transform");
            }
            4 => {
                assert_eq!(self.page, Page::Library);
                assert_eq!(self.library_query, "json");
                assert_eq!(self.home_category, "全部分类");
            }
            5 => {
                assert_eq!(self.tool_state.selected.id(), "uuid");
                assert!(
                    self.navigation_history
                        .history
                        .candidate(false, |l| self.history_location_available(l))
                        .is_none()
                );
            }
            6 => {
                assert_eq!(self.page, Page::Images);
                assert!(self.clear_tool_confirm);
            }
            _ => unreachable!(),
        }
        println!("PASS history phase {phase}; exact instance/view and drafts retained");
    }
}
