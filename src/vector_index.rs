//! Optional local vectors derived only from the explicitly synchronized knowledge index.
use anyhow::{Context, Result, ensure};
use eframe::egui;
use reqwest::blocking::Client;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::Duration,
};

use crate::{embedding, knowledge_index::SearchHit, knowledge_sources::Source};

const MAX_CHUNKS: usize = 100_000;
const MAX_VECTOR_BYTES: usize = 128 * 1024 * 1024;
const MAX_TAGS_BYTES: u64 = 2 * 1024 * 1024;

pub fn default_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".zi-devtools/knowledge-vectors.sqlite3")
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub unchanged: usize,
    pub dimensions: usize,
    pub vector_bytes: usize,
    pub model_digest: String,
}

#[derive(Clone, Debug)]
pub struct SemanticHit {
    pub hit: SearchHit,
    pub cosine: f64,
}

#[derive(Clone, Debug)]
struct Chunk {
    hit: SearchHit,
}

type Key = (String, String, u64);

fn key(hit: &SearchHit) -> Key {
    (hit.source_id.clone(), hit.relative.clone(), hit.ordinal)
}

fn regular_file(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            ensure!(meta.file_type().is_file(), "索引路径不是普通文件");
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn check_version(conn: &Connection, expected: i64) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    ensure!(version == expected, "索引数据库版本不受支持：{version}");
    let result: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    ensure!(result == "ok", "索引数据库完整性检查失败：{result}");
    Ok(())
}

fn read_chunks(path: &Path) -> Result<Vec<Chunk>> {
    ensure!(regular_file(path)?, "请先同步本机全文知识索引");
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    check_version(&conn, 1)?;
    let mut stmt = conn.prepare(
        "SELECT c.source_id,c.relative,c.location,c.ordinal,f.sha256,c.sha256,c.text
         FROM chunks c JOIN indexed_files f
         ON f.source_id=c.source_id AND f.relative=c.relative
         ORDER BY c.source_id,c.relative,c.ordinal LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![(MAX_CHUNKS + 1) as i64], |row| {
        Ok(Chunk {
            hit: SearchHit {
                source_id: row.get(0)?,
                relative: row.get(1)?,
                location: row.get(2)?,
                ordinal: row.get(3)?,
                file_sha256: row.get(4)?,
                chunk_sha256: row.get(5)?,
                text: row.get(6)?,
            },
        })
    })?;
    let chunks = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    ensure!(chunks.len() <= MAX_CHUNKS, "向量索引最多处理 100000 个分块");
    Ok(chunks)
}

fn model_digest(endpoint: &str, model: &str, cancel: &AtomicBool) -> Result<String> {
    let mut url = embedding::endpoint_url(endpoint)?;
    embedding::validate_model(model)?;
    ensure!(!cancel.load(Ordering::Relaxed), "向量任务已取消");
    url.set_path("/api/tags");
    let client = Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let response = client
        .get(url)
        .send()
        .context("无法查询本机 Ollama 模型版本")?;
    ensure!(
        response.status().is_success(),
        "本机模型列表返回 HTTP {}",
        response.status()
    );
    let mut bytes = Vec::new();
    response.take(MAX_TAGS_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_TAGS_BYTES,
        "本机模型列表超过 2 MiB"
    );
    ensure!(!cancel.load(Ordering::Relaxed), "向量任务已取消");
    let json: Value = serde_json::from_slice(&bytes).context("本机模型列表不是 JSON")?;
    let models = json
        .get("models")
        .and_then(Value::as_array)
        .context("本机模型列表缺少 models")?;
    let digest = models
        .iter()
        .find(|entry| entry.get("name").and_then(Value::as_str) == Some(model))
        .and_then(|entry| entry.get("digest"))
        .and_then(Value::as_str)
        .context("模型不在本机列表中，请填写完整模型名并确认已安装")?;
    ensure!(
        digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
        "本机模型摘要无效，无法安全复用旧向量"
    );
    Ok(digest.to_ascii_lowercase())
}

fn open_vectors(path: &Path) -> Result<Connection> {
    let existed = regular_file(path)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path).context("无法打开本机向量索引")?;
    conn.busy_timeout(Duration::from_secs(5))?;
    if existed {
        check_version(&conn, 1)?;
    } else {
        conn.execute_batch(
            "PRAGMA page_size=4096;
             PRAGMA user_version=1;
             CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE vectors(
               source_id TEXT NOT NULL, relative TEXT NOT NULL, ordinal INTEGER NOT NULL,
               file_sha256 TEXT NOT NULL, chunk_sha256 TEXT NOT NULL,
               values_blob BLOB NOT NULL,
               PRIMARY KEY(source_id,relative,ordinal)
             );",
        )?;
    }
    conn.execute_batch(
        "PRAGMA journal_mode=DELETE; PRAGMA secure_delete=ON; PRAGMA max_page_count=65536;",
    )?;
    Ok(conn)
}

pub fn delete_index(path: &Path) -> Result<()> {
    for suffix in ["", "-journal", "-wal", "-shm"] {
        let candidate = PathBuf::from(format!("{}{suffix}", path.display()));
        if regular_file(&candidate)? {
            fs::remove_file(candidate)?;
        }
    }
    Ok(())
}

fn meta(conn: &Connection, name: &str) -> Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT value FROM meta WHERE key=?1")?;
    let mut rows = stmt.query(params![name])?;
    Ok(rows.next()?.map(|row| row.get(0)).transpose()?)
}

fn set_meta(conn: &Connection, name: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![name, value],
    )?;
    Ok(())
}

fn encode_vector(vector: &[f64]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| (*value as f32).to_le_bytes())
        .collect()
}

fn cosine(query: &[f64], bytes: &[u8]) -> Result<f64> {
    ensure!(bytes.len() == query.len() * 4, "向量维度或数据长度无效");
    let mut dot = 0.0;
    let mut norm = 0.0;
    for (value, raw) in query.iter().zip(bytes.chunks_exact(4)) {
        let stored = f32::from_le_bytes(raw.try_into().unwrap()) as f64;
        ensure!(stored.is_finite(), "向量数据包含无效数值");
        dot += value * stored;
        norm += stored * stored;
    }
    let qnorm: f64 = query.iter().map(|value| value * value).sum();
    ensure!(norm > 0.0 && qnorm > 0.0, "向量范数无效");
    Ok((dot / (norm.sqrt() * qnorm.sqrt())).clamp(-1.0, 1.0))
}

/// Synchronize all chunks from the current explicit keyword snapshot in one rollbackable transaction.
pub fn sync(
    path: &Path,
    keyword_path: &Path,
    endpoint: &str,
    model: &str,
    cancel: &AtomicBool,
    mut progress: impl FnMut(usize, usize),
) -> Result<SyncReport> {
    let chunks = read_chunks(keyword_path)?;
    let digest = model_digest(endpoint, model, cancel)?;
    let mut conn = open_vectors(path)?;
    let tx = conn.transaction()?;
    let compatible = meta(&tx, "model")?.as_deref() == Some(model)
        && meta(&tx, "digest")?.as_deref() == Some(digest.as_str());
    let mut report = SyncReport {
        model_digest: digest.clone(),
        ..SyncReport::default()
    };
    let mut old = HashMap::<Key, (String, String)>::new();
    let old_dimensions = meta(&tx, "dimensions")?
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    if compatible {
        let mut stmt =
            tx.prepare("SELECT source_id,relative,ordinal,file_sha256,chunk_sha256,length(values_blob) FROM vectors")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                (row.get(0)?, row.get(1)?, row.get(2)?),
                (row.get(3)?, row.get(4)?),
                row.get::<_, usize>(5)?,
            ))
        })?;
        for row in rows {
            let (key, hashes, bytes) = row?;
            ensure!(
                old_dimensions > 0 && bytes == old_dimensions * 4,
                "已存向量维度无效，请删除向量索引后重建"
            );
            old.insert(key, hashes);
        }
    } else {
        report.removed = tx.execute("DELETE FROM vectors", [])?;
    }
    let mut retained = HashSet::new();
    let mut vector_bytes: usize = tx.query_row(
        "SELECT COALESCE(SUM(length(values_blob)),0) FROM vectors",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        vector_bytes <= MAX_VECTOR_BYTES,
        "现有向量数据已超过 128 MiB"
    );
    let mut pending: Vec<&Chunk> = Vec::new();
    let mut dimensions = old_dimensions;
    if !compatible {
        dimensions = 0;
    }
    for (index, chunk) in chunks.iter().enumerate() {
        ensure!(
            !cancel.load(Ordering::Relaxed),
            "向量同步已取消；本轮修改已回滚"
        );
        let id = key(&chunk.hit);
        retained.insert(id.clone());
        if old.get(&id).is_some_and(|(file, text)| {
            file == &chunk.hit.file_sha256 && text == &chunk.hit.chunk_sha256
        }) {
            report.unchanged += 1;
        } else {
            pending.push(chunk);
        }
        if pending.len() == 16 || index + 1 == chunks.len() {
            if !pending.is_empty() {
                let texts = pending
                    .iter()
                    .map(|item| item.hit.text.clone())
                    .collect::<Vec<_>>();
                let vectors = embedding::embed_many(endpoint, model, &texts, cancel)?;
                for (item, vector) in pending.drain(..).zip(vectors) {
                    if dimensions == 0 {
                        dimensions = vector.len();
                    }
                    ensure!(
                        vector.len() == dimensions,
                        "模型维度在同步期间改变；本轮修改已回滚"
                    );
                    let bytes = encode_vector(&vector);
                    let previous_bytes: usize = tx
                        .query_row(
                            "SELECT length(values_blob) FROM vectors WHERE source_id=?1 AND relative=?2 AND ordinal=?3",
                            params![item.hit.source_id,item.hit.relative,item.hit.ordinal],
                            |row| row.get(0),
                        )
                        .optional()?
                        .unwrap_or(0);
                    vector_bytes = vector_bytes - previous_bytes + bytes.len();
                    ensure!(
                        vector_bytes <= MAX_VECTOR_BYTES,
                        "向量数据超过 128 MiB；本轮修改已回滚"
                    );
                    tx.execute(
                        "INSERT INTO vectors(source_id,relative,ordinal,file_sha256,chunk_sha256,values_blob)
                         VALUES(?1,?2,?3,?4,?5,?6)
                         ON CONFLICT(source_id,relative,ordinal) DO UPDATE SET
                         file_sha256=excluded.file_sha256,chunk_sha256=excluded.chunk_sha256,values_blob=excluded.values_blob",
                        params![item.hit.source_id,item.hit.relative,item.hit.ordinal,item.hit.file_sha256,item.hit.chunk_sha256,bytes],
                    )?;
                    if old.contains_key(&key(&item.hit)) {
                        report.updated += 1;
                    } else {
                        report.added += 1;
                    }
                }
            }
            progress(index + 1, chunks.len());
        }
    }
    for id in old.keys().filter(|id| !retained.contains(*id)) {
        ensure!(
            !cancel.load(Ordering::Relaxed),
            "向量同步已取消；本轮修改已回滚"
        );
        tx.execute(
            "DELETE FROM vectors WHERE source_id=?1 AND relative=?2 AND ordinal=?3",
            params![id.0, id.1, id.2],
        )?;
        report.removed += 1;
    }
    // The keyword snapshot may be updated by another UI task while embeddings are generated.
    let current = read_chunks(keyword_path)?;
    ensure!(
        current.len() == chunks.len()
            && current
                .iter()
                .zip(&chunks)
                .all(|(a, b)| key(&a.hit) == key(&b.hit)
                    && a.hit.file_sha256 == b.hit.file_sha256
                    && a.hit.chunk_sha256 == b.hit.chunk_sha256),
        "全文索引在向量同步期间变化；本轮修改已回滚"
    );
    ensure!(
        !cancel.load(Ordering::Relaxed),
        "向量同步已取消；本轮修改已回滚"
    );
    ensure!(
        model_digest(endpoint, model, cancel)? == digest,
        "本机模型在同步期间变化；本轮修改已回滚"
    );
    set_meta(&tx, "model", model)?;
    set_meta(&tx, "digest", &digest)?;
    set_meta(&tx, "dimensions", &dimensions.to_string())?;
    report.dimensions = dimensions;
    report.vector_bytes = tx.query_row(
        "SELECT COALESCE(SUM(length(values_blob)),0) FROM vectors",
        [],
        |row| row.get(0),
    )?;
    tx.commit()?;
    Ok(report)
}

pub fn search(
    path: &Path,
    keyword_path: &Path,
    endpoint: &str,
    model: &str,
    query: &str,
    limit: usize,
    cancel: &AtomicBool,
) -> Result<Vec<SemanticHit>> {
    ensure!((1..=50).contains(&limit), "结果上限必须在 1–50 条之间");
    ensure!(regular_file(path)?, "向量索引尚未建立，请先同步");
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    check_version(&conn, 1)?;
    ensure!(
        meta(&conn, "model")?.as_deref() == Some(model),
        "索引模型与当前选择不一致，请重新同步"
    );
    let digest = model_digest(endpoint, model, cancel)?;
    ensure!(
        meta(&conn, "digest")?.as_deref() == Some(digest.as_str()),
        "本机模型版本已变化，请重新同步向量"
    );
    let texts = vec![query.to_string()];
    let vector = embedding::embed_many(endpoint, model, &texts, cancel)?.remove(0);
    let dimensions = meta(&conn, "dimensions")?
        .and_then(|v| v.parse::<usize>().ok())
        .context("索引缺少维度")?;
    ensure!(vector.len() == dimensions, "模型维度与索引不同，请重新同步");
    let current = read_chunks(keyword_path)?
        .into_iter()
        .map(|chunk| (key(&chunk.hit), chunk.hit))
        .collect::<HashMap<_, _>>();
    let mut stmt = conn.prepare(
        "SELECT source_id,relative,ordinal,file_sha256,chunk_sha256,values_blob FROM vectors",
    )?;
    let mut rows = stmt.query([])?;
    let mut hits = Vec::new();
    while let Some(row) = rows.next()? {
        ensure!(!cancel.load(Ordering::Relaxed), "语义检索已取消");
        let id: Key = (row.get(0)?, row.get(1)?, row.get(2)?);
        let file_sha: String = row.get(3)?;
        let chunk_sha: String = row.get(4)?;
        if let Some(hit) = current.get(&id) {
            if hit.file_sha256 == file_sha && hit.chunk_sha256 == chunk_sha {
                let bytes: Vec<u8> = row.get(5)?;
                hits.push(SemanticHit {
                    hit: hit.clone(),
                    cosine: cosine(&vector, &bytes)?,
                });
            }
        }
    }
    hits.sort_by(|a, b| {
        b.cosine
            .total_cmp(&a.cosine)
            .then_with(|| key(&a.hit).cmp(&key(&b.hit)))
    });
    hits.truncate(limit);
    Ok(hits)
}

enum Event {
    Progress(usize, usize),
    Synced(Result<SyncReport, String>),
    Searched(Result<Vec<SemanticHit>, String>),
}

pub struct State {
    path: PathBuf,
    keyword_path: PathBuf,
    endpoint: String,
    model: String,
    query: String,
    limit: usize,
    running: Option<Receiver<Event>>,
    cancel: Arc<AtomicBool>,
    results: Vec<SemanticHit>,
    message: String,
    confirm_delete: bool,
}

impl Drop for State {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl State {
    pub fn new(path: PathBuf, keyword_path: PathBuf) -> Self {
        Self {
            path,
            keyword_path,
            endpoint: "http://127.0.0.1:11434/api/embed".into(),
            model: String::new(),
            query: String::new(),
            limit: 10,
            running: None,
            cancel: Arc::new(AtomicBool::new(false)),
            results: Vec::new(),
            message: String::new(),
            confirm_delete: false,
        }
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self, sources: &[Source]) {
        self.model = "bge-m3:latest".into();
        self.query = "如何避免并发数据竞争？".into();
        self.results = sources
            .first()
            .map(|source| SemanticHit {
                hit: SearchHit {
                    source_id: source.id.clone(),
                    relative: "notes/rust.md".into(),
                    location: "第 2 段".into(),
                    ordinal: 2,
                    file_sha256: "a".repeat(64),
                    chunk_sha256: "b".repeat(64),
                    text: "Rust 使用所有权与借用检查器，在编译期避免许多内存安全错误和数据竞争。检索结果保留文档位置与哈希，可返回原文件核对。".into(),
                },
                cosine: 0.873,
            })
            .into_iter()
            .collect();
        self.message = "合成界面预览 · 1024 维向量索引 · 未调用模型".into();
    }

    fn start_sync(&mut self) {
        if self.running.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let (path, keyword_path, endpoint, model) = (
            self.path.clone(),
            self.keyword_path.clone(),
            self.endpoint.clone(),
            self.model.clone(),
        );
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Arc::clone(&cancel);
        self.running = Some(rx);
        self.results.clear();
        self.message = "正在同步本机向量…".into();
        std::thread::spawn(move || {
            let report = sync(
                &path,
                &keyword_path,
                &endpoint,
                &model,
                &cancel,
                |done, total| {
                    let _ = tx.send(Event::Progress(done, total));
                },
            )
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Synced(report));
        });
    }

    fn start_search(&mut self) {
        if self.running.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let (path, keyword_path, endpoint, model, query, limit) = (
            self.path.clone(),
            self.keyword_path.clone(),
            self.endpoint.clone(),
            self.model.clone(),
            self.query.clone(),
            self.limit,
        );
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Arc::clone(&cancel);
        self.running = Some(rx);
        self.results.clear();
        self.message = "正在生成查询向量并检索…".into();
        std::thread::spawn(move || {
            let hits = search(
                &path,
                &keyword_path,
                &endpoint,
                &model,
                &query,
                limit,
                &cancel,
            )
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Searched(hits));
        });
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, sources: &[Source], sources_locked: bool) {
        if let Some(rx) = &self.running {
            match rx.try_recv() {
                Ok(Event::Progress(done, total)) => {
                    self.message = format!("正在同步向量：{done}/{total} 个分块")
                }
                Ok(Event::Synced(result)) => {
                    self.message = match result {
                        Ok(report) => format!(
                            "同步完成：新增 {}、更新 {}、移除 {}、复用 {}；{} 维，向量 {:.1} MiB。",
                            report.added,
                            report.updated,
                            report.removed,
                            report.unchanged,
                            report.dimensions,
                            report.vector_bytes as f64 / 1048576.0
                        ),
                        Err(error) => error,
                    };
                    self.running = None;
                }
                Ok(Event::Searched(result)) => {
                    match result {
                        Ok(hits) => {
                            self.message = format!(
                                "找到 {} 条语义命中；分数是余弦相似度，不代表事实可信度。",
                                hits.len()
                            );
                            self.results = hits;
                        }
                        Err(error) => self.message = error,
                    };
                    self.running = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.message = "向量任务意外结束".into();
                    self.running = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ui.ctx().request_repaint_after(Duration::from_millis(100))
                }
            }
        }
        ui.heading("本机向量索引");
        ui.label("从已手动同步的全文索引生成可选向量。文本仅发送到你明确选择的本机 Ollama；向量保存于本机数据库。" );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label("接口");
            if ui
                .add_enabled(
                    self.running.is_none(),
                    egui::TextEdit::singleline(&mut self.endpoint).desired_width(420.0),
                )
                .changed()
            {
                self.results.clear();
            }
        });
        ui.horizontal(|ui| {
            ui.label("模型");
            if ui
                .add_enabled(
                    self.running.is_none(),
                    egui::TextEdit::singleline(&mut self.model).desired_width(420.0),
                )
                .changed()
            {
                self.results.clear();
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.running.is_none() && !sources_locked,
                    egui::Button::new("同步全部已索引分块"),
                )
                .clicked()
            {
                self.start_sync();
            }
            if self.running.is_some() && ui.button("取消任务").clicked() {
                self.cancel.store(true, Ordering::Relaxed);
            }
            if self.running.is_some() {
                ui.spinner();
            }
            if ui
                .add_enabled(
                    self.running.is_none(),
                    egui::Button::new("删除本机向量索引"),
                )
                .clicked()
            {
                self.confirm_delete = true;
            }
        });
        if self.confirm_delete {
            ui.horizontal(|ui| {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    "仅删除本机生成的向量，不删除全文索引或源文件。",
                );
                if ui.button("确认删除向量").clicked() {
                    self.message = match delete_index(&self.path) {
                        Ok(()) => "本机向量索引已删除。".into(),
                        Err(error) => format!("删除失败：{error:#}"),
                    };
                    self.results.clear();
                    self.confirm_delete = false;
                }
                if ui.button("取消").clicked() {
                    self.confirm_delete = false;
                }
            });
        }
        ui.small("模型名称与本机摘要共同决定索引版本；模型升级需重新生成。同步失败或取消会回滚。本机向量上限 128 MiB。" );
        ui.separator();
        let response = ui.add_enabled(
            self.running.is_none(),
            egui::TextEdit::singleline(&mut self.query)
                .hint_text("自然语言查询")
                .desired_width(ui.available_width().min(820.0)),
        );
        if response.changed() {
            self.results.clear();
        }
        let enter = response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("vector-limit")
                .selected_text(format!("最多 {} 条", self.limit))
                .show_ui(ui, |ui| {
                    for limit in [10, 25, 50] {
                        ui.selectable_value(&mut self.limit, limit, format!("最多 {limit} 条"));
                    }
                });
            if (ui
                .add_enabled(self.running.is_none(), egui::Button::new("语义检索"))
                .clicked()
                || enter)
                && self.running.is_none()
            {
                self.start_search();
            }
        });
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        for (index, item) in self.results.iter().enumerate() {
            let source_name = sources
                .iter()
                .find(|source| source.id == item.hit.source_id)
                .map_or("已移除来源", |source| source.name.as_str());
            ui.group(|ui| {
                ui.strong(format!(
                    "{}. {} / {} · 相似度 {:.3}",
                    index + 1,
                    source_name,
                    item.hit.relative,
                    item.cosine
                ));
                ui.small(format!(
                    "{} · 第 {} 片段 · 文件 SHA-256 {}…",
                    item.hit.location,
                    item.hit.ordinal,
                    &item.hit.file_sha256[..item.hit.file_sha256.len().min(12)]
                ));
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
                    if ui.button("打开原文件").clicked() {
                        self.message =
                            match crate::knowledge_search::verified_result_path(&item.hit, sources)
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
