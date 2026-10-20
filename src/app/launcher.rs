use super::*;
pub(super) mod keyboard;
#[cfg(feature = "ui-preview")]
mod preview;
mod workflows;

fn filter_category(entries: &mut Vec<ToolEntry>, category: &str) {
    if category != "全部分类" {
        entries.retain(|entry| entry.category == category);
    }
}

pub(super) fn filter_collection(
    entries: &mut Vec<ToolEntry>,
    collection: &crate::preferences::ToolCollection,
    preserve_relevance: bool,
) {
    entries.retain(|entry| collection.tool_ids.contains(&entry.id));
    if !preserve_relevance {
        entries.sort_by_key(|entry| {
            collection
                .tool_ids
                .iter()
                .position(|id| id == &entry.id)
                .unwrap_or(usize::MAX)
        });
    }
}

pub(super) fn sort_by_usage(
    entries: &mut [ToolEntry],
    usage: &std::collections::BTreeMap<String, u32>,
    recent: &[String],
) {
    entries.sort_by_key(|entry| {
        (
            std::cmp::Reverse(usage.get(&entry.id).copied().unwrap_or(0)),
            recent
                .iter()
                .position(|id| id == &entry.id)
                .unwrap_or(usize::MAX),
        )
    });
}

#[cfg(test)]
mod category_tests {
    use super::*;

    #[test]
    fn quick_panel_category_filter_keeps_only_matching_tools_and_all_mode_is_unchanged() {
        let all = super::super::registry::catalog();
        let mut selected = all.to_vec();
        filter_category(&mut selected, "文件与系统");
        assert!(!selected.is_empty());
        assert!(selected.iter().all(|entry| entry.category == "文件与系统"));
        assert!(!selected.iter().any(|entry| entry.id == "json"));

        let mut unchanged = all.to_vec();
        filter_category(&mut unchanged, "全部分类");
        assert_eq!(unchanged.len(), all.len());
    }

    #[test]
    fn quick_panel_collection_filter_keeps_the_curated_order() {
        let mut entries = super::super::registry::catalog().to_vec();
        let collection = crate::preferences::ToolCollection {
            id: uuid::Uuid::new_v4().to_string(),
            name: "任务常用".into(),
            tool_ids: vec!["uuid".into(), "json".into()],
        };
        filter_collection(&mut entries, &collection, false);
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            ["uuid", "json"]
        );
        let mut matching = vec![
            super::super::registry::catalog()
                .iter()
                .find(|entry| entry.id == "json")
                .unwrap()
                .clone(),
            super::super::registry::catalog()
                .iter()
                .find(|entry| entry.id == "uuid")
                .unwrap()
                .clone(),
        ];
        filter_collection(&mut matching, &collection, true);
        assert_eq!(
            matching
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            ["json", "uuid"]
        );
    }

    #[test]
    fn frequent_tools_use_recent_visit_to_break_usage_ties() {
        let catalog = super::super::registry::catalog();
        let mut entries = ["json", "uuid", "base64"]
            .into_iter()
            .map(|id| catalog.iter().find(|entry| entry.id == id).unwrap().clone())
            .collect::<Vec<_>>();
        let usage = std::collections::BTreeMap::from([
            ("json".to_owned(), 8),
            ("uuid".to_owned(), 8),
            ("base64".to_owned(), 3),
        ]);
        let recent = vec!["uuid".to_owned(), "json".to_owned()];

        sort_by_usage(&mut entries, &usage, &recent);

        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            ["uuid", "json", "base64"]
        );
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Scope {
    #[default]
    All,
    Tools,
    Workflows,
    Favorites,
    Instances,
}
impl Scope {
    pub const ALL: [(Self, &'static str, egui::Key); 5] = [
        (Self::All, "全部", egui::Key::Num1),
        (Self::Tools, "工具", egui::Key::Num2),
        (Self::Workflows, "流程", egui::Key::Num3),
        (Self::Favorites, "收藏", egui::Key::Num4),
        (Self::Instances, "工作实例", egui::Key::Num5),
    ];
}
pub(super) struct Results {
    pub tools: Vec<ToolEntry>,
    pub workflows: crate::workbench::sessions::WorkflowMatches,
    pub saved: Vec<usize>,
    pub recent: Vec<usize>,
    pub instances: Vec<crate::instance_navigation::Entry>,
}
pub(super) enum Choice {
    Tool(String),
    Workflow(crate::workbench::sessions::WorkflowMatch),
    SavedWorkflow(crate::preferences::SavedWorkflow),
}

impl DevToolsApp {
    pub(super) fn launcher_results(&mut self, query: &str) -> Results {
        let mut tools = if matches!(self.launcher_scope, Scope::Workflows | Scope::Instances) {
            Vec::new()
        } else {
            self.entries(query)
        };
        if self.launcher_scope == Scope::Favorites {
            tools.retain(|entry| self.preferences.favorites.contains(&entry.id));
        }
        let workflows = if matches!(self.launcher_scope, Scope::All | Scope::Workflows) {
            self.data_state.workflow_matches(
                if query.trim().is_empty() && self.launcher_scope == Scope::Workflows {
                    "流程"
                } else {
                    query
                },
            )
        } else {
            Default::default()
        };
        let saved = if matches!(self.launcher_scope, Scope::Tools | Scope::Instances) {
            Vec::new()
        } else {
            self.preferences
                .workflow_favorites
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.matches(query))
                .map(|(index, _)| index)
                .collect()
        };
        let recent = if matches!(self.launcher_scope, Scope::All | Scope::Workflows) {
            self.preferences
                .workflow_recent
                .iter()
                .enumerate()
                .filter(|(_, entry)| {
                    entry.matches_recent(query)
                        && !self
                            .preferences
                            .workflow_favorites
                            .iter()
                            .any(|favorite| favorite.path == entry.path)
                })
                .map(|(index, _)| index)
                .collect()
        } else {
            Vec::new()
        };
        Results {
            tools,
            workflows,
            saved,
            recent,
            instances: if matches!(self.launcher_scope, Scope::All | Scope::Instances) {
                self.instance_results(query)
            } else {
                Vec::new()
            },
        }
    }
    pub(super) fn open_search_choice(&mut self, choice: Choice) {
        match choice {
            Choice::SavedWorkflow(selected) => {
                match self.data_state.open_workflow_bookmark(&selected) {
                    Ok(()) => {
                        self.page = Page::Data;
                        self.launcher_open = false;
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
        self.instance_search.quick = Default::default();
        self.quick_context = false;
        self.prefix.active = false;
        self.quick_open = true;
        self.quick_keyboard = Default::default();
        self.quick_active.store(true, Ordering::Release);
        self.quick_focus = true;
        self.quick_had_focus = false;
        self.quick_opened = Instant::now();
        self.launcher_open = false;
        self.launcher_query.clear();
        self.launcher_index = 0;
        self.quick_tab = if self.preferences.favorites.is_empty()
            && self.preferences.workflow_favorites.is_empty()
        {
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
        self.instance_search.quick = Default::default();
        self.open_quick(ctx);
        self.quick_context = true;
        if self.preferences.recent.is_empty()
            && self.preferences.favorites.is_empty()
            && self.preferences.workflow_recent.is_empty()
            && self.preferences.workflow_favorites.is_empty()
        {
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
            self.open_startup_files(paths);
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
        let mut workflow_chosen = None;
        let mut workflow_search = false;
        let mut instance_chosen = None;
        let contextual = self.quick_context;
        let system_theme = ctx
            .input(|i| i.raw.system_theme)
            .unwrap_or(egui::Theme::Light);
        let original_colors = self.colors;
        if self.quick_context {
            self.colors = palette(if system_theme == egui::Theme::Dark {
                Theme::Dark
            } else {
                Theme::Light
            });
        }
        let panel_bg = self.colors.bg;
        let panel_surface = self.colors.surface;
        let panel_width = self.quick_size.x;
        ctx.show_viewport_immediate(id, builder, |panel, viewport_class| {
            #[cfg(feature = "ui-preview")]
            if let Some((_, frame)) = self.preview_tray_workflow.as_mut() {
                if *frame == 40 {
                    panel.input_mut(|i| {
                        i.events.push(egui::Event::Key {
                            key: egui::Key::Enter,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        })
                    });
                }
                *frame += 1;
            }
            let keyboard_allowed = self.quick_keyboard.allow_shortcuts(panel);
            let focused = panel.input(|i| i.viewport().focused.unwrap_or(false));
            if focused {
                self.quick_had_focus = true;
            }
            if panel.input(|i| i.viewport().close_requested())
                || (keyboard_allowed && keyboard::consume_plain(panel, egui::Key::Escape))
                || (self.quick_had_focus
                    && self.quick_opened.elapsed() > Duration::from_millis(300)
                    && !focused
                    && !self.quick_focus
                    && panel.input(|i| i.raw.hovered_files.is_empty()))
            {
                self.quick_open = false;
            }
            let mut panel_contents = |ui: &mut egui::Ui| {
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
                        ui.small("搜索 · 收藏 · 实例 · 流程 · 服务");
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
                        .hint_text("搜索工具、流程、实例、用途…"),
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
                ui.horizontal_wrapped(|ui| {
                    for tab in [
                        "收藏",
                        "最近",
                        "常用",
                        "集合",
                        "全部",
                        "流程",
                        "工作实例",
                        "服务",
                    ] {
                        let tab_color = if self.quick_tab == tab {
                            Color32::WHITE
                        } else {
                            self.colors.text
                        };
                        if ui
                            .selectable_value(
                                &mut self.quick_tab,
                                tab.into(),
                                RichText::new(tab).color(tab_color),
                            )
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
                if self.quick_tab == "集合" {
                    if self.preferences.tool_collections.is_empty() {
                        ui.horizontal(|ui| {
                            ui.label("还没有工具集合。");
                            if ui.small_button("去工具库创建 →").clicked() {
                                self.page = Page::Library;
                                self.quick_open = false;
                                show_main = true;
                            }
                        });
                    } else {
                        if !self
                            .preferences
                            .tool_collections
                            .iter()
                            .any(|collection| collection.id == self.quick_collection_id)
                        {
                            self.quick_collection_id =
                                self.preferences.tool_collections[0].id.clone();
                        }
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("集合").color(self.colors.muted));
                            egui::ComboBox::from_id_salt("quick-panel-collection")
                                .selected_text(
                                    self.preferences
                                        .tool_collections
                                        .iter()
                                        .find(|collection| {
                                            collection.id == self.quick_collection_id
                                        })
                                        .map_or("选择集合", |collection| {
                                            collection.name.as_str()
                                        }),
                                )
                                .width(ui.available_width().min(240.0))
                                .show_ui(ui, |ui| {
                                    for collection in &self.preferences.tool_collections {
                                        ui.selectable_value(
                                            &mut self.quick_collection_id,
                                            collection.id.clone(),
                                            format!(
                                                "{} · {}",
                                                collection.name,
                                                collection.tool_ids.len()
                                            ),
                                        );
                                    }
                                });
                        });
                    }
                }
                if !matches!(self.quick_tab.as_str(), "流程" | "工作实例") {
                    let mut categories = self
                        .entries("")
                        .into_iter()
                        .map(|entry| entry.category)
                        .collect::<Vec<_>>();
                    categories.sort();
                    categories.dedup();
                    if self.quick_category != "全部分类"
                        && !categories
                            .iter()
                            .any(|category| category == &self.quick_category)
                    {
                        self.quick_category = "全部分类".into();
                    }
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("分类").color(self.colors.muted));
                        egui::ComboBox::from_id_salt("quick-panel-category")
                            .selected_text(&self.quick_category)
                            .width(ui.available_width().min(240.0))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.quick_category,
                                    "全部分类".into(),
                                    "全部分类",
                                );
                                for category in &categories {
                                    ui.selectable_value(
                                        &mut self.quick_category,
                                        category.clone(),
                                        category,
                                    );
                                }
                            });
                    });
                    if self.quick_category != "全部分类" {
                        ui.small("当前分类只筛选工具；流程入口请切换到“流程”搜索。");
                    }
                }
                let mut entries = self.entries(&self.launcher_query);
                if self.quick_tab == "流程" {
                    entries.clear();
                }
                let mut workflow_rows =
                    workflows::shortcuts(&self.preferences, &self.quick_tab, &self.launcher_query);
                let instances = if matches!(self.quick_tab.as_str(), "全部" | "工作实例") {
                    self.instance_results(&self.launcher_query)
                } else {
                    Vec::new()
                };
                if self.quick_tab == "工作实例" {
                    entries.clear();
                    workflow_rows.entries.clear();
                    workflow_rows.total = 0;
                }
                if self.quick_tab == "常用" {
                    entries.retain(|e| self.preferences.usage.get(&e.id).copied().unwrap_or(0) > 0);
                    sort_by_usage(
                        &mut entries,
                        &self.preferences.usage,
                        &self.preferences.recent,
                    );
                } else if self.quick_tab == "收藏" {
                    entries.retain(|e| self.preferences.favorites.contains(&e.id));
                    entries.sort_by_key(|e| {
                        self.preferences.favorites.iter().position(|id| id == &e.id)
                    });
                } else if self.quick_tab == "最近" {
                    entries.retain(|e| self.preferences.recent.contains(&e.id));
                    entries
                        .sort_by_key(|e| self.preferences.recent.iter().position(|id| id == &e.id));
                } else if self.quick_tab == "集合" {
                    if let Some(collection) = self
                        .preferences
                        .tool_collections
                        .iter()
                        .find(|collection| collection.id == self.quick_collection_id)
                    {
                        filter_collection(
                            &mut entries,
                            collection,
                            !self.launcher_query.trim().is_empty(),
                        );
                    } else {
                        entries.clear();
                    }
                }
                if self.quick_tab != "流程" {
                    filter_category(&mut entries, &self.quick_category);
                }
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!(
                        "{} 个工具 · {} 条流程 · {} 个实例",
                        entries.len(),
                        workflow_rows.total,
                        instances.len()
                    ))
                    .small()
                    .color(self.colors.muted),
                );
                let instance_offset = entries.len() + workflow_rows.entries.len();
                let keys = workflow_rows
                    .entries
                    .iter()
                    .map(|(e, kind)| format!("workflow:{kind}:{:?}", e.path))
                    .chain(entries.iter().map(|e| format!("tool:{}", e.id)))
                    .chain(instances.iter().map(|e| e.key.token()))
                    .collect();
                self.instance_search.quick.reconcile(
                    format!(
                        "quick:{}:{}:{}:{}",
                        self.quick_tab,
                        self.quick_category,
                        self.quick_collection_id,
                        self.launcher_query
                    ),
                    keys,
                    &mut self.launcher_index,
                );
                keyboard::preserve_guarded_focus(
                    panel,
                    &response,
                    keyboard_allowed
                        && instance_offset + instances.len() > 0
                        && self.instance_search.quick.armed,
                );
                let arrow_down =
                    keyboard_allowed && keyboard::consume_plain(panel, egui::Key::ArrowDown);
                let arrow_up =
                    keyboard_allowed && keyboard::consume_plain(panel, egui::Key::ArrowUp);
                if arrow_down {
                    self.launcher_index = self.launcher_index.saturating_add(1);
                }
                if arrow_up {
                    self.launcher_index = self.launcher_index.saturating_sub(1);
                }
                scroll_selection |= arrow_down || arrow_up;
                if arrow_down || arrow_up {
                    self.instance_search.quick.armed = true;
                }
                self.launcher_index = self
                    .launcher_index
                    .min((instance_offset + instances.len()).saturating_sub(1));
                egui::ScrollArea::vertical()
                    .id_salt("quick-results")
                    .max_height(
                        (ui.available_height()
                            - if workflow_rows.total > 0 || self.quick_tab == "流程" {
                                160.0
                            } else {
                                112.0
                            })
                        .max(40.0),
                    )
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if entries.is_empty()
                            && workflow_rows.entries.is_empty()
                            && instances.is_empty()
                        {
                            ui.add_space(24.0);
                            ui.strong("这里还没有匹配的工具或流程");
                            ui.label("切换“全部”或输入关键词。点击 ☆ 添加收藏。");
                        }
                        for (index, (entry, kind)) in workflow_rows.entries.iter().enumerate() {
                            let selected =
                                self.instance_search.quick.armed && index == self.launcher_index;
                            let name = if entry.name.chars().count() > 22 {
                                format!("{}…", entry.name.chars().take(22).collect::<String>())
                            } else {
                                entry.name.clone()
                            };
                            let button = ui
                                .add_sized(
                                    [ui.available_width(), 54.0],
                                    egui::Button::new(
                                        RichText::new(format!(
                                            "{} · {}步\n{} · 载入后确认",
                                            name, entry.steps, kind
                                        ))
                                        .color(
                                            if selected {
                                                Color32::WHITE
                                            } else {
                                                self.colors.text
                                            },
                                        ),
                                    )
                                    .selected(selected),
                                )
                                .on_hover_text(&entry.name);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(
                                        entry
                                            .path
                                            .file_name()
                                            .unwrap_or_default()
                                            .to_string_lossy(),
                                    )
                                    .size(12.0)
                                    .color(self.colors.muted),
                                )
                                .truncate(),
                            )
                            .on_hover_text(entry.path.display().to_string());
                            if selected && scroll_selection {
                                button.scroll_to_me(Some(egui::Align::Center));
                            }
                            if button.clicked() {
                                workflow_chosen = Some(entry.clone());
                            }
                            ui.add_space(6.0);
                        }
                        for (index, entry) in entries.iter().enumerate() {
                            let index = index + workflow_rows.entries.len();
                            ui.push_id(&entry.id, |ui| {
                                let selected = self.instance_search.quick.armed
                                    && index == self.launcher_index;
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
                                                .small_button(if favorite { "★" } else { "☆" })
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
                        for (offset, entry) in instances.iter().enumerate() {
                            if self.instance_row(
                                ui,
                                entry,
                                self.instance_search.quick.armed
                                    && instance_offset + offset == self.launcher_index,
                                scroll_selection,
                            ) {
                                instance_chosen = Some(entry.key.clone());
                            }
                        }
                    });
                if !self.instance_search.quick.armed {
                    ui.small("原选中项已关闭或不再匹配；请重新选择。");
                }
                if keyboard_allowed
                    && keyboard::consume_plain(panel, egui::Key::Enter)
                    && self.instance_search.quick.armed
                {
                    if let Some(entry) = instances
                        .get(self.launcher_index.saturating_sub(instance_offset))
                        .filter(|_| self.launcher_index >= instance_offset)
                    {
                        instance_chosen = Some(entry.key.clone());
                    } else if let Some((entry, _)) = workflow_rows.entries.get(self.launcher_index)
                    {
                        workflow_chosen = Some(entry.clone());
                    } else if let Some(entry) = entries.get(
                        self.launcher_index
                            .saturating_sub(workflow_rows.entries.len()),
                    ) {
                        chosen = Some(entry.id.clone());
                    }
                }
                if workflow_rows.total > 0 || self.quick_tab == "流程" {
                    if ui.button("全部流程搜索 ↗").clicked() {
                        workflow_search = true;
                    }
                    if workflow_rows.total > workflow_rows.entries.len() {
                        ui.small("收藏与最近各显示最多6条；完整列表请打开流程搜索。");
                    }
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
            };
            match viewport_class {
                egui::ViewportClass::Embedded => {
                    egui::Window::new("快捷面板")
                        .id(egui::Id::new("embedded-quick-panel"))
                        .title_bar(false)
                        .collapsible(false)
                        .resizable(false)
                        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -12.0))
                        .fixed_size(egui::vec2(
                            panel_width.min((panel.screen_rect().width() - 64.0).max(240.0)),
                            height.min((panel.screen_rect().height() - 64.0).max(240.0)),
                        ))
                        .frame(
                            egui::Frame::new()
                                .fill(panel_bg)
                                .inner_margin(20.0)
                                .stroke(egui::Stroke::new(1.0, panel_surface)),
                        )
                        .show(panel, &mut panel_contents);
                }
                egui::ViewportClass::Root
                | egui::ViewportClass::Immediate
                | egui::ViewportClass::Deferred => {
                    egui::CentralPanel::default()
                        .frame(
                            egui::Frame::new()
                                .fill(panel_bg)
                                .inner_margin(20.0)
                                .stroke(egui::Stroke::new(1.0, panel_surface)),
                        )
                        .show(panel, panel_contents);
                }
            }
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
        if let Some(key) = instance_chosen
            && self.open_instance_result(ctx, &key)
        {
            show_main = true;
        }
        if show_main {
            self.quick_open = false;
            restore_main_window(self.window_handle, ctx);
        }
        if let Some(entry) = workflow_chosen {
            self.open_tray_workflow(ctx, entry);
        }
        if workflow_search {
            self.quick_open = false;
            restore_main_window(self.window_handle, ctx);
            self.open_launcher();
            self.launcher_scope = Scope::Workflows;
        }
        if !self.quick_open {
            wake_main_window(self.window_handle, ctx);
        }
    }
}
