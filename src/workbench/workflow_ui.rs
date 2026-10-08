use super::{workflow::*, *};

mod inspector;
#[cfg(feature = "ui-preview")]
mod preview;
mod row_editor;

pub(super) mod saved {
    use super::*;
    pub(crate) fn serialize<S: serde::Serializer>(
        state: &State,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        state.definition.serialize(serializer)
    }
    pub(crate) fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<State, D::Error> {
        let definition = Definition::deserialize(deserializer)?;
        definition
            .validate_draft()
            .map_err(serde::de::Error::custom)?;
        let mut state = State::default();
        state.output.load_settings(definition.output.as_ref());
        state.definition = definition;
        state.validate_saved().map_err(serde::de::Error::custom)?;
        Ok(state)
    }
}

pub(super) struct State {
    definition: Definition,
    proposal: Option<Preview>,
    source: Option<Dataset>,
    receiver: Option<Receiver<std::result::Result<Preview, String>>>,
    cancel: Arc<AtomicBool>,
    pub(super) job: Job,
    pub(super) files: workflow_files::State,
    error: String,
    reveal: bool,
    scroll_until: Option<std::time::Instant>,
    inspect: inspector::State,
    pub(super) output: super::workflow_output::State,
    #[cfg(feature = "ui-preview")]
    buttons: [Option<(egui::Rect, egui::Rect)>; 10],
}
impl Default for State {
    fn default() -> Self {
        Self {
            definition: Definition {
                output: None,
                version: 2,
                name: "表格清洗".into(),
                steps: vec![],
            },
            proposal: None,
            source: None,
            receiver: None,
            cancel: Arc::new(AtomicBool::new(false)),
            job: Job::default(),
            files: workflow_files::State::default(),
            error: String::new(),
            reveal: false,
            scroll_until: None,
            inspect: inspector::State::default(),
            output: super::workflow_output::State::default(),
            #[cfg(feature = "ui-preview")]
            buttons: [None; 10],
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl State {
    pub(super) fn validate_saved(&self) -> Result<()> {
        self.definition.validate_draft()?;
        anyhow::ensure!(
            serde_json::to_vec(&self.definition)?.len() <= MAX_DEFINITION_BYTES,
            "流程定义最多256 KiB"
        );
        Ok(())
    }
    pub(super) fn modal_open(&self) -> bool {
        self.files.pending_review() || self.inspect.open || self.output.modal_open()
    }
    pub(super) fn has_content(&self) -> bool {
        !self.definition.steps.is_empty() || self.files.pending_review()
    }
    pub(super) fn invalidate(&mut self) {
        self.output.invalidate();
        self.inspect = inspector::State::default();
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
    #[cfg(feature = "ui-preview")]
    pub fn preview_flow_snapshot_check(&self, ran: bool) {
        assert_eq!(self.workflow.definition.steps.len(), 2);
        assert!(self.input.contains("001, Zi Tools ,2"));
        assert!(!self.busy() && !self.dialog_pending());
        if ran {
            assert_eq!(self.workflow.job.phase, Phase::Done);
            assert_eq!(
                self.workflow.proposal.as_ref().unwrap().result.rows[0][2],
                serde_json::json!(2)
            );
            assert_eq!(
                self.dataset.as_ref().unwrap().rows[0][2],
                serde_json::json!("2")
            );
        } else {
            assert_eq!(self.workflow.job.phase, Phase::Idle);
            assert!(self.workflow.proposal.is_none());
        }
    }
    pub fn show_workflow_output(&mut self) {
        self.show_workflow();
        self.workflow.output.reveal = true;
    }
    pub(super) fn open_bookmarked_workflow(&mut self, path: PathBuf) -> Result<()> {
        ensure_not_busy(self)?;
        self.workflow.files.read(path)?;
        Ok(())
    }
    pub(super) fn open_searched_workflow(
        &mut self,
        file: &std::ffi::OsStr,
        revision: u64,
    ) -> Result<()> {
        ensure_not_busy(self)?;
        anyhow::ensure!(
            self.workflow.files.listing_revision == revision,
            "流程列表已变化，请重新搜索"
        );
        let path = self
            .workflow
            .files
            .listing
            .as_ref()
            .and_then(|listing| {
                listing
                    .entries
                    .iter()
                    .find(|entry| entry.path.file_name() == Some(file))
            })
            .context("流程列表已变化，请刷新目录并重新搜索")?
            .path
            .clone();
        self.workflow.files.read(path)?;
        Ok(())
    }
    pub fn workflow_folder_settings(
        &mut self,
        prefs: &mut crate::preferences::Preferences,
        path: &std::path::Path,
    ) {
        let files = &mut self.workflow.files;
        files.remembered_folder = prefs.workflow_library_folder.clone();
        if !files.folder_hydrated {
            files.folder_hydrated = true;
            if files.folder.is_none() {
                files.folder = files.remembered_folder.clone();
                if files.folder.is_some() {
                    files.memory_message =
                        "已恢复记住的目录位置；点击刷新列表后才读取流程文件。".into();
                }
            }
        }
        if let Some(folder) = files.folder_request.take() {
            let forgetting = folder.is_none();
            match prefs.save_workflow_folder(path, folder) {
                Ok(()) => {
                    files.remembered_folder = prefs.workflow_library_folder.clone();
                    files.memory_message = if forgetting {
                        "已忘记目录位置；文件和当前实例内容保留。"
                    } else {
                        "已记住此目录位置；下次点击刷新列表后才读取文件，不自动运行。"
                    }
                    .into();
                }
                Err(error) => {
                    files.memory_message =
                        format!("目录记忆未保存：{error:#}；当前选择保留，可重试。");
                }
            }
        }
    }
    pub fn show_workflow(&mut self) {
        self.set_active_tool("pipeline");
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

    #[cfg(feature = "ui-preview")]
    pub fn preview_workflow_import(&mut self) {
        self.preview_workflow();
        let mut imported = self.workflow.definition.clone();
        imported.name = "每日资料整理".into();
        imported.steps.push(Step::SelectColumns {
            columns: vec!["编号".into(), "名称".into(), "数量".into()],
        });
        self.workflow.files.review = Some(imported);
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_workflow_library(&mut self, fixture: &std::path::Path) {
        self.preview_workflow();
        let folder = fixture.parent().unwrap().join("workflow-library-fixture");
        std::fs::create_dir_all(&folder).unwrap();
        let mut definition = self.workflow.definition.clone();
        for (file, name) in [
            ("daily.json", "每日资料清洗"),
            ("orders.json", "订单列类型转换"),
        ] {
            definition.name = name.into();
            std::fs::write(
                folder.join(file),
                serde_json::to_vec_pretty(&definition).unwrap(),
            )
            .unwrap();
        }
        self.workflow.files.list(folder).unwrap();
    }

    pub(super) fn start_workflow(&mut self) -> Result<()> {
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
        self.workflow.output.poll();
        self.workflow.files.poll();
        if self.workflow.files.loaded_tool == Some("pipeline")
            || self.workflow.files.image_review.is_some()
        {
            self.show_workflow();
        }
        if !self.text_flow.busy()
            && !self.text_flow.modal_open()
            && let Some(definition) = self.workflow.files.tool_review.take()
        {
            if let Err(e) = self.text_flow.receive_recipe(definition) {
                self.workflow.error = e.to_string();
            } else {
                self.workflow.reveal = false;
                self.set_active_tool("text-flow");
            }
        }

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
        anyhow::ensure!(self.dataset.is_some(), "请先解析表格");
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
        let has_input = self.dataset.is_some();
        let headers = self
            .dataset
            .as_ref()
            .map(|data| data.headers.clone())
            .unwrap_or_default();
        let schemas = step_schemas(&headers, &self.workflow.definition.steps);
        let running = self.workflow.job.phase.active();
        let active = running
            || self.workflow.files.job.phase.active()
            || self.workflow.files.review.is_some();
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
            .is_some_and(|until| std::time::Instant::now() < until)
            && !self.workflow.output.wants_scroll();
        let response = egui::CollapsingHeader::new("操作流程 · 连续预览")
            .id_salt("table-workflow")
            .open(scroll.then_some(true))
            .default_open(!self.workflow.definition.steps.is_empty())
            .show(ui, |ui| {
                if has_input {
                    ui.label("按顺序处理上次解析的全部行，包含筛选隐藏的行；先预览，再主动应用。");
                } else {
                    ui.label("先读取已有流程或建立步骤，列名可手动填写。载入并解析表格后才能预览和应用。");
                }
                ui.small(
                    "可另存流程文件供下次使用；只含步骤与参数，不含原表或授权。实例快照不含流程。",
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
                        } else if !has_input {
                            240.0
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
                                                            .hint_text("填写列名")
                                                            .desired_width(150.0),
                                                    )
                                                    .changed();
                                                if !available.is_empty() { egui::ComboBox::from_id_salt("column-picker")
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
                                                    }); }
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
                                            if !has_input || columns.iter().any(|name| !available.contains(name)) {
                                                let mut remove_column = None;
                                                for (index, name) in columns.iter_mut().enumerate() {
                                                    if has_input && available.contains(name) { continue; }
                                                    ui.horizontal_wrapped(|ui| {
                                                        changed |= ui.add(egui::TextEdit::singleline(name).char_limit(256).hint_text("填写列名").desired_width(180.0)).changed();
                                                        if ui.button("移除列名").clicked() { remove_column = Some(index); }
                                                    });
                                                }
                                                if let Some(index) = remove_column { columns.remove(index); changed = true; }
                                                if !has_input && ui.add_enabled(columns.len() < 128, egui::Button::new("添加列名")).clicked() { columns.push(String::new()); changed = true; }
                                            }
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
                                            if has_input && !missing.is_empty() {
                                                ui.colored_label(
                                                    ui.visuals().error_fg_color,
                                                    format!("当前表缺少：{missing:?}"),
                                                );
                                            }
                                        }
                                        step => { changed |= row_editor::edit(ui, step, available); }
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
                        ui.add_enabled_ui(self.workflow.definition.steps.len() < 32, |ui| {
                            ui.menu_button("添加行步骤", |ui| {
                                let name = schemas.last().and_then(|s| s.first()).cloned().unwrap_or_default();
                                for (title, step) in [
                                    ("筛选行", Step::Filter { column: name.clone(), predicate: Predicate::Contains, value: String::new(), case_sensitive: false }),
                                    ("多列排序", Step::Sort { keys: vec![SortKey { column: name.clone(), descending: false }] }),
                                    ("按列去重", Step::Deduplicate { columns: vec![name] }),
                                ] {
                                    if ui.button(title).clicked() {
                                        self.workflow.definition.steps.push(step);
                                        self.workflow.definition.version = self.workflow.definition.version.max(2);
                                        changed = true;
                                        ui.close();
                                    }
                                }
                            });
                        });
                    });
                });
                if changed {
                    self.workflow.invalidate();
                    self.workflow.error.clear();
                }
                self.workflow_file_buttons(ui);
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    let preview = ui
                        .add_enabled(
                            has_input && !active && !self.busy() && !self.workflow.definition.steps.is_empty(),
                            primary(ui, "预览全部步骤"),
                        );
                    #[cfg(feature = "ui-preview")]
                    { self.workflow.buttons[2] = Some((preview.rect, ui.clip_rect())); }
                    start = preview.clicked();
                    if ui
                        .add_enabled(running, egui::Button::new("取消预览"))
                        .clicked()
                    {
                        self.workflow.cancel();
                    }
                    if running {
                        ui.spinner();
                        ui.label(self.workflow.job.phase.label());
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(30));
                    }
                    let apply_button = ui
                        .add_enabled(
                            has_input && !active && !self.busy() && self.workflow.proposal.is_some(),
                            primary(ui, "应用流程结果"),
                        );
                    #[cfg(feature = "ui-preview")]
                    { self.workflow.buttons[3] = Some((apply_button.rect, ui.clip_rect())); }
                    apply = apply_button.clicked();
                    let undo = ui
                        .add_enabled(
                            !active && !self.busy() && self.can_undo_transform(),
                            egui::Button::new("撤销最近一次表格修改"),
                        );
                    #[cfg(feature = "ui-preview")]
                    { self.workflow.buttons[4] = Some((undo.rect, ui.clip_rect())); }
                    if undo.clicked() {
                        self.undo_transform();
                    }
                });
                if !self.workflow.error.is_empty() {
                    ui.colored_label(ui.visuals().error_fg_color, &self.workflow.error);
                }
                if let Some(preview) = &self.workflow.proposal {
                    let save = ui.add_enabled(!active && !self.busy(), egui::Button::new("保存流程结果为文件…"));
                    #[cfg(feature = "ui-preview")]
                    { self.workflow.buttons[6] = Some((save.rect, ui.clip_rect())); }
                    if save.clicked() {self.workflow.output.reveal=true;}
                    let inspect = ui.add_enabled(!active && !self.busy(), egui::Button::new("查看输入与结果表格…"));
                    #[cfg(feature = "ui-preview")]
                    { self.workflow.buttons[5] = Some((inspect.rect, ui.clip_rect())); }
                    if inspect.clicked() {
                        self.workflow.inspect.open(&preview.result);
                    }
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
        self.workflow_import_modal(ui);
        self.workflow_inspector(ui);
        if scroll {
            if let Some(body) = response.body_response {
                ui.scroll_to_rect(body.rect, Some(egui::Align::Max));
            }
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
        self.workflow.output.ui(
            ui,
            if self.dataset == self.workflow.source {
                self.dataset.as_ref()
            } else {
                None
            },
            &mut self.workflow.definition,
            self.workflow.proposal.as_ref().map(|p| &p.result),
        );
        if start && let Err(error) = self.start_workflow() {
            self.workflow.error = format!("{error:#}");
        }
        if apply && let Err(error) = self.apply_workflow() {
            self.workflow.error = format!("{error:#}");
        }
    }
}

impl DataState {
    fn workflow_file_buttons(&mut self, ui: &mut egui::Ui) {
        #[cfg(feature = "ui-preview")]
        {
            self.workflow.files.memory_buttons = [None, None];
        }
        ui.horizontal_wrapped(|ui| {
            let allowed = !self.busy()
                && !self.workflow.files.pending_review()
                && !self.text_flow.modal_open();
            if ui
                .add_enabled(
                    allowed && !self.workflow.definition.steps.is_empty(),
                    egui::Button::new("保存流程…"),
                )
                .clicked()
            {
                match self.workflow.definition.validate() {
                    Err(error) => self.workflow.error = format!("{error:#}"),
                    Ok(()) => {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_title("另存新的流程文件")
                            .add_filter("流程 JSON", &["json"])
                            .set_file_name("表格流程.json")
                            .save_file()
                            && let Err(error) = self
                                .workflow
                                .files
                                .save(self.workflow.definition.clone(), path)
                        {
                            self.workflow.error = format!("{error:#}");
                        }
                    }
                }
            }
            if ui
                .add_enabled(allowed, egui::Button::new("使用已有流程…"))
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_title("选择流程文件，预览后确认")
                    .add_filter("流程 JSON", &["json"])
                    .pick_file()
                && let Err(error) = self.workflow.files.read(path)
            {
                self.workflow.error = format!("{error:#}");
            }
            if self.workflow.files.job.phase.active() {
                ui.spinner();
                ui.label("流程文件处理中…");
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(30));
            }
        });
        self.workflow_library_ui(ui);
    }
    pub(super) fn workflow_library_ui(&mut self, ui: &mut egui::Ui) {
        if self.workflow.files.image_review.is_some() {
            ui.group(|ui| {
                ui.label("图片流程等待接收：请完成或取消图片页当前任务和导入；输入与结果保留。");
                if ui.button("取消等待图片流程").clicked() {
                    self.workflow.files.image_review = None;
                    self.workflow.files.message = "已取消接收，原工作保留".into();
                }
            });
        }

        let allowed =
            !self.busy() && !self.workflow.files.pending_review() && !self.text_flow.modal_open();
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(allowed, egui::Button::new("流程文件夹…"))
                .clicked()
                && let Some(folder) = rfd::FileDialog::new()
                    .set_title("选择流程文件夹，只检查直接 JSON 文件")
                    .pick_folder()
                && let Err(error) = self.workflow.files.list(folder)
            {
                self.workflow.error = format!("{error:#}");
            }
        });
        if let Some(folder) = self.workflow.files.folder.clone() {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!(
                    "流程文件夹：{}",
                    folder.file_name().unwrap_or_default().to_string_lossy()
                ))
                .on_hover_text(folder.display().to_string());
                if ui
                    .add_enabled(
                        !self.busy()
                            && !self.workflow.files.pending_review()
                            && !self.text_flow.modal_open(),
                        egui::Button::new("刷新列表"),
                    )
                    .clicked()
                    && let Err(error) = self.workflow.files.list(folder.clone())
                {
                    self.workflow.error = format!("{error:#}");
                }
                if self.workflow.files.remembered_folder.as_ref() == Some(&folder) {
                    ui.weak("已记住此目录");
                } else {
                    let remember = ui
                        .button("记住此目录")
                        .on_hover_text("仅保存目录位置；不保存正文，不自动扫描或运行");
                    #[cfg(feature = "ui-preview")]
                    {
                        self.workflow.files.memory_buttons[0] =
                            Some(remember.rect.intersect(ui.clip_rect()));
                    }
                    if remember.clicked() {
                        self.workflow.files.folder_request = Some(Some(folder.clone()));
                    }
                }
                if let Some(remembered) = self
                    .workflow
                    .files
                    .remembered_folder
                    .as_ref()
                    .filter(|saved| *saved != &folder)
                {
                    ui.weak(format!(
                        "已记住：{}",
                        remembered
                            .file_name()
                            .unwrap_or_else(|| remembered.as_os_str())
                            .to_string_lossy()
                    ))
                    .on_hover_text(remembered.display().to_string());
                }
                if self.workflow.files.remembered_folder.is_some() {
                    let forget = ui
                        .button("忘记已记住目录")
                        .on_hover_text("只清除记忆设置，不删除文件或当前实例内容");
                    #[cfg(feature = "ui-preview")]
                    {
                        self.workflow.files.memory_buttons[1] =
                            Some(forget.rect.intersect(ui.clip_rect()));
                    }
                    if forget.clicked() {
                        self.workflow.files.folder_request = Some(None);
                    }
                }
            });
        }
        if !self.workflow.files.memory_message.is_empty() {
            ui.label(&self.workflow.files.memory_message);
        }
        let mut selected = None;
        let allowed =
            !self.busy() && !self.workflow.files.pending_review() && !self.text_flow.modal_open();
        if let Some(listing) = &self.workflow.files.listing {
            ui.add(
                egui::TextEdit::singleline(&mut self.workflow.files.query)
                    .hint_text("搜索流程名称或文件名…"),
            );
            let visible = listing
                .entries
                .iter()
                .filter(|entry| entry.matches(&self.workflow.files.query))
                .count();
            ui.label(format!(
                "匹配 {visible} / {} 个流程 · 跳过 {} 个无效或无法读取的 JSON 文件",
                listing.entries.len(),
                listing.skipped
            ));
            if listing.partial {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    "仅显示部分列表：达到枚举、读取或条目上限，请缩小文件夹范围。",
                );
            }
            egui::ScrollArea::vertical()
                .id_salt("workflow-library")
                .max_height(180.0)
                .show(ui, |ui| {
                    for entry in listing
                        .entries
                        .iter()
                        .filter(|entry| entry.matches(&self.workflow.files.query))
                    {
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .add_enabled(allowed, egui::Button::new("载入并确认…"))
                                .clicked()
                            {
                                selected = Some(entry.path.clone());
                            }
                            ui.label(format!("{} · {} 步", entry.name, entry.steps));
                            ui.weak(entry.path.file_name().unwrap_or_default().to_string_lossy());
                        });
                    }
                    if visible == 0 {
                        ui.label(
                            "没有匹配的流程；可调整搜索词、选择其他文件夹或保存新的流程文件。",
                        );
                    }
                });
            ui.weak("仅列出当前文件夹，不递归；载入时重新检查文件，不自动执行。点击记住目录可跨次恢复位置。");
        }
        if let Some(path) = selected
            && let Err(error) = self.workflow.files.read(path)
        {
            self.workflow.error = format!("{error:#}");
        }
        if !self.workflow.files.message.is_empty() {
            ui.label(&self.workflow.files.message);
        }
    }
    fn workflow_import_modal(&mut self, ui: &mut egui::Ui) {
        let Some(imported) = self.workflow.files.review.clone() else {
            return;
        };
        let mut confirm = false;
        let mut cancel = false;
        let response = egui::Modal::new(ui.id().with("workflow-import")).show(ui.ctx(), |ui| {
            ui.set_max_width((ui.ctx().screen_rect().width() - 48.0).clamp(180.0, 620.0));
            ui.heading("确认使用流程");
            ui.label(format!(
                "{} · {}步，将替换当前{}步",
                imported.name,
                imported.steps.len(),
                self.workflow.definition.steps.len()
            ));
            ui.label("替换步骤与输出设置；当前表格与原始输入保留，不自动预览、运行、保存或授权。");
            if let Some(output) = &imported.output {
                ui.label(format!(
                    "保存的输出：{}；目标文件需重新选择并确认",
                    output.summary()
                ));
            }
            egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| {
                    for (index, step) in imported.steps.iter().enumerate() {
                        ui.separator();
                        ui.strong(format!("第{}步", index + 1));
                        match step {
                            Step::Column {
                                column,
                                operation,
                                value,
                            } => {
                                ui.label(format!("{} · {}", column, label(*operation)));
                                if matches!(
                                    operation,
                                    ColumnOperation::Rename | ColumnOperation::FillNull
                                ) {
                                    ui.label(format!("参数：{value}"));
                                }
                            }
                            Step::SelectColumns { columns } => {
                                ui.label(format!("保留列：{}", columns.join("、")));
                            }
                            step => row_editor::review(ui, step),
                        }
                    }
                });
            ui.horizontal_wrapped(|ui| {
                let cancel_button = ui.button("取消，保留当前步骤");
                let confirm_button =
                    ui.add_enabled(!self.busy(), egui::Button::new("确认替换流程步骤"));
                #[cfg(feature = "ui-preview")]
                {
                    self.workflow.buttons[0] = Some((cancel_button.rect, ui.clip_rect()));
                    self.workflow.buttons[1] = Some((confirm_button.rect, ui.clip_rect()));
                }
                cancel = cancel_button.clicked();
                confirm = confirm_button.clicked();
            });
        });
        if confirm {
            if let Err(error) = self.confirm_workflow_import() {
                self.workflow.error = format!("{error:#}");
            }
        } else if cancel || response.should_close() {
            self.workflow.files.review = None;
        }
    }
    fn confirm_workflow_import(&mut self) -> Result<()> {
        anyhow::ensure!(!self.busy(), "请等待当前实例任务结束再替换流程");
        if let Some(definition) = &self.workflow.files.review {
            definition.validate()?;
        }
        if let Some(definition) = self.workflow.files.review.take() {
            self.workflow.invalidate();
            self.workflow
                .output
                .load_settings(definition.output.as_ref());
            self.workflow.definition = definition;
            self.workflow.error.clear();
            self.workflow.files.message = "流程步骤已载入；尚未预览或应用".into();
            self.workflow.reveal = true;
        }
        Ok(())
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
            Step::SelectColumns { columns }
                if !columns.is_empty()
                    && columns.iter().all(|column| current.contains(column))
                    && columns
                        .iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        == columns.len() =>
            {
                current.retain(|c| columns.contains(c));
            }
            _ => {}
        }
        schemas.push(current.clone());
    }
    schemas
}

fn ensure_not_busy(state: &DataState) -> Result<()> {
    anyhow::ensure!(
        !state.workflow.inspect.open,
        "请先关闭只读结果检查再运行或应用"
    );
    anyhow::ensure!(!state.busy(), "请等待当前实例任务结束");
    anyhow::ensure!(
        !state.workflow.files.pending_review() && !state.text_flow.modal_open(),
        "请先确认或取消已读取的流程"
    );
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
    #[test]
    fn workspace_snapshot_restores_pipeline_definition_without_results_or_authority() {
        let mut state = fixture();
        state.workflow.definition.version = 3;
        state.workflow.definition.output = Some(Output::Sqlite {
            version: 1,
            table: "result_rows".into(),
        });
        state.workflow.files.folder = Some(PathBuf::from("nonexistent-private-runtime-folder"));
        state.start_workflow().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while state.busy() {
            assert!(std::time::Instant::now() < deadline);
            state.poll();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(state.workflow.proposal.is_some());
        let snapshot = state.snapshot().unwrap();
        let encoded: serde_json::Value = serde_json::from_slice(&snapshot).unwrap();
        assert_eq!(
            encoded["workflow"],
            serde_json::to_value(&state.workflow.definition).unwrap()
        );
        assert!(!String::from_utf8_lossy(&snapshot).contains("nonexistent-private-runtime-folder"));
        let mut restored = DataState::restore(&snapshot).unwrap();
        assert_eq!(restored.workflow.definition, state.workflow.definition);
        assert_eq!(restored.dataset, state.dataset);
        assert!(restored.workflow.proposal.is_none() && restored.workflow.source.is_none());
        assert!(restored.workflow.files.folder.is_none() && !restored.workflow.modal_open());
        assert!(!restored.busy());
        assert_eq!(restored.workflow.job.phase, Phase::Idle);
        restored.show_workflow();
        restored.run_primary().unwrap();
        while restored.busy() {
            assert!(std::time::Instant::now() < deadline);
            restored.poll();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(
            restored.workflow.proposal.as_ref().unwrap().result.rows[0][1],
            serde_json::json!(2)
        );
        assert_eq!(restored.dataset, state.dataset);
    }
    #[test]
    fn old_snapshots_default_empty_pipeline_and_new_snapshots_reject_extra_authority() {
        let state = fixture();
        let mut encoded = serde_json::to_value(&state).unwrap();
        encoded.as_object_mut().unwrap().remove("workflow");
        let restored = DataState::restore(&serde_json::to_vec(&encoded).unwrap()).unwrap();
        assert!(restored.workflow.definition.steps.is_empty());
        assert_eq!(restored.dataset, state.dataset);
        let empty = DataState::default();
        assert!(DataState::restore(&empty.snapshot().unwrap()).is_ok());
        assert!(empty.workflow.definition.validate().is_err());
        let mut encoded = serde_json::to_value(&state).unwrap();
        encoded["workflow"]["command"] = serde_json::json!("unexpected command");
        assert!(DataState::restore(&serde_json::to_vec(&encoded).unwrap()).is_err());
        encoded["workflow"]
            .as_object_mut()
            .unwrap()
            .remove("command");
        encoded["workflow"]["version"] = serde_json::json!(99);
        assert!(DataState::restore(&serde_json::to_vec(&encoded).unwrap()).is_err());
    }
    #[test]
    fn primary_pipeline_dispatch_previews_without_applying_source() {
        let mut state = fixture();
        let source = state.dataset.clone();
        state.show_workflow();
        state.run_primary().unwrap();
        assert!(state.run_primary().is_err());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while state.busy() {
            assert!(std::time::Instant::now() < deadline);
            state.poll();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(state.dataset, source);
        assert!(state.workflow.proposal.is_some());
        assert_eq!(state.workflow.job.phase, Phase::Done);
    }
    use super::*;
    #[test]
    fn remembered_folder_restores_location_only_and_does_not_replace_instance_content() {
        let dir = std::env::temp_dir().join(format!("zi-folder-memory-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let config = dir.join("prefs.json");
        let folder = dir.join("deleted-directory");
        let alternate = dir.join("alternate");
        let mut prefs = crate::preferences::Preferences::default();
        prefs
            .save_workflow_folder(&config, Some(folder.clone()))
            .unwrap();
        let mut first = fixture();
        let original = first.dataset.clone();
        let steps = first.workflow.definition.clone();
        first.workflow_folder_settings(&mut prefs, &config);
        assert_eq!(first.workflow.files.folder, Some(folder.clone()));
        assert!(first.workflow.files.listing.is_none());
        assert!(!first.workflow.files.job.phase.active());
        assert!(first.workflow.files.review.is_none());
        first.workflow.files.folder = Some(alternate.clone());
        first.workflow_folder_settings(&mut prefs, &config);
        assert_eq!(first.workflow.files.folder, Some(alternate.clone()));
        first.workflow.files.folder_request = Some(Some(alternate.clone()));
        first.workflow_folder_settings(&mut prefs, &dir);
        assert_eq!(prefs.workflow_library_folder, Some(folder));
        assert!(first.workflow.files.memory_message.contains("未保存"));
        assert_eq!(first.workflow.files.folder, Some(alternate.clone()));
        first.workflow.files.folder_request = Some(Some(alternate.clone()));
        first.workflow_folder_settings(&mut prefs, &config);
        let mut second = DataState::default();
        second.workflow_folder_settings(&mut prefs, &config);
        assert_eq!(second.workflow.files.folder, Some(alternate.clone()));
        first.workflow.files.folder_request = Some(None);
        first.workflow_folder_settings(&mut prefs, &config);
        assert!(prefs.workflow_library_folder.is_none());
        assert_eq!(first.workflow.files.folder, Some(alternate.clone()));
        second.workflow_folder_settings(&mut prefs, &config);
        assert_eq!(second.workflow.files.folder, Some(alternate));
        assert_eq!(first.dataset, original);
        assert_eq!(first.workflow.definition, steps);
        assert!(second.workflow.files.listing.is_none());
        assert!(second.workflow.files.review.is_none());
        assert!(!second.busy());
        std::fs::remove_dir_all(dir).unwrap();
    }
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
    fn wait_files(state: &mut DataState) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while state.workflow.files.job.phase.active() {
            state.poll_workflow();
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
    #[test]
    fn inspection_blocks_application_and_clears_with_stale_preview() {
        let mut state = fixture();
        let input = state.input.clone();
        state.start_workflow().unwrap();
        wait(&mut state);
        let original = state.dataset.clone();
        state
            .workflow
            .inspect
            .open(&state.workflow.proposal.as_ref().unwrap().result);
        assert!(state.apply_workflow().is_err());
        assert!(state.start_workflow().is_err());
        assert_eq!(state.dataset, original);
        assert_eq!(state.input, input);
        state.workflow.inspect.open = false;
        state.apply_workflow().unwrap();
        assert!(!state.workflow.inspect.open);
        assert!(state.workflow.proposal.is_none());
        assert_eq!(state.input, input);
        state.undo_transform();
        assert_eq!(state.dataset, original);
        state.start_workflow().unwrap();
        wait(&mut state);
        state
            .workflow
            .inspect
            .open(&state.workflow.proposal.as_ref().unwrap().result);
        state.workflow.invalidate();
        assert!(!state.workflow.inspect.open);
        assert!(state.workflow.proposal.is_none());
    }
    #[test]
    fn definition_loads_without_input_and_survives_later_parsing() {
        let definition = fixture().workflow.definition.clone();
        let path =
            std::env::temp_dir().join(format!("zi-flow-empty-{}.json", uuid::Uuid::new_v4()));
        let mut state = DataState {
            input: "编号,数量\n001,2".into(),
            ..Default::default()
        };
        let input = state.input.clone();
        state
            .workflow
            .files
            .save(definition.clone(), path.clone())
            .unwrap();
        wait_files(&mut state);
        state.workflow.files.read(path.clone()).unwrap();
        wait_files(&mut state);
        assert!(state.dataset.is_none());
        state.confirm_workflow_import().unwrap();
        assert_eq!(state.workflow.definition, definition);
        assert_eq!(state.input, input);
        assert!(state.start_workflow().is_err());
        assert!(state.apply_workflow().is_err());
        assert!(!state.busy());
        state.format = DataFormat::Csv;
        state.parse();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while state.busy() {
            state.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(state.workflow.definition, definition);
        assert_eq!(state.input, input);
        assert_eq!(state.dataset.as_ref().unwrap().rows[0][1], "2");
        state.start_workflow().unwrap();
        wait(&mut state);
        assert_eq!(state.dataset.as_ref().unwrap().rows[0][1], "2");
        state.apply_workflow().unwrap();
        assert_eq!(
            state.dataset.as_ref().unwrap().rows[0][1],
            serde_json::json!(2)
        );
        assert_eq!(state.input, input);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn inactive_file_tasks_block_exit_and_pending_import_blocks_silent_close() {
        let definition = fixture().workflow.definition.clone();
        let path =
            std::env::temp_dir().join(format!("zi-flow-owner-{}.json", uuid::Uuid::new_v4()));
        let db =
            std::env::temp_dir().join(format!("zi-flow-owner-{}.sqlite3", uuid::Uuid::new_v4()));
        let mut workspace = sessions::Workspace::new(db.clone());
        let owner = workspace.create("读取流程").unwrap();
        workspace
            .workflow
            .files
            .save(definition.clone(), path.clone())
            .unwrap();
        assert!(workspace.has_active_tasks());
        assert!(workspace.close(&owner, true).is_err());
        wait_files(&mut workspace);
        workspace.workflow.files.read(path.clone()).unwrap();
        let other = workspace.create("其他编辑").unwrap();
        workspace.input = "保留其他实例输入".into();
        assert!(!workspace.busy());
        assert!(workspace.has_active_tasks());
        let row = workspace
            .snapshots()
            .into_iter()
            .find(|row| row.key == "workflow-file" && row.instance.as_deref() == Some(&owner))
            .unwrap();
        assert_eq!(row.instance_name.as_deref(), Some("读取流程"));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while workspace.has_active_tasks() {
            workspace.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(workspace.active_id(), other);
        assert_eq!(workspace.input, "保留其他实例输入");
        assert!(workspace.workflow.files.review.is_none());
        assert!(workspace.close(&owner, false).is_err());
        workspace.select(&owner).unwrap();
        assert!(workspace.input.is_empty());
        assert!(workspace.dataset.is_none());
        assert!(workspace.workflow.definition.steps.is_empty());
        assert_eq!(workspace.workflow.files.review, Some(definition.clone()));
        assert!(workspace.has_content());
        workspace.confirm_workflow_import().unwrap();
        assert_eq!(workspace.workflow.definition, definition);
        assert!(workspace.dataset.is_none());
        assert!(!db.exists(), "no automatic workspace persistence");
        workspace.close(&owner, true).unwrap();
        assert_eq!(workspace.input, "保留其他实例输入");
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn saved_flow_requires_import_confirmation_and_never_applies_to_source() {
        let mut state = fixture();
        let original = state.dataset.clone();
        let input = state.input.clone();
        let current = state.workflow.definition.clone();
        let mut imported = current.clone();
        imported.name = "重新使用的流程".into();
        imported.steps = vec![Step::Column {
            column: "编号".into(),
            operation: ColumnOperation::Rename,
            value: "id".into(),
        }];
        let path = std::env::temp_dir().join(format!("zi-flow-ui-{}.json", uuid::Uuid::new_v4()));
        state
            .workflow
            .files
            .save(imported.clone(), path.clone())
            .unwrap();
        assert!(state.busy());
        assert!(state.snapshot().is_err());
        wait_files(&mut state);
        assert_eq!(state.workflow.files.job.phase, Phase::Done);
        state.start_workflow().unwrap();
        wait(&mut state);
        state.workflow.files.read(path.clone()).unwrap();
        wait_files(&mut state);
        assert_eq!(state.workflow.definition, current);
        assert_eq!(state.workflow.files.review, Some(imported.clone()));
        assert!(state.workflow.proposal.is_some());
        assert!(state.start_workflow().is_err());
        assert!(state.apply_workflow().is_err());
        assert!(state.workflow.files.read(path.clone()).is_err());
        assert!(
            state
                .workflow
                .files
                .save(current.clone(), path.clone())
                .is_err()
        );
        // Cancelling the review retains both current steps and their completed preview.
        state.workflow.files.review = None;
        assert_eq!(state.workflow.definition, current);
        assert!(state.workflow.proposal.is_some());
        state.workflow.files.read(path.clone()).unwrap();
        wait_files(&mut state);
        state.parse_job.begin();
        assert!(state.confirm_workflow_import().is_err());
        assert_eq!(state.workflow.files.review, Some(imported.clone()));
        state.parse_job.finish(Phase::Done, "test");
        state.confirm_workflow_import().unwrap();
        assert_eq!(state.workflow.definition, imported);
        assert!(state.workflow.files.review.is_none());
        assert!(state.workflow.proposal.is_none());
        assert!(state.workflow.receiver.is_none());
        assert_eq!(state.dataset, original);
        assert_eq!(state.input, input);
        assert!(state.apply_workflow().is_err());
        state.start_workflow().unwrap();
        wait(&mut state);
        assert_eq!(state.dataset, original);
        state.apply_workflow().unwrap();
        assert_eq!(state.dataset.as_ref().unwrap().headers[0], "id");
        state.undo_transform();
        assert_eq!(state.dataset, original);
        assert_eq!(state.input, input);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn failed_flow_file_operations_preserve_current_steps_and_data() {
        let mut state = fixture();
        let original = state.dataset.clone();
        let current = state.workflow.definition.clone();
        let path =
            std::env::temp_dir().join(format!("zi-flow-bad-ui-{}.json", uuid::Uuid::new_v4()));
        std::fs::write(&path, "invalid definition").unwrap();
        state.workflow.files.read(path.clone()).unwrap();
        wait_files(&mut state);
        assert_eq!(state.workflow.files.job.phase, Phase::Failed);
        assert!(state.workflow.files.review.is_none());
        state
            .workflow
            .files
            .save(current.clone(), path.clone())
            .unwrap();
        wait_files(&mut state);
        assert_eq!(state.workflow.files.job.phase, Phase::Failed);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "invalid definition"
        );
        assert_eq!(state.workflow.definition, current);
        assert_eq!(state.dataset, original);
        std::fs::remove_file(path).unwrap();
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
        assert_eq!(
            step_schemas(
                &["old".into()],
                &[Step::SelectColumns {
                    columns: vec!["missing".into()]
                }]
            ),
            vec![vec!["old"], vec!["old"]]
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
