use crate::{
    mcp::{self, Action, Config, ConnectedRequest, Report},
    mcp_access::{self, Decision, Rule, Store},
    mcp_http::{self, HttpConfig},
};
use eframe::egui;
use serde_json::Value;
#[cfg(feature = "ui-preview")]
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::{Duration, Instant},
};

fn display_json(value: &Value, limit: usize) -> String {
    let text = serde_json::to_string_pretty(value).unwrap_or_default();
    let mut chars = text.chars();
    let excerpt: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        format!("{excerpt}\n…界面预览已截断")
    } else {
        excerpt
    }
}

fn http_credential(
    endpoint: &str,
    saved: bool,
    temporary: String,
) -> anyhow::Result<crate::credentials::Secret> {
    HttpConfig {
        endpoint: endpoint.into(),
    }
    .validate()?;
    let secret = if saved {
        let target = crate::credentials::Target::mcp_http(endpoint)?;
        crate::credentials::read(&target)?
            .ok_or_else(|| anyhow::anyhow!("此端点没有已保存令牌，请输入后保存或使用临时令牌"))?
    } else {
        crate::credentials::Secret::new(temporary)?
    };
    anyhow::ensure!(
        secret
            .expose()
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~+/=".contains(&b)),
        "Bearer 令牌格式无效"
    );
    Ok(secret)
}

fn declared_read_only(tool: &Value) -> bool {
    tool.pointer("/annotations/readOnlyHint") == Some(&Value::Bool(true))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum McpSection {
    Tools,
    Resources,
    Prompts,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Transport {
    Stdio,
    Http,
}

struct CredentialAck {
    revision: u64,
    deadline: Instant,
    receiver: Receiver<Result<(), String>>,
}

pub struct McpState {
    transport: Transport,
    oauth: crate::mcp_oauth_ui::OAuthPanel,
    executable: String,
    arguments: String,
    http_endpoint: String,
    temporary_bearer: bool,
    use_saved_bearer: bool,
    http_token: String,
    report: Option<Report>,
    section: McpSection,
    selected_tool: String,
    selected_resource: String,
    selected_prompt: String,
    call_arguments: String,
    confirmation_name: String,
    last_called_tool: String,
    prompt_arguments: String,
    call_confirm: bool,
    access: Store,
    server_scope: Option<String>,
    scope_error: Option<String>,
    receiver: Option<Receiver<Result<Report, String>>>,
    connection: Option<Sender<ConnectedRequest>>,
    connection_alive: Arc<AtomicBool>,
    credential_updates: Option<Sender<mcp_http::CredentialUpdate>>,
    credential_ack: Option<CredentialAck>,
    applied_revision: u64,
    cancelled: Arc<AtomicBool>,
    message: String,
}

impl McpState {
    pub fn new(access_path: PathBuf) -> Self {
        Self {
            transport: Transport::Stdio,
            oauth: crate::mcp_oauth_ui::OAuthPanel::default(),
            executable: String::new(),
            arguments: "[]".into(),
            http_endpoint: "http://127.0.0.1:3000/mcp".into(),
            temporary_bearer: false,
            use_saved_bearer: false,
            http_token: String::new(),
            report: None,
            section: McpSection::Tools,
            selected_tool: String::new(),
            selected_resource: String::new(),
            selected_prompt: String::new(),
            call_arguments: "{}".into(),
            confirmation_name: String::new(),
            last_called_tool: String::new(),
            prompt_arguments: "{}".into(),
            call_confirm: false,
            access: Store::load(access_path),
            server_scope: None,
            scope_error: None,
            receiver: None,
            connection: None,
            connection_alive: Arc::new(AtomicBool::new(false)),
            credential_updates: None,
            credential_ack: None,
            applied_revision: 0,
            cancelled: Arc::new(AtomicBool::new(false)),
            message: String::new(),
        }
    }
}

impl Drop for McpState {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl McpState {
    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self, show_prompts: bool) {
        self.executable = r"C:\Tools\mcp-demo.exe".into();
        self.arguments = "[\"--stdio\"]".into();
        self.report = Some(Report {
            server: "示例 MCP 服务".into(),
            protocol: "2025-06-18".into(),
            tools: vec![
                json!({"name":"search_notes","description":"在已授权的笔记中搜索关键字","annotations":{"readOnlyHint":true,"destructiveHint":false,"openWorldHint":false},"inputSchema":{"type":"object","properties":{"query":{"type":"string","description":"搜索词"}},"required":["query"]}}),
                json!({"name":"read_note","description":"读取单篇笔记","inputSchema":{"type":"object","properties":{"id":{"type":"string"}},"required":["id"]}}),
            ],
            resources: vec![json!({"name":"说明文档","uri":"demo://guide"})],
            prompts: vec![json!({"name":"summarize","description":"概括选定笔记"})],
            call_result: None,
            resource_result: Some((
                "demo://guide".into(),
                json!({"contents":[{"uri":"demo://guide","mimeType":"text/plain","text":"合成测试说明，不读取真实文件"}]}),
            )),
            prompt_result: Some((
                "summarize".into(),
                json!({"messages":[{"role":"user","content":{"type":"text","text":"概括选定笔记"}}]}),
            )),
        });
        self.selected_tool = "search_notes".into();
        self.selected_resource = "demo://guide".into();
        self.selected_prompt = "summarize".into();
        self.section = if show_prompts {
            McpSection::Prompts
        } else {
            McpSection::Resources
        };
        self.call_arguments = "{\n  \"query\": \"Rust 错误处理\"\n}".into();
        self.message = "合成界面预览 · 未启动外部程序".into();
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_tool_review(&mut self, unknown_behavior: bool) {
        self.preview_fixture(false);
        self.section = McpSection::Tools;
        self.selected_tool = if unknown_behavior {
            "read_note".into()
        } else {
            "search_notes".into()
        };
        self.call_arguments = if unknown_behavior {
            "{\n  \"id\": \"note-1\"\n}".into()
        } else {
            "{\n  \"query\": \"Rust 错误处理\"\n}".into()
        };
        self.call_confirm = true;
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_permissions(&mut self) {
        self.preview_tool_review(false);
        self.server_scope = Some("a".repeat(64));
        self.call_confirm = false;
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_connected(&mut self) {
        self.preview_fixture(false);
        let (sender, _receiver) = mpsc::channel();
        self.connection = Some(sender);
        self.connection_alive.store(true, Ordering::Relaxed);
        self.message = "合成界面预览 · 持续连接状态，无外部进程".into();
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_http(&mut self) {
        self.preview_fixture(false);
        self.transport = Transport::Http;
        self.http_endpoint = "https://mcp.example.test/mcp".into();
        let (sender, _receiver) = mpsc::channel();
        self.connection = Some(sender);
        self.connection_alive.store(true, Ordering::Relaxed);
        self.message = "合成 HTTP 界面预览 · 无网络请求".into();
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_http_authentication(&mut self) {
        self.preview_http();
        self.connection = None;
        self.connection_alive.store(false, Ordering::Relaxed);
        self.report = None;
        self.temporary_bearer = true;
        self.http_token.clear();
        self.message = "合成认证界面预览 · 未输入令牌 · 无网络请求".into();
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_oauth_metadata(&mut self) {
        self.preview_http_authentication();
        self.temporary_bearer = false;
        self.oauth.preview(&self.http_endpoint);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_oauth_auto(&mut self) {
        self.preview_oauth_refresh();
        self.oauth.preview_auto();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_oauth_refresh(&mut self) {
        self.preview_oauth_metadata();
        self.oauth.preview_refresh(&self.http_endpoint);
        self.message = "合成刷新界面预览 · 无网络请求".into();
    }

    fn config(&self) -> Result<Config, String> {
        let args: Vec<String> = serde_json::from_str(&self.arguments)
            .map_err(|_| "参数必须是 JSON 字符串数组，例如 [\"server.js\"]".to_owned())?;
        let config = Config {
            executable: PathBuf::from(self.executable.trim()),
            args,
        };
        config.validate().map_err(|error| error.to_string())?;
        Ok(config)
    }

    fn start(&mut self, action: Action, manual_confirmed: bool) {
        if self.oauth.pending() || self.credential_ack.is_some() {
            self.message = "授权更新中，请等待连接确认后再操作".into();
            return;
        }
        let config = if self.transport == Transport::Stdio {
            let config = match self.config() {
                Ok(config) => config,
                Err(error) => {
                    self.message = error;
                    return;
                }
            };
            match mcp_access::server_scope(&config) {
                Ok(scope) => {
                    self.server_scope = Some(scope);
                    self.scope_error = None;
                }
                Err(error) => {
                    self.server_scope = None;
                    self.scope_error = Some(format!("无法核对 MCP 服务身份：{error:#}"));
                    if matches!(&action, Action::Call { .. }) {
                        self.message = self.scope_error.clone().unwrap_or_default();
                        return;
                    }
                }
            }
            Some(config)
        } else {
            if self.connection.is_none() {
                self.message = "请先连接 MCP HTTP 服务".into();
                return;
            }
            if matches!(&action, Action::Call { .. }) && !manual_confirmed {
                self.message = "MCP HTTP 工具调用须逐次确认".into();
                return;
            }
            None
        };
        let (sender, receiver) = mpsc::channel();
        self.last_called_tool = match &action {
            Action::Call { tool, .. } => tool.clone(),
            _ => String::new(),
        };
        if matches!(&action, Action::Inspect) {
            self.report = None;
        } else if let Some(report) = &mut self.report {
            report.call_result = None;
            report.resource_result = None;
            report.prompt_result = None;
        }
        if let Some(connection) = &self.connection {
            if connection
                .send(ConnectedRequest {
                    action,
                    manual_confirmed,
                    response: sender,
                })
                .is_err()
            {
                self.connection = None;
                self.message = "MCP 连接已退出；请重新连接".into();
                return;
            }
        } else {
            let Some(config) = config else {
                self.message = "MCP HTTP 连接已退出，请重新连接".into();
                return;
            };
            self.cancelled = Arc::new(AtomicBool::new(false));
            let cancelled = Arc::clone(&self.cancelled);
            let access_path = self.access.path().to_path_buf();
            std::thread::spawn(move || {
                let result = if matches!(&action, Action::Call { .. }) {
                    mcp::run_with_access(config, action, cancelled, access_path, manual_confirmed)
                } else {
                    mcp::run(config, action, cancelled)
                }
                .map_err(|error| error.to_string());
                let _ = sender.send(result);
            });
        }
        self.receiver = Some(receiver);
        self.message = "正在与 MCP 服务通信…".into();
        self.call_confirm = false;
        self.confirmation_name.clear();
    }

    fn connect(&mut self) {
        if self.oauth.pending() || self.credential_ack.is_some() {
            self.message = "请等待授权任务完成后连接".into();
            return;
        }
        let stdio = if self.transport == Transport::Stdio {
            match self.config() {
                Ok(config) => Some(config),
                Err(error) => {
                    self.message = error;
                    return;
                }
            }
        } else {
            None
        };
        let http = if self.transport == Transport::Http {
            let config = HttpConfig {
                endpoint: self.http_endpoint.trim().into(),
            };
            if let Err(error) = config.validate() {
                self.message = error.to_string();
                return;
            }
            Some(config)
        } else {
            None
        };
        let credential = if self.transport == Transport::Http && self.oauth.enabled() {
            match self.oauth.credential(&self.http_endpoint) {
                Ok(secret) => Some(secret),
                Err(error) => {
                    self.message = error.to_string();
                    return;
                }
            }
        } else if self.transport == Transport::Http && self.temporary_bearer {
            let result = http_credential(
                &self.http_endpoint,
                self.use_saved_bearer,
                std::mem::take(&mut self.http_token),
            );
            match result {
                Ok(secret) => Some(secret),
                Err(error) => {
                    self.message = error.to_string();
                    return;
                }
            }
        } else {
            None
        };
        self.cancelled = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::clone(&self.cancelled);
        self.connection_alive = Arc::new(AtomicBool::new(true));
        let alive = Arc::clone(&self.connection_alive);
        let access_path = self.access.path().to_path_buf();
        let (sender, requests) = mpsc::channel();
        let (updates, update_rx) = mpsc::channel();
        self.credential_updates = http.as_ref().map(|_| updates);
        self.credential_ack = None;
        self.applied_revision = self.oauth.revision();
        std::thread::spawn(move || {
            if let Some(config) = stdio {
                let _ = mcp::serve_connected(config, cancelled, access_path, requests);
            } else if let Some(config) = http {
                let _ = mcp_http::serve_http_with_updates(
                    config, credential, cancelled, requests, update_rx,
                );
            }
            alive.store(false, Ordering::Relaxed);
        });
        self.connection = Some(sender);
        self.start(Action::Inspect, false);
    }

    fn end_authorized_connection(&mut self, message: String) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.connection = None;
        // A cancelled operation must not restore a report after authorization is discarded.
        self.receiver = None;
        self.credential_updates = None;
        self.credential_ack = None;
        self.report = None;
        self.oauth.clear();
        self.message = message;
    }

    pub fn tick(&mut self, ctx: &egui::Context) {
        self.oauth.tick(ctx);
        if self.connection.is_none() {
            self.credential_updates = None;
            self.credential_ack = None;
        }
        if self.connection.is_some() && self.oauth.enabled() {
            if let Some(ack) = &self.credential_ack {
                match ack.receiver.try_recv() {
                    Ok(Ok(())) => {
                        self.applied_revision = ack.revision;
                        self.credential_ack = None;
                        self.message = "授权已续期，连接已确认，可以继续操作".into();
                    }
                    Ok(Err(error)) => {
                        self.end_authorized_connection(format!("连接拒绝授权更新：{error}"))
                    }
                    Err(mpsc::TryRecvError::Disconnected) => self
                        .end_authorized_connection("授权更新确认失败，已断开，请重新登录".into()),
                    Err(mpsc::TryRecvError::Empty) if Instant::now() >= ack.deadline => self
                        .end_authorized_connection("授权更新确认超时，已断开，请重新登录".into()),
                    Err(mpsc::TryRecvError::Empty) => {
                        ctx.request_repaint_after(Duration::from_millis(50));
                    }
                }
            }
            if self.connection.is_some() && !self.oauth.pending() && self.credential_ack.is_none() {
                match self.oauth.credential(&self.http_endpoint) {
                    Err(_) => self.end_authorized_connection(
                        "当前授权已失效或刷新失败，已断开，请重新登录".into(),
                    ),
                    Ok(secret) if self.oauth.revision() != self.applied_revision => {
                        let (response, ack) = mpsc::channel();
                        let update = mcp_http::CredentialUpdate {
                            endpoint: self.http_endpoint.clone(),
                            credential: secret,
                            response,
                        };
                        if self
                            .credential_updates
                            .as_ref()
                            .is_some_and(|sender| sender.send(update).is_ok())
                        {
                            self.credential_ack = Some(CredentialAck {
                                revision: self.oauth.revision(),
                                deadline: Instant::now() + Duration::from_secs(3),
                                receiver: ack,
                            });
                            self.message = "续期成功，正在等待连接确认…".into();
                        } else {
                            self.end_authorized_connection(
                                "无法更新连接授权，已断开，请重新登录".into(),
                            );
                        }
                    }
                    Ok(_) if self.receiver.is_none() && self.oauth.refresh_due() => {
                        if let Err(error) = self.oauth.start_refresh() {
                            self.end_authorized_connection(format!("无法开始自动续期：{error}"));
                        } else {
                            self.message = "正在自动续期，新操作暂时暂停…".into();
                        }
                    }
                    _ => {}
                }
            }
        }

        if self.connection.is_some() && self.oauth.enabled() && self.oauth.expired() {
            self.end_authorized_connection("OAuth 访问令牌已过期，已断开，请重新登录".into());
        }
        if self.connection.is_some() && !self.connection_alive.load(Ordering::Relaxed) {
            self.connection = None;
            self.oauth.clear();
            if self.receiver.is_none() {
                self.message = "MCP 连接已断开".into();
            }
        } else if self.connection.is_some() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        let Some(receiver) = &self.receiver else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(report)) => {
                let suffix = if self.connection.is_some() {
                    "；连接保持中"
                } else {
                    "；本次会话已退出"
                };
                self.message = if report.call_result.is_some() {
                    "工具响应已收到"
                } else if report.resource_result.is_some() {
                    "资源内容已收到"
                } else if report.prompt_result.is_some() {
                    "提示词内容已收到"
                } else {
                    "能力清单已读取"
                }
                .to_owned()
                    + suffix;
                if !report.tools.iter().any(|tool| {
                    tool.get("name").and_then(Value::as_str) == Some(&self.selected_tool)
                }) {
                    self.selected_tool = report
                        .tools
                        .first()
                        .and_then(|tool| tool.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .into();
                }
                if !report.resources.iter().any(|resource| {
                    resource.get("uri").and_then(Value::as_str) == Some(&self.selected_resource)
                }) {
                    self.selected_resource = report
                        .resources
                        .first()
                        .and_then(|resource| resource.get("uri"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .into();
                }
                if !report.prompts.iter().any(|prompt| {
                    prompt.get("name").and_then(Value::as_str) == Some(&self.selected_prompt)
                }) {
                    self.selected_prompt = report
                        .prompts
                        .first()
                        .and_then(|prompt| prompt.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .into();
                }
                self.report = Some(report);
                self.receiver = None;
            }
            Ok(Err(error)) => {
                self.message = error;
                self.receiver = None;
                if self.transport != Transport::Http
                    || !self.connection_alive.load(Ordering::Relaxed)
                {
                    self.connection = None;
                }
                self.report = None;
                self.call_confirm = false;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.message = "MCP 后台任务意外结束".into();
                self.receiver = None;
            }
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(80));
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.tick(ui.ctx());
        let busy = self.receiver.is_some() || self.oauth.pending() || self.credential_ack.is_some();
        ui.heading("MCP 协议调试台");
        ui.label("检查 MCP 工具、资源和提示词，并在逐次确认后调用工具。可选本机 stdio 或 Streamable HTTP；连接信息与结果不会保存。");
        if self.connection.is_some() {
            ui.colored_label(
                egui::Color32::from_rgb(86, 163, 118),
                "● 持续连接中 · 闲置 2 分钟自动断开",
            );
        }
        ui.add_space(8.0);
        ui.group(|ui| {
            ui.strong("连接目标");
            ui.horizontal(|ui| {
                let editable = !busy && self.connection.is_none();
                let previous = self.transport;
                ui.add_enabled_ui(editable, |ui| {
                    ui.selectable_value(&mut self.transport, Transport::Stdio, "本机 stdio");
                    ui.selectable_value(&mut self.transport, Transport::Http, "Streamable HTTP（2025）");
                });
                if self.transport != previous {
                    self.oauth.clear();
                    self.report = None;
                    self.server_scope = None;
                    self.scope_error = None;
                    self.call_confirm = false;
                    self.http_token.clear();
                    self.temporary_bearer = false;
                    self.use_saved_bearer = false;
                }
            });
            if self.transport == Transport::Stdio {
                ui.weak("只接受存在的绝对 EXE 路径；不经 shell。服务程序由你选择执行，本功能不是沙箱。");
                ui.horizontal(|ui| {
                ui.label("可执行程序");
                if ui
                    .add_enabled(
                        !busy && self.connection.is_none(),
                        egui::TextEdit::singleline(&mut self.executable)
                            .desired_width((ui.available_width() - 100.0).max(280.0)),
                    )
                    .changed()
                {
                    self.report = None;
                    self.server_scope = None;
                    self.scope_error = None;
                    self.call_confirm = false;
                    self.confirmation_name.clear();
                }
                #[cfg(windows)]
                if ui
                    .add_enabled(
                        !busy && self.connection.is_none(),
                        egui::Button::new("选择 EXE"),
                    )
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Executable", &["exe"])
                        .pick_file()
                {
                    self.executable = path.to_string_lossy().into_owned();
                    self.report = None;
                    self.server_scope = None;
                    self.scope_error = None;
                    self.call_confirm = false;
                    self.confirmation_name.clear();
                }
                });
                ui.label("参数 · JSON 字符串数组");
                if ui
                .add_enabled(
                    !busy && self.connection.is_none(),
                    egui::TextEdit::singleline(&mut self.arguments).desired_width(f32::INFINITY),
                )
                .changed()
                {
                    self.report = None;
                    self.server_scope = None;
                    self.scope_error = None;
                    self.call_confirm = false;
                    self.confirmation_name.clear();
                }
            } else {
                ui.weak("HTTPS 或精确 127.0.0.1/::1 的 HTTP 端点；不跟随重定向、不使用环境代理。HTTP 工具调用每次人工确认。");
                ui.label("MCP HTTP 端点");
                if ui.add_enabled(!busy && self.connection.is_none(), egui::TextEdit::singleline(&mut self.http_endpoint).desired_width(f32::INFINITY)).changed() {
                    self.oauth.clear();
                    self.report = None;
                    self.call_confirm = false;
                    self.confirmation_name.clear();
                    self.http_token.clear();
                    self.use_saved_bearer = false;
                }
                ui.add_enabled_ui(!busy && self.connection.is_none(), |ui| {
                    if ui.checkbox(&mut self.temporary_bearer, "本次连接使用 Bearer 令牌").changed() {
                        self.oauth.clear();
                        self.http_token.clear();
                        self.use_saved_bearer = false;
                    }
                    if self.temporary_bearer && self.connection.is_none() {
                        #[cfg(windows)]
                        if ui.checkbox(&mut self.use_saved_bearer, "连接时使用此端点的已保存令牌").changed() { self.http_token.clear(); }
                        if !self.use_saved_bearer {
                        ui.add(egui::TextEdit::singleline(&mut self.http_token).password(true).hint_text("仅输入为上方端点签发的令牌").desired_width(f32::INFINITY));
                        }
                        #[cfg(windows)]
                        ui.horizontal_wrapped(|ui| {
                            if ui.add_enabled(!self.use_saved_bearer && !self.http_token.trim().is_empty(), egui::Button::new("保存输入令牌")).clicked() {
                                let result = crate::credentials::Target::mcp_http(&self.http_endpoint).and_then(|target| {
                                    let secret = crate::credentials::Secret::new(std::mem::take(&mut self.http_token))?;
                                    anyhow::ensure!(secret.expose().bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~+/=".contains(&b)), "Bearer 令牌格式无效");
                                    crate::credentials::save(&target, secret.expose())
                                });
                                self.message = match result {
                                    Ok(()) => { self.use_saved_bearer = true; "已保存到 Windows 凭据管理器，输入已清空；仅用于上方完整端点".into() },
                                    Err(error) => error.to_string(),
                                };
                            }
                            if ui.button("删除此端点已保存令牌").clicked() {
                                self.message = match crate::credentials::Target::mcp_http(&self.http_endpoint).and_then(|target| crate::credentials::delete(&target)) {
                                    Ok(()) => { self.use_saved_bearer = false; "此端点已保存令牌已删除".into() },
                                    Err(error) => error.to_string(),
                                };
                            }
                        });
                    }
                });
                if self.temporary_bearer {
                    if self.connection.is_some() {
                        ui.small("本次连接已启用认证，令牌输入已清空。OAuth 浏览器登录见下方；可在下方主动开启自动续期。");
                    } else {
                        ui.small("仅用于上方完整端点；临时输入不保存，主动保存使用 Windows 凭据管理器。OAuth 浏览器登录见下方；可在下方主动开启自动续期。");
                    }
                }
            }
            ui.horizontal(|ui| {
                if (self.transport == Transport::Stdio || self.connection.is_some()) && ui
                    .add_enabled(!busy, egui::Button::new("检查服务能力"))
                    .clicked()
                {
                    self.start(Action::Inspect, false);
                }
                if self.connection.is_none()
                    && ui
                        .add_enabled(!busy, egui::Button::new("连接并保持"))
                        .clicked()
                {
                    self.connect();
                }
                if self.transport == Transport::Http && self.connection.is_some()
                    && ui.add_enabled(!busy && self.oauth.has_refresh(), egui::Button::new("断开并刷新授权")).clicked() {
                    self.cancelled.store(true, Ordering::Relaxed);
                    self.connection = None;
                    self.report = None;
                    self.message = match self.oauth.start_refresh() {
                        Ok(()) => "正在断开并刷新 OAuth 授权，成功后请主动重新连接".into(),
                        Err(error) => error.to_string(),
                    };
                }
                if ui
                    .add_enabled(
                        busy || self.connection.is_some(),
                        egui::Button::new(if self.transport == Transport::Http { "断开 HTTP 连接" } else { "断开并退出进程" }),
                    )
                    .clicked()
                {
                    self.cancelled.store(true, Ordering::Relaxed);
                    self.oauth.clear();
                    self.http_token.clear();
                    self.connection = None;
                    self.message = "正在断开 MCP 连接…".into();
                }
                if busy {
                    ui.spinner();
                }
            });
        });
        if self.transport == Transport::Http {
            self.oauth.ui(
                ui,
                &self.http_endpoint,
                self.receiver.is_none() && self.connection.is_none(),
            );
            if self.oauth.enabled() {
                self.temporary_bearer = false;
                self.use_saved_bearer = false;
                self.http_token.clear();
            }
        }
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        if let Some(error) = &self.scope_error {
            ui.colored_label(egui::Color32::from_rgb(210, 145, 60), error);
        }
        let Some(report) = &self.report else { return };
        let mut requested_action = None;
        let mut manual_confirmed = false;
        ui.add_space(8.0);
        ui.heading(format!("{} · MCP {}", report.server, report.protocol));
        ui.label(format!(
            "{} 个工具 · {} 个资源 · {} 个提示词",
            report.tools.len(),
            report.resources.len(),
            report.prompts.len()
        ));
        ui.weak("能力与结果仅供本次查看；列表不会自动读取资源或执行工具。");
        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut self.section,
                McpSection::Tools,
                format!("工具 ({})", report.tools.len()),
            );
            ui.selectable_value(
                &mut self.section,
                McpSection::Resources,
                format!("资源 ({})", report.resources.len()),
            );
            ui.selectable_value(
                &mut self.section,
                McpSection::Prompts,
                format!("提示词 ({})", report.prompts.len()),
            );
        });
        if self.section == McpSection::Tools && !report.tools.is_empty() {
            ui.separator();
            ui.strong("工具清单");
            egui::ComboBox::from_id_salt("mcp-tool")
                .selected_text(if self.selected_tool.is_empty() {
                    "选择工具"
                } else {
                    &self.selected_tool
                })
                .show_ui(ui, |ui| {
                    for tool in &report.tools {
                        if let Some(name) = tool.get("name").and_then(Value::as_str)
                            && ui
                                .selectable_value(&mut self.selected_tool, name.to_owned(), name)
                                .changed()
                        {
                            self.call_confirm = false;
                            self.confirmation_name.clear();
                            self.call_arguments = "{}".into();
                        }
                    }
                });
            let selected_descriptor = report
                .tools
                .iter()
                .find(|tool| tool.get("name").and_then(Value::as_str) == Some(&self.selected_tool))
                .cloned();
            let mut access_decision = Decision::Confirm;
            if let Some(tool) = &selected_descriptor {
                if let Some(description) = tool.get("description").and_then(Value::as_str) {
                    ui.label(description.chars().take(500).collect::<String>());
                }
                if mcp_access::declared_low_impact(tool) {
                    ui.colored_label(
                        egui::Color32::from_rgb(90, 160, 110),
                        "服务声明：低影响只读工具",
                    );
                } else if declared_read_only(tool) {
                    ui.colored_label(
                        egui::Color32::from_rgb(210, 145, 60),
                        "服务声明只读，但未完整声明不破坏且不访问外部系统",
                    );
                } else {
                    ui.colored_label(
                        egui::Color32::from_rgb(210, 145, 60),
                        "服务未声明只读；调用可能修改数据或访问外部系统",
                    );
                }
                ui.small("工具行为由服务自行声明，不能证明程序安全；仅连接可信服务。");
                if let Some(schema) = tool.get("inputSchema") {
                    egui::CollapsingHeader::new("输入参数 schema")
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.monospace(display_json(schema, 12_000));
                        });
                }
                ui.add_space(4.0);
                if self.transport == Transport::Http {
                    access_decision = Decision::Confirm;
                    ui.small("HTTP 端点不会继承本机 EXE 授权规则；每次工具调用都要人工确认。服务可能位于外部网络，请确认端点与参数。");
                } else if let Some(error) = &self.access.error {
                    ui.colored_label(egui::Color32::from_rgb(210, 100, 80), error);
                    access_decision = Decision::Deny;
                } else if let Some(scope) = self.server_scope.as_deref() {
                    match self.access.decision(scope, tool) {
                        Ok(decision) => access_decision = decision,
                        Err(error) => {
                            self.message = error.to_string();
                            access_decision = Decision::Deny;
                        }
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.strong("本机调用规则");
                        ui.label(match access_decision {
                            Decision::Deny => "已禁止",
                            Decision::Confirm => "每次确认（默认）",
                            Decision::Direct => "允许直接调用声明只读工具",
                        });
                        if ui.button("每次确认").clicked() {
                            match self.access.set(scope, tool, None) {
                                Ok(()) => {
                                    access_decision = Decision::Confirm;
                                    self.call_confirm = false;
                                }
                                Err(error) => self.message = error.to_string(),
                            }
                            if busy && self.last_called_tool == self.selected_tool {
                                self.cancelled.store(true, Ordering::Relaxed);
                            }
                        }
                        if ui.button("禁止调用").clicked() {
                            match self.access.set(scope, tool, Some(Rule::Deny)) {
                                Ok(()) => {
                                    access_decision = Decision::Deny;
                                    self.call_confirm = false;
                                }
                                Err(error) => self.message = error.to_string(),
                            }
                            if busy && self.last_called_tool == self.selected_tool {
                                self.cancelled.store(true, Ordering::Relaxed);
                            }
                        }
                        if mcp_access::declared_low_impact(tool)
                            && ui.button("允许只读直接调用").clicked()
                        {
                            match self
                                .access
                                .set(scope, tool, Some(Rule::AllowDeclaredReadOnly))
                            {
                                Ok(()) => {
                                    access_decision = Decision::Direct;
                                    self.call_confirm = false;
                                }
                                Err(error) => self.message = error.to_string(),
                            }
                        }
                    });
                    ui.small("规则绑定当前程序、参数和工具定义；服务声明只读不等于安全保证。撤销会停止当前会话，已发出的动作无法回滚。");
                } else {
                    access_decision = Decision::Deny;
                    ui.small("无法核对服务身份，工具调用不可用；资源和提示词仍可手动查看。");
                }
            }
            ui.label("调用参数 · JSON 对象");
            if ui
                .add_enabled(
                    !busy,
                    egui::TextEdit::multiline(&mut self.call_arguments)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace),
                )
                .changed()
            {
                self.call_confirm = false;
                self.confirmation_name.clear();
            }
            if access_decision == Decision::Deny {
                ui.add_enabled(false, egui::Button::new("此工具已禁止调用"));
            } else if access_decision == Decision::Direct {
                if ui
                    .add_enabled(!busy, egui::Button::new("调用已允许的只读工具"))
                    .clicked()
                {
                    match serde_json::from_str::<Value>(&self.call_arguments) {
                        Ok(arguments) if arguments.is_object() => {
                            if let Some(expected_tool) = &selected_descriptor {
                                requested_action = Some(Action::Call {
                                    tool: self.selected_tool.clone(),
                                    arguments,
                                    expected_tool: expected_tool.clone(),
                                });
                            }
                        }
                        _ => self.message = "调用参数必须是 JSON 对象".into(),
                    }
                }
            } else if ui
                .add_enabled(!busy, egui::Button::new("准备调用选中工具"))
                .clicked()
            {
                self.call_confirm = true;
            }
            if self.call_confirm && access_decision == Decision::Confirm {
                ui.horizontal_wrapped(|ui| {
                    ui.weak(format!(
                        "将连接 {} 并调用 {}；参数会交给该服务。",
                        if self.transport == Transport::Http {
                            &self.http_endpoint
                        } else {
                            &self.executable
                        },
                        self.selected_tool
                    ));
                    let low_impact = selected_descriptor
                        .as_ref()
                        .is_some_and(mcp_access::declared_low_impact);
                    if !low_impact {
                        ui.label("输入工具名再次确认：");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.confirmation_name)
                                .desired_width(170.0),
                        );
                    }
                    if ui
                        .add_enabled(
                            low_impact || self.confirmation_name == self.selected_tool,
                            egui::Button::new("确认调用"),
                        )
                        .clicked()
                    {
                        match serde_json::from_str::<Value>(&self.call_arguments) {
                            Ok(arguments)
                                if arguments.is_object() && selected_descriptor.is_some() =>
                            {
                                requested_action = Some(Action::Call {
                                    tool: self.selected_tool.clone(),
                                    arguments,
                                    expected_tool: selected_descriptor.clone().unwrap(),
                                });
                                manual_confirmed = true;
                            }
                            _ => self.message = "调用参数必须是 JSON 对象".into(),
                        }
                    }
                    if ui.button("取消").clicked() {
                        self.call_confirm = false;
                        self.confirmation_name.clear();
                    }
                });
            }
        }
        if self.section == McpSection::Resources && !report.resources.is_empty() {
            ui.separator();
            ui.strong(format!("资源 · {} 项", report.resources.len()));
            egui::ComboBox::from_id_salt("mcp-resource")
                .selected_text(
                    report
                        .resources
                        .iter()
                        .find(|item| {
                            item.get("uri").and_then(Value::as_str) == Some(&self.selected_resource)
                        })
                        .and_then(|item| item.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or("选择资源"),
                )
                .show_ui(ui, |ui| {
                    for item in &report.resources {
                        if let (Some(name), Some(uri)) = (
                            item.get("name").and_then(Value::as_str),
                            item.get("uri").and_then(Value::as_str),
                        ) {
                            ui.selectable_value(
                                &mut self.selected_resource,
                                uri.to_owned(),
                                format!("{name} · {}", uri.chars().take(80).collect::<String>()),
                            );
                        }
                    }
                });
            ui.weak(format!(
                "URI：{}",
                self.selected_resource.chars().take(300).collect::<String>()
            ));
            if ui
                .add_enabled(
                    !busy && !self.selected_resource.is_empty(),
                    egui::Button::new("读取选中资源"),
                )
                .clicked()
            {
                requested_action = Some(Action::ReadResource {
                    uri: self.selected_resource.clone(),
                });
            }
        }
        if self.section == McpSection::Prompts && !report.prompts.is_empty() {
            ui.separator();
            ui.strong(format!("提示词 · {} 项", report.prompts.len()));
            egui::ComboBox::from_id_salt("mcp-prompt")
                .selected_text(if self.selected_prompt.is_empty() {
                    "选择提示词"
                } else {
                    &self.selected_prompt
                })
                .show_ui(ui, |ui| {
                    for item in &report.prompts {
                        if let Some(name) = item.get("name").and_then(Value::as_str) {
                            ui.selectable_value(&mut self.selected_prompt, name.to_owned(), name);
                        }
                    }
                });
            if let Some(prompt) = report.prompts.iter().find(|item| {
                item.get("name").and_then(Value::as_str) == Some(&self.selected_prompt)
            }) {
                if let Some(description) = prompt.get("description").and_then(Value::as_str) {
                    ui.label(description.chars().take(500).collect::<String>());
                }
                if let Some(arguments) = prompt.get("arguments") {
                    egui::CollapsingHeader::new("参数声明").show(ui, |ui| {
                        ui.monospace(display_json(arguments, 12_000));
                    });
                }
            }
            ui.label("提示词参数 · 字符串值的 JSON 对象");
            ui.add_enabled(
                !busy,
                egui::TextEdit::multiline(&mut self.prompt_arguments)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace),
            );
            if ui
                .add_enabled(
                    !busy && !self.selected_prompt.is_empty(),
                    egui::Button::new("获取选中提示词"),
                )
                .clicked()
            {
                match serde_json::from_str::<Value>(&self.prompt_arguments) {
                    Ok(arguments)
                        if arguments.as_object().is_some_and(|items| {
                            items.len() <= 32 && items.values().all(Value::is_string)
                        }) =>
                    {
                        requested_action = Some(Action::GetPrompt {
                            name: self.selected_prompt.clone(),
                            arguments,
                        });
                    }
                    _ => self.message = "提示词参数须为字符串值的 JSON 对象".into(),
                }
            }
        }
        if self.section == McpSection::Tools
            && self.last_called_tool == self.selected_tool
            && let Some(result) = &report.call_result
        {
            ui.separator();
            ui.strong("工具响应");
            let display = display_json(result, 30_000);
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .show(ui, |ui| {
                    ui.monospace(display);
                });
        }
        let results = match self.section {
            McpSection::Tools => None,
            McpSection::Resources => Some(("资源内容", &report.resource_result)),
            McpSection::Prompts => Some(("提示词消息", &report.prompt_result)),
        };
        if let Some((title, Some((name, value)))) = results {
            ui.separator();
            ui.strong(format!("{title} · {name}"));
            let display = display_json(value, 30_000);
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .show(ui, |ui| {
                    ui.monospace(display);
                });
        }
        if let Some(action) = requested_action {
            self.start(action, manual_confirmed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::declared_read_only;
    use serde_json::json;

    #[test]
    #[cfg(windows)]
    #[ignore = "explicit disposable MCP UI credential selection test"]
    fn saved_selection_is_bound_to_endpoint_and_never_falls_back_to_temporary() {
        use crate::credentials::{Target, delete, save};
        let endpoint = format!("https://example.test/mcp/{}", uuid::Uuid::new_v4());
        let target = Target::mcp_http(&endpoint).unwrap();
        struct Cleanup(Target);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = delete(&self.0);
            }
        }
        let cleanup = Cleanup(target);
        save(&cleanup.0, "synthetic-ui-saved").unwrap();
        assert!(
            super::http_credential(&endpoint, true, String::new())
                .unwrap()
                .expose()
                == "synthetic-ui-saved"
        );
        assert!(
            super::http_credential(&endpoint, false, "synthetic-ui-temporary".into())
                .unwrap()
                .expose()
                == "synthetic-ui-temporary"
        );
        let other = format!("{endpoint}/other");
        assert!(super::http_credential(&other, true, "synthetic-ui-temporary".into()).is_err());
        delete(&cleanup.0).unwrap();
        assert!(super::http_credential(&endpoint, true, String::new()).is_err());
    }

    #[test]
    fn only_explicit_true_counts_as_read_only() {
        assert!(declared_read_only(
            &json!({"annotations":{"readOnlyHint":true}})
        ));
        for tool in [
            json!({"name":"unknown"}),
            json!({"annotations":{"readOnlyHint":false}}),
            json!({"annotations":{"readOnlyHint":"true"}}),
        ] {
            assert!(!declared_read_only(&tool));
        }
    }
}

#[cfg(test)]
mod renewal_tests {
    use super::*;
    fn state() -> (
        McpState,
        Receiver<ConnectedRequest>,
        Receiver<mcp_http::CredentialUpdate>,
    ) {
        let mut state = McpState::new(
            std::env::temp_dir().join(format!("zi-renewal-{}.json", uuid::Uuid::new_v4())),
        );
        state.transport = Transport::Http;
        state.http_endpoint = "https://mcp.example.test/mcp".into();
        state.oauth = crate::mcp_oauth_ui::OAuthPanel::test_ready(false);
        state.applied_revision = state.oauth.revision();
        let (sender, requests) = mpsc::channel();
        state.connection = Some(sender);
        state.connection_alive.store(true, Ordering::Relaxed);
        let (updates, receiver) = mpsc::channel();
        state.credential_updates = Some(updates);
        (state, requests, receiver)
    }
    #[test]
    fn new_token_blocks_operations_until_worker_ack_and_preserves_connection() {
        let (mut state, requests, updates) = state();
        let _job = state.oauth.test_pending(Some(true));
        let ctx = egui::Context::default();
        state.tick(&ctx); // Entirely outside the MCP page.
        let update = updates.try_recv().unwrap();
        assert_eq!(update.endpoint, state.http_endpoint);
        assert!(state.credential_ack.is_some());
        state.start(Action::Inspect, false);
        assert!(requests.try_recv().is_err());
        update.response.send(Ok(())).unwrap();
        state.tick(&ctx);
        assert!(state.credential_ack.is_none());
        assert!(state.connection.is_some());
        assert_eq!(state.applied_revision, state.oauth.revision());
        state.start(Action::Inspect, false);
        assert!(requests.try_recv().is_ok());
    }
    #[test]
    fn failed_refresh_or_lost_update_ack_closes_without_replay() {
        for failure in 0..3 {
            let (mut state, requests, updates) = state();
            let _job = state.oauth.test_pending(Some(failure != 0));
            let ctx = egui::Context::default();
            state.tick(&ctx);
            if failure != 0 {
                let update = updates.try_recv().unwrap();
                if failure == 1 {
                    update
                        .response
                        .send(Err("synthetic rejection".into()))
                        .unwrap();
                }
                drop(update);
                state.tick(&ctx);
            }
            assert!(state.connection.is_none());
            assert!(state.cancelled.load(Ordering::Relaxed));
            assert!(!state.oauth.enabled());
            assert!(state.report.is_none());
            assert!(requests.try_recv().is_err());
        }
    }
    #[test]
    fn expiry_during_pending_refresh_disconnects_and_discards_late_result() {
        let (mut state, requests, _) = state();
        let (late_response, receiver) = mpsc::channel();
        state.receiver = Some(receiver);
        let _job = state.oauth.test_pending(None);
        state.oauth.test_expire_pending();
        state.tick(&egui::Context::default());
        assert!(state.connection.is_none());
        assert!(!state.oauth.pending());
        assert!(state.cancelled.load(Ordering::Relaxed));
        assert!(requests.try_recv().is_err());
        assert!(late_response.send(Ok(Report::default())).is_err());
    }
    #[test]
    fn stalled_ack_is_bounded_without_sending_another_operation() {
        let (mut state, requests, updates) = state();
        let _job = state.oauth.test_pending(Some(true));
        let ctx = egui::Context::default();
        state.tick(&ctx);
        let _update = updates.try_recv().unwrap();
        state.credential_ack.as_mut().unwrap().deadline = Instant::now() - Duration::from_secs(1);
        state.tick(&ctx);
        assert!(state.connection.is_none());
        assert!(state.message.contains("超时"));
        assert!(requests.try_recv().is_err());
    }
    #[test]
    fn busy_operation_defers_automatic_refresh_and_pending_refresh_blocks_new_action() {
        let (mut state, requests, _) = state();
        state.oauth = crate::mcp_oauth_ui::OAuthPanel::test_ready(true);
        let (_sender, receiver) = mpsc::channel();
        state.receiver = Some(receiver);
        state.tick(&egui::Context::default());
        assert!(!state.oauth.pending());
        assert!(state.oauth.refresh_due());
        let _job = state.oauth.test_pending(None);
        state.receiver = None;
        state.start(Action::Inspect, false);
        assert!(requests.try_recv().is_err());
        assert!(state.oauth.pending());
        state.end_authorized_connection("cancelled".into());
        assert!(!state.oauth.pending());
        assert!(!state.oauth.refresh_due());
    }
}
