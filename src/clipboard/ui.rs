use super::History;
use eframe::egui::{self, RichText};

#[derive(Default)]
pub struct State {
    history: History,
    search: String,
    selected: Vec<u64>,
    format: usize,
    message: String,
    output: String,
    clear_confirm: bool,
    paused: bool,
    #[cfg(windows)]
    listener: Option<super::native::Listener>,
}
impl State {
    pub fn poll(&mut self) {
        #[cfg(windows)]
        if let Some(listener) = &self.listener {
            for event in listener.rx.try_iter().take(64) {
                match event {
                    super::native::Event::Text(text, source) if !self.paused => {
                        if let Err(e) = self.history.insert(text, source) {
                            self.message = e;
                        }
                    }
                    super::native::Event::Error(e) => self.message = e,
                    _ => {}
                }
            }
            if listener.lost.swap(0, std::sync::atomic::Ordering::AcqRel) > 0 {
                self.message = "复制事件过快，部分条目未采集".into();
            }
        }
        let selected_count = self.selected.len();
        self.selected
            .retain(|id| self.history.entries.iter().any(|e| e.id == *id));
        if self.selected.len() != selected_count {
            self.output.clear();
        }
    }
    fn copy(&mut self, ui: &egui::Ui, text: String) {
        #[cfg(windows)]
        if let Some(listener) = &self.listener {
            self.message = match listener.copy(&text) {
                Ok(()) => "已复制；不会重复采集本工具输出".into(),
                Err(e) => e,
            };
            return;
        }
        ui.ctx().copy_text(text);
        self.message = "已交给系统剪贴板".into();
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("超级剪贴板");
        ui.label("v0.1.0 · 开发中：文本历史 / 搜索与置顶 / 按选择顺序组合复制");
        ui.label("主动开启后采集新复制的文本，只保存在本次运行内存中；暂停不补采，彻底退出后清空。图片、富文本、文件引用及本机历史保存尚待开发。");
        #[cfg(windows)]
        ui.horizontal_wrapped(|ui| {
            if self.listener.is_none() {
                if ui.button("开启文本历史采集").clicked() {
                    match super::native::Listener::start(ui.ctx().clone()) {
                        Ok(listener) => {
                            self.listener = Some(listener);
                            self.paused = false;
                            self.message = "已开启；仅采集后续复制的文本".into();
                        }
                        Err(e) => self.message = e,
                    }
                }
            } else {
                if ui
                    .button(if self.paused {
                        "恢复采集"
                    } else {
                        "私密暂停"
                    })
                    .clicked()
                {
                    self.paused = !self.paused;
                    if let Some(listener) = &self.listener {
                        listener.set_paused(self.paused);
                    }
                    self.message = if self.paused {
                        "已暂停；现有历史仍可使用"
                    } else {
                        "已恢复；不补采暂停期间内容"
                    }
                    .into();
                }
                if ui.button("关闭采集").clicked() {
                    self.listener = None;
                    self.message = "已关闭，历史仅保留到退出；可主动清空".into();
                }
            }
            ui.label(format!(
                "{} 条 · {:.2} MiB / 32 MiB",
                self.history.entries.len(),
                self.history.bytes() as f32 / (1024.0 * 1024.0)
            ));
            if ui.button("清空历史…").clicked() {
                self.clear_confirm = true;
            }
        });
        #[cfg(not(windows))]
        ui.label("剪贴板采集当前仅支持Windows。");
        if self.clear_confirm {
            ui.horizontal_wrapped(|ui| {
                ui.label("删除全部会话历史（含置顶）？");
                if ui.button("确认清空").clicked() {
                    self.history.entries.clear();
                    self.selected.clear();
                    self.clear_confirm = false;
                }
                if ui.button("取消").clicked() {
                    self.clear_confirm = false;
                }
            });
        }
        ui.small("当前最多500条、单条1 MiB。来源可能未知；应用排除尚未接入，复制敏感内容前请暂停。默认不采集、不保存到磁盘、不联网。");
        ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("搜索文本或来源应用"));
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        ui.separator();
        let query = self.search.to_lowercase();
        let mut copy = None;
        let mut delete = None;
        egui::ScrollArea::vertical()
            .id_salt("clipboard-history-list")
            .max_height(340.0)
            .show(ui, |ui| {
                if self.history.entries.is_empty() {
                    ui.label("开启采集后，在任意应用复制文本；也可先试用合成示例。");
                }
                let mut indices: Vec<usize> = (0..self.history.entries.len()).collect();
                indices.sort_by_key(|i| !self.history.entries[*i].pinned);
                for i in indices {
                    let entry = &mut self.history.entries[i];
                    if !query.is_empty()
                        && !entry.text.to_lowercase().contains(&query)
                        && !entry.source.to_lowercase().contains(&query)
                    {
                        continue;
                    }
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            let mut selected = self.selected.contains(&entry.id);
                            if ui.checkbox(&mut selected, "选择").changed() {
                                if selected {
                                    self.selected.push(entry.id);
                                } else {
                                    self.selected.retain(|id| *id != entry.id);
                                }
                            }
                            ui.checkbox(&mut entry.pinned, "置顶保留");
                            ui.small(format!("{} · {}", entry.source, entry.time));
                            if ui.button("复制").clicked() {
                                copy = Some(entry.text.clone());
                            }
                            if ui.button("删除").clicked() {
                                delete = Some(entry.id);
                            }
                        });
                        let preview: String = entry.text.chars().take(180).collect();
                        ui.label(preview);
                        if entry.text.chars().count() > 180 {
                            ui.small("长文本已截断；下方组合预览可查看完整结果");
                        }
                    });
                }
            });
        if let Some(id) = delete {
            self.history.entries.retain(|e| e.id != id);
            self.selected.retain(|x| *x != id);
        }
        if let Some(text) = copy {
            self.copy(ui, text);
        }
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("组合预览 · {}项", self.selected.len())).strong());
            egui::ComboBox::from_id_salt("clipboard-format")
                .selected_text(["段落", "编号", "JSON数组", "CSV单列"][self.format])
                .show_ui(ui, |ui| {
                    for (i, label) in ["段落", "编号", "JSON数组", "CSV单列"].iter().enumerate()
                    {
                        ui.selectable_value(&mut self.format, i, *label);
                    }
                });
            if ui.button("取消全部选择").clicked() {
                self.selected.clear();
            }
        });
        match self.history.combined(&self.selected, self.format) {
            Ok(mut text) => {
                self.output.clone_from(&text);
                ui.small("顺序按勾选先后；取消后重选可改变次序。CSV对可能的公式加前置单引号；历史原文不变。可通过上方接力入口发送备忘/数据等工具。");
                if ui.button("复制组合结果").clicked() {
                    self.copy(ui, text.clone());
                }
                ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .desired_rows(5)
                        .desired_width(f32::INFINITY)
                        .interactive(false),
                );
            }
            Err(e) => {
                self.output.clear();
                ui.small(e);
            }
        }
        if ui.button("载入合成示例（不读取系统剪贴板）").clicked() {
            self.preview_fixture();
        }
    }
    pub fn transfer_text(&self) -> Option<(String, &str)> {
        if self.output.is_empty() {
            None
        } else {
            Some((
                format!("超级剪贴板0.1.0组合 · 条目{:?}", self.selected),
                &self.output,
            ))
        }
    }
    pub fn preview_fixture(&mut self) {
        self.selected.clear();
        self.output.clear();
        for (text, source) in [
            ("教程第一步：准备示例数据\n仅使用合成内容", "示例编辑器"),
            (
                "{\"project\":\"Zi DevTools\",\"status\":\"demo\"}",
                "示例终端",
            ),
            (
                "第二步：选择JSON数组组合，多片段可以保持原来的复制顺序。",
                "示例浏览器",
            ),
        ] {
            if let Err(e) = self.history.insert(text.into(), source.into()) {
                self.message = e;
                return;
            }
            self.selected.push(self.history.entries[0].id);
        }
        if let Some(entry) = self
            .history
            .entries
            .iter_mut()
            .find(|e| e.source == "示例编辑器")
        {
            entry.pinned = true;
        }
        self.format = 2;
        self.output = self
            .history
            .combined(&self.selected, self.format)
            .unwrap_or_default();
        self.message = "合成示例不读取系统剪贴板；采集状态见上方".into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn demo_rejects_full_pinned_history_without_panicking_or_replacing_originals() {
        let mut state = State::default();
        for i in 0..super::super::ITEM_LIMIT {
            state
                .history
                .insert(format!("fixture-{i}"), "fixture".into())
                .unwrap();
            state.history.entries[0].pinned = true;
        }
        state.output = "stale".into();
        state.preview_fixture();
        assert_eq!(state.history.entries.len(), super::super::ITEM_LIMIT);
        assert!(
            state
                .history
                .entries
                .iter()
                .all(|e| e.text.starts_with("fixture-"))
        );
        assert!(state.transfer_text().is_none());
        assert!(!state.message.is_empty());
    }
    #[test]
    fn evicted_selection_invalidates_handoff_without_replacing_other_inputs() {
        let mut state = State::default();
        state.preview_fixture();
        assert!(state.transfer_text().is_some());
        let remove = state.selected[0];
        state.history.entries.retain(|e| e.id != remove);
        state.poll();
        assert!(state.transfer_text().is_none());
        assert!(!state.selected.contains(&remove));
    }
}
