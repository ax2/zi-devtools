//! Stable image tool IDs resolve to a view without executing image operations.
use super::{Mode, State};

impl Mode {
    const ALL: [Self; 6] = [
        Self::Single,
        Self::Batch,
        Self::Metadata,
        Self::Editor,
        Self::Screenshot,
        Self::Workflow,
    ];

    fn tool_id(self) -> &'static str {
        match self {
            Self::Single => "image-tools",
            Self::Batch => "image-batch",
            Self::Metadata => "image-metadata",
            Self::Editor => "image-crop-annotate",
            Self::Screenshot => "screenshot-workbench",
            Self::Workflow => "image-workflow",
        }
    }
}

impl State {
    #[cfg(feature = "ui-preview")]
    pub fn preview_navigation_images(&mut self) -> (String, String) {
        self.workflow.preview_relay_instances();
        self.show_mode(Mode::Workflow);
        (
            self.workflow.active_id(),
            self.workflow.preview_relay_target_id(),
        )
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_navigation_images_check(&self) {
        self.workflow.preview_navigation_images_check();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_navigation_tasks(&mut self, ctx: &eframe::egui::Context) -> (String, String) {
        self.workflow.preview_tasks_fixture(ctx);
        self.show_mode(Mode::Workflow);
        self.workflow.preview_navigation_task_ids()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_navigation_b_finished(&self) -> bool {
        self.workflow.preview_navigation_b_finished()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_navigation_task_check(&self, finished: bool) {
        self.workflow.preview_navigation_task_check(finished);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_navigation_cancel(&mut self) {
        self.workflow.preview_navigation_cancel();
    }

    pub(crate) fn navigation_entries(&self) -> Vec<crate::instance_navigation::Entry> {
        self.workflow.navigation_entries()
    }
    pub(crate) fn navigation_instance(&self) -> (String, String) {
        self.workflow.navigation_identity()
    }
    pub(crate) fn navigation_has_instance(&self, id: &str) -> bool {
        self.workflow.navigation_has(id)
    }
    pub(crate) fn navigation_select_instance(&mut self, id: &str) -> anyhow::Result<()> {
        self.workflow.navigation_select(id)
    }
    pub(crate) fn navigation_blocked(&self) -> bool {
        self.relay_active()
            || self.screenshot_capture_active()
            || self.workflow.navigation_blocked()
    }

    pub fn active_tool_id(&self) -> &'static str {
        self.mode.tool_id()
    }

    pub(crate) fn select_entry(&mut self, id: &str) -> bool {
        let Some(mode) = Mode::ALL.into_iter().find(|mode| mode.tool_id() == id) else {
            return false;
        };
        self.show_mode(mode);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::DynamicImage;
    use std::sync::Arc;

    #[test]
    fn every_pair_of_image_entries_restores_requested_view_and_preserves_drafts() {
        let source = Arc::new(DynamicImage::new_rgba8(3, 2));
        let encoded = Arc::new(vec![1, 2, 3, 4]);
        let mut state = State {
            source: Some(source.clone()),
            encoded: Some(encoded.clone()),
            input: "unchanged input".into(),
            output: "unchanged output".into(),
            width: 713,
            jpeg_quality: 83,
            ..State::default()
        };
        for from in Mode::ALL {
            for to in Mode::ALL {
                assert!(state.select_entry(from.tool_id()));
                assert!(state.select_entry(to.tool_id()));
                assert_eq!(state.active_tool_id(), to.tool_id());
                assert!(Arc::ptr_eq(state.source.as_ref().unwrap(), &source));
                assert!(Arc::ptr_eq(state.encoded.as_ref().unwrap(), &encoded));
                assert_eq!(state.input, "unchanged input");
                assert_eq!(state.output, "unchanged output");
                assert_eq!(state.width, 713);
                assert_eq!(state.jpeg_quality, 83);
                assert!(!state.background_active());
            }
        }
    }

    #[test]
    fn unknown_entry_does_not_fall_back_to_a_different_tool() {
        let mut state = State::default();
        assert!(state.select_entry("image-crop-annotate"));
        for id in [
            "",
            "image",
            "IMAGE-TOOLS",
            "plugin:image-tools",
            "not-an-image-tool",
        ] {
            assert!(!state.select_entry(id));
            assert_eq!(state.active_tool_id(), "image-crop-annotate");
        }
    }
}
