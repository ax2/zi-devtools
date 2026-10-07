use super::{
    State,
    worksheet::{Data, Document},
};
use eframe::egui;
use std::{path::PathBuf, sync::mpsc};

enum Completion {
    Saved(Box<Data>),
    Read(Box<Document>),
}
#[derive(Default)]
pub(super) struct Files {
    pub baseline: Option<Data>,
    incoming: Option<Document>,
    job: Option<mpsc::Receiver<Result<Completion, String>>>,
    message: String,
    error: bool,
    allow_replace: bool,
    allow_discard: bool,
    #[cfg(feature = "ui-preview")]
    pub save_rect: Option<egui::Rect>,
    #[cfg(feature = "ui-preview")]
    replace_rect: Option<egui::Rect>,
    #[cfg(feature = "ui-preview")]
    apply_rect: Option<egui::Rect>,
    #[cfg(feature = "ui-preview")]
    fixture_path: Option<PathBuf>,
}
impl State {
    pub(super) fn snapshot(&self) -> Data {
        Data {
            name: self.sheet_name.clone(),
            expression: self.expression.clone(),
            degrees: self.degrees,
            matrix_mode: self.matrix_mode,
            variables: self.variables.clone(),
            history: self.history.clone(),
            matrix: self.matrix.snapshot(),
        }
    }
    pub fn dirty(&self) -> bool {
        self.files.baseline.as_ref().is_none_or(|data| {
            self.sheet_name != data.name
                || self.expression != data.expression
                || self.degrees != data.degrees
                || self.matrix_mode != data.matrix_mode
                || self.variables != data.variables
                || self.history != data.history
                || !self.matrix.matches(&data.matrix)
        })
    }
    pub fn busy(&self) -> bool {
        self.files.job.is_some()
    }
    pub fn awaiting_restore(&self) -> bool {
        self.files.incoming.is_some()
    }
    pub fn has_work(&self) -> bool {
        self.dirty() || self.busy() || self.files.incoming.is_some()
    }
    fn apply(&mut self, data: Data) {
        self.message.clear();
        self.sheet_name = data.name.clone();
        self.expression = data.expression.clone();
        self.degrees = data.degrees;
        self.matrix_mode = data.matrix_mode;
        self.variables = data.variables.clone();
        self.history = data.history.clone();
        self.matrix.restore(data.matrix.clone());
        self.files.baseline = Some(data);
        self.files.incoming = None;
        self.files.allow_replace = false;
        self.files.message = "已恢复工作表；未回放历史赋值，矩阵结果需重新计算".into();
        self.files.error = false;
    }
    pub fn poll(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.files.job else {
            return;
        };
        let result = match job.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("工作表后台任务意外结束，现有工作仍保留".into())
            }
        };
        self.files.job = None;
        match result {
            Ok(Completion::Saved(data)) => {
                self.files.baseline = Some(*data);
                self.files.message = if self.dirty() {
                    "已保存启动时的快照；保存期间产生的新修改仍需另存"
                } else {
                    "工作表已保存；不自动覆盖文件或加载最近路径"
                }
                .into();
                self.files.error = false;
            }
            Ok(Completion::Read(doc)) => {
                self.files.incoming = Some(*doc);
                self.files.allow_replace = false;
                self.files.message = "已读取并验证，确认前保留当前工作".into();
                self.files.error = false;
            }
            Err(error) => {
                self.files.message = error;
                self.files.error = true;
            }
        }
    }
    fn start_save(&mut self, path: PathBuf, ctx: &egui::Context) {
        if self.busy() {
            return;
        }
        let data = self.snapshot();
        let (tx, rx) = mpsc::channel();
        self.files.job = Some(rx);
        self.files.message = "正在另存工作表快照，可继续编辑".into();
        self.files.error = false;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = super::worksheet::save(&path, data.clone())
                .map(|_| Completion::Saved(Box::new(data)))
                .map_err(|e| format!("工作表保存失败：{e:#}"));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
    fn start_read(&mut self, path: PathBuf, ctx: &egui::Context) {
        if self.busy() || self.files.incoming.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.files.job = Some(rx);
        self.files.message = "正在读取工作表，当前工作不变".into();
        self.files.error = false;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = super::worksheet::read(&path)
                .map(|d| Completion::Read(Box::new(d)))
                .map_err(|e| format!("工作表读取失败：{e:#}"));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
    pub(super) fn worksheet_ui(&mut self, ui: &mut egui::Ui) {
        let dirty = self.dirty();
        let busy = self.busy();
        let pending = self.files.incoming.is_some();
        ui.horizontal_wrapped(|ui| {
            ui.label("工作表");
            ui.label(if busy {
                "后台读写中"
            } else if dirty {
                "● 未保存修改"
            } else {
                "与保存 / 恢复基线一致"
            });
            #[cfg(windows)]
            {
                let save = ui.add_enabled(!busy, egui::Button::new("另存工作表…"));
                #[cfg(feature = "ui-preview")]
                {
                    self.files.save_rect = Some(save.rect);
                }
                if save.clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Zi计算工作表", &["json"])
                        .set_file_name("calculator-worksheet.json")
                        .save_file()
                {
                    self.start_save(path, ui.ctx());
                }
                if ui
                    .add_enabled(!busy && !pending, egui::Button::new("读取工作表…"))
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Zi计算工作表", &["json"])
                        .pick_file()
                {
                    self.start_read(path, ui.ctx());
                }
            }
        });
        ui.small("明文 JSON · 包含算式、变量 / ans、历史及两矩阵与粘贴草稿 · 另存不覆盖");
        egui::CollapsingHeader::new("工作表名称与保存范围").id_salt("calculator-sheet-details").show(ui,|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.sheet_name).char_limit(128).hint_text("工作表名称"));
            ui.small("明文JSON，保存算式、变量/ans、历史及矩阵/粘贴草稿；请选择新文件。原子另存不覆盖，需NTFS等支持硬链接的文件系统。");
            ui.small("不含矩阵派生结果或文件路径；读取不回放历史，确认恢复后仍需手动固定赋值及计算矩阵。");
        });
        if !self.files.message.is_empty() {
            if self.files.error {
                ui.colored_label(ui.visuals().error_fg_color, &self.files.message);
            } else {
                ui.small(&self.files.message);
            }
        }
        let mut discard = false;
        if dirty && !busy && !pending {
            egui::CollapsingHeader::new("放弃未保存修改")
                .id_salt("calculator-discard")
                .show(ui, |ui| {
                    ui.checkbox(
                        &mut self.files.allow_discard,
                        "确认放弃当前算式、变量、历史及矩阵/粘贴草稿的修改",
                    );
                    discard = ui
                        .add_enabled(
                            self.files.allow_discard,
                            egui::Button::new("恢复上次保存 / 初始内容"),
                        )
                        .clicked();
                });
        }
        if discard && let Some(data) = self.files.baseline.clone() {
            self.apply(data);
            self.files.allow_discard = false;
        }
        let mut apply = false;
        let mut cancel = false;
        if let Some(doc) = &self.files.incoming {
            egui::Frame::group(ui.style())
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.strong("读取预览 · 确认后替换整个计算工作表");
                    ui.label(format!(
                        "{} · 来源工具 v{}",
                        doc.data.name, doc.tool_version
                    ));
                    if let Some(at) = chrono::DateTime::from_timestamp(doc.created_utc, 0) {
                        ui.small(format!(
                            "文件记录的保存UTC：{}（不证明内容来源）",
                            at.to_rfc3339()
                        ));
                    }
                    ui.label(format!(
                        "{}变量 · {}条历史 · {}",
                        doc.data.variables.len(),
                        doc.data.history.len(),
                        if doc.data.degrees { "DEG" } else { "RAD" }
                    ));
                    ui.monospace(&doc.data.expression);
                    ui.small(doc.data.matrix.describe());
                    ui.small(
                        "替换名称、算式、变量、历史、角度和两矩阵；不会重放旧计算或自动保存。",
                    );
                    if dirty {
                        let response = ui.checkbox(
                            &mut self.files.allow_replace,
                            "允许替换当前未保存的工作表修改",
                        );
                        #[cfg(feature = "ui-preview")]
                        {
                            self.files.replace_rect = Some(response.rect);
                        }
                        #[cfg(not(feature = "ui-preview"))]
                        let _ = response;
                    }
                    ui.horizontal_wrapped(|ui| {
                        let response = ui.add_enabled(
                            !busy && (!dirty || self.files.allow_replace),
                            egui::Button::new("确认恢复工作表"),
                        );
                        #[cfg(feature = "ui-preview")]
                        {
                            self.files.apply_rect = Some(response.rect);
                        }
                        apply = response.clicked();
                        cancel = ui.button("放弃本次读取").clicked();
                    });
                });
        }
        if apply && let Some(doc) = self.files.incoming.take() {
            self.apply(doc.data);
        }
        if cancel {
            self.files.incoming = None;
            self.files.allow_replace = false;
            self.files.message = "已放弃读取；当前工作保持不变".into();
            self.files.error = false;
        }
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_sheet_io_start(&mut self, ctx: &egui::Context, folder: &std::path::Path) {
        self.files.incoming = None;
        self.expression = "deferred = 123".into();
        self.variables
            .insert("third".into(), super::Value::Exact(1, 3));
        let path = folder.join(format!("worksheet-smoke-{}.json", uuid::Uuid::new_v4()));
        self.files.fixture_path = Some(path.clone());
        self.start_save(path, ctx);
        self.expression = "changed during save".into();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_sheet_io_read(&mut self, ctx: &egui::Context) {
        assert!(!self.busy());
        assert!(self.dirty());
        assert!(self.files.message.contains("新修改"));
        self.start_read(self.files.fixture_path.clone().unwrap(), ctx);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_sheet_position(&self, apply: bool) -> egui::Pos2 {
        if apply {
            self.files.apply_rect.unwrap().center()
        } else {
            self.files.replace_rect.unwrap().center()
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_sheet_io_check(&self, restored: bool) {
        assert!(!self.busy());
        if restored {
            assert!(!self.has_work());
            assert_eq!(self.expression, "deferred = 123");
            assert!(!self.variables.contains_key("deferred"));
            assert_eq!(self.variables["third"], super::Value::Exact(1, 3));
        } else {
            assert!(self.files.incoming.is_some());
            assert!(self.dirty());
            assert_eq!(self.expression, "changed during save");
            assert!(!self.files.allow_replace);
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_sheet_overwrite(&mut self, ctx: &egui::Context) {
        self.start_save(self.files.fixture_path.clone().unwrap(), ctx);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_sheet_cleanup(&mut self) {
        assert!(!self.busy());
        assert!(self.files.error);
        assert!(!self.dirty());
        let path = self.files.fixture_path.take().unwrap();
        let doc = super::worksheet::read(&path).unwrap();
        assert_eq!(doc.data.expression, "deferred = 123");
        assert_eq!(doc.data.variables["third"], super::Value::Exact(1, 3));
        std::fs::remove_file(path).unwrap();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_sheet_fixture(&mut self) {
        self.preview_fixture();
        self.sheet_name = "预算与精确分数 · 合成示例".into();
        let mut imported = self.snapshot();
        imported.name = "待恢复的矩阵工作表 · 合成示例".into();
        imported.expression = "price = 19.90".into();
        self.files.incoming = Some(Document::new(imported).unwrap());
        self.files.allow_replace = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calculator::Value;
    #[test]
    fn pending_restore_cannot_be_bypassed_by_matrix_replacement_consent() {
        let mut state = State::default();
        state.files.incoming = Some(Document::new(state.snapshot()).unwrap());
        let text =
            super::super::exchange::NumericTable::new(1, 1, vec![super::super::Value::Exact(1, 3)])
                .unwrap()
                .json(super::super::exchange::Representation::Typed)
                .unwrap();
        assert!(
            state
                .receive_numeric_with_policy(&text, super::super::exchange::MatrixSlot::B, true)
                .is_err()
        );
        assert!(state.awaiting_restore());
    }
    #[test]
    fn restored_values_do_not_execute_assignments_or_reuse_matrix_results() {
        let mut state = State::default();
        assert!(!state.has_work());
        let mut data = state.snapshot();
        data.expression = "should_not_exist = 900".into();
        data.variables.insert("ans".into(), Value::Exact(7, 1));
        state.apply(data);
        assert!(!state.dirty());
        assert!(!state.variables.contains_key("should_not_exist"));
        assert_eq!(state.variables["ans"], Value::Exact(7, 1));
        state.expression = "4".into();
        assert!(state.has_work());
    }
    #[test]
    fn successful_snapshot_save_does_not_mark_later_changes_saved_and_load_waits_for_confirmation()
    {
        let ctx = egui::Context::default();
        let mut state = State::default();
        let data = state.snapshot();
        let (tx, rx) = mpsc::channel();
        state.files.job = Some(rx);
        state.expression = "changed while saving".into();
        tx.send(Ok(Completion::Saved(Box::new(data)))).unwrap();
        state.poll(&ctx);
        assert!(state.dirty());
        assert!(!state.busy());
        assert!(state.files.message.contains("新修改"));
        let before = state.expression.clone();
        let (tx, rx) = mpsc::channel();
        state.files.job = Some(rx);
        tx.send(Ok(Completion::Read(Box::new(
            Document::new(State::default().snapshot()).unwrap(),
        ))))
        .unwrap();
        state.poll(&ctx);
        assert_eq!(state.expression, before);
        assert!(state.has_work());
        assert!(state.files.incoming.is_some());
        assert!(!state.files.allow_replace);
    }
    #[test]
    fn failed_or_disconnected_io_preserves_baseline_and_current_work() {
        let ctx = egui::Context::default();
        let mut state = State {
            expression: "keep".into(),
            ..State::default()
        };
        let baseline = state.files.baseline.clone();
        let (tx, rx) = mpsc::channel();
        state.files.job = Some(rx);
        tx.send(Err("fixture failure".into())).unwrap();
        state.poll(&ctx);
        assert_eq!(state.files.baseline, baseline);
        assert_eq!(state.expression, "keep");
        assert!(state.files.error);
        let (tx, rx) = mpsc::channel();
        state.files.job = Some(rx);
        drop(tx);
        state.poll(&ctx);
        assert!(!state.busy());
        assert!(state.files.error);
        assert!(state.dirty());
    }
}
