use crate::explorer::lifecycle::{
    Identity, Ledger,
    installer::{self, Request, Ticket},
    store,
};
use eframe::egui;
use std::{path::Path, sync::mpsc, time::Duration};
mod cancellation_ui;
mod completion_ui;
use store::recovery::{self, Operation, Status, Summary};

fn path_text(path: &Path) -> String {
    let text = path.display().to_string();
    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
    }
}

fn path_label(ui: &mut egui::Ui, prefix: &str, path: &Path) {
    ui.add(
        egui::Label::new(egui::RichText::new(format!("{prefix}{}", path_text(path))).small())
            .wrap(),
    );
}

enum Work {
    Loaded(Ledger),
    Preview(Box<Ticket>),
    Applied(Ledger),
    Journal(Vec<Summary>),
    CompletionPreview(Box<recovery::completion::Ticket>),
    CancellationPreview(Box<recovery::cancellation::Ticket>),
    Completed {
        ledger: Ledger,
        id: String,
        status: Status,
        refreshed: Result<Vec<Summary>, String>,
    },
    Forgotten {
        id: String,
        refreshed: Result<Vec<Summary>, String>,
    },
}
#[derive(Default)]
pub(super) struct Maintenance {
    receiver: Option<mpsc::Receiver<Result<Work, String>>>,
    ledger: Option<Ledger>,
    ticket: Option<Box<Ticket>>,
    agreed: bool,
    message: Option<(bool, String)>,
    journal: Option<Vec<Summary>>,
    forget: Option<Summary>,
    completion: Option<Box<recovery::completion::Ticket>>,
    cancellation: Option<Box<recovery::cancellation::Ticket>>,
}
impl Maintenance {
    fn run(&mut self, job: impl FnOnce() -> anyhow::Result<Work> + Send + 'static) {
        if self.receiver.is_some() {
            return;
        }
        self.ticket = None;
        self.forget = None;
        self.completion = None;
        self.cancellation = None;
        self.agreed = false;
        self.message = None;
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        std::thread::spawn(move || {
            let _ = sender.send(job().map_err(|error| error.to_string()));
        });
    }
    fn poll(&mut self) -> bool {
        let Some(receiver) = &self.receiver else {
            return false;
        };
        match receiver.try_recv() {
            Ok(result) => {
                self.receiver = None;
                match result {
                    Ok(Work::Loaded(ledger)) => self.ledger = Some(ledger),
                    Ok(Work::Journal(records)) => self.journal = Some(records),
                    Ok(Work::CompletionPreview(ticket)) => {
                        self.ledger = Some(ticket.current().clone());
                        self.completion = Some(ticket);
                    }
                    Ok(Work::CancellationPreview(ticket)) => {
                        self.ledger = Some(ticket.current().clone());
                        self.cancellation = Some(ticket);
                    }
                    Ok(Work::Completed {
                        ledger,
                        id,
                        status,
                        refreshed,
                    }) => {
                        self.ledger = Some(ledger);
                        if let Some(records) = &mut self.journal {
                            for record in records {
                                if record.operation_id == id {
                                    record.status = status;
                                }
                            }
                        }
                        self.message = Some(match refreshed {
                            Ok(records) => {
                                self.journal = Some(records);
                                (
                                    true,
                                    if status == Status::Canceled {
                                        "未执行的菜单维护已取消，安装文件、菜单与程序登记保留。"
                                    } else {
                                        "维护已结束，当前菜单与程序登记保留。"
                                    }
                                    .into(),
                                )
                            }
                            Err(error) => (
                                false,
                                format!(
                                    "维护已结束，但刷新记录失败：{error}；其他显示内容可能已过期。"
                                ),
                            ),
                        });
                        return true;
                    }
                    Ok(Work::Forgotten { id, refreshed }) => {
                        if let Some(records) = &mut self.journal {
                            records.retain(|record| record.operation_id != id);
                        }
                        self.message = Some(match refreshed {
                            Ok(records) => {
                                self.journal = Some(records);
                                (true, "这条已结束的维护记录已清理。".into())
                            }
                            Err(error) => (
                                false,
                                format!(
                                    "记录已清理，但刷新失败：{error}；其他显示内容可能已过期。"
                                ),
                            ),
                        });
                    }
                    Ok(Work::Preview(ticket)) => {
                        self.ledger = Some(ticket.current().clone());
                        self.ticket = Some(ticket);
                    }
                    Ok(Work::Applied(ledger)) => {
                        self.ledger = Some(ledger);
                        self.message = Some((
                            true,
                            "登记与菜单已一起更新。文件右键可在“显示更多选项”中查看经典入口。"
                                .into(),
                        ));
                        return true;
                    }
                    Err(error) => self.message = Some((false, error)),
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.receiver = None;
                self.message = Some((false, "维护任务意外结束，请重新检查当前入口".into()));
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        false
    }
    pub(super) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        config: &Path,
        controlled: bool,
    ) -> bool {
        let changed = self.poll();
        let busy = self.receiver.is_some();
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("刷新安装记录"))
                .clicked()
            {
                self.run(|| store::load().map(Work::Loaded));
            }
            if ui
                .add_enabled(
                    !busy && !controlled,
                    egui::Button::new("预览安装 / 修复此程序"),
                )
                .clicked()
            {
                let config = config.to_path_buf();
                self.run(move || {
                    installer::preview(&config, Request::Install)
                        .map(|ticket| Work::Preview(Box::new(ticket)))
                });
            }
        });
        if self.receiver.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("正在核验文件或维护入口…");
            });
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if let Some((success, message)) = &self.message {
            ui.colored_label(
                if *success {
                    ui.visuals().text_color()
                } else {
                    ui.visuals().error_fg_color
                },
                message,
            );
        }
        if let Some(ledger) = &self.ledger {
            if ledger.pending_msi_operation().is_some() {
                ui.colored_label(ui.visuals().warn_fg_color,
                    "安装维护尚未结束，已保留恢复位置。请回到安装程序完成或回滚；已登记的工具仍可使用。");
            }
            if ledger.entries().is_empty() && ledger.pending_msi_operation().is_none() {
                ui.small("尚无安装登记。预览后确认，可为此程序添加经典菜单入口。");
            }
            let mut request = None;
            if !ledger.entries().is_empty() {
                egui::CollapsingHeader::new(format!(
                    "已登记的程序 · {} 份",
                    ledger.entries().len()
                ))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("explorer-installations")
                        .max_height(210.0)
                        .show(ui, |ui| {
                            for entry in ledger.entries() {
                                let record = &entry.installation;
                                ui.push_id(
                                    serde_json::to_string(&record.identity).unwrap(),
                                    |ui| {
                                        ui.horizontal_wrapped(|ui| {
                                            ui.strong(
                                                if matches!(record.identity, Identity::Msi) {
                                                    "MSI安装"
                                                } else {
                                                    "手动登记"
                                                },
                                            );
                                            ui.label(format!("v{}", record.version));
                                            if ledger.active() == Some(record) {
                                                ui.label("· 当前入口");
                                            }
                                            if ui
                                                .add_enabled(
                                                    !busy && !controlled,
                                                    egui::Button::new("设为入口"),
                                                )
                                                .clicked()
                                            {
                                                request = Some(Request::Activate(
                                                    record.identity.clone(),
                                                ));
                                            }
                                            if ui
                                                .add_enabled(
                                                    !busy
                                                        && !controlled
                                                        && matches!(
                                                            record.identity,
                                                            Identity::Portable(_)
                                                        ),
                                                    egui::Button::new("迁移到此程序"),
                                                )
                                                .clicked()
                                            {
                                                request = Some(Request::Relocate(
                                                    record.identity.clone(),
                                                ));
                                            }
                                            if ui
                                                .add_enabled(
                                                    !busy
                                                        && !controlled
                                                        && matches!(
                                                            record.identity,
                                                            Identity::Portable(_)
                                                        ),
                                                    egui::Button::new("移除登记"),
                                                )
                                                .clicked()
                                            {
                                                request =
                                                    Some(Request::Remove(record.identity.clone()));
                                            }
                                        });
                                        path_label(ui, "程序：", &record.executable);
                                        path_label(ui, "配置：", &record.config);
                                        ui.add_space(8.0);
                                    },
                                );
                            }
                        });
                });
            }
            if let Some(request) = request {
                let config = config.to_path_buf();
                self.run(move || {
                    installer::preview(&config, request)
                        .map(|ticket| Work::Preview(Box::new(ticket)))
                });
            }
        }
        let mut submit = false;
        let mut cancel = false;
        if let Some(ticket) = &self.ticket {
            ui.group(|ui| {
                ui.set_width(ui.available_width());
                ui.strong(format!("待确认：{}", ticket.title()));
                if let Some(active) = ticket.current().active() { path_label(ui, "当前程序：", &active.executable); }
                if let Some(active) = ticket.result().active() {
                    ui.label("确认后由以下程序接收文件：");
                    ui.add(egui::Label::new(path_text(&active.executable)).wrap());
                    path_label(ui, "配置：", &active.config);
                    ui.small("程序文件已核验，应用前会再次核验。菜单选择后仍需在导入页确认处理文件。");
                } else { ui.label("本次没有可用的剩余安装，将移除共享菜单入口。程序、配置和用户数据保持不变。"); }
                let (writes, removals) = ticket.counts();
                egui::CollapsingHeader::new("变更数量").show(ui, |ui| { ui.small(format!("更新{writes}项入口设置，移除{removals}项；确认后保留{}份安装登记。", ticket.result().entries().len())); });
                let current = ticket.matches_current(config);
                if !current { ui.colored_label(ui.visuals().warn_fg_color, "配置位置已变化，请重新预览"); }
                if controlled || ticket.preview_only() { ui.small("受控界面预览 · 禁止应用到实际菜单"); }
                ui.checkbox(&mut self.agreed, "同意更新当前用户的右键入口及安装登记");
                ui.horizontal(|ui| {
                    submit = ui.add_enabled(self.agreed && current && !controlled && !ticket.preview_only() && !busy, egui::Button::new("确认应用")).clicked();
                    cancel = ui.add_enabled(!busy, egui::Button::new("取消预览")).clicked();
                });
            });
        }
        if cancel {
            self.ticket = None;
            self.agreed = false;
        }
        if submit {
            let ticket = *self.ticket.take().expect("confirmed ticket");
            let config = config.to_path_buf();
            self.run(move || installer::apply(ticket, &config).map(Work::Applied));
        }
        self.journal_ui(ui, controlled);
        self.completion_ui(ui, controlled);
        self.cancellation_ui(ui, controlled);
        ui.add_space(8.0);
        changed
    }
    fn journal_ui(&mut self, ui: &mut egui::Ui, controlled: bool) {
        let busy = self.receiver.is_some();
        if ui
            .add_enabled(
                !busy && !controlled,
                egui::Button::new("查看 / 刷新安装维护记录"),
            )
            .clicked()
        {
            self.run(|| recovery::list().map(Work::Journal));
        }
        let mut selected = None;
        let mut complete = None;
        let mut cancel_prepared = None;
        if let Some(records) = &self.journal {
            egui::CollapsingHeader::new(format!("安装维护记录 · {} 条", records.len()))
                .default_open(true).show(ui, |ui| {
                    ui.small("最多保留32条。查看不运行安装程序；仅已结束的历史允许逐条清理，未结束记录保留用于恢复。");
                    if records.is_empty() { ui.label("没有维护记录。"); }
                    egui::ScrollArea::vertical().id_salt("explorer-journal").max_height(220.0).show(ui, |ui| {
                        for record in records {
                            ui.push_id(&record.operation_id, |ui| {
                                let terminal = matches!(record.status, Status::Completed | Status::Reverted | Status::Canceled);
                                ui.horizontal_wrapped(|ui| {
                                    ui.strong(match record.status {
                                        Status::Prepared => "已准备 / 待应用",
                                        Status::Applied => "已应用 / 待结束",
                                        Status::CommitReady => "提交动作已执行 / 待安装器确认",
                                        Status::Committed => "安装已结束 / 待收尾",
                                        Status::Reverted => "已回滚",
                                        Status::Completed => "已完成",
                                        Status::Canceled => "已取消 / 未执行菜单",
                                    });
                                    ui.label(match &record.operation {
                                        Operation::Install { version } => format!("安装 / 更新 v{version}"),
                                        Operation::Remove => "移除菜单登记".into(),
                                    });
                                    if terminal && ui.add_enabled(!busy && !controlled, egui::Button::new("清理这条历史")).clicked() {
                                        selected = Some(record.clone());
                                    }
                                    if record.status == Status::Committed && ui.add_enabled(!busy && !controlled, egui::Button::new("预览结束维护")).clicked() {
                                        complete = Some(record.operation_id.clone());
                                    }
                                    if record.status == Status::Prepared && ui.add_enabled(!busy && !controlled, egui::Button::new("预览取消菜单维护")).clicked() {
                                        cancel_prepared = Some(record.operation_id.clone());
                                    }
                                });
                                ui.add(egui::Label::new(egui::RichText::new(&record.operation_id).small().monospace()).wrap());
                                if record.status == Status::Committed { ui.small("已有安装事务结束凭据，可预览并确认释放维护占用，保留当前入口。"); }
                                else if !terminal { ui.small("没有安装事务结束凭据。请先完成或回滚安装，不能在此强制标为完成。"); }
                                ui.add_space(6.0);
                            });
                        }
                    });
                });
        }
        if let Some(record) = selected {
            self.ticket = None;
            self.agreed = false;
            self.forget = Some(record);
            self.completion = None;
            self.cancellation = None;
        }
        if let Some(id) = complete {
            self.run(move || {
                recovery::completion::preview(&id)
                    .map(|ticket| Work::CompletionPreview(Box::new(ticket)))
            });
        }
        if let Some(id) = cancel_prepared {
            self.run(move || {
                recovery::cancellation::preview(&id)
                    .map(|ticket| Work::CancellationPreview(Box::new(ticket)))
            });
        }
        let mut confirmed = false;
        let mut canceled = false;
        if let Some(record) = &self.forget {
            ui.group(|ui| {
                ui.strong("清理已结束的维护记录");
                ui.add(egui::Label::new(&record.operation_id).wrap());
                ui.label("只删除这条历史，不改安装登记、菜单、程序或用户文件。清理后无法再查看这条历史。");
                ui.checkbox(&mut self.agreed, "同意清理这条已结束的记录");
                ui.horizontal(|ui| {
                    confirmed = ui.add_enabled(self.agreed && !busy && !controlled, egui::Button::new("确认清理记录")).clicked();
                    canceled = ui.add_enabled(!busy, egui::Button::new("取消清理")).clicked();
                });
            });
        }
        if canceled {
            self.forget = None;
            self.agreed = false;
        }
        if confirmed {
            let id = self
                .forget
                .take()
                .expect("confirmed journal record")
                .operation_id;
            self.run(move || {
                recovery::forget(&id)?;
                Ok(Work::Forgotten {
                    id,
                    refreshed: recovery::list().map_err(|error| error.to_string()),
                })
            });
        }
    }
    #[cfg(feature = "ui-preview")]
    pub(super) fn fixture(&mut self, config: std::path::PathBuf) {
        let pending = std::env::args().any(|arg| arg == "--pending-msi");
        let journal = std::env::args().any(|arg| arg == "--journal-cleanup");
        let completion = std::env::args().any(|arg| arg == "--journal-recovery");
        let cancellation = std::env::args().any(|arg| arg == "--journal-cancel");
        self.run(move || {
            if cancellation {
                return Ok(Work::CancellationPreview(Box::new(
                    recovery::cancellation::fixture_preview(),
                )));
            }
            if completion {
                return recovery::completion::fixture_preview()
                    .map(|ticket| Work::CompletionPreview(Box::new(ticket)));
            }
            if pending || journal || completion {
                // Synthetic read-only capture state; never a recovery receipt.
                let mut value = serde_json::to_value(Ledger::default())?;
                value["pending_msi"] = uuid::Uuid::new_v4().to_string().into();
                value["msi_cursor"] = uuid::Uuid::new_v4().to_string().into();
                value["revision"] = 1.into();
                let ledger: Ledger = serde_json::from_value(value)?;
                ledger.validate()?;
                return Ok(Work::Loaded(ledger));
            }
            installer::fixture_preview(&config).map(|ticket| Work::Preview(Box::new(ticket)))
        });
        if journal {
            let completed = Summary {
                operation_id: "51d0102f-8b51-40e5-97d3-556062d4b9f6".into(),
                status: Status::Completed,
                operation: Operation::Install {
                    version: "0.82.0".into(),
                },
            };
            self.journal = Some(vec![
                completed.clone(),
                Summary {
                    operation_id: "374d06dc-5c86-4dbf-97fb-3985d8f37f19".into(),
                    status: Status::Applied,
                    operation: Operation::Remove,
                },
            ]);
            self.forget = Some(completed);
        }
        if completion {
            self.journal = Some(vec![Summary {
                operation_id: "374d06dc-5c86-4dbf-97fb-3985d8f37f19".into(),
                status: Status::Committed,
                operation: Operation::Install {
                    version: "0.82.0".into(),
                },
            }]);
        }
        if cancellation {
            self.journal = Some(vec![Summary {
                operation_id: "374d06dc-5c86-4dbf-97fb-3985d8f37f19".into(),
                status: Status::Prepared,
                operation: Operation::Install {
                    version: "0.82.0".into(),
                },
            }]);
        }
    }
    #[cfg(feature = "ui-preview")]
    pub(super) fn fixture_finished(&self) -> bool {
        self.receiver.is_none()
    }
}

#[cfg(test)]
mod journal_tests {
    use super::*;

    #[test]
    fn committed_cleanup_with_refresh_failure_keeps_other_records_and_reports_partial_success() {
        let completed = Summary {
            operation_id: uuid::Uuid::new_v4().to_string(),
            status: Status::Completed,
            operation: Operation::Remove,
        };
        let pending = Summary {
            operation_id: uuid::Uuid::new_v4().to_string(),
            status: Status::Applied,
            operation: Operation::Remove,
        };
        let mut state = Maintenance {
            journal: Some(vec![completed.clone(), pending.clone()]),
            ..Default::default()
        };
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Ok(Work::Forgotten {
                id: completed.operation_id,
                refreshed: Err("读取失败".into()),
            }))
            .unwrap();
        state.receiver = Some(receiver);
        assert!(!state.poll());
        assert_eq!(state.journal, Some(vec![pending]));
        let (success, message) = state.message.unwrap();
        assert!(!success);
        assert!(message.contains("记录已清理") && message.contains("刷新失败"));
    }

    #[test]
    fn rejected_cleanup_retains_visible_records() {
        let pending = Summary {
            operation_id: uuid::Uuid::new_v4().to_string(),
            status: Status::Applied,
            operation: Operation::Remove,
        };
        let mut state = Maintenance {
            journal: Some(vec![pending.clone()]),
            ..Default::default()
        };
        let (sender, receiver) = mpsc::channel();
        sender.send(Err("未完成记录不能清理".into())).unwrap();
        state.receiver = Some(receiver);
        assert!(!state.poll());
        assert_eq!(state.journal, Some(vec![pending]));
        assert_eq!(state.message, Some((false, "未完成记录不能清理".into())));
    }
}
