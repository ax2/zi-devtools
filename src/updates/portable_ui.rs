//! Explicit prepare/confirm/exit flow; preparation runs outside the UI thread.
use super::{Report, download, portable};
use anyhow::{Context, Result, ensure};
use eframe::egui;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};

struct Job {
    receiver: mpsc::Receiver<Result<portable::Prepared, String>>,
    cancel: Arc<AtomicBool>,
    desktop: Arc<AtomicU64>,
    mcp: Arc<AtomicU64>,
    total: u64,
}
#[derive(Default)]
pub struct State {
    job: Option<Job>,
    prepared: Option<Arc<portable::Prepared>>,
    pub message: String,
    error: bool,
    confirm: bool,
    history: Vec<(portable::Prepared, portable::Receipt)>,
    inspected: bool,
    eligibility_error: Option<String>,
    restore: bool,
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
    desktop_progress: &AtomicU64,
    mcp_progress: &AtomicU64,
) -> Result<portable::Prepared> {
    let directory = portable::current_directory()?;
    ensure!(
        report
            .version
            .cmp_precedence(&semver::Version::parse(env!("CARGO_PKG_VERSION"))?)
            .is_gt(),
        "没有更高版本可安装"
    );
    ensure!(
        report
            .assets
            .iter()
            .any(|a| a.name == "update-manifest.json")
            && report
                .assets
                .iter()
                .any(|a| a.name == "update-manifest.sig"),
        "便携升级必须有完整发布者签名，不接受历史无签名包"
    );
    let desktop = report
        .assets
        .iter()
        .find(|a| a.name == format!("ZiDevTools-{}-windows-x64.exe", report.version))
        .context("缺少桌面程序")?;
    let mcp = report
        .assets
        .iter()
        .find(|a| a.name == format!("ZiDevToolsMcp-{}-windows-x64.exe", report.version))
        .context("缺少MCP程序")?;
    let desktop = download::download(&report, desktop, cancel, desktop_progress)?;
    let mcp = download::download(&report, mcp, cancel, mcp_progress)?;
    let signed = desktop.signed_metadata.context("桌面附件未认证")?;
    ensure!(
        mcp.signed_metadata.as_deref() == Some(signed.as_ref()),
        "两件附件的签名清单不同，请重新检查"
    );
    let helper = super::delta_files::read(&std::env::current_exe()?, false, cancel)?;
    portable::prepare(
        &directory,
        env!("CARGO_PKG_VERSION"),
        &signed.0,
        &signed.1,
        [&desktop.bytes, &mcp.bytes],
        &helper,
        cancel,
    )
}
impl State {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
    pub fn poll(&mut self) {
        let Some(job) = &self.job else { return };
        let result = match job.receiver.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("更新准备任务中断".into()),
        };
        self.job = None;
        match result {
            Ok(p) => {
                self.prepared = Some(Arc::new(p));
                self.message = "已认证并暂存新程序，当前文件尚未替换。".into();
                self.error = false;
                self.confirm = false;
                self.restore = false
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
        config: &std::path::Path,
    ) -> bool {
        if !self.inspected {
            self.inspected = true;
            match portable::current_directory() {
                Ok(dir) => match portable::history(&dir) {
                    Ok(rows) => self.history = rows,
                    Err(e) => {
                        self.message = format!("更新记录读取失败：{e}");
                        self.error = true;
                    }
                },
                Err(e) => self.eligibility_error = Some(e.to_string()),
            }
        }
        ui.separator();
        ui.label(egui::RichText::new("便携版升级").strong());
        ui.weak(
            "认证下载桌面与MCP程序，保留旧版本；确认后退出、替换并重新启动。MSI请使用安装器升级。",
        );
        if let Some(error) = &self.eligibility_error {
            ui.weak(format!("当前目录不能便携升级：{error}"));
        }
        if let Some(report) = report {
            let newer = semver::Version::parse(env!("CARGO_PKG_VERSION"))
                .is_ok_and(|current| report.version.cmp_precedence(&current).is_gt());
            let signed = ["update-manifest.json", "update-manifest.sig"]
                .iter()
                .all(|name| report.assets.iter().any(|a| a.name == *name));
            if !newer {
                ui.weak("当前发布不高于本机版本，不进行同版替换或降级。");
            }
            if !signed {
                ui.weak("当前发布没有完整签名，可下载校验，但不能用于便携升级。");
            }
            if ui
                .add_enabled(
                    !self.busy()
                        && self.prepared.is_none()
                        && newer
                        && signed
                        && self.eligibility_error.is_none(),
                    egui::Button::new("下载并准备便携版升级"),
                )
                .clicked()
            {
                let (tx, receiver) = mpsc::channel();
                let cancel = Arc::new(AtomicBool::new(false));
                let worker = cancel.clone();
                let desktop = Arc::new(AtomicU64::new(0));
                let mcp = Arc::new(AtomicU64::new(0));
                let desktop_worker = desktop.clone();
                let mcp_worker = mcp.clone();
                let total = report
                    .assets
                    .iter()
                    .filter(|a| a.name.ends_with(".exe"))
                    .map(|a| a.size)
                    .sum();
                let ctx = ui.ctx().clone();
                match std::thread::Builder::new()
                    .name("portable-update-prepare".into())
                    .spawn(move || {
                        let _ = tx.send(
                            prepare_online(report, &worker, &desktop_worker, &mcp_worker)
                                .map_err(|e| e.to_string()),
                        );
                        ctx.request_repaint();
                    }) {
                    Ok(_) => {
                        self.job = Some(Job {
                            receiver,
                            cancel,
                            desktop,
                            mcp,
                            total,
                        });
                        self.message = "正在认证和下载完整升级文件…".into();
                        self.error = false;
                    }
                    Err(_) => {
                        self.message = "无法启动升级准备任务".into();
                        self.error = true;
                    }
                }
            }
        } else if self.prepared.is_none() {
            ui.weak("先检查发布版本；仅支持高于当前版本且提供完整签名的发布。");
        }
        if let Some(job) = &self.job {
            let done = job.desktop.load(Ordering::Relaxed) + job.mcp.load(Ordering::Relaxed);
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
                ui.label("正在下载 / 校验 / 暂存");
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
        let mut requested = false;
        if let Some(p) = self.prepared.clone() {
            ui.label(format!(
                "{} → {} · {}",
                p.old_version,
                p.version,
                super::size(p.bytes)
            ));
            ui.weak(format!("更新及恢复副本：{}", p.directory.display()));
            egui::CollapsingHeader::new(format!("工具变化（{}项）", p.changes.len())).show(
                ui,
                |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(180.0)
                        .show(ui, |ui| {
                            for row in &p.changes {
                                ui.label(row);
                            }
                        });
                },
            );
            ui.checkbox(
                &mut self.confirm,
                if self.restore {
                    "同意退出程序并恢复旧版本；用户数据保持原位"
                } else {
                    "同意退出程序并替换两个EXE；保留旧版副本，用户数据保持原位"
                },
            );
            if !blockers.is_empty() {
                ui.weak("请先保存或丢弃未保存工作，并结束录屏、托管服务、下载及其它后台任务，再进行更新。");
                ui.label(format!("待处理：{}", blockers.join("、")));
            }
            if ui
                .add_enabled(
                    self.confirm && blockers.is_empty() && !self.busy(),
                    egui::Button::new(if self.restore {
                        "退出并恢复旧版本"
                    } else {
                        "退出并应用更新"
                    }),
                )
                .clicked()
            {
                match portable::launch(&p, self.restore, config) {
                    Ok(()) => requested = true,
                    Err(e) => {
                        self.message = e.to_string();
                        self.error = true;
                    }
                }
            }
            if ui.button("取消本次选择").clicked() {
                self.prepared = None;
                self.confirm = false;
                self.restore = false;
                self.message = "暂存文件保留，可在原目录检查；当前程序未被替换。".into();
            }
        }
        if !self.history.is_empty() {
            egui::CollapsingHeader::new("升级记录与恢复")
                .default_open(true)
                .show(ui, |ui| {
                    for (p, r) in &self.history {
                        ui.label(format!("{} → {} · {}", r.old_version, r.version, r.message));
                        if self.prepared.is_none()
                            && !self.busy()
                            && ui.button(format!("选择恢复 {}", p.old_version)).clicked()
                        {
                            self.prepared = Some(Arc::new(p.clone()));
                            self.restore = true;
                            self.confirm = false;
                        }
                    }
                });
        }
        requested
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview(&mut self, failed: bool) {
        self.inspected = true;
        self.error = failed;
        self.message = if failed {
            "合成演示：新程序被占用，更新失败，旧版本已恢复；未操作真实文件。"
        } else {
            "合成演示：新程序已认证并准备，尚未替换真实文件。"
        }
        .into();
        if !failed {
            self.prepared = Some(Arc::new(portable::Prepared {
                directory: std::path::PathBuf::from(
                    r"C:\ZiDevTools\.zi-update-00000000-0000-4000-8000-000000000000",
                ),
                version: "0.83.0".into(),
                old_version: env!("CARGO_PKG_VERSION").into(),
                bytes: 30_000_000,
                changes: vec!["程序更新检查 · 0.5.0 → 0.6.0".into()],
            }));
        }
    }
}
