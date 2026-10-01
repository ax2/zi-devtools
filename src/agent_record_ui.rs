//! A read-only viewer for explicitly exported Agent run records.
use crate::agent_record::{self, ImportedRecord};
use eframe::egui;
use std::path::Path;

#[derive(Default)]
pub struct State {
    record: Option<ImportedRecord>,
    file_name: String,
    show_content: bool,
    message: String,
    message_error: bool,
}

impl State {
    fn load(&mut self, path: &Path) {
        match agent_record::load_file(path) {
            Ok(record) => {
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

    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self, show_content: bool) {
        use serde_json::json;
        let record = json!({
            "schema":"zi-devtools-agent-run",
            "schema_version":1,
            "finished_at_utc":"2026-10-01T01:00:00Z",
            "status":"completed",
            "model":"qwen2.5:7b",
            "approved":true,
            "allowed_tools":["search_knowledge"],
            "max_calls":2,
            "calls_made":1,
            "model_tokens_reported":428,
            "steps":[{"tool":"search_knowledge","elapsed_ms":138,"content_items":2,"response_bytes":512,"model_excerpt_bytes":246,"is_error":false}],
            "content":{"goal":"查找 Rust 错误处理笔记","plan":"检索已授权知识索引，再总结结果。","answer":"合成记录：先传播错误，再在边界补充上下文。","error":null}
        });
        self.record = Some(agent_record::parse_json(record.to_string().as_bytes()).unwrap());
        self.file_name = "zi-agent-run-example.json".into();
        self.message = "合成记录预览；未读取本机文件".into();
        self.message_error = false;
        self.show_content = show_content;
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Agent 运行记录查看器");
        ui.label("打开 Zi DevTools 导出的 JSON，按时间线只读回看。不会连接模型或 MCP 服务，也不会重新执行工具调用。");
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
        });
        if !self.message.is_empty() {
            if self.message_error {
                ui.colored_label(egui::Color32::from_rgb(216, 102, 92), &self.message);
            } else {
                ui.weak(&self.message);
            }
        }
        let Some(record) = &self.record else {
            ui.add_space(8.0);
            ui.weak("尚未打开记录。文件最多 1 MiB；只接受当前支持的 schema v1。");
            return;
        };
        ui.add_space(8.0);
        ui.group(|ui| {
            ui.set_min_width(ui.available_width());
            ui.strong("01 执行概览");
            ui.label(format!("文件：{}", self.file_name));
            let status = match record.status.as_str() {
                "completed" => "已完成",
                "failed" => "失败",
                "cancelled" => "已取消",
                _ => "未知",
            };
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
                    "{} 个内容项 · 响应 {} 字节 · 送入模型摘录 {} 字节",
                    step.content_items, step.response_bytes, step.model_excerpt_bytes
                ));
            }
        });
        ui.add_space(8.0);
        ui.group(|ui| {
            ui.set_min_width(ui.available_width());
            ui.strong("03 可选任务内容");
            if let Some(content) = &record.content {
                ui.checkbox(
                    &mut self.show_content,
                    "显示目标、计划、答案与错误文字（可能含本机资料）",
                );
                if self.show_content {
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
