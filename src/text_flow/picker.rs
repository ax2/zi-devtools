//! In-memory discovery of registered actions; never runs or reads files.
use super::{ACTIONS, Action, Kind};
use eframe::egui;
use unicode_normalization::UnicodeNormalization;
const PAGE: usize = 16;

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
enum Category {
    #[default]
    All,
    Json,
    Encoding,
    Text,
    Digest,
    Table,
}
impl Category {
    fn label(self) -> &'static str {
        match self {
            Self::All => "全部分类",
            Self::Json => "JSON 格式",
            Self::Encoding => "编码与转义",
            Self::Text => "文本处理",
            Self::Digest => "摘要校验",
            Self::Table => "表格处理",
        }
    }
    fn of(action: &Action) -> Self {
        match action.id.split('.').next().unwrap_or("") {
            "json" => Self::Json,
            "base64" | "url" | "html" | "hex" => Self::Encoding,
            "sha256" => Self::Digest,
            "table" => Self::Table,
            _ => Self::Text,
        }
    }
}
fn aliases(action: &Action) -> &'static str {
    match action.id {
        "json.pretty" => "美化 缩进 beautify pretty format",
        "json.minify" => "紧凑 压缩 compact minify",
        "base64.encode" => "加码 encode",
        "base64.decode" => "解码 decode",
        "url.encode" => "网址 百分号 percent encode",
        "url.decode" => "网址 百分号 percent decode",
        "html.escape" => "网页 实体 entity escape",
        "html.unescape" => "网页 实体 entity unescape decode",
        "text.escape" => "字符串 转义 escape",
        "text.unescape" => "字符串 还原 unescape",
        "hex.encode" => "十六进制 encode",
        "hex.decode" => "十六进制 decode",
        "sha256.digest" => "哈希 散列 摘要 校验 hash checksum sha256",
        "table.parse_csv" => "导入 解析 csv import parse",
        "table.parse_tsv" => "导入 解析 tsv tab import parse",
        "table.parse_json" => "导入 解析 json import parse",
        "table.parse_schema" => "导入 解析 列结构 空表 headers rows schema import parse",
        "table.trim" => "去空白 去首尾空格 清洗 trim whitespace",
        "table.export_csv" => "导出 csv export",
        "table.export_tsv" => "导出 tsv tab export",
        "table.export_json" => "导出 json export",
        "table.export_schema" => "导出 列结构 空表 headers rows schema export",
        _ => "",
    }
}
fn normalize(text: &str) -> String {
    text.nfkc().flat_map(char::to_lowercase).collect()
}
#[derive(Default)]
pub(super) struct Picker {
    query: String,
    category: Category,
    all_types: bool,
    selected: usize,
    focus: bool,
}
impl Picker {
    pub(super) fn focus(&mut self) {
        self.focus = true;
    }
    pub(super) fn added(&mut self) {
        self.query.clear();
        self.selected = 0;
        self.focus();
    }
    fn matches(&self, next: Option<Kind>) -> Vec<&'static Action> {
        let query = normalize(self.query.trim());
        let terms: Vec<_> = query.split_whitespace().collect();
        let mut found: Vec<_> = ACTIONS
            .iter()
            .enumerate()
            .filter_map(|(index, action)| {
                if self.category != Category::All && self.category != Category::of(action)
                    || !self.all_types && next != Some(action.input)
                {
                    return None;
                }
                let haystack = normalize(&format!(
                    "{} {} {} {} {} {}",
                    action.id,
                    action.label,
                    aliases(action),
                    action.note,
                    action.input.label(),
                    action.output.label()
                ));
                if !terms.iter().all(|term| haystack.contains(term)) {
                    return None;
                }
                let primary = normalize(&format!(
                    "{} {} {}",
                    action.id,
                    action.label,
                    aliases(action)
                ));
                let rank = if query == action.id {
                    0
                } else if normalize(action.label) == query {
                    1
                } else if terms.iter().all(|term| primary.contains(term)) {
                    2
                } else {
                    3
                };
                Some((next != Some(action.input), rank, index, action))
            })
            .collect();
        found.sort_by_key(|(incompatible, rank, index, _)| (*incompatible, *rank, *index));
        found.into_iter().map(|(_, _, _, action)| action).collect()
    }
    pub(super) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        next: Option<Kind>,
        can_add: bool,
    ) -> Option<&'static Action> {
        let mut changed = false;
        let reveal = self.focus;
        let picker_top = ui.cursor().min;
        let search_id = ui.make_persistent_id("flow-picker-search-input");
        let focused = ui.is_enabled() && ui.memory(|memory| memory.has_focus(search_id));
        let down = focused
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
        let up = focused
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
        let keyboard_add = focused
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        ui.horizontal_wrapped(|ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .id(search_id)
                    .hint_text("搜索操作，例如 CSV 解析、去空白、列结构 导出")
                    .char_limit(256)
                    .desired_width(360.0),
            );
            #[cfg(feature = "ui-preview")]
            ui.ctx().data_mut(|data| {
                data.insert_temp(
                    egui::Id::new("flow-picker-search"),
                    (response.rect, ui.clip_rect()),
                )
            });
            changed |= response.changed();
            if self.focus {
                if !response.has_focus() {
                    response.request_focus();
                }
                self.focus = false;
            }
            egui::ComboBox::from_id_salt("flow-action-category")
                .selected_text(self.category.label())
                .show_ui(ui, |ui| {
                    for category in [
                        Category::All,
                        Category::Json,
                        Category::Encoding,
                        Category::Text,
                        Category::Digest,
                        Category::Table,
                    ] {
                        changed |= ui
                            .selectable_value(&mut self.category, category, category.label())
                            .changed();
                    }
                });
            changed |= ui
                .checkbox(&mut self.all_types, "包含其他输入类型")
                .on_hover_text("查看全部操作；不匹配的操作仍不可添加")
                .changed();
            if ui.button("清除搜索").clicked() {
                self.query.clear();
                self.focus();
                changed = true;
            }
        });
        if changed {
            self.selected = 0;
        }
        let matches = self.matches(next);
        self.selected = self.selected.min(matches.len().saturating_sub(1));
        if down {
            self.selected = (self.selected + 1).min(matches.len().saturating_sub(1));
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
        }
        let moved = down || up;
        ui.small(format!(
            "下一步输入：{} · 找到 {} / {} 项 · ↑↓选择，Enter 添加；Ctrl Enter 运行流程",
            next.map_or("先修正步骤顺序", Kind::label),
            matches.len(),
            ACTIONS.len()
        ));
        if !can_add {
            ui.small("已达到16步；移除步骤后才能继续添加。");
        }
        if matches.is_empty() {
            ui.label("没有匹配操作。可清除搜索、切换分类，或查看其他输入类型。");
        }
        let mut chosen = None;
        let pages = matches.len().div_ceil(PAGE).max(1);
        if pages > 1 {
            ui.horizontal(|ui| {
                let page = self.selected / PAGE;
                if ui
                    .add_enabled(page > 0, egui::Button::new("上一页"))
                    .clicked()
                {
                    self.selected = (page - 1) * PAGE;
                }
                ui.small(format!(
                    "{} / {} 页 · 每页最多{}项",
                    self.selected / PAGE + 1,
                    pages,
                    PAGE
                ));
                if ui
                    .add_enabled(page + 1 < pages, egui::Button::new("下一页"))
                    .clicked()
                {
                    self.selected = (page + 1) * PAGE;
                }
            });
        }
        let start = self.selected / PAGE * PAGE;
        let viewport = egui::ScrollArea::vertical()
            .id_salt("text-flow-actions")
            // Reserve useful browsing space inside the outer workbench scroll area.
            .min_scrolled_height(180.0)
            .max_height(180.0)
            .show(ui, |ui| {
                for (index, action) in matches.iter().enumerate().skip(start).take(PAGE) {
                    let compatible = next == Some(action.input);
                    let row = ui.group(|ui| {
                        ui.set_min_width((ui.available_width() - 16.0).max(0.0));
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .selectable_label(self.selected == index, action.label)
                                .clicked()
                            {
                                self.selected = index;
                            }
                            ui.weak(format!(
                                "{} → {} · v{}",
                                action.input.label(),
                                action.output.label(),
                                action.version
                            ));
                            if ui
                                .add_enabled(can_add && compatible, egui::Button::new("添加"))
                                .clicked()
                            {
                                chosen = Some(*action);
                            }
                        });
                        ui.small(action.note);
                        if !compatible {
                            ui.small(format!(
                                "需要{}；当前下一步为{}。先添加解析或导出操作转换材料。",
                                action.input.label(),
                                next.map_or("无效步骤顺序", Kind::label)
                            ));
                        }
                    });
                    if moved && self.selected == index {
                        row.response.scroll_to_me(Some(egui::Align::Center));
                    }
                }
            });
        if reveal {
            // Reveal search and results together in the outer workbench viewport.
            ui.scroll_to_rect(
                egui::Rect::from_min_max(picker_top, viewport.inner_rect.max),
                Some(egui::Align::Max),
            );
        }
        #[cfg(any(test, feature = "ui-preview"))]
        ui.ctx().data_mut(|data| {
            data.insert_temp(egui::Id::new("flow-picker-viewport"), viewport.inner_rect)
        });
        #[cfg(not(any(test, feature = "ui-preview")))]
        let _ = viewport;
        if keyboard_add
            && can_add
            && let Some(action) = matches.get(self.selected)
            && next == Some(action.input)
        {
            chosen = Some(*action);
        }
        chosen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(
        picker: &mut Picker,
        ctx: &egui::Context,
        key: Option<(egui::Key, egui::Modifiers)>,
        kind: Kind,
        can_add: bool,
    ) -> Option<&'static Action> {
        let mut input = egui::RawInput::default();
        if let Some((key, modifiers)) = key {
            input.modifiers = modifiers;
            input.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            });
        }
        let mut chosen = None;
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                chosen = picker.ui(ui, Some(kind), can_add);
            });
        });
        chosen
    }
    #[test]
    fn nested_picker_keeps_browsing_height_and_outer_scroll_reaches_controls() {
        let ctx = egui::Context::default();
        let mut picker = Picker::default();
        let mut outer = None;
        let mut bottom = 0.0;
        let mut footer = egui::Rect::NOTHING;
        for frame in 0..4 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(640.0, 480.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.allocate_space(egui::vec2(0.0, 150.0)); // Pinned instance header.
                    let result = egui::ScrollArea::vertical()
                        .id_salt("picker-layout-test")
                        .auto_shrink([false, false])
                        .vertical_scroll_offset(if frame >= 2 { bottom } else { 0.0 })
                        .show(ui, |ui| {
                            ui.allocate_space(egui::vec2(0.0, 160.0)); // Input editor.
                            picker.ui(ui, Some(Kind::Text), true);
                            footer = ui.button("Run workflow").rect;
                        });
                    bottom = (result.content_size.y - result.inner_rect.height()).max(0.0);
                    outer = Some(result.inner_rect);
                });
            });
            let viewport = ctx
                .data(|data| data.get_temp::<egui::Rect>(egui::Id::new("flow-picker-viewport")))
                .unwrap();
            assert!(viewport.height() >= 179.0, "{viewport:?}");
        }
        assert!(
            outer.unwrap().contains_rect(footer),
            "outer scroll must reach run controls: outer={:?}, footer={footer:?}",
            outer.unwrap()
        );
    }
    #[test]
    fn focused_navigation_adds_only_matching_type_and_does_not_consume_control_enter() {
        let ctx = egui::Context::default();
        let mut picker = Picker {
            query: "json".into(),
            ..Default::default()
        };
        picker.focus();
        frame(&mut picker, &ctx, None, Kind::Text, true);
        frame(&mut picker, &ctx, None, Kind::Text, true);
        frame(
            &mut picker,
            &ctx,
            Some((egui::Key::ArrowDown, egui::Modifiers::NONE)),
            Kind::Text,
            true,
        );
        assert_eq!(picker.selected, 1);
        let selected = frame(
            &mut picker,
            &ctx,
            Some((egui::Key::Enter, egui::Modifiers::NONE)),
            Kind::Text,
            true,
        )
        .unwrap();
        assert_eq!(selected.id, "json.minify");
        assert!(
            frame(
                &mut picker,
                &ctx,
                Some((egui::Key::Enter, egui::Modifiers::CTRL)),
                Kind::Text,
                true
            )
            .is_none()
        );
        assert!(
            frame(
                &mut picker,
                &ctx,
                Some((egui::Key::Enter, egui::Modifiers::NONE)),
                Kind::Text,
                false
            )
            .is_none()
        );
        picker.query = "去空白".into();
        picker.all_types = true;
        picker.selected = 0;
        assert!(
            frame(
                &mut picker,
                &ctx,
                Some((egui::Key::Enter, egui::Modifiers::NONE)),
                Kind::Text,
                true
            )
            .is_none()
        );
    }
    #[test]
    fn navigation_crosses_bounded_pages_without_losing_access_to_registry() {
        let ctx = egui::Context::default();
        let mut picker = Picker {
            all_types: true,
            ..Default::default()
        };
        picker.focus();
        frame(&mut picker, &ctx, None, Kind::Text, true);
        frame(&mut picker, &ctx, None, Kind::Text, true);
        for _ in 0..PAGE {
            frame(
                &mut picker,
                &ctx,
                Some((egui::Key::ArrowDown, egui::Modifiers::NONE)),
                Kind::Text,
                true,
            );
        }
        assert_eq!(picker.selected, PAGE);
        assert!(picker.matches(Some(Kind::Text)).len() > PAGE);
        picker.added();
        assert_eq!(picker.selected, 0);
    }
    #[test]
    fn aliases_multiple_terms_and_fullwidth_search_respect_next_type() {
        let mut picker = Picker {
            query: "ＣＳＶ 解析".into(),
            ..Default::default()
        };
        assert_eq!(
            picker
                .matches(Some(Kind::Text))
                .iter()
                .map(|a| a.id)
                .collect::<Vec<_>>(),
            vec!["table.parse_csv"]
        );
        assert!(picker.matches(Some(Kind::Table)).is_empty());
        picker.query = "列结构 导出".into();
        assert_eq!(
            picker.matches(Some(Kind::Table))[0].id,
            "table.export_schema"
        );
        picker.query = "哈希".into();
        assert_eq!(picker.matches(Some(Kind::Text))[0].id, "sha256.digest");
    }
    #[test]
    fn categories_cover_registry_and_other_types_are_explicitly_visible() {
        assert!(ACTIONS.iter().all(|action| !aliases(action).is_empty()));
        let mut picker = Picker {
            category: Category::Table,
            ..Default::default()
        };
        let compatible = picker.matches(Some(Kind::Table));
        assert!(compatible.iter().all(|a| a.input == Kind::Table));
        assert!(!compatible.is_empty());
        picker.all_types = true;
        let all = picker.matches(Some(Kind::Table));
        assert!(all.len() > compatible.len());
        assert!(
            all[..compatible.len()]
                .iter()
                .all(|a| a.input == Kind::Table)
        );
        assert!(all.iter().all(|a| a.id.starts_with("table.")));
        assert!(Picker::default().matches(None).is_empty());
    }
}
