use super::*;

impl DevToolsApp {
    #[cfg(feature = "ui-preview")]
    pub fn preview_sidebar_scroll_delta(&self, name: &str) -> f32 {
        let (rect, clip) = self
            .preview_sidebar
            .get(name)
            .expect("sidebar control exists");
        if clip.contains_rect(*rect) {
            0.0
        } else {
            (clip.center().y - rect.center().y).clamp(-240.0, 240.0)
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_sidebar_position(&self, name: &str) -> egui::Pos2 {
        let (rect, clip) = self
            .preview_sidebar
            .get(name)
            .expect("sidebar control exists");
        assert!(
            clip.contains_rect(*rect),
            "{name} clipped: {rect:?}, {clip:?}"
        );
        rect.center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_sidebar_assert(&self, phase: u8) {
        match phase {
            0 => {
                for key in ["title", "back", "category", "theme", "tray", "settings"] {
                    self.preview_sidebar_position(key);
                }
            }
            1 => assert_eq!(self.theme, Theme::Light),
            2 => {
                assert_eq!(self.page, Page::Library);
                assert_eq!(self.home_category, "时间与生成");
            }
            3 => {
                assert_eq!(self.page, Page::Library);
                assert_eq!(self.home_filter, "收藏");
                assert_eq!(self.library_query, "日历");
            }
            4 => assert_eq!(self.page, Page::Settings),
            5 => {
                self.preview_sidebar_assert(0);
                self.preview_sidebar_position("sixth");
            }
            6 => {
                assert_eq!(self.page, Page::Notes);
                println!(
                    "PASS sidebar: minimum-height dock visible; real theme/category/back/settings clicks; scroll reaches sixth favorite and opens memo"
                );
            }
            7 => {
                assert_eq!(self.page, Page::Library);
                assert_eq!(self.home_filter, "常用");
                println!("PASS sidebar frequent shortcut selects usage-ranked library");
            }
            _ => unreachable!(),
        }
    }

    fn navigation_footer(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = self.colors;
        ui.vertical(|ui| {
            ui.add_space(8.0);
            ui.separator();
            ui.horizontal(|ui| {
                let theme = ui
                    .add_sized(
                        [36.0, 32.0],
                        egui::Button::new(RichText::new("☀").size(20.0)),
                    )
                    .on_hover_text(format!(
                        "当前{} · 点击切换主题，自动保存",
                        self.theme.label()
                    ));
                #[cfg(feature = "ui-preview")]
                self.preview_sidebar
                    .insert("theme", (theme.rect, ui.clip_rect()));
                if theme.clicked() {
                    self.set_theme(
                        ctx,
                        if self.theme == Theme::Dark {
                            Theme::Light
                        } else {
                            Theme::Dark
                        },
                    );
                }
                let tray = ui
                    .add_enabled(
                        self.tray.is_some(),
                        egui::Button::new(RichText::new("↓").size(20.0))
                            .min_size([36.0, 32.0].into()),
                    )
                    .on_hover_text("隐藏到托盘 · 任务继续运行；单击托盘图标可打开");
                #[cfg(feature = "ui-preview")]
                self.preview_sidebar
                    .insert("tray", (tray.rect, ui.clip_rect()));
                if tray.clicked() {
                    self.hide_to_tray(ctx);
                }
                let settings = ui
                    .add_sized(
                        [36.0, 32.0],
                        egui::Button::new(RichText::new("⚙").size(20.0)),
                    )
                    .on_hover_text("设置");
                #[cfg(feature = "ui-preview")]
                self.preview_sidebar
                    .insert("settings", (settings.rect, ui.clip_rect()));
                if settings.clicked() {
                    self.page = Page::Settings;
                }
            });
            ui.label(
                RichText::new(format!("Stage 77  ·  v{}", env!("CARGO_PKG_VERSION")))
                    .size(11.0)
                    .color(p.muted),
            );
        });
    }
}

impl DevToolsApp {
    fn open_library(&mut self, filter: &str, category: &str) {
        self.page = Page::Library;
        self.home_filter = filter.into();
        self.home_category = category.into();
        self.library_query.clear();
        self.launcher_open = false;
    }

    pub(super) fn sidebar(&mut self, ctx: &egui::Context) {
        let p = self.colors;
        #[cfg(feature = "ui-preview")]
        self.preview_sidebar.clear();
        egui::SidePanel::left("sidebar")
            .resizable(false)
            .exact_width(224.0)
            .frame(egui::Frame::new().fill(p.panel).inner_margin(16.0))
            .show(ctx, |ui| {
                // Reserve the tool context and footer before laying out the
                // independently scrolling navigation. Content count cannot push
                // these actions out of view at the supported minimum height.
                egui::TopBottomPanel::bottom("sidebar-dock")
                    .frame(egui::Frame::NONE)
                    .show_separator_line(false)
                    .default_height(210.0)
                    .show_inside(ui, |ui| {
                        self.current_tool_navigation(ui);
                        self.navigation_footer(ui, ctx);
                    });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Zi")
                            .size(27.0)
                            .strong()
                            .color(p.accent_hover),
                    );
                    ui.vertical(|ui| {
                        ui.label(RichText::new("DEVTOOLS").size(17.0).strong());
                        ui.label(
                            RichText::new("你的本地工具工作台")
                                .size(11.0)
                                .color(p.muted),
                        );
                    });
                });
                ui.add_space(18.0);
                if ui
                    .add_sized(
                        [ui.available_width(), 34.0],
                        egui::Button::new("搜索工具     Ctrl K"),
                    )
                    .clicked()
                {
                    self.open_launcher();
                }
                ui.add_space(14.0);
                ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();
                egui::ScrollArea::vertical()
                    .id_salt("sidebar-scroll")
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .show(ui, |ui| {
                        for (page, label) in [
                            (Page::Home, "开始"),
                            (Page::Library, "工具库"),
                            (Page::Tasks, "后台任务"),
                            (Page::Plugins, "扩展与连接器"),
                        ] {
                            if ui
                                .add_sized(
                                    [ui.available_width(), 38.0],
                                    egui::Button::new(label).selected(self.page == page),
                                )
                                .clicked()
                            {
                                if page == Page::Library {
                                    self.open_library("全部", "全部分类");
                                } else {
                                    self.navigate(page, None);
                                }
                                if page == Page::Plugins {
                                    self.plugins.selected = None;
                                }
                            }
                            ui.add_space(4.0);
                        }
                        ui.add_space(16.0);
                        ui.label(RichText::new("快捷访问").small().color(p.muted));
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if ui.button("全部收藏").clicked() {
                                self.open_library("收藏", "全部分类");
                            }
                            if ui.button("最近使用").clicked() {
                                self.open_library("最近", "全部分类");
                            }
                        });
                        let frequent = ui.add_sized(
                            [ui.available_width(), 30.0],
                            egui::Button::new("常用排行 →"),
                        );
                        #[cfg(feature = "ui-preview")]
                        self.preview_sidebar
                            .insert("frequent", (frequent.rect, ui.clip_rect()));
                        if frequent.clicked() {
                            self.open_library("常用", "全部分类");
                        }
                        let entries = self.entries("");
                        let pinned: Vec<_> = self
                            .preferences
                            .favorites
                            .iter()
                            .filter_map(|id| entries.iter().find(|e| &e.id == id))
                            .take(6)
                            .cloned()
                            .collect();
                        if pinned.is_empty() {
                            ui.label(
                                RichText::new("在工具旁点 ☆，固定常用入口")
                                    .small()
                                    .color(p.muted),
                            );
                        }
                        for (index, entry) in pinned.iter().enumerate() {
                            let favorite = ui
                                .add_sized(
                                    [ui.available_width(), 32.0],
                                    egui::Button::new(&entry.title).frame(false).truncate(),
                                )
                                .on_hover_text(format!("{}\n{}", entry.title, entry.description));
                            #[cfg(feature = "ui-preview")]
                            if index == 5 {
                                self.preview_sidebar
                                    .insert("sixth", (favorite.rect, ui.clip_rect()));
                            }
                            #[cfg(not(feature = "ui-preview"))]
                            let _ = index;
                            if favorite.clicked() {
                                self.open_entry(entry);
                            }
                        }
                        ui.add_space(10.0);
                    });
            });
    }

    fn current_tool_navigation(&mut self, ui: &mut egui::Ui) {
        if matches!(
            self.page,
            Page::Home | Page::Library | Page::Tasks | Page::Plugins | Page::Settings
        ) {
            return;
        }
        ui.separator();
        ui.small("当前工作台");
        let entries = self.entries("");
        let active = entries.iter().find(|e| {
            e.page == self.page
                && match e.kind {
                    Some(kind) => kind == self.tool_state.selected,
                    None if matches!(self.page, Page::Java | Page::Django) => {
                        e.id == self.frameworks.selected.id()
                    }
                    None => true,
                }
        });
        if let Some(entry) = active {
            let title = ui
                .add(
                    egui::Label::new(RichText::new(&entry.title).color(self.colors.accent_hover))
                        .truncate(),
                )
                .on_hover_text(&entry.title);
            #[cfg(feature = "ui-preview")]
            self.preview_sidebar
                .insert("title", (title.rect, ui.clip_rect()));
            #[cfg(not(feature = "ui-preview"))]
            let _ = title;
        }
        let back = ui.add_sized(
            [ui.available_width(), 30.0],
            egui::Button::new("← 返回工具库"),
        );
        #[cfg(feature = "ui-preview")]
        self.preview_sidebar
            .insert("back", (back.rect, ui.clip_rect()));
        if back.clicked() {
            self.page = Page::Library;
        }
        if let Some(entry) = active {
            let category = ui.add_sized(
                [ui.available_width(), 30.0],
                egui::Button::new("浏览同类工具 →"),
            );
            #[cfg(feature = "ui-preview")]
            self.preview_sidebar
                .insert("category", (category.rect, ui.clip_rect()));
            if category.clicked() {
                self.open_library("全部", &entry.category);
            }
        }
    }

    pub(super) fn start_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(RichText::new("从手头的任务开始").size(30.0));
        ui.label(
            RichText::new("打开常用工具，或把文件拖进来选择处理方式。").color(self.colors.muted),
        );
        ui.add_space(20.0);
        egui::Frame::new()
            .fill(self.colors.card)
            .corner_radius(12)
            .inner_margin(18)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.strong("想做什么？");
                ui.label("试试搜索“压缩图片”“合并表格”“时间戳转换”。");
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_sized([180.0, 36.0], egui::Button::new("搜索工具  ·  Ctrl K"))
                        .clicked()
                    {
                        self.open_launcher();
                    }
                    if ui
                        .add_sized([140.0, 36.0], egui::Button::new("从文件开始…"))
                        .clicked()
                    {
                        self.navigate(Page::Intake, None);
                    }
                    if ui
                        .add_sized([140.0, 36.0], egui::Button::new("浏览全部工具 →"))
                        .clicked()
                    {
                        self.open_library("全部", "全部分类");
                    }
                });
            });
        ui.add_space(22.0);
        ui.horizontal(|ui| {
            ui.strong("继续数据工作");
            if ui.small_button("已保存实例…").clicked() {
                self.page = Page::Data;
                self.data_state.open_library();
            }
        });
        let work: Vec<_> = self
            .data_state
            .instances
            .iter()
            .filter(|i| i.state.has_content())
            .take(4)
            .map(|i| (i.id.clone(), i.name.clone(), i.state.busy()))
            .collect();
        if work.is_empty() {
            ui.small("临时工作保留在本次运行中；已保存的快照可在重启后恢复。");
        }
        for (id, name, busy) in work {
            if ui
                .button(format!("{name}{} →", if busy { " · 运行中" } else { "" }))
                .clicked()
            {
                let _ = self.data_state.select(&id);
                self.page = Page::Data;
            }
        }
        ui.add_space(16.0);
        let entries = self.entries("");
        for (title, filter, ids) in [
            ("我的常用", "收藏", self.preferences.favorites.clone()),
            ("最近使用", "最近", self.preferences.recent.clone()),
        ] {
            ui.horizontal(|ui| {
                ui.strong(title);
                if ui.small_button("查看全部 →").clicked() {
                    self.open_library(filter, "全部分类");
                }
            });
            let selected: Vec<_> = ids
                .iter()
                .filter_map(|id| entries.iter().find(|e| &e.id == id))
                .take(4)
                .cloned()
                .collect();
            if selected.is_empty() {
                ui.label(
                    RichText::new(if filter == "收藏" {
                        "点工具旁的 ☆，把常用工具留在这里。"
                    } else {
                        "打开过的工具会出现在这里。"
                    })
                    .color(self.colors.muted),
                );
            } else {
                for entry in selected {
                    self.directory_row(ui, &entry, "");
                }
            }
            ui.add_space(16.0);
        }
        ui.separator();
        ui.add_space(8.0);
        ui.strong("按用途浏览");
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            for category in crate::plugins::CATEGORIES {
                if ui.button(*category).clicked() {
                    self.open_library("全部", category);
                }
            }
        });
        ui.add_space(12.0);
        ui.label(
            RichText::new(format!(
                "{} 个内置入口 · {} 个已启用插件工具",
                catalog().len(),
                self.plugins.store.tool_refs().count()
            ))
            .small()
            .color(self.colors.muted),
        );
    }

    pub(super) fn library_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(RichText::new("工具库").size(28.0));
        ui.label(
            RichText::new("按名称或用途查找；打开工具后再选择操作。").color(self.colors.muted),
        );
        ui.add_space(12.0);
        ui.add_sized(
            [ui.available_width(), 36.0],
            egui::TextEdit::singleline(&mut self.library_query)
                .hint_text("搜索名称或用途，例如 压缩图片、合并表格、JSON…")
                .char_limit(160),
        );
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            for filter in ["全部", "收藏", "最近", "常用"] {
                ui.selectable_value(&mut self.home_filter, filter.into(), filter);
            }
            ui.separator();
            egui::ComboBox::from_id_salt("tool-category")
                .selected_text(&self.home_category)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.home_category, "全部分类".into(), "全部分类");
                    for category in crate::plugins::CATEGORIES {
                        ui.selectable_value(&mut self.home_category, (*category).into(), *category);
                    }
                });
            if ui.small_button("重置筛选").clicked() {
                self.open_library("全部", "全部分类");
            }
        });
        let entries = self.library_entries();
        ui.add_space(10.0);
        ui.small(format!("{} 个匹配工具", entries.len()));
        ui.add_space(6.0);
        let key = (
            self.library_query.clone(),
            self.home_filter.clone(),
            self.home_category.clone(),
        );
        let changed = self.home_query_key != key;
        self.home_query_key = key;
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt("tool-library-list")
            .auto_shrink([false, false]);
        if changed {
            scroll = scroll.vertical_scroll_offset(0.0);
        }
        if entries.is_empty() {
            ui.add_space(24.0);
            ui.strong("没有找到匹配的工具");
            ui.label("换一个用途词，或清空分类与收藏筛选。停用的插件可在“扩展与连接器”中启用。");
            return;
        }
        let query = self.library_query.clone();
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            scroll.show_rows(ui, 64.0, entries.len(), |ui, range| {
                for entry in &entries[range] {
                    self.directory_row(ui, entry, &query);
                }
            });
        });
    }

    fn directory_row(&mut self, ui: &mut egui::Ui, entry: &ToolEntry, query: &str) {
        ui.push_id(&entry.id, |ui| {
            let (rect, _) = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), 64.0), egui::Sense::hover());
            let mut row = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::top_down(egui::Align::LEFT)),
            );
            row.set_clip_rect(rect.intersect(ui.clip_rect()));
            row.spacing_mut().item_spacing.y = 4.0;
            row.spacing_mut().button_padding = egui::vec2(8.0, 4.0);
            row.spacing_mut().interact_size.y = 28.0;
            let _rendered = egui::Frame::new()
                .fill(self.colors.card)
                .corner_radius(8)
                .inner_margin(8)
                .show(&mut row, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let favorite = self.preferences.favorites.contains(&entry.id);
                        if ui
                            .add_sized(
                                [28.0, 28.0],
                                egui::Button::new(if favorite { "★" } else { "☆" }),
                            )
                            .on_hover_text(if favorite {
                                "取消收藏"
                            } else {
                                "收藏工具"
                            })
                            .clicked()
                        {
                            self.toggle_favorite(&entry.id);
                        }
                        let wide = ui.available_width() >= 640.0;
                        let reserve = if wide { 290.0 } else { 240.0 };
                        let title_width = (ui.available_width() - reserve).max(100.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(title_width, 28.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                if ui
                                    .add(
                                        egui::Button::new(RichText::new(&entry.title).strong())
                                            .frame(false)
                                            .truncate(),
                                    )
                                    .on_hover_text(&entry.title)
                                    .clicked()
                                {
                                    self.open_entry(entry);
                                }
                            },
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("打开 →").clicked() {
                                self.open_entry(entry);
                            }
                            ui.label(RichText::new(entry.badge()).small().color(
                                if entry.in_progress {
                                    self.colors.amber
                                } else {
                                    self.colors.muted
                                },
                            ));
                            if wide {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&entry.category)
                                            .small()
                                            .color(self.colors.muted),
                                    )
                                    .truncate(),
                                );
                            }
                        });
                    });
                    let detail = if query.trim().is_empty() {
                        entry.description.clone()
                    } else {
                        format!("{}  ·  {}", entry.match_hint(query), entry.description)
                    };
                    ui.allocate_ui_with_layout(
                        egui::vec2(
                            ui.available_width(),
                            ui.text_style_height(&egui::TextStyle::Small),
                        ),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.add_space(37.0);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(detail).small().color(self.colors.muted),
                                )
                                .truncate(),
                            )
                            .on_hover_text(&entry.description);
                        },
                    );
                });
            #[cfg(feature = "ui-preview")]
            assert!(
                _rendered.response.rect.bottom() <= rect.bottom() + 0.5,
                "directory row overflows fixed viewport: {} ({} > {})",
                entry.id,
                _rendered.response.rect.height(),
                rect.height()
            );
        });
    }
}

impl DevToolsApp {
    pub(super) fn library_entries(&self) -> Vec<ToolEntry> {
        let mut entries = self.entries(&self.library_query);
        entries.retain(|e| {
            (self.home_category == "全部分类" || e.category == self.home_category)
                && match self.home_filter.as_str() {
                    "收藏" => self.preferences.favorites.contains(&e.id),
                    "最近" => self.preferences.recent.contains(&e.id),
                    "常用" => self.preferences.usage.contains_key(&e.id),
                    _ => true,
                }
        });
        // A view narrows the search, but must not override relevance while typing.
        let browsing = self.library_query.trim().is_empty();
        if browsing && self.home_filter == "收藏" {
            entries.sort_by_key(|e| {
                self.preferences
                    .favorites
                    .iter()
                    .position(|id| id == &e.id)
                    .unwrap_or(usize::MAX)
            });
        }
        if browsing && self.home_filter == "最近" {
            entries.sort_by_key(|e| {
                self.preferences
                    .recent
                    .iter()
                    .position(|id| id == &e.id)
                    .unwrap_or(usize::MAX)
            });
        }
        if browsing && self.home_filter == "常用" {
            entries.sort_by_key(|e| {
                (
                    std::cmp::Reverse(self.preferences.usage.get(&e.id).copied().unwrap_or(0)),
                    self.preferences
                        .recent
                        .iter()
                        .position(|id| id == &e.id)
                        .unwrap_or(usize::MAX),
                )
            });
        }
        entries
    }
}
