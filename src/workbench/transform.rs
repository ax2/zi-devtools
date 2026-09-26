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
    ToText,
    ToInteger,
    ToNumber,
    ToBool,
    SelectColumns,
}
impl Operation {
    const ALL: [Self; 11] = [
        Self::Trim,
        Self::Lower,
        Self::Upper,
        Self::EmptyToNull,
        Self::FillNull,
        Self::Rename,
        Self::ToText,
        Self::ToInteger,
        Self::ToNumber,
        Self::ToBool,
        Self::SelectColumns,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Trim => "去除首尾空白",
            Self::Lower => "转为小写",
            Self::Upper => "转为大写",
            Self::EmptyToNull => "空字符串 → null",
            Self::FillNull => "填充 null（文本）",
            Self::Rename => "修改列名",
            Self::ToText => "转为文本",
            Self::ToInteger => "转为整数（64 位）",
            Self::ToNumber => "转为数字",
            Self::ToBool => "转为布尔值",
            Self::SelectColumns => "选择保留列",
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
    keep: Vec<bool>,
    force_open: bool,
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
        for (index, row) in result.rows.iter_mut().enumerate() {
            let cell = &row[column];
            let next = match (operation, cell) {
                (Operation::Trim, Value::String(s)) => Value::String(s.trim().into()),
                (Operation::Lower, Value::String(s)) => Value::String(s.to_lowercase()),
                (Operation::Upper, Value::String(s)) => Value::String(s.to_uppercase()),
                (Operation::EmptyToNull, Value::String(s)) if s.is_empty() => Value::Null,
                (Operation::FillNull, Value::Null) => Value::String(value.into()),
                (
                    Operation::ToText
                    | Operation::ToInteger
                    | Operation::ToNumber
                    | Operation::ToBool,
                    _,
                ) => convert(cell, operation).with_context(|| {
                    format!("第 {} 行，列 {} 转换失败；原数据未修改", index + 1, header)
                })?,
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

fn convert(cell: &Value, operation: Operation) -> Result<Value> {
    if cell.is_null() {
        return Ok(Value::Null);
    }
    if operation == Operation::ToText {
        return Ok(Value::String(
            cell.as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| cell.to_string()),
        ));
    }
    let text = match cell {
        Value::String(s) => s.trim().to_owned(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) if operation == Operation::ToBool => b.to_string(),
        _ => bail!("该类型无法转换"),
    };
    if operation == Operation::ToBool {
        return match text.to_ascii_lowercase().as_str() {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            _ => bail!("期望 true 或 false"),
        };
    }
    if operation == Operation::ToInteger || !text.contains(['.', 'e', 'E']) {
        return text
            .parse::<i64>()
            .map(Value::from)
            .or_else(|_| text.parse::<u64>().map(Value::from))
            .map_err(|_| anyhow!("期望 64 位范围内的整数，不接受小数或指数形式"));
    }
    let number: serde_json::Number = serde_json::from_str(&text)
        .map_err(|_| anyhow!("期望有限 JSON 数字，不接受 NaN 或 Infinity"))?;
    Ok(Value::Number(number))
}

fn select_columns(data: &Dataset, keep: &[bool]) -> Result<Proposal> {
    if keep.len() != data.headers.len() || !keep.iter().any(|v| *v) {
        bail!("至少保留一列");
    }
    let indices: Vec<_> = keep
        .iter()
        .enumerate()
        .filter_map(|(i, keep)| keep.then_some(i))
        .collect();
    Ok(Proposal {
        data: Dataset {
            headers: indices.iter().map(|&i| data.headers[i].clone()).collect(),
            rows: data
                .rows
                .iter()
                .map(|row| indices.iter().map(|&i| row[i].clone()).collect())
                .collect(),
        },
        changed: data.headers.len() - indices.len(),
        examples: data
            .headers
            .iter()
            .zip(keep)
            .filter(|(_, keep)| !**keep)
            .take(6)
            .map(|(h, _)| (h.clone(), "移除".into()))
            .collect(),
        description: format!(
            "保留 {} / {} 列，变化计数为移除列数",
            indices.len(),
            data.headers.len()
        ),
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

    #[cfg(feature = "ui-preview")]
    pub fn preview_schema(&mut self, projection: bool) {
        self.input = "id,count,enabled\n1,42,true\n2,9007199254740993,false".into();
        self.format = DataFormat::Csv;
        self.dataset = Some(Dataset::parse(&self.input, self.format, b',').unwrap());
        self.transform = State {
            column: 1,
            operation: if projection {
                Operation::SelectColumns
            } else {
                Operation::ToInteger
            },
            keep: vec![true, true, false],
            force_open: true,
            ..Default::default()
        };
        let data = self.dataset.as_ref().unwrap();
        self.transform.proposal = Some(
            if projection {
                select_columns(data, &self.transform.keep)
            } else {
                propose(data, 1, Operation::ToInteger, "")
            }
            .unwrap(),
        );
        self.refresh_transformed_view();
    }
    pub fn show_transform(&mut self) {
        self.transform.force_open = true;
    }
    fn refresh_transformed_view(&mut self) {
        self.output.clear();
        if let Some(data) = &self.dataset {
            self.visible = data.view(&self.query, self.sort, self.descending);
        }
    }
    fn apply_transform(&mut self) {
        if let Some(proposal) = self.transform.proposal.take() {
            if self
                .dataset
                .as_ref()
                .is_some_and(|d| d.headers != proposal.data.headers)
            {
                self.sort = None;
                self.transform.column = 0;
                self.transform.keep.clear();
            }
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
            if self
                .dataset
                .as_ref()
                .is_some_and(|d| d.headers != previous.headers)
            {
                self.sort = None;
                self.transform.column = 0;
                self.transform.keep.clear();
            }
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
        if self.transform.keep.len() != data.headers.len() {
            self.transform.keep = vec![true; data.headers.len()];
        }
        ui.add_space(12.0);
        let mut apply = false;
        let mut undo = false;
        egui::CollapsingHeader::new("列转换 · 预览后应用")
            .id_salt("data-transform")
            .open(self.transform.force_open.then_some(true))
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
                if self.transform.operation == Operation::SelectColumns {
                    egui::ScrollArea::vertical().id_salt("keep-columns").max_height(120.0).show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            for (i, name) in data.headers.iter().enumerate() {
                                changed |= ui.checkbox(&mut self.transform.keep[i], name).changed();
                            }
                        });
                    });
                    ui.small("保留勾选列及其原始顺序；取消勾选的列将从数据表移除，可撤销。");
                }
                if changed { self.transform.proposal = None; }
                if matches!(self.transform.operation, Operation::ToText | Operation::ToInteger | Operation::ToNumber | Operation::ToBool) {
                    ui.small("null 保留。整数限有符号/无符号 64 位；小数使用浮点表示，可能舍入。布尔只接受 true/false（忽略大小写和首尾空白）。对象转文本使用紧凑 JSON。任一行失败则不应用整列。");
                }
                ui.small("空字符串是 \"\"，null 是缺失值。纯空白需先去空白，再转 null；填充 null 不会替换空字符串。");
                ui.horizontal(|ui| {
                    if ui.button("预览转换").clicked() {
                        match if self.transform.operation == Operation::SelectColumns { select_columns(data, &self.transform.keep) } else { propose(data, self.transform.column, self.transform.operation, &self.transform.value) } {
                            Ok(proposal) => { self.transform.proposal = Some(proposal); self.message.clear(); }
                            Err(error) => { self.transform.proposal = None; self.message = format!("{error:#}"); }
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
        self.transform.force_open = false;
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
    fn explicit_types_are_strict_and_large_integers_are_exact() {
        assert_eq!(
            convert(
                &Value::String("18446744073709551615".into()),
                Operation::ToNumber
            )
            .unwrap(),
            Value::from(u64::MAX)
        );
        for text in ["18446744073709551616", "1.5", "1e3", ""] {
            assert!(convert(&Value::String(text.into()), Operation::ToInteger).is_err());
        }
        assert_eq!(
            convert(&Value::String(" TRUE ".into()), Operation::ToBool).unwrap(),
            Value::Bool(true)
        );
        assert!(convert(&Value::String("1".into()), Operation::ToBool).is_err());
        assert!(convert(&Value::String("1e999".into()), Operation::ToNumber).is_err());
        assert_eq!(
            convert(&Value::String("1.25".into()), Operation::ToNumber).unwrap(),
            serde_json::json!(1.25)
        );
        assert_eq!(
            convert(&Value::Null, Operation::ToText).unwrap(),
            Value::Null
        );
        assert_eq!(
            convert(&serde_json::json!({"a":1}), Operation::ToText).unwrap(),
            Value::String("{\"a\":1}".into())
        );
    }
    #[test]
    fn invalid_cell_rejects_entire_proposal_with_row_context() {
        let original = Dataset::parse("a\n42\nnot-a-number", DataFormat::Csv, b',').unwrap();
        let error = propose(&original, 0, Operation::ToInteger, "")
            .err()
            .unwrap();
        assert!(format!("{error:#}").contains("第 2 行"));
        assert_eq!(original.rows[0][0], "42");
    }
    #[test]
    fn projecting_sorted_columns_and_undo_reset_indices_and_preserve_values() {
        let original = data();
        assert!(select_columns(&original, &[false, false]).is_err());
        assert!(select_columns(&original, &[true]).is_err());
        let mut state = DataState {
            dataset: Some(original.clone()),
            sort: Some(1),
            ..Default::default()
        };
        state.transform.column = 1;
        state.transform.proposal = Some(select_columns(&original, &[false, true]).unwrap());
        state.apply_transform();
        assert_eq!(state.dataset.as_ref().unwrap().headers, vec!["b"]);
        assert_eq!(state.dataset.as_ref().unwrap().rows[0][0], 2);
        assert_eq!(state.sort, None);
        assert_eq!(state.transform.column, 0);
        state.undo_transform();
        assert_eq!(state.dataset.unwrap(), original);
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
