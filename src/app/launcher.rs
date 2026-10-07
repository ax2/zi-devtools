use super::*;
pub(super) enum Choice {
    Tool(String),
    Workflow(crate::workbench::sessions::WorkflowMatch),
    SavedWorkflow(crate::preferences::SavedWorkflow),
}

impl DevToolsApp {
    pub(super) fn open_search_choice(&mut self, choice: Choice) {
        match choice {
            Choice::SavedWorkflow(selected) => {
                match self.data_state.open_workflow_bookmark(&selected) {
                    Ok(()) => {
                        self.page = Page::Data;
                        self.launcher_open = false;
                        self.visit("pipeline");
                    }
                    Err(error) => self.toast = Some((format!("{error:#}"), Instant::now())),
                }
            }
            Choice::Tool(id) => {
                if let Some(entry) = self.entries("").into_iter().find(|entry| entry.id == id) {
                    self.open_entry(&entry);
                }
            }
            Choice::Workflow(selected) => match self.data_state.open_workflow_match(&selected) {
                Ok(()) => {
                    self.page = Page::Data;
                    self.launcher_open = false;
                    self.visit("pipeline");
                }
                Err(error) => self.toast = Some((format!("{error:#}"), Instant::now())),
            },
        }
    }
    pub(super) fn toggle_workflow_bookmark(&mut self, entry: crate::preferences::SavedWorkflow) {
        match self
            .preferences
            .toggle_workflow(&self.preferences_path, entry)
        {
            Ok(saved) => {
                self.toast = Some((
                    if saved {
                        "已收藏流程；下次可用Ctrl K直接找到，打开后仍需确认".into()
                    } else {
                        "已移除流程收藏，原文件保留".into()
                    },
                    Instant::now(),
                ))
            }
            Err(error) => self.toast = Some((format!("{error:#}"), Instant::now())),
        }
    }
}

impl DevToolsApp {
    pub(super) fn open_quick(&mut self, ctx: &egui::Context) {
        self.quick_context = false;
        self.prefix.active = false;
        self.quick_open = true;
        self.quick_active.store(true, Ordering::Release);
        self.quick_focus = true;
        self.quick_had_focus = false;
        self.quick_opened = Instant::now();
        self.launcher_open = false;
        self.launcher_query.clear();
        self.launcher_index = 0;
        self.quick_tab = if self.preferences.favorites.is_empty() {
            "最近"
        } else {
            "收藏"
        }
        .into();
        if let Some((position, size)) = panel_origin(ctx.pixels_per_point()) {
            self.quick_position = Some(position);
            self.quick_size = size;
        }
        ctx.request_repaint();
    }

    pub(super) fn open_tray_context(&mut self, ctx: &egui::Context) {
        self.open_quick(ctx);
        self.quick_context = true;
        if self.preferences.recent.is_empty() && self.preferences.favorites.is_empty() {
            self.quick_tab = "全部".into();
        }
    }

    fn tray_panel_actions(&mut self, ui: &mut egui::Ui, show_main: &mut bool) {
        ui.horizontal_wrapped(|ui| {
            if ui.small_button("本地服务").clicked() {
                self.quick_tab = "服务".into();
                self.launcher_query.clear();
            }
            if ui.small_button("设置").clicked() {
                self.page = Page::Settings;
                *show_main = true;
            }
            if ui.small_button("退出…").clicked() {
                self.quick_open = false;
                self.tray_exit_requested.store(true, Ordering::Release);
            }
            ui.small("跟随系统主题");
        });
        if !self.notification.is_empty() {
            ui.add(egui::Label::new(&self.notification).truncate())
                .on_hover_text(&self.notification);
        }
    }

    fn tray_services(&mut self, ui: &mut egui::Ui, show_main: &mut bool) {
        if ui.button("打开服务管理 · 新增 / 编辑 / 删除 ↗").clicked() {
            self.navigate(Page::Services, None);
            *show_main = true;
        }
        let query = self.launcher_query.to_lowercase();
        let statuses: Vec<_> = self
            .statuses
            .iter()
            .filter(|s| {
                format!("{} {} {:?}", s.name, s.id, s.port)
                    .to_lowercase()
                    .contains(&query)
            })
            .cloned()
            .collect();
        egui::ScrollArea::vertical()
            .id_salt("tray-services")
            .max_height((ui.available_height() - 40.0).max(50.0))
            .show(ui, |ui| {
                if statuses.is_empty() {
                    ui.label("没有匹配的服务，可在服务管理中添加。");
                }
                for status in statuses {
                    ui.push_id(&status.id, |ui| {
                        egui::Frame::new()
                            .fill(self.colors.card)
                            .corner_radius(9)
                            .inner_margin(10.0)
                            .show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                ui.horizontal_wrapped(|ui| {
                                    ui.strong(&status.name);
                                    ui.colored_label(
                                        if status.failed_exit() {
                                            self.colors.red
                                        } else if status.state.is_available()
                                            && (!status.managed || status.health.ok == Some(false))
                                        {
                                            self.colors.amber
                                        } else if status.managed {
                                            self.colors.green
                                        } else {
                                            self.colors.muted
                                        },
                                        status.display_state(),
                                    );
                                });
                                if status.pid.is_some() || status.port.is_some() {
                                    ui.small(format!(
                                        "{}{}",
                                        status
                                            .pid
                                            .map(|p| format!("PID {p}  "))
                                            .unwrap_or_default(),
                                        status
                                            .port
                                            .map(|p| format!("端口 {p}"))
                                            .unwrap_or_default()
                                    ));
                                }
                                ui.horizontal_wrapped(|ui| {
                                    if ui
                                        .add_enabled(
                                            !status.state.is_available()
                                                && !self.service_batch.busy()
                                                && !self.service_pending.contains_key(&status.id),
                                            egui::Button::new("启动"),
                                        )
                                        .clicked()
                                    {
                                        self.run_action(status.id.clone(), "start");
                                    }
                                    if ui
                                        .add_enabled(
                                            status.managed
                                                && !self.service_batch.busy()
                                                && !self.service_pending.contains_key(&status.id),
                                            egui::Button::new("停止"),
                                        )
                                        .clicked()
                                    {
                                        self.run_action(status.id.clone(), "stop");
                                    }
                                    if ui
                                        .add_enabled(
                                            status.managed
                                                && !self.service_batch.busy()
                                                && !self.service_pending.contains_key(&status.id),
                                            egui::Button::new("重启"),
                                        )
                                        .clicked()
                                    {
                                        self.run_action(status.id.clone(), "restart");
                                    }
                                    if ui.button("日志 ↗").clicked() {
                                        self.navigate(Page::Services, None);
                                        self.request_logs(status.id.clone());
                                        *show_main = true;
                                    }
                                });
                            });
                        ui.add_space(6.0);
                    });
                }
            });
    }

    pub(super) fn receive_drop(&mut self, ctx: &egui::Context) {
        let paths: Vec<_> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if !paths.is_empty() {
            self.intake.accept(paths);
            self.quick_open = false;
            self.launcher_open = false;
            self.navigate(Page::Intake, None);
            restore_main_window(self.window_handle, ctx);
        }
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("drop-hint"),
            ));
            let rect = ctx.screen_rect();
            painter.rect_filled(rect, 0.0, Color32::from_black_alpha(200));
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "松开文件 · 选择工具导入",
                egui::FontId::proportional(24.0),
                Color32::WHITE,
            );
        }
    }

    pub(super) fn apply_import(&mut self, value: crate::intake::Imported) -> anyhow::Result<()> {
        use crate::framework::Tool;
        use crate::intake::Target;
        match value.target {
            Target::Files => {
                self.file_state.append_paths(&value.paths)?;
                self.navigate(Page::Files, None);
            }
            Target::Csv | Target::Tsv | Target::JsonData => {
                let name = value
                    .paths
                    .first()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().chars().take(80).collect::<String>())
                    .unwrap_or_else(|| "导入数据".into());
                self.data_state.import_new(
                    value.text,
                    if value.target == Target::JsonData {
                        crate::workbench::DataFormat::Json
                    } else {
                        crate::workbench::DataFormat::Csv
                    },
                    value.target == Target::Tsv,
                    &name,
                )?;
                self.navigate(Page::Data, None);
            }
            Target::Json | Target::Java | Target::Python => {
                let kind = match value.target {
                    Target::Json => ToolKind::Json,
                    Target::Java => ToolKind::JavaTrace,
                    _ => ToolKind::DjangoTrace,
                };
                self.navigate(
                    if kind.is_encoding() {
                        Page::EncodingTools
                    } else {
                        Page::SmallTools
                    },
                    Some(kind),
                );
                self.tool_state.input = value.text;
                self.tool_state.clear_result();
            }
            target => {
                let tool = match target {
                    Target::Threads => Tool::Threads,
                    Target::Gc => Tool::Gc,
                    Target::Sql => Tool::Sql,
                    _ => Tool::Celery,
                };
                self.frameworks.import_text(tool, value.text)?;
                self.page = if tool.category() == "Java 与 JVM" {
                    Page::Java
                } else {
                    Page::Django
                };
                self.visit(tool.id());
            }
        }
        Ok(())
    }

    fn quick_recorder_controls(&mut self, ui: &mut egui::Ui) {
        use crate::recorder_ui::TrayRecordingStatus;
        let status = self.recorder.tray_status();
        if status == TrayRecordingStatus::Idle {
            return;
        }
        let duration = self.recorder.elapsed_duration();
        let label = match status {
            TrayRecordingStatus::Countdown => "即将开始录制".to_owned(),
            TrayRecordingStatus::Starting => "正在启动录制".to_owned(),
            TrayRecordingStatus::Recording | TrayRecordingStatus::Paused => {
                let elapsed = duration.unwrap_or_default().as_secs();
                format!(
                    "{}  {:02}:{:02}",
                    if status == TrayRecordingStatus::Paused {
                        "已暂停"
                    } else {
                        "录制中"
                    },
                    elapsed / 60,
                    elapsed % 60
                )
            }
            TrayRecordingStatus::Saving => "正在保存 MP4".to_owned(),
            TrayRecordingStatus::Idle => unreachable!(),
        };
        enum Action {
            Pause,
            Stop,
        }
        let mut action = None;
        egui::Frame::new()
            .fill(self.colors.surface)
            .corner_radius(9.0)
            .inner_margin(10.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("●").color(
                        if status == TrayRecordingStatus::Recording {
                            self.colors.red
                        } else {
                            self.colors.amber
                        },
                    ));
                    ui.strong(label);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if matches!(
                            status,
                            TrayRecordingStatus::Countdown
                                | TrayRecordingStatus::Starting
                                | TrayRecordingStatus::Recording
                                | TrayRecordingStatus::Paused
                        ) && ui
                            .small_button(if status == TrayRecordingStatus::Countdown {
                                "取消"
                            } else {
                                "停止保存"
                            })
                            .clicked()
                        {
                            action = Some(Action::Stop);
                        }
                        if matches!(
                            status,
                            TrayRecordingStatus::Recording | TrayRecordingStatus::Paused
                        ) && ui
                            .small_button(if status == TrayRecordingStatus::Paused {
                                "继续"
                            } else {
                                "暂停"
                            })
                            .clicked()
                        {
                            action = Some(Action::Pause);
                        }
                    });
                });
            });
        match action {
            Some(Action::Pause) => self.recorder.toggle_pause(),
            Some(Action::Stop) => {
                self.recorder.request_stop();
                self.quick_open = false;
            }
            None => {}
        }
        ui.add_space(12.0);
    }

    pub(super) fn quick_panel(&mut self, ctx: &egui::Context) {
        if !self.quick_open {
            return;
        }
        let id = egui::ViewportId::from_hash_of("zi-quick-panel");
        let height = if self.quick_context && self.quick_tab == "服务" {
            self.quick_size
                .y
                .min(320.0 + self.statuses.len().min(4) as f32 * 100.0)
        } else {
            self.quick_size.y
        };
        let mut builder = egui::ViewportBuilder::default()
            .with_title("Zi DevTools · 快捷面板")
            .with_inner_size(egui::vec2(self.quick_size.x, height))
            .with_decorations(false)
            .with_resizable(false)
            .with_taskbar(false)
            .with_window_level(egui::WindowLevel::AlwaysOnTop);
        if let Some(position) = self.quick_position {
            builder = builder.with_position(egui::pos2(
                position.x,
                position.y + self.quick_size.y - height,
            ));
        }
        let mut chosen = None;
        let mut show_main = false;
        let original_colors = self.colors;
        let contextual = self.quick_context;
        let system_theme = ctx
            .input(|i| i.raw.system_theme)
            .unwrap_or(egui::Theme::Light);
        if self.quick_context {
            self.colors = palette(if system_theme == egui::Theme::Dark {
                Theme::Dark
            } else {
                Theme::Light
            });
        }
        ctx.show_viewport_immediate(id, builder, |panel, _| {
            let focused = panel.input(|i| i.viewport().focused.unwrap_or(false));
            if focused {
                self.quick_had_focus = true;
            }
            if panel.input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape))
                || (self.quick_had_focus
                    && self.quick_opened.elapsed() > Duration::from_millis(300)
                    && !focused
                    && !self.quick_focus
                    && panel.input(|i| i.raw.hovered_files.is_empty()))
            {
                self.quick_open = false;
            }
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(self.colors.bg)
                        .inner_margin(20.0)
                        .stroke(egui::Stroke::new(1.0, self.colors.surface)),
                )
                .show(panel, |ui| {
                    if self.quick_context {
                        ui.style_mut().visuals = if system_theme == egui::Theme::Dark {
                            egui::Visuals::dark()
                        } else {
                            egui::Visuals::light()
                        };
                        ui.style_mut().visuals.override_text_color = Some(self.colors.text);
                        ui.style_mut().visuals.selection.bg_fill = self.colors.accent;
                        ui.style_mut().visuals.selection.stroke =
                            egui::Stroke::new(1.0, Color32::WHITE);
                        ui.style_mut().spacing.button_padding = egui::vec2(10.0, 6.0);
                        ui.style_mut().spacing.item_spacing = egui::vec2(8.0, 8.0);
                        ui.style_mut().visuals.widgets.inactive.weak_bg_fill = self.colors.card;
                        ui.style_mut().visuals.widgets.hovered.weak_bg_fill = self.colors.surface;
                    }
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Zi").size(28.0).color(self.colors.accent));
                        ui.vertical(|ui| {
                            ui.strong(if self.quick_context {
                                "托盘快捷菜单"
                            } else {
                                "工具速启"
                            });
                            ui.small("搜索 · 收藏 · 最近 · 服务");
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("×").on_hover_text("收起 · Esc").clicked() {
                                self.quick_open = false;
                            }
                            if ui.small_button("工作台 ↗").clicked() {
                                show_main = true;
                            }
                        });
                    });
                    if self.quick_context {
                        self.tray_panel_actions(ui, &mut show_main);
                    }
                    ui.add_space(14.0);
                    self.quick_recorder_controls(ui);
                    if self.prefix.active {
                        if self.quick_focus {
                            panel.send_viewport_cmd(egui::ViewportCommand::Focus);
                            panel.memory_mut(|memory| {
                                if let Some(id) = memory.focused() {
                                    memory.surrender_focus(id);
                                }
                            });
                            self.quick_focus = false;
                        }
                        chosen = self.prefix_ui(ui, panel);
                        return;
                    }
                    if ui.small_button("前缀快捷指令 →").clicked() {
                        self.prefix.open();
                        self.quick_focus = true;
                        return;
                    }
                    let response = ui.add_sized(
                        [ui.available_width(), 38.0],
                        egui::TextEdit::singleline(&mut self.launcher_query)
                            .hint_text("搜索工具、用途、关键词…"),
                    );
                    let mut scroll_selection = response.changed() || self.quick_focus;
                    if response.changed() {
                        self.launcher_index = 0;
                    }
                    if self.quick_focus {
                        response.request_focus();
                        panel.send_viewport_cmd(egui::ViewportCommand::Focus);
                        self.quick_focus = false;
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        for tab in ["收藏", "最近", "常用", "全部", "服务"] {
                            if ui
                                .selectable_value(&mut self.quick_tab, tab.into(), tab)
                                .clicked()
                            {
                                self.launcher_index = 0;
                            }
                        }
                    });
                    if self.quick_tab == "服务" {
                        self.tray_services(ui, &mut show_main);
                        return;
                    }
                    let mut entries = self.entries(&self.launcher_query);
                    if self.launcher_query.trim().is_empty() {
                        if self.quick_tab == "常用" {
                            entries.retain(|e| {
                                self.preferences.usage.get(&e.id).copied().unwrap_or(0) > 0
                            });
                            entries.sort_by_key(|e| {
                                std::cmp::Reverse(
                                    self.preferences.usage.get(&e.id).copied().unwrap_or(0),
                                )
                            });
                        } else if self.quick_tab == "收藏" {
                            entries.retain(|e| self.preferences.favorites.contains(&e.id));
                            entries.sort_by_key(|e| {
                                self.preferences.favorites.iter().position(|id| id == &e.id)
                            });
                        } else if self.quick_tab == "最近" {
                            entries.retain(|e| self.preferences.recent.contains(&e.id));
                            entries.sort_by_key(|e| {
                                self.preferences.recent.iter().position(|id| id == &e.id)
                            });
                        }
                    }
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(format!("{} 个工具", entries.len()))
                            .small()
                            .color(self.colors.muted),
                    );
                    let arrow_down = panel
                        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
                    let arrow_up = panel
                        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
                    if arrow_down {
                        self.launcher_index = self.launcher_index.saturating_add(1);
                    }
                    if arrow_up {
                        self.launcher_index = self.launcher_index.saturating_sub(1);
                    }
                    scroll_selection |= arrow_down || arrow_up;
                    self.launcher_index = self.launcher_index.min(entries.len().saturating_sub(1));
                    egui::ScrollArea::vertical()
                        .id_salt("quick-results")
                        .max_height((ui.available_height() - 96.0).max(40.0))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if entries.is_empty() {
                                ui.add_space(24.0);
                                ui.strong("这里还没有工具");
                                ui.label("切换“全部”或输入关键词。点击 ☆ 添加收藏。");
                            }
                            for (index, entry) in entries.iter().enumerate() {
                                ui.push_id(&entry.id, |ui| {
                                    let selected = index == self.launcher_index;
                                    egui::Frame::new()
                                        .fill(if selected {
                                            self.colors.surface
                                        } else {
                                            self.colors.card
                                        })
                                        .corner_radius(9)
                                        .inner_margin(10.0)
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                let favorite =
                                                    self.preferences.favorites.contains(&entry.id);
                                                if ui
                                                    .small_button(if favorite {
                                                        "★"
                                                    } else {
                                                        "☆"
                                                    })
                                                    .on_hover_text("收藏 / 取消收藏")
                                                    .clicked()
                                                {
                                                    self.toggle_favorite(&entry.id);
                                                }
                                                let button = ui.add_sized(
                                                    [ui.available_width(), 40.0],
                                                    egui::Button::new(format!(
                                                        "{}\n{}",
                                                        entry.title, entry.category
                                                    ))
                                                    .frame(false),
                                                );
                                                if selected && scroll_selection {
                                                    button.scroll_to_me(Some(egui::Align::Center));
                                                }
                                                if button.clicked() {
                                                    chosen = Some(entry.id.clone());
                                                }
                                            });
                                        });
                                    ui.add_space(6.0);
                                });
                            }
                        });
                    if panel.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
                        && let Some(entry) = entries.get(self.launcher_index)
                    {
                        chosen = Some(entry.id.clone());
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("拖入文件 / 导入…").clicked() {
                            chosen = Some("file-intake".into());
                        }
                        if !self.quick_context
                            && ui
                                .small_button(if self.theme == Theme::Dark {
                                    "亮主题"
                                } else {
                                    "暗主题"
                                })
                                .clicked()
                        {
                            self.set_theme(
                                panel,
                                if self.theme == Theme::Dark {
                                    Theme::Light
                                } else {
                                    Theme::Dark
                                },
                            );
                        }
                    });
                    let running = self.statuses.iter().filter(|s| s.managed).count();
                    if ui
                        .small_button(format!("服务：{running} 个托管运行  →"))
                        .clicked()
                    {
                        chosen = Some("services".into());
                    }
                    ui.small("↑ ↓ 选择   Enter 打开   Esc 收起");
                });
            self.receive_drop(panel);
            #[cfg(feature = "ui-preview")]
            {
                self.preview_panel_frames += 1;
            }
        });
        if contextual {
            self.colors = original_colors;
        }
        if let Some(id) = chosen
            && let Some(entry) = self.entries("").into_iter().find(|e| e.id == id)
        {
            self.open_entry(&entry);
            show_main = true;
        }
        if show_main {
            self.quick_open = false;
            restore_main_window(self.window_handle, ctx);
        }
        if !self.quick_open {
            wake_main_window(self.window_handle, ctx);
        }
    }
}
