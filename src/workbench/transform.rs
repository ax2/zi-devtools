use super::*;
#[cfg(test)]
use zi_table_core::transform::convert;
pub(super) use zi_table_core::transform::{Operation, Proposal, propose, select_columns};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct State {
    column: usize,
    operation: Operation,
    value: String,
    proposal: Option<Proposal>,
    undo: Option<Dataset>,
    keep: Vec<bool>,
    force_open: bool,
}

impl State {
    pub(super) fn validate_saved(&self, columns: usize) -> Result<()> {
        anyhow::ensure!(
            self.column < columns.max(1) && (self.keep.is_empty() || self.keep.len() == columns),
            "转换列索引无效"
        );
        anyhow::ensure!(self.value.len() <= INPUT_LIMIT, "转换参数过大");
        if let Some(p) = &self.proposal {
            p.data.validate_saved()?;
        }
        if let Some(d) = &self.undo {
            d.validate_saved()?;
        }
        Ok(())
    }
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
        self.set_active_tool("data-transform");
        self.transform.force_open = true;
    }
    pub(super) fn run_transform_preview(&mut self) -> Result<()> {
        anyhow::ensure!(
            !self.busy() && !self.dialog_pending(),
            "请先结束任务或关闭当前审核"
        );
        let data = self.dataset.as_ref().context("请先解析表格")?;
        if self.transform.keep.len() != data.headers.len() {
            self.transform.keep = vec![true; data.headers.len()];
        }
        let result = if self.transform.operation == Operation::SelectColumns {
            select_columns(data, &self.transform.keep)
        } else {
            propose(
                data,
                self.transform.column,
                self.transform.operation,
                &self.transform.value,
            )
        };
        self.transform.proposal = None;
        self.transform.proposal = Some(result?);
        self.message.clear();
        self.transform.force_open = true;
        Ok(())
    }
    fn refresh_transformed_view(&mut self) {
        self.workflow.invalidate();
        self.output.clear();
        if let Some(data) = &self.dataset {
            self.visible = data.view(&self.query, self.sort, self.descending);
        }
    }
    pub(super) fn replace_with_join(&mut self, data: Dataset) {
        self.transform = State {
            undo: self.dataset.replace(data),
            ..Default::default()
        };
        self.sort = None;
        self.query.clear();
        self.join.invalidate();
        self.refresh_transformed_view();
    }
    fn apply_transform(&mut self) {
        if let Some(proposal) = self.transform.proposal.take() {
            self.join.invalidate();
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
    pub(super) fn can_undo_transform(&self) -> bool {
        self.transform.undo.is_some()
    }

    pub(super) fn undo_transform(&mut self) {
        if let Some(previous) = self.transform.undo.take() {
            self.join.invalidate();
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
        let mut preview_requested = false;
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
                    preview_requested = ui.button("预览转换").on_hover_text("Ctrl Enter；仅生成预览，不应用").clicked();
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
        } else if preview_requested && let Err(error) = self.run_transform_preview() {
            self.message = format!("{error:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_transform_restores_preview_and_undo_data() {
        let mut state = DataState::default();
        state.input = "name\n padded ".into();
        state.dataset = Some(Dataset::parse(&state.input, DataFormat::Csv, b',').unwrap());
        state.transform.proposal =
            Some(propose(state.dataset.as_ref().unwrap(), 0, Operation::Trim, "").unwrap());
        let mut restored = DataState::restore(&state.snapshot().unwrap()).unwrap();
        restored.apply_transform();
        assert_eq!(restored.dataset.as_ref().unwrap().rows[0][0], "padded");
        let mut restored = DataState::restore(&restored.snapshot().unwrap()).unwrap();
        restored.undo_transform();
        assert_eq!(restored.dataset.as_ref().unwrap().rows[0][0], " padded ");
    }
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
            rows: vec![vec![Value::Null]; 3000].into(),
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
