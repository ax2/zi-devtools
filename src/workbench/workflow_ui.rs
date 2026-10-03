use super::{workflow::*, *};

pub(super) struct State {
    definition: Definition,
    proposal: Option<Preview>,
    source: Option<Dataset>,
    receiver: Option<Receiver<std::result::Result<Preview, String>>>,
    cancel: Arc<AtomicBool>,
    pub(super) job: Job,
    error: String,
    reveal: bool,
    scroll_until: Option<std::time::Instant>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            definition: Definition {
                version: 1,
                name: "表格清洗".into(),
                steps: vec![],
            },
            proposal: None,
            source: None,
            receiver: None,
            cancel: Arc::new(AtomicBool::new(false)),
            job: Job::default(),
            error: String::new(),
            reveal: false,
            scroll_until: None,
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl State {
    pub(super) fn has_content(&self) -> bool {
        !self.definition.steps.is_empty()
    }
    pub(super) fn invalidate(&mut self) {
        self.proposal = None;
        self.source = None;
        if self.job.phase.active() {
            self.cancel();
        }
    }
    pub(super) fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.job.cancelling();
    }
}

impl DataState {
    pub fn show_workflow(&mut self) {
        self.workflow.reveal = true;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_workflow(&mut self) {
        self.input = "编号,名称,数量\n001, Zi Tools ,2\n002, Local Notes ,3".into();
        self.dataset = Some(Dataset::parse(&self.input, DataFormat::Csv, b',').unwrap());
        self.workflow.definition.steps = vec![
            Step::Column {
                column: "名称".into(),
                operation: ColumnOperation::Trim,
                value: String::new(),
            },
            Step::Column {
                column: "数量".into(),
                operation: ColumnOperation::ToInteger,
                value: String::new(),
            },
        ];
        self.workflow.source = self.dataset.clone();
        self.workflow.proposal = Some(
            self.workflow
                .definition
                .preview(self.dataset.as_ref().unwrap(), &AtomicBool::new(false))
                .unwrap(),
        );
        self.show_workflow();
    }

    fn start_workflow(&mut self) -> Result<()> {
        ensure_not_busy(self)?;
        let input = self.dataset.as_ref().context("请先解析表格")?;
        self.workflow.definition.validate()?;
        let definition = self.workflow.definition.clone();
        let input = input.clone();
        self.workflow.source = Some(input.clone());
        self.workflow.proposal = None;
        self.workflow.error.clear();
        self.workflow.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.workflow.cancel.clone();
        let (tx, rx) = mpsc::channel();
        self.workflow.receiver = Some(rx);
        self.workflow.job.begin();
        std::thread::spawn(move || {
            let result = definition
                .preview(&input, &cancel)
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(result);
        });
        Ok(())
    }

    pub(super) fn poll_workflow(&mut self) {
        let reply = self
            .workflow
            .receiver
            .as_ref()
            .and_then(|rx| match rx.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("流程预览任务意外结束".into())),
            });
        let Some(reply) = reply else {
            return;
        };
        self.workflow.receiver = None;
        if self.workflow.cancel.load(Ordering::Relaxed) {
            self.workflow
                .job
                .finish(Phase::Cancelled, "预览已取消，来源保留");
            self.workflow.invalidate();
            return;
        }
        if self.dataset != self.workflow.source {
            self.workflow
                .job
                .finish(Phase::Failed, "来源已变化，请重新预览");
            self.workflow.invalidate();
            self.workflow.error = "来源已变化，请重新预览".into();
            return;
        }
        match reply {
            Ok(preview) => {
                self.workflow.job.finish(
                    Phase::Done,
                    format!("{}步预览完成，尚未应用", preview.steps.len()),
                );
                self.workflow.proposal = Some(preview);
                self.workflow.reveal = true;
            }
            Err(error) => {
                self.workflow
                    .job
                    .finish(Phase::Failed, "预览失败，来源保留");
                self.workflow.error = error;
            }
        }
    }

    fn apply_workflow(&mut self) -> Result<()> {
        ensure_not_busy(self)?;
        anyhow::ensure!(
            self.dataset == self.workflow.source,
            "来源已变化，请重新预览"
        );
        let preview = self.workflow.proposal.take().context("请先完成流程预览")?;
        let count = preview.steps.len();
        self.replace_with_join(preview.result);
        self.message = format!("已应用{count}步流程，原始输入保留；可撤销最近一次");
        Ok(())
    }

    pub(super) fn workflow_ui(&mut self, ui: &mut egui::Ui) {
        let Some(data) = self.dataset.as_ref() else {
            return;
        };
        let headers = data.headers.clone();
        let schemas = step_schemas(&headers, &self.workflow.definition.steps);
        let active = self.workflow.job.phase.active();
        let mut changed = false;
        let mut start = false;
        let mut apply = false;
        if self.workflow.reveal {
            self.workflow.scroll_until =
                Some(std::time::Instant::now() + std::time::Duration::from_millis(500));
        }
        let scroll = self
            .workflow
            .scroll_until
            .is_some_and(|until| std::time::Instant::now() < until);
        let response = egui::CollapsingHeader::new("操作流程 · 连续预览")
            .id_salt("table-workflow")
            .open(scroll.then_some(true))
            .default_open(!self.workflow.definition.steps.is_empty())
            .show(ui, |ui| {
                ui.label("按顺序处理上次解析的全部行，包含筛选隐藏的行；先预览，再主动应用。");
                ui.small(
                    "开发中：步骤仅本次运行保留，不包含在工作实例保存中。文件导出仍需另行确认。",
                );
                ui.add_enabled_ui(!active, |ui| {
                    changed |= ui
                        .text_edit_singleline(&mut self.workflow.definition.name)
                        .changed();
                    let mut movement = None;
                    let mut remove = None;
                    egui::ScrollArea::vertical()
                        .id_salt("workflow-step-list")
                        .auto_shrink([false, false])
                        .min_scrolled_height(if self.workflow.definition.steps.is_empty() {
                            20.0
                        } else {
                            180.0
                        })
                        .max_height(240.0)
                        .show(ui, |ui| {
                            let len = self.workflow.definition.steps.len();
                            for (index, step) in
                                self.workflow.definition.steps.iter_mut().enumerate()
                            {
                                let available = &schemas[index];
                                ui.push_id(index, |ui| {
                                    ui.separator();
                                    ui.horizontal_wrapped(|ui| {
                                        ui.strong(format!("第{}步", index + 1));
                                        if ui
                                            .add_enabled(index > 0, egui::Button::new("上移"))
                                            .clicked()
                                        {
                                            movement = Some((index, index - 1));
                                        }
                                        if ui
                                            .add_enabled(index + 1 < len, egui::Button::new("下移"))
                                            .clicked()
                                        {
                                            movement = Some((index, index + 1));
                                        }
                                        if ui.button("移除").clicked() {
                                            remove = Some(index);
                                        }
                                    });
                                    match step {
                                        Step::Column {
                                            column,
                                            operation,
                                            value,
                                        } => {
                                            ui.horizontal_wrapped(|ui| {
                                                ui.label("列名");
                                                changed |= ui
                                                    .add(
                                                        egui::TextEdit::singleline(column)
                                                            .char_limit(256)
                                                            .desired_width(150.0),
                                                    )
                                                    .changed();
                                                egui::ComboBox::from_id_salt("column-picker")
                                                    .selected_text("选择列")
                                                    .show_ui(ui, |ui| {
                                                        for name in available {
                                                            changed |= ui
                                                                .selectable_value(
                                                                    column,
                                                                    name.clone(),
                                                                    name,
                                                                )
                                                                .changed();
                                                        }
                                                    });
                                                egui::ComboBox::from_id_salt("operation")
                                                    .selected_text(label(*operation))
                                                    .show_ui(ui, |ui| {
                                                        for op in OPERATIONS {
                                                            changed |= ui
                                                                .selectable_value(
                                                                    operation,
                                                                    op,
                                                                    label(op),
                                                                )
                                                                .changed();
                                                        }
                                                    });
                                                if matches!(
                                                    operation,
                                                    ColumnOperation::FillNull
                                                        | ColumnOperation::Rename
                                                ) {
                                                    changed |= ui
                                                        .add(
                                                            egui::TextEdit::singleline(value)
                                                                .char_limit(4096)
                                                                .hint_text("新列名或填充文本")
                                                                .desired_width(180.0),
                                                        )
                                                        .changed();
                                                }
                                            });
                                        }
                                        Step::SelectColumns { columns } => {
                                            ui.label("选择保留列（沿用原列顺序）");
                                            ui.horizontal_wrapped(|ui| {
                                                for name in available {
                                                    let mut selected = columns.contains(name);
                                                    if ui.checkbox(&mut selected, name).changed() {
                                                        if selected {
                                                            columns.push(name.clone());
                                                        } else {
                                                            columns.retain(|c| c != name);
                                                        }
                                                        changed = true;
                                                    }
                                                }
                                            });
                                            let missing: Vec<_> = columns
                                                .iter()
                                                .filter(|c| !available.contains(c))
                                                .collect();
                                            if !missing.is_empty() {
                                                ui.colored_label(
                                                    ui.visuals().error_fg_color,
                                                    format!("当前表缺少：{missing:?}"),
                                                );
                                            }
                                        }
                                    }
                                });
                            }
                        });
                    if let Some(index) = remove {
                        self.workflow.definition.steps.remove(index);
                        changed = true;
                    } else if let Some((from, to)) = movement {
                        self.workflow.definition.steps.swap(from, to);
                        changed = true;
                    }
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(
                                self.workflow.definition.steps.len() < 32,
                                egui::Button::new("添加列操作"),
                            )
                            .clicked()
                        {
                            self.workflow.definition.steps.push(Step::Column {
                                column: schemas
                                    .last()
                                    .and_then(|s| s.first())
                                    .cloned()
                                    .unwrap_or_default(),
                                operation: ColumnOperation::Trim,
                                value: String::new(),
                            });
                            changed = true;
                        }
                        if ui
                            .add_enabled(
                                self.workflow.definition.steps.len() < 32,
                                egui::Button::new("添加选列步骤"),
                            )
                            .clicked()
                        {
                            self.workflow.definition.steps.push(Step::SelectColumns {
                                columns: schemas.last().cloned().unwrap_or_default(),
                            });
                            changed = true;
                        }
                    });
                });
                if changed {
                    self.workflow.invalidate();
                    self.workflow.error.clear();
                }
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    start = ui
                        .add_enabled(
                            !self.busy() && !self.workflow.definition.steps.is_empty(),
                            primary(ui, "预览全部步骤"),
                        )
                        .clicked();
                    if ui
                        .add_enabled(active, egui::Button::new("取消预览"))
                        .clicked()
                    {
                        self.workflow.cancel();
                    }
                    if active {
                        ui.spinner();
                        ui.label(self.workflow.job.phase.label());
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(30));
                    }
                    apply = ui
                        .add_enabled(
                            !self.busy() && self.workflow.proposal.is_some(),
                            primary(ui, "应用流程结果"),
                        )
                        .clicked();
                });
                if !self.workflow.error.is_empty() {
                    ui.colored_label(ui.visuals().error_fg_color, &self.workflow.error);
                }
                if let Some(preview) = &self.workflow.proposal {
                    ui.strong(format!(
                        "预览完成：{}行 · {}列；尚未应用",
                        preview.result.rows.len(),
                        preview.result.headers.len()
                    ));
                    for report in &preview.steps {
                        egui::CollapsingHeader::new(format!(
                            "第{}步 · {} · 变化{}处",
                            report.step, report.description, report.changed
                        ))
                        .id_salt(("report", report.step))
                        .show(ui, |ui| {
                            ui.label(format!(
                                "{}行 / {}列；最多6处变化样本",
                                report.rows, report.columns
                            ));
                            for (before, after) in &report.examples {
                                ui.label(format!("{before} → {after}"));
                            }
                        });
                    }
                }
            });
        self.workflow.reveal = false;
        if scroll {
            if let Some(body) = response.body_response {
                ui.scroll_to_rect(body.rect, Some(egui::Align::Max));
            }
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
        if start && let Err(error) = self.start_workflow() {
            self.workflow.error = format!("{error:#}");
        }
        if apply && let Err(error) = self.apply_workflow() {
            self.workflow.error = format!("{error:#}");
        }
    }
}

/// Predict only valid schema changes; execution still performs full validation.
fn step_schemas(headers: &[String], steps: &[Step]) -> Vec<Vec<String>> {
    let mut current = headers.to_vec();
    let mut schemas = vec![current.clone()];
    for step in steps {
        match step {
            Step::Column {
                column,
                operation: ColumnOperation::Rename,
                value,
            } => {
                let name = value.trim();
                if !name.is_empty()
                    && name.len() <= 256
                    && let Some(index) = current.iter().position(|c| c == column)
                    && !current
                        .iter()
                        .enumerate()
                        .any(|(i, c)| i != index && c == name)
                {
                    current[index] = name.into();
                }
            }
            Step::SelectColumns { columns } if !columns.is_empty() => {
                current.retain(|c| columns.contains(c));
            }
            _ => {}
        }
        schemas.push(current.clone());
    }
    schemas
}

fn ensure_not_busy(state: &DataState) -> Result<()> {
    anyhow::ensure!(!state.busy(), "请等待当前实例任务结束");
    Ok(())
}
fn label(operation: ColumnOperation) -> &'static str {
    transform::Operation::from(operation).label()
}
const OPERATIONS: [ColumnOperation; 10] = [
    ColumnOperation::Trim,
    ColumnOperation::Lower,
    ColumnOperation::Upper,
    ColumnOperation::EmptyToNull,
    ColumnOperation::FillNull,
    ColumnOperation::Rename,
    ColumnOperation::ToText,
    ColumnOperation::ToInteger,
    ColumnOperation::ToNumber,
    ColumnOperation::ToBool,
];

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> DataState {
        let mut state = DataState {
            input: "编号,数量\n001,2\n002,3".into(),
            ..Default::default()
        };
        state.dataset = Some(Dataset::parse(&state.input, DataFormat::Csv, b',').unwrap());
        state.workflow.definition.steps.push(Step::Column {
            column: "数量".into(),
            operation: ColumnOperation::ToInteger,
            value: String::new(),
        });
        state
    }
    fn wait(state: &mut DataState) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while state.workflow.receiver.is_some() {
            state.poll_workflow();
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
    #[test]
    fn preview_apply_undo_preserve_input_and_require_explicit_application() {
        let mut state = fixture();
        let original = state.dataset.clone();
        state.start_workflow().unwrap();
        assert!(state.busy());
        assert!(state.snapshot().is_err());
        wait(&mut state);
        assert_eq!(state.dataset, original);
        assert!(state.workflow.proposal.is_some());
        state.apply_workflow().unwrap();
        assert_eq!(
            state.dataset.as_ref().unwrap().rows[0][1],
            serde_json::json!(2)
        );
        state.undo_transform();
        assert_eq!(state.dataset, original);
        assert!(state.input.contains("001"));
    }
    #[test]
    fn source_change_and_cancel_discard_late_results_without_applying() {
        let mut state = fixture();
        state.start_workflow().unwrap();
        state.dataset.as_mut().unwrap().rows[0][0] = serde_json::json!("new");
        wait(&mut state);
        assert!(state.workflow.proposal.is_none());
        assert_eq!(state.workflow.job.phase, Phase::Failed);
        state.start_workflow().unwrap();
        state.workflow.cancel();
        wait(&mut state);
        assert_eq!(state.workflow.job.phase, Phase::Cancelled);
        assert!(state.workflow.proposal.is_none());
        assert_eq!(state.dataset.as_ref().unwrap().rows[0][0], "new");
    }
    #[test]
    fn editor_schema_follows_renames_and_projection_without_running_data() {
        let steps = vec![
            Step::Column {
                column: "old".into(),
                operation: ColumnOperation::Rename,
                value: " new ".into(),
            },
            Step::SelectColumns {
                columns: vec!["new".into()],
            },
        ];
        assert_eq!(
            step_schemas(&["old".into(), "other".into()], &steps),
            vec![vec!["old", "other"], vec!["new", "other"], vec!["new"]]
        );
    }
    #[test]
    fn inactive_instance_jobs_keep_identity_and_reject_stale_cancellation() {
        let mut workspace = sessions::Workspace::new(
            std::env::temp_dir().join(format!("zi-workflow-{}.sqlite3", uuid::Uuid::new_v4())),
        );
        let first = workspace.create("first").unwrap();
        *workspace = fixture();
        workspace.start_workflow().unwrap();
        let generation = workspace
            .snapshots()
            .iter()
            .find(|r| r.key == "pipeline")
            .unwrap()
            .generation;
        workspace.create("second").unwrap();
        *workspace = fixture();
        workspace.cancel_workflow(&first, generation + 1);
        workspace.select(&first).unwrap();
        assert!(!workspace.workflow.cancel.load(Ordering::Relaxed));
        workspace.cancel_workflow(&first, generation);
        wait(&mut workspace);
        assert_eq!(workspace.workflow.job.phase, Phase::Cancelled);
        assert!(workspace.workflow.proposal.is_none());
    }
}
