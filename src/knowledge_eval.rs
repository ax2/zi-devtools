//! Repeatable, explicitly triggered evaluation of local retrieval and cited answers.
use anyhow::{Context, Result, ensure};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::Instant,
};

use crate::{
    knowledge_answer::{self, Protocol},
    knowledge_sources::Source,
};

const MAX_SUITE_BYTES: usize = 32 * 1024;
const MAX_CASES: usize = 10;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    pub question: String,
    pub terms: String,
    pub expected_source: String,
    pub expected_relative: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_answer_contains: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Suite {
    pub version: u32,
    pub cases: Vec<Case>,
}

fn valid_text(value: &str, max_chars: usize, max_bytes: usize) -> bool {
    !value.trim().is_empty()
        && value.chars().count() <= max_chars
        && value.len() <= max_bytes
        && !value.chars().any(char::is_control)
}

pub fn parse_suite(input: &str, sources: &[Source]) -> Result<Suite> {
    ensure!(input.len() <= MAX_SUITE_BYTES, "评测集超过 32 KiB");
    let suite: Suite = serde_json::from_str(input).context("评测集 JSON 格式无效")?;
    ensure!(suite.version == 1, "只支持评测集版本 1");
    ensure!(
        (1..=MAX_CASES).contains(&suite.cases.len()),
        "评测集需包含 1–10 个样例"
    );
    let mut ids = HashSet::new();
    for case in &suite.cases {
        ensure!(
            valid_text(&case.id, 64, 128),
            "样例 ID 为空、过长或含控制字符"
        );
        ensure!(ids.insert(&case.id), "样例 ID 重复：{}", case.id);
        ensure!(
            valid_text(&case.question, 1000, 4000),
            "{}：问题为空、过长或含控制字符",
            case.id
        );
        ensure!(
            valid_text(&case.terms, 120, 512),
            "{}：检索词为空、过长或含控制字符",
            case.id
        );
        ensure!(
            valid_text(&case.expected_source, 80, 320)
                && valid_text(&case.expected_relative, 512, 2048),
            "{}：预期来源名称或相对路径无效",
            case.id
        );
        if let Some(expected) = &case.expected_answer_contains {
            ensure!(
                valid_text(expected, 200, 800),
                "{}：预期答案片段为空、过长或含控制字符",
                case.id
            );
        }
        let matches: Vec<_> = sources
            .iter()
            .filter(|source| source.name == case.expected_source)
            .collect();
        ensure!(
            matches.len() == 1,
            "{}：预期知识源必须唯一且已添加：{}",
            case.id,
            case.expected_source
        );
        ensure!(
            matches[0].snapshot.as_ref().is_some_and(|snapshot| snapshot
                .files
                .iter()
                .any(|file| file.relative == case.expected_relative)),
            "{}：预期文件不在当前扫描快照中：{}",
            case.id,
            case.expected_relative
        );
    }
    Ok(suite)
}

#[derive(Clone, Debug, Serialize)]
pub struct EvidenceRecord {
    pub id: usize,
    pub source: String,
    pub relative: String,
    pub location: String,
    pub file_sha256: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CaseResult {
    pub id: String,
    pub question: String,
    pub terms: String,
    pub expected_source: String,
    pub expected_relative: String,
    pub expected_answer_contains: Option<String>,
    pub evidence_count: usize,
    pub evidence: Vec<EvidenceRecord>,
    pub expected_rank: Option<usize>,
    pub cited_ids: Vec<usize>,
    pub expected_cited: Option<bool>,
    pub answer_contains_expected: Option<bool>,
    pub answer: Option<String>,
    pub error: Option<String>,
    pub elapsed_ms: u128,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub schema: &'static str,
    pub version: u32,
    pub created_at: String,
    pub mode: &'static str,
    pub model: Option<String>,
    pub top_k: usize,
    pub total: usize,
    pub completed: usize,
    pub retrieval_hits: usize,
    pub cited_expected: usize,
    pub literal_expected_cases: usize,
    pub literal_answer_matches: usize,
    pub cancelled: bool,
    pub results: Vec<CaseResult>,
}

#[derive(Clone, Debug)]
pub struct ModelConfig {
    pub endpoint: String,
    pub protocol: Protocol,
    pub model: String,
}

pub fn run(
    index_path: &Path,
    suite: &Suite,
    sources: &[Source],
    top_k: usize,
    model: Option<&ModelConfig>,
    cancel: &AtomicBool,
    mut progress: impl FnMut(usize, usize, &str),
) -> Result<Report> {
    ensure!([3, 5, 8].contains(&top_k), "Top K 只能为 3、5 或 8");
    ensure!(
        (1..=MAX_CASES).contains(&suite.cases.len()),
        "评测集大小无效"
    );
    if let Some(config) = model {
        knowledge_answer::loopback_endpoint(&config.endpoint, config.protocol)?;
        ensure!(!config.model.trim().is_empty(), "请填写本机模型名称");
    }
    let mut report = Report {
        schema: "zi-devtools-rag-eval",
        version: 1,
        created_at: chrono::Utc::now().to_rfc3339(),
        mode: if model.is_some() {
            "local-model"
        } else {
            "retrieval-only"
        },
        model: model.map(|config| config.model.trim().to_owned()),
        top_k,
        total: suite.cases.len(),
        completed: 0,
        retrieval_hits: 0,
        cited_expected: 0,
        literal_expected_cases: 0,
        literal_answer_matches: 0,
        cancelled: false,
        results: Vec::new(),
    };
    for case in &suite.cases {
        if cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            break;
        }
        progress(report.completed + 1, report.total, &case.id);
        let started = Instant::now();
        let mut row = CaseResult {
            id: case.id.clone(),
            question: case.question.clone(),
            terms: case.terms.clone(),
            expected_source: case.expected_source.clone(),
            expected_relative: case.expected_relative.clone(),
            expected_answer_contains: case.expected_answer_contains.clone(),
            evidence_count: 0,
            evidence: Vec::new(),
            expected_rank: None,
            cited_ids: Vec::new(),
            expected_cited: None,
            answer_contains_expected: None,
            answer: None,
            error: None,
            elapsed_ms: 0,
        };
        match knowledge_answer::prepare(
            index_path,
            sources,
            &case.question,
            &case.terms,
            None,
            top_k,
        ) {
            Ok(prepared) => {
                row.evidence_count = prepared.evidence.len();
                row.evidence = prepared
                    .evidence
                    .iter()
                    .map(|evidence| EvidenceRecord {
                        id: evidence.id,
                        source: evidence.source_name.clone(),
                        relative: evidence.hit.relative.clone(),
                        location: evidence.hit.location.clone(),
                        file_sha256: evidence.hit.file_sha256.clone(),
                    })
                    .collect();
                row.expected_rank = prepared.evidence.iter().find_map(|evidence| {
                    (evidence.source_name == case.expected_source
                        && evidence.hit.relative == case.expected_relative)
                        .then_some(evidence.id)
                });
                if row.expected_rank.is_some() {
                    report.retrieval_hits += 1;
                }
                if let Some(config) = model {
                    if prepared.evidence.is_empty() {
                        row.error = Some("没有可核验的证据；未调用模型".into());
                    } else {
                        match knowledge_answer::generate(
                            &prepared,
                            sources,
                            &config.endpoint,
                            config.protocol,
                            &config.model,
                            cancel,
                        ) {
                            Ok(answer) => {
                                row.cited_ids = answer.citations.clone();
                                let cited = prepared.evidence.iter().any(|evidence| {
                                    evidence.source_name == case.expected_source
                                        && evidence.hit.relative == case.expected_relative
                                        && answer.citations.contains(&evidence.id)
                                });
                                row.expected_cited = Some(cited);
                                if cited {
                                    report.cited_expected += 1;
                                }
                                if let Some(expected) = &case.expected_answer_contains {
                                    let matched =
                                        !answer.insufficient && answer.text.contains(expected);
                                    row.answer_contains_expected = Some(matched);
                                    if matched {
                                        report.literal_answer_matches += 1;
                                    }
                                }
                                row.answer = Some(answer.text);
                            }
                            Err(error) => row.error = Some(format!("{error:#}")),
                        }
                    }
                }
            }
            Err(error) => row.error = Some(format!("{error:#}")),
        }
        row.elapsed_ms = started.elapsed().as_millis();
        if case.expected_answer_contains.is_some() {
            report.literal_expected_cases += 1;
        }
        report.completed += 1;
        report.results.push(row);
        if cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            break;
        }
    }
    Ok(report)
}

pub fn save_report(path: &Path, report: &Report) -> Result<()> {
    ensure!(
        path.extension().and_then(|s| s.to_str()) == Some("json"),
        "报告需保存为 .json 文件"
    );
    let bytes = serde_json::to_vec_pretty(report)?;
    ensure!(bytes.len() <= 64 * 1024, "报告超过 64 KiB，不予保存");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .context("无法新建报告；已有同名文件不会覆盖")?;
    file.write_all(&bytes).context("写入评测报告失败")?;
    file.sync_all().context("同步评测报告失败")?;
    Ok(())
}

enum Event {
    Progress(usize, usize, String),
    Finished(Result<Report, String>),
}

pub struct State {
    index_path: PathBuf,
    suite_input: String,
    suite: Option<Suite>,
    top_k: usize,
    protocol: Protocol,
    endpoint: String,
    model: String,
    running: Option<Receiver<Event>>,
    cancel: Arc<AtomicBool>,
    report: Option<Report>,
    export_path: String,
    message: String,
}

impl State {
    pub fn new(index_path: PathBuf) -> Self {
        Self {
            index_path,
            suite_input: String::new(),
            suite: None,
            top_k: 5,
            protocol: Protocol::Ollama,
            endpoint: Protocol::Ollama.default_endpoint().into(),
            model: String::new(),
            running: None,
            cancel: Arc::new(AtomicBool::new(false)),
            report: None,
            export_path: String::new(),
            message: String::new(),
        }
    }

    fn start(&mut self, sources: &[Source], with_model: bool) {
        if self.running.is_some() {
            return;
        }
        let Some(suite) = self.suite.clone() else {
            self.message = "请先校验评测集".into();
            return;
        };
        let config = with_model.then(|| ModelConfig {
            endpoint: self.endpoint.clone(),
            protocol: self.protocol,
            model: self.model.clone(),
        });
        if let Some(config) = &config {
            if let Err(error) =
                knowledge_answer::loopback_endpoint(&config.endpoint, config.protocol)
            {
                self.message = format!("本机模型地址无效：{error:#}");
                return;
            }
            if config.model.trim().is_empty() {
                self.message = "请填写本机模型名称".into();
                return;
            }
        }
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let index_path = self.index_path.clone();
        let sources = sources.to_vec();
        let top_k = self.top_k;
        let (tx, rx) = mpsc::channel();
        self.running = Some(rx);
        self.report = None;
        self.message = "正在逐题评测…".into();
        std::thread::spawn(move || {
            let result = run(
                &index_path,
                &suite,
                &sources,
                top_k,
                config.as_ref(),
                &cancel,
                |done, total, id| {
                    let _ = tx.send(Event::Progress(done, total, id.to_owned()));
                },
            );
            let _ = tx.send(Event::Finished(
                result.map_err(|error| format!("{error:#}")),
            ));
        });
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self, sources: &[Source]) {
        let source = sources
            .first()
            .map_or("示例资料", |source| source.name.as_str());
        self.suite_input = format!(
            r#"{{"version":1,"cases":[{{"id":"case-1","question":"知识库如何核对引用？","terms":"知识库","expected_source":"{source}","expected_relative":"notes/knowledge.md","expected_answer_contains":"文件版本摘要"}}]}}"#
        );
        self.suite = serde_json::from_str(&self.suite_input).ok();
        self.model = "qwen2.5:7b".into();
        self.report = Some(Report {
            schema: "zi-devtools-rag-eval",
            version: 1,
            created_at: "2026-09-30T00:00:00Z".into(),
            mode: "local-model",
            model: Some(self.model.clone()),
            top_k: 5,
            total: 1,
            completed: 1,
            retrieval_hits: 1,
            cited_expected: 1,
            literal_expected_cases: 1,
            literal_answer_matches: 1,
            cancelled: false,
            results: vec![CaseResult {
                id: "case-1".into(),
                question: "知识库如何核对引用？".into(),
                terms: "知识库".into(),
                expected_source: source.into(),
                expected_relative: "notes/knowledge.md".into(),
                expected_answer_contains: Some("文件版本摘要".into()),
                evidence_count: 3,
                evidence: vec![EvidenceRecord {
                    id: 1,
                    source: source.into(),
                    relative: "notes/knowledge.md".into(),
                    location: "第 2 段".into(),
                    file_sha256: "a".repeat(64),
                }],
                expected_rank: Some(1),
                cited_ids: vec![1],
                expected_cited: Some(true),
                answer_contains_expected: Some(true),
                answer: Some("结果保留文件版本摘要，便于回到原文核对。".into()),
                error: None,
                elapsed_ms: 842,
            }],
        });
        self.message = "合成示例报告；未发送真实文档到模型。".into();
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, sources: &[Source]) {
        let mut finished = None;
        if let Some(rx) = &self.running {
            loop {
                match rx.try_recv() {
                    Ok(Event::Progress(done, total, id)) => {
                        self.message = format!("正在评测 {done}/{total}：{id}")
                    }
                    Ok(Event::Finished(result)) => {
                        finished = Some(result);
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        finished = Some(Err("评测任务意外结束".into()));
                        break;
                    }
                }
            }
        }
        if let Some(result) = finished {
            self.running = None;
            match result {
                Ok(report) => {
                    self.message = format!(
                        "已完成 {}/{} 题；预期来源进入 Top K：{} 题{}。",
                        report.completed,
                        report.total,
                        report.retrieval_hits,
                        if report.cancelled {
                            "（已取消）"
                        } else {
                            ""
                        }
                    );
                    self.report = Some(report);
                }
                Err(error) => self.message = error,
            }
        }
        if self.running.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        ui.heading("本机 RAG 评测");
        ui.label("用固定样例检查预期文件是否进入证据 Top K；可选明确调用本机模型，检查有效引用与答案字面片段。指标只用于回归，不证明语义正确。");
        ui.add_space(8.0);
        ui.strong("评测集 JSON · 最多 10 题");
        let changed = ui.add_enabled(
            self.running.is_none(),
            egui::TextEdit::multiline(&mut self.suite_input)
                .desired_rows(8)
                .desired_width(ui.available_width().min(920.0))
                .hint_text(r#"{"version":1,"cases":[{"id":"case-1","question":"...","terms":"...","expected_source":"来源名","expected_relative":"notes/a.md","expected_answer_contains":"可选文本"}]}"#),
        ).changed();
        if changed {
            self.suite = None;
            self.report = None;
            self.message = "评测集已修改，请重新校验。".into();
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.running.is_none(), egui::Button::new("校验评测集"))
                .clicked()
            {
                match parse_suite(&self.suite_input, sources) {
                    Ok(suite) => {
                        self.message = format!(
                            "已校验 {} 题；预期文件均在当前知识源快照中。",
                            suite.cases.len()
                        );
                        self.suite = Some(suite);
                        self.report = None;
                    }
                    Err(error) => {
                        self.suite = None;
                        self.message = format!("校验失败：{error:#}");
                    }
                }
            }
            egui::ComboBox::from_id_salt("rag-eval-top-k")
                .selected_text(format!("Top {}", self.top_k))
                .show_ui(ui, |ui| {
                    for count in [3, 5, 8] {
                        if ui
                            .add_enabled(
                                self.running.is_none(),
                                egui::Button::new(format!("Top {count}")),
                            )
                            .clicked()
                        {
                            self.top_k = count;
                            self.report = None;
                        }
                    }
                });
        });
        if let Some(suite) = &self.suite {
            for case in &suite.cases {
                ui.small(format!(
                    "{} · {} → {} / {}",
                    case.id, case.terms, case.expected_source, case.expected_relative
                ));
            }
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.suite.is_some() && self.running.is_none(),
                    egui::Button::new("运行检索评测（不联网）"),
                )
                .clicked()
            {
                self.start(sources, false);
            }
            if ui
                .add_enabled(
                    self.suite.is_some() && self.running.is_none(),
                    egui::Button::new("运行本机模型评测"),
                )
                .clicked()
            {
                self.start(sources, true);
            }
            if self.running.is_some() {
                ui.spinner();
                if ui.button("取消余下评测").clicked() {
                    self.cancel.store(true, Ordering::Relaxed);
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("本机模型");
            ui.add_enabled_ui(self.running.is_none(), |ui| {
                let old = self.protocol;
                egui::ComboBox::from_id_salt("rag-eval-protocol")
                    .selected_text(self.protocol.label())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.protocol,
                            Protocol::Ollama,
                            Protocol::Ollama.label(),
                        );
                        ui.selectable_value(
                            &mut self.protocol,
                            Protocol::OpenAi,
                            Protocol::OpenAi.label(),
                        );
                    });
                if self.protocol != old {
                    self.endpoint = self.protocol.default_endpoint().into();
                }
            });
        });
        ui.add_enabled(
            self.running.is_none(),
            egui::TextEdit::singleline(&mut self.endpoint)
                .desired_width(ui.available_width().min(700.0))
                .hint_text("本机回环接口"),
        );
        ui.add_enabled(
            self.running.is_none(),
            egui::TextEdit::singleline(&mut self.model)
                .desired_width(300.0)
                .hint_text("模型名称，例如 qwen2.5:7b"),
        );
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        if let Some(report) = &self.report {
            ui.separator();
            ui.strong(format!(
                "报告 · {}/{} 题 · 来源召回 {}/{}",
                report.completed, report.total, report.retrieval_hits, report.completed
            ));
            if report.model.is_some() {
                ui.label(format!(
                    "引用预期文件 {} 题；答案字面片段命中 {}/{} 题。只统计已完成题目。",
                    report.cited_expected,
                    report.literal_answer_matches,
                    report.literal_expected_cases
                ));
            }
            for row in &report.results {
                ui.group(|ui| {
                    ui.set_min_width(ui.available_width().min(880.0));
                    ui.strong(format!("{} · {} ms", row.id, row.elapsed_ms));
                    ui.label(format!(
                        "证据 {} 条；预期来源 {}",
                        row.evidence_count,
                        row.expected_rank
                            .map_or("未命中".into(), |rank| format!("第 {rank} 条"))
                    ));
                    if let Some(cited) = row.expected_cited {
                        ui.small(format!(
                            "模型引用预期文件：{}",
                            if cited { "是" } else { "否" }
                        ));
                    }
                    if let Some(matched) = row.answer_contains_expected {
                        ui.small(format!(
                            "答案包含预期文本：{}",
                            if matched { "是" } else { "否" }
                        ));
                    }
                    if let Some(answer) = &row.answer {
                        ui.label(answer);
                    }
                    if !row.cited_ids.is_empty() {
                        ui.small(format!(
                            "答案引用编号：{}",
                            row.cited_ids
                                .iter()
                                .map(usize::to_string)
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                    }
                    if let Some(error) = &row.error {
                        ui.colored_label(egui::Color32::LIGHT_RED, error);
                    }
                });
            }
            ui.add_space(6.0);
            ui.label("导出报告路径（仅新建 .json，不覆盖）");
            ui.add(
                egui::TextEdit::singleline(&mut self.export_path)
                    .desired_width(ui.available_width().min(920.0)),
            );
            if ui.button("保存报告").clicked() {
                self.message = match save_report(Path::new(self.export_path.trim()), report) {
                    Ok(()) => "报告已保存到指定路径。".into(),
                    Err(error) => format!("保存失败：{error:#}"),
                };
            }
        }
        ui.small("评测集、问答与报告默认仅在当前进程内存中；模型模式会把问题及已核验片段发送到你选择的本机模型服务。导出报告包含问题与答案，请选择合适目录。");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge_sources::{add_source, scan};
    use std::fs;

    #[test]
    fn validates_suite_and_runs_retrieval_without_model() {
        let root = std::env::temp_dir().join(format!("zi-eval-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("alpha.txt"),
            "银杉项目的准入码是 ZX-42。\n\n第二段资料。 ",
        )
        .unwrap();
        let mut sources = Vec::new();
        let mut source = add_source(&mut sources, &root, "Fixture", "").unwrap();
        source.snapshot = Some(scan(&source, &AtomicBool::new(false)).unwrap());
        let index = root.join("index.sqlite3");
        crate::knowledge_index::sync_all(
            &index,
            &[source.clone()],
            false,
            &AtomicBool::new(false),
            |_, _, _| {},
        )
        .unwrap();
        let json = r#"{"version":1,"cases":[{"id":"a","question":"准入码是什么？","terms":"准入码","expected_source":"Fixture","expected_relative":"alpha.txt","expected_answer_contains":"ZX-42"}]}"#;
        let suite = parse_suite(json, &[source.clone()]).unwrap();
        let duplicate = json.replace(
            "]}",
            ", {\"id\":\"a\",\"question\":\"另一问题\",\"terms\":\"准入码\",\"expected_source\":\"Fixture\",\"expected_relative\":\"alpha.txt\"}]}"
        );
        assert!(parse_suite(&duplicate, &[source.clone()]).is_err());
        let report = run(
            &index,
            &suite,
            &[source.clone()],
            3,
            None,
            &AtomicBool::new(false),
            |_, _, _| {},
        )
        .unwrap();
        assert_eq!(report.mode, "retrieval-only");
        assert_eq!(report.retrieval_hits, 1);
        assert_eq!(report.completed, 1);
        assert!(report.results[0].expected_rank.is_some());
        assert_eq!(report.results[0].expected_cited, None);
        assert!(parse_suite(json.replace("\"id\":\"a\"", "\"id\":\"b\"").as_str(), &[]).is_err());
        assert!(parse_suite(r#"{"version":1,"cases":[]}"#, &[source.clone()]).is_err());
        let path = root.join("report.json");
        save_report(&path, &report).unwrap();
        assert!(save_report(&path, &report).is_err());
        let cancelled = AtomicBool::new(true);
        let cancelled_report = run(
            &index,
            &suite,
            &[source.clone()],
            3,
            None,
            &cancelled,
            |_, _, _| {},
        )
        .unwrap();
        assert!(cancelled_report.cancelled);
        assert_eq!(cancelled_report.completed, 0);
        fs::write(root.join("alpha.txt"), "文件已经变更").unwrap();
        let config = ModelConfig {
            endpoint: "http://127.0.0.1:1/api/chat".into(),
            protocol: Protocol::Ollama,
            model: "fixture".into(),
        };
        let stale = run(
            &index,
            &suite,
            &[source.clone()],
            3,
            Some(&config),
            &AtomicBool::new(false),
            |_, _, _| {},
        )
        .unwrap();
        assert_eq!(stale.retrieval_hits, 0);
        assert_eq!(stale.results[0].evidence_count, 0);
        assert!(
            stale.results[0]
                .error
                .as_deref()
                .unwrap()
                .contains("未调用模型")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
