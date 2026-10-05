//! Explicit bounded official release download; no installation or execution.
use super::{Asset, REPOSITORY, Report, delta_files, verification};
use anyhow::{Context, Result, ensure};
use eframe::egui;
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};
const MAX_ASSET: u64 = 128 * 1024 * 1024;

fn cancelled(cancel: &AtomicBool) -> Result<()> {
    ensure!(!cancel.load(Ordering::Relaxed), "下载或保存已取消");
    Ok(())
}
fn allowed_redirect(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && match url.host_str() {
            Some("github.com") => url
                .path()
                .starts_with("/ax2/zi-devtools/releases/download/"),
            Some("release-assets.githubusercontent.com" | "objects.githubusercontent.com") => true,
            _ => false,
        }
}
fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() <= 3 && allowed_redirect(attempt.url()) {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .user_agent(concat!("ZiDevTools/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("无法创建下载连接")
}
fn read_body(
    mut reader: impl Read,
    expected: u64,
    limit: u64,
    cancel: &AtomicBool,
    progress: &AtomicU64,
) -> Result<Vec<u8>> {
    ensure!(expected > 0 && expected <= limit, "发布附件超过大小限制");
    let mut result = Vec::new();
    let mut buffer = [0; 65536];
    loop {
        cancelled(cancel)?;
        let n = reader
            .read(&mut buffer)
            .map_err(|_| anyhow::anyhow!("下载中断或超时，请重新下载"))?;
        if n == 0 {
            break;
        }
        ensure!(
            result.len() as u64 + n as u64 <= expected,
            "下载内容超过发布大小"
        );
        result.extend_from_slice(&buffer[..n]);
        progress.store(result.len() as u64, Ordering::Relaxed);
    }
    ensure!(
        result.len() as u64 == expected,
        "下载不完整，与发布大小不一致"
    );
    cancelled(cancel)?;
    Ok(result)
}
fn fetch(
    client: &reqwest::blocking::Client,
    url: &str,
    expected: u64,
    limit: u64,
    cancel: &AtomicBool,
    progress: &AtomicU64,
) -> Result<Vec<u8>> {
    cancelled(cancel)?;
    ensure!(expected > 0 && expected <= limit, "发布附件超过大小限制");
    let response = client
        .get(url)
        .header("Accept-Encoding", "identity")
        .send()
        .map_err(|_| anyhow::anyhow!("无法下载官方发布附件，请检查网络后重试"))?;
    ensure!(
        response.status() == reqwest::StatusCode::OK,
        "下载服务返回HTTP {}",
        response.status().as_u16()
    );
    ensure!(
        response.content_length().is_none_or(|n| n == expected),
        "响应大小与发布信息不一致"
    );
    read_body(response, expected, limit, cancel, progress)
}
#[derive(Clone, Debug)]
pub struct Downloaded {
    pub name: String,
    pub version: semver::Version,
    pub sha256: String,
    pub bytes: Arc<[u8]>,
}
pub fn download(
    report: &Report,
    asset: &Asset,
    cancel: &AtomicBool,
    progress: &AtomicU64,
) -> Result<Downloaded> {
    cancelled(cancel)?;
    ensure!(report.version.to_string().len() <= 128, "版本字段超出限制");
    let base = format!("{REPOSITORY}/releases/download/v{}/", report.version);
    ensure!(
        report.url == format!("{REPOSITORY}/releases/tag/v{}", report.version),
        "不支持非官方发布"
    );
    ensure!(
        asset.browser_download_url == format!("{base}{}", asset.name)
            && report.assets.iter().any(|a| a.name == asset.name
                && a.size == asset.size
                && a.browser_download_url == asset.browser_download_url),
        "附件不属于本次官方发布"
    );
    ensure!(
        [
            format!("ZiDevTools-{}-windows-x64.exe", report.version),
            format!("ZiDevTools-{}-windows-x64.msi", report.version),
            format!("ZiDevToolsMcp-{}-windows-x64.exe", report.version)
        ]
        .contains(&asset.name),
        "不支持该附件名称"
    );
    ensure!(
        asset.size > 0 && asset.size <= MAX_ASSET,
        "发布附件超过128 MiB限制"
    );
    let sums = report
        .assets
        .iter()
        .find(|a| {
            a.name == "SHA256SUMS.txt" && a.browser_download_url == format!("{base}SHA256SUMS.txt")
        })
        .context("发布缺少官方SHA256SUMS.txt，不能开始下载")?;
    let client = client()?;
    let sums = fetch(
        &client,
        &sums.browser_download_url,
        sums.size,
        8192,
        cancel,
        &AtomicU64::new(0),
    )?;
    let sums = verification::checksums(&sums, &report.version)?;
    let expected = sums.get(&asset.name).context("清单缺少所选附件")?;
    let bytes = fetch(
        &client,
        &asset.browser_download_url,
        asset.size,
        MAX_ASSET,
        cancel,
        progress,
    )?;
    let mut hash = Sha256::new();
    for chunk in bytes.chunks(65536) {
        cancelled(cancel)?;
        hash.update(chunk);
    }
    let sha256 = format!("{:x}", hash.finalize());
    ensure!(
        &sha256 == expected,
        "下载摘要不匹配，内容已丢弃，请重新检查版本后重试"
    );
    cancelled(cancel)?;
    Ok(Downloaded {
        name: asset.name.clone(),
        version: report.version.clone(),
        sha256,
        bytes: bytes.into(),
    })
}
enum Completion {
    Ready(Downloaded),
    Saved(PathBuf),
}
struct Job {
    receiver: mpsc::Receiver<Result<Completion, String>>,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicU64>,
    total: u64,
}
#[derive(Default)]
pub struct State {
    pending: Option<Job>,
    preview: Option<Downloaded>,
    unsaved: bool,
    message: String,
    error: bool,
}
impl Drop for State {
    fn drop(&mut self) {
        if let Some(job) = &self.pending {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
}
impl State {
    pub fn has_preview(&self) -> bool {
        self.preview.is_some()
    }
    pub fn has_work(&self) -> bool {
        self.unsaved || self.pending.is_some()
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    fn spawn(
        &mut self,
        ctx: &egui::Context,
        total: u64,
        work: impl FnOnce(&AtomicBool, &AtomicU64) -> Result<Completion> + Send + 'static,
    ) {
        if self.busy() {
            return;
        }
        let (tx, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker = cancel.clone();
        let progress = Arc::new(AtomicU64::new(0));
        let worker_progress = progress.clone();
        let ctx = ctx.clone();
        match std::thread::Builder::new()
            .name("release-download".into())
            .spawn(move || {
                let _ = tx.send(work(&worker, &worker_progress).map_err(|e| e.to_string()));
                ctx.request_repaint();
            }) {
            Ok(_) => {
                self.pending = Some(Job {
                    receiver,
                    cancel,
                    progress,
                    total,
                });
                self.message = "后台处理中…".into();
                self.error = false;
            }
            Err(_) => {
                self.message = "无法启动后台下载任务".into();
                self.error = true;
            }
        }
    }
    pub fn start(&mut self, report: Report, asset: Asset, ctx: &egui::Context) {
        if self.has_work() {
            return;
        }
        self.spawn(ctx, asset.size, move |cancel, progress| {
            download(&report, &asset, cancel, progress).map(Completion::Ready)
        });
    }
    pub fn poll(&mut self) {
        let Some(job) = &self.pending else {
            return;
        };
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Disconnected) => Err("后台下载任务未完成".into()),
            Err(mpsc::TryRecvError::Empty) => return,
        };
        let cancelled = self
            .pending
            .take()
            .is_some_and(|j| j.cancel.load(Ordering::Relaxed));
        match result {
            Ok(Completion::Saved(path)) => {
                self.unsaved = false;
                self.message = format!("已保存新文件：{}；未安装或执行。", path.display());
                self.error = false;
            }
            Ok(Completion::Ready(preview)) if !cancelled => {
                self.preview = Some(preview);
                self.unsaved = true;
                self.message = "下载与摘要校验通过，内容在内存中；请选择保存位置。".into();
                self.error = false;
            }
            Ok(_) => {
                self.message = "操作已取消，未保存下载内容。".into();
                self.error = false;
            }
            Err(error) => {
                self.message = error;
                self.error = true;
            }
        }
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if self.pending.is_none() && self.preview.is_none() && self.message.is_empty() {
            return;
        }
        ui.add_space(8.0);
        ui.separator();
        ui.label(egui::RichText::new("下载与保存").strong());
        if let Some(job) = &self.pending {
            let done = job.progress.load(Ordering::Relaxed);
            ui.add(
                egui::ProgressBar::new(if job.total == 0 {
                    0.0
                } else {
                    done as f32 / job.total as f32
                })
                .text(format!(
                    "{:.2} / {:.2} MiB",
                    done as f64 / 1048576.0,
                    job.total as f64 / 1048576.0
                )),
            );
            if ui.button("取消后台任务").clicked() {
                job.cancel.store(true, Ordering::Relaxed);
            }
            ui.weak("网络等待最多30秒；取消在当前请求/读取返回后生效。保存任务等待文件写入返回。");
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
        if let Some(preview) = &self.preview {
            ui.label(format!(
                "{} · {} · {:.2} MiB",
                preview.version,
                preview.name,
                preview.bytes.len() as f64 / 1048576.0
            ));
            ui.horizontal_wrapped(|ui| {
                ui.monospace(&preview.sha256);
                if ui.small_button("复制摘要").clicked() {
                    ui.ctx().copy_text(preview.sha256.clone());
                }
            });
            let preview = preview.clone();
            ui.add_enabled_ui(!self.busy(), |ui| {
                #[cfg(windows)]
                if ui.button("选择位置并另存新文件").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_file_name(&preview.name)
                        .save_file()
                    {
                        self.spawn(ui.ctx(), 0, move |cancel, _| {
                            delta_files::save_new(&path, &preview.bytes, cancel)
                                .map(Completion::Saved)
                        });
                    }
                }
                if ui.button("丢弃下载预览").clicked() {
                    self.preview = None;
                    self.unsaved = false;
                    self.message = "预览已丢弃，已保存的文件不会删除。".into();
                    self.error = false;
                }
            });
            ui.weak("SHA-256与官方同版清单一致，尚未验证发布者签名。不会自动运行、安装或修改Windows Installer状态。");
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self, failure: bool) {
        self.error = failure;
        self.message = if failure {
            "下载不完整，与发布大小不一致（合成演示）；没有保存或执行。"
        } else {
            "下载与摘要校验通过（合成演示），内容尚未保存。"
        }
        .into();
        if !failure {
            self.preview = Some(Downloaded {
                name: "ZiDevTools-0.81.0-windows-x64.exe".into(),
                version: semver::Version::new(0, 81, 0),
                sha256: "a".repeat(64),
                bytes: vec![0; 1024].into(),
            });
        }
        self.unsaved = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redirect_hosts_protocol_and_credentials_are_restricted() {
        for url in [
            "https://release-assets.githubusercontent.com/a?temporary=fixture",
            "https://objects.githubusercontent.com/a",
            "https://github.com/ax2/zi-devtools/releases/download/v1.0.0/a.exe",
        ] {
            assert!(allowed_redirect(&reqwest::Url::parse(url).unwrap()));
        }
        for url in [
            "http://release-assets.githubusercontent.com/a",
            "https://github.com/evil/releases/download/a",
            "https://release-assets.githubusercontent.com.evil.example/a",
            "https://user@objects.githubusercontent.com/a",
            "https://objects.githubusercontent.com:444/a",
            "https://example.com/a",
        ] {
            assert!(!allowed_redirect(&reqwest::Url::parse(url).unwrap()));
        }
    }
    #[test]
    fn body_bounds_truncation_growth_and_cancel_are_errors() {
        let cancel = AtomicBool::new(false);
        let progress = AtomicU64::new(0);
        assert_eq!(
            read_body(&b"abc"[..], 3, 3, &cancel, &progress).unwrap(),
            b"abc"
        );
        assert_eq!(progress.load(Ordering::Relaxed), 3);
        for (size, limit) in [(4, 4), (2, 2), (0, 3), (3, 2)] {
            assert!(read_body(&b"abc"[..], size, limit, &cancel, &progress).is_err());
        }
        cancel.store(true, Ordering::Relaxed);
        assert!(read_body(&b"abc"[..], 3, 3, &cancel, &progress).is_err());
    }
    #[test]
    fn http_failure_redirect_length_and_partial_body_are_rejected() {
        use std::{io::Write, net::TcpListener};
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(2))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        for response in [
            "HTTP/1.1 302 Found\r\nContent-Length: 0\r\n\r\n",
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nabcd",
            "HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\na",
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/", listener.local_addr().unwrap());
            let thread = std::thread::spawn(move || {
                let (mut s, _) = listener.accept().unwrap();
                s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let _ = s.read(&mut [0; 4096]);
                s.write_all(response.as_bytes()).unwrap();
            });
            assert!(
                fetch(
                    &client,
                    &url,
                    3,
                    3,
                    &AtomicBool::new(false),
                    &AtomicU64::new(0)
                )
                .is_err()
            );
            thread.join().unwrap();
        }
    }
    #[test]
    fn saved_commit_wins_late_cancel_and_drop_cancels_worker() {
        let (tx, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(true));
        let mut state = State {
            pending: Some(Job {
                receiver,
                cancel: cancel.clone(),
                progress: Arc::new(AtomicU64::new(0)),
                total: 0,
            }),
            unsaved: true,
            preview: None,
            message: String::new(),
            error: false,
        };
        tx.send(Ok(Completion::Saved(PathBuf::from("fixture.exe"))))
            .unwrap();
        state.poll();
        assert!(!state.has_work());
        assert!(state.message.contains("已保存"));
        let (_, receiver) = mpsc::channel();
        cancel.store(false, Ordering::Relaxed);
        state.pending = Some(Job {
            receiver,
            cancel: cancel.clone(),
            progress: Arc::new(AtomicU64::new(0)),
            total: 1,
        });
        drop(state);
        assert!(cancel.load(Ordering::Relaxed));
    }
    #[test]
    fn foreign_metadata_is_rejected_before_any_request() {
        let asset = Asset {
            name: "ZiDevTools-1.0.0-windows-x64.exe".into(),
            size: 3,
            browser_download_url: "https://example.com/file.exe".into(),
        };
        let report = Report {
            version: semver::Version::new(1, 0, 0),
            url: format!("{REPOSITORY}/releases/tag/v1.0.0"),
            notes: String::new(),
            assets: vec![asset.clone()],
        };
        assert!(
            download(&report, &asset, &AtomicBool::new(false), &AtomicU64::new(0))
                .unwrap_err()
                .to_string()
                .contains("官方发布")
        );
    }
    #[test]
    fn cancelled_late_preview_never_becomes_unsaved_work() {
        let (tx, receiver) = mpsc::channel();
        let mut state = State {
            pending: Some(Job {
                receiver,
                cancel: Arc::new(AtomicBool::new(true)),
                progress: Arc::new(AtomicU64::new(0)),
                total: 1,
            }),
            preview: None,
            unsaved: false,
            message: String::new(),
            error: false,
        };
        tx.send(Ok(Completion::Ready(Downloaded {
            name: "fixture.exe".into(),
            version: semver::Version::new(1, 0, 0),
            sha256: "a".repeat(64),
            bytes: vec![0].into(),
        })))
        .unwrap();
        state.poll();
        assert!(!state.has_work());
        assert!(state.preview.is_none());
    }
}
