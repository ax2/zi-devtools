use super::*;
use crate::instance_navigation::{Entry, Key, Kind, Selection};
#[cfg(feature = "ui-preview")]
mod preview;

#[derive(Default)]
pub(super) struct State {
    recent: Vec<Key>,
    current: Option<Key>,
    pub main: Selection,
    pub quick: Selection,
}
impl State {
    pub fn observe(&mut self, key: Option<Key>) {
        if self.current == key {
            return;
        }
        if let Some(key) = &key {
            self.recent.retain(|old| old != key);
            self.recent.insert(0, key.clone());
            self.recent.truncate(128);
        }
        self.current = key;
    }
}
impl DevToolsApp {
    pub(super) fn instance_entries(&self) -> Vec<Entry> {
        let mut entries: Vec<_> = self
            .data_state
            .instances
            .iter()
            .map(|instance| Entry {
                key: Key {
                    kind: Kind::Data,
                    id: instance.id.clone(),
                },
                name: instance.name.clone(),
                tool: instance.state.active_tool_id().into(),
                busy: instance.state.busy(),
                pending: instance.state.dialog_pending(),
            })
            .collect();
        entries.extend(self.images.navigation_entries());
        entries
    }
    pub(super) fn instance_results(&self, query: &str) -> Vec<Entry> {
        crate::instance_navigation::search(
            self.instance_entries(),
            query,
            &self.instance_search.recent,
        )
    }
    pub(super) fn instance_row(
        &self,
        ui: &mut egui::Ui,
        entry: &Entry,
        selected: bool,
        scroll: bool,
    ) -> bool {
        let kind = match entry.key.kind {
            Kind::Data => "数据实例",
            Kind::Image => "图片实例",
        };
        let title = catalog()
            .iter()
            .find(|tool| tool.id == entry.tool)
            .map_or(entry.tool.as_str(), |tool| tool.title.as_str());
        let status = if entry.pending {
            "待确认"
        } else if entry.busy {
            "运行中"
        } else {
            "可继续编辑"
        };
        let short: String = entry.key.id.chars().take(8).collect();
        let id_hint = if self
            .instance_entries()
            .iter()
            .filter(|candidate| {
                candidate.name == entry.name && candidate.key.kind == entry.key.kind
            })
            .count()
            > 1
        {
            format!(" · 标识{short}")
        } else {
            String::new()
        };
        let current = if self.instance_search.current.as_ref() == Some(&entry.key) {
            " · 当前"
        } else {
            ""
        };
        let row = ui
            .push_id(entry.key.token(), |ui| {
                let response = ui.add(
                    egui::Button::selectable(selected, format!("{} · {kind}{current}", entry.name))
                        .truncate(),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("{title} · {status}{id_hint}"))
                            .small()
                            .color(self.colors.muted),
                    )
                    .truncate(),
                );
                response
            })
            .inner
            .on_hover_text(format!(
                "{}\n{title} · {status}\n切换到当前工作实例；保留输入、结果和后台任务。",
                entry.name
            ));
        #[cfg(feature = "ui-preview")]
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                egui::Id::new(entry.key.token()),
                row.rect.intersect(ui.clip_rect()),
            )
        });
        if selected && scroll {
            row.scroll_to_me(Some(egui::Align::Center));
        }
        ui.add_space(6.0);
        row.clicked()
    }
    pub(super) fn open_instance_result(&mut self, ctx: &egui::Context, key: &Key) -> bool {
        match self.navigate_instance(ctx, key) {
            Ok(()) => true,
            Err(error) => {
                self.toast = Some((format!("无法打开工作实例：{error:#}"), Instant::now()));
                false
            }
        }
    }
}
