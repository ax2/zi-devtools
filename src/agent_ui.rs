use crate::{agent, agent_record, mcp};
use eframe::egui;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::Duration,
};

enum Event {
    Inspected(Result<agent::Inspection, String>),
    Planned(Result<agent::Plan, String>),
    Step(agent::Step),
    Finished(Result<agent::Outcome, String>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Draft,
    Inspecting,
    Ready,
    Planning,
    AwaitingApproval,
    Running,
    Complete,
    Failed,
    Cancelled,
}

impl Phase {
    fn label(self) -> &'static str {
        match self {
            Self::Draft => "草稿",
            Self::Inspecting => "检查授权",
            Self::Ready => "选择工具",
            Self::Planning => "生成计划",
            Self::AwaitingApproval => "等待批准",
            Self::Running => "执行中",
            Self::Complete => "已完成",
            Self::Failed => "失败",
            Self::Cancelled => "已取消",
        }
    }
}

pub struct State {
    endpoint: String,
    model: String,
    executable: String,
    arguments: String,
    goal: String,
    max_calls: usize,
    access_path: PathBuf,
    inspection: Option<agent::Inspection>,
    selected: BTreeSet<String>,
    plan: Option<agent::Plan>,
    outcome: Option<agent::Outcome>,
    steps: Vec<agent::Step>,
    approved: bool,
    finished_at: Option<String>,
    execution_error: Option<String>,
    include_export_content: bool,
    #[cfg(feature = "ui-preview")]
    preview_scroll_bottom: bool,
    phase: Phase,
    message: String,
    receiver: Option<Receiver<Event>>,
    cancelled: Arc<AtomicBool>,
}

impl State {
    pub fn new(access_path: PathBuf) -> Self {
        Self {
            endpoint: "http://127.0.0.1:11434/api/chat".into(),
            model: "qwen2.5:7b".into(),
            executable: String::new(),
            arguments: "[]".into(),
            goal: String::new(),
            max_calls: 3,
            access_path,
            inspection: None,
            selected: BTreeSet::new(),
            plan: None,
            outcome: None,
            steps: Vec::new(),
            approved: false,
            finished_at: None,
            execution_error: None,
            include_export_content: false,
            #[cfg(feature = "ui-preview")]
            preview_scroll_bottom: false,
            phase: Phase::Draft,
            message: String::new(),
            receiver: None,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    fn server(&self) -> Result<mcp::Config, String> {
        let args: Vec<String> = serde_json::from_str(&self.arguments)
            .map_err(|_| "MCP 参数须为 JSON 字符串数组".to_owned())?;
        let server = mcp::Config {
            executable: PathBuf::from(self.executable.trim()),
            args,
        };
        server.validate().map_err(|error| error.to_string())?;
        Ok(server)
    }

    fn invalidate(&mut self, inspection: bool) {
        if inspection {
            self.inspection = None;
            self.selected.clear();
        }
        self.plan = None;
        self.outcome = None;
        self.steps.clear();
        self.approved = false;
        self.finished_at = None;
        self.execution_error = None;
        self.include_export_content = false;
        self.phase = if self.inspection.is_some() {
            Phase::Ready
        } else {
            Phase::Draft
        };
        self.message.clear();
    }

    fn start_inspect(&mut self) {
        let server = match self.server() {
            Ok(value) => value,
            Err(error) => {
                self.message = error;
                return;
            }
        };
        let access_path = self.access_path.clone();
        self.invalidate(true);
        self.cancelled = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::clone(&self.cancelled);
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result =
                agent::inspect(&server, &access_path, cancelled).map_err(|error| error.to_string());
            let _ = sender.send(Event::Inspected(result));
        });
        self.receiver = Some(receiver);
        self.phase = Phase::Inspecting;
    }

    fn start_plan(&mut self) {
        let server = match self.server() {
            Ok(value) => value,
            Err(error) => {
                self.message = error;
                return;
            }
        };
        let config = agent::Config {
            endpoint: self.endpoint.trim().into(),
            model: self.model.trim().into(),
            server,
            access_path: self.access_path.clone(),
            goal: self.goal.trim().into(),
            selected: self.selected.iter().cloned().collect(),
            max_calls: self.max_calls,
        };
        self.invalidate(false);
        self.cancelled = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::clone(&self.cancelled);
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = agent::prepare(config, cancelled).map_err(|error| error.to_string());
            let _ = sender.send(Event::Planned(result));
        });
        self.receiver = Some(receiver);
        self.phase = Phase::Planning;
    }

    fn start_execute(&mut self) {
        let Some(plan) = self.plan.clone() else {
            return;
        };
        self.cancelled = Arc::new(AtomicBool::new(false));
        self.approved = true;
        self.finished_at = None;
        self.execution_error = None;
        let cancelled = Arc::clone(&self.cancelled);
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = agent::execute(plan, cancelled, |step| {
                let _ = sender.send(Event::Step(step));
            })
            .map_err(|error| error.to_string());
            let _ = sender.send(Event::Finished(result));
        });
        self.receiver = Some(receiver);
        self.phase = Phase::Running;
        self.message.clear();
    }

    fn poll(&mut self, ui: &egui::Ui) {
        let Some(receiver) = &self.receiver else {
            return;
        };
        let mut done = false;
        loop {
            let event = match receiver.try_recv() {
                Ok(event) => event,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if !done {
                        self.phase = Phase::Failed;
                        self.message = "Agent 后台任务意外结束".into();
                        if self.approved {
                            self.execution_error = Some(self.message.clone());
                            self.finished_at = Some(chrono::Utc::now().to_rfc3339());
                        }
                        done = true;
                    }
                    break;
                }
            };
            match event {
                Event::Inspected(Ok(inspection)) => {
                    self.message = format!(
                        "{}：{} 个可直接调用的只读工具",
                        inspection.server_name,
                        inspection.tools.len()
                    );
                    self.inspection = Some(inspection);
                    self.phase = Phase::Ready;
                    done = true;
                }
                Event::Planned(Ok(plan)) => {
                    self.plan = Some(plan);
                    self.phase = Phase::AwaitingApproval;
                    done = true;
                }
                Event::Step(step) => self.steps.push(step),
                Event::Finished(Ok(outcome)) => {
                    self.outcome = Some(outcome);
                    self.phase = Phase::Complete;
                    self.finished_at = Some(chrono::Utc::now().to_rfc3339());
                    done = true;
                }
                Event::Inspected(Err(error))
                | Event::Planned(Err(error))
                | Event::Finished(Err(error)) => {
                    self.message = error;
                    self.phase = if self.cancelled.load(Ordering::Relaxed) {
                        Phase::Cancelled
                    } else {
                        Phase::Failed
                    };
                    if self.approved {
                        self.execution_error = Some(self.message.clone());
                        self.finished_at = Some(chrono::Utc::now().to_rfc3339());
                    }
                    done = true;
                }
            }
        }
        if done {
            self.receiver = None;
        } else {
            ui.ctx().request_repaint_after(Duration::from_millis(80));
        }
    }

    fn export_json(&self) -> Result<String, String> {
        let plan = self.plan.as_ref().ok_or("没有可导出的执行计划")?;
        let status = match self.phase {
            Phase::Complete => agent_record::Status::Completed,
            Phase::Failed => agent_record::Status::Failed,
            Phase::Cancelled => agent_record::Status::Cancelled,
            _ => return Err("执行尚未结束".into()),
        };
        agent_record::export_json(
            agent_record::Snapshot {
                plan,
                steps: &self.steps,
                outcome: self.outcome.as_ref(),
                status,
                approved: self.approved,
                finished_at: self.finished_at.as_deref().unwrap_or(""),
                error: self.execution_error.as_deref(),
            },
            self.include_export_content,
        )
        .map_err(|error| error.to_string())
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self, completed: bool) {
        use serde_json::json;
        self.executable = r"C:\Tools\ZiDevToolsMcp.exe".into();
        self.model = "qwen2.5:7b".into();
        self.max_calls = 2;
        self.goal = "查找本地笔记中有关 Rust 错误处理的资料，并总结要点。".into();
        let tool = json!({"name":"search_knowledge","description":"只读检索本机知识索引","inputSchema":{"type":"object","properties":{"query":{"type":"string"}},"required":["query"]},"annotations":{"readOnlyHint":true,"destructiveHint":false,"openWorldHint":false}});
        self.inspection = Some(agent::Inspection {
            server_name: "Zi 知识库 MCP".into(),
            tools: vec![tool.clone()],
        });
        self.selected.insert("search_knowledge".into());
        self.plan = Some(agent::Plan {
            text:
                "1. 用已授权的 search_knowledge 检索相关笔记。\n2. 核对返回片段，归纳可追溯要点。"
                    .into(),
            config: agent::Config {
                endpoint: self.endpoint.clone(),
                model: self.model.clone(),
                server: mcp::Config {
                    executable: PathBuf::from(&self.executable),
                    args: vec![],
                },
                access_path: self.access_path.clone(),
                goal: self.goal.clone(),
                selected: vec!["search_knowledge".into()],
                max_calls: 2,
            },
            tools: vec![tool],
        });
        self.phase = if completed {
            Phase::Complete
        } else {
            Phase::AwaitingApproval
        };
        if completed {
            self.steps = vec![agent::Step {
                tool: "search_knowledge".into(),
                elapsed_ms: 138,
                result: "检索到 2 条本机片段（合成预览）".into(),
                content_items: 2,
                response_bytes: 512,
                model_excerpt_bytes: 246,
                is_error: false,
                references: vec![agent::EvidenceRef {
                    citation_id: Some(1),
                    source_id: "synthetic-source".into(),
                    source_name: "Rust 笔记".into(),
                    relative_path: "notes/rust-errors.md".into(),
                    location: "第 2 段".into(),
                    file_sha256: "a".repeat(64),
                    chunk_sha256: "b".repeat(64),
                }],
            }];
            self.outcome = Some(agent::Outcome { answer:"找到两条相关笔记。建议先用 Result 传播错误，再在边界处补充上下文。[K1] 此处为合成界面预览。".into(), steps:self.steps.clone(), model_tokens:428, citations: vec![1] });
            self.approved = true;
            self.finished_at = Some("2026-10-01T01:00:00Z".into());
        }
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_record_export(&mut self) {
        self.preview_fixture(true);
        self.preview_scroll_bottom = true;
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_scroll_bottom(&self) -> bool {
        self.preview_scroll_bottom
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.poll(ui);
        let busy = self.receiver.is_some();
        ui.heading("本机 Agent 任务工作台");
        ui.label("由本机 Ollama 生成计划；批准后最多调用四次已单独授权的只读 MCP 工具。计划与结果仅在本次运行内存中保留。");
        ui.add_space(8.0);
        ui.group(|ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.strong("01 连接与任务");
                ui.weak(format!("状态：{}", self.phase.label()));
            });
            ui.label("Ollama /api/chat（仅 127.0.0.1 或 ::1）");
            if ui
                .add_enabled(
                    !busy,
                    egui::TextEdit::singleline(&mut self.endpoint).desired_width(f32::INFINITY),
                )
                .changed()
            {
                self.invalidate(false);
            }
            ui.horizontal(|ui| {
                ui.label("模型");
                if ui
                    .add_enabled(
                        !busy,
                        egui::TextEdit::singleline(&mut self.model).desired_width(260.0),
                    )
                    .changed()
                {
                    self.invalidate(false);
                }
                ui.label("调用上限");
                if ui
                    .add_enabled(!busy, egui::Slider::new(&mut self.max_calls, 1..=4))
                    .changed()
                {
                    self.invalidate(false);
                }
            });
            ui.label("MCP 服务 EXE（先在协议调试台为具体工具授予直接许可）");
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !busy,
                        egui::TextEdit::singleline(&mut self.executable)
                            .desired_width((ui.available_width() - 100.0).max(280.0)),
                    )
                    .changed()
                {
                    self.invalidate(true);
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
                    self.invalidate(true);
                }
            });
            ui.label("服务参数 · JSON 字符串数组");
            if ui
                .add_enabled(
                    !busy,
                    egui::TextEdit::singleline(&mut self.arguments).desired_width(f32::INFINITY),
                )
                .changed()
            {
                self.invalidate(true);
            }
            ui.label("任务目标");
            if ui
                .add_enabled(
                    !busy,
                    egui::TextEdit::multiline(&mut self.goal)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                )
                .changed()
            {
                self.invalidate(false);
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!busy, egui::Button::new("检查已授权工具"))
                    .clicked()
                {
                    self.start_inspect();
                }
                if ui
                    .add_enabled(busy, egui::Button::new("取消当前操作"))
                    .clicked()
                {
                    self.cancelled.store(true, Ordering::Relaxed);
                    self.message = "正在取消请求并停止 MCP 进程…".into();
                }
                if busy {
                    ui.spinner();
                }
            });
        });
        if !self.message.is_empty() {
            ui.colored_label(egui::Color32::from_rgb(216, 156, 70), &self.message);
        }
        let Some(inspection) = &self.inspection else {
            return;
        };
        let tools: Vec<(String, String)> = inspection
            .tools
            .iter()
            .filter_map(|tool: &Value| {
                Some((
                    tool.get("name")?.as_str()?.to_owned(),
                    tool.get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                ))
            })
            .collect();
        ui.add_space(8.0);
        ui.group(|ui| {
            ui.set_min_width(ui.available_width());
            ui.strong("02 本次工具白名单");
            ui.weak("只显示完整声明低影响只读且已有直接许可的工具。勾选仅对本次任务有效；此页不授予权限。");
            if tools.is_empty() { ui.label("暂无可用工具。请先到 MCP 协议调试台检查并单独授权。"); }
            for (name, description) in &tools {
                let mut checked = self.selected.contains(name);
                if ui.add_enabled(!busy, egui::Checkbox::new(&mut checked, format!("{name}  ·  {description}"))).changed() {
                    if checked { self.selected.insert(name.clone()); } else { self.selected.remove(name); }
                    self.invalidate(false);
                }
            }
            if ui.add_enabled(!busy && !self.selected.is_empty(), egui::Button::new("生成执行计划")).clicked() { self.start_plan(); }
        });
        if let Some(plan) = self.plan.clone() {
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.set_min_width(ui.available_width());
                ui.strong("03 审阅与批准");
                ui.label(&plan.text);
                ui.weak(format!(
                    "本次仅允许：{} · 最多 {} 次调用",
                    plan.config.selected.join("、"),
                    plan.config.max_calls
                ));
                if ui
                    .add_enabled(
                        self.phase == Phase::AwaitingApproval && !busy,
                        egui::Button::new("批准并执行只读任务"),
                    )
                    .clicked()
                {
                    self.start_execute();
                }
            });
        }
        if !self.steps.is_empty() {
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.set_min_width(ui.available_width());
                ui.strong("04 执行记录");
                for (index, step) in self.steps.iter().enumerate() {
                    egui::CollapsingHeader::new(
                        format!(
                            "{}. {} · {} ms{}{}",
                            index + 1,
                            step.tool,
                            step.elapsed_ms,
                            if step.is_error { " · 错误" } else { "" },
                            if step.references.is_empty() { String::new() } else { format!(" · 模型消息来源 {} 条", step.references.len()) }
                        ),
                    ).default_open(!step.references.is_empty()).show(ui, |ui| {
                            ui.monospace(&step.result);
                            if !step.references.is_empty() {
                                ui.weak("以下来源与片段已选入下一轮模型消息；失败或取消时可能尚未发送。第三方服务的来源声明未独立认证。");
                                for reference in &step.references {
                                    ui.label(format!("{}{} · {} · {}", reference.citation_id.map_or(String::new(), |id| format!("[K{id}] ")), reference.source_name, reference.relative_path, reference.location));
                                    ui.weak(format!("来源 ID：{}", reference.source_id));
                                    ui.monospace(format!("文件 SHA-256：{}", reference.file_sha256));
                                    ui.monospace(format!("片段 SHA-256：{}", reference.chunk_sha256));
                                }
                            }
                        });
                }
            });
        }
        if let Some(outcome) = &self.outcome {
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.set_min_width(ui.available_width());
                ui.strong("最终回答");
                ui.label(&outcome.answer);
                let has_references = outcome.steps.iter().any(|step| !step.references.is_empty());
                if has_references {
                    if outcome.citations.is_empty() {
                        ui.weak("回答未提供可核对的 [K编号] 引用；请结合检索片段自行核查。");
                    } else {
                        ui.weak(format!(
                            "本次回答引用编号已核对：{}。编号有效不证明陈述正确。",
                            outcome
                                .citations
                                .iter()
                                .map(|id| format!("[K{id}]"))
                                .collect::<Vec<_>>()
                                .join("、")
                        ));
                    }
                }
                ui.weak(format!(
                    "{} 次工具调用 · 模型报告 {} tokens",
                    outcome.steps.len(),
                    outcome.model_tokens
                ));
            });
        }
        if self.approved && self.finished_at.is_some() && !busy {
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.set_min_width(ui.available_width());
                ui.strong("05 运行记录导出");
                ui.weak("默认只含状态、工具与用量摘要，不含任务目标、计划、答案、服务路径、参数或工具原文。只有主动保存才写入文件。");
                ui.checkbox(&mut self.include_export_content, "额外包含目标、计划、答案、错误文字与来源路径（可能含本机资料）");
                match self.export_json() {
                    Ok(record) => {
                        let preview = egui::CollapsingHeader::new(format!("预览将保存的 JSON · {} 字节", record.len()));
                        #[cfg(feature = "ui-preview")]
                        let preview = preview.default_open(self.preview_scroll_bottom);
                        preview.show(ui, |ui| {
                            egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                                ui.monospace(&record);
                            });
                        });
                        #[cfg(windows)]
                        if ui.button("保存运行记录 JSON…").clicked()
                            && let Some(path) = rfd::FileDialog::new()
                                .add_filter("JSON", &["json"])
                                .set_file_name("zi-agent-run.json")
                                .save_file()
                        {
                            match std::fs::write(&path, &record) {
                                Ok(()) => self.message = format!("运行记录已保存：{}", path.display()),
                                Err(error) => self.message = format!("保存运行记录失败：{error}"),
                            }
                        }
                    }
                    Err(error) => { ui.colored_label(egui::Color32::from_rgb(216, 156, 70), error); }
                }
            });
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}
