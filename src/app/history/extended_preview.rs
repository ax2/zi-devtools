use super::*;
impl DevToolsApp {
    pub fn preview_history_extended_prepare(
        &mut self,
        ctx: &egui::Context,
        light: bool,
    ) -> (String, String, String) {
        let fingerprint = self.preview_image_entry_prepare(ctx, light);
        let mut count = 0;
        for entry in catalog().iter().cloned() {
            self.open_entry(&entry);
            let location = self.navigation_location();
            self.recorder
                .select_audio_mode(crate::recorder::AudioMode::None);
            self.page = Page::Home;
            self.restore_navigation(&location).unwrap();
            assert!(
                location.same_place(&self.navigation_location()),
                "route {}",
                entry.id
            );
            if entry.page != Page::Plugins {
                assert_eq!(self.current_entry().unwrap().id, entry.id);
            }
            assert_eq!(
                self.recorder.preview_navigation_audio(),
                crate::recorder::AudioMode::None
            );
            count += 1;
        }
        println!("PASS restore all {count} registered built-in entries; recorder audio unchanged");
        let root = std::env::temp_dir().join(format!("zi-history-plugin-{}", uuid::Uuid::new_v4()));
        self.plugins.store = crate::plugins::Store::load(root);
        self.plugins
            .store
            .install(include_bytes!("../../../plugins-examples/local-text.json"))
            .unwrap();
        self.plugins.store.set_enabled("local-text", true).unwrap();
        let (a, b) = self.images.preview_navigation_images();
        self.page = Page::Images;
        self.navigation_history = State::default();
        (a, b, fingerprint)
    }
    pub fn preview_history_extended_step(&mut self, phase: u8, b: &str) {
        match phase {
            1 => self.open_startup_tool("json"),
            2 => {
                self.images.navigation_select_instance(b).unwrap();
                self.open_startup_tool("image-workflow");
            }
            3 => self.open_startup_tool("plugin:local-text/deduplicate"),
            4 => self.open_startup_tool("clock-workbench"),
            5 => self.plugins.store.set_enabled("local-text", false).unwrap(),
            6 => {
                self.open_startup_tool("screen-recorder-audio-mix");
                self.recorder
                    .select_audio_mode(crate::recorder::AudioMode::None);
            }
            7 => self.open_library("全部", "全部分类"),
            8 => {
                self.open_library("全部", "全部分类");
                self.library_query = "uuid".into();
            }
            _ => unreachable!(),
        }
    }
    pub fn preview_history_extended_check(
        &self,
        phase: u8,
        a: &str,
        b: &str,
        fingerprint: &str,
        scroll: f32,
    ) -> f32 {
        assert_eq!(self.images.preview_entry_fingerprint(), fingerprint);
        self.images.preview_navigation_images_check();
        match phase {
            1 | 2 => {
                assert_eq!(self.page, Page::Images);
                assert_eq!(self.images.active_tool_id(), "image-workflow");
                assert_eq!(
                    self.images.navigation_instance().0,
                    if phase == 1 { a } else { b }
                );
            }
            3 => {
                assert_eq!(self.page, Page::Recorder);
                assert_eq!(self.recorder.active_tool_id(), "screen-recorder-audio-mix");
                assert_eq!(
                    self.recorder.preview_navigation_audio(),
                    crate::recorder::AudioMode::None
                );
            }
            4 => {
                assert_eq!(self.page, Page::Library);
                assert!(
                    self.library_scroll_offset > 200.0,
                    "scroll={}",
                    self.library_scroll_offset
                );
            }
            5 => {
                assert_eq!(self.page, Page::Library);
                assert!(
                    (self.library_scroll_offset - scroll).abs() < 2.0,
                    "restored={} saved={scroll}",
                    self.library_scroll_offset
                );
            }
            6 => assert_eq!(self.page, Page::Library),
            7 => {
                assert_eq!(self.page, Page::Library);
                assert_eq!(self.library_query, "uuid");
                assert!(self.library_scroll_offset < 2.0);
            }
            _ => unreachable!(),
        }
        println!(
            "PASS extended navigation phase {phase}; instances, outputs and settings retained"
        );
        self.library_scroll_offset
    }
    pub fn preview_history_tasks_prepare(
        &mut self,
        ctx: &egui::Context,
        light: bool,
    ) -> (String, String) {
        self.preview_image_entry_prepare(ctx, light);
        let ids = self.images.preview_navigation_tasks(ctx);
        self.page = Page::Images;
        self.navigation_history = State::default();
        ids
    }
    pub fn preview_history_task_check(&self, id: &str, finished: bool) {
        assert_eq!(self.page, Page::Images);
        assert_eq!(self.images.navigation_instance().0, id);
        self.images.preview_navigation_task_check(finished);
        println!("PASS owned background image tasks retained across history; finished={finished}");
    }
    pub fn preview_history_task_cancel(&mut self) {
        self.images.preview_navigation_cancel();
    }
    pub fn preview_history_handoff_prepare(&mut self, ctx: &egui::Context, light: bool) {
        self.preview_delimited_handoff_prepare(ctx, light, false);
        self.navigation_history = State::default();
    }
    pub fn preview_history_handoff_check(&self, source: bool) -> u64 {
        assert!(self.handoff.is_none());
        assert_eq!(self.page, Page::Data);
        assert_eq!(self.data_state.instances.len(), 2);
        assert_eq!(
            self.data_state.active_id(),
            self.data_state.instances[usize::from(!source)].id
        );
        assert_eq!(
            self.data_state.instances[0].state.input,
            "preserve existing data draft"
        );
        assert_eq!(
            self.data_state.instances[0].state.output,
            "preserve existing export"
        );
        assert_eq!(
            self.data_state.instances[1].state.input,
            "name,count\nZi,2\n工具,3"
        );
        assert_eq!(
            self.data_state.instances[1].state.parse_job.phase,
            crate::tasks::Phase::Done
        );
        println!("PASS actual confirmed table handoff history; source={source}");
        self.data_state.instances[1].state.parse_job.generation()
    }
    pub fn preview_history_task_page(&mut self) {
        self.navigate(Page::Tasks, None);
    }
}
