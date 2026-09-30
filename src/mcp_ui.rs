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

fn declared_read_only(tool: &Value) -> bool {
    tool.pointer("/annotations/readOnlyHint") == Some(&Value::Bool(true))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum McpSection {
    Tools,
    Resources,
    Prompts,
}

pub struct McpState {
    executable: String,
    arguments: String,
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
            section: McpSection::Tools,
            selected_tool: String::new(),
            selected_resource: String::new(),
            selected_prompt: String::new(),
            call_arguments: "{}".into(),
            confirmation_name: String::new(),
            last_called_tool: String::new(),
            prompt_arguments: "{}".into(),
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
    pub fn preview_fixture(&mut self, show_prompts: bool) {
        self.executable = r"C:\Tools\mcp-demo.exe".into();
        self.arguments = "[\"--stdio\"]".into();
        self.report = Some(Report {
            server: "示例 MCP 服务".into(),
            protocol: "2025-06-18".into(),
            tools: vec![
                json!({"name":"search_notes","description":"在已授权的笔记中搜索关键字","annotations":{"readOnlyHint":true},"inputSchema":{"type":"object","properties":{"query":{"type":"string","description":"搜索词"}},"required":["query"]}}),
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
        self.cancelled = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::clone(&self.cancelled);
        std::thread::spawn(move || {
            let result = mcp::run(config, action, cancelled).map_err(|error| error.to_string());
            let _ = sender.send(result);
        });
        self.receiver = Some(receiver);
        self.message = "正在与 MCP 服务通信…".into();
        self.call_confirm = false;
        self.confirmation_name.clear();
    }

    fn poll(&mut self, ui: &egui::Ui) {
        let Some(receiver) = &self.receiver else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(report)) => {
                self.message = if report.call_result.is_some() {
                    "工具响应已收到；本次会话已退出"
                } else if report.resource_result.is_some() {
                    "资源内容已收到；本次会话已退出"
                } else if report.prompt_result.is_some() {
                    "提示词内容已收到；本次会话已退出"
                } else {
                    "能力清单已读取；本次会话已退出"
                }
                .into();
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
        ui.label("明确启动本机 stdio 服务，查看能力并手动调用工具、读取资源或获取提示词。每次操作建立短会话；不会保存路径、参数或结果。");
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
                    self.confirmation_name.clear();
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
                    self.confirmation_name.clear();
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
                self.confirmation_name.clear();
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
        let mut requested_action = None;
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
            if let Some(tool) = &selected_descriptor {
                if let Some(description) = tool.get("description").and_then(Value::as_str) {
                    ui.label(description.chars().take(500).collect::<String>());
                }
                if declared_read_only(tool) {
                    ui.colored_label(egui::Color32::from_rgb(90, 160, 110), "服务声明：只读工具");
                } else {
                    ui.colored_label(
                        egui::Color32::from_rgb(210, 145, 60),
                        "服务未声明只读；调用可能修改数据或访问外部系统",
                    );
                }
                ui.small("工具行为由服务自行声明，不能证明程序安全；仅连接可信服务。");
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
                self.confirmation_name.clear();
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
                    let read_only = selected_descriptor.as_ref().is_some_and(declared_read_only);
                    if !read_only {
                        ui.label("输入工具名再次确认：");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.confirmation_name)
                                .desired_width(170.0),
                        );
                    }
                    if ui
                        .add_enabled(
                            read_only || self.confirmation_name == self.selected_tool,
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
                                })
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
            self.start(action);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::declared_read_only;
    use serde_json::json;

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
