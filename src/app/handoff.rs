use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Tool(ToolKind),
    Csv,
    Tsv,
    JsonData,
    Before,
    After,
}
impl Target {
    fn label(self) -> &'static str {
        match self {
            Self::Tool(tool) => tool.label(),
            Self::Csv => "数据工作台 · CSV",
            Self::Tsv => "数据工作台 · TSV",
            Self::JsonData => "数据工作台 · JSON 对象数组",
            Self::Before => "文本对比 · 左侧原文",
            Self::After => "文本对比 · 右侧新文",
        }
    }
    fn all() -> Vec<Self> {
        ToolKind::ALL
            .into_iter()
            .filter(|kind| !matches!(kind, ToolKind::Uuid | ToolKind::Random))
            .map(Self::Tool)
            .chain([
                Self::Csv,
                Self::Tsv,
                Self::JsonData,
                Self::Before,
                Self::After,
            ])
            .collect()
    }
}

pub(super) struct Transfer {
    source: String,
    text: String,
    preview: String,
    query: String,
    target: Target,
    error: String,
    new_data_instance: bool,
}
impl Transfer {
    pub(super) fn new(source: String, text: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(!text.is_empty(), "当前没有可发送的结果");
        anyhow::ensure!(
            text.len() <= 2 * 1024 * 1024,
            "结果超过 2 MiB，请先缩小范围"
        );
        Ok(Self {
            source,
            text: text.into(),
            preview: text.chars().take(1200).collect(),
            query: String::new(),
            target: Target::Tool(ToolKind::Json),
            error: String::new(),
            new_data_instance: true,
        })
    }
    fn apply(
        &self,
        tools: &mut ToolState,
        data: &mut DataState,
        diff: &mut DiffState,
    ) -> anyhow::Result<(Page, Option<ToolKind>)> {
        match self.target {
            Target::Tool(kind) => {
                tools.select(kind);
                tools.input.clone_from(&self.text);
                tools.output.clear();
                tools.message.clear();
                tools.qr_image = None;
                Ok((
                    if kind.is_encoding() {
                        Page::EncodingTools
                    } else {
                        Page::SmallTools
                    },
                    Some(kind),
                ))
            }
            Target::Csv | Target::Tsv | Target::JsonData => {
                data.import_text(
                    self.text.clone(),
                    if self.target == Target::JsonData {
                        crate::workbench::DataFormat::Json
                    } else {
                        crate::workbench::DataFormat::Csv
                    },
                    self.target == Target::Tsv,
                )?;
                Ok((Page::Data, None))
            }
            Target::Before | Target::After => {
                if self.target == Target::Before {
                    diff.diff_before.clone_from(&self.text);
                } else {
                    diff.diff_after.clone_from(&self.text);
                }
                diff.diff_output.clear();
                diff.message.clear();
                Ok((Page::Diff, None))
            }
        }
    }
}

impl DevToolsApp {
    fn handoff_source(&self) -> Option<(String, &str)> {
        match self.page {
            Page::SmallTools | Page::EncodingTools => Some((
                self.tool_state.selected.label().into(),
                &self.tool_state.output,
            )),
            Page::Data => Some(("数据工作台导出".into(), &self.data_state.output)),
            Page::Diff => Some(("文本差异报告".into(), &self.diff_state.diff_output)),
            Page::Network => Some(("网络诊断报告".into(), &self.network_state.output)),
            Page::Java | Page::Django => Some((
                format!("{}诊断报告", self.frameworks.selected.category()),
                self.frameworks.result_text(),
            )),
            Page::Http => self
                .http_state
                .tabs
                .get(self.http_state.selected)
                .map(|tab| {
                    (
                        "HTTP 完整响应（含状态与响应头）".into(),
                        tab.output.as_str(),
                    )
                }),
            _ => None,
        }
    }
    pub(super) fn handoff_bar(&mut self, ctx: &egui::Context) {
        let Some((source, text)) = self.handoff_source() else {
            return;
        };
        let mut clicked = false;
        egui::TopBottomPanel::top("result-handoff").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                clicked = ui
                    .add_enabled(
                        !text.is_empty() && self.handoff.is_none(),
                        egui::Button::new("发送结果到工具…"),
                    )
                    .clicked();
                ui.small(if text.is_empty() {
                    "生成结果后，可交给其他工具继续处理"
                } else {
                    "预览结果 → 选择目标 → 继续处理"
                });
            });
        });
        if clicked {
            match Transfer::new(source, text) {
                Ok(transfer) => self.handoff = Some(transfer),
                Err(error) => self.toast = Some((error.to_string(), Instant::now())),
            }
        }
    }
    pub(super) fn handoff_dialog(&mut self, ctx: &egui::Context) {
        let Some(transfer) = self.handoff.as_mut() else {
            return;
        };
        let mut apply = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("handoff-modal")).show(ctx, |ui| {
            ui.set_width(480.0_f32.min(ctx.screen_rect().width() - 64.0));
            ui.heading("发送结果到工具");
            ui.label(format!("来源：{} · {} 字节", transfer.source, transfer.text.len()));
            ui.small("预览最多 1200 字符；发送完整快照。此内容仅保留在本次内存中。");
            egui::ScrollArea::vertical().id_salt("handoff-preview").max_height(100.0).show(ui, |ui| {
                ui.monospace(&transfer.preview);
            });
            ui.separator();
            ui.add(egui::TextEdit::singleline(&mut transfer.query).hint_text("搜索目标工具，例如 JSON、Base64、对比"));
            egui::ScrollArea::vertical().id_salt("handoff-targets").max_height(160.0).show(ui, |ui| {
                let query = transfer.query.trim().to_lowercase();
                let mut count = 0;
                for target in Target::all().into_iter().filter(|t| t.label().to_lowercase().contains(&query)) {
                    ui.selectable_value(&mut transfer.target, target, target.label());
                    count += 1;
                }
                if count == 0 { ui.label("没有匹配的目标，请调整关键词"); }
            });
            ui.separator();
            ui.label(format!("目标：{}", transfer.target.label()));
            let data_target = matches!(transfer.target, Target::Csv | Target::Tsv | Target::JsonData);
            if data_target { ui.checkbox(&mut transfer.new_data_instance,"在新数据实例中打开，保留已有工作"); }
            ui.small(if data_target && transfer.new_data_instance {"创建新实例并解析预览；当前工作和原结果保留。"} else {"将替换目标输入并清除旧结果，保留其他参数。数据工作台会解析预览，其他工具需手动运行。"});
            if !transfer.error.is_empty() { ui.colored_label(self.colors.red, &transfer.error); }
            ui.horizontal(|ui| {
                cancel = ui.button("取消").clicked();
                apply = ui.button(if data_target && transfer.new_data_instance {"新建实例并打开"}else{"替换输入并打开"}).clicked();
            });
        });
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            cancel = true;
        }
        if cancel {
            self.handoff = None;
        } else if apply {
            let transfer = self.handoff.as_ref().unwrap();
            let result = if transfer.new_data_instance
                && matches!(
                    transfer.target,
                    Target::Csv | Target::Tsv | Target::JsonData
                ) {
                self.data_state
                    .import_new(
                        transfer.text.clone(),
                        if transfer.target == Target::JsonData {
                            crate::workbench::DataFormat::Json
                        } else {
                            crate::workbench::DataFormat::Csv
                        },
                        transfer.target == Target::Tsv,
                        "接力数据",
                    )
                    .map(|_| (Page::Data, None))
            } else {
                transfer.apply(
                    &mut self.tool_state,
                    &mut self.data_state,
                    &mut self.diff_state,
                )
            };
            match result {
                Ok((page, kind)) => {
                    self.handoff = None;
                    self.navigate(page, kind);
                    self.qr_texture = None;
                    self.toast = Some(("结果已填入目标工具".into(), Instant::now()));
                }
                Err(error) => self.handoff.as_mut().unwrap().error = error.to_string(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transfer_preserves_source_and_other_drafts_without_running_target() {
        let mut tools = ToolState::default();
        tools.input = "source".into();
        tools.output = "result".into();
        tools.select(ToolKind::Base64);
        tools.input = "old target".into();
        tools.output = "stale".into();
        tools.select(ToolKind::Json);
        let mut transfer = Transfer::new("JSON".into(), &tools.output).unwrap();
        transfer.target = Target::Tool(ToolKind::Base64);
        transfer
            .apply(
                &mut tools,
                &mut DataState::default(),
                &mut DiffState::default(),
            )
            .unwrap();
        assert_eq!(tools.input, "result");
        assert!(tools.output.is_empty());
        tools.select(ToolKind::Json);
        assert_eq!(tools.input, "source");
        assert_eq!(tools.output, "result");
    }
    #[test]
    fn diff_transfer_preserves_other_side_and_clears_stale_report() {
        let mut diff = DiffState {
            diff_before: "left".into(),
            diff_after: "right".into(),
            diff_output: "old".into(),
            message: "old error".into(),
        };
        let mut transfer = Transfer::new("report".into(), "new").unwrap();
        transfer.target = Target::After;
        transfer
            .apply(
                &mut ToolState::default(),
                &mut DataState::default(),
                &mut diff,
            )
            .unwrap();
        assert_eq!(diff.diff_before, "left");
        assert_eq!(diff.diff_after, "new");
        assert!(diff.diff_output.is_empty() && diff.message.is_empty());
    }
    #[test]
    fn busy_data_rejects_transfer_without_replacing_input() {
        let mut data = DataState::default();
        data.import_text(
            "id\noriginal".into(),
            crate::workbench::DataFormat::Csv,
            false,
        )
        .unwrap();
        let mut transfer = Transfer::new("result".into(), "id\nreplacement").unwrap();
        transfer.target = Target::Csv;
        assert!(
            transfer
                .apply(
                    &mut ToolState::default(),
                    &mut data,
                    &mut DiffState::default()
                )
                .is_err()
        );
        assert_eq!(data.input, "id\noriginal");
        assert_eq!(transfer.text, "id\nreplacement");
    }
    #[test]
    fn snapshot_is_bounded_and_preview_preserves_unicode() {
        assert!(Transfer::new("test".into(), "").is_err());
        assert!(Transfer::new("test".into(), &"a".repeat(2 * 1024 * 1024 + 1)).is_err());
        let text = "中文🦀".repeat(600);
        let transfer = Transfer::new("test".into(), &text).unwrap();
        assert_eq!(transfer.preview.chars().count(), 1200);
        assert_eq!(transfer.text, text);
    }
}
