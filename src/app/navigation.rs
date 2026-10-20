use super::*;
use std::io::Read;

#[derive(Clone, Debug)]
struct CollectionDragPayload {
    collection_id: String,
    tool_id: String,
}

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
                let (_, scroll_clip) = self.preview_sidebar.get("scroll-clip").unwrap();
                let (dock, _) = self.preview_sidebar.get("frequent").unwrap();
                assert!(
                    scroll_clip.bottom() <= dock.top() - 6.0,
                    "navigation must have a visible gap above dock: {scroll_clip:?}, {dock:?}"
                );
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
                RichText::new(format!(
                    "Stage {}  ·  v{}",
                    env!("CARGO_PKG_VERSION_MINOR"),
                    env!("CARGO_PKG_VERSION")
                ))
                .size(11.0)
                .color(p.muted),
            );
        });
    }
}

impl DevToolsApp {
    pub(super) fn open_library(&mut self, filter: &str, category: &str) {
        self.observe_navigation();
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
                    .default_height(246.0)
                    .show_inside(ui, |ui| {
                        self.history_controls(ui);
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
                        self.current_tool_navigation(ui);
                        self.navigation_footer(ui, ctx);
                    });
                // The dock reduces layout space, but the parent paint clip still
                // includes it. Bound the navigation paint to the remaining area
                // so partially visible rows cannot draw over fixed actions.
                ui.set_clip_rect(ui.clip_rect().intersect(ui.available_rect_before_wrap()));
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
                let navigation_height = (ui.available_height() - 12.0).max(0.0);
                egui::ScrollArea::vertical()
                    .id_salt("sidebar-scroll")
                    .auto_shrink([false, false])
                    .max_height(navigation_height)
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .show(ui, |ui| {
                        #[cfg(feature = "ui-preview")]
                        self.preview_sidebar
                            .insert("scroll-clip", (ui.max_rect(), ui.clip_rect()));
                        for (page, label) in [
                            (Page::Home, "开始"),
                            (Page::Library, "工具库"),
                            (Page::Services, "本地服务"),
                            (Page::Tasks, "后台任务"),
                            (Page::Plugins, "扩展与连接器"),
                        ] {
                            let button = ui.add_sized(
                                [ui.available_width(), 38.0],
                                egui::Button::new(label).selected(self.page == page),
                            );
                            #[cfg(feature = "ui-preview")]
                            if page == Page::Services {
                                self.preview_services
                                    .insert("sidebar", (button.rect, ui.clip_rect()));
                            }
                            if button.clicked() {
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
        let active_entry = self.current_entry();
        let active = active_entry.as_ref();
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
                    self.directory_row(ui, &entry, "", None);
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
            RichText::new("按名称、用途、当前/历史版本或变更说明查找；打开工具后再选择操作。")
                .color(self.colors.muted),
        );
        ui.add_space(12.0);
        ui.add_sized(
            [ui.available_width(), 36.0],
            egui::TextEdit::singleline(&mut self.library_query)
                .hint_text("搜索名称、用途、当前/历史版本或变更说明，例如 0.3.0、sqlformat…")
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
            let selected_collection = self.home_filter.strip_prefix("集合:").and_then(|id| {
                self.preferences
                    .tool_collections
                    .iter()
                    .find(|item| item.id == id)
            });
            egui::ComboBox::from_id_salt("tool-collection-filter")
                .selected_text(selected_collection.map_or("工具集合", |item| item.name.as_str()))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.home_filter, "全部".into(), "全部集合");
                    for collection in &self.preferences.tool_collections {
                        ui.selectable_value(
                            &mut self.home_filter,
                            format!("集合:{}", collection.id),
                            &collection.name,
                        );
                    }
                });
            if ui.small_button("重置筛选").clicked() {
                self.open_library("全部", "全部分类");
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.add_sized(
                [ui.available_width().min(250.0), 28.0],
                egui::TextEdit::singleline(&mut self.collection_name_draft)
                    .hint_text("新建工具集合，例如：Java排查")
                    .char_limit(40),
            );
            if ui
                .add_enabled(
                    !self.collection_name_draft.trim().is_empty(),
                    egui::Button::new("＋ 新建集合"),
                )
                .clicked()
            {
                match self
                    .preferences
                    .create_tool_collection(&self.preferences_path, &self.collection_name_draft)
                {
                    Ok(id) => {
                        self.home_filter = format!("集合:{id}");
                        self.collection_name_draft.clear();
                    }
                    Err(error) => {
                        self.toast = Some((error.to_string(), std::time::Instant::now()));
                    }
                }
            }
            #[cfg(windows)]
            {
                if ui
                    .add_enabled(
                        !self.preferences.tool_collections.is_empty(),
                        egui::Button::new("导出集合…"),
                    )
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Zi DevTools 工具集合", &["json"])
                        .set_file_name("zi-tool-collections.json")
                        .save_file()
                {
                    let result = self
                        .preferences
                        .export_tool_collections()
                        .and_then(|json| std::fs::write(&path, json).map_err(Into::into));
                    self.toast = Some((
                        result.map_or_else(
                            |error: anyhow::Error| format!("导出失败：{error}"),
                            |_| format!("已导出到 {}", path.display()),
                        ),
                        std::time::Instant::now(),
                    ));
                }
                if ui.small_button("导入集合…").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Zi DevTools 工具集合", &["json"])
                        .pick_file()
                {
                    let load = (|| -> anyhow::Result<_> {
                        let file = std::fs::File::open(&path)?;
                        let mut reader = file
                            .take(crate::preferences::TOOL_COLLECTIONS_EXPORT_MAX_BYTES as u64 + 1);
                        let mut bytes = Vec::new();
                        reader.read_to_end(&mut bytes)?;
                        anyhow::ensure!(
                            bytes.len() <= crate::preferences::TOOL_COLLECTIONS_EXPORT_MAX_BYTES,
                            "导入文件超过1 MiB"
                        );
                        self.preferences.preview_tool_collections_import(
                            path.file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or("工具集合文件"),
                            &bytes,
                        )
                    })();
                    match load {
                        Ok(preview) => self.collection_import_preview = Some(preview),
                        Err(error) => {
                            self.toast = Some((
                                format!("无法预览导入文件：{error}"),
                                std::time::Instant::now(),
                            ));
                        }
                    }
                }
            }
            if let Some(collection) = self.home_filter.strip_prefix("集合:").and_then(|id| {
                self.preferences
                    .tool_collections
                    .iter()
                    .find(|item| item.id == id)
            }) {
                let selected_id = collection.id.clone();
                let selected_name = collection.name.clone();
                if self.collection_rename_target != selected_id {
                    self.collection_rename_target = selected_id.clone();
                    self.collection_rename_draft = selected_name.clone();
                }
                ui.add_sized(
                    [ui.available_width().min(220.0), 28.0],
                    egui::TextEdit::singleline(&mut self.collection_rename_draft)
                        .hint_text("重命名集合")
                        .char_limit(40),
                );
                if ui
                    .add_enabled(
                        self.collection_rename_draft.trim() != selected_name,
                        egui::Button::new("重命名"),
                    )
                    .clicked()
                {
                    match self.preferences.rename_tool_collection(
                        &self.preferences_path,
                        &selected_id,
                        &self.collection_rename_draft,
                    ) {
                        Ok(()) => {
                            self.collection_rename_draft =
                                self.collection_rename_draft.trim().to_owned()
                        }
                        Err(error) => {
                            self.toast = Some((error.to_string(), std::time::Instant::now()));
                        }
                    }
                }
                if ui.small_button("删除集合…").clicked() {
                    self.collection_delete_pending = Some(selected_id);
                }
            } else {
                ui.small("集合可保存一组任务常用入口；在工具行的“集合”菜单中增减工具。");
            }
        });
        if let Some(preview) = self.collection_import_preview.clone() {
            let mut open = true;
            let mut confirm = false;
            let mut cancel = false;
            egui::Window::new("导入工具集合预览")
                .open(&mut open)
                .collapsible(false)
                .resizable(true)
                .default_width(500.0)
                .show(ui.ctx(), |ui| {
                    let membership_count = preview
                        .items
                        .iter()
                        .map(|item| item.collection.tool_ids.len())
                        .sum::<usize>();
                    ui.label(format!(
                        "文件：{}。将新增 {} 个集合和 {} 个工具入口；现有集合不会被覆盖或删除。",
                        preview.source_name,
                        preview.items.len(),
                        membership_count
                    ));
                    ui.small("只导入集合名称与工具 ID，不包含工具输入、收藏或其他偏好。未知插件 ID 会保留。");
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical()
                        .max_height(260.0)
                        .show(ui, |ui| {
                            for item in &preview.items {
                                ui.horizontal(|ui| {
                                    if item.renamed_to_avoid_conflict {
                                        ui.label(format!(
                                            "{} → {}",
                                            item.source_name, item.collection.name
                                        ));
                                    } else {
                                        ui.label(&item.collection.name);
                                    }
                                    ui.label(
                                        RichText::new(format!(
                                            "{} 个工具",
                                            item.collection.tool_ids.len()
                                        ))
                                        .color(self.colors.muted),
                                    );
                                });
                            }
                        });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("确认导入").clicked() {
                            confirm = true;
                        }
                        if ui.button("取消").clicked() {
                            cancel = true;
                        }
                    });
                });
            if cancel {
                open = false;
            }
            if confirm {
                match self
                    .preferences
                    .import_tool_collections(&self.preferences_path, &preview)
                {
                    Ok(()) => {
                        if let Some(first) = preview.items.first() {
                            self.home_filter = format!("集合:{}", first.collection.id);
                        }
                        self.toast = Some((
                            format!("已导入 {} 个工具集合", preview.items.len()),
                            std::time::Instant::now(),
                        ));
                    }
                    Err(error) => {
                        self.toast =
                            Some((format!("导入未完成：{error}"), std::time::Instant::now()));
                    }
                }
                self.collection_import_preview = None;
            } else if !open {
                self.collection_import_preview = None;
            }
        }
        if let Some(id) = self.collection_delete_pending.clone() {
            let name = self
                .preferences
                .tool_collections
                .iter()
                .find(|collection| collection.id == id)
                .map_or("此集合", |collection| collection.name.as_str())
                .to_owned();
            egui::Window::new("删除工具集合")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ui.ctx(), |ui| {
                    ui.label(format!("删除“{name}”及其中的工具列表？工具本身不会删除。"));
                    ui.horizontal(|ui| {
                        if ui.button("删除集合").clicked() {
                            match self
                                .preferences
                                .delete_tool_collection(&self.preferences_path, &id)
                            {
                                Ok(()) => {
                                    self.collection_delete_pending = None;
                                    self.home_filter = "全部".into();
                                }
                                Err(error) => {
                                    self.toast =
                                        Some((error.to_string(), std::time::Instant::now()));
                                }
                            }
                        }
                        if ui.button("取消").clicked() {
                            self.collection_delete_pending = None;
                        }
                    });
                });
        }
        let entries = self.library_entries();
        ui.add_space(10.0);
        ui.small(format!("{} 个匹配工具", entries.len()));
        let reorder_collection_id = if self.library_query.trim().is_empty() {
            self.home_filter
                .strip_prefix("集合:")
                .filter(|id| {
                    self.preferences
                        .tool_collections
                        .iter()
                        .any(|collection| collection.id == *id)
                })
                .map(str::to_owned)
        } else {
            None
        };
        if reorder_collection_id.is_some() {
            ui.small("拖动工具行左侧的 ↕ 按任务顺序调整；搜索时按匹配度排序。");
        }
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
        if let Some(offset) = self.library_scroll_restore.take() {
            scroll = scroll.vertical_scroll_offset(offset);
        } else if changed {
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
            let output = scroll.show_rows(ui, 64.0, entries.len(), |ui, range| {
                for entry in &entries[range] {
                    self.directory_row(ui, entry, &query, reorder_collection_id.as_deref());
                }
            });
            self.library_scroll_offset = output.state.offset.y;
            #[cfg(feature = "ui-preview")]
            ui.ctx().data_mut(|data| {
                data.insert_temp(egui::Id::new("history-library-scroll"), output.inner_rect)
            });
        });
    }

    fn directory_row(
        &mut self,
        ui: &mut egui::Ui,
        entry: &ToolEntry,
        query: &str,
        reorder_collection: Option<&str>,
    ) {
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
                        if let Some(collection_id) = reorder_collection {
                            let source_id =
                                egui::Id::new(("tool-collection-order", collection_id, &entry.id));
                            let source = ui.dnd_drag_source(
                                source_id,
                                CollectionDragPayload {
                                    collection_id: collection_id.to_owned(),
                                    tool_id: entry.id.clone(),
                                },
                                |ui| {
                                    ui.add_sized([24.0, 28.0], egui::Button::new("↕").frame(false))
                                },
                            );
                            source.response.on_hover_text("拖动以调整集合顺序");
                        }
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
                        let reserve = if wide {
                            if reorder_collection.is_some() {
                                520.0
                            } else {
                                495.0
                            }
                        } else if reorder_collection.is_some() {
                            470.0
                        } else {
                            445.0
                        };
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
                            ui.menu_button("版本", |ui| {
                                ui.set_min_width(300.0);
                                let current = entry.version.as_deref().map_or_else(
                                    || "版本未声明".to_owned(),
                                    |version| {
                                        if entry.in_progress {
                                            format!("v{version} · 开发中")
                                        } else {
                                            format!("v{version}")
                                        }
                                    },
                                );
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new("当前版本").small().color(self.colors.muted),
                                    );
                                    ui.strong(current);
                                });
                                ui.separator();
                                if entry.version_history.is_empty() {
                                    ui.label("此工具尚未声明版本历史");
                                } else {
                                    egui::ScrollArea::vertical()
                                        .max_height(280.0)
                                        .show(ui, |ui| {
                                            for record in entry.version_history.iter().rev() {
                                                ui.horizontal_wrapped(|ui| {
                                                    ui.strong(format!("v{}", record.version));
                                                    ui.label(
                                                        RichText::new(&record.date)
                                                            .small()
                                                            .color(self.colors.muted),
                                                    );
                                                });
                                                ui.label(&record.changes);
                                                ui.add_space(6.0);
                                            }
                                        });
                                }
                            });
                            ui.menu_button("快捷方式", |ui| {
                                ui.label("工具入口保留当前配置位置");
                                ui.separator();
                                if ui.button("创建桌面快捷方式").clicked() {
                                    self.create_tool_shortcut(entry, false);
                                    ui.close();
                                }
                                if ui.button("创建用于任务栏的快捷方式…").clicked() {
                                    self.create_tool_shortcut(entry, true);
                                    ui.close();
                                }
                                if ui.button("复制启动参数").clicked() {
                                    match crate::tool_shortcuts::arguments(
                                        &entry.id,
                                        &self.manager.config_snapshot().path,
                                    ) {
                                        Ok(args) => ui.ctx().copy_text(args),
                                        Err(e) => {
                                            self.toast =
                                                Some((e.to_string(), std::time::Instant::now()))
                                        }
                                    }
                                    ui.close();
                                }
                            });
                            let collections = self.preferences.tool_collections.clone();
                            ui.menu_button("集合", |ui| {
                                if collections.is_empty() {
                                    ui.label("先创建一个工具集合");
                                }
                                for collection in collections {
                                    let mut included = collection.tool_ids.contains(&entry.id);
                                    if ui.checkbox(&mut included, &collection.name).changed() {
                                        match self.preferences.toggle_tool_in_collection(
                                            &self.preferences_path,
                                            &collection.id,
                                            &entry.id,
                                        ) {
                                            Ok(added) => {
                                                self.toast = Some((
                                                    if added {
                                                        format!("已加入“{}”", collection.name)
                                                    } else {
                                                        format!("已从“{}”移除", collection.name)
                                                    },
                                                    std::time::Instant::now(),
                                                ));
                                            }
                                            Err(error) => {
                                                self.toast = Some((
                                                    error.to_string(),
                                                    std::time::Instant::now(),
                                                ));
                                            }
                                        }
                                    }
                                }
                            });
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
                    let version = entry
                        .version
                        .as_deref()
                        .map_or_else(|| "版本未声明".to_owned(), |version| format!("v{version}"));
                    let description = format!("{version}  ·  {}", entry.description);
                    let detail = if query.trim().is_empty() {
                        description.clone()
                    } else {
                        format!("{}  ·  {description}", entry.match_detail(query))
                    };
                    let detail_hover = format!("{detail}\n{description}");
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
                                    RichText::new(&detail).small().color(self.colors.muted),
                                )
                                .truncate(),
                            )
                            .on_hover_text(detail_hover);
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
            let Some(collection_id) = reorder_collection else {
                return;
            };
            let drop_response = ui.interact(
                rect,
                egui::Id::new(("tool-collection-drop-row", collection_id, &entry.id)),
                egui::Sense::hover(),
            );
            let pointer = ui.ctx().input(|input| input.pointer.interact_pos());
            if let Some(payload) = drop_response.dnd_hover_payload::<CollectionDragPayload>()
                && payload.collection_id == collection_id
                && payload.tool_id != entry.id
                && let Some(pointer) = pointer
            {
                let y = if pointer.y < rect.center().y {
                    rect.top() + 1.0
                } else {
                    rect.bottom() - 1.0
                };
                ui.painter().line_segment(
                    [
                        egui::pos2(rect.left() + 6.0, y),
                        egui::pos2(rect.right() - 6.0, y),
                    ],
                    egui::Stroke::new(2.0, self.colors.accent),
                );
            }
            if let Some(payload) = drop_response.dnd_release_payload::<CollectionDragPayload>()
                && payload.collection_id == collection_id
                && payload.tool_id != entry.id
                && let Some(pointer) = pointer
            {
                let after = pointer.y >= rect.center().y;
                match self.preferences.move_tool_in_collection(
                    &self.preferences_path,
                    collection_id,
                    &payload.tool_id,
                    &entry.id,
                    after,
                ) {
                    Ok(()) => {
                        self.toast = Some(("集合顺序已保存".into(), std::time::Instant::now()))
                    }
                    Err(error) => self.toast = Some((error.to_string(), std::time::Instant::now())),
                }
            }
        });
    }

    fn create_tool_shortcut(&mut self, entry: &ToolEntry, taskbar: bool) {
        let result = (|| -> anyhow::Result<std::path::PathBuf> {
            let directory = if taskbar {
                dirs::data_local_dir()
                    .ok_or_else(|| anyhow::anyhow!("无法定位用户应用目录"))?
                    .join("ZiDevTools")
                    .join("Tool Shortcuts")
            } else {
                dirs::desktop_dir().ok_or_else(|| anyhow::anyhow!("无法定位桌面目录"))?
            };
            let path = crate::tool_shortcuts::create(
                &directory,
                &std::env::current_exe()?,
                &self.manager.config_snapshot().path,
                &entry.id,
                &entry.title,
            )?;
            if taskbar {
                open::that(&directory)?;
            }
            Ok(path)
        })();
        self.toast = Some((
            match result {
                Ok(path) if taskbar => format!(
                    "已创建 {}。请右键该快捷方式，在 Windows 菜单中选择“固定到任务栏”（可能位于“显示更多选项”）。",
                    path.display()
                ),
                Ok(path) => format!("桌面快捷方式已创建：{}", path.display()),
                Err(e) => format!("创建快捷方式失败：{e}"),
            },
            std::time::Instant::now(),
        ));
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
                    filter if filter.starts_with("集合:") => self
                        .preferences
                        .tool_collections
                        .iter()
                        .find(|collection| format!("集合:{}", collection.id) == filter)
                        .is_some_and(|collection| collection.tool_ids.contains(&e.id)),
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
        if browsing {
            if let Some(collection) = self.home_filter.strip_prefix("集合:").and_then(|id| {
                self.preferences
                    .tool_collections
                    .iter()
                    .find(|item| item.id == id)
            }) {
                super::launcher::filter_collection(&mut entries, collection, false);
            }
        }
        if browsing && self.home_filter == "常用" {
            super::launcher::sort_by_usage(
                &mut entries,
                &self.preferences.usage,
                &self.preferences.recent,
            );
        }
        entries
    }
}
