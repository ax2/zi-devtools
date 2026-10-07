use super::*;

impl DevToolsApp {
    pub fn preview_service_scene(&mut self, ctx: &egui::Context, scene: usize, fixture: &Path) {
        self.page = Page::Services;
        self.notification.clear();
        self.selected_service = Some("demo".into());
        match scene {
            456 | 457 => {
                self.preview_service_attention_prepare();
                self.service_filter = ServiceFilter::Attention;
            }
            450..=453 => {
                self.preview_service_scale_prepare();
                if scene >= 452 {
                    self.search = "fixture-499".into();
                    self.selected_service = Some("fixture-499".into());
                }
            }
            444 | 445 => {
                let old = self.manager.service_spec("demo").unwrap();
                let mut spec = old.clone();
                spec.command = "exit /b 37".into();
                self.manager.save_service(Some(&old), spec).unwrap();
                assert!(self.manager.start("demo").is_err());
                self.statuses = self.manager.list_services();
                assert_eq!(self.statuses[0].last_exit.as_ref().unwrap().code, Some(37));
                self.refresh_inflight = false;
                self.request_refresh();
            }
            446 | 447 => {
                self.service_editor.new_service();
                let mut spec = self.manager.service_spec("demo").unwrap();
                spec.id = "native".into();
                spec.name = "教程示例服务".into();
                self.service_editor.preview_fill(spec);
            }
            448 | 449 => {
                let log = self
                    .manager
                    .config_snapshot()
                    .state_dir
                    .join("logs/demo.log");
                fs::write(log,"[INFO] 本地示例启动\n[INFO] 正在等待连接\n[WARN] 这是示例日志，未连接真实服务\n").unwrap();
                self.request_logs("demo".into());
            }
            _ => {}
        }
        let _ = (ctx, fixture);
    }
    pub fn preview_service_scale_prepare(&mut self) {
        self.page = Page::Services;
        self.startup_warning = None;
        self.notification.clear();
        self.selected_service = None;
        self.search.clear();
        self.service_compact = None;
        self.refresh_cancel.store(true, Ordering::Release);
        self.refresh_generation = self.refresh_generation.wrapping_add(1);
        self.refresh_inflight = true;
        let base = self.manager.list_services().remove(0);
        self.statuses = (0..500)
            .map(|index| {
                let mut status = base.clone();
                status.id = if index == 0 {
                    "demo".into()
                } else {
                    format!("fixture-{index:03}")
                };
                status.name = format!("开发服务 {index:03} · 这是完整服务名称");
                status.description = "合成规模样例，不代表 500 个真实运行进程".into();
                status.tags = vec!["开发环境".into()];
                status
            })
            .collect();
        fs::write(
            self.manager
                .config_snapshot()
                .state_dir
                .join("logs/demo.log"),
            "scale fixture log\n",
        )
        .unwrap();
    }
    pub fn preview_service_attention_prepare(&mut self) {
        self.preview_service_scale_prepare();
        let status = &mut self.statuses[499];
        status.state = ServiceState::Running;
        status.managed = true;
        status.health_url = Some("http://127.0.0.1:9/health".into());
        status.health.ok = Some(false);
        status.health.status_code = Some(503);
        assert_eq!(
            self.statuses.iter().filter(|s| s.needs_attention()).count(),
            1
        );
    }

    pub fn preview_service_attention_check(&self, phase: u8) {
        assert_eq!(self.statuses.len(), 500);
        assert_eq!(self.service_filter, ServiceFilter::Attention);
        assert_eq!(self.preview_service_rows, 1);
        if phase == 1 {
            assert_eq!(self.selected_service.as_deref(), Some("fixture-499"));
        }
    }
    pub fn preview_service_scale_check(&mut self, phase: u8) {
        match phase {
            0 => {
                assert_eq!(self.statuses.len(), 500);
                assert!(
                    self.preview_service_rows > 0 && self.preview_service_rows < 30,
                    "painted {}",
                    self.preview_service_rows
                );
            }
            1 => {
                assert_eq!(self.search, "fixture-499");
                assert_eq!(self.preview_service_rows, 1);
            }
            2 => {
                assert_eq!(self.selected_service.as_deref(), Some("fixture-499"));
                self.selected_service = None;
                self.search.clear();
            }
            3 => {
                assert_eq!(self.log_view.as_deref(), Some("demo"));
                assert!(!self.log_inflight);
                assert!(self.log_text.contains("scale fixture log"));
            }
            _ => unreachable!(),
        }
    }
    pub fn preview_service_position(&self, key: &str) -> egui::Pos2 {
        let (rect, clip) = self
            .preview_services
            .get(key)
            .or_else(|| self.service_editor.rects.get(key))
            .unwrap_or_else(|| {
                panic!(
                    "service control not rendered: {key}; dashboard {:?}; editor {:?}",
                    self.preview_services.keys(),
                    self.service_editor.rects.keys()
                )
            });
        assert!(
            rect.intersect(*clip).contains(rect.center()),
            "service control clipped: {key}"
        );
        rect.center()
    }
    pub fn preview_service_check(&mut self, phase: u8) -> bool {
        match phase {
            0 => {
                assert_eq!(self.page, Page::Services);
                true
            }
            1 => {
                let mut spec = self.manager.service_spec("demo").unwrap();
                spec.id = "native".into();
                spec.name = "Native saved".into();
                self.service_editor.preview_fill(spec);
                true
            }
            2 => {
                if !self.service_editor.preview_ready()
                    || !self.statuses.iter().any(|s| s.id == "native")
                {
                    return false;
                }
                let saved = self.manager.service_spec("native").unwrap();
                assert_eq!(saved.name, "Native saved");
                assert_eq!(
                    load_config(&self.manager.config_snapshot().path)
                        .unwrap()
                        .services["native"],
                    saved
                );
                true
            }
            3 => {
                let mut spec = self.manager.service_spec("native").unwrap();
                spec.name = "Native edited".into();
                self.service_editor.preview_fill(spec);
                true
            }
            4 => {
                if !self.service_editor.preview_ready()
                    || !self
                        .statuses
                        .iter()
                        .any(|s| s.id == "native" && s.name == "Native edited")
                {
                    return false;
                }
                assert_eq!(
                    self.manager.service_spec("native").unwrap().name,
                    "Native edited"
                );
                true
            }
            5 => {
                if self.manager.service_spec("native").is_ok() {
                    return false;
                }
                assert!(self.manager.service_spec("demo").is_ok());
                assert!(self.manager.service_ids().len() == 1);
                true
            }
            6 => {
                if self.log_inflight || self.log_view.as_deref() != Some("demo") {
                    return false;
                }
                assert!(self.log_text.contains("fixture log"));
                assert!(self.log_auto && self.log_follow);
                true
            }
            _ => unreachable!(),
        }
    }
    pub fn preview_service_smoke_prepare(&mut self) {
        self.page = Page::Home;
        self.startup_warning = None;
        fs::write(
            self.manager
                .config_snapshot()
                .state_dir
                .join("logs/demo.log"),
            "fixture log\nWARN demo\n",
        )
        .unwrap();
    }
    pub fn preview_service_stale_queue(&self) {
        let mut stale = self.statuses.clone();
        assert!(!stale.is_empty());
        for status in &mut stale {
            status.name = "stale-snapshot".into();
        }
        self.event_tx
            .send(BackgroundEvent::Statuses(
                self.refresh_generation.wrapping_sub(1),
                stale,
            ))
            .unwrap();
    }
    pub fn preview_service_stale_check(&self) {
        assert!(
            self.statuses
                .iter()
                .all(|status| status.name != "stale-snapshot")
        );
    }
    pub fn preview_tray_scene(&mut self, ctx: &egui::Context, folder: &Path, index: usize) {
        let light = index % 2 == 1;
        ctx.input_mut(|i| {
            i.raw.system_theme = Some(if light {
                egui::Theme::Light
            } else {
                egui::Theme::Dark
            })
        });
        if self.preview_tray_capture.is_none() {
            // Intentionally use the opposite main theme to verify independence.
            self.set_theme(ctx, if light { Theme::Dark } else { Theme::Light });
            self.open_tray_context(ctx);
            self.quick_tab = "服务".into();
            self.preview_panel_frames = 0;
            self.preview_tray_capture = Some(folder.join(if light {
                "tray-panel-light.png"
            } else {
                "tray-panel-dark.png"
            }));
        }
        assert_eq!(self.theme, if light { Theme::Dark } else { Theme::Light });
    }
}
