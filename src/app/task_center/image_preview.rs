use super::*;
impl DevToolsApp {
    pub fn preview_image_tasks_prepare(&mut self, ctx: &egui::Context, light: bool) {
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.startup_warning = None;
        self.tasks = Default::default();
        self.images = Default::default();
        self.images.preview_image_tasks_fixture(ctx);
        self.page = Page::Tasks;
    }
    pub fn preview_image_tasks_ready(&self) -> bool {
        self.images.preview_image_tasks_ready()
    }
    pub fn preview_image_task_control(&self, action: &str) -> String {
        let row = self
            .tasks
            .rows
            .iter()
            .find(|r| r.instance_name.as_deref() == Some("教程图 A"))
            .unwrap();
        format!(
            "image-task-{}-{}-{}",
            action,
            row.instance.as_deref().unwrap(),
            row.generation
        )
    }
    pub fn preview_image_tasks_show(&mut self) {
        self.page = Page::Tasks;
    }
    pub fn preview_image_tasks_check(&self, phase: u8) {
        self.images.preview_image_tasks_check(phase.min(3));
        if phase == 5 {
            assert!(self.tasks.rows.is_empty());
            return;
        }
        let a = self
            .tasks
            .rows
            .iter()
            .find(|r| r.instance_name.as_deref() == Some("教程图 A"))
            .unwrap();
        assert_eq!(a.phase, Phase::Cancelled);
        if phase == 2 {
            assert_eq!(self.page, Page::Images);
        }
        if phase == 4 {
            assert_eq!(self.page, Page::Tasks);
            assert!(self.toast.as_ref().unwrap().0.contains("图片实例已关闭"));
        }
    }
}

impl DevToolsApp {
    pub fn preview_instance_relay_prepare(&mut self, ctx: &egui::Context, light: bool) {
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.startup_warning = None;
        self.images.preview_instance_relay_fixture();
        self.page = Page::Images;
    }
    pub fn preview_instance_relay_ready(&self) -> bool {
        self.images.preview_instance_relay_ready()
    }
    pub fn preview_instance_relay_control(&self) -> String {
        self.images.preview_instance_relay_control()
    }
    pub fn preview_instance_relay_check(&self, phase: u8) {
        self.images.preview_instance_relay_check(phase);
        assert_eq!(self.page, Page::Images);
    }
}

impl DevToolsApp {
    pub fn preview_budget_prepare(&mut self, ctx: &egui::Context, light: bool) {
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.startup_warning = None;
        self.images.preview_budget_fixture();
        self.page = Page::Images;
    }
    pub fn preview_budget_check(&self, ctx: &egui::Context, phase: u8) {
        self.images.preview_budget_check(phase);
        if phase == 4 {
            assert!(
                !ctx.data(|d| d.get_temp::<bool>(egui::Id::new("image-memory-menu-open")))
                    .unwrap()
            );
        }
    }
}
