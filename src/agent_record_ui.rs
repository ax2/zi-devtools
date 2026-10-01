//! A read-only viewer for explicitly exported Agent run records.
use crate::{
    agent_record::{self, ImportedRecord},
    agent_record_compare,
    agent_record_library::{self, Entry, ScanResult},
};
use crossbeam_channel::{Receiver, bounded};
use eframe::egui;
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

struct FolderTask {
    receiver: Receiver<anyhow::Result<ScanResult>>,
    cancelled: Arc<AtomicBool>,
}

impl Drop for FolderTask {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

#[derive(Default)]
pub struct State {
    record: Option<ImportedRecord>,
    file_name: String,
    show_content: bool,
    message: String,
    message_error: bool,
    folder_name: String,
    library: Vec<Entry>,
    rejected: usize,
    search: String,
    search_content: bool,
    folder_task: Option<FolderTask>,
    compare_baseline: Option<usize>,
    compare_candidate: Option<usize>,
}

impl State {
    fn load(&mut self, path: &Path) {
        self.cancel_folder();
        match agent_record::load_file(path) {
            Ok(record) => {
                self.library.clear();
                self.compare_baseline = None;
                self.compare_candidate = None;
                self.folder_name.clear();
                self.search.clear();
                self.search_content = false;
                self.file_name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "运行记录.json".into());
                self.record = Some(record);
                self.show_content = false;
                self.message = "记录已读取；不会运行模型或 MCP 工具".into();
                self.message_error = false;
            }
            Err(error) => {
                self.message = format!("无法读取记录：{error}");
                self.message_error = true;
            }
        }
    }

    fn start_folder(&mut self, path: &Path) {
        self.cancel_folder();
        self.record = None;
        self.file_name.clear();
        self.library.clear();
        self.compare_baseline = None;
        self.compare_candidate = None;
        self.rejected = 0;
        self.show_content = false;
        self.search_content = false;
        self.search.clear();
        self.folder_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "所选目录".into());
        self.message = "正在读取目录中的 JSON 记录…".into();
        self.message_error = false;
        let (sender, receiver) = bounded(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let folder = path.to_path_buf();
        std::thread::spawn(move || {
            let result = agent_record_library::scan_folder(&folder, &worker_cancelled);
            let _ = sender.send(result);
        });
        self.folder_task = Some(FolderTask {
            receiver,
            cancelled,
        });
    }

    fn cancel_folder(&mut self) {
        if let Some(task) = self.folder_task.take() {
            task.cancelled.store(true, Ordering::Relaxed);
        }
    }

    fn poll_folder(&mut self, ui: &egui::Ui) {
        let Some(task) = &self.folder_task else {
            return;
        };
        match task.receiver.try_recv() {
            Ok(Ok(scan)) => {
                self.folder_task = None;
                self.rejected = scan.rejected;
                self.library = scan.entries;
                self.message = format!(
                    "已读取 {} 份记录；跳过 {} 份无效或超限 JSON。只在当前窗口保留。",
                    self.library.len(),
                    self.rejected
                );
                self.message_error = false;
            }
            Ok(Err(error)) => {
                self.folder_task = None;
                self.message = format!("无法读取目录：{error}");
                self.message_error = true;
            }
            Err(crossbeam_channel::TryRecvError::Empty) => {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(100));
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                self.folder_task = None;
                self.message = "目录读取线程意外结束".into();
                self.message_error = true;
            }
        }
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self, show_content: bool) {
        use serde_json::json;
        let record = json!({
            "schema":"zi-devtools-agent-run",
            "schema_version":3,
            "finished_at_utc":"2026-10-01T01:00:00Z",
            "status":"completed",
            "model":"qwen2.5:7b",
            "approved":true,
            "allowed_tools":["search_knowledge"],
            "max_calls":2,
            "calls_made":1,
            "model_tokens_reported":428,
            "steps":[{"tool":"search_knowledge","elapsed_ms":138,"content_items":2,"response_bytes":512,"model_excerpt_bytes":246,"is_error":false,
                "references":[{"citation_id":1,"source_id":"synthetic-source","source_name":"Rust 笔记","relative_path":"notes/rust-errors.md","location":"第 2 段","file_sha256":"a".repeat(64),"chunk_sha256":"b".repeat(64)}]}],
            "content":{"goal":"查找 Rust 错误处理笔记","plan":"检索已授权知识索引，再总结结果。","answer":"合成记录：先传播错误，再在边界补充上下文。[K1]","error":null,"citations":[1]}
        });
        self.record = Some(agent_record::parse_json(record.to_string().as_bytes()).unwrap());
        self.file_name = "zi-agent-run-example.json".into();
        self.message = "合成记录预览；未读取本机文件".into();
        self.message_error = false;
        self.show_content = show_content;
        let earlier = serde_json::json!({
            "schema":"zi-devtools-agent-run","schema_version":1,
            "finished_at_utc":"2026-09-30T01:00:00Z","status":"failed",
            "model":"qwen2.5:7b","approved":true,
            "allowed_tools":["search_knowledge"],"max_calls":2,"calls_made":0,
            "model_tokens_reported":null,"steps":[]
        });
        self.library = vec![
            Entry {
                file_name: "zi-agent-run-example.json".into(),
                record: self.record.as_ref().unwrap().clone(),
            },
            Entry {
                file_name: "zi-agent-run-earlier.json".into(),
                record: agent_record::parse_json(earlier.to_string().as_bytes()).unwrap(),
            },
        ];
        self.folder_name = "合成记录目录".into();
        self.compare_baseline = Some(1);
        self.compare_candidate = Some(0);
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.poll_folder(ui);
        ui.heading("Agent 运行记录查看器");
        ui.label("打开单份 JSON 或明确选择一个目录，按时间线只读回看并检索记录。不会连接模型或 MCP 服务，也不会重新执行工具调用。");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            #[cfg(windows)]
            if ui.button("打开 JSON 记录…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Agent JSON", &["json"])
                    .pick_file()
            {
                self.load(&path);
            }
            #[cfg(windows)]
            if ui.button("读取记录目录…").clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                self.start_folder(&path);
            }
            if ui
                .add_enabled(self.folder_task.is_some(), egui::Button::new("停止读取"))
                .clicked()
            {
                self.cancel_folder();
                self.message = "已停止目录读取".into();
            }
            if ui
                .add_enabled(self.record.is_some(), egui::Button::new("清空当前记录"))
                .clicked()
            {
                self.record = None;
                self.file_name.clear();
                self.show_content = false;
                self.message = "已从当前窗口移除记录".into();
                self.message_error = false;
            }
            if ui
                .add_enabled(
                    !self.library.is_empty() || self.folder_task.is_some(),
                    egui::Button::new("清空目录结果"),
                )
                .clicked()
            {
                self.cancel_folder();
                self.library.clear();
                self.compare_baseline = None;
                self.compare_candidate = None;
                self.folder_name.clear();
                self.search.clear();
                self.search_content = false;
                self.rejected = 0;
                self.record = None;
                self.file_name.clear();
                self.show_content = false;
                self.message = "已从当前窗口移除目录结果".into();
                self.message_error = false;
            }
        });
        if !self.message.is_empty() {
            if self.message_error {
                ui.colored_label(egui::Color32::from_rgb(216, 102, 92), &self.message);
            } else {
                ui.weak(&self.message);
            }
        }
        if !self.library.is_empty() {
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.set_min_width(ui.available_width());
                ui.strong(format!("目录记录 · {}", self.folder_name));
                ui.horizontal(|ui| {
                    ui.label("搜索");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .hint_text("文件名、时间、状态、模型、工具"),
                    );
                    ui.checkbox(&mut self.search_content, "同时搜索任务内容");
                });
                let mut chosen = None;
                let mut shown = 0;
                egui::ScrollArea::vertical()
                    .id_salt("agent-record-library")
                    .max_height(240.0)
                    .show(ui, |ui| {
                        for (index, entry) in self.library.iter().enumerate() {
                            if !agent_record_library::matches(
                                entry,
                                &self.search,
                                self.search_content,
                            ) {
                                continue;
                            }
                            shown += 1;
                            ui.horizontal(|ui| {
                                if ui
                                    .button(format!(
                                        "{}  ·  {}  ·  {}  ·  {}",
                                        entry.record.finished_at_utc,
                                        agent_record_library::status_label(&entry.record.status),
                                        entry.record.model,
                                        entry.file_name
                                    ))
                                    .clicked()
                                {
                                    chosen = Some(index);
                                }
                                if ui
                                    .selectable_label(self.compare_baseline == Some(index), "基线")
                                    .clicked()
                                {
                                    self.compare_baseline = Some(index);
                                    if self.compare_candidate == Some(index) {
                                        self.compare_candidate = None;
                                    }
                                }
                                if ui
                                    .selectable_label(self.compare_candidate == Some(index), "候选")
                                    .clicked()
                                {
                                    self.compare_candidate = Some(index);
                                    if self.compare_baseline == Some(index) {
                                        self.compare_baseline = None;
                                    }
                                }
                            });
                        }
                    });
                ui.weak(format!(
                    "匹配 {shown} / {} 份记录；可选正文默认不参与搜索。",
                    self.library.len()
                ));
                if let Some(index) = chosen {
                    self.record = Some(self.library[index].record.clone());
                    self.file_name = self.library[index].file_name.clone();
                    self.show_content = false;
                }
            });
        }
        if self.library.len() >= 2 {
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.strong("元数据对比 · 基线 → 候选");
                    if ui
                        .add_enabled(
                            self.compare_baseline.is_some() || self.compare_candidate.is_some(),
                            egui::Button::new("清除对比选择"),
                        )
                        .clicked()
                    {
                        self.compare_baseline = None;
                        self.compare_candidate = None;
                    }
                });
                if let (Some(baseline), Some(candidate)) =
                    (self.compare_baseline, self.compare_candidate)
                    && baseline != candidate
                {
                    let comparison = agent_record_compare::compare(
                        &self.library[baseline].record,
                        &self.library[candidate].record,
                    );
                    ui.weak(format!(
                        "基线：{}  ·  候选：{}",
                        self.library[baseline].file_name, self.library[candidate].file_name
                    ));
                    ui.label(format!(
                        "状态：{} → {}  ·  模型：{} → {}",
                        agent_record_library::status_label(&comparison.baseline_status),
                        agent_record_library::status_label(&comparison.candidate_status),
                        comparison.baseline_model,
                        comparison.candidate_model
                    ));
                    ui.label(format!(
                        "工具调用：{} → {}  ·  模型报告 token：{}",
                        comparison.baseline_calls,
                        comparison.candidate_calls,
                        agent_record_compare::token_change(
                            comparison.baseline_tokens,
                            comparison.candidate_tokens
                        )
                    ));
                    if !comparison.allowed_added.is_empty() || !comparison.allowed_removed.is_empty() {
                        ui.label(format!(
                            "白名单新增：{}  ·  移除：{}",
                            if comparison.allowed_added.is_empty() { "无".into() } else { comparison.allowed_added.join("、") },
                            if comparison.allowed_removed.is_empty() { "无".into() } else { comparison.allowed_removed.join("、") },
                        ));
                    }
                    for tool in &comparison.tools {
                        ui.label(format!(
                            "{}  ·  调用 {}→{}  ·  耗时 {}→{} ms  ·  响应 {}→{} 字节  ·  错误 {}→{}",
                            tool.name,
                            tool.baseline.calls,
                            tool.candidate.calls,
                            tool.baseline.elapsed_ms,
                            tool.candidate.elapsed_ms,
                            tool.baseline.response_bytes,
                            tool.candidate.response_bytes,
                            tool.baseline.errors,
                            tool.candidate.errors,
                        ));
                    }
                    if ui.button("复制元数据对比摘要").clicked() {
                        ui.ctx().copy_text(comparison.metadata_summary());
                        self.message = "已复制不含任务正文的对比摘要".into();
                        self.message_error = false;
                    }
                    ui.weak("仅比较记录声明的元数据，不判断答案质量、真实性或费用。复制内容不含目标、计划、答案与错误正文。");
                } else {
                    ui.weak("在目录列表中分别选择一份基线和一份候选记录。两份记录不能相同。");
                }
            });
        }
        let Some(record) = &self.record else {
            ui.add_space(8.0);
            ui.weak("尚未打开记录。单份文件最多 1 MiB；目录仅扫描顶层 JSON，最多 64 份、累计 16 MiB，支持 schema v1/v2/v3。");
            return;
        };
        ui.add_space(8.0);
        ui.group(|ui| {
            ui.set_min_width(ui.available_width());
            ui.strong("01 执行概览");
            ui.label(format!("文件：{}", self.file_name));
            let status = agent_record_library::status_label(&record.status);
            ui.label(format!(
                "状态：{status}  ·  完成时间（UTC）：{}",
                record.finished_at_utc
            ));
            ui.label(format!("模型：{}  ·  人工批准：是", record.model));
            ui.label(format!(
                "调用：{} / {}  ·  模型报告用量：{}",
                record.calls_made,
                record.max_calls,
                record
                    .model_tokens_reported
                    .map_or("未知".into(), |tokens| format!("{tokens} tokens"))
            ));
            ui.label(format!(
                "本次工具白名单：{}",
                record.allowed_tools.join("、")
            ));
        });
        ui.add_space(8.0);
        ui.group(|ui| {
            ui.set_min_width(ui.available_width());
            ui.strong("02 工具调用时间线");
            if record.steps.is_empty() {
                ui.weak("本次没有已记录的工具调用。");
            }
            for (index, step) in record.steps.iter().enumerate() {
                ui.label(format!(
                    "{}. {} · {} ms{}",
                    index + 1,
                    step.tool,
                    step.elapsed_ms,
                    if step.is_error {
                        " · 工具报告错误"
                    } else {
                        ""
                    }
                ));
                ui.weak(format!(
                    "{} 个内容项 · 响应 {} 字节 · 下轮模型消息 {} 字节",
                    step.content_items, step.response_bytes, step.model_excerpt_bytes
                ));
                if let Some(references) = &step.references {
                    if self.show_content {
                        if record.schema_version == 3 {
                            ui.weak("记录声明这些来源已选入模型消息；失败或取消时可能尚未发送。导入文件不能认证来源或证明答案事实正确。");
                        } else {
                            ui.weak("旧版记录只保存工具返回的来源，无法确认每条是否送入模型。");
                        }
                        for reference in references {
                            ui.label(format!(
                                "{}{} · {} · {}",
                                reference.citation_id.map_or(String::new(), |id| format!("[K{id}] ")),
                                reference.source_name, reference.relative_path, reference.location
                            ));
                            ui.weak(format!("来源 ID：{}", reference.source_id));
                            ui.monospace(format!("文件 SHA-256：{}", reference.file_sha256));
                            ui.monospace(format!("片段 SHA-256：{}", reference.chunk_sha256));
                        }
                    } else {
                        ui.weak(format!("{} 条来源声明（默认隐藏路径）", references.len()));
                    }
                }
            }
        });
        ui.add_space(8.0);
        ui.group(|ui| {
            ui.set_min_width(ui.available_width());
            ui.strong("03 可选任务内容");
            if let Some(content) = &record.content {
                ui.checkbox(
                    &mut self.show_content,
                    "显示目标、计划、答案、错误文字与来源路径（可能含本机资料）",
                );
                if self.show_content {
                    if let Some(citations) = &content.citations {
                        if citations.is_empty() {
                            ui.weak("本次回答未提供可核对的知识引用。");
                        } else {
                            ui.weak(format!(
                                "记录中的引用编号：{}。编号有效不证明陈述正确。",
                                citations
                                    .iter()
                                    .map(|id| format!("[K{id}]"))
                                    .collect::<Vec<_>>()
                                    .join("、")
                            ));
                        }
                    }
                    for (label, value) in [
                        ("任务目标", Some(content.goal.as_str())),
                        ("批准计划", Some(content.plan.as_str())),
                        ("最终答案", content.answer.as_deref()),
                        ("错误文字", content.error.as_deref()),
                    ] {
                        if let Some(value) = value {
                            ui.strong(label);
                            egui::ScrollArea::vertical()
                                .id_salt(("agent-record-content", label))
                                .max_height(180.0)
                                .show(ui, |ui| {
                                    ui.label(value);
                                });
                        }
                    }
                }
            } else {
                ui.weak("导出时未选择包含任务内容，此记录没有目标、计划或答案。");
            }
        });
        ui.add_space(8.0);
        ui.weak("记录来自用户选定的文件，格式校验不证明内容真实。此工作台只读，不恢复执行状态。");
    }
}
