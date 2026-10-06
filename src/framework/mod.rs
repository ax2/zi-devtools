//! Optional development diagnostics. Inputs/results remain in memory.
mod contracts;
mod django;
mod java;
pub mod runtime;
#[cfg(test)]
mod tests;

use anyhow::{Context, Result, ensure};
use eframe::egui;
use serde_json::Value;
use std::{
    collections::HashMap,
    io::{Read, Write},
    sync::mpsc,
    time::{Duration, Instant},
};

fn bounded(input: &str) -> Result<()> {
    ensure!(input.len() <= 2 * 1024 * 1024, "输入超过 2 MiB");
    ensure!(input.lines().count() <= 20000, "输入超过 20000 行");
    Ok(())
}
fn report(value: Value) -> Result<String> {
    let text = serde_json::to_string_pretty(&value)?;
    ensure!(
        text.len() <= 8 * 1024 * 1024,
        "报告超过 8 MiB，请缩小输入范围"
    );
    Ok(text)
}

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum Tool {
    #[default]
    JavaEnvironment,
    PythonEnvironment,
    Threads,
    Dependencies,
    Migrations,
    Sql,
    Gc,
    Jfr,
    SpringConfig,
    Actuator,
    Urls,
    Drf,
    Checks,
    Celery,
}
impl Tool {
    pub const ALL: [Self; 14] = [
        Self::JavaEnvironment,
        Self::PythonEnvironment,
        Self::Threads,
        Self::Dependencies,
        Self::Migrations,
        Self::Sql,
        Self::Gc,
        Self::Jfr,
        Self::SpringConfig,
        Self::Actuator,
        Self::Urls,
        Self::Drf,
        Self::Checks,
        Self::Celery,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Self::JavaEnvironment => "java-environment",
            Self::PythonEnvironment => "django-environment",
            Self::Threads => "java-thread-dump",
            Self::Dependencies => "java-dependencies",
            Self::Migrations => "django-migrations",
            Self::Sql => "django-sql",
            Self::Gc => "java-gc-log",
            Self::Jfr => "java-jfr",
            Self::SpringConfig => "spring-config",
            Self::Actuator => "spring-actuator",
            Self::Urls => "django-urls",
            Self::Drf => "django-drf",
            Self::Checks => "django-checks",
            Self::Celery => "celery-diagnostics",
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.id() == id)
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::JavaEnvironment => "JDK / 构建环境",
            Self::PythonEnvironment => "Python / Django 环境",
            Self::Threads => "Java 线程转储",
            Self::Dependencies => "Maven / Gradle 依赖",
            Self::Migrations => "Django 迁移计划",
            Self::Sql => "Django SQL / N+1",
            Self::Gc => "Java GC 日志",
            Self::Jfr => "JFR 录制报告",
            Self::SpringConfig => "Spring 配置差异",
            Self::Actuator => "Spring Actuator",
            Self::Urls => "Django URL / reverse",
            Self::Drf => "DRF OpenAPI 对比",
            Self::Checks => "Django 部署检查",
            Self::Celery => "Celery 任务诊断",
        }
    }
    pub fn category(self) -> &'static str {
        if matches!(
            self,
            Self::JavaEnvironment
                | Self::Threads
                | Self::Dependencies
                | Self::Gc
                | Self::Jfr
                | Self::SpringConfig
                | Self::Actuator
        ) {
            "Java 与 JVM"
        } else {
            "Python 与 Django"
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            Self::JavaEnvironment => "选择 JDK，对比版本、JAVA_HOME 与构建入口",
            Self::PythonEnvironment => "检查解释器、虚拟环境及 Django 分发位置",
            Self::Threads => "线程状态、监视器锁等待关系与循环线索",
            Self::Dependencies => "依赖来源路径、版本选择与多版本提示",
            Self::Migrations => "已应用状态、显式依赖与 SQL 风险提示",
            Self::Sql => "按请求归一化 SQL，聚合耗时和重复查询",
            Self::Gc => "统一 GC 日志停顿分布、堆变化与时间线",
            Self::Jfr => "读取 JFR JSON 或用所选 JDK 转换小型录制",
            Self::SpringConfig => "比较两份 JSON/YAML 配置，遮盖敏感键值",
            Self::Actuator => "显式 GET health/info/metrics，保留 HTTP 状态",
            Self::Urls => "导出 URL 清单的命名冲突与 reverse 参数检查",
            Self::Drf => "OpenAPI 3.x 端点、字段及共享组件变化",
            Self::Checks => "解释 check --deploy 编号与部署检查输出",
            Self::Celery => "按任务 ID 关联失败、重试与已观察状态",
        }
    }
    pub fn help(self) -> &'static str {
        match self {
            Self::JavaEnvironment => {
                "仅运行所选可信 java 的固定版本检查；不会执行 Maven/Gradle wrapper，不修改 PATH。可选第二个 JDK 对比。"
            }
            Self::PythonEnvironment => {
                "仅运行所选可信 python -I；不加载项目 settings，不安装依赖。解释器自身 site/.pth 启动钩子仍可能执行。"
            }
            Self::Threads => {
                "粘贴 jstack / jcmd Thread.print 文本。只支持平台线程；循环等待线索与 JVM 明确死锁报告分别显示。"
            }
            Self::Dependencies => {
                "Maven dependency:tree 或 Gradle dependencies/insight 文本；insight 仅提取坐标，反向树不重建。多 scope/version 不等于运行时冲突。"
            }
            Self::Migrations => {
                "建议 showmigrations --plan --verbosity 2；下方可粘贴 sqlmigrate SQL。只使用明确依赖，不执行迁移。"
            }
            Self::Sql => {
                "JSON 数组：requestId（可选）、sql、durationMs（毫秒）。同请求至少 3 次 SELECT 为疑似 N+1，不证明根因。"
            }
            Self::Gc => {
                "支持 JDK 11–21 G1/Parallel/Serial 统一日志 Pause 完成行；不解释所有收集器，不包含并发阶段 CPU。"
            }
            Self::Jfr => {
                "可粘贴 jfr print --json（8 MiB / 20000 事件），或选择可信 jfr.exe 与 ≤64 MiB 录制。转换仅取采样/GC/锁/park 事件，30 秒超时。"
            }
            Self::SpringConfig => {
                "左右分别粘贴 JSON 或单文档 YAML，输入先脱敏；不模拟实际配置覆盖顺序，不直接读取 properties 文件。"
            }
            Self::Actuator => {
                "填写 Actuator 基础地址，例如 https://host/actuator。只允许所选端点的 GET；令牌不落盘，不枚举 env/configprops。"
            }
            Self::Urls => {
                "数组字段 route/name/namespace，正则路由显式提供 parameters 字符串数组。可选查询 {name,kwargs}；只检查参数名，不实际 reverse。"
            }
            Self::Drf => {
                "左右粘贴 OpenAPI 3.x JSON；共享 components 单独报告，不解析外部/循环 $ref。兼容性为保守提示，不执行接口。"
            }
            Self::Checks => {
                "粘贴 Django check --deploy 的脱敏输出，按编号分类；不会运行项目或改变设置。"
            }
            Self::Celery => {
                "粘贴按时间排序的标准 Task name[id] 日志；统计观测到的失败和重试，不连接 broker、不重放任务。"
            }
        }
    }
    pub fn sample(self) -> &'static str {
        match self {
            Self::Threads => {
                "\"worker-1\" #12 tid=0x1 nid=0x11\n   java.lang.Thread.State: BLOCKED (on object monitor)\n\tat demo.Task.run(Task.java:8)\n\t- waiting to lock <0x02>\n\t- locked <0x01>\n\"worker-2\" #13 tid=0x2 nid=0x12\n   java.lang.Thread.State: BLOCKED\n\tat demo.Task.run(Task.java:9)\n\t- waiting to lock <0x01>\n\t- locked <0x02>"
            }
            Self::Dependencies => {
                "[INFO] demo:app:jar:1.0\n[INFO] +- org.example:client:jar:2.0:compile\n[INFO] |  \\- org.example:common:jar:1.0:compile\n[INFO] \\- org.example:common:jar:2.0:compile"
            }
            Self::Migrations => {
                "[X]  shop.0001_initial\n[ ]  shop.0002_order ... (shop.0001_initial)\n[ ]  shop.0003_index ... (shop.0002_order)"
            }
            Self::Sql => {
                r#"[{"requestId":"demo","sql":"SELECT name FROM users WHERE id=1","durationMs":2.5},{"requestId":"demo","sql":"SELECT name FROM users WHERE id=2","durationMs":3},{"requestId":"demo","sql":"SELECT name FROM users WHERE id=3","durationMs":4}]"#
            }
            Self::Gc => {
                "[0.002s][info][gc] Using G1\n[1.020s][info][gc] GC(0) Pause Young (Normal) (G1 Evacuation Pause) 24M->3M(256M) 4.500ms\n[3.540s][info][gc] GC(1) Pause Young (Normal) (G1 Evacuation Pause) 42M->7M(256M) 8.000ms"
            }
            Self::Jfr => {
                r#"{"recording":{"events":[{"type":"jdk.ExecutionSample","values":{"startTime":"2026-09-26T10:00:00Z","stackTrace":{"frames":[{"method":{"type":{"name":"demo/Worker"},"name":"run"}}]}}},{"type":"jdk.GarbageCollection","values":{"startTime":"2026-09-26T10:00:01Z","duration":"PT0.004S"}}]}}"#
            }
            Self::SpringConfig => {
                "spring:\n  profiles:\n    active: dev\n  datasource:\n    password: example-only\nserver:\n  port: 8080"
            }
            Self::Urls => {
                r#"[{"route":"users/<int:pk>/","name":"detail","namespace":"api"},{"route":"accounts/<int:pk>/","name":"detail","namespace":"api"}]"#
            }
            Self::Drf => {
                r#"{"openapi":"3.0.3","paths":{"/users/":{"get":{"responses":{"200":{"description":"OK"}}}}},"components":{"schemas":{"User":{"type":"object","properties":{"name":{"type":"string"}}}}}}"#
            }
            Self::Checks => {
                "WARNINGS:\n?: (security.W018) You should not have DEBUG set to True in deployment.\n?: (security.W020) ALLOWED_HOSTS must not be empty in deployment."
            }
            Self::Celery => {
                "[2026-09-26 10:00:00: INFO/MainProcess] Task demo.send[example-task] received\n[2026-09-26 10:00:01: INFO/ForkPoolWorker-1] Task demo.send[example-task] retry: Retry in 1s\n[2026-09-26 10:00:02: INFO/ForkPoolWorker-1] Task demo.send[example-task] succeeded in 0.25s: None"
            }
            _ => "",
        }
    }
    pub fn second_sample(self) -> &'static str {
        match self {
            Self::Migrations => {
                "ALTER TABLE shop_order ADD COLUMN note text;\nDROP TABLE old_orders;"
            }
            Self::SpringConfig => {
                "spring:\n  profiles:\n    active: production\n  datasource:\n    password: ${DB_PASSWORD}\nserver:\n  port: 8081"
            }
            Self::Urls => r#"{"name":"api:detail","kwargs":{"id":42}}"#,
            Self::Drf => {
                r#"{"openapi":"3.0.3","paths":{"/users/":{"post":{"responses":{"201":{"description":"Created"}}}}},"components":{"schemas":{"User":{"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}}}}"#
            }
            _ => "",
        }
    }
    fn second_label(self) -> Option<&'static str> {
        match self {
            Self::Migrations => Some("SQL 输出（可选）"),
            Self::Urls => Some("reverse 参数检查（可选 JSON）"),
            Self::SpringConfig | Self::Drf => Some("右侧 / 新版本"),
            _ => None,
        }
    }
    fn environment(self) -> bool {
        matches!(self, Self::JavaEnvironment | Self::PythonEnvironment)
    }
}
pub fn analyze(tool: Tool, input: &str, second: &str) -> Result<String> {
    match tool {
        Tool::Threads => java::threads(input),
        Tool::Dependencies => java::dependencies(input),
        Tool::Migrations => django::migrations(input, second),
        Tool::Sql => django::sql(input),
        Tool::Gc => java::gc(input),
        Tool::Jfr => java::jfr(input),
        Tool::SpringConfig => contracts::config(input, second),
        Tool::Urls => django::urls(input, second),
        Tool::Drf => contracts::openapi(input, second),
        Tool::Checks => django::checks(input),
        Tool::Celery => django::celery(input),
        _ => anyhow::bail!("该工具需要明确选择执行目标"),
    }
}

#[derive(Clone, Default)]
struct Draft {
    input: String,
    second: String,
    output: String,
    summary: String,
    executable: String,
    comparison: String,
    file: String,
    export: String,
    base: String,
    endpoint: String,
}
#[derive(Default)]
pub struct State {
    pub selected: Tool,
    #[cfg(feature = "ui-preview")]
    pub preview_completed: usize,
    drafts: HashMap<Tool, Draft>,
    running: Option<(Tool, mpsc::Receiver<Result<String, String>>)>,
    token: String,
    message: String,
    copied: Option<Instant>,
}
impl State {
    pub(crate) fn background_active(&self) -> bool {
        self.running.is_some()
    }

    pub fn result_text(&self) -> &str {
        self.drafts
            .get(&self.selected)
            .map_or("", |draft| draft.output.as_str())
    }

    pub fn import_text(&mut self, tool: Tool, text: String) -> Result<()> {
        ensure!(
            !self
                .running
                .as_ref()
                .is_some_and(|(active, _)| *active == tool),
            "该诊断正在运行，请稍后重试"
        );
        self.select(tool);
        let draft = self.drafts.entry(tool).or_default();
        draft.input = text;
        draft.output.clear();
        draft.summary.clear();
        Ok(())
    }

    pub fn select(&mut self, tool: Tool) {
        if self.selected != tool {
            self.token.clear();
            self.message.clear();
        }
        self.selected = tool;
    }
    pub fn clear_token(&mut self) {
        self.token.clear();
    }
    pub fn poll(&mut self) -> Option<String> {
        let (tool, rx) = self.running.as_ref()?;
        let result = match rx.try_recv() {
            Ok(v) => v,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => Err("执行线程意外结束".into()),
        };
        let message = format!(
            "{}：{}",
            tool.label(),
            if result.is_ok() {
                "分析完成"
            } else {
                "执行失败，请查看结果"
            }
        );
        let draft = self.drafts.entry(*tool).or_default();
        draft.output = result.unwrap_or_else(|e| format!("失败：{e}"));
        draft.summary = summary(*tool, &draft.output);
        #[cfg(feature = "ui-preview")]
        {
            self.preview_completed += 1;
        }
        self.running = None;
        Some(message)
    }
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview(&mut self, tool: Tool) {
        self.select(tool);
        let d = self.drafts.entry(tool).or_default();
        d.input = tool.sample().into();
        d.second = tool.second_sample().into();
        d.output = analyze(tool, &d.input, &d.second)
            .unwrap_or_else(|_| "选择可信运行环境后执行；此截图未运行外部程序。".into());
        d.summary = summary(tool, &d.output);
        if tool == Tool::Actuator {
            d.base = "http://127.0.0.1:8080/actuator".into();
            d.endpoint = "health".into();
        }
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, shortcuts: bool, category: &str) {
        ui.heading(egui::RichText::new(format!("{category}诊断工作台")).size(27.0));
        ui.label("环境、性能与项目诊断 · 每个工具独立保留本次会话草稿");
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            for tool in Tool::ALL.into_iter().filter(|t| t.category() == category) {
                if ui
                    .selectable_label(self.selected == tool, tool.label())
                    .clicked()
                {
                    self.select(tool);
                }
            }
        });
        ui.separator();
        let tool = self.selected;
        let busy = self.running.is_some();
        ui.heading(tool.label());
        ui.label(tool.description());
        ui.small(tool.help());
        ui.add_space(10.0);
        let d = self.drafts.entry(tool).or_insert_with(|| Draft {
            endpoint: "health".into(),
            base: "http://127.0.0.1:8080/actuator".into(),
            ..Default::default()
        });
        let mut run = false;
        let mut recording = false;
        let mut import = false;
        ui.add_enabled_ui(!busy, |ui| {
            if tool.environment() || tool == Tool::Jfr {
                ui.label(if tool == Tool::Jfr {
                    "可选：可信 jfr.exe 绝对路径（读取 .jfr 文件时使用）"
                } else {
                    "可信可执行文件绝对路径"
                });
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut d.executable)
                            .desired_width((ui.available_width() - 130.0).max(180.0)),
                    );
                    if ui.button("填入 PATH 入口").clicked() {
                        let names = if tool == Tool::PythonEnvironment {
                            &["python.exe", "python3", "python"][..]
                        } else if tool == Tool::Jfr {
                            &["jfr.exe", "jfr"][..]
                        } else {
                            &["java.exe", "java"][..]
                        };
                        d.executable = runtime::path_entry(names)
                            .map(|p| p.display().to_string())
                            .unwrap_or_default();
                    }
                });
                if tool.environment() {
                    ui.label("比较入口（可选，填写另一个 java / python 绝对路径）");
                    ui.add(
                        egui::TextEdit::singleline(&mut d.comparison).desired_width(f32::INFINITY),
                    );
                }
            }
            if tool == Tool::Actuator {
                ui.label("Actuator 基础地址");
                ui.add(egui::TextEdit::singleline(&mut d.base).desired_width(f32::INFINITY));
                ui.horizontal(|ui| {
                    ui.label("端点");
                    egui::ComboBox::from_id_salt("actuator-endpoint")
                        .selected_text(&d.endpoint)
                        .show_ui(ui, |ui| {
                            for s in ["health", "info", "metrics"] {
                                ui.selectable_value(&mut d.endpoint, s.into(), s);
                            }
                        });
                    ui.add(
                        egui::TextEdit::singleline(&mut d.endpoint)
                            .hint_text("或 metrics/jvm.memory.used"),
                    );
                });
                ui.label("Bearer 令牌（可选，仅内存，切换工具/页面清空）");
                ui.add(
                    egui::TextEdit::singleline(&mut self.token)
                        .password(true)
                        .desired_width(f32::INFINITY),
                );
                ui.small(format!(
                    "即将 GET {}/{}",
                    d.base.trim_end_matches('/'),
                    d.endpoint
                ));
            } else if !tool.environment() {
                ui.horizontal(|ui| {
                    if ui.button("填入示例").clicked() {
                        d.input = tool.sample().into();
                        d.second = tool.second_sample().into();
                    }
                    if ui.button("清空输入").clicked() {
                        d.input.clear();
                        d.second.clear();
                    }
                });
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut d.file)
                            .hint_text(if tool == Tool::Jfr {
                                "文本 / JSON / .jfr 文件绝对路径"
                            } else {
                                "可选：UTF-8 文本文件绝对路径"
                            })
                            .desired_width((ui.available_width() - 230.0).max(160.0)),
                    );
                    import = ui.button("读取文本").clicked();
                    if tool == Tool::Jfr {
                        recording = ui.button("转换 .jfr 并分析").clicked();
                    }
                });
                ui.label(if tool.second_label().is_some() {
                    "左侧 / 输入"
                } else {
                    "输入"
                });
                ui.add(
                    egui::TextEdit::multiline(&mut d.input)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(9),
                );
                if let Some(label) = tool.second_label() {
                    ui.label(label);
                    ui.add(
                        egui::TextEdit::multiline(&mut d.second)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .desired_rows(5),
                    );
                }
            }
            run = ui
                .button(if tool.environment() {
                    "运行所选环境检查"
                } else if tool == Tool::Actuator {
                    "发送只读 GET"
                } else {
                    "分析输入"
                })
                .clicked()
                || (shortcuts && ui.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Enter)));
        });
        if import {
            let result = (|| -> Result<String> {
                let file = std::fs::File::open(&d.file).context("无法读取文本文件")?;
                let limit = if tool == Tool::Jfr {
                    8 * 1024 * 1024
                } else {
                    2 * 1024 * 1024
                };
                let mut bytes = vec![];
                file.take(limit + 1).read_to_end(&mut bytes)?;
                ensure!(bytes.len() as u64 <= limit, "文件过大");
                String::from_utf8(bytes).context("仅支持 UTF-8 文本")
            })();
            match result {
                Ok(text) => {
                    d.input = text;
                    self.message = "已载入文本，点击分析输入开始".into();
                }
                Err(e) => self.message = e.to_string(),
            }
        }
        if run || recording {
            let d = d.clone();
            let token = self.token.clone();
            let (tx, rx) = mpsc::channel();
            self.running = Some((tool, rx));
            std::thread::spawn(move || {
                let result = if recording {
                    runtime::recording(&d.executable, &d.file)
                } else {
                    match tool {
                        Tool::JavaEnvironment => {
                            runtime::java_environment(&d.executable, &d.comparison)
                        }
                        Tool::PythonEnvironment => {
                            runtime::python_environment(&d.executable, &d.comparison)
                        }
                        Tool::Actuator => runtime::actuator(&d.base, &d.endpoint, &token),
                        _ => analyze(tool, &d.input, &d.second),
                    }
                };
                let _ = tx.send(result.map_err(|e| format!("{e:#}")));
            });
        }
        if busy || run || recording {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("正在处理，可切换页面；结果会保留在原工具中。");
            });
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.strong("上次分析结果");
            if ui
                .add_enabled(
                    !d.output.is_empty(),
                    egui::Button::new(
                        if self
                            .copied
                            .is_some_and(|t| t.elapsed() < Duration::from_secs(2))
                        {
                            "已复制"
                        } else {
                            "复制报告"
                        },
                    ),
                )
                .clicked()
            {
                ui.ctx().copy_text(d.output.clone());
                self.copied = Some(Instant::now());
                ui.ctx().request_repaint_after(Duration::from_secs(2));
            }
        });
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut d.export)
                    .hint_text("可选：新报告文件绝对路径，不覆盖已有文件")
                    .desired_width((ui.available_width() - 110.0).max(160.0)),
            );
            if ui
                .add_enabled(!d.output.is_empty(), egui::Button::new("保存报告"))
                .clicked()
            {
                let result = (|| -> Result<()> {
                    ensure!(
                        std::path::Path::new(&d.export).is_absolute(),
                        "请填写绝对路径"
                    );
                    std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&d.export)?
                        .write_all(d.output.as_bytes())?;
                    Ok(())
                })();
                self.message = result
                    .map(|_| "报告已保存".into())
                    .unwrap_or_else(|e| e.to_string());
            }
        });
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        if !d.summary.is_empty() {
            egui::Frame::group(ui.style())
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.strong(&d.summary);
                });
        }
        let mut output = d.output.as_str();
        ui.add(
            egui::TextEdit::multiline(&mut output)
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY)
                .desired_rows(18),
        );
    }
}

fn summary(tool: Tool, text: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return String::new();
    };
    let count = |key: &str| v[key].as_array().map_or(0, Vec::len);
    match tool {
        Tool::Threads => format!(
            "{} 个线程 · {} 条等待关系 · {} 个循环相关线程 · JVM 明确死锁报告：{}",
            v["threadCount"],
            count("waitEdges"),
            count("cycleThreadIndices"),
            if v["explicitJvmDeadlockReport"] == true {
                "有"
            } else {
                "无"
            }
        ),
        Tool::Dependencies => format!(
            "{} 条依赖 · {} 组多版本选择",
            count("entries"),
            count("multipleSelectedVersions")
        ),
        Tool::Migrations => format!(
            "{} 项迁移 · {} 条显式依赖 · {} 项 SQL 风险提示",
            count("migrations"),
            count("dependencies"),
            count("sqlRisks")
        ),
        Tool::Sql => format!(
            "{} 次查询 · {} 组指纹 · 累计 {} ms",
            v["queryCount"],
            count("groups"),
            v["totalMs"]
        ),
        Tool::Gc => format!(
            "{} 次暂停 · 最大 {} ms · P95 {} ms",
            v["pauseCount"], v["maxPauseMs"], v["p95PauseMs"]
        ),
        Tool::Jfr => format!(
            "{} 项事件 · {} 个采样热点（非 CPU 百分比）",
            v["eventCount"],
            count("sampleTopFrames")
        ),
        Tool::SpringConfig => format!(
            "{} 项配置差异 · {} 个未解析占位符路径",
            count("changes"),
            count("unresolvedPlaceholderPaths")
        ),
        Tool::Urls => format!(
            "{} 条路由 · {} 组同名路由",
            v["routeCount"],
            count("duplicateNames")
        ),
        Tool::Drf => format!(
            "{} 项端点变化 · {} 项共享组件差异",
            count("operationChanges"),
            count("componentChanges")
        ),
        Tool::Checks => format!(
            "{} 项检查 · 原文明示无问题：{}",
            count("items"),
            v["explicitNoIssues"]
        ),
        Tool::Celery => format!(
            "{} 个任务 · {} 行未解析",
            count("tasks"),
            v["unparsedLines"]
        ),
        Tool::JavaEnvironment => format!(
            "JDK {} · JAVA_HOME 不一致：{}",
            v["selected"]["properties"]["java.version"]
                .as_str()
                .unwrap_or("未知"),
            v["javaHomeMismatch"]
        ),
        Tool::PythonEnvironment => format!(
            "Python {} · Django {} · 虚拟环境：{}",
            v["selected"]["version"].as_str().unwrap_or("未知"),
            v["selected"]["django"]["version"]
                .as_str()
                .unwrap_or("未安装"),
            v["selected"]["virtualEnvironment"]
        ),
        Tool::Actuator => format!(
            "HTTP {} · {}",
            v["httpStatus"],
            v["url"].as_str().unwrap_or("")
        ),
    }
}
