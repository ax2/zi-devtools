use super::*;
impl DevToolsApp {
    pub fn preview_instances_prepare(
        &mut self,
        ctx: &egui::Context,
        light: bool,
        flow_folder: &std::path::Path,
    ) -> (String, String, String) {
        self.preview_image_entry_prepare(ctx, light);
        self.images.preview_navigation_images();
        let image = self.images.navigation_entries()[1].key.id.clone();
        let a = self.data_state.active_id().to_owned();
        self.data_state.instances[0].name = "重复材料".into();
        self.data_state.preview_workflow_search_fixture(flow_folder);
        self.data_state.input = "source A remains".into();
        self.data_state.output = "export A remains".into();
        let b = self.data_state.create("重复材料").unwrap();
        self.data_state.input = "source B remains".into();
        self.data_state.set_active_tool("data-transform");
        for i in 3..=16 {
            self.data_state.create(&format!("批次 {i:02}")).unwrap();
        }
        self.data_state.instances[15].name = format!("最长名字{}", "示例材料".repeat(19));
        self.data_state.select(&a).unwrap();
        self.page = Page::Data;
        self.navigation_history = Default::default();
        self.instance_search = Default::default();
        (a, b, image)
    }
    pub fn preview_instances_open(&mut self, ctx: &egui::Context, quick: bool, query: &str) {
        if quick {
            ctx.set_embed_viewports(true);
            self.open_quick(ctx);
            self.quick_tab = "工作实例".into();
            self.quick_category = "全部分类".into();
        } else {
            self.open_launcher();
        }
        self.launcher_scope = launcher::Scope::Instances;
        self.launcher_query = query.into();
    }
    pub fn preview_instances_scope_all(&mut self) {
        self.launcher_scope = launcher::Scope::All;
    }
    pub fn preview_instances_mixed_begin(&mut self, phase: u8, data_id: &str) {
        if phase == 0 {
            self.data_state
                .instances
                .iter_mut()
                .find(|instance| instance.id == data_id)
                .unwrap()
                .name = "JSON".into();
        }
        self.open_launcher();
        self.launcher_scope = launcher::Scope::All;
        self.launcher_query = "JSON".into();
    }
    pub fn preview_instances_mixed_arm(&mut self, phase: u8, data_id: &str) {
        let results = self.launcher_results("JSON");
        let flow_offset = results.saved.len() + results.recent.len() + results.tools.len();
        let instance_offset = flow_offset + results.workflows.entries.len();
        if phase == 0 {
            assert!(results.tools.iter().any(|entry| entry.title == "JSON"));
            assert!(
                results
                    .workflows
                    .entries
                    .iter()
                    .any(|entry| entry.name == "JSON")
            );
            assert!(results.instances.iter().any(|entry| {
                entry.key.kind == Kind::Data && entry.key.id == data_id && entry.name == "JSON"
            }));
            println!(
                "PASS mixed All-scope search: tools={} checked_flows={} instances={}",
                results.tools.len(),
                results.workflows.total,
                results.instances.len()
            );
        }
        self.launcher_index = match phase {
            0 => results
                .tools
                .iter()
                .position(|entry| entry.id == "json")
                .map(|index| results.saved.len() + results.recent.len() + index)
                .unwrap(),
            1 => results
                .instances
                .iter()
                .position(|entry| entry.key.kind == Kind::Data && entry.key.id == data_id)
                .map(|index| instance_offset + index)
                .unwrap(),
            2 => results
                .workflows
                .entries
                .iter()
                .position(|entry| entry.name == "JSON")
                .map(|index| flow_offset + index)
                .unwrap(),
            _ => unreachable!(),
        };
        self.instance_search.main.armed = true;
    }
    pub fn preview_instances_mixed_check(&self, phase: u8, data_id: &str) {
        assert!(!self.launcher_open);
        match phase {
            0 => {
                let expected = self
                    .entries("")
                    .into_iter()
                    .find(|entry| entry.id == "json")
                    .unwrap();
                assert_eq!(self.page, expected.page);
                println!("PASS Enter in mixed search opens the selected built-in tool route");
            }
            1 => {
                assert_eq!(self.page, Page::Data);
                assert_eq!(self.data_state.active_id(), data_id);
                println!("PASS Enter in mixed search opens the selected data instance identity");
            }
            2 => {
                assert_eq!(self.page, Page::Data);
                assert_eq!(self.data_state.active_id(), data_id);
                println!("PASS Enter in mixed search opens the selected checked-flow owner");
            }
            _ => unreachable!(),
        }
    }
    pub fn preview_instances_rect(&self, ctx: &egui::Context, image: &str) -> egui::Rect {
        let key = Key {
            kind: Kind::Image,
            id: image.into(),
        };
        assert!(
            self.instance_results(&self.launcher_query)
                .iter()
                .any(|entry| entry.key == key)
        );
        let rect: egui::Rect = ctx
            .data(|data| data.get_temp(egui::Id::new(key.token())))
            .unwrap();
        assert!(rect.width() > 0.0 && rect.height() > 0.0);
        rect
    }
    pub fn preview_instances_mutate(&mut self, phase: u8, a: &str, b: &str) {
        match phase {
            1 => {
                self.data_state
                    .instances
                    .iter_mut()
                    .find(|i| i.id == a)
                    .unwrap()
                    .name = "已改名材料".into()
            }
            2 => self.data_state.close(b, true).unwrap(),
            3 => self.launcher_query = "已改名材料".into(),
            _ => unreachable!(),
        }
    }
    pub fn preview_instances_check(&mut self, phase: u8, a: &str, b: &str, image: &str) {
        println!(
            "CHECK phase={phase} open={} quick={} query={:?} index={} armed={} page={:?}",
            self.launcher_open,
            self.quick_open,
            self.launcher_query,
            self.launcher_index,
            if self.quick_open {
                self.instance_search.quick.armed
            } else {
                self.instance_search.main.armed
            },
            self.page
        );
        let source = self
            .data_state
            .instances
            .iter()
            .find(|i| i.id == a)
            .unwrap();
        assert_eq!(source.state.input, "source A remains");
        assert_eq!(source.state.output, "export A remains");
        self.images.preview_navigation_images_check();
        match phase {
            0 => {
                assert_eq!(self.launcher_scope, launcher::Scope::Instances);
                assert_eq!(self.instance_entries().len(), 18);
                let results = self.launcher_results("重复材料");
                assert!(
                    results.tools.is_empty()
                        && results.saved.is_empty()
                        && results.recent.is_empty()
                        && results.workflows.entries.is_empty()
                );
                assert_eq!(results.instances.len(), 2);
            }
            1 => {
                assert!(self.launcher_open || self.quick_open);
                assert_eq!(self.data_state.active_id(), a);
            }
            2 => {
                assert!(!self.launcher_open && !self.quick_open);
                assert_eq!(self.page, Page::Data);
                assert_eq!(self.data_state.active_id(), b);
                assert_eq!(self.data_state.active_tool_id(), "data-transform");
                assert_eq!(self.data_state.input, "source B remains");
            }
            3 => {
                assert_eq!(self.page, Page::Images);
                assert_eq!(self.images.navigation_instance().0, image);
            }
            4 => {
                assert!(self.launcher_open || self.quick_open);
                assert!(!if self.quick_open {
                    self.instance_search.quick.armed
                } else {
                    self.instance_search.main.armed
                });
            }
            5 => {
                assert!(!self.launcher_open && !self.quick_open);
                assert_eq!(self.data_state.active_id(), a);
            }
            _ => unreachable!(),
        }
        println!(
            "PASS instance search phase {phase}; identity, source draft and image results retained"
        );
    }
    pub fn preview_instances_guard(&mut self, ctx: &egui::Context, image: &str) {
        let original = self.data_state.active_id().to_owned();
        self.clear_tool_confirm = true;
        assert!(
            self.navigate_instance(
                ctx,
                &Key {
                    kind: Kind::Image,
                    id: image.into()
                }
            )
            .is_err()
        );
        assert_eq!(self.page, Page::Data);
        assert_eq!(self.data_state.active_id(), original);
        self.clear_tool_confirm = false;
        assert!(
            self.navigate_instance(
                ctx,
                &Key {
                    kind: Kind::Data,
                    id: "already closed".into()
                }
            )
            .is_err()
        );
        assert_eq!(self.data_state.active_id(), original);
        println!("PASS blocked and unavailable instance targets do not change current work");
    }
    pub fn preview_instances_tasks(&mut self, ctx: &egui::Context) -> (String, String) {
        self.images.preview_navigation_tasks(ctx)
    }
    pub fn preview_instances_task_open(&mut self, ctx: &egui::Context, id: &str) {
        assert!(self.open_instance_result(
            ctx,
            &Key {
                kind: Kind::Image,
                id: id.into()
            }
        ));
    }
    pub fn preview_instances_task_check(&self, id: &str, finished: bool) {
        assert_eq!(self.images.navigation_instance().0, id);
        self.images.preview_navigation_task_check(finished);
        println!("PASS searched instance background ownership; finished={finished}");
    }
    pub fn preview_instances_b_finished(&self) -> bool {
        self.images.preview_navigation_b_finished()
    }
}
