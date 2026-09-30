//! Explicit capture of pasted text, saved HTML, or a public HTTPS page into a local knowledge source.
use anyhow::{Context, Result, ensure};
use eframe::egui;
use scraper::{ElementRef, Html, Selector};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    net::{IpAddr, ToSocketAddrs},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    time::Duration,
};

use crate::knowledge_sources::{Kind, Source};

const MAX_HTML: usize = 2 * 1024 * 1024;
const MAX_BODY: usize = 256 * 1024;
const MAX_OUTPUT: usize = 512 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Paste,
    HtmlFile,
    Https,
}
impl Mode {
    fn label(self) -> &'static str {
        match self {
            Self::Paste => "粘贴文字",
            Self::HtmlFile => "本机 HTML",
            Self::Https => "公开 HTTPS 网页",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Preview {
    pub title: String,
    pub source: String,
    pub kind: String,
    pub captured_at: String,
    pub body: String,
    pub content_sha256: String,
    pub filename: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    Created(PathBuf),
    Duplicate(PathBuf),
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn bounded_line(input: &str, max_chars: usize, label: &str) -> Result<String> {
    let value = input.trim();
    ensure!(
        !value.is_empty()
            && value.chars().count() <= max_chars
            && !value.chars().any(char::is_control),
        "{label}为空、过长或包含控制字符"
    );
    Ok(value.to_owned())
}

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(a == 0
                || a == 10
                || a == 127
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && (b == 168 || (b == 0 && c == 0) || (b == 0 && c == 2)))
                || (a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return public_ip(IpAddr::V4(v4));
            }
            let octets = ip.octets();
            let global_unicast = (octets[0] & 0xe0) == 0x20;
            let documentation = octets[..4] == [0x20, 0x01, 0x0d, 0xb8];
            global_unicast && !documentation
        }
    }
}

fn validate_url(input: &str) -> Result<reqwest::Url> {
    ensure!(input.len() <= 2048, "网页 URL 超过 2048 字节");
    let mut url = reqwest::Url::parse(input.trim()).context("网页 URL 格式无效")?;
    ensure!(url.scheme() == "https", "仅支持公开 HTTPS 网页");
    ensure!(
        url.port_or_known_default() == Some(443),
        "只支持 HTTPS 默认端口 443"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "URL 不能包含账号或密码"
    );
    let host = url.host_str().context("URL 缺少主机名")?;
    ensure!(
        host != "localhost" && !host.ends_with(".localhost") && !host.ends_with(".local"),
        "本机或局域网地址不能作为公开网页抓取"
    );
    if let Ok(ip) = host.parse::<IpAddr>() {
        ensure!(public_ip(ip), "不允许抓取本机、私有或保留地址");
    }
    for (key, _) in url.query_pairs() {
        let key = key.to_ascii_lowercase();
        ensure!(
            ![
                "token",
                "key",
                "secret",
                "password",
                "passwd",
                "auth",
                "session",
                "code",
                "credential",
                "access_token",
                "api_key"
            ]
            .iter()
            .any(|sensitive| key == *sensitive || key.ends_with(&format!("_{sensitive}"))),
            "URL 查询参数可能包含凭据，请改用无凭据的公开网页地址"
        );
    }
    url.set_fragment(None);
    Ok(url)
}

fn visible_text(element: ElementRef<'_>) -> String {
    element
        .descendants()
        .filter_map(|node| {
            let text = node.value().as_text()?;
            let hidden = node.ancestors().filter_map(ElementRef::wrap).any(|parent| {
                matches!(
                    parent.value().name(),
                    "script"
                        | "style"
                        | "noscript"
                        | "template"
                        | "nav"
                        | "footer"
                        | "aside"
                        | "header"
                )
            });
            (!hidden).then_some(text.trim().to_owned())
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn html_body(html: &str) -> Result<(String, String)> {
    let doc = Html::parse_document(html);
    let title_selector = Selector::parse("title").expect("static selector");
    let title = doc
        .select(&title_selector)
        .next()
        .map(visible_text)
        .unwrap_or_default();
    let mut best = String::new();
    for selector in ["main", "article", "body"] {
        let selector = Selector::parse(selector).expect("static selector");
        for element in doc.select(&selector) {
            let text = visible_text(element);
            if text.len() > best.len() {
                best = text;
            }
        }
        if best.chars().count() >= 80 {
            break;
        }
    }
    ensure!(
        !best.trim().is_empty(),
        "网页没有可提取的正文；请从浏览器复制可见文字"
    );
    ensure!(best.len() <= MAX_BODY, "网页提取正文超过 256 KiB");
    Ok((title, best))
}

fn make_preview(title: &str, source: &str, kind: &str, body: &str) -> Result<Preview> {
    let title = bounded_line(title, 180, "标题")?;
    let source = bounded_line(source, 2048, "来源")?;
    let body = body.replace("\r\n", "\n").replace('\r', "\n");
    ensure!(
        !body.trim().is_empty() && body.len() <= MAX_BODY && !body.contains('\0'),
        "正文为空、超过 256 KiB 或包含 NUL"
    );
    let content_sha256 = sha256(body.as_bytes());
    let identity = sha256(format!("{kind}\n{source}\n{content_sha256}").as_bytes());
    Ok(Preview {
        title,
        source,
        kind: kind.into(),
        captured_at: chrono::Utc::now().to_rfc3339(),
        body,
        content_sha256,
        filename: format!("zi-capture-{}.md", &identity[..20]),
    })
}

pub fn preview_paste(title: &str, source: &str, body: &str, html: bool) -> Result<Preview> {
    if html {
        ensure!(body.len() <= MAX_HTML, "粘贴的 HTML 超过 2 MiB");
        let (parsed_title, text) = html_body(body)?;
        make_preview(
            if title.trim().is_empty() {
                &parsed_title
            } else {
                title
            },
            if source.trim().is_empty() {
                "手动粘贴 HTML"
            } else {
                source
            },
            "粘贴 HTML",
            &text,
        )
    } else {
        make_preview(
            title,
            if source.trim().is_empty() {
                "手动粘贴文字"
            } else {
                source
            },
            "粘贴文字",
            body,
        )
    }
}

pub fn preview_html_file(
    path: &Path,
    title_override: &str,
    source_override: &str,
) -> Result<Preview> {
    let metadata = fs::symlink_metadata(path).context("无法读取 HTML 文件")?;
    ensure!(
        metadata.file_type().is_file() && metadata.len() <= MAX_HTML as u64,
        "HTML 必须是 2 MiB 内的普通文件"
    );
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_HTML as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= MAX_HTML, "HTML 超过 2 MiB");
    let html = std::str::from_utf8(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))
        .context("HTML 不是 UTF-8；请先转换编码")?;
    let (parsed_title, text) = html_body(html)?;
    let file_source = path.display().to_string();
    make_preview(
        if title_override.trim().is_empty() {
            &parsed_title
        } else {
            title_override
        },
        if source_override.trim().is_empty() {
            &file_source
        } else {
            source_override
        },
        "本机 HTML",
        &text,
    )
}

pub fn preview_https(input: &str) -> Result<Preview> {
    let url = validate_url(input)?;
    let host = url.host_str().context("URL 缺少主机名")?;
    let addresses = (host, 443)
        .to_socket_addrs()
        .context("无法解析公开网页地址")?
        .collect::<Vec<_>>();
    ensure!(
        !addresses.is_empty() && addresses.len() <= 32,
        "网页地址解析结果异常"
    );
    ensure!(
        addresses.iter().all(|address| public_ip(address.ip())),
        "网页地址解析到本机、私有或保留地址"
    );
    let selected = addresses
        .iter()
        .find(|address| address.is_ipv4())
        .unwrap_or(&addresses[0]);
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .resolve(host, *selected)
        .build()
        .context("无法创建网页读取客户端")?;
    let response = client
        .get(url.clone())
        .header("Accept", "text/html")
        .header("User-Agent", "ZiDevTools/0.53 (explicit local capture)")
        .send()
        .context("公开网页连接失败或超时")?;
    ensure!(
        response.status().is_success(),
        "网页返回 HTTP {}；不跟随重定向",
        response.status().as_u16()
    );
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    ensure!(
        content_type.to_ascii_lowercase().contains("text/html"),
        "网页响应不是 HTML；可改用粘贴或文件导入"
    );
    let mut bytes = Vec::new();
    response.take(MAX_HTML as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= MAX_HTML, "网页 HTML 超过 2 MiB");
    let html = std::str::from_utf8(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))
        .context("网页 HTML 不是 UTF-8；请从浏览器复制可见文字")?;
    let (title, body) = html_body(html)?;
    make_preview(&title, url.as_str(), "公开 HTTPS", &body)
}

fn markdown(preview: &Preview) -> Result<Vec<u8>> {
    let data = format!(
        "# {}\n\n来源类型：{}\n来源：{}\n采集时间：{}\n\n<!-- zi-devtools-content-sha256:{} -->\n\n{}\n",
        preview.title,
        preview.kind,
        preview.source,
        preview.captured_at,
        preview.content_sha256,
        preview.body.trim()
    );
    ensure!(data.len() <= MAX_OUTPUT, "收集文档超过 512 KiB");
    Ok(data.into_bytes())
}

pub fn save(preview: &Preview, source: &Source) -> Result<SaveOutcome> {
    ensure!(source.kind == Kind::Directory, "请选择目录型知识源");
    ensure!(
        preview.filename.starts_with("zi-capture-")
            && preview.filename.ends_with(".md")
            && preview.filename.len() == "zi-capture-".len() + 20 + ".md".len()
            && preview.filename.as_bytes()["zi-capture-".len().."zi-capture-".len() + 20]
                .iter()
                .all(|byte| byte.is_ascii_hexdigit()),
        "收集文档文件名无效"
    );
    let meta = fs::symlink_metadata(&source.path).context("目标知识源目录不存在")?;
    ensure!(
        meta.file_type().is_dir(),
        "目标知识源不是普通目录或已变为链接"
    );
    ensure!(
        source.path.canonicalize()? == source.path,
        "目标知识源目录路径已变化"
    );
    let path = source.path.join(&preview.filename);
    ensure!(
        path.parent() == Some(source.path.as_path()),
        "输出路径超出知识源目录"
    );
    let bytes = markdown(preview)?;
    let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(&path)?;
            ensure!(
                metadata.file_type().is_file() && metadata.len() <= MAX_OUTPUT as u64,
                "同名文件不是普通文件或过大，不能视为重复"
            );
            let existing = fs::read(&path)?;
            let marker = format!(
                "<!-- zi-devtools-content-sha256:{} -->",
                preview.content_sha256
            );
            ensure!(
                String::from_utf8_lossy(&existing).contains(&marker),
                "同名文件内容不同，拒绝覆盖"
            );
            return Ok(SaveOutcome::Duplicate(path));
        }
        Err(error) => return Err(error).context("无法新建收集文档"),
    };
    file.write_all(&bytes).context("写入收集文档失败")?;
    file.sync_all().context("同步收集文档失败")?;
    Ok(SaveOutcome::Created(path))
}

enum Event {
    Ready(Result<Preview, String>),
}

pub struct State {
    mode: Mode,
    title: String,
    source: String,
    body: String,
    html: bool,
    html_path: String,
    https_url: String,
    selected_source: String,
    running: Option<Receiver<Event>>,
    preview: Option<Preview>,
    message: String,
}
impl Default for State {
    fn default() -> Self {
        Self {
            mode: Mode::Paste,
            title: String::new(),
            source: String::new(),
            body: String::new(),
            html: false,
            html_path: String::new(),
            https_url: String::new(),
            selected_source: String::new(),
            running: None,
            preview: None,
            message: String::new(),
        }
    }
}
impl State {
    fn start(&mut self) {
        if self.running.is_some() {
            return;
        }
        let mode = self.mode;
        let title = self.title.clone();
        let source = self.source.clone();
        let body = self.body.clone();
        let html = self.html;
        let html_path = self.html_path.clone();
        let https_url = self.https_url.clone();
        let (tx, rx) = mpsc::channel();
        self.running = Some(rx);
        self.preview = None;
        self.message = "正在准备可保存的正文预览…".into();
        std::thread::spawn(move || {
            let result = match mode {
                Mode::Paste => preview_paste(&title, &source, &body, html),
                Mode::HtmlFile => preview_html_file(
                    Path::new(html_path.trim().trim_matches('"')),
                    &title,
                    &source,
                ),
                Mode::Https => preview_https(&https_url),
            };
            let _ = tx.send(Event::Ready(result.map_err(|error| format!("{error:#}"))));
        });
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self, sources: &[Source]) {
        self.mode = Mode::Paste;
        self.title = "浏览器与 Agent 收集示例".into();
        self.source = "https://example.com/notes".into();
        self.body = "本机知识库可以收集网页、浏览器保存的 HTML 和 Agent 对话结果。保存后扫描知识源，再同步索引。".into();
        self.selected_source = sources
            .first()
            .map(|source| source.id.clone())
            .unwrap_or_default();
        self.preview = preview_paste(&self.title, &self.source, &self.body, false).ok();
        self.message = "合成预览：正文尚未写入磁盘。".into();
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, sources: &mut crate::knowledge_sources::State) {
        if let Some(rx) = &self.running {
            match rx.try_recv() {
                Ok(Event::Ready(Ok(preview))) => {
                    self.message = format!(
                        "预览已准备：{} 字节正文；检查后再保存。",
                        preview.body.len()
                    );
                    self.preview = Some(preview);
                    self.running = None;
                }
                Ok(Event::Ready(Err(error))) => {
                    self.message = error;
                    self.running = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.message = "正文准备任务意外结束".into();
                    self.running = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ui.ctx().request_repaint_after(Duration::from_millis(100));
                }
            }
        }
        ui.heading("网页与对话收集");
        ui.label("把公开网页、浏览器保存的 HTML，或复制的 Agent 对话正文预览后保存到本机目录型知识源。不会读取浏览器 Cookie 或自动抓取登录页。");
        ui.add_space(8.0);
        let mut changed = false;
        ui.add_enabled_ui(self.running.is_none(), |ui| {
            ui.horizontal(|ui| {
                for mode in [Mode::Paste, Mode::HtmlFile, Mode::Https] {
                    changed |= ui.selectable_value(&mut self.mode, mode, mode.label()).changed();
                }
            });
            match self.mode {
                Mode::Paste => {
                    ui.label("标题");
                    changed |= ui.add(egui::TextEdit::singleline(&mut self.title).desired_width(ui.available_width().min(900.0))).changed();
                    ui.label("来源标签或 URL（可选）");
                    changed |= ui.add(egui::TextEdit::singleline(&mut self.source).desired_width(ui.available_width().min(900.0))).changed();
                    changed |= ui.checkbox(&mut self.html, "粘贴的是 HTML 源码，提取可见文字").changed();
                    ui.label("正文");
                    changed |= ui.add(egui::TextEdit::multiline(&mut self.body).desired_rows(8).desired_width(ui.available_width().min(920.0))).changed();
                }
                Mode::HtmlFile => {
                    ui.label("本机 HTML 文件路径");
                    ui.horizontal(|ui| {
                        changed |= ui.add(egui::TextEdit::singleline(&mut self.html_path).desired_width(ui.available_width().min(680.0))).changed();
                        if ui.button("选择 HTML…").clicked() && let Some(path) = rfd::FileDialog::new().add_filter("HTML", &["html", "htm"]).pick_file() {
                            self.html_path = path.display().to_string(); changed = true;
                        }
                    });
                    ui.label("标题覆盖（可选）");
                    changed |= ui.add(egui::TextEdit::singleline(&mut self.title).desired_width(ui.available_width().min(900.0))).changed();
                    ui.label("来源 URL 或标签（可选）");
                    changed |= ui.add(egui::TextEdit::singleline(&mut self.source).desired_width(ui.available_width().min(900.0))).changed();
                }
                Mode::Https => {
                    ui.label("公开 HTTPS 网页 URL");
                    changed |= ui.add(egui::TextEdit::singleline(&mut self.https_url).desired_width(ui.available_width().min(900.0))).changed();
                    ui.small("只请求公开页面，不带浏览器认证或 Cookie；不跟随重定向。动态/登录页面请改用浏览器复制正文。");
                }
            }
        });
        if changed {
            self.preview = None;
            self.message = "输入已变化，请重新准备预览。".into();
        }
        if ui
            .add_enabled(self.running.is_none(), egui::Button::new("准备正文预览"))
            .clicked()
        {
            self.start();
        }
        if self.running.is_some() {
            ui.spinner();
        }
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        if let Some(preview) = &self.preview {
            ui.separator();
            ui.strong(&preview.title);
            ui.small(format!(
                "{} · {} · 文件 {}",
                preview.kind, preview.source, preview.filename
            ));
            ui.small(format!("正文 SHA-256：{}", preview.content_sha256));
            ui.group(|ui| {
                ui.set_min_width(ui.available_width().min(900.0));
                let excerpt: String = preview.body.chars().take(2500).collect();
                ui.label(&excerpt);
                if preview.body.chars().count() > 2500 {
                    ui.small("这里只显示前 2500 字；保存时包含完整正文（最多 256 KiB）。");
                }
            });
            ui.add_space(8.0);
            ui.label("目标知识源目录");
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("capture-source")
                    .selected_text(
                        sources
                            .sources()
                            .iter()
                            .find(|source| source.id == self.selected_source)
                            .map_or("请选择目录源", |source| source.name.as_str()),
                    )
                    .show_ui(ui, |ui| {
                        for source in sources
                            .sources()
                            .iter()
                            .filter(|source| source.kind == Kind::Directory)
                        {
                            ui.selectable_value(
                                &mut self.selected_source,
                                source.id.clone(),
                                format!("{} · {}", source.name, source.path.display()),
                            );
                        }
                    });
                if ui
                    .add_enabled(
                        !sources.is_locked(),
                        egui::Button::new("创建/选择 Zi 收集箱"),
                    )
                    .clicked()
                {
                    self.message = match sources.ensure_inbox() {
                        Ok(id) => {
                            self.selected_source = id;
                            "已选择本机 Zi 收集箱。".into()
                        }
                        Err(error) => format!("无法建立收集箱：{error:#}"),
                    };
                }
            });
            if let Some(source) = sources
                .sources()
                .iter()
                .find(|source| source.id == self.selected_source)
            {
                ui.small(format!(
                    "将新建于：{}",
                    source.path.join(&preview.filename).display()
                ));
                if ui
                    .add_enabled(self.running.is_none(), egui::Button::new("保存到知识源"))
                    .clicked()
                {
                    self.message = match save(preview, source) {
                        Ok(SaveOutcome::Created(path)) => format!(
                            "已保存：{}。下一步在“本地知识源”扫描/刷新，再到“增量知识索引”同步。",
                            path.display()
                        ),
                        Ok(SaveOutcome::Duplicate(path)) => format!(
                            "重复内容已跳过：{}。需要查询时检查知识源扫描与索引状态。",
                            path.display()
                        ),
                        Err(error) => format!("保存失败：{error:#}"),
                    };
                }
            }
        }
        ui.small("内容默认只在内存中预览；点击保存后才写入所选知识源。保存不会自动扫描目录或更新索引，以免读取其他未确认文件。");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge_sources::add_source;
    #[test]
    fn html_extraction_and_duplicate_capture_are_bounded() {
        let root = std::env::temp_dir().join(format!("zi-capture-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let html = "<html><head><title>示例标题</title><script>secret</script></head><body><nav>菜单</nav><article><h1>示例标题</h1><p>正文内容 ZX-42</p><script>ignore</script></article><footer>页脚</footer></body></html>";
        let path = root.join("page.html");
        fs::write(&path, html).unwrap();
        let preview = preview_html_file(&path, "", "https://example.com/page").unwrap();
        assert_eq!(preview.title, "示例标题");
        assert!(preview.body.contains("ZX-42"));
        assert!(!preview.body.contains("secret"));
        assert!(!preview.body.contains("菜单"));
        let mut sources = Vec::new();
        let source = add_source(&mut sources, &root, "Fixture", "").unwrap();
        let created = save(&preview, &source).unwrap();
        assert!(matches!(created, SaveOutcome::Created(_)));
        assert!(matches!(
            save(&preview, &source).unwrap(),
            SaveOutcome::Duplicate(_)
        ));
        let changed = preview_paste("另一标题", "Agent 对话", "不同内容", false).unwrap();
        assert!(matches!(
            save(&changed, &source).unwrap(),
            SaveOutcome::Created(_)
        ));
        let collision = preview_paste("冲突标题", "冲突来源", "碰撞内容", false).unwrap();
        let collision_path = root.join(&collision.filename);
        fs::write(&collision_path, "已有文件，不能覆盖").unwrap();
        assert!(save(&collision, &source).is_err());
        assert_eq!(
            fs::read_to_string(collision_path).unwrap(),
            "已有文件，不能覆盖"
        );
        let mut indexed_source = source.clone();
        indexed_source.snapshot = Some(
            crate::knowledge_sources::scan(&source, &std::sync::atomic::AtomicBool::new(false))
                .unwrap(),
        );
        assert!(indexed_source.snapshot.as_ref().unwrap().files.len() >= 3);
        let index = root.join("index.sqlite3");
        crate::knowledge_index::sync_all(
            &index,
            &[indexed_source],
            false,
            &std::sync::atomic::AtomicBool::new(false),
            |_, _, _| {},
        )
        .unwrap();
        let found = crate::knowledge_index::search(&index, "不同内容", None, 10).unwrap();
        assert!(
            found
                .hits
                .iter()
                .any(|hit| hit.relative == changed.filename)
        );
        let mut registry = crate::knowledge_sources::State::new(root.join("sources.json"));
        let inbox_id = registry.ensure_inbox().unwrap();
        assert_eq!(registry.ensure_inbox().unwrap(), inbox_id);
        assert!(root.join("knowledge-inbox").is_dir());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_private_addresses_and_sensitive_url_parameters() {
        for url in [
            "http://example.com",
            "https://localhost/",
            "https://127.0.0.1/",
            "https://10.0.0.1/",
            "https://example.com:8443/",
            "https://user:pass@example.com/",
            "https://example.com/?access_token=fixture",
        ] {
            assert!(validate_url(url).is_err(), "{url}");
        }
        assert!(
            validate_url("https://example.com/path#fragment")
                .unwrap()
                .fragment()
                .is_none()
        );
        assert!(!public_ip("192.168.1.1".parse().unwrap()));
        assert!(!public_ip("::1".parse().unwrap()));
    }
}
