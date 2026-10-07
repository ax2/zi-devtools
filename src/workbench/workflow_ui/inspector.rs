use super::*;
const PAGE: usize = 50;

#[derive(Default)]
pub(super) struct State {
    pub(super) open: bool,
    source: bool,
    query: String,
    visible: Vec<usize>,
    page: usize,
    selected: Option<(usize, usize)>,
}
impl State {
    #[cfg(feature = "ui-preview")]
    pub(super) fn selection(&self) -> Option<(usize, usize)> {
        self.selected
    }
    pub(super) fn open(&mut self, result: &Dataset) {
        *self = Self {
            open: true,
            visible: (0..result.rows.len()).collect(),
            ..Default::default()
        };
    }
    fn filter(&mut self, data: &Dataset) {
        self.visible = data.view(&self.query, None, false);
        self.page = 0;
        self.selected = None;
    }
    fn range(&self) -> std::ops::Range<usize> {
        let start = self.page.saturating_mul(PAGE).min(self.visible.len());
        start..(start + PAGE).min(self.visible.len())
    }
    fn pages(&self) -> usize {
        self.visible.len().div_ceil(PAGE).max(1)
    }
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null（缺失值）",
        Value::Bool(_) => "布尔",
        Value::Number(_) => "数字",
        Value::String(_) => "文本",
        Value::Array(_) => "数组",
        Value::Object(_) => "对象",
    }
}

// Serialize a bounded prefix instead of allocating an entire long JSON cell every frame.
fn fragment(value: &Value, limit: usize) -> String {
    struct Prefix {
        bytes: Vec<u8>,
        limit: usize,
        truncated: bool,
    }
    impl std::io::Write for Prefix {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let room = self.limit.saturating_sub(self.bytes.len());
            self.bytes
                .extend_from_slice(&bytes[..bytes.len().min(room)]);
            if bytes.len() > room {
                self.truncated = true;
                return Err(std::io::Error::other("bounded JSON preview"));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut prefix = Prefix {
        bytes: Vec::new(),
        limit,
        truncated: false,
    };
    let _ = serde_json::to_writer(&mut prefix, value);
    let valid =
        std::str::from_utf8(&prefix.bytes).map_or_else(|e| e.valid_up_to(), |_| prefix.bytes.len());
    let mut result = std::str::from_utf8(&prefix.bytes[..valid])
        .expect("valid UTF8 prefix")
        .to_owned();
    if prefix.truncated {
        result.push('…');
    }
    result
}

impl DataState {
    pub(super) fn workflow_inspector(&mut self, ui: &mut egui::Ui) {
        if !self.workflow.inspect.open {
            return;
        }
        let (Some(preview), Some(source)) = (&self.workflow.proposal, &self.workflow.source) else {
            self.workflow.inspect = State::default();
            return;
        };
        let state = &mut self.workflow.inspect;
        #[cfg(feature = "ui-preview")]
        let buttons = &mut self.workflow.buttons;
        let mut close = false;
        let response = egui::Modal::new(ui.id().with("workflow-inspector")).show(ui.ctx(), |ui| {
            let width = (ui.ctx().screen_rect().width() - 64.0).clamp(240.0, 820.0);
            ui.set_width(width);
            ui.heading("检查流程结果");
            ui.small("只读快照 · 查看筛选和分页不改变流程范围；关闭后仍需明确应用。");
            ui.horizontal_wrapped(|ui| {
                let before = state.source;
                let input = ui.selectable_value(&mut state.source, true, format!("输入表 · {}行", source.rows.len()));
                let output = ui.selectable_value(&mut state.source, false, format!("流程结果 · {}行", preview.result.rows.len()));
                #[cfg(feature = "ui-preview")]
                { buttons[7] = Some((input.rect, ui.clip_rect())); buttons[8] = Some((output.rect, ui.clip_rect())); }
                #[cfg(not(feature = "ui-preview"))]
                let _ = (input, output);
                if before != state.source { state.query.clear(); state.filter(if state.source { source } else { &preview.result }); }
            });
            let data = if state.source { source } else { &preview.result };
            if ui.add(egui::TextEdit::singleline(&mut state.query).char_limit(4096).hint_text("仅筛选查看内容，匹配所有列…").desired_width(width)).changed() { state.filter(data); }
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("{}列 · 匹配{} / {}行 · 第{} / {}页", data.headers.len(), state.visible.len(), data.rows.len(), state.page + 1, state.pages()));
                if ui.add_enabled(state.page > 0, egui::Button::new("上一页")).clicked() { state.page -= 1; state.selected = None; }
                if ui.add_enabled(state.page + 1 < state.pages(), egui::Button::new("下一页")).clicked() { state.page += 1; state.selected = None; }
            });
            if state.visible.is_empty() { ui.label(if data.rows.is_empty() { "此快照没有数据行；仍保留列结构。" } else { "没有匹配行，请调整查看筛选。" }); }
            egui::ScrollArea::both().id_salt("inspect-grid-scroll").auto_shrink([false, false]).max_height(200.0).min_scrolled_height(140.0).show(ui, |ui| {
                egui::Grid::new("inspect-grid").striped(true).min_col_width(150.0).max_col_width(170.0).show(ui, |ui| {
                    ui.strong("快照行号");
                    for name in &data.headers { ui.add(egui::Label::new(RichText::new(name).strong()).truncate()).on_hover_text(name); }
                    ui.end_row();
                    for position in state.range() {
                        let row = state.visible[position];
                        ui.weak((row + 1).to_string());
                        for (column, value) in data.rows[row].iter().enumerate() {
                            let selected = state.selected == Some((row, column));
                            let cell = ui.add_sized([160.0, 24.0], egui::Button::new(RichText::new(fragment(value, 240)).monospace()).selected(selected).truncate());
                            #[cfg(feature = "ui-preview")]
                            if position == state.range().start && column == 0 { buttons[9] = Some((cell.rect, ui.clip_rect())); }
                            if cell.on_hover_text(kind(value)).clicked() { state.selected = Some((row, column)); }
                        }
                        ui.end_row();
                    }
                });
            });
            ui.small("每页最多50行，左右滚动查看其他列；单元格按JSON显示，长值截断。点击单元格查看类型或复制完整值。");
            if let Some((row, column)) = state.selected {
                let value = &data.rows[row][column];
                ui.horizontal_wrapped(|ui| {
                    ui.strong(format!("第{}行 · {} · {}", row + 1, data.headers[column], kind(value)));
                    if ui.button("复制完整JSON值").clicked() { ui.ctx().copy_text(value.to_string()); }
                });
                egui::ScrollArea::vertical().id_salt("inspect-cell").max_height(65.0).show(ui, |ui| { ui.monospace(fragment(value, 4096)); });
            } else { ui.small("尚未选择单元格。查看不会修改输入或结果。"); }
            ui.separator();
            let button = ui.button("关闭检查，返回流程");
            #[cfg(feature = "ui-preview")]
            { buttons[6] = Some((button.rect, ui.clip_rect())); }
            close = button.clicked();
        });
        if close || response.should_close() {
            state.open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn inspection_filters_pages_and_keeps_both_snapshots_unchanged() {
        let result = Dataset {
            headers: vec!["id".into()],
            rows: (0..123).map(|i| vec![json!(format!("row{i}"))]).collect(),
        };
        let original = result.clone();
        let mut state = State::default();
        state.open(&result);
        assert_eq!(state.pages(), 3);
        state.page = 2;
        assert_eq!(state.range(), 100..123);
        state.selected = Some((100, 0));
        state.query = "row12".into();
        state.filter(&result);
        assert_eq!(state.visible, [12, 120, 121, 122]);
        assert_eq!(state.page, 0);
        assert_eq!(state.selected, None);
        state.query = "missing".into();
        state.filter(&result);
        assert_eq!(state.range(), 0..0);
        assert_eq!(state.pages(), 1);
        assert_eq!(result, original);
    }
    #[test]
    fn bounded_json_cells_preserve_types_utf8_and_full_copy_representation() {
        for (value, text) in [
            (json!(null), "null"),
            (json!(""), "\"\""),
            (json!("001"), "\"001\""),
            (json!(1), "1"),
        ] {
            assert_eq!(fragment(&value, 100), text);
        }
        let value = json!("中文😀\n".repeat(10000));
        let preview = fragment(&value, 240);
        assert!(preview.ends_with('…'));
        assert!(preview.len() <= 243);
        assert!(!preview.contains('\u{fffd}'));
        assert!(value.to_string().len() > 240);
        let empty = Dataset {
            headers: vec!["x".into()],
            rows: vec![],
        };
        let mut state = State::default();
        state.open(&empty);
        assert_eq!(state.range(), 0..0);
        assert_eq!(state.pages(), 1);
    }
}
