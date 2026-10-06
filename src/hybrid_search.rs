//! Explicit, explainable reciprocal-rank fusion over local keyword and semantic candidates.
use anyhow::{Result, ensure};
use eframe::egui;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::Duration,
};

use crate::{
    knowledge_index::{self, SearchHit},
    knowledge_sources::Source,
    vector_index::{self, SemanticHit},
};

const RRF_K: f64 = 60.0;
type Key = (String, String, u64);

fn key(hit: &SearchHit) -> Key {
    (hit.source_id.clone(), hit.relative.clone(), hit.ordinal)
}

#[derive(Clone, Debug)]
pub struct HybridHit {
    pub hit: SearchHit,
    pub keyword_rank: Option<usize>,
    pub semantic_rank: Option<usize>,
    pub cosine: Option<f64>,
    pub keyword_contribution: f64,
    pub semantic_contribution: f64,
    pub score: f64,
}

#[derive(Clone, Debug)]
pub struct HybridResults {
    pub hits: Vec<HybridHit>,
    pub keyword_candidates: usize,
    pub semantic_candidates: usize,
    pub stale_dropped: usize,
    pub literal_match: bool,
}

fn fuse(
    keyword: Vec<SearchHit>,
    semantic: Vec<SemanticHit>,
    keyword_weight: u8,
    limit: usize,
    literal_match: bool,
) -> Result<HybridResults> {
    ensure!((1..=50).contains(&limit), "结果上限必须在 1–50 条之间");
    ensure!(keyword_weight <= 100, "关键词权重必须在 0–100% 之间");
    ensure!(
        keyword.len() <= 50 && semantic.len() <= 50,
        "候选结果超过通道上限"
    );
    let keyword_candidates = keyword.len();
    let semantic_candidates = semantic.len();
    let mut merged = HashMap::<Key, HybridHit>::new();
    let mut conflict = HashSet::<Key>::new();
    for (position, hit) in keyword.into_iter().enumerate() {
        let id = key(&hit);
        merged.entry(id).or_insert(HybridHit {
            hit,
            keyword_rank: Some(position + 1),
            semantic_rank: None,
            cosine: None,
            keyword_contribution: 0.0,
            semantic_contribution: 0.0,
            score: 0.0,
        });
    }
    for (position, item) in semantic.into_iter().enumerate() {
        let id = key(&item.hit);
        if let Some(existing) = merged.get_mut(&id) {
            if existing.hit.file_sha256 != item.hit.file_sha256
                || existing.hit.chunk_sha256 != item.hit.chunk_sha256
            {
                conflict.insert(id);
                continue;
            }
            existing.semantic_rank = Some(position + 1);
            existing.cosine = Some(item.cosine);
        } else {
            merged.insert(
                id,
                HybridHit {
                    hit: item.hit,
                    keyword_rank: None,
                    semantic_rank: Some(position + 1),
                    cosine: Some(item.cosine),
                    keyword_contribution: 0.0,
                    semantic_contribution: 0.0,
                    score: 0.0,
                },
            );
        }
    }
    for id in &conflict {
        merged.remove(id);
    }
    let lexical_weight = keyword_weight as f64 / 100.0;
    let semantic_weight = 1.0 - lexical_weight;
    let mut hits = merged.into_values().collect::<Vec<_>>();
    for item in &mut hits {
        item.keyword_contribution = item
            .keyword_rank
            .map_or(0.0, |rank| lexical_weight / (RRF_K + rank as f64));
        item.semantic_contribution = item
            .semantic_rank
            .map_or(0.0, |rank| semantic_weight / (RRF_K + rank as f64));
        item.score = item.keyword_contribution + item.semantic_contribution;
    }
    hits.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| key(&a.hit).cmp(&key(&b.hit)))
    });
    Ok(HybridResults {
        hits,
        keyword_candidates,
        semantic_candidates,
        stale_dropped: conflict.len(),
        literal_match,
    })
}

pub struct SearchOptions<'a> {
    pub source_id: Option<&'a str>,
    pub limit: usize,
    pub keyword_weight: u8,
}

pub fn search(
    keyword_path: &Path,
    vector_path: &Path,
    endpoint: &str,
    model: &str,
    query: &str,
    options: SearchOptions<'_>,
    cancel: &AtomicBool,
) -> Result<HybridResults> {
    let SearchOptions {
        source_id,
        limit,
        keyword_weight,
    } = options;
    ensure!(!cancel.load(Ordering::Relaxed), "混合检索已取消");
    let lexical = knowledge_index::search(keyword_path, query, source_id, 50)?;
    ensure!(!cancel.load(Ordering::Relaxed), "混合检索已取消");
    let semantic = vector_index::search_filtered(
        vector_path,
        keyword_path,
        endpoint,
        model,
        query,
        vector_index::SearchScope {
            source_id,
            limit: 50,
        },
        cancel,
    )?;
    ensure!(!cancel.load(Ordering::Relaxed), "混合检索已取消");
    let mut result = fuse(
        lexical.hits,
        semantic,
        keyword_weight,
        limit,
        lexical.literal_match,
    )?;
    let candidates = result
        .hits
        .iter()
        .map(|item| item.hit.clone())
        .collect::<Vec<_>>();
    let (current, stale) = knowledge_index::filter_current_hits(keyword_path, &candidates)?;
    let valid = current
        .iter()
        .map(|hit| (key(hit), hit.file_sha256.clone(), hit.chunk_sha256.clone()))
        .collect::<HashSet<_>>();
    result.hits.retain(|item| {
        valid.contains(&(
            key(&item.hit),
            item.hit.file_sha256.clone(),
            item.hit.chunk_sha256.clone(),
        ))
    });
    result.stale_dropped += stale;
    result.hits.truncate(limit);
    ensure!(!cancel.load(Ordering::Relaxed), "混合检索已取消");
    Ok(result)
}

pub struct State {
    keyword_path: PathBuf,
    vector_path: PathBuf,
    endpoint: String,
    model: String,
    query: String,
    source_id: String,
    limit: usize,
    keyword_weight: u8,
    running: Option<Receiver<Result<HybridResults, String>>>,
    cancel: Arc<AtomicBool>,
    results: Option<HybridResults>,
    message: String,
}

impl Drop for State {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl State {
    pub(crate) fn background_active(&self) -> bool {
        self.running.is_some()
    }

    pub fn new(keyword_path: PathBuf, vector_path: PathBuf) -> Self {
        Self {
            keyword_path,
            vector_path,
            endpoint: "http://127.0.0.1:11434/api/embed".into(),
            model: String::new(),
            query: String::new(),
            source_id: String::new(),
            limit: 10,
            keyword_weight: 50,
            running: None,
            cancel: Arc::new(AtomicBool::new(false)),
            results: None,
            message: String::new(),
        }
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self, sources: &[Source]) {
        self.model = "bge-m3:latest".into();
        self.query = "Rust 如何避免数据竞争？".into();
        let hit = sources.first().map(|source| SearchHit {
            source_id: source.id.clone(),
            relative: "notes/rust.md".into(),
            location: "第 2 段".into(),
            ordinal: 2,
            file_sha256: "a".repeat(64),
            chunk_sha256: "b".repeat(64),
            text: "Rust 使用所有权和借用检查器，在编译期排除许多数据竞争。结果展示关键词和语义通道排名，并保留可复核的来源。".into(),
        });
        self.results = Some(HybridResults {
            hits: hit
                .into_iter()
                .map(|hit| HybridHit {
                    hit,
                    keyword_rank: Some(2),
                    semantic_rank: Some(1),
                    cosine: Some(0.873),
                    keyword_contribution: 0.5 / 62.0,
                    semantic_contribution: 0.5 / 61.0,
                    score: 0.5 / 62.0 + 0.5 / 61.0,
                })
                .collect(),
            keyword_candidates: 5,
            semantic_candidates: 8,
            stale_dropped: 0,
            literal_match: true,
        });
        self.message = "合成界面预览 · 双通道融合 · 未调用模型".into();
    }

    fn start(&mut self) {
        if self.running.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let (keyword_path, vector_path, endpoint, model, query, source, limit, weight) = (
            self.keyword_path.clone(),
            self.vector_path.clone(),
            self.endpoint.clone(),
            self.model.clone(),
            self.query.clone(),
            (!self.source_id.is_empty()).then(|| self.source_id.clone()),
            self.limit,
            self.keyword_weight,
        );
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Arc::clone(&cancel);
        self.running = Some(rx);
        self.results = None;
        self.message = "正在查询全文索引、生成问题向量并融合结果…".into();
        std::thread::spawn(move || {
            let result = search(
                &keyword_path,
                &vector_path,
                &endpoint,
                &model,
                &query,
                SearchOptions {
                    source_id: source.as_deref(),
                    limit,
                    keyword_weight: weight,
                },
                &cancel,
            )
            .map_err(|error| format!("{error:#}"));
            let _ = tx.send(result);
        });
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, sources: &[Source], sources_locked: bool) {
        if !self.source_id.is_empty() && !sources.iter().any(|source| source.id == self.source_id) {
            self.source_id.clear();
            self.results = None;
        }
        if let Some(rx) = &self.running {
            match rx.try_recv() {
                Ok(Ok(results)) => {
                    self.message = format!(
                        "关键词候选 {} 条 · 语义候选 {} 条 · 已剔除过期/冲突 {} 条。{}",
                        results.keyword_candidates,
                        results.semantic_candidates,
                        results.stale_dropped,
                        if results.literal_match {
                            "中文关键词按短语字面包含匹配。"
                        } else {
                            "英文关键词按 FTS5 相关度排序。"
                        }
                    );
                    self.results = Some(results);
                    self.running = None;
                }
                Ok(Err(error)) => {
                    self.message = error;
                    self.running = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.message = "混合检索任务意外结束".into();
                    self.running = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ui.ctx().request_repaint_after(Duration::from_millis(100));
                }
            }
        }
        ui.heading("混合检索工作台");
        ui.label("同时读取本机全文和向量索引，按可调权重融合排名；每条结果展示命中的通道与贡献。");
        ui.add_space(8.0);
        let busy = self.running.is_some();
        ui.horizontal(|ui| {
            ui.label("本机嵌入接口");
            if ui
                .add_enabled(
                    !busy,
                    egui::TextEdit::singleline(&mut self.endpoint).desired_width(460.0),
                )
                .changed()
            {
                self.results = None;
            }
        });
        ui.horizontal(|ui| {
            ui.label("嵌入模型");
            if ui
                .add_enabled(
                    !busy,
                    egui::TextEdit::singleline(&mut self.model).desired_width(460.0),
                )
                .changed()
            {
                self.results = None;
            }
        });
        ui.add_space(6.0);
        let query_response = ui.add_enabled(
            !busy,
            egui::TextEdit::singleline(&mut self.query)
                .hint_text("输入问题或关键词，按 Enter 检索")
                .desired_width(ui.available_width().min(900.0)),
        );
        let enter =
            query_response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        if query_response.changed() {
            self.results = None;
        }
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!busy, |ui| {
                egui::ComboBox::from_id_salt("hybrid-source")
                    .selected_text(
                        sources
                            .iter()
                            .find(|source| source.id == self.source_id)
                            .map_or("全部来源", |source| source.name.as_str()),
                    )
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_value(&mut self.source_id, String::new(), "全部来源")
                            .changed()
                        {
                            self.results = None;
                        }
                        for source in sources {
                            if ui
                                .selectable_value(
                                    &mut self.source_id,
                                    source.id.clone(),
                                    &source.name,
                                )
                                .changed()
                            {
                                self.results = None;
                            }
                        }
                    });
                egui::ComboBox::from_id_salt("hybrid-limit")
                    .selected_text(format!("最多 {} 条", self.limit))
                    .show_ui(ui, |ui| {
                        for limit in [10, 25, 50] {
                            if ui
                                .selectable_value(
                                    &mut self.limit,
                                    limit,
                                    format!("最多 {limit} 条"),
                                )
                                .changed()
                            {
                                self.results = None;
                            }
                        }
                    });
                egui::ComboBox::from_id_salt("hybrid-weight")
                    .selected_text(format!(
                        "关键词 {}% · 语义 {}%",
                        self.keyword_weight,
                        100 - self.keyword_weight
                    ))
                    .show_ui(ui, |ui| {
                        for weight in [25, 50, 75] {
                            if ui
                                .selectable_value(
                                    &mut self.keyword_weight,
                                    weight,
                                    format!("关键词 {weight}% · 语义 {}%", 100 - weight),
                                )
                                .changed()
                            {
                                self.results = None;
                            }
                        }
                    });
            });
            if (ui
                .add_enabled(!busy && !sources_locked, egui::Button::new("混合检索"))
                .clicked()
                || enter)
                && !busy
                && !sources_locked
            {
                self.start();
            }
            if busy && ui.button("取消").clicked() {
                self.cancel.store(true, Ordering::Relaxed);
            }
            if busy {
                ui.spinner();
            }
        });
        ui.label("固定 RRF 平滑常数 60；贡献 = 通道权重 ÷ (60 + 名次)。综合分只用于本次排序，不是事实可信度。模型或索引失效时不会假装给出双通道结果。" );
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        ui.add_space(8.0);
        if let Some(results) = &self.results {
            for (index, item) in results.hits.iter().enumerate() {
                let source_name = sources
                    .iter()
                    .find(|source| source.id == item.hit.source_id)
                    .map_or("已移除来源", |source| source.name.as_str());
                ui.group(|ui| {
                    ui.strong(format!(
                        "{}. {} / {} · 综合分 {:.5}",
                        index + 1,
                        source_name,
                        item.hit.relative,
                        item.score
                    ));
                    ui.small(format!(
                        "{} · 第 {} 片段 · 文件 SHA-256 {}…",
                        item.hit.location,
                        item.hit.ordinal,
                        &item.hit.file_sha256[..item.hit.file_sha256.len().min(12)]
                    ));
                    ui.horizontal_wrapped(|ui| {
                        if let Some(rank) = item.keyword_rank {
                            ui.label(format!(
                                "关键词第 {rank} 名 · 贡献 {:.5}",
                                item.keyword_contribution
                            ));
                        }
                        if let Some(rank) = item.semantic_rank {
                            ui.label(format!(
                                "语义第 {rank} 名 · 贡献 {:.5} · 余弦 {:.3}",
                                item.semantic_contribution,
                                item.cosine.unwrap_or_default()
                            ));
                        }
                    });
                    ui.label(item.hit.text.chars().take(260).collect::<String>());
                    ui.horizontal(|ui| {
                        if ui.button("复制引用").clicked() {
                            ui.ctx().copy_text(format!(
                                "[{}] {} · {}（文件 SHA-256: {}；片段 SHA-256: {}）",
                                source_name,
                                item.hit.relative,
                                item.hit.location,
                                item.hit.file_sha256,
                                item.hit.chunk_sha256
                            ));
                        }
                        if ui.button("复制片段与引用").clicked() {
                            ui.ctx().copy_text(format!(
                                "{}\n\n[{}] {} · {}（文件 SHA-256: {}；片段 SHA-256: {}）",
                                item.hit.text,
                                source_name,
                                item.hit.relative,
                                item.hit.location,
                                item.hit.file_sha256,
                                item.hit.chunk_sha256
                            ));
                        }
                        if ui.button("打开原文件").clicked() {
                            self.message = match crate::knowledge_search::verified_result_path(
                                &item.hit, sources,
                            )
                            .and_then(|path| open::that(path).map_err(Into::into))
                            {
                                Ok(()) => "已打开原文件，请按位置核对。".into(),
                                Err(error) => format!("无法打开原文件：{error:#}"),
                            };
                        }
                    });
                });
                ui.add_space(5.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(relative: &str, file: &str, chunk: &str) -> SearchHit {
        SearchHit {
            source_id: "source".into(),
            relative: relative.into(),
            location: "第 1 段".into(),
            ordinal: 1,
            file_sha256: file.into(),
            chunk_sha256: chunk.into(),
            text: relative.into(),
        }
    }

    #[test]
    fn fusion_explains_both_channels_and_weight_changes_order() {
        let lexical = vec![hit("a", "1", "1"), hit("b", "1", "1")];
        let semantic = vec![
            SemanticHit {
                hit: hit("b", "1", "1"),
                cosine: 0.9,
            },
            SemanticHit {
                hit: hit("a", "1", "1"),
                cosine: 0.8,
            },
        ];
        let lexical_first = fuse(lexical.clone(), semantic.clone(), 75, 10, false).unwrap();
        assert_eq!(lexical_first.hits[0].hit.relative, "a");
        assert_eq!(
            (
                lexical_first.hits[0].keyword_rank,
                lexical_first.hits[0].semantic_rank
            ),
            (Some(1), Some(2))
        );
        let semantic_first = fuse(lexical, semantic, 25, 10, false).unwrap();
        assert_eq!(semantic_first.hits[0].hit.relative, "b");
    }

    #[test]
    fn conflicting_versions_are_not_merged() {
        let result = fuse(
            vec![hit("a", "old", "one")],
            vec![SemanticHit {
                hit: hit("a", "new", "two"),
                cosine: 0.9,
            }],
            50,
            10,
            false,
        )
        .unwrap();
        assert!(result.hits.is_empty());
        assert_eq!(result.stale_dropped, 1);
    }
}
