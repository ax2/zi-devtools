//! Bounded, anonymous release discovery. Installation is deliberately separate.
pub mod delta;
pub mod delta_files;
pub mod delta_ui;
pub mod download;
pub mod signed;
pub mod verification;
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    sync::mpsc,
    time::{Duration, Instant},
};

const REPOSITORY: &str = "https://github.com/ax2/zi-devtools";
const MAX_RESPONSE: u64 = 2 * 1024 * 1024;
const INTERVAL: i64 = 24 * 60 * 60;

pub(super) fn size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1048576 {
        format!("{:.2} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.2} MiB", bytes as f64 / 1048576.0)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Channel {
    #[default]
    Stable,
    Preview,
}
impl Channel {
    fn label(self) -> &'static str {
        match self {
            Self::Stable => "稳定版",
            Self::Preview => "预览版（也包含稳定版）",
        }
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Policy {
    pub channel: Channel,
    pub automatic: bool,
    pub last_attempt: Option<i64>,
}
impl Policy {
    pub fn due(&self, now: i64) -> bool {
        self.automatic
            && self.last_attempt.is_none_or(|last| {
                now.checked_sub(last)
                    .is_some_and(|elapsed| elapsed >= INTERVAL)
            })
    }
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    draft: bool,
    prerelease: bool,
    body: Option<String>,
    assets: Vec<Asset>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    browser_download_url: String,
}
#[derive(Clone, Debug)]
pub struct Report {
    pub version: semver::Version,
    pub url: String,
    pub notes: String,
    pub assets: Vec<Asset>,
}
fn parse(bytes: &[u8], channel: Channel) -> Result<Option<Report>, String> {
    if bytes.len() > MAX_RESPONSE as usize {
        return Err("发布信息超过大小限制".into());
    }
    let releases: Vec<Release> = if channel == Channel::Stable {
        vec![serde_json::from_slice(bytes).map_err(|_| "发布信息格式无效")?]
    } else {
        serde_json::from_slice(bytes).map_err(|_| "发布信息格式无效")?
    };
    if releases.len() > 20 {
        return Err("发布信息条目过多".into());
    }
    let mut reports = Vec::new();
    for release in releases {
        if release.draft || (channel == Channel::Stable && release.prerelease) {
            continue;
        }
        let Some(tag) = release.tag_name.strip_prefix('v') else {
            continue;
        };
        if tag.len() > 128 || !tag.is_ascii() {
            continue;
        }
        let Ok(version) = semver::Version::parse(tag) else {
            continue;
        };
        if release.prerelease == version.pre.is_empty()
            || release.html_url != format!("{REPOSITORY}/releases/tag/v{tag}")
        {
            continue;
        }
        if release.assets.len() > 32 {
            return Err("发布附件数量过多".into());
        }
        let names = [
            format!("ZiDevTools-{tag}-windows-x64.exe"),
            format!("ZiDevTools-{tag}-windows-x64.msi"),
            format!("ZiDevToolsMcp-{tag}-windows-x64.exe"),
            "SHA256SUMS.txt".into(),
            "update-manifest.json".into(),
            "update-manifest.sig".into(),
        ];
        let mut assets = Vec::new();
        for asset in release.assets {
            if names.contains(&asset.name)
                && asset.size > 0
                && asset.browser_download_url
                    == format!("{REPOSITORY}/releases/download/v{tag}/{}", asset.name)
                && !assets
                    .iter()
                    .any(|previous: &Asset| previous.name == asset.name)
            {
                assets.push(asset);
            }
        }
        reports.push(Report {
            version,
            url: release.html_url,
            notes: release
                .body
                .unwrap_or_default()
                .chars()
                .take(16_000)
                .collect(),
            assets,
        });
    }
    Ok(reports
        .into_iter()
        .max_by(|a, b| a.version.cmp_precedence(&b.version)))
}
fn fetch(channel: Channel) -> Result<Option<Report>, String> {
    let path = if channel == Channel::Stable {
        "latest"
    } else {
        "?per_page=20"
    };
    let separator = if channel == Channel::Stable { "/" } else { "" };
    fetch_url(
        channel,
        &format!("https://api.github.com/repos/ax2/zi-devtools/releases{separator}{path}"),
    )
}
fn fetch_url(channel: Channel, url: &str) -> Result<Option<Report>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("ZiDevTools/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| "无法创建更新检查连接")?;
    let response = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2026-03-10")
        .send()
        .map_err(|_| "无法连接发布服务，请检查网络或稍后重试")?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if matches!(response.status().as_u16(), 403 | 429) {
        return Err("发布服务暂时限制请求，请稍后重试".into());
    }
    if !response.status().is_success() {
        return Err(format!("发布服务返回 HTTP {}", response.status().as_u16()));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_RESPONSE)
    {
        return Err("发布信息超过大小限制".into());
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_RESPONSE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "未能完整读取发布信息")?;
    parse(&bytes, channel)
}

pub struct State {
    pending: Option<mpsc::Receiver<Result<Option<Report>, String>>>,
    pub result: Option<Result<Option<Report>, String>>,
    channel: Channel,
    started: Instant,
    pub message: String,
    verification: verification::State,
    download: download::State,
}
impl Default for State {
    fn default() -> Self {
        Self {
            pending: None,
            result: None,
            channel: Channel::Stable,
            started: Instant::now(),
            message: String::new(),
            verification: Default::default(),
            download: Default::default(),
        }
    }
}
impl State {
    pub fn begin(&mut self, channel: Channel, ctx: &egui::Context) {
        if self.pending.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        match std::thread::Builder::new()
            .name("release-check".into())
            .spawn(move || {
                let _ = tx.send(fetch(channel));
                ctx.request_repaint();
            }) {
            Ok(_) => {
                self.pending = Some(rx);
                self.channel = channel;
                self.result = None;
                self.message.clear();
            }
            Err(_) => {
                self.result = Some(Err("无法启动更新检查，请稍后重试".into()));
            }
        }
    }
    pub fn poll(&mut self) {
        self.verification.poll();
        self.download.poll();
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(result) => {
                    self.result = Some(result);
                    self.pending = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.result = Some(Err("更新检查未完成，请重试".into()));
                    self.pending = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
    }
    pub fn automatic_due(&self, policy: &Policy, now: i64) -> bool {
        self.pending.is_none()
            && self.started.elapsed() >= Duration::from_secs(5)
            && policy.due(now)
    }
    /// Returns a staged preference change and/or a request; caller saves before activation.
    pub fn ui(&mut self, ui: &mut egui::Ui, policy: &Policy) -> (Option<Policy>, bool) {
        ui.heading("程序更新");
        ui.label(format!("当前版本  {}", env!("CARGO_PKG_VERSION")));
        ui.label("检查 GitHub 公开发布信息。下载与安装由你决定；本页不会自动替换程序。");
        ui.add_space(12.0);
        let mut next = policy.clone();
        let mut check = false;
        ui.add_enabled_ui(self.pending.is_none(), |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut next.channel, Channel::Stable, "稳定版");
                ui.selectable_value(&mut next.channel, Channel::Preview, "预览版");
            });
            ui.checkbox(&mut next.automatic, "每 24 小时自动检查（默认关闭）");
            check = ui.button("立即检查").clicked();
        });
        if self.pending.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("后台检查中，最长等待约 15 秒…");
            });
        }
        if let Some(last) = policy
            .last_attempt
            .and_then(|n| chrono::DateTime::from_timestamp(n, 0))
        {
            ui.weak(format!(
                "上次请求：{}",
                last.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M")
            ));
        }
        if policy
            .last_attempt
            .is_some_and(|last| last > chrono::Utc::now().timestamp())
        {
            ui.weak("记录的检查时间晚于当前系统时间；自动检查暂缓，手动检查可刷新时间。");
        }
        if !self.message.is_empty() {
            ui.colored_label(ui.visuals().warn_fg_color, &self.message);
        }
        if let Some(result) = &self.result {
            ui.separator();
            ui.label(format!("检查频道：{}", self.channel.label()));
            match result {
                Err(error) => {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                    ui.label("未取得版本信息，无法判断是否需要更新。");
                }
                Ok(None) => {
                    ui.label("该频道暂无可识别的公开发布版本。");
                }
                Ok(Some(report)) => {
                    let current =
                        semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("package semver");
                    let status = match report.version.cmp_precedence(&current) {
                        std::cmp::Ordering::Greater => "有新版本可查看",
                        std::cmp::Ordering::Equal => "与当前发布版本相同",
                        std::cmp::Ordering::Less => "当前版本较新，无需降级",
                    };
                    ui.label(
                        egui::RichText::new(format!("{status} · {}", report.version)).strong(),
                    );
                    ui.hyperlink_to("查看 GitHub 发布页", &report.url);
                    ui.label(
                        "附件大小来自发布信息；点击下载后核对大小、摘要及该版本提供的发布签名。",
                    );
                    if report.assets.is_empty() {
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            "未找到符合命名规则的 Windows x64 附件。",
                        );
                    }
                    for asset in &report.assets {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(format!("{} · {}", asset.name, size(asset.size)));
                            if (asset.name.ends_with(".exe") || asset.name.ends_with(".msi"))
                                && ui
                                    .add_enabled(
                                        !self.download.has_work(),
                                        egui::Button::new("下载并校验"),
                                    )
                                    .clicked()
                            {
                                self.download.start(report.clone(), asset.clone(), ui.ctx());
                            }
                        });
                    }
                    ui.add_space(8.0);
                    ui.label("发布说明");
                    egui::ScrollArea::vertical()
                        .id_salt("release-notes")
                        .max_height(360.0)
                        .show(ui, |ui| {
                            ui.label(&report.notes);
                        });
                }
            }
        }
        self.download.ui(ui);
        egui::CollapsingHeader::new("已有下载文件？离线校验")
            .default_open(!self.download.has_preview())
            .show(ui, |ui| self.verification.ui(ui));
        let changed = next.channel != policy.channel || next.automatic != policy.automatic;
        if next.channel != policy.channel {
            next.last_attempt = None;
        }
        (changed.then_some(next), check)
    }
    pub fn clear_result(&mut self) {
        self.result = None;
    }
    pub fn download_has_work(&self) -> bool {
        self.download.has_work()
    }
    pub fn download_busy(&self) -> bool {
        self.download.busy()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_download(&mut self, failure: bool) {
        self.result = None;
        self.download.preview_fixture(failure);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_signed_download(&mut self, failure: bool) {
        self.result = None;
        self.download.preview_signed_fixture(failure);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_signed_report(&mut self) {
        let version = semver::Version::new(0, 83, 0);
        let assets = [
            ("ZiDevTools-0.83.0-windows-x64.exe", 27835392),
            ("ZiDevTools-0.83.0-windows-x64.msi", 9744384),
            ("ZiDevToolsMcp-0.83.0-windows-x64.exe", 2176512),
            ("SHA256SUMS.txt", 306),
            ("update-manifest.json", 25000),
            ("update-manifest.sig", 212),
        ]
        .into_iter()
        .map(|(name, size)| Asset {
            name: name.into(),
            size,
            browser_download_url: format!("{REPOSITORY}/releases/download/v0.83.0/{name}"),
        })
        .collect();
        self.result = Some(Ok(Some(Report {
            version,
            url: format!("{REPOSITORY}/releases/tag/v0.83.0"),
            notes:
                "合成界面演示，版本/附件并非实际公开发布。签名在下载时验证，列表不预先宣称已认证。"
                    .into(),
            assets,
        })));
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_verification(&mut self, failure: bool) {
        self.result = None;
        self.verification.preview_fixture(failure);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self, offline: bool) {
        self.result = Some(if offline {
            Err("无法连接发布服务，请检查网络或稍后重试".into())
        } else {
            Ok(Some(Report { version: semver::Version::new(0, 81, 0), url: format!("{REPOSITORY}/releases/tag/v0.81.0"), notes: "界面验收用合成说明：可查看已发布版本、附件大小和说明。当前为较新的开发构建，程序不会建议降级。\n\n正式发布信息需通过立即检查获取。".into(), assets: vec![Asset { name: "ZiDevTools-0.81.0-windows-x64.exe".into(), size: 27_000_000, browser_download_url: String::new() }] }))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn release(tag: &str, prerelease: bool) -> serde_json::Value {
        serde_json::json!({"tag_name":tag,"html_url":format!("{REPOSITORY}/releases/tag/{tag}"),"draft":false,"prerelease":prerelease,"body":"notes","assets":[]})
    }
    #[test]
    fn channel_order_and_semver_are_explicit() {
        let releases = serde_json::json!([
            release("v0.9.0", false),
            release("v0.10.0-beta.2", true),
            release("v0.10.0-beta.10", true)
        ]);
        let report = parse(&serde_json::to_vec(&releases).unwrap(), Channel::Preview)
            .unwrap()
            .unwrap();
        assert_eq!(report.version.to_string(), "0.10.0-beta.10");
        assert!(
            parse(
                &serde_json::to_vec(&release("v1.0.0-beta", true)).unwrap(),
                Channel::Stable
            )
            .unwrap()
            .is_none()
        );
        assert_eq!(
            semver::Version::parse("1.0.0+a")
                .unwrap()
                .cmp_precedence(&semver::Version::parse("1.0.0+b").unwrap()),
            std::cmp::Ordering::Equal
        );
    }
    #[test]
    fn rejects_untrusted_links_and_inconsistent_flags() {
        let mut value = release("v1.0.0", false);
        value["html_url"] = "https://example.com".into();
        assert!(
            parse(&serde_json::to_vec(&value).unwrap(), Channel::Stable)
                .unwrap()
                .is_none()
        );
        assert!(
            parse(
                &serde_json::to_vec(&release("v1.0.0-beta", false)).unwrap(),
                Channel::Stable
            )
            .unwrap()
            .is_none()
        );
    }
    #[test]
    fn automatic_checks_default_off_and_resist_clock_reversal() {
        let mut policy = Policy::default();
        assert!(!policy.due(1));
        policy.automatic = true;
        assert!(policy.due(1));
        policy.last_attempt = Some(100);
        assert!(!policy.due(99));
        assert!(!policy.due(100 + INTERVAL - 1));
        assert!(policy.due(100 + INTERVAL));
    }
    #[test]
    fn bounds_response_and_accepts_only_expected_assets() {
        assert!(parse(&vec![b' '; MAX_RESPONSE as usize + 1], Channel::Stable).is_err());
        let mut value = release("v1.0.0", false);
        value["assets"] = serde_json::json!([{"name":"ZiDevTools-1.0.0-windows-x64.exe","size":100,"browser_download_url":"https://evil.example/app.exe"}]);
        assert!(
            parse(&serde_json::to_vec(&value).unwrap(), Channel::Stable)
                .unwrap()
                .unwrap()
                .assets
                .is_empty()
        );
    }
    #[test]
    fn http_errors_limits_and_redirects_are_not_success() {
        use std::{io::Write, net::TcpListener};
        for (headers, body, expected) in [
            ("404 Not Found", "", "none"),
            ("429 Too Many Requests", "", "limited"),
            ("302 Found\r\nLocation: https://example.com", "", "http"),
            ("200 OK", "{}", "invalid"),
            ("200 OK\r\nContent-Length: 2097153", "", "large"),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/", listener.local_addr().unwrap());
            let thread = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = [0; 4096];
                let _ = stream.read(&mut request);
                stream
                    .write_all(
                        format!("HTTP/1.1 {headers}\r\nConnection: close\r\n\r\n{body}").as_bytes(),
                    )
                    .unwrap();
            });
            let result = fetch_url(Channel::Stable, &url);
            thread.join().unwrap();
            match expected {
                "none" => assert!(result.unwrap().is_none()),
                "limited" => assert!(result.unwrap_err().contains("限制")),
                "http" => assert!(result.unwrap_err().contains("302")),
                "invalid" => assert!(result.unwrap_err().contains("格式")),
                "large" => assert!(result.unwrap_err().contains("大小")),
                _ => unreachable!(),
            }
        }
    }
    #[test]
    #[ignore = "explicit anonymous public network check"]
    fn published_stable_release_is_readable() {
        let report = fetch(Channel::Stable)
            .unwrap()
            .expect("public stable release");
        assert!(report.version.pre.is_empty());
        assert!(
            report
                .assets
                .iter()
                .any(|a| a.name.ends_with("windows-x64.exe"))
        );
        println!(
            "public release {} with {} recognized assets",
            report.version,
            report.assets.len()
        );
    }
    #[test]
    #[ignore = "explicit anonymous public asset download, no save or execution"]
    fn published_asset_download_passes_integrity() {
        use std::sync::atomic::{AtomicBool, AtomicU64};
        let report = fetch(Channel::Stable).unwrap().expect("stable release");
        let asset = report
            .assets
            .iter()
            .find(|a| a.name.starts_with("ZiDevTools-") && a.name.ends_with(".exe"))
            .unwrap();
        let result =
            download::download(&report, asset, &AtomicBool::new(false), &AtomicU64::new(0))
                .unwrap();
        assert_eq!(result.bytes.len() as u64, asset.size);
        println!(
            "public asset {} {} bytes SHA256 {}; not saved or executed",
            result.name,
            result.bytes.len(),
            result.sha256
        );
    }
}
