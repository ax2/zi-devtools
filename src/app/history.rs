//! Session-only back/forward navigation retains exact workbench identities.
use super::*;
#[cfg(feature = "ui-preview")]
mod extended_preview;
mod model;
#[cfg(feature = "ui-preview")]
mod preview;
use model::History;
#[derive(Clone)]
struct Library {
    query: String,
    filter: String,
    category: String,
    scroll: f32,
}
#[derive(Clone)]
struct Location {
    page: Page,
    tool: Option<String>,
    instance: Option<String>,
    title: String,
    library: Option<Library>,
}
impl Location {
    fn same_place(&self, other: &Self) -> bool {
        self.page == other.page && self.tool == other.tool && self.instance == other.instance
    }
}
#[derive(Default)]
pub(super) struct State {
    history: History,
    keyboard: launcher::keyboard::Guard,
    allowed: bool,
    pub(super) route_hint: Option<(Page, String)>,
}
impl DevToolsApp {
    pub(super) fn current_entry(&self) -> Option<ToolEntry> {
        if self.page == Page::Plugins {
            return self
                .plugins
                .selected
                .as_deref()
                .and_then(|id| self.navigation_entry(id));
        }
        if !matches!(
            self.page,
            Page::SmallTools
                | Page::EncodingTools
                | Page::Images
                | Page::Data
                | Page::Recorder
                | Page::Java
                | Page::Django
        ) && let Some((page, id)) = &self.navigation_history.route_hint
            && *page == self.page
        {
            return self.navigation_entry(id);
        }
        catalog()
            .iter()
            .find(|entry| {
                entry.page == self.page
                    && match self.page {
                        Page::Plugins => {
                            self.plugins.selected.as_deref() == Some(entry.id.as_str())
                        }
                        Page::Images => entry.id == self.images.active_tool_id(),
                        Page::Data => entry.id == self.data_state.active_tool_id(),
                        Page::Recorder => entry.id == self.recorder.active_tool_id(),
                        Page::Java | Page::Django => entry.id == self.frameworks.selected.id(),
                        _ => entry
                            .kind
                            .is_none_or(|kind| kind == self.tool_state.selected),
                    }
            })
            .cloned()
    }
    fn navigation_entry(&self, id: &str) -> Option<ToolEntry> {
        catalog()
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
            .or_else(|| {
                self.plugins
                    .store
                    .tool_refs()
                    .find(|(key, _)| key == id)
                    .map(|(id, tool)| ToolEntry::plugin(id, tool))
            })
    }
    fn navigation_location(&self) -> Location {
        let entry = self.current_entry();
        let mut title = entry.as_ref().map(|e| e.title.clone()).unwrap_or_else(|| {
            match self.page {
                Page::Home => "开始",
                Page::Library => "工具库",
                Page::Settings => "设置",
                Page::Plugins => "扩展与连接器",
                _ => "工作台",
            }
            .into()
        });
        let instance = if self.page == Page::Data {
            let active = &self
                .data_state
                .instances
                .iter()
                .find(|i| i.id == self.data_state.active_id())
                .unwrap();
            title = format!("{} · {}", title, active.name);
            Some(active.id.clone())
        } else if self.page == Page::Images && self.images.active_tool_id() == "image-workflow" {
            let (id, name) = self.images.navigation_instance();
            title = format!("{title} · {name}");
            Some(id)
        } else {
            None
        };
        Location {
            page: self.page,
            tool: entry.map(|e| e.id),
            instance,
            title,
            library: (self.page == Page::Library).then(|| Library {
                query: self.library_query.clone(),
                filter: self.home_filter.clone(),
                category: self.home_category.clone(),
                scroll: self.library_scroll_offset,
            }),
        }
    }
    pub(super) fn observe_navigation(&mut self) {
        if self
            .navigation_history
            .route_hint
            .as_ref()
            .is_some_and(|(page, _)| *page != self.page)
        {
            self.navigation_history.route_hint = None;
        }

        let location = self.navigation_location();
        self.instance_search
            .observe(
                location
                    .instance
                    .as_ref()
                    .map(|id| crate::instance_navigation::Key {
                        kind: if location.page == Page::Data {
                            crate::instance_navigation::Kind::Data
                        } else {
                            crate::instance_navigation::Kind::Image
                        },
                        id: id.clone(),
                    }),
            );
        self.navigation_history.history.observe(location);
    }
    fn history_location_available(&self, location: &Location) -> bool {
        if let Some(id) = &location.tool
            && !self
                .navigation_entry(id)
                .is_some_and(|e| e.page == location.page)
        {
            return false;
        }
        match (location.page, location.instance.as_deref()) {
            (Page::Data, Some(id)) => self
                .data_state
                .instances
                .iter()
                .any(|instance| instance.id == id),
            (Page::Images, Some(id)) => self.images.navigation_has_instance(id),
            (Page::Data, None) => false,
            _ => true,
        }
    }
    fn history_blocked(&self, ctx: &egui::Context) -> bool {
        self.launcher_open || self.quick_open || self.navigation_action_blocked(ctx)
    }
    fn navigation_action_blocked(&self, ctx: &egui::Context) -> bool {
        self.handoff.is_some()
            || self.workspace_exit_confirm
            || self.clear_tool_confirm
            || self.data_state.modal_open()
            || self.data_state.operation_pending()
            || self.images.navigation_blocked()
            || self.recorder.navigation_blocked()
            || self.reminder_center_open
            || ctx.memory(|m| m.top_modal_layer().is_some())
            || egui::Popup::is_any_open(ctx)
    }
    pub(super) fn navigate_instance(
        &mut self,
        ctx: &egui::Context,
        key: &crate::instance_navigation::Key,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.navigation_action_blocked(ctx),
            "请先完成或取消当前审核、保存或恢复"
        );
        let target = self
            .instance_entries()
            .into_iter()
            .find(|entry| &entry.key == key)
            .ok_or_else(|| anyhow::anyhow!("实例已关闭，请重新选择"))?;
        let location = Location {
            page: match key.kind {
                crate::instance_navigation::Kind::Data => Page::Data,
                crate::instance_navigation::Kind::Image => Page::Images,
            },
            tool: Some(target.tool),
            instance: Some(key.id.clone()),
            title: target.name,
            library: None,
        };
        self.observe_navigation();
        self.restore_navigation(&location)?;
        self.observe_navigation();
        Ok(())
    }
    fn restore_navigation(&mut self, location: &Location) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.history_location_available(location),
            "目标工具或实例已不可用"
        );
        let entry = location
            .tool
            .as_ref()
            .and_then(|id| self.navigation_entry(id));
        if location.page == Page::Data {
            let id = location.instance.as_deref().unwrap();
            let target = self
                .data_state
                .instances
                .iter()
                .find(|i| i.id == id)
                .unwrap();
            anyhow::ensure!(
                !target.state.dialog_pending(),
                "请先完成或取消目标实例的审核"
            );
            self.data_state.select(id)?;
            if let Some(entry) = &entry {
                self.data_state.set_active_tool(&entry.id);
            }
        } else if location.page == Page::Images {
            if let Some(id) = &location.instance {
                self.images.navigation_select_instance(id)?;
            }
            if let Some(entry) = &entry {
                self.images.select_entry(&entry.id);
            }
        } else if location.page == Page::Plugins {
            if let Some(entry) = &entry {
                self.plugins.select(&entry.id);
            } else {
                self.plugins.selected = None;
            }
        } else if let Some(entry) = &entry {
            if let Some(kind) = entry.kind {
                self.tool_state.select(kind);
            }
            if let Some(tool) = crate::framework::Tool::from_id(&entry.id) {
                self.frameworks.select(tool);
            }
            if entry.page == Page::Recorder {
                self.recorder.select_entry(&entry.id);
                if entry.id == "recorder-tutorial" {
                    self.recorder.show_tutorial();
                }
            }
        }
        if matches!(location.page, Page::Notes | Page::Calendar) {
            self.planner.calendar = location.page == Page::Calendar;
        }
        if let Some(library) = &location.library {
            self.library_query = library.query.clone();
            self.home_filter = library.filter.clone();
            self.home_category = library.category.clone();
            self.home_query_key = (
                library.query.clone(),
                library.filter.clone(),
                library.category.clone(),
            );
            self.library_scroll_offset = library.scroll;
            self.library_scroll_restore = Some(library.scroll);
        }
        self.page = location.page;
        self.navigation_history.route_hint =
            location.tool.as_ref().map(|id| (location.page, id.clone()));
        self.tool_search.clear();
        self.launcher_open = false;
        Ok(())
    }
    pub(super) fn navigate_history(&mut self, ctx: &egui::Context, backwards: bool) {
        if self.history_blocked(ctx) {
            return;
        }
        self.observe_navigation();
        let Some((index, location, skipped)) = self
            .navigation_history
            .history
            .candidate(backwards, |l| self.history_location_available(l))
        else {
            return;
        };
        match self.restore_navigation(&location) {
            Ok(()) => {
                self.navigation_history.history.commit(index);
                if skipped > 0 {
                    self.toast = Some((
                        format!("已跳过{skipped}个已关闭或不可用的位置"),
                        Instant::now(),
                    ));
                }
            }
            Err(error) => self.toast = Some((format!("无法返回：{error:#}"), Instant::now())),
        }
    }
    pub(super) fn history_keyboard(&mut self, ctx: &egui::Context) {
        self.navigation_history.allowed = self.navigation_history.keyboard.allow_shortcuts(ctx);
        if !self.navigation_history.allowed || self.history_blocked(ctx) {
            return;
        }
        let action = ctx.input_mut(|input| {
            let mut action = None;
            input.events.retain(|event| {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    repeat: false,
                    ..
                } = event
                    && *modifiers == egui::Modifiers::ALT
                    && matches!(key, egui::Key::ArrowLeft | egui::Key::ArrowRight)
                {
                    action = Some(*key == egui::Key::ArrowLeft);
                    return false;
                }
                true
            });
            action
        });
        if let Some(backwards) = action {
            self.navigate_history(ctx, backwards);
        }
    }
    pub(super) fn history_controls(&mut self, ui: &mut egui::Ui) {
        let enabled = self.navigation_history.allowed && !self.history_blocked(ui.ctx());
        ui.horizontal(|ui| {
            for (backwards, label, key) in [
                (true, "← 后退", "history-back"),
                (false, "前进 →", "history-forward"),
            ] {
                let candidate = self
                    .navigation_history
                    .history
                    .candidate(backwards, |l| self.history_location_available(l));
                let hint = candidate
                    .as_ref()
                    .map(|(_, l, _)| {
                        format!("{} · Alt+{}", l.title, if backwards { "←" } else { "→" })
                    })
                    .unwrap_or_else(|| "没有可用的历史位置".into());
                let response = ui
                    .add_enabled(
                        enabled && candidate.is_some(),
                        egui::Button::new(label).min_size([88.0, 28.0].into()),
                    )
                    .on_hover_text(hint);
                #[cfg(feature = "ui-preview")]
                self.preview_sidebar
                    .insert(key, (response.rect, ui.clip_rect()));
                #[cfg(not(feature = "ui-preview"))]
                let _ = key;
                if response.clicked() {
                    self.navigate_history(ui.ctx(), backwards);
                }
            }
        });
        ui.add_space(4.0);
    }
}
