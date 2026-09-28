use crate::mcp::{self, Action, Config, Report};
use eframe::egui;
use serde_json::Value;
#[cfg(feature = "ui-preview")]
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::Duration,
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

pub struct McpState {
    executable: String,
    arguments: String,
    report: Option<Report>,
    selected_tool: String,
    call_arguments: String,
    call_confirm: bool,
    receiver: Option<Receiver<Result<Report, String>>>,
    cancelled: Arc<AtomicBool>,
    message: String,
}

impl Default for McpState {
    fn default() -> Self {
        Self {
            executable: String::new(),
            arguments: "[]".into(),
            report: None,
            selected_tool: String::new(),
            call_arguments: "{}".into(),
            call_confirm: false,
            receiver: None,
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
    pub fn preview_fixture(&mut self) {
        self.executable = r"C:\Tools\mcp-demo.exe".into();
        self.arguments = "[\"--stdio\"]".into();
        self.report = Some(Report {
            server: "示例 MCP 服务".into(),
            protocol: "2025-06-18".into(),
            tools: vec![
                json!({"name":"search_notes","description":"在已授权的笔记中搜索关键字","inputSchema":{"type":"object","properties":{"query":{"type":"string","description":"搜索词"}},"required":["query"]}}),
                json!({"name":"read_note","description":"读取单篇笔记","inputSchema":{"type":"object","properties":{"id":{"type":"string"}},"required":["id"]}}),
            ],
            resources: vec![json!({"name":"说明文档","uri":"demo://guide"})],
            prompts: vec![json!({"name":"summarize","description":"概括选定笔记"})],
            call_result: None,
        });
        self.selected_tool = "search_notes".into();
        self.call_arguments = "{\n  \"query\": \"Rust 错误处理\"\n}".into();
        self.message = "合成界面预览 · 未启动外部程序".into();
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

    fn start(&mut self, action: Action) {
        let config = match self.config() {
            Ok(config) => config,
            Err(error) => {
                self.message = error;
                return;
            }
        };
        let (sender, receiver) = mpsc::channel();
        if matches!(action, Action::Inspect) {
            self.report = None;
        } else if let Some(report) = &mut self.report {
            report.call_result = None;
        }
        self.cancelled = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::clone(&self.cancelled);
        std::thread::spawn(move || {
            let result = mcp::run(config, action, cancelled).map_err(|error| error.to_string());
            let _ = sender.send(result);
        });
        self.receiver = Some(receiver);
        self.message = "正在与 MCP 服务通信…".into();
        self.call_confirm = false;
    }

    fn poll(&mut self, ui: &egui::Ui) {
        let Some(receiver) = &self.receiver else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(report)) => {
                if report.call_result.is_some() {
                    self.message = "工具响应已收到；本次会话已退出".into();
                } else {
                    self.message = "能力清单已读取；本次会话已退出".into();
                }
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
                self.report = Some(report);
                self.receiver = None;
            }
            Ok(Err(error)) => {
                self.message = error;
                self.receiver = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.message = "MCP 后台任务意外结束".into();
                self.receiver = None;
            }
            Err(mpsc::TryRecvError::Empty) => {
                ui.ctx().request_repaint_after(Duration::from_millis(80));
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.poll(ui);
        let busy = self.receiver.is_some();
        ui.heading("MCP 协议调试台");
        ui.label("明确启动本机 stdio 服务，查看能力并手动调用工具。每次操作建立短会话；不会保存路径、参数或结果。");
        ui.add_space(8.0);
        ui.group(|ui| {
            ui.strong("连接目标");
            ui.weak(
                "只接受存在的绝对 EXE 路径；不经 shell。服务程序由你选择执行，本功能不是沙箱。",
            );
            ui.horizontal(|ui| {
                ui.label("可执行程序");
                if ui
                    .add_enabled(
                        !busy,
                        egui::TextEdit::singleline(&mut self.executable)
                            .desired_width((ui.available_width() - 100.0).max(280.0)),
                    )
                    .changed()
                {
                    self.report = None;
                    self.call_confirm = false;
                }
                #[cfg(windows)]
                if ui
                    .add_enabled(!busy, egui::Button::new("选择 EXE"))
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Executable", &["exe"])
                        .pick_file()
                {
                    self.executable = path.to_string_lossy().into_owned();
                    self.report = None;
                    self.call_confirm = false;
                }
            });
            ui.label("参数 · JSON 字符串数组");
            if ui
                .add_enabled(
                    !busy,
                    egui::TextEdit::singleline(&mut self.arguments).desired_width(f32::INFINITY),
                )
                .changed()
            {
                self.report = None;
                self.call_confirm = false;
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!busy, egui::Button::new("检查服务能力"))
                    .clicked()
                {
                    self.start(Action::Inspect);
                }
                if ui
                    .add_enabled(busy, egui::Button::new("停止并退出进程"))
                    .clicked()
                {
                    self.cancelled.store(true, Ordering::Relaxed);
                    self.message = "正在停止 MCP 进程…".into();
                }
                if busy {
                    ui.spinner();
                }
            });
        });
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        let Some(report) = &self.report else { return };
        let mut requested_call = None;
        ui.add_space(8.0);
        ui.heading(format!("{} · MCP {}", report.server, report.protocol));
        ui.label(format!(
            "{} 个工具 · {} 个资源 · {} 个提示词",
            report.tools.len(),
            report.resources.len(),
            report.prompts.len()
        ));
        ui.weak("能力与结果仅供本次查看；列表不会自动读取资源或执行工具。");
        if !report.tools.is_empty() {
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
                            self.call_arguments = "{}".into();
                        }
                    }
                });
            if let Some(tool) = report
                .tools
                .iter()
                .find(|tool| tool.get("name").and_then(Value::as_str) == Some(&self.selected_tool))
            {
                if let Some(description) = tool.get("description").and_then(Value::as_str) {
                    ui.label(description.chars().take(500).collect::<String>());
                }
                if let Some(schema) = tool.get("inputSchema") {
                    egui::CollapsingHeader::new("输入参数 schema")
                        .default_open(true)
                        .show(ui, |ui| {
                            ui.monospace(display_json(schema, 12_000));
                        });
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
            }
            if ui
                .add_enabled(!busy, egui::Button::new("准备调用选中工具"))
                .clicked()
            {
                self.call_confirm = true;
            }
            if self.call_confirm {
                ui.horizontal_wrapped(|ui| {
                    ui.weak(format!(
                        "将启动 {} 并调用 {}；参数会交给该服务。",
                        self.executable, self.selected_tool
                    ));
                    if ui.button("确认调用").clicked() {
                        match serde_json::from_str::<Value>(&self.call_arguments) {
                            Ok(arguments) if arguments.is_object() => {
                                requested_call = Some(Action::Call {
                                    tool: self.selected_tool.clone(),
                                    arguments,
                                })
                            }
                            _ => self.message = "调用参数必须是 JSON 对象".into(),
                        }
                    }
                    if ui.button("取消").clicked() {
                        self.call_confirm = false;
                    }
                });
            }
        }
        for (title, items) in [("资源", &report.resources), ("提示词", &report.prompts)] {
            if !items.is_empty() {
                egui::CollapsingHeader::new(format!("{title} · {} 项", items.len())).show(
                    ui,
                    |ui| {
                        for item in items {
                            let name = item.get("name").and_then(Value::as_str).unwrap_or("未命名");
                            let detail = item.get("uri").and_then(Value::as_str).unwrap_or("");
                            ui.label(format!(
                                "{name}  {}",
                                detail.chars().take(300).collect::<String>()
                            ));
                        }
                    },
                );
            }
        }
        if let Some(result) = &report.call_result {
            ui.separator();
            ui.strong("工具响应");
            let display = display_json(result, 30_000);
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .show(ui, |ui| {
                    ui.monospace(display);
                });
        }
        if let Some(action) = requested_call {
            self.start(action);
        }
    }
}
