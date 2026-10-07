use super::*;

impl DevToolsApp {
    pub(super) fn services_page(&mut self, ui: &mut egui::Ui) {
        let p = self.colors;
        let running = self
            .statuses
            .iter()
            .filter(|status| status.state == ServiceState::Running)
            .count();
        let external = self
            .statuses
            .iter()
            .filter(|status| {
                matches!(
                    status.state,
                    ServiceState::External | ServiceState::PortOpen
                )
            })
            .count();
        let stopped = self.statuses.len().saturating_sub(running + external);

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading(RichText::new("本地服务").size(28.0));
                ui.label(RichText::new("管理开发环境进程、健康状态与日志").color(p.muted));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let add = ui.button("＋ 新增服务");
                #[cfg(feature = "ui-preview")]
                self.preview_services
                    .insert("add", (add.rect, ui.clip_rect()));
                if add.clicked() {
                    self.service_editor.new_service();
                }
                if ui.button("全部停止").clicked() {
                    self.run_all(false);
                }
                if ui
                    .add(
                        egui::Button::new(RichText::new("全部启动").color(Color32::WHITE))
                            .fill(p.accent),
                    )
                    .clicked()
                {
                    self.run_all(true);
                }
                if ui.button("↻ 刷新").clicked() {
                    self.last_refresh = Instant::now() - Duration::from_secs(30);
                }
            });
        });
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            metric(ui, "托管运行", running, p.green);
            metric(ui, "外部/占用", external, p.amber);
            metric(ui, "已停止", stopped, p.muted);
            metric(ui, "服务总数", self.statuses.len(), p.accent);
        });
        ui.add_space(16.0);
        if let Some(warning) = self.startup_warning.clone() {
            egui::Frame::new()
                .fill(Color32::from_rgb(69, 55, 34))
                .corner_radius(8.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(warning).color(Color32::WHITE));
                        if ui.small_button("关闭提示").clicked() {
                            self.startup_warning = None;
                        }
                    });
                });
            ui.add_space(12.0);
        }
        if let Some(error) = self.config_error.clone() {
            egui::Frame::new()
                .fill(Color32::from_rgb(69, 37, 44))
                .corner_radius(8.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("服务配置无法加载：{error}"));
                        if ui.button("打开设置").clicked() {
                            self.page = Page::Settings;
                        }
                    });
                });
            ui.add_space(12.0);
        }
        if !self.notification.is_empty() {
            egui::Frame::new()
                .fill(if self.notification_error {
                    Color32::from_rgb(69, 37, 44)
                } else {
                    Color32::from_rgb(27, 65, 54)
                })
                .corner_radius(8.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&self.notification).color(Color32::WHITE));
                        if ui.small_button("关闭提示").clicked() {
                            self.notification.clear();
                        }
                    });
                });
            ui.add_space(12.0);
        }
        ui.horizontal(|ui| {
            ui.label("筛选服务");
            let search = ui.add_sized(
                [ui.available_width().min(420.0), 32.0],
                egui::TextEdit::singleline(&mut self.search).hint_text("名称、标签或端口"),
            );
            #[cfg(feature = "ui-preview")]
            self.preview_services
                .insert("scale-search", (search.rect, ui.clip_rect()));
            #[cfg(not(feature = "ui-preview"))]
            let _ = search;
        });
        ui.horizontal_wrapped(|ui| {
            for filter in ServiceFilter::ALL {
                let count = self
                    .statuses
                    .iter()
                    .filter(|status| {
                        filter.matches(status.state, status.managed, status.needs_attention())
                    })
                    .count();
                let response = ui.selectable_label(
                    self.service_filter == filter,
                    format!("{} {count}", filter.label()),
                );
                #[cfg(feature = "ui-preview")]
                if filter == ServiceFilter::Attention {
                    self.preview_services
                        .insert("attention", (response.rect, ui.clip_rect()));
                }
                if response.clicked() {
                    self.service_filter = filter;
                }
            }
        });
        if self.service_filter == ServiceFilter::Attention {
            ui.label(
                RichText::new("异常退出或健康检查未通过；按最近状态显示，不自动处理")
                    .small()
                    .color(p.muted),
            );
        }

        let query = self.search.to_lowercase();
        let statuses: Vec<usize> = self
            .statuses
            .iter()
            .enumerate()
            .filter(|(_, status)| {
                self.service_filter
                    .matches(status.state, status.managed, status.needs_attention())
                    && (query.is_empty()
                        || status.name.to_lowercase().contains(&query)
                        || status.id.to_lowercase().contains(&query)
                        || status
                            .tags
                            .iter()
                            .any(|tag| tag.to_lowercase().contains(&query))
                        || status
                            .port
                            .is_some_and(|port| port.to_string().contains(&query)))
            })
            .map(|(index, _)| index)
            .collect();
        ui.label(
            RichText::new(format!(
                "显示 {} / {} 项",
                statuses.len(),
                self.statuses.len()
            ))
            .small()
            .color(p.muted),
        );
        ui.add_space(8.0);
        if statuses.is_empty() {
            ui.add_space(24.0);
            ui.label(
                RichText::new(if self.statuses.is_empty() {
                    "还没有服务。点击“新增服务”配置工作目录和启动命令。"
                } else {
                    "没有匹配的服务，请调整筛选词。"
                })
                .color(p.muted),
            );
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("显示方式");
            ui.selectable_value(&mut self.service_compact, None, "自动");
            ui.selectable_value(&mut self.service_compact, Some(false), "卡片");
            ui.selectable_value(&mut self.service_compact, Some(true), "列表");
        });
        let compact = self.service_compact.unwrap_or(self.statuses.len() >= 30);
        #[cfg(feature = "ui-preview")]
        {
            self.preview_service_rows = 0;
        }
        if compact {
            ui.label(
                RichText::new("点击服务查看完整详情；右侧可直接打开日志")
                    .small()
                    .color(p.muted),
            );
            egui::ScrollArea::vertical()
                .id_salt("compact-services")
                .auto_shrink([false, false])
                .show_rows(ui, 56.0, statuses.len(), |ui, range| {
                    #[cfg(feature = "ui-preview")]
                    {
                        self.preview_service_rows += range.len();
                    }
                    for index in range {
                        self.compact_service_row(ui, self.statuses[statuses[index]].clone());
                    }
                });
            if let Some(id) = self.selected_service.clone() {
                if let Some(status) = self.statuses.iter().find(|s| s.id == id).cloned() {
                    let mut open = true;
                    egui::Window::new(format!("服务详情 · {}", status.name))
                        .id(egui::Id::new(("service-detail", &id)))
                        .open(&mut open)
                        .default_width(620.0)
                        .show(ui.ctx(), |ui| {
                            egui::ScrollArea::vertical()
                                .max_height(560.0)
                                .show(ui, |ui| {
                                    self.service_card(ui, status);
                                });
                        });
                    if !open {
                        self.selected_service = None;
                    }
                } else {
                    self.selected_service = None;
                }
            }
        } else {
            egui::ScrollArea::vertical()
                .id_salt("service-cards")
                .show(ui, |ui| {
                    for index in statuses {
                        self.service_card(ui, self.statuses[index].clone());
                        ui.add_space(10.0);
                    }
                });
        }
    }

    fn compact_service_row(&mut self, ui: &mut egui::Ui, status: ServiceStatus) {
        ui.push_id(status.id.clone(), |ui| {
            self.compact_service_row_content(ui, status)
        });
    }

    fn compact_service_row_content(&mut self, ui: &mut egui::Ui, status: ServiceStatus) {
        let p = self.colors;
        let pending = self.service_pending.get(&status.id).copied();
        let color = if status.failed_exit() {
            p.red
        } else if status.unhealthy() {
            p.amber
        } else if status.managed {
            p.green
        } else if status.state.is_available() {
            p.amber
        } else {
            p.muted
        };
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 56.0), egui::Sense::hover());
        ui.painter().rect_filled(
            rect,
            6.0,
            if self.selected_service.as_deref() == Some(&status.id) {
                p.accent.linear_multiply(0.15)
            } else {
                p.card
            },
        );
        let logs_rect = egui::Rect::from_min_max(
            egui::pos2(rect.max.x - 74.0, rect.min.y + 8.0),
            egui::pos2(rect.max.x - 8.0, rect.max.y - 8.0),
        );
        let main_rect = egui::Rect::from_min_max(
            rect.min + egui::vec2(8.0, 4.0),
            egui::pos2(logs_rect.min.x - 8.0, rect.max.y - 4.0),
        );
        let meta = if let Some(action) = pending {
            format!("正在{}…", action_label(action))
        } else {
            format!(
                "{} · {} · PID {} · 端口 {}",
                status.display_state(),
                status.id,
                status.pid.map_or("—".into(), |id| id.to_string()),
                status.port.map_or("—".into(), |port| port.to_string())
            )
        };
        let response = ui.interact(
            main_rect,
            ui.id().with("select-service"),
            egui::Sense::click(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &status.name)
        });
        for (text, offset, size, text_color) in [
            (&status.name, 4.0, 15.0, p.text),
            (&meta, 26.0, 12.0, color),
        ] {
            let mut job = egui::text::LayoutJob::simple(
                text.clone(),
                egui::FontId::proportional(size),
                text_color,
                main_rect.width(),
            );
            job.wrap.max_rows = 1;
            let galley = ui.fonts(|fonts| fonts.layout_job(job));
            ui.painter()
                .galley(main_rect.min + egui::vec2(4.0, offset), galley, text_color);
        }
        #[cfg(feature = "ui-preview")]
        if status.id == "fixture-499" {
            self.preview_services
                .insert("scale-last", (response.rect, ui.clip_rect()));
        }
        if response
            .on_hover_text(format!("{}\n{}\n{}", status.name, meta, status.description))
            .clicked()
        {
            self.selected_service = Some(status.id.clone());
        }
        let logs = ui.put(logs_rect, egui::Button::new("日志"));
        #[cfg(feature = "ui-preview")]
        if status.id == "demo" {
            self.preview_services
                .insert("logs", (logs.rect, ui.clip_rect()));
        }
        if logs.clicked() {
            self.request_logs(status.id);
        }
    }

    fn service_card(&mut self, ui: &mut egui::Ui, status: ServiceStatus) {
        let p = self.colors;
        let pending = self.service_pending.get(&status.id).copied();
        egui::Frame::new()
            .fill(p.card)
            .corner_radius(12.0)
            .inner_margin(16.0)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    let state_color = if status.failed_exit() {
                        p.red
                    } else if status.unhealthy() {
                        p.amber
                    } else {
                        match status.state {
                            ServiceState::Running => p.green,
                            ServiceState::External | ServiceState::PortOpen => p.amber,
                            ServiceState::Stopped => p.muted,
                        }
                    };
                    ui.label(RichText::new("●").color(state_color).size(17.0));
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&status.name).size(17.0).strong());
                            ui.label(
                                RichText::new(status.display_state())
                                    .color(state_color)
                                    .small(),
                            );
                            for tag in status.tags.iter().take(3) {
                                ui.label(RichText::new(tag).color(p.muted).small());
                            }
                        });
                        if !status.description.is_empty() {
                            ui.label(RichText::new(&status.description).color(p.muted));
                        }
                    });
                });
                ui.add_space(8.0);
                if let Some(exit) = &status.last_exit {
                    ui.label(
                        RichText::new(exit.summary())
                            .small()
                            .color(if exit.success { p.muted } else { p.red }),
                    );
                }
                if status.unhealthy() {
                    let detail = status
                        .health
                        .status_code
                        .map(|code| format!("HTTP {code}"))
                        .or_else(|| status.health.message.clone())
                        .unwrap_or_else(|| "请检查健康端点".into());
                    ui.label(
                        RichText::new(format!("健康检查未通过：{detail}"))
                            .small()
                            .color(p.amber),
                    );
                }
                if let Some(action) = pending {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(format!("正在{}…", action_label(action)));
                    });
                }
                ui.add_enabled_ui(pending.is_none(), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if status.managed {
                            if ui.button("停止").clicked() {
                                self.run_action(status.id.clone(), "stop");
                            }
                            if ui.button("重启").clicked() {
                                self.run_action(status.id.clone(), "restart");
                            }
                        } else if status.state == ServiceState::Stopped
                            && ui
                                .add(
                                    egui::Button::new(RichText::new("启动").color(Color32::WHITE))
                                        .fill(p.accent),
                                )
                                .clicked()
                        {
                            self.run_action(status.id.clone(), "start");
                        }
                        if matches!(
                            status.state,
                            ServiceState::External | ServiceState::PortOpen
                        ) {
                            ui.label(
                                RichText::new("外部进程占用：不执行启停")
                                    .small()
                                    .color(p.amber),
                            );
                        }
                        let logs = ui.button("查看日志");
                        #[cfg(feature = "ui-preview")]
                        if status.id == "demo" {
                            self.preview_services
                                .insert("logs", (logs.rect, ui.clip_rect()));
                        }
                        if logs.clicked() {
                            self.request_logs(status.id.clone());
                        }
                        let edit = ui
                            .add_enabled(!status.managed, egui::Button::new("编辑"))
                            .on_disabled_hover_text("请先停止托管服务");
                        #[cfg(feature = "ui-preview")]
                        if status.id == "native" {
                            self.preview_services
                                .insert("edit", (edit.rect, ui.clip_rect()));
                        }
                        if edit.clicked() {
                            self.service_editor.edit(&self.manager, &status.id);
                        }
                        let delete = ui
                            .add_enabled(!status.managed, egui::Button::new("删除"))
                            .on_disabled_hover_text("请先停止托管服务");
                        #[cfg(feature = "ui-preview")]
                        if status.id == "native" {
                            self.preview_services
                                .insert("delete", (delete.rect, ui.clip_rect()));
                        }
                        if delete.clicked() {
                            self.service_editor.delete(&self.manager, &status.id);
                        }
                    });
                });
                ui.horizontal_wrapped(|ui| {
                    if let Some(port) = status.port {
                        ui.label(RichText::new(format!("端口 {port}")).color(p.muted));
                    }
                    if let Some(pid) = status.pid {
                        ui.label(RichText::new(format!("PID {pid}")).color(p.muted));
                    }
                    if status.managed {
                        ui.label(RichText::new("本程序托管").color(p.green));
                    } else if status.port_open == Some(true) {
                        ui.label(RichText::new("检测到外部监听").color(p.amber));
                    }
                    if let Some(timeout) = status.graceful_stop_timeout_ms {
                        ui.label(RichText::new(format!("优雅停止 {timeout} ms")).color(p.muted));
                    }
                    if let Some(code) = status.health.status_code {
                        ui.label(RichText::new(format!("HTTP {code}")).color(p.muted));
                    }
                    if let Some(ms) = status.health.elapsed_ms {
                        ui.label(RichText::new(format!("{ms} ms")).color(p.muted));
                    }
                    ui.label(RichText::new(status.repo.display().to_string()).color(p.muted));
                });

                let expanded = self.selected_service.as_deref() == Some(&status.id);
                if ui
                    .small_button(if expanded {
                        "收起详情"
                    } else {
                        "查看详情"
                    })
                    .clicked()
                {
                    self.selected_service = if expanded {
                        None
                    } else {
                        Some(status.id.clone())
                    };
                }
                if expanded {
                    ui.separator();
                    ui.label(RichText::new("启动命令").small().color(p.muted));
                    ui.monospace(&status.command);
                    if let Some(url) = &status.health_url {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("健康检查").small().color(p.muted));
                            if ui.link(url).clicked() {
                                let _ = open::that(url);
                            }
                        });
                    }
                    ui.label(
                        RichText::new(format!(
                            "日志 {} · {} bytes · 环境变量 {} 项（值已隐藏）",
                            status.log_path.display(),
                            status.log_size,
                            status.env_count
                        ))
                        .small()
                        .color(p.muted),
                    );
                    if let Some(message) = &status.health.message {
                        ui.label(RichText::new(message).small().color(p.muted));
                    }
                    if !status.config_files.is_empty() {
                        ui.label(RichText::new("配置文件").small().color(p.muted));
                        for file in &status.config_files {
                            ui.horizontal(|ui| {
                                let marker = if file.exists { "●" } else { "○" };
                                ui.label(
                                    RichText::new(format!(
                                        "{marker} {} · {} bytes",
                                        file.configured_path, file.size
                                    ))
                                    .small()
                                    .color(if file.exists { p.muted } else { p.red }),
                                );
                                if file.exists && ui.small_button("查看").clicked() {
                                    self.request_config_preview(
                                        status.id.clone(),
                                        file.configured_path.clone(),
                                    );
                                }
                                if file.exists && ui.small_button("打开所在目录").clicked() {
                                    if let Some(parent) = file.absolute_path.parent() {
                                        let _ = open::that(parent);
                                    }
                                }
                            });
                        }
                    }
                }
            });
    }
}
