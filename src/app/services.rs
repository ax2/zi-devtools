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
            ui.add_sized(
                [ui.available_width().min(420.0), 32.0],
                egui::TextEdit::singleline(&mut self.search).hint_text("名称、标签或端口"),
            );
        });
        ui.horizontal(|ui| {
            for filter in ServiceFilter::ALL {
                if ui
                    .selectable_label(self.service_filter == filter, filter.label())
                    .clicked()
                {
                    self.service_filter = filter;
                }
            }
        });

        let query = self.search.to_lowercase();
        let statuses: Vec<ServiceStatus> = self
            .statuses
            .iter()
            .filter(|status| {
                self.service_filter.matches(status.state, status.managed)
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
            .cloned()
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
        egui::ScrollArea::vertical().show(ui, |ui| {
            for status in statuses {
                self.service_card(ui, status);
                ui.add_space(10.0);
            }
        });
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
                    } else if status.managed && status.health.ok == Some(false) {
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
                if status.managed && status.health_url.is_some() && status.health.ok == Some(false)
                {
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
