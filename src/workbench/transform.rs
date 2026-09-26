use super::*;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
enum Operation {
    #[default]
    Trim,
    Lower,
    Upper,
    EmptyToNull,
    FillNull,
    Rename,
}
impl Operation {
    const ALL: [Self; 6] = [
        Self::Trim,
        Self::Lower,
        Self::Upper,
        Self::EmptyToNull,
        Self::FillNull,
        Self::Rename,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Trim => "去除首尾空白",
            Self::Lower => "转为小写",
            Self::Upper => "转为大写",
            Self::EmptyToNull => "空字符串 → null",
            Self::FillNull => "填充 null（文本）",
            Self::Rename => "修改列名",
        }
    }
}

struct Proposal {
    data: Dataset,
    changed: usize,
    examples: Vec<(String, String)>,
    description: String,
}

#[derive(Default)]
pub(super) struct State {
    column: usize,
    operation: Operation,
    value: String,
    proposal: Option<Proposal>,
    undo: Option<Dataset>,
}

fn propose(data: &Dataset, column: usize, operation: Operation, value: &str) -> Result<Proposal> {
    let header = data
        .headers
        .get(column)
        .ok_or_else(|| anyhow!("请选择有效列"))?;
    if value.len() > 4096 {
        bail!("替换文本最多 4096 字节");
    }
    let mut result = data.clone();
    let mut changed = 0;
    let mut examples = Vec::new();
    if operation == Operation::Rename {
        let name = value.trim();
        if name.is_empty() || name.len() > 256 {
            bail!("列名须为 1–256 字节，不能只有空白");
        }
        if data
            .headers
            .iter()
            .enumerate()
            .any(|(i, h)| i != column && h == name)
        {
            bail!("列名已存在，不能覆盖其他列");
        }
        if header != name {
            result.headers[column] = name.into();
            changed = 1;
            examples.push((header.clone(), name.into()));
        }
    } else {
        let mut replacement_bytes = 0usize;
        for row in &mut result.rows {
            let cell = &row[column];
            let next = match (operation, cell) {
                (Operation::Trim, Value::String(s)) => Value::String(s.trim().into()),
                (Operation::Lower, Value::String(s)) => Value::String(s.to_lowercase()),
                (Operation::Upper, Value::String(s)) => Value::String(s.to_uppercase()),
                (Operation::EmptyToNull, Value::String(s)) if s.is_empty() => Value::Null,
                (Operation::FillNull, Value::Null) => Value::String(value.into()),
                _ => continue,
            };
            replacement_bytes += next.to_string().len();
            if replacement_bytes > 8 * 1024 * 1024 {
                bail!("转换结果超过 8 MiB，请缩小数据范围或替换文本");
            }
            if *cell != next {
                changed += 1;
                if examples.len() < 6 {
                    examples.push((preview(cell), preview(&next)));
                }
                row[column] = next;
            }
        }
    }
    // Bound the complete result, including untouched columns, without serializing a huge buffer.
    let mut bytes = result.headers.iter().map(String::len).sum::<usize>();
    for row in &result.rows {
        for cell in row {
            bytes += cell.to_string().len();
            if bytes > 8 * 1024 * 1024 {
                bail!("转换结果超过 8 MiB；原数据未修改");
            }
        }
    }
    Ok(Proposal {
        data: result,
        changed,
        examples,
        description: format!("{} · {}", header, operation.label()),
    })
}

fn preview(value: &Value) -> String {
    let text = value.to_string();
    let mut out: String = text.chars().take(100).collect();
    if text.chars().count() > 100 {
        out.push('…');
    }
    out
}

impl DataState {
    #[cfg(feature = "ui-preview")]
    pub fn preview_transform(&mut self) {
        self.format = DataFormat::Json;
        self.input = r#"[{"name":"  Zi Tools  ","stars":120},{"name":" Local Notes ","stars":64},{"name":null,"stars":3}]"#.into();
        self.dataset = Some(Dataset::parse(&self.input, self.format, b',').unwrap());
        self.transform = State::default();
        self.transform.proposal =
            Some(propose(self.dataset.as_ref().unwrap(), 0, Operation::Trim, "").unwrap());
        self.refresh_transformed_view();
    }

    fn refresh_transformed_view(&mut self) {
        self.output.clear();
        if let Some(data) = &self.dataset {
            self.visible = data.view(&self.query, self.sort, self.descending);
        }
    }
    fn apply_transform(&mut self) {
        if let Some(proposal) = self.transform.proposal.take() {
            self.transform.undo = self.dataset.replace(proposal.data);
            self.message = format!(
                "已应用 {}，改变 {} 处；可撤销最近一次转换",
                proposal.description, proposal.changed
            );
            self.refresh_transformed_view();
        }
    }
    fn undo_transform(&mut self) {
        if let Some(previous) = self.transform.undo.take() {
            self.dataset = Some(previous);
            self.transform.proposal = None;
            self.message = "已撤销最近一次转换；原始输入始终保留".into();
            self.refresh_transformed_view();
        }
    }
    pub(super) fn transform_ui(&mut self, ui: &mut egui::Ui) {
        let Some(data) = &self.dataset else {
            return;
        };
        ui.add_space(12.0);
        let mut apply = false;
        let mut undo = false;
        egui::CollapsingHeader::new("列转换 · 预览后应用")
            .id_salt("data-transform")
            .default_open(self.transform.proposal.is_some()).show(ui, |ui| {
                ui.small("作用于全部行，包含被筛选隐藏的行。文本操作保留数字、布尔及对象类型；只保留一步撤销。重新解析原始输入可重置全部转换。");
                let mut changed = false;
                ui.horizontal_wrapped(|ui| {
                    egui::ComboBox::from_id_salt("transform-column")
                        .selected_text(&data.headers[self.transform.column])
                        .show_ui(ui, |ui| {
                            for (i, name) in data.headers.iter().enumerate() {
                                changed |= ui.selectable_value(&mut self.transform.column, i, name).changed();
                            }
                        });
                    egui::ComboBox::from_id_salt("transform-operation")
                        .selected_text(self.transform.operation.label())
                        .show_ui(ui, |ui| {
                            for operation in Operation::ALL {
                                changed |= ui.selectable_value(&mut self.transform.operation, operation, operation.label()).changed();
                            }
                        });
                    if matches!(self.transform.operation, Operation::FillNull | Operation::Rename) {
                        changed |= ui.add(egui::TextEdit::singleline(&mut self.transform.value).hint_text("新列名 / 填充值").desired_width(180.0)).changed();
                    }
                });
                if changed { self.transform.proposal = None; }
                ui.small("空字符串是 \"\"，null 是缺失值。纯空白需先去空白，再转 null；填充 null 不会替换空字符串。");
                ui.horizontal(|ui| {
                    if ui.button("预览转换").clicked() {
                        match propose(data, self.transform.column, self.transform.operation, &self.transform.value) {
                            Ok(proposal) => { self.transform.proposal = Some(proposal); self.message.clear(); }
                            Err(error) => { self.transform.proposal = None; self.message = error.to_string(); }
                        }
                    }
                    undo = ui.add_enabled(self.transform.undo.is_some(), egui::Button::new("撤销最近一次")).clicked();
                });
                if let Some(proposal) = &self.transform.proposal {
                    ui.label(format!("{} · 将改变 {} 处", proposal.description, proposal.changed));
                    egui::ScrollArea::horizontal().id_salt("transform-preview-scroll").show(ui, |ui| {
                        egui::Grid::new("transform-preview-grid").striped(true).show(ui, |ui| {
                            ui.strong("之前"); ui.strong("之后"); ui.end_row();
                            for (before, after) in &proposal.examples { ui.monospace(before); ui.monospace(after); ui.end_row(); }
                        });
                    });
                    ui.small("最多展示 6 处变化；内容超过 100 字符时仅缩略显示。");
                    apply = ui.add_enabled(proposal.changed > 0, primary(ui, "应用到数据表")).clicked();
                }
            });
        if undo {
            self.undo_transform();
        } else if apply {
            self.apply_transform();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn data() -> Dataset {
        Dataset::parse(
            r#"[{"a":"  ß  ","b":2},{"a":"","b":true},{"a":null},{"a":42},{"a":{"x":1}}]"#,
            DataFormat::Json,
            b',',
        )
        .unwrap()
    }
    #[test]
    fn unicode_and_null_operations_preserve_types_and_require_explicit_steps() {
        let original = data();
        let trimmed = propose(&original, 0, Operation::Trim, "").unwrap();
        assert_eq!(trimmed.changed, 1);
        assert_eq!(original.rows[0][0], "  ß  ");
        let upper = propose(&trimmed.data, 0, Operation::Upper, "").unwrap();
        assert_eq!(upper.data.rows[0][0], "SS");
        assert_eq!(upper.data.rows[3][0], 42);
        assert_eq!(upper.data.rows[4][0], serde_json::json!({"x":1}));
        let filled = propose(&original, 0, Operation::FillNull, "missing").unwrap();
        assert_eq!(filled.changed, 1);
        assert_eq!(filled.data.rows[1][0], "");
        let nulls = propose(&original, 0, Operation::EmptyToNull, "").unwrap();
        assert_eq!(nulls.changed, 1);
        assert!(nulls.data.rows[1][0].is_null());
    }
    #[test]
    fn rename_and_size_errors_do_not_mutate_source() {
        let original = data();
        assert!(propose(&original, 0, Operation::Rename, "b").is_err());
        assert!(propose(&original, 0, Operation::Rename, " ").is_err());
        assert!(propose(&original, 99, Operation::Trim, "").is_err());
        let renamed = propose(&original, 0, Operation::Rename, "title").unwrap();
        assert_eq!(renamed.data.headers[0], "title");
        assert_eq!(original.headers[0], "a");
        let large = Dataset {
            headers: vec!["x".into()],
            rows: vec![vec![Value::Null]; 3000],
        };
        assert!(propose(&large, 0, Operation::FillNull, &"x".repeat(4096)).is_err());
        assert!(large.rows[0][0].is_null());
    }
    #[test]
    fn apply_and_undo_refresh_filter_and_invalidate_export_preserving_input() {
        let mut state = DataState {
            dataset: Some(data()),
            input: "original draft".into(),
            query: "missing".into(),
            output: "old export".into(),
            ..Default::default()
        };
        state.transform.proposal = Some(
            propose(
                state.dataset.as_ref().unwrap(),
                0,
                Operation::FillNull,
                "missing",
            )
            .unwrap(),
        );
        state.apply_transform();
        assert_eq!(state.visible, vec![2]);
        assert!(state.output.is_empty());
        assert_eq!(state.input, "original draft");
        state.undo_transform();
        assert!(state.visible.is_empty());
        assert_eq!(state.dataset.unwrap(), data());
        assert!(state.transform.undo.is_none());
    }
}
