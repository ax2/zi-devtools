//! Read-only keyword search over the explicitly synchronized local index.
use eframe::egui;
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
};

use crate::{
    knowledge_index::{self, SearchHit, SearchResults},
    knowledge_sources::Source,
};

pub fn verified_result_path(hit: &SearchHit, sources: &[Source]) -> anyhow::Result<PathBuf> {
    let source = sources
        .iter()
        .find(|source| source.id == hit.source_id)
        .ok_or_else(|| anyhow::anyhow!("来源已移除，请先同步索引"))?;
    let file = source
        .snapshot
        .as_ref()
        .and_then(|snapshot| {
            snapshot
                .files
                .iter()
                .find(|file| file.relative == hit.relative && file.sha256 == hit.file_sha256)
        })
        .ok_or_else(|| anyhow::anyhow!("文件不在当前扫描快照中，或版本已变化"))?;
    crate::document_ingestion::verified_source_path(source, file)
}

fn excerpt(text: &str, query: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let lower = text.to_lowercase();
    let pos = lower
        .find(&query.to_lowercase())
        .map(|byte| lower[..byte].chars().count())
        .unwrap_or(0);
    let start = pos.saturating_sub(60).min(chars.len());
    let end = (start + 260).min(chars.len());
    format!(
        "{}{}{}",
        if start > 0 { "…" } else { "" },
        chars[start..end].iter().collect::<String>(),
        if end < chars.len() { "…" } else { "" }
    )
}

fn citation(hit: &SearchHit, source_name: &str) -> String {
    format!(
        "[{}] {} · {}（文件 SHA-256: {}；片段 SHA-256: {}）",
        source_name, hit.relative, hit.location, hit.file_sha256, hit.chunk_sha256
    )
}

#[derive(Default)]
pub struct State {
    path: PathBuf,
    query: String,
    source_id: String,
    limit: usize,
    running: Option<Receiver<Result<SearchResults, String>>>,
    results: Option<SearchResults>,
    result_query: String,
    message: String,
}

impl State {
    pub(crate) fn background_active(&self) -> bool {
        self.running.is_some()
    }

    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            limit: 25,
            ..Self::default()
        }
    }

    fn start(&mut self) {
        if self.running.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let path = self.path.clone();
        let query = self.query.clone();
        let source = (!self.source_id.is_empty()).then(|| self.source_id.clone());
        let limit = self.limit;
        self.running = Some(rx);
        self.results = None;
        self.result_query = query.clone();
        self.message = "正在检索本机索引…".into();
        std::thread::spawn(move || {
            let result = knowledge_index::search(&path, &query, source.as_deref(), limit)
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(result);
        });
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self, sources: &[Source]) {
        self.query = "知识库".into();
        self.result_query = self.query.clone();
        self.results = Some(SearchResults { literal_match: true, hits: sources.first().map(|source| SearchHit {
            source_id: source.id.clone(),
            relative: "notes/knowledge.md".into(),
            location: "第 2 段".into(),
            ordinal: 3,
            file_sha256: "a".repeat(64),
            chunk_sha256: "b".repeat(64),
            text: "本机知识库把手动选择的 Markdown、PDF 和 Word 文档统一建立索引。检索结果保留来源位置，方便回到原文件核对。".into(),
        }).into_iter().collect() });
        self.message = "显示 1 个片段；中文查询使用字面包含匹配。".into();
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, sources: &[Source]) {
        if !self.source_id.is_empty() && !sources.iter().any(|source| source.id == self.source_id) {
            self.source_id.clear();
        }
        if let Some(rx) = &self.running {
            match rx.try_recv() {
                Ok(Ok(results)) => {
                    self.message = format!(
                        "显示 {} 个片段（最多 {} 个）{}；结果为本次查询快照。",
                        results.hits.len(),
                        self.limit,
                        if results.literal_match {
                            "，中文使用字面包含匹配"
                        } else {
                            "，按 FTS 相关度排序"
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
                    self.message = "检索任务意外结束".into();
                    self.running = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(100));
                }
            }
        }
        ui.heading("本机关键词检索");
        ui.label("查询手动同步的本机文档索引。结果显示文档位置与片段；打开原文件前重新校验当前扫描快照及内容哈希。");
        ui.add_space(8.0);
        let entered = ui
            .add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text("输入关键词，按 Enter 检索")
                    .desired_width(ui.available_width().min(920.0)),
            )
            .lost_focus()
            && ui.input(|input| input.key_pressed(egui::Key::Enter));
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("knowledge-search-source")
                .selected_text(
                    sources
                        .iter()
                        .find(|source| source.id == self.source_id)
                        .map_or("全部来源", |source| source.name.as_str()),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.source_id, String::new(), "全部来源");
                    for source in sources {
                        ui.selectable_value(&mut self.source_id, source.id.clone(), &source.name);
                    }
                });
            egui::ComboBox::from_id_salt("knowledge-search-limit")
                .selected_text(format!("最多 {} 条", self.limit))
                .show_ui(ui, |ui| {
                    for limit in [10, 25, 50] {
                        ui.selectable_value(&mut self.limit, limit, format!("最多 {limit} 条"));
                    }
                });
            if ui
                .add_enabled(self.running.is_none(), egui::Button::new("检索"))
                .clicked()
                || entered
            {
                self.start();
            }
            if self.running.is_some() {
                ui.spinner();
            }
        });
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        ui.small("英文词项使用 FTS5 相关度排序；含中文的查询使用字面包含匹配。当前不含向量语义检索。索引内容可能落后于源文件，以打开前校验为准。");
        ui.add_space(10.0);
        if let Some(results) = &self.results {
            for (index, hit) in results.hits.iter().enumerate() {
                let source_name = sources
                    .iter()
                    .find(|source| source.id == hit.source_id)
                    .map_or("已移除来源", |source| source.name.as_str());
                ui.group(|ui| {
                    ui.strong(format!("{}. {} / {}", index + 1, source_name, hit.relative));
                    ui.small(format!(
                        "{} · 第 {} 片段 · 文件 SHA-256 {}…",
                        hit.location,
                        hit.ordinal,
                        &hit.file_sha256[..hit.file_sha256.len().min(12)]
                    ));
                    ui.label(excerpt(&hit.text, &self.result_query));
                    ui.horizontal(|ui| {
                        if ui.button("复制引用").clicked() {
                            ui.ctx().copy_text(citation(hit, source_name));
                        }
                        if ui.button("复制片段与引用").clicked() {
                            ui.ctx().copy_text(format!(
                                "{}\n\n{}",
                                hit.text,
                                citation(hit, source_name)
                            ));
                        }
                        if ui.button("打开原文件").clicked() {
                            self.message = match verified_result_path(hit, sources)
                                .and_then(|path| open::that(path).map_err(Into::into))
                            {
                                Ok(()) => "已打开原文件；请按上方位置核对片段。".into(),
                                Err(error) => format!("无法打开原文件：{error:#}"),
                            };
                        }
                    });
                });
                ui.add_space(6.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge_sources::{add_source, scan};
    use std::{fs, sync::atomic::AtomicBool};

    #[test]
    fn result_open_requires_current_source_and_file_hash() {
        let root = std::env::temp_dir().join(format!("zi-search-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("note.txt"), "alpha").unwrap();
        let mut sources = Vec::new();
        let mut source = add_source(&mut sources, &root, "Docs", "").unwrap();
        source.snapshot = Some(scan(&source, &AtomicBool::new(false)).unwrap());
        let file = &source.snapshot.as_ref().unwrap().files[0];
        let hit = SearchHit {
            source_id: source.id.clone(),
            relative: file.relative.clone(),
            location: "第 1 段".into(),
            ordinal: 1,
            file_sha256: file.sha256.clone(),
            chunk_sha256: "a".repeat(64),
            text: "alpha".into(),
        };
        assert_eq!(
            verified_result_path(&hit, &[source.clone()]).unwrap(),
            root.join("note.txt").canonicalize().unwrap()
        );
        assert!(verified_result_path(&hit, &[]).is_err());
        fs::write(root.join("note.txt"), "changed").unwrap();
        assert!(verified_result_path(&hit, &[source]).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
