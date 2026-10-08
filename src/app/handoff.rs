use super::*;
#[cfg(feature = "ui-preview")]
mod date_preview;
mod discovery;
#[cfg(feature = "ui-preview")]
mod numeric_preview;
#[cfg(feature = "ui-preview")]
mod plot_preview;
#[cfg(feature = "ui-preview")]
mod table_statistics_preview;
use crate::calculator::exchange::{MatrixSlot, NumericTable, Representation};

fn calculator_version() -> &'static str {
    catalog()
        .iter()
        .find(|entry| entry.id == "advanced-calculator")
        .and_then(|entry| entry.version.as_deref())
        .unwrap_or("未声明")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Tool(ToolKind),
    Diagnostic(crate::framework::Tool),
    Csv,
    Tsv,
    JsonData,
    TextFlow,
    Before,
    After,
    Memo,
    Event,
    Calculator,
}
impl Target {
    fn label(self) -> &'static str {
        match self {
            Self::Tool(tool) => tool.label(),
            Self::Diagnostic(tool) => tool.label(),
            Self::Csv => "数据工作台 · CSV",
            Self::Tsv => "数据工作台 · TSV",
            Self::JsonData => "数据工作台 · JSON 对象数组",
            Self::TextFlow => "操作流程 · 文本工具链（新实例）",
            Self::Before => "文本对比 · 左侧原文",
            Self::After => "文本对比 · 右侧新文",
            Self::Memo => "备忘录 · 新建草稿",
            Self::Event => "万年历 / 日程 · 新建草稿",
            Self::Calculator => "计算器 · 数值表格填入矩阵",
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
                Self::TextFlow,
                Self::Before,
                Self::After,
                Self::Memo,
                Self::Event,
                Self::Calculator,
            ])
            .chain(
                crate::framework::Tool::ALL
                    .into_iter()
                    .filter(|tool| tool.accepts_text_handoff())
                    .map(Self::Diagnostic),
            )
            .collect()
    }
}

pub(super) struct Transfer {
    matrix_slot: MatrixSlot,
    allow_matrix_replace: bool,
    numeric: Option<NumericTable>,
    event_date: Option<chrono::NaiveDate>,
    representation: Representation,
    numeric_rendered: Option<(Representation, Target)>,
    source: String,
    origin_service: Option<String>,
    text: String,
    preview: String,
    query: String,
    target: Target,
    error: String,
    new_data_instance: bool,
    category: String,
    recommendations: Vec<discovery::Recommendation>,
    matches_key: Option<(String, String)>,
    matches: Vec<Target>,
    #[cfg(feature = "ui-preview")]
    preview_rects: [Option<egui::Rect>; 2],
    #[cfg(feature = "ui-preview")]
    numeric_mode_rects: [Option<egui::Rect>; 3],
    #[cfg(feature = "ui-preview")]
    matrix_rects: [Option<egui::Rect>; 3],
    #[cfg(feature = "ui-preview")]
    preview_recommendation_rect: Option<egui::Rect>,
    #[cfg(feature = "ui-preview")]
    preview_log_target: Option<egui::Rect>,
}
impl Transfer {
    pub(super) fn new(source: String, text: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(!text.is_empty(), "当前没有可发送的结果");
        anyhow::ensure!(
            text.len() <= 2 * 1024 * 1024,
            "结果超过 2 MiB，请先缩小范围"
        );
        Ok(Self {
            matrix_slot: MatrixSlot::A,
            allow_matrix_replace: false,
            numeric: None,
            event_date: None,
            representation: Representation::Typed,
            numeric_rendered: None,
            source,
            origin_service: None,
            text: text.into(),
            preview: text.chars().take(1200).collect(),
            query: String::new(),
            target: Target::Tool(ToolKind::Json),
            error: String::new(),
            new_data_instance: true,
            category: String::new(),
            recommendations: discovery::recommendations(text),
            matches_key: None,
            matches: Vec::new(),
            #[cfg(feature = "ui-preview")]
            preview_rects: [None; 2],
            #[cfg(feature = "ui-preview")]
            numeric_mode_rects: [None; 3],
            #[cfg(feature = "ui-preview")]
            matrix_rects: [None; 3],
            #[cfg(feature = "ui-preview")]
            preview_recommendation_rect: None,
            #[cfg(feature = "ui-preview")]
            preview_log_target: None,
        })
    }
    fn dated(source: String, date: chrono::NaiveDate, text: &str) -> anyhow::Result<Self> {
        use chrono::Datelike;
        anyhow::ensure!((1..=9999).contains(&date.year()), "日期快照支持0001–9999年");
        let mut transfer = Self::new(source, text)?;
        transfer.event_date = Some(date);
        transfer.target = Target::Event;
        Ok(transfer)
    }
    fn numeric(source: String, table: NumericTable) -> anyhow::Result<Self> {
        let text = table
            .json(Representation::Typed)
            .map_err(anyhow::Error::msg)?;
        let mut transfer = Self::new(source, &text)?;
        transfer.numeric = Some(table);
        transfer.target = Target::JsonData;
        Ok(transfer)
    }
    fn refresh_numeric(&mut self) {
        let Some(table) = &self.numeric else {
            return;
        };
        let key = (self.representation, self.target);
        if self.numeric_rendered == Some(key) {
            return;
        }
        let result = match self.target {
            Target::Csv => table.delimited(self.representation, b','),
            Target::Tsv => table.delimited(self.representation, b'\t'),
            _ => table.json(self.representation),
        };
        match result {
            Ok(text) => {
                self.text = text;
                self.preview = self.text.chars().take(1200).collect();
                self.error.clear();
                self.numeric_rendered = Some(key);
            }
            Err(error) => {
                self.preview = format!("无法生成此格式：{error}");
                self.text.clear();
                self.error = error;
                self.numeric_rendered = Some(key);
            }
        }
    }
    fn compatible(&self) -> bool {
        if self.event_date.is_some() && self.target == Target::Calculator {
            return false;
        }
        !(self.numeric.is_some()
            && self.representation == Representation::Typed
            && matches!(self.target, Target::Csv | Target::Tsv))
    }
    fn matching_targets(&mut self) -> Vec<Target> {
        let key = (registry::normalized(&self.query), self.category.clone());
        if self.matches_key.as_ref() != Some(&key) {
            self.matches = discovery::search(&key.0, &key.1);
            self.matches_key = Some(key);
        }
        self.matches
            .iter()
            .copied()
            .filter(|target| {
                !(self.event_date.is_some() && *target == Target::Calculator
                    || self.numeric.is_some()
                        && self.representation == Representation::Typed
                        && matches!(target, Target::Csv | Target::Tsv))
            })
            .collect()
    }
    fn apply(
        &self,
        tools: &mut ToolState,
        data: &mut DataState,
        diff: &mut DiffState,
    ) -> anyhow::Result<(Page, Option<ToolKind>)> {
        match self.target {
            Target::Memo
            | Target::Event
            | Target::Calculator
            | Target::Diagnostic(_)
            | Target::TextFlow => {
                anyhow::bail!("备忘 / 日程草稿需通过资料入口接收")
            }
            Target::Tool(kind) => {
                tools.select(kind);
                tools.input.clone_from(&self.text);
                tools.clear_result();
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
    #[cfg(feature = "ui-preview")]
    pub fn preview_image_report_prepare(&mut self, ctx: &egui::Context, light: bool) {
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.startup_warning = None;
        self.page = Page::Images;
        self.images = Default::default();
        self.images.preview_fixture(ctx);
        self.open_startup_tool("image-tools");
        self.tool_state.select(ToolKind::Json);
        self.tool_state.input = "existing-target-draft".into();
        self.tool_state.clear_result();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_batch_report_prepare(&mut self, ctx: &egui::Context, light: bool) {
        self.preview_image_report_prepare(ctx, light);
        self.images.preview_batch_report_fixture();
        self.open_startup_tool("image-batch");
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_image_report_check(&self, phase: u8) {
        let text = self.images.preview_image_report_assert();
        match phase {
            0 => {
                assert!(self.handoff.is_none());
                assert_eq!(self.page, Page::Images);
                assert_eq!(self.handoff_source().unwrap().1, text);
            }
            1 => {
                assert_eq!(self.handoff.as_ref().unwrap().text, text);
                assert_eq!(self.tool_state.input, "existing-target-draft");
            }
            2 => {
                assert!(self.handoff.is_none());
                assert_eq!(self.tool_state.input, "existing-target-draft");
            }
            3 => {
                assert!(self.handoff.is_none());
                assert_eq!(self.tool_state.selected, ToolKind::Json);
                assert_eq!(self.tool_state.input, text);
                assert!(!self.tool_state.has_result());
            }
            4 => {
                assert!(self.handoff.is_none());
                assert_eq!(self.page, Page::Notes);
                assert!(self.planner.has_unsaved());
                assert_eq!(self.planner.transfer_text().unwrap().1, text);
            }
            _ => panic!("image report phase"),
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_image_report_memo_prepare(&mut self) {
        self.planner.preview(false, false);
        let text = self.images.preview_image_report_assert();
        let mut transfer = Transfer::new("图片信息报告（冻结快照）".into(), &text).unwrap();
        transfer.target = Target::Memo;
        self.handoff = Some(transfer);
        self.page = Page::Images;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_file_report_dispatch(&mut self) {
        let report = self.file_state.report().to_owned();
        let (source, text) = self.handoff_source().unwrap();
        let mut transfer = Transfer::new(source, text).unwrap();
        transfer.target = Target::Tool(ToolKind::Json);
        self.handoff = Some(transfer);
        self.apply_handoff();
        assert_eq!(self.tool_state.input, report);
        assert!(!self.tool_state.has_result());
        assert_eq!(self.file_state.report(), report);
        self.page = Page::Files;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_handoff_result_prepare(&mut self, ctx: &egui::Context, light: bool) {
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.startup_warning = None;
        self.tool_state.select(ToolKind::Base64);
        self.tool_state.input.clear();
        let result = self.tool_state.run(0);
        assert!(self.tool_state.apply_result(result));
        assert!(self.tool_state.has_result() && self.tool_state.output.is_empty());
        self.tool_state.select(ToolKind::Json);
        self.tool_state.input = r#"{"source":true}"#.into();
        let result = self.tool_state.run(0);
        assert!(self.tool_state.apply_result(result));
        let mut transfer = Transfer::new("JSON source".into(), &self.tool_state.output).unwrap();
        transfer.target = Target::Tool(ToolKind::Base64);
        self.handoff = Some(transfer);
        self.apply_handoff();
        assert!(self.handoff.is_none());
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_handoff_result_copy_position(&self) -> egui::Pos2 {
        self.preview_text_copy
            .expect("rendered copy button")
            .0
            .center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_handoff_result_check(&mut self, ran: bool) {
        assert_eq!(self.tool_state.selected, ToolKind::Base64);
        assert_eq!(self.tool_state.input, "{\n  \"source\": true\n}");
        assert_eq!(self.tool_state.has_result(), ran);
        assert_eq!(self.preview_text_copy.expect("rendered copy button").1, ran);
        if ran {
            assert_eq!(self.tool_state.output, self.tool_state.run(0).unwrap());
        } else {
            assert!(self.tool_state.output.is_empty());
        }
        self.tool_state.select(ToolKind::Json);
        assert_eq!(self.tool_state.input, r#"{"source":true}"#);
        assert!(self.tool_state.has_result());
        assert_eq!(self.tool_state.output, "{\n  \"source\": true\n}");
        self.tool_state.select(ToolKind::Base64);
    }
    pub(super) fn send_service_logs(&mut self, service_id: &str, filtered: bool) {
        if self.log_inflight || self.log_error || self.handoff.is_some() {
            return;
        }
        self.log_lines.update(&self.log_text, &self.log_filter);
        let text = if filtered {
            self.log_lines.copy_matches(&self.log_text)
        } else {
            self.log_text.clone()
        };
        let source = format!(
            "服务日志 · {service_id} · {} · 末尾有界快照",
            if filtered {
                "筛选结果"
            } else {
                "全部预览"
            }
        );
        match Transfer::new(source, &text) {
            Ok(mut transfer) => {
                transfer.origin_service = Some(service_id.into());
                self.handoff = Some(transfer);
                self.log_view = None;
            }
            Err(error) => {
                self.toast = Some((error.to_string(), Instant::now()));
            }
        }
    }
    fn cancel_handoff(&mut self) {
        let origin = self
            .handoff
            .take()
            .and_then(|transfer| transfer.origin_service);
        if let Some(id) = origin.filter(|id| self.manager.service_spec(id).is_ok()) {
            self.log_view = Some(id);
            self.last_logs = Instant::now();
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_log_transfer_scene(&mut self) {
        self.page = Page::Services;
        self.startup_warning = None;
        self.log_inflight = false;
        self.log_error = false;
        self.handoff = None;
        self.log_text = crate::framework::Tool::Gc.sample().into();
        self.log_lines.invalidate();
        self.log_filter = "gc".into();
        self.send_service_logs("demo", true);
        let transfer = self.handoff.as_mut().unwrap();
        transfer.target = Target::Diagnostic(crate::framework::Tool::Gc);
        transfer.query = "gc".into();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_log_handoff_prepare(&mut self, round: usize) {
        self.page = Page::Services;
        self.startup_warning = None;
        self.notification.clear();
        self.handoff = None;
        self.log_auto = false;
        let text = match round {
            0 => "noise\n[0.1s][gc] GC(0) Pause Young 10M->2M(20M) 1.0ms\n",
            1 => {
                "Traceback (most recent call last):\n  File \"demo.py\", line 1, in demo\nValueError: example\n"
            }
            _ => {
                "Exception in thread \"main\" java.lang.IllegalStateException: example\n\tat demo.Main.run(Main.java:1)\n"
            }
        };
        fs::write(
            self.manager
                .config_snapshot()
                .state_dir
                .join("logs/demo.log"),
            text,
        )
        .unwrap();
        self.request_logs("demo".into());
        if round == 0 {
            self.log_filter = "[gc]".into();
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_log_handoff_position(&self, index: usize) -> egui::Pos2 {
        let transfer = self.handoff.as_ref().unwrap();
        if index == 0 {
            transfer.preview_log_target.unwrap().center()
        } else {
            transfer.preview_rects[index].unwrap().center()
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_log_handoff_check(&mut self, round: usize, phase: usize) {
        match phase {
            0 => {
                assert!(!self.log_inflight && !self.log_error);
                self.log_error = true;
                self.send_service_logs("demo", false);
                assert!(self.handoff.is_none());
                self.log_error = false;
                self.log_inflight = true;
                self.send_service_logs("demo", false);
                assert!(self.handoff.is_none());
                self.log_inflight = false;
                let filter = self.log_filter.clone();
                let target_input = self.tool_state.input.clone();
                self.send_service_logs("demo", true);
                assert!(self.handoff.is_some() && self.log_view.is_none());
                self.cancel_handoff();
                assert_eq!(self.log_view.as_deref(), Some("demo"));
                assert_eq!(self.log_filter, filter);
                assert_eq!(self.tool_state.input, target_input);
            }
            1 => {
                let transfer = self.handoff.as_mut().unwrap();
                assert!(self.log_view.is_none());
                if round == 0 {
                    assert!(!transfer.text.contains("noise"));
                    assert!(transfer.text.contains("[gc]"));
                }
                let snapshot = transfer.text.clone();
                self.log_text = "later log update".into();
                self.log_lines.invalidate();
                assert_eq!(transfer.text, snapshot);
                transfer.query = match round {
                    0 => crate::framework::Tool::Gc.id().into(),
                    1 => ToolKind::DjangoTrace.id().into(),
                    _ => ToolKind::JavaTrace.id().into(),
                };
            }
            2 => {
                assert!(self.handoff.is_none());
                if round == 0 {
                    assert_eq!(self.page, Page::Java);
                    self.frameworks
                        .preview_received_log(crate::framework::Tool::Gc, "[gc]");
                } else {
                    assert_eq!(
                        self.tool_state.selected,
                        if round == 1 {
                            ToolKind::DjangoTrace
                        } else {
                            ToolKind::JavaTrace
                        }
                    );
                    assert!(self.tool_state.output.is_empty());
                    assert!(self.tool_state.input.contains(if round == 1 {
                        "Traceback"
                    } else {
                        "Exception in thread"
                    }));
                    assert!(
                        run_tool(self.tool_state.selected, 0, &self.tool_state.input, "", 10)
                            .is_ok()
                    );
                }
            }
            _ => unreachable!(),
        }
    }
    pub(super) fn handoff_source(&self) -> Option<(String, &str)> {
        match self.page {
            Page::SmallTools | Page::EncodingTools => Some((
                self.tool_state.selected.label().into(),
                &self.tool_state.output,
            )),
            Page::Clipboard => self.clipboard.transfer_text(),
            Page::Files => Some(("文件校验报告".into(), self.file_state.report())),
            Page::Images => self.images.image_report().map(|text| {
                (
                    format!("{} 报告（冻结快照）", self.images.active_tool_id()),
                    text,
                )
            }),
            Page::Data => Some(("数据工作台导出".into(), &self.data_state.output)),
            Page::Notes | Page::Calendar => self.planner.transfer_text(),
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
        if self.page == Page::Calculator && self.calculator.date_output_active() {
            return;
        }
        if self.page == Page::Calculator && self.calculator.plot_active() {
            let result = self.calculator.plot_description();
            let mut send = false;
            egui::TopBottomPanel::top("result-handoff").show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let response = ui.add_enabled(
                        result.is_ok() && self.handoff.is_none(),
                        egui::Button::new("发送采样表到工具…"),
                    );
                    send = response.clicked();
                    #[cfg(feature = "ui-preview")]
                    {
                        self.calculator.preview_numeric_send = Some(response.rect);
                    }
                    match &result {
                        Ok(description) => {
                            ui.small(description);
                        }
                        Err(e) => {
                            ui.small(e);
                        }
                    }
                });
            });
            if send {
                match Transfer::new(
                    format!("函数采样表 · 计算器v{} · 近似值", calculator_version()),
                    self.calculator.plot_csv().unwrap(),
                ) {
                    Ok(mut transfer) => {
                        transfer.target = Target::Csv;
                        transfer.new_data_instance = true;
                        self.handoff = Some(transfer);
                    }
                    Err(e) => self.toast = Some((e.to_string(), Instant::now())),
                }
            }
            return;
        }
        if self.page == Page::Calculator {
            let result = self.calculator.numeric_description();
            let mut send = false;
            egui::TopBottomPanel::top("result-handoff").show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let response = ui.add_enabled(
                        result.is_ok() && self.handoff.is_none(),
                        egui::Button::new("发送数值到工具…"),
                    );
                    send = response.clicked();
                    #[cfg(feature = "ui-preview")]
                    {
                        self.calculator.preview_numeric_send = Some(response.rect);
                    }
                    match &result {
                        Ok(table) => {
                            ui.small(format!("{table} · 发送时捕获快照，不固定赋值"));
                        }
                        Err(error) => {
                            ui.small(error);
                        }
                    }
                });
            });
            if send {
                match Transfer::numeric(
                    format!("计算器结果快照 · v{}", calculator_version()),
                    self.calculator.numeric_result().unwrap(),
                ) {
                    Ok(t) => self.handoff = Some(t),
                    Err(e) => self.toast = Some((e.to_string(), Instant::now())),
                }
            }
            return;
        }
        let Some((source, text)) = self.handoff_source() else {
            return;
        };
        let mut clicked = false;
        let body_source = matches!(self.page, Page::Notes | Page::Calendar);
        egui::TopBottomPanel::top("result-handoff").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                let response = ui.add_enabled(
                    !text.is_empty() && self.handoff.is_none(),
                    egui::Button::new(if body_source {
                        "发送正文到工具…"
                    } else {
                        "发送结果到工具…"
                    }),
                );
                clicked = response.clicked();
                #[cfg(feature = "ui-preview")]
                if self.page == Page::Images {
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(egui::Id::new("image-report-send"), response.rect)
                    });
                }
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
        if self.handoff.is_none()
            && let Some((source, table)) = self.data_state.take_numeric_request()
        {
            match Transfer::numeric(source, table) {
                Ok(mut transfer) => {
                    transfer.target = Target::Calculator;
                    self.handoff = Some(transfer);
                }
                Err(e) => self.toast = Some((e.to_string(), Instant::now())),
            }
        }
        let Some(transfer) = self.handoff.as_mut() else {
            return;
        };
        let mut apply = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("handoff-modal")).show(ctx, |ui| {
            ui.set_width(480.0_f32.min(ctx.screen_rect().width() - 64.0));
            let footer_height = if transfer.target == Target::Calculator { 300.0 } else { 150.0 } + if transfer.error.is_empty() { 0.0 } else { 70.0 };
            egui::ScrollArea::vertical().id_salt("handoff-body").max_height((ctx.screen_rect().height()-footer_height).max(100.0)).show(ui,|ui| {
            ui.heading("发送结果到工具");
            if let Some(table)=&transfer.numeric {
                ui.label(format!("数值快照：{}",table.description()));
                ui.horizontal_wrapped(|ui| {for (index,mode) in Representation::ALL.into_iter().enumerate() {let response=ui.selectable_value(&mut transfer.representation,mode,mode.label());
                    #[cfg(feature="ui-preview")] {transfer.numeric_mode_rects[index]=Some(response.rect);}
                    #[cfg(not(feature="ui-preview"))] {let _=(index,response);}
                }});
                ui.small(match transfer.representation {Representation::Typed=>"规范分子/分母字符串与完整f64类型。只有这种JSON表格可以无损送回矩阵。",Representation::Text=>"保留完整数值文本（近似值带≈），JSON/CSV/TSV不再保留数值类型；回传矩阵需保留类型。",Representation::Approximate=>"明确转换为近似f64：大整数、分数可能损失精度；JSON/CSV/TSV无法恢复原精确值。"});
            }
            transfer.refresh_numeric();
            ui.label(format!("来源：{} · {} 字节", transfer.source, transfer.text.len()));
            ui.small("预览最多 1200 字符；发送完整快照。此内容仅保留在本次内存中。");
            egui::ScrollArea::vertical().id_salt("handoff-preview").max_height(100.0).show(ui, |ui| {
                ui.monospace(&transfer.preview);
            });
            ui.separator();
            ui.add(egui::TextEdit::singleline(&mut transfer.query).hint_text("搜索目标，例如 JSON、对比、备忘录、日程"));
            egui::ComboBox::from_id_salt("handoff-category")
                .selected_text(if transfer.category.is_empty() { "全部分类" } else { &transfer.category })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut transfer.category, String::new(), "全部分类");
                    for category in discovery::categories() {
                        ui.selectable_value(&mut transfer.category, category.clone(), category);
                    }
                });
            let targets = transfer.matching_targets();
            ui.small(format!("{} 个可接收目标 · 可按名称、别名、分类或用途搜索", targets.len()));
            egui::ScrollArea::vertical().id_salt("handoff-targets").max_height(160.0).show(ui, |ui| {
                let recommended: Vec<_> = if transfer.query.trim().is_empty() {
                    transfer.recommendations.iter().filter(|item| targets.contains(&item.target)).collect()
                } else { Vec::new() };
                if !recommended.is_empty() {
                    ui.small("推荐目标 · 仅提示，不自动执行");
                    for item in &recommended {
                        let response = ui.selectable_value(&mut transfer.target, item.target, item.target.label());
                        #[cfg(feature = "ui-preview")]
                        if item.target == Target::JsonData { transfer.preview_recommendation_rect = Some(response.rect); }
                        if response.changed() { transfer.error.clear(); }
                        ui.label(egui::RichText::new(item.reason).small().weak());
                    }
                    ui.separator();
                    ui.small("其他目标");
                }
                for target in targets.iter().copied().filter(|target| !recommended.iter().any(|item| item.target == *target)) {
                    let _response = ui.selectable_value(&mut transfer.target, target, target.label());
                    if _response.changed() { transfer.error.clear(); }
                    #[cfg(feature = "ui-preview")]
                    if target == Target::Event { transfer.preview_rects[0] = Some(_response.rect); }
                    #[cfg(feature = "ui-preview")]
                    if matches!(target, Target::Diagnostic(_) | Target::Tool(ToolKind::JavaTrace | ToolKind::DjangoTrace)) { transfer.preview_log_target = Some(_response.rect); }
                }
                if targets.is_empty() { ui.label("没有匹配的目标，请调整关键词或分类。已选目标仍显示在下方。"); }
            });
            ui.separator();
            transfer.refresh_numeric();
            let data_target = matches!(transfer.target, Target::Csv | Target::Tsv | Target::JsonData);
            if transfer.target == Target::Event {
                if let Some(date)=transfer.event_date { ui.label(format!("新日程：{date} 全天 · 不重复 · 提醒关闭")); }
                else { ui.label(format!("新日程：{} 09:00 · 本机时区 · 不重复 · 提醒关闭", self.planner.incoming_event_date())); }
            }
            if data_target { ui.checkbox(&mut transfer.new_data_instance,"在新数据实例中打开，保留已有工作"); }
            ui.small(if matches!(transfer.target, Target::Diagnostic(_)) {"替换该诊断主输入并清除旧结果，保留对照输入与其他参数；同目标执行中拒绝接收。接收不自动分析或执行命令。"} else if transfer.target == Target::Event {"完整结果作为日程正文，最多 128 KiB；已有备忘/日程编辑需先保存或放弃。打开后调整日期时间并主动开启提醒，再保存到本机。"} else if transfer.target == Target::Memo {"创建备忘草稿，最多 128 KiB；已有编辑需先保存或放弃。点击备忘录中的保存后才会写入本机。"} else if data_target && transfer.new_data_instance {"创建新实例并解析预览；当前工作和原结果保留。"} else {"将替换目标输入并清除旧结果，保留其他参数。数据工作台会解析预览，其他工具需手动运行。"});
            });
            ui.separator();
            ui.label(format!("目标：{}", transfer.target.label()));
            if let Some(date)=transfer.event_date {
                ui.label(format!("日期快照：{date} · 日程目标使用此日期，不使用当前选中日期"));
                if transfer.target==Target::Event { ui.small("创建单次全天草稿，提醒关闭，不自动保存。日程支持1901–2099年，范围外可选择备忘录或文本目标。"); }
            }

            if transfer.target==Target::Calculator {
                let previous_slot=transfer.matrix_slot;
                ui.horizontal_wrapped(|ui| {ui.label("接收位置");let a=ui.selectable_value(&mut transfer.matrix_slot,MatrixSlot::A,"矩阵A");let b=ui.selectable_value(&mut transfer.matrix_slot,MatrixSlot::B,"矩阵B");
                    #[cfg(feature="ui-preview")] {transfer.matrix_rects[0]=Some(a.rect);transfer.matrix_rects[1]=Some(b.rect);}
                    #[cfg(not(feature="ui-preview"))] {let _=(a,b);}});
                if previous_slot!=transfer.matrix_slot {transfer.allow_matrix_replace=false;}
                if self.calculator.dirty() {let _response=ui.checkbox(&mut transfer.allow_matrix_replace,format!("允许替换未保存的{}（其他工作保留）",transfer.matrix_slot.label()));
                    #[cfg(feature="ui-preview")] {transfer.matrix_rects[2]=Some(_response.rect);}}
                match NumericTable::read_json(&transfer.text) {Ok(table)=>{ui.label(format!("将替换{}：{}",transfer.matrix_slot.label(),table.description()));},Err(e)=>{ui.colored_label(self.colors.red,e);}}
                ui.small("仅接受c1..cN规范数值类型JSON；未保存内容须先保存或明确允许替换所选矩阵；待读取或后台I/O须先处理。保留另一矩阵、算式和变量，不自动计算；接收后标记未保存。");
            }
            if !transfer.error.is_empty() {
                egui::ScrollArea::vertical().id_salt("handoff-error").max_height(56.0).show(ui, |ui| { ui.colored_label(self.colors.red, &transfer.error); });
            }
            let data_target=matches!(transfer.target,Target::Csv|Target::Tsv|Target::JsonData);
            ui.horizontal(|ui| {
                cancel = ui.button("取消").clicked();
                let response = ui.add_enabled(transfer.compatible(),egui::Button::new(if transfer.target == Target::Event {"创建日程草稿"} else if transfer.target == Target::Memo {"创建备忘草稿"} else if data_target && transfer.new_data_instance {"新建实例并打开"}else{"替换输入并打开"}));
                apply = response.clicked();
                #[cfg(feature = "ui-preview")]
                { transfer.preview_rects[1] = Some(response.rect); ui.ctx().data_mut(|data|data.insert_temp(egui::Id::new("handoff-apply"),response.rect)); }
            });
        });
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            cancel = true;
        }
        if cancel {
            self.cancel_handoff();
        } else if apply {
            self.apply_handoff();
        }
    }
    pub(super) fn finish_date_transfer(&mut self) {
        match self.calculator.take_date_transfer() {
            Ok(Some((date, text))) if self.handoff.is_none() => match Transfer::dated(
                format!("计算器日期结果 · v{}", calculator_version()),
                date,
                &text,
            ) {
                Ok(transfer) => self.handoff = Some(transfer),
                Err(error) => self.toast = Some((error.to_string(), Instant::now())),
            },
            Err(error) => self.toast = Some((error, Instant::now())),
            _ => {}
        }
    }
    fn apply_handoff(&mut self) {
        if let Some(transfer) = self.handoff.as_ref() {
            if !transfer.compatible() {
                self.handoff.as_mut().unwrap().error = "类型保留需选择JSON目标".into();
                return;
            }
            let result = if transfer.target == Target::Calculator {
                self.calculator
                    .receive_numeric_with_policy(
                        &transfer.text,
                        transfer.matrix_slot,
                        transfer.allow_matrix_replace,
                    )
                    .map(|_| (Page::Calculator, None))
                    .map_err(anyhow::Error::msg)
            } else if let Target::Diagnostic(tool) = transfer.target {
                self.frameworks
                    .receive_handoff(tool, transfer.text.clone())
                    .map(|_| {
                        (
                            if tool.category() == "Java 与 JVM" {
                                Page::Java
                            } else {
                                Page::Django
                            },
                            None,
                        )
                    })
            } else if transfer.target == Target::Event {
                (if let Some(date) = transfer.event_date {
                    self.planner
                        .receive_event_date(&transfer.source, &transfer.text, date)
                } else {
                    self.planner
                        .receive_event_text(&transfer.source, &transfer.text)
                })
                .map(|_| (Page::Calendar, None))
            } else if transfer.target == Target::Memo {
                self.planner
                    .receive_text(&transfer.source, &transfer.text)
                    .map(|_| (Page::Notes, None))
            } else if transfer.target == Target::TextFlow {
                self.data_state
                    .import_text_flow(transfer.text.clone(), "接力文本流程")
                    .map(|_| (Page::Data, None))
            } else if transfer.new_data_instance
                && matches!(
                    transfer.target,
                    Target::Csv | Target::Tsv | Target::JsonData
                )
            {
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
    #[cfg(feature = "ui-preview")]
    pub fn preview_handoff_discovery_position(&self, index: usize) -> egui::Pos2 {
        let transfer = self.handoff.as_ref().unwrap();
        if index == 0 {
            transfer.preview_recommendation_rect.unwrap().center()
        } else {
            transfer.preview_rects[1].unwrap().center()
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_handoff_discovery_smoke(&mut self, phase: u8) {
        if phase == 0 {
            self.preview_memo_handoff(false);
            self.data_state.input = "existing data work".into();
            self.data_state.output = "original export".into();
            self.tool_state.output =
                r#"[{"name":"Zi","count":1},{"name":"Tools","count":2}]"#.into();
            self.handoff =
                Some(Transfer::new("JSON 对象数组结果".into(), &self.tool_state.output).unwrap());
        } else {
            assert!(self.handoff.is_none() && self.page == Page::Data);
            assert_eq!(self.data_state.instances.len(), 2);
            let old = &self.data_state.instances[0];
            assert_eq!(old.state.input, "existing data work");
            assert_eq!(old.state.output, "original export");
            assert_ne!(self.data_state.active_id(), old.id);
            assert_eq!(self.data_state.input, self.tool_state.output);
            assert!(self.data_state.format == crate::workbench::DataFormat::Json);
            assert_eq!(self.data_state.parse_job.phase, crate::tasks::Phase::Done);
            println!(
                "PASS handoff discovery: actual recommended JSON-table click and explicit apply; new instance parses successfully; original data draft/export and source JSON preserved"
            );
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_event_handoff_position(&self, index: usize) -> egui::Pos2 {
        self.handoff.as_ref().unwrap().preview_rects[index]
            .unwrap()
            .center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_event_handoff_scene(&mut self) {
        self.preview_event_handoff_smoke(0);
        self.handoff.as_mut().unwrap().target = Target::Event;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_event_handoff_smoke(&mut self, phase: u8) {
        if phase == 0 {
            self.preview_memo_handoff(false);
            let transfer = self.handoff.as_mut().unwrap();
            transfer.target = Target::Tool(ToolKind::Json);
            transfer.query = "日程".into();
        } else {
            assert!(self.handoff.is_none() && self.page == Page::Calendar);
            let source = self.tool_state.output.clone();
            self.planner.preview_incoming_event_assert(&source);
            let mut transfer = Transfer::new("another".into(), "must not overwrite").unwrap();
            transfer.target = Target::Event;
            self.handoff = Some(transfer);
            self.apply_handoff();
            assert!(!self.handoff.as_ref().unwrap().error.is_empty());
            self.planner.preview_incoming_event_assert(&source);
            self.handoff = None;
            println!(
                "PASS event handoff: actual target selection and create click; full source preserved; unsaved event, selected date at 09:00, reminder off; second handoff rejected; no database write"
            );
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_memo_handoff(&mut self, receive: bool) {
        self.planner.preview(false, false);
        let mut transfer = Transfer::new(
            "JSON 格式化结果".into(),
            "{\n  \"project\": \"Zi DevTools\",\n  \"status\": \"ready\"\n}",
        )
        .unwrap();
        transfer.target = Target::Memo;
        transfer.query = "备忘录".into();
        self.handoff = Some(transfer);
        self.page = Page::EncodingTools;
        self.tool_state.select(ToolKind::Json);
        self.tool_state.output = self.handoff.as_ref().unwrap().text.clone();
        if receive {
            self.apply_handoff();
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_memo_roundtrip(&mut self) {
        self.preview_memo_handoff(true);
        assert_eq!(self.page, Page::Notes);
        assert!(self.planner.has_unsaved());
        let source = self.tool_state.output.clone();
        let (title, text) = self.handoff_source().unwrap();
        assert_eq!(text, source);
        let mut transfer = Transfer::new(title, text).unwrap();
        transfer.target = Target::Tool(ToolKind::Base64);
        self.handoff = Some(transfer);
        self.apply_handoff();
        assert_eq!(self.tool_state.selected, ToolKind::Base64);
        assert_eq!(self.tool_state.input, source);
        assert!(self.tool_state.output.is_empty());
        self.tool_state.select(ToolKind::Json);
        assert_eq!(self.tool_state.output, source);
        let mut transfer = Transfer::new("新结果".into(), "replacement").unwrap();
        transfer.target = Target::Memo;
        self.handoff = Some(transfer);
        self.apply_handoff();
        assert!(self.handoff.as_ref().is_some_and(|t| !t.error.is_empty()));
        assert_eq!(self.planner.transfer_text().unwrap().1, source);
        self.handoff = None;
        self.planner.preview(false, false);
        println!(
            "PASS memo handoff: actual app routes, source preserved, unsaved draft guarded, outbound input without execution"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receiving_text_invalidates_empty_and_nonempty_success_without_running_target() {
        for previous in ["", "older target input"] {
            for same_tool in [false, true] {
                let mut tools = ToolState::default();
                tools.select(ToolKind::Base64);
                tools.input = previous.into();
                let result = tools.run(0);
                assert!(tools.apply_result(result));
                assert!(tools.has_result());
                if !same_tool {
                    tools.select(ToolKind::Json);
                    tools.input = r#"{"source":true}"#.into();
                    let result = tools.run(0);
                    assert!(tools.apply_result(result));
                }
                let old_source = (tools.input.clone(), tools.output.clone());
                let mut transfer = Transfer::new("fixture".into(), "新的资料🦀").unwrap();
                transfer.target = Target::Tool(ToolKind::Base64);
                transfer
                    .apply(
                        &mut tools,
                        &mut DataState::default(),
                        &mut DiffState::default(),
                    )
                    .unwrap();
                assert_eq!(tools.input, "新的资料🦀");
                assert!(
                    !tools.has_result(),
                    "new input has no completed result to copy or swap"
                );
                assert!(tools.output.is_empty() && tools.message.is_empty());
                assert_eq!(transfer.text, "新的资料🦀");
                if !same_tool {
                    tools.select(ToolKind::Json);
                    assert_eq!((tools.input.clone(), tools.output.clone()), old_source);
                    assert!(
                        tools.has_result(),
                        "unrelated source draft remains completed"
                    );
                }
                tools.select(ToolKind::Base64);
                let result = tools.run(0);
                assert!(tools.apply_result(result));
                assert_eq!(tools.output, "5paw55qE6LWE5paZ8J+mgA==");
                assert!(tools.has_result());
            }
        }
    }
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
