//! Explicit local SQLite/FTS5 index for scanned knowledge sources.
use anyhow::{Context, Result, ensure};
use eframe::egui;
use rusqlite::{Connection, OpenFlags, params};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
};

use crate::{document_ingestion, knowledge_sources::Source};

const MAX_FILES: usize = 2_000;
const MAX_SOURCE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_TEXT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_CHUNKS: u64 = 100_000;
const MAX_DB_PAGES: u64 = 65_536; // 4 KiB pages = 256 MiB.

pub fn default_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".zi-devtools/knowledge-index.sqlite3")
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexStats {
    pub files: u64,
    pub chunks: u64,
    pub text_bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub unchanged: usize,
    pub unscanned_sources: usize,
    pub stats: IndexStats,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SearchHit {
    pub source_id: String,
    pub relative: String,
    pub location: String,
    pub ordinal: u64,
    pub file_sha256: String,
    pub chunk_sha256: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    pub literal_match: bool,
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32, 0x3400..=0x9fff | 0xf900..=0xfaff | 0x20000..=0x2fa1f)
}

pub fn search(
    path: &Path,
    query: &str,
    source_id: Option<&str>,
    limit: usize,
) -> Result<SearchResults> {
    let query = query.trim();
    ensure!(!query.is_empty(), "请输入关键词");
    ensure!(
        query.chars().count() <= 120 && query.len() <= 512,
        "关键词最多 120 字符、512 字节"
    );
    ensure!(
        !query.chars().any(char::is_control),
        "关键词不能包含控制字符"
    );
    ensure!((1..=50).contains(&limit), "结果上限必须在 1 到 50 之间");
    ensure!(checked_file(path)?, "知识索引尚未建立，请先同步知识源");
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    check_version(&conn)?;
    let literal_match = query.chars().any(is_cjk);
    let fts_query = query
        .split_whitespace()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ");
    let sql = if literal_match {
        "SELECT c.source_id,c.relative,c.location,c.ordinal,f.sha256,c.sha256,c.text
         FROM chunks c JOIN indexed_files f ON f.source_id=c.source_id AND f.relative=c.relative
         WHERE instr(lower(c.text),lower(?1))>0 AND (?2 IS NULL OR c.source_id=?2)
         ORDER BY c.relative,c.ordinal LIMIT ?3"
    } else {
        "SELECT c.source_id,c.relative,c.location,c.ordinal,f.sha256,c.sha256,c.text
         FROM chunks_fts JOIN chunks c ON c.id=chunks_fts.rowid
         JOIN indexed_files f ON f.source_id=c.source_id AND f.relative=c.relative
         WHERE chunks_fts MATCH ?1 AND (?2 IS NULL OR c.source_id=?2)
         ORDER BY bm25(chunks_fts),c.relative,c.ordinal LIMIT ?3"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(
        params![
            if literal_match { query } else { &fts_query },
            source_id,
            limit as u64
        ],
        |row| {
            Ok(SearchHit {
                source_id: row.get(0)?,
                relative: row.get(1)?,
                location: row.get(2)?,
                ordinal: row.get(3)?,
                file_sha256: row.get(4)?,
                chunk_sha256: row.get(5)?,
                text: row.get(6)?,
            })
        },
    )?;
    let hits = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(SearchResults {
        hits,
        literal_match,
    })
}

fn checked_file(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            ensure!(meta.file_type().is_file(), "索引路径不是普通文件");
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

fn check_version(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    ensure!(version == 1, "知识索引数据库版本不受支持：{version}");
    let check: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    ensure!(check == "ok", "知识索引数据库完整性检查失败：{check}");
    Ok(())
}

fn open_index(path: &Path) -> Result<Connection> {
    let existed = checked_file(path)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path).context("无法打开本地知识索引")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    if existed {
        check_version(&conn)?;
    } else {
        conn.execute_batch(
            "PRAGMA page_size=4096;
             PRAGMA user_version=1;
             CREATE TABLE indexed_files (
               source_id TEXT NOT NULL,
               relative TEXT NOT NULL,
               sha256 TEXT NOT NULL,
               bytes INTEGER NOT NULL,
               text_bytes INTEGER NOT NULL,
               chunk_count INTEGER NOT NULL,
               indexed_at TEXT NOT NULL,
               PRIMARY KEY(source_id, relative)
             );
             CREATE TABLE chunks (
               id INTEGER PRIMARY KEY,
               source_id TEXT NOT NULL,
               relative TEXT NOT NULL,
               ordinal INTEGER NOT NULL,
               location TEXT NOT NULL,
               sha256 TEXT NOT NULL,
               text TEXT NOT NULL,
               FOREIGN KEY(source_id, relative) REFERENCES indexed_files(source_id, relative) ON DELETE CASCADE
             );
             CREATE INDEX chunks_source ON chunks(source_id, relative, ordinal);
             CREATE VIRTUAL TABLE chunks_fts USING fts5(text, content='chunks', content_rowid='id', tokenize='unicode61');
             CREATE TRIGGER chunks_ai AFTER INSERT ON chunks BEGIN
               INSERT INTO chunks_fts(rowid, text) VALUES(new.id, new.text);
             END;
             CREATE TRIGGER chunks_ad AFTER DELETE ON chunks BEGIN
               INSERT INTO chunks_fts(chunks_fts, rowid, text) VALUES('delete', old.id, old.text);
             END;
             CREATE TRIGGER chunks_au AFTER UPDATE ON chunks BEGIN
               INSERT INTO chunks_fts(chunks_fts, rowid, text) VALUES('delete', old.id, old.text);
               INSERT INTO chunks_fts(rowid, text) VALUES(new.id, new.text);
             END;",
        )
        .context("无法初始化本地 FTS5 索引")?;
    }
    conn.execute_batch(&format!(
        "PRAGMA foreign_keys=ON; PRAGMA secure_delete=ON; PRAGMA journal_mode=DELETE; PRAGMA max_page_count={MAX_DB_PAGES};"
    ))?;
    Ok(conn)
}

fn stats(conn: &Connection) -> Result<IndexStats> {
    let values = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(chunk_count),0), COALESCE(SUM(text_bytes),0) FROM indexed_files",
        [],
        |row| Ok(IndexStats { files: row.get(0)?, chunks: row.get(1)?, text_bytes: row.get(2)? }),
    )?;
    Ok(values)
}

pub fn inspect(path: &Path) -> Result<IndexStats> {
    if !checked_file(path)? {
        return Ok(IndexStats::default());
    }
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    check_version(&conn)?;
    stats(&conn)
}

fn source_stats(path: &Path) -> Result<HashMap<String, IndexStats>> {
    if !checked_file(path)? {
        return Ok(HashMap::new());
    }
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    check_version(&conn)?;
    let mut statement = conn.prepare("SELECT source_id, COUNT(*), COALESCE(SUM(chunk_count),0), COALESCE(SUM(text_bytes),0) FROM indexed_files GROUP BY source_id")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            IndexStats {
                files: row.get(1)?,
                chunks: row.get(2)?,
                text_bytes: row.get(3)?,
            },
        ))
    })?;
    let mut result = HashMap::new();
    for row in rows {
        let (id, stats) = row?;
        result.insert(id, stats);
    }
    Ok(result)
}

pub fn delete_index(path: &Path) -> Result<()> {
    // SQLite uses DELETE journaling here; remove crash sidecars as well.
    for suffix in ["", "-journal", "-wal", "-shm"] {
        let candidate = if suffix.is_empty() {
            path.to_path_buf()
        } else {
            PathBuf::from(format!("{}{suffix}", path.display()))
        };
        if checked_file(&candidate)? {
            fs::remove_file(&candidate)?;
        }
    }
    Ok(())
}

fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    ensure!(
        !cancel.load(Ordering::Relaxed),
        "索引同步已取消；本轮修改已回滚"
    );
    Ok(())
}

pub fn sync_all(
    path: &Path,
    sources: &[Source],
    rebuild: bool,
    cancel: &AtomicBool,
    mut progress: impl FnMut(usize, usize, &str),
) -> Result<SyncReport> {
    ensure!(sources.len() <= 32, "知识源数量超过 32 个");
    let file_count: usize = sources
        .iter()
        .filter_map(|s| s.snapshot.as_ref())
        .map(|s| s.files.len())
        .sum();
    let input_bytes: u64 = sources
        .iter()
        .filter_map(|s| s.snapshot.as_ref())
        .map(|s| s.total_bytes)
        .sum();
    ensure!(
        file_count <= MAX_FILES && input_bytes <= MAX_SOURCE_BYTES,
        "索引最多处理 2000 个文件、128 MiB 源数据，请缩小知识源范围"
    );
    check_cancel(cancel)?;
    let mut conn = open_index(path)?;
    let tx = conn.transaction()?;
    let mut old = HashMap::<(String, String), (String, u64, u64)>::new();
    {
        let mut statement = tx.prepare(
            "SELECT source_id, relative, sha256, chunk_count, text_bytes FROM indexed_files",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                (row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                (
                    row.get::<_, String>(2)?,
                    row.get::<_, u64>(3)?,
                    row.get::<_, u64>(4)?,
                ),
            ))
        })?;
        for row in rows {
            let (key, value) = row?;
            old.insert(key, value);
        }
    }
    let mut report = SyncReport::default();
    let mut retained = HashSet::new();
    for source in sources {
        if let Some(snapshot) = &source.snapshot {
            for file in &snapshot.files {
                retained.insert((source.id.clone(), file.relative.clone()));
            }
        } else {
            report.unscanned_sources += 1;
        }
    }
    for (source_id, relative) in old.keys() {
        check_cancel(cancel)?;
        if rebuild || !retained.contains(&(source_id.clone(), relative.clone())) {
            tx.execute(
                "DELETE FROM indexed_files WHERE source_id=?1 AND relative=?2",
                params![source_id, relative],
            )?;
            if !retained.contains(&(source_id.clone(), relative.clone())) {
                report.removed += 1;
            }
        }
    }
    let mut aggregate = if rebuild {
        IndexStats::default()
    } else {
        stats(&tx)?
    };
    let mut done = 0;
    for source in sources {
        let Some(snapshot) = &source.snapshot else {
            continue;
        };
        for file in &snapshot.files {
            check_cancel(cancel)?;
            done += 1;
            progress(done, file_count, &file.relative);
            let key = (source.id.clone(), file.relative.clone());
            let previous = old.get(&key);
            if !rebuild && previous.is_some_and(|(sha, _, _)| sha == &file.sha256) {
                report.unchanged += 1;
                continue;
            }
            let preview = document_ingestion::ingest(source, file)
                .with_context(|| format!("解析 {} / {} 失败", source.name, file.relative))?;
            check_cancel(cancel)?;
            let text_bytes: u64 = preview.chunks.iter().map(|c| c.text.len() as u64).sum();
            let count = preview.chunks.len() as u64;
            if !rebuild && let Some((_, old_count, old_bytes)) = previous {
                tx.execute(
                    "DELETE FROM indexed_files WHERE source_id=?1 AND relative=?2",
                    params![source.id, file.relative],
                )?;
                aggregate.chunks -= old_count;
                aggregate.text_bytes -= old_bytes;
                aggregate.files -= 1;
            }
            aggregate.files += 1;
            aggregate.chunks += count;
            aggregate.text_bytes += text_bytes;
            ensure!(
                aggregate.chunks <= MAX_CHUNKS && aggregate.text_bytes <= MAX_TEXT_BYTES,
                "索引超过 100000 个分块或 128 MiB 正文预算；本轮修改已回滚"
            );
            tx.execute(
                "INSERT INTO indexed_files(source_id,relative,sha256,bytes,text_bytes,chunk_count,indexed_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![source.id, file.relative, file.sha256, file.bytes, text_bytes, count, chrono::Utc::now().to_rfc3339()],
            )?;
            {
                let mut insert = tx.prepare("INSERT INTO chunks(source_id,relative,ordinal,location,sha256,text) VALUES(?1,?2,?3,?4,?5,?6)")?;
                for chunk in &preview.chunks {
                    check_cancel(cancel)?;
                    insert.execute(params![
                        source.id,
                        file.relative,
                        chunk.ordinal,
                        chunk.location,
                        chunk.sha256,
                        chunk.text
                    ])?;
                }
            }
            if previous.is_some() {
                report.updated += 1;
            } else {
                report.added += 1;
            }
        }
    }
    check_cancel(cancel)?;
    report.stats = stats(&tx)?;
    ensure!(
        report.stats.chunks <= MAX_CHUNKS && report.stats.text_bytes <= MAX_TEXT_BYTES,
        "索引资源预算已超出；本轮修改已回滚"
    );
    tx.commit()?;
    Ok(report)
}

enum Event {
    Progress(usize, usize, String),
    Finished(Result<SyncReport, String>),
}

pub struct State {
    path: PathBuf,
    running: Option<Receiver<Event>>,
    cancel: Arc<AtomicBool>,
    stats: Option<IndexStats>,
    source_stats: HashMap<String, IndexStats>,
    message: String,
    confirm_rebuild: bool,
    confirm_delete: bool,
}

impl State {
    pub fn new(path: PathBuf) -> Self {
        let (stats, message) = match inspect(&path) {
            Ok(stats) => (Some(stats), String::new()),
            Err(e) => (None, format!("索引读取失败：{e:#}；不会覆盖现有数据库")),
        };
        let source_stats = if stats.is_some() {
            source_stats(&path).unwrap_or_default()
        } else {
            HashMap::new()
        };
        Self {
            path,
            running: None,
            cancel: Arc::new(AtomicBool::new(false)),
            stats,
            source_stats,
            message,
            confirm_rebuild: false,
            confirm_delete: false,
        }
    }

    fn start(&mut self, sources: Vec<Source>, rebuild: bool) {
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let path = self.path.clone();
        let (tx, rx) = mpsc::channel();
        self.running = Some(rx);
        self.message = if rebuild {
            "正在重建知识索引…"
        } else {
            "正在同步知识索引…"
        }
        .into();
        std::thread::spawn(move || {
            let result = sync_all(
                &path,
                &sources,
                rebuild,
                &cancel,
                |done, total, relative| {
                    let _ = tx.send(Event::Progress(done, total, relative.to_owned()));
                },
            );
            let _ = tx.send(Event::Finished(result.map_err(|e| format!("{e:#}"))));
        });
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self, sources: &[Source]) {
        self.stats = Some(IndexStats {
            files: 12,
            chunks: 84,
            text_bytes: 183_920,
        });
        if let Some(source) = sources.first() {
            self.source_stats.insert(
                source.id.clone(),
                IndexStats {
                    files: 12,
                    chunks: 84,
                    text_bytes: 183_920,
                },
            );
        }
        self.message = "上次同步：新增 2、更新 1、移除 1、未变 8；索引已保存到本机。".into();
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, sources: &[Source], sources_locked: bool) {
        let mut finished = None;
        if let Some(rx) = &self.running {
            loop {
                match rx.try_recv() {
                    Ok(Event::Progress(done, total, relative)) => {
                        self.message = format!("正在处理 {done}/{total}：{relative}")
                    }
                    Ok(Event::Finished(result)) => {
                        finished = Some(result);
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        finished = Some(Err("索引任务意外结束".into()));
                        break;
                    }
                }
            }
        }
        if let Some(result) = finished {
            self.running = None;
            match result {
                Ok(report) => {
                    self.stats = Some(report.stats);
                    self.source_stats = source_stats(&self.path).unwrap_or_default();
                    self.message = format!(
                        "同步完成：新增 {}、更新 {}、移除 {}、未变 {}；{} 个来源尚未扫描",
                        report.added,
                        report.updated,
                        report.removed,
                        report.unchanged,
                        report.unscanned_sources
                    );
                }
                Err(e) => self.message = e,
            }
        }
        ui.heading("增量知识索引");
        ui.label("手动同步已扫描的本地知识源。索引会在本机保存分块正文；不上传网络。文件变更会更新，移出快照或移除知识源会删除对应索引记录。");
        if sources_locked {
            ui.colored_label(
                egui::Color32::RED,
                "知识源配置读取失败；同步与重建已禁用，避免误删现有索引。请先修复知识源配置。",
            );
        }
        ui.add_space(8.0);
        ui.group(|ui| {
            ui.strong("本机存储");
            ui.monospace(self.path.display().to_string());
            if let Some(stats) = &self.stats {
                ui.label(format!("{} 个文件 · {} 个分块 · 正文约 {} KiB", stats.files, stats.chunks, stats.text_bytes.div_ceil(1024)));
                for source in sources {
                    let count = self.source_stats.get(&source.id).map_or(0, |s| s.files);
                    ui.small(format!("{}：{} 个已索引文件{}", source.name, count, if source.snapshot.is_none() { " · 待扫描" } else { "" }));
                }
            } else { ui.label("索引状态无法读取，请检查数据库文件。操作不会自动覆盖它。"); }
            ui.small("预算：最多 2000 个源文件、128 MiB 输入、100000 分块、128 MiB 正文、约 256 MiB 数据库。未扫描的来源不加入索引。关键词检索可从左侧进入；带引用问答仍在规划。");
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.running.is_none() && self.stats.is_some() && !sources_locked,
                    egui::Button::new("同步全部已扫描来源"),
                )
                .clicked()
            {
                self.start(sources.to_vec(), false);
            }
            if ui
                .add_enabled(
                    self.running.is_none() && self.stats.is_some() && !sources_locked,
                    egui::Button::new("重建索引…"),
                )
                .clicked()
            {
                self.confirm_rebuild = true;
            }
            if ui
                .add_enabled(self.running.is_none(), egui::Button::new("删除本机索引…"))
                .clicked()
            {
                self.confirm_delete = true;
            }
            if self.running.is_some() {
                ui.spinner();
                if ui.button("取消").clicked() {
                    self.cancel.store(true, Ordering::Relaxed);
                }
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(100));
            }
        });
        if self.confirm_rebuild {
            ui.horizontal(|ui| {
                ui.label("重新解析所有已扫描文件；失败时保留原索引。确认？");
                if ui.button("确认重建").clicked() {
                    self.confirm_rebuild = false;
                    self.start(sources.to_vec(), true);
                }
                if ui.button("取消").clicked() {
                    self.confirm_rebuild = false;
                }
            });
        }
        if self.confirm_delete {
            ui.horizontal(|ui| {
                ui.label("删除索引数据库及其中正文；源文件和知识源清单保留。确定？");
                if ui.button("确认删除索引").clicked() {
                    self.confirm_delete = false;
                    self.message = match delete_index(&self.path) {
                        Ok(()) => {
                            self.stats = Some(IndexStats::default());
                            self.source_stats.clear();
                            "索引数据库已删除；源文件未修改".into()
                        }
                        Err(e) => format!("删除索引失败：{e:#}"),
                    };
                }
                if ui.button("取消").clicked() {
                    self.confirm_delete = false;
                }
            });
        }
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge_sources::{add_source, scan};

    fn fixture() -> (PathBuf, PathBuf, Source) {
        let root = std::env::temp_dir().join(format!("zi-index-test-{}", uuid::Uuid::new_v4()));
        let docs = root.join("docs");
        fs::create_dir_all(&docs).unwrap();
        fs::write(docs.join("one.txt"), "alpha first document").unwrap();
        fs::write(docs.join("two.md"), "# Bravo\n\nsecond document").unwrap();
        let mut sources = Vec::new();
        let mut source = add_source(&mut sources, &docs, "Docs", "").unwrap();
        source.snapshot = Some(scan(&source, &AtomicBool::new(false)).unwrap());
        let index = root.join("knowledge-index.sqlite3");
        (root, index, source)
    }

    fn count_match(path: &Path, term: &str) -> u64 {
        let conn = Connection::open(path).unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH ?1",
            [term],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn keyword_search_is_bounded_filtered_and_tracks_index_changes() {
        let (root, index, mut source) = fixture();
        fs::write(
            root.join("docs/chinese.txt"),
            "本机知识库支持查找文档片段。另一个知识库示例。",
        )
        .unwrap();
        source.snapshot = Some(scan(&source, &AtomicBool::new(false)).unwrap());
        sync_all(
            &index,
            &[source.clone()],
            false,
            &AtomicBool::new(false),
            |_, _, _| {},
        )
        .unwrap();
        let english = search(&index, "alpha", None, 10).unwrap();
        assert_eq!(english.hits.len(), 1);
        assert!(!english.literal_match);
        assert_eq!(english.hits[0].relative, "one.txt");
        assert_eq!(
            search(&index, "alpha", Some("missing"), 10)
                .unwrap()
                .hits
                .len(),
            0
        );
        let chinese = search(&index, "知识库", None, 1).unwrap();
        assert!(chinese.literal_match);
        assert_eq!(chinese.hits.len(), 1);
        assert_eq!(chinese.hits[0].relative, "chinese.txt");
        assert!(
            search(&index, "alpha\" OR bravo", None, 10)
                .unwrap()
                .hits
                .is_empty()
        );
        assert!(search(&index, "alpha", None, 51).is_err());
        fs::write(root.join("docs/one.txt"), "revised document").unwrap();
        source.snapshot = Some(scan(&source, &AtomicBool::new(false)).unwrap());
        sync_all(
            &index,
            &[source],
            false,
            &AtomicBool::new(false),
            |_, _, _| {},
        )
        .unwrap();
        assert!(search(&index, "alpha", None, 10).unwrap().hits.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn incremental_sync_updates_and_deletes_fts_rows() {
        let (root, index, mut source) = fixture();
        assert_eq!(inspect(&index).unwrap(), IndexStats::default());
        let first = sync_all(
            &index,
            &[source.clone()],
            false,
            &AtomicBool::new(false),
            |_, _, _| {},
        )
        .unwrap();
        assert_eq!(
            (first.added, first.updated, first.removed, first.unchanged),
            (2, 0, 0, 0)
        );
        assert_eq!(first.stats.files, 2);
        assert_eq!(count_match(&index, "alpha"), 1);
        assert_eq!(count_match(&index, "bravo"), 1);

        let again = sync_all(
            &index,
            &[source.clone()],
            false,
            &AtomicBool::new(false),
            |_, _, _| {},
        )
        .unwrap();
        assert_eq!(
            (again.added, again.updated, again.removed, again.unchanged),
            (0, 0, 0, 2)
        );

        fs::write(root.join("docs/one.txt"), "charlie revised document").unwrap();
        fs::remove_file(root.join("docs/two.md")).unwrap();
        source.snapshot = Some(scan(&source, &AtomicBool::new(false)).unwrap());
        let changed = sync_all(
            &index,
            &[source.clone()],
            false,
            &AtomicBool::new(false),
            |_, _, _| {},
        )
        .unwrap();
        assert_eq!(
            (
                changed.added,
                changed.updated,
                changed.removed,
                changed.unchanged
            ),
            (0, 1, 1, 0)
        );
        assert_eq!(changed.stats.files, 1);
        assert_eq!(count_match(&index, "alpha"), 0);
        assert_eq!(count_match(&index, "bravo"), 0);
        assert_eq!(count_match(&index, "charlie"), 1);

        let rebuilt = sync_all(
            &index,
            &[source.clone()],
            true,
            &AtomicBool::new(false),
            |_, _, _| {},
        )
        .unwrap();
        assert_eq!(
            (
                rebuilt.added,
                rebuilt.updated,
                rebuilt.removed,
                rebuilt.unchanged
            ),
            (0, 1, 0, 0)
        );
        assert_eq!(count_match(&index, "charlie"), 1);
        let removed = sync_all(&index, &[], false, &AtomicBool::new(false), |_, _, _| {}).unwrap();
        assert_eq!(removed.removed, 1);
        assert_eq!(removed.stats.files, 0);
        assert_eq!(count_match(&index, "charlie"), 0);
        assert!(source_stats(&index).unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancellation_and_parse_failure_preserve_previous_index() {
        let (root, index, mut source) = fixture();
        sync_all(
            &index,
            &[source.clone()],
            false,
            &AtomicBool::new(false),
            |_, _, _| {},
        )
        .unwrap();
        let before = inspect(&index).unwrap();
        let cancelled = AtomicBool::new(true);
        assert!(sync_all(&index, &[source.clone()], true, &cancelled, |_, _, _| {}).is_err());
        assert_eq!(inspect(&index).unwrap(), before);
        fs::write(root.join("docs/one.txt"), "changed outside snapshot").unwrap();
        let error = sync_all(
            &index,
            &[source.clone()],
            true,
            &AtomicBool::new(false),
            |_, _, _| {},
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("快照"));
        assert_eq!(inspect(&index).unwrap(), before);
        assert_eq!(count_match(&index, "alpha"), 1);
        fs::write(root.join("docs/one.txt"), "delta replacement").unwrap();
        fs::write(root.join("docs/two.md"), "# Echo replacement").unwrap();
        source.snapshot = Some(scan(&source, &AtomicBool::new(false)).unwrap());
        let stop_midway = AtomicBool::new(false);
        let interrupted = sync_all(
            &index,
            &[source.clone()],
            false,
            &stop_midway,
            |done, _, _| {
                if done == 2 {
                    stop_midway.store(true, Ordering::Relaxed);
                }
            },
        );
        assert!(interrupted.is_err());
        assert_eq!(count_match(&index, "alpha"), 1);
        assert_eq!(count_match(&index, "delta"), 0);
        sync_all(
            &index,
            &[source],
            false,
            &AtomicBool::new(false),
            |_, _, _| {},
        )
        .unwrap();
        assert_eq!(count_match(&index, "alpha"), 0);
        let conn = Connection::open(&index).unwrap();
        conn.execute(
            "INSERT INTO chunks_fts(chunks_fts) VALUES('integrity-check')",
            [],
        )
        .unwrap();
        drop(conn);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupt_database_is_not_overwritten() {
        let (root, index, source) = fixture();
        let broken = b"not a sqlite database";
        fs::write(&index, broken).unwrap();
        assert!(
            sync_all(
                &index,
                &[source],
                false,
                &AtomicBool::new(false),
                |_, _, _| {}
            )
            .is_err()
        );
        assert_eq!(fs::read(&index).unwrap(), broken);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unscanned_source_is_removed_and_explicit_delete_preserves_documents() {
        let (root, index, mut source) = fixture();
        sync_all(
            &index,
            &[source.clone()],
            false,
            &AtomicBool::new(false),
            |_, _, _| {},
        )
        .unwrap();
        source.snapshot = None;
        let report = sync_all(
            &index,
            &[source],
            false,
            &AtomicBool::new(false),
            |_, _, _| {},
        )
        .unwrap();
        assert_eq!(report.removed, 2);
        assert_eq!(report.unscanned_sources, 1);
        assert_eq!(inspect(&index).unwrap().files, 0);
        delete_index(&index).unwrap();
        assert!(!index.exists());
        assert!(root.join("docs/one.txt").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
