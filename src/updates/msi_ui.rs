//! Installer prepare/consent/exit. All preparation stays off the UI thread.
use super::{Report, download, installer, msi};
use anyhow::{Context, Result, ensure};
use eframe::egui;
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};
struct Job {
    receiver: mpsc::Receiver<Result<msi::Prepared, String>>,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicU64>,
    total: u64,
}
#[derive(Default)]
pub struct State {
    job: Option<Job>,
    prepared: Option<Arc<msi::Prepared>>,
    inspected: bool,
    installed: bool,
    confirm: bool,
    message: String,
    error: bool,
    history: Vec<msi::Receipt>,
}
impl Drop for State {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
}
fn prepare_online(
    report: Report,
    cancel: &AtomicBool,
    progress: &AtomicU64,
) -> Result<msi::Prepared> {
    installer::numeric_version(&report.version.to_string())?;
    ensure!(
        ["update-manifest.json", "update-manifest.sig"]
            .iter()
            .all(|name| report.assets.iter().any(|a| a.name == *name)),
        "MSI升级必须有完整发布者签名"
    );
    let asset = report
        .assets
        .iter()
        .find(|a| a.name == format!("ZiDevTools-{}-windows-x64.msi", report.version))
        .context("缺少MSI安装器")?;
    let downloaded = download::download(&report, asset, cancel, progress)?;
    let signed = downloaded.signed_metadata.context("MSI发布未认证")?;
    msi::prepare(&signed.0, &signed.1, &downloaded.bytes, cancel)
}
impl State {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
    pub fn available(&mut self) -> bool {
        if !self.inspected {
            self.inspected = true;
            self.installed = msi::current_installation().is_ok();
            match msi::history() {
                Ok(rows) => self.history = rows,
                Err(e) => {
                    self.message = format!("MSI升级记录读取失败：{e}");
                    self.error = true;
                }
            }
        }
        self.installed
    }
    pub fn poll(&mut self) {
        let Some(job) = &self.job else { return };
        let result = match job.receiver.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("MSI准备任务中断".into()),
        };
        self.job = None;
        match result {
            Ok(p) => {
                self.prepared = Some(Arc::new(p));
                self.confirm = false;
                self.error = false;
                self.message = "安装器已认证并暂存；当前安装版尚未修改。".into();
            }
            Err(e) => {
                self.message = e;
                self.error = true;
            }
        }
    }
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        report: Option<Report>,
        blockers: &[&str],
        config: &Path,
    ) -> bool {
        ui.separator();
        ui.label(egui::RichText::new("MSI安装版升级").strong());
        ui.weak("认证完整安装器；确认后退出，由Windows Installer升级。用户数据保留，重启程序不自动启动服务。");
        if let Some(report) = report {
            let newer = semver::Version::parse(env!("CARGO_PKG_VERSION"))
                .is_ok_and(|current| report.version.cmp_precedence(&current).is_gt());
            let numeric = installer::numeric_version(&report.version.to_string()).is_ok();
            let signed = ["update-manifest.json", "update-manifest.sig"]
                .iter()
                .all(|name| report.assets.iter().any(|a| a.name == *name));
            if !newer {
                ui.weak("无需同版替换或降级。");
            }
            if !numeric {
                ui.weak("该发布不是三段数值MSI版本，不能用于安装器升级。");
            }
            if !signed {
                ui.weak("此发布没有完整签名，可以手动下载，但不能自动安装。");
            }
            if ui
                .add_enabled(
                    !self.busy() && self.prepared.is_none() && newer && numeric && signed,
                    egui::Button::new("下载并准备MSI升级"),
                )
                .clicked()
            {
                let total = report
                    .assets
                    .iter()
                    .find(|a| a.name.ends_with(".msi"))
                    .map_or(0, |a| a.size);
                let (tx, receiver) = mpsc::channel();
                let cancel = Arc::new(AtomicBool::new(false));
                let worker = cancel.clone();
                let progress = Arc::new(AtomicU64::new(0));
                let worker_progress = progress.clone();
                let ctx = ui.ctx().clone();
                match std::thread::Builder::new()
                    .name("msi-update-prepare".into())
                    .spawn(move || {
                        let _ = tx.send(
                            prepare_online(report, &worker, &worker_progress)
                                .map_err(|e| e.to_string()),
                        );
                        ctx.request_repaint();
                    }) {
                    Ok(_) => {
                        self.job = Some(Job {
                            receiver,
                            cancel,
                            progress,
                            total,
                        });
                        self.error = false;
                        self.message = "正在认证、下载并核对安装器身份…".into();
                    }
                    Err(_) => {
                        self.error = true;
                        self.message = "无法启动MSI准备任务".into();
                    }
                }
            }
        } else if self.prepared.is_none() {
            ui.weak("先检查更高版本的正式签名发布。");
        }
        if let Some(job) = &self.job {
            let done = job.progress.load(Ordering::Relaxed);
            if job.total > 0 {
                ui.add(
                    egui::ProgressBar::new(done as f32 / job.total as f32).text(format!(
                        "已下载 {} / {}",
                        super::size(done),
                        super::size(job.total)
                    )),
                );
            }
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("下载 / 校验 / 暂存");
                if ui.button("取消准备").clicked() {
                    job.cancel.store(true, Ordering::Relaxed);
                }
            });
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        if !self.message.is_empty() {
            ui.colored_label(
                if self.error {
                    ui.visuals().error_fg_color
                } else {
                    ui.visuals().text_color()
                },
                &self.message,
            );
        }
        if let Some(p) = self.prepared.clone() {
            ui.label(format!(
                "安装版 {} → {} · {}",
                p.installed_version,
                p.version,
                super::size(p.bytes)
            ));
            ui.weak(format!("安装目录：{}", p.directory.display()));
            ui.weak(format!("安装器与处理记录：{}", p.stage.display()));
            egui::CollapsingHeader::new(format!("工具变化（{}项）", p.changes.len())).show(
                ui,
                |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(150.0)
                        .show(ui, |ui| {
                            for row in &p.changes {
                                ui.label(row);
                            }
                        });
                },
            );
            ui.checkbox(&mut self.confirm, "同意退出程序并使用Windows Installer升级");
            ui.weak("安装器会显示真实进度并可取消；若要求系统重启，将保留记录，不自动重启电脑。");
            if !blockers.is_empty() {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!("先处理这些工作：{}", blockers.join("、")),
                );
            }
            if ui
                .add_enabled(
                    self.confirm && blockers.is_empty() && !self.busy(),
                    egui::Button::new("退出并使用MSI升级"),
                )
                .clicked()
            {
                match msi::launch(&p, config) {
                    Ok(()) => return true,
                    Err(e) => {
                        self.error = true;
                        self.message = format!("没有退出或安装：{e}");
                    }
                }
            }
            if ui.button("重新准备（保留本次暂存）").clicked() {
                self.prepared = None;
                self.confirm = false;
                self.message.clear();
            }
        }
        false
    }
    pub fn records_ui(&self, ui: &mut egui::Ui) {
        if !self.installed && self.error && !self.message.is_empty() {
            ui.colored_label(ui.visuals().error_fg_color, &self.message);
        }
        if !self.history.is_empty() {
            egui::CollapsingHeader::new(format!("MSI升级记录（{}项）", self.history.len())).show(
                ui,
                |ui| {
                    for row in &self.history {
                        let message = if row.state == "installing" {
                            "上次升级没有终态记录，状态未知；请检查Windows Installer，不自动重试"
                        } else {
                            &row.message
                        };
                        ui.label(format!(
                            "{} → {} · {}",
                            row.installed_version, row.version, message
                        ));
                    }
                },
            );
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview(&mut self, failed: bool) {
        self.inspected = true;
        self.installed = true;
        self.confirm = false;
        self.prepared = if failed {
            None
        } else {
            Some(Arc::new(msi::Prepared {
                stage: r"C:\Users\演示\AppData\Local\ZiDevTools\updates\msi-示例标识".into(),
                directory: r"C:\Users\演示\AppData\Local\Programs\ZiDevTools".into(),
                installed_version: "0.81.0".into(),
                version: "0.83.0".into(),
                bytes: 9_744_384,
                changes: vec!["程序更新检查 0.6.0 → 0.7.0".into()],
            }))
        };
        self.message = if failed {
            "合成示例，未执行安装：安装器取消（1602）；旧产品与双EXE已核验，使用旧版继续。"
        } else {
            "合成示例，未下载或安装：签名与包身份核验通过，等待明确确认。"
        }
        .into();
        self.error = failed;
    }
}
