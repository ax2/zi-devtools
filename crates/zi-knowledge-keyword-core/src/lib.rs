//! Query planning, ranking and excerpts shared by native search and approved static snapshots.
use anyhow::{Result, ensure};
use rusqlite::{Connection, params};

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SearchMode {
    /// Preserve the original behavior: CJK phrases use literal matching, other text uses AND.
    #[default]
    Auto,
    ExactPhrase,
    AllTerms,
    AnyTerm,
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32, 0x3400..=0x9fff | 0xf900..=0xfaff | 0x20000..=0x2fa1f)
}

pub const INDEX_DDL: &str = r#"PRAGMA page_size=4096;
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
             END;"#;

pub fn checked_query(query: &str, limit: usize) -> Result<&str> {
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
    Ok(query)
}

pub fn uses_literal_match(query: &str, mode: SearchMode) -> bool {
    match mode {
        SearchMode::Auto => query.chars().any(is_cjk),
        SearchMode::ExactPhrase => true,
        SearchMode::AllTerms | SearchMode::AnyTerm => false,
    }
}

pub fn search_checked(
    conn: &Connection,
    query: &str,
    source_id: Option<&str>,
    limit: usize,
    mode: SearchMode,
) -> Result<SearchResults> {
    let literal_match = uses_literal_match(query, mode);
    if !literal_match {
        ensure!(
            query.chars().any(char::is_alphanumeric),
            "查询没有可搜索的文字或数字"
        );
    }
    let fts_query = query
        .split_whitespace()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(if mode == SearchMode::AnyTerm {
            " OR "
        } else {
            " AND "
        });
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

pub fn excerpt(text: &str, query: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let lower = text.to_lowercase();
    let lowered_query = query.to_lowercase();
    let pos = lower
        .find(&lowered_query)
        .or_else(|| {
            query
                .split_whitespace()
                .filter(|term| !term.is_empty())
                .filter_map(|term| lower.find(&term.to_lowercase()))
                .min()
        })
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
