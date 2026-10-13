use super::*;
use crate::explorer::{Plan, audit::Report};
use anyhow::Context;
use sha2::{Digest, Sha256};
use std::sync::mpsc;
#[cfg(windows)]
mod maintenance;

type AuditResult = ([u8; 32], Result<Report, String>);
#[derive(Default)]
pub(super) struct Audit {
    receiver: Option<mpsc::Receiver<AuditResult>>,
    result: Option<AuditResult>,
    #[cfg(windows)]
    maintenance: maintenance::Maintenance,
}
fn fingerprint(plan: &Plan) -> [u8; 32] {
    Sha256::digest(serde_json::to_vec(plan).expect("plan serialization")).into()
}
impl Audit {
    #[cfg(all(windows, feature = "ui-preview"))]
    pub(super) fn fixture_maintenance(&mut self, config: std::path::PathBuf) {
        self.maintenance.fixture(config);
    }
    #[cfg(all(windows, feature = "ui-preview"))]
    pub(super) fn fixture_finished(&self) -> bool {
        self.maintenance.fixture_finished()
    }
    pub(super) fn start(&mut self, plan: &Plan) {
        if self.receiver.is_some() {
            return;
        }
        let plan = plan.clone();
        let digest = fingerprint(&plan);
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.result = None;
        std::thread::spawn(move || {
            let result = crate::explorer::audit::inspect(&plan).map_err(|error| error.to_string());
            let _ = sender.send((digest, result));
        });
    }
    fn ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, plan: Option<&Plan>) {
        #[cfg(windows)]
        if let Some(plan) = plan {
            let controlled = ctx.data(|data| {
                data.get_temp::<bool>(egui::Id::new("preview-explorer-only"))
                    .unwrap_or(false)
            });
            if self.maintenance.ui(ui, ctx, &plan.config, controlled) {
                self.result = None;
                self.receiver = None;
            }
        }
        if let Some(receiver) = &self.receiver {
            match receiver.try_recv() {
                Ok(result) => {
                    self.result = Some(result);
                    self.receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.receiver = None;
                    self.result = Some((
                        plan.map(fingerprint).unwrap_or_default(),
                        Err("状态检查意外结束，请重试".into()),
                    ));
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let busy = self.receiver.is_some();
        if ui
            .add_enabled(
                !busy && plan.is_some(),
                egui::Button::new("检查当前入口状态"),
            )
            .clicked()
        {
            self.start(plan.unwrap());
            ctx.request_repaint();
        }
        let busy = self.receiver.is_some();
        if busy {
            ui.spinner();
            ui.label("正在只读检查…");
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if let Some((digest, result)) = &self.result {
            if plan.map(fingerprint).as_ref() != Some(digest) {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "程序或配置已变化，旧结果已过期，请重新检查",
                );
            } else {
                match result {
                    Ok(report) => {
                        ui.strong(report.label());
                        ui.small(format!("缺失{}项 · 变化{}项 · 未知{}项。仅对照当前用户注册内容，不代表Explorer显示或调用已验收。",report.missing,report.changed,report.unknown));
                    }
                    Err(error) => {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                }
            }
        } else if !busy {
            ui.small("尚未检查。此检查只读取本产品候选入口，不创建、修复或移除任何键。");
        }
        ui.add_space(8.0);
    }
}

impl DevToolsApp {
    pub(super) fn explorer_preview(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let plan = std::env::current_exe()
            .context("无法定位当前程序")
            .and_then(|exe| crate::explorer::plan(&exe, &self.manager.config_snapshot().path));
        let expanded;
        #[cfg(feature = "ui-preview")]
        {
            expanded = ctx.data(|data| {
                data.get_temp::<bool>(egui::Id::new("preview-explorer-only"))
                    .unwrap_or(false)
            });
        }
        #[cfg(not(feature = "ui-preview"))]
        {
            expanded = false;
        }
        egui::Frame::new().fill(self.colors.card).corner_radius(12.0).inner_margin(18.0).show(ui, |ui| {
            ui.strong("资源管理器右键入口");
            ui.label("开发中 · 手动登记与维护单文件经典菜单");
            ui.small("预览并确认后更新当前用户入口；不需要管理员权限。MSI安装器可设置菜单；完整多选、目录、窗口转发和现代菜单继续开发。");
            self.explorer_audit.ui(ui, ctx, plan.as_ref().ok());
            egui::CollapsingHeader::new("查看此程序的候选菜单与注册明细").default_open(expanded).show(ui, |ui| {
                match &plan {
                    Ok(plan) => {
                        ui.label("Zi DevTools › 选择工具处理…");
                        ui.add_space(6.0);
                        for target in crate::intake::Target::ALL {
                            ui.horizontal_wrapped(|ui| {
                                ui.strong(target.label());
                                ui.small(if target == crate::intake::Target::Files { "外部接收最多64个文件；当前菜单计划仅单选" } else { "一次一个UTF-8文件 · 最多2 MiB" });
                            });
                        }
                        ui.add_space(10.0);
                        ui.small("选择动作只预选接收工具，仍需在导入页确认，不自动处理文件。明细含本机程序及配置位置，请按需复制。");
                        let mut json = serde_json::to_string_pretty(&plan).expect("registration plan serialization");
                        if ui.button("复制注册预览（JSON）").clicked() { ctx.copy_text(json.clone()); }
                        egui::ScrollArea::vertical().id_salt("explorer-plan-json").max_height(240.0).show(ui, |ui| {
                            ui.add(egui::TextEdit::multiline(&mut json).code_editor().interactive(false).desired_width(f32::INFINITY));
                        });
                    }
                    Err(error) => { ui.colored_label(self.colors.red, error.to_string()); }
                }
            });
        });
    }
}
