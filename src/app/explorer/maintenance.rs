use crate::explorer::lifecycle::{
    Identity, Ledger,
    installer::{self, Request, Ticket},
    store,
};
use eframe::egui;
use std::{path::Path, sync::mpsc, time::Duration};

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
}
#[derive(Default)]
pub(super) struct Maintenance {
    receiver: Option<mpsc::Receiver<Result<Work, String>>>,
    ledger: Option<Ledger>,
    ticket: Option<Box<Ticket>>,
    agreed: bool,
    message: Option<(bool, String)>,
}
impl Maintenance {
    fn run(&mut self, job: impl FnOnce() -> anyhow::Result<Work> + Send + 'static) {
        if self.receiver.is_some() {
            return;
        }
        self.ticket = None;
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
            if ledger.entries().is_empty() {
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
        ui.add_space(8.0);
        changed
    }
    #[cfg(feature = "ui-preview")]
    pub(super) fn fixture(&mut self, config: std::path::PathBuf) {
        self.run(move || {
            installer::fixture_preview(&config).map(|ticket| Work::Preview(Box::new(ticket)))
        });
    }
    #[cfg(feature = "ui-preview")]
    pub(super) fn fixture_finished(&self) -> bool {
        self.receiver.is_none()
    }
}
