//! Explicitly saved, versioned local artifacts shared between tools.
use anyhow::{Context, Result, ensure};
use eframe::egui;
use image::{DynamicImage, ImageFormat, ImageReader, Limits};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use sha2::{Digest, Sha256};
use similar::{ChangeTag, TextDiff};
use std::{
    fs,
    io::Cursor,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::{Duration, Instant},
};

pub const MAX_VERSION_BYTES: usize = 16 * 1024 * 1024;
const MAX_TOTAL_BYTES: i64 = 256 * 1024 * 1024;
const MAX_ARTIFACTS: i64 = 512;
const MAX_VERSIONS_PER_ARTIFACT: i64 = 256;
const SEARCH_PREVIEW_CONTEXT_CHARS: usize = 36;
const MAX_DB_BYTES: u64 = 320 * 1024 * 1024;
const MAX_DIFF_INPUT_BYTES: usize = 2 * 1024 * 1024;
const MAX_DIFF_OUTPUT_BYTES: usize = 64 * 1024;
const APPLICATION_ID: i64 = 1_514_359_890;
const MAX_IMAGE_PIXELS: u64 = 16_000_000;

fn image_limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(12_000);
    limits.max_image_height = Some(12_000);
    limits.max_alloc = Some(192 * 1024 * 1024);
    limits
}

fn decode_png(payload: &[u8]) -> Result<DynamicImage> {
    ensure!(!payload.is_empty(), "PNG 图片内容为空");
    ensure!(payload.len() <= MAX_VERSION_BYTES, "PNG 图片超过 16 MiB");
    let mut reader = ImageReader::with_format(Cursor::new(payload), ImageFormat::Png);
    reader.limits(image_limits());
    let (width, height) = reader.into_dimensions().context("PNG 图片头无效")?;
    ensure!(
        width > 0
            && height > 0
            && width <= 12_000
            && height <= 12_000
            && u64::from(width) * u64::from(height) <= MAX_IMAGE_PIXELS,
        "PNG 图片尺寸超过资料库限制"
    );
    let mut reader = ImageReader::with_format(Cursor::new(payload), ImageFormat::Png);
    reader.limits(image_limits());
    let image = reader.decode().context("PNG 图片数据无效")?;
    ensure!(
        image.as_bytes().len() <= 192 * 1024 * 1024,
        "PNG 解码内存超限"
    );
    Ok(image)
}

fn encode_png(image: &DynamicImage) -> Result<Vec<u8>> {
    ensure!(
        image.width() > 0
            && image.height() > 0
            && image.width() <= 12_000
            && image.height() <= 12_000
            && u64::from(image.width()) * u64::from(image.height()) <= MAX_IMAGE_PIXELS,
        "图片尺寸超过资料库限制"
    );
    let mut cursor = Cursor::new(Vec::new());
    image
        .write_to(&mut cursor, ImageFormat::Png)
        .context("无法编码 PNG 图片")?;
    let payload = cursor.into_inner();
    ensure!(
        payload.len() <= MAX_VERSION_BYTES,
        "编码后 PNG 超过单版 16 MiB"
    );
    Ok(payload)
}

fn image_preview(image: &DynamicImage) -> egui::ColorImage {
    let thumbnail = image.thumbnail(640, 400).to_rgba8();
    egui::ColorImage::from_rgba_unmultiplied(
        [thumbnail.width() as usize, thumbnail.height() as usize],
        thumbnail.as_raw(),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Json,
    Csv,
    Tsv,
    Table,
    Image,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Json => "json",
            Self::Csv => "csv",
            Self::Tsv => "tsv",
            Self::Table => "table",
            Self::Image => "image",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            "csv" => Ok(Self::Csv),
            "tsv" => Ok(Self::Tsv),
            "table" => Ok(Self::Table),
            "image" => Ok(Self::Image),
            _ => anyhow::bail!("资料类型无效"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Text => "文本",
            Self::Json => "JSON",
            Self::Csv => "CSV",
            Self::Tsv => "TSV",
            Self::Table => "结构化表格",
            Self::Image => "PNG 图片",
        }
    }

    fn validate(self, payload: &[u8]) -> Result<()> {
        ensure!(
            payload.len() <= MAX_VERSION_BYTES,
            "单个资料版本最多 16 MiB"
        );
        if self == Self::Image {
            decode_png(payload)?;
            return Ok(());
        }
        let text = std::str::from_utf8(payload).context("当前阶段仅保存 UTF-8 文本材料")?;
        match self {
            Self::Text => {}
            Self::Json => {
                serde_json::from_str::<serde_json::Value>(text).context("JSON 内容无效")?;
            }
            Self::Csv | Self::Tsv => {
                let mut reader = csv::ReaderBuilder::new()
                    .delimiter(if self == Self::Csv { b',' } else { b'\t' })
                    .has_headers(false)
                    .from_reader(payload);
                for row in reader.records() {
                    row.context("表格内容无效")?;
                }
            }
            Self::Table => {
                let dataset: crate::workbench::Dataset =
                    serde_json::from_str(text).context("结构化表格内容无效")?;
                crate::workbench::Dataset::from_parts(dataset.headers, dataset.rows)
                    .context("结构化表格结构无效")?;
            }
            Self::Image => unreachable!("image validation returned above"),
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VersionAction {
    Create,
    Append,
    Replace,
}

impl VersionAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Append => "append",
            Self::Replace => "replace",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "create" => Ok(Self::Create),
            "append" => Ok(Self::Append),
            "replace" => Ok(Self::Replace),
            _ => anyhow::bail!("资料操作记录无效"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Create => "新建",
            Self::Append => "追加版本",
            Self::Replace => "替换当前版",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub revision: i64,
    pub updated: String,
    pub bytes: i64,
    pub sha256: String,
    pub versions: i64,
    pub source: String,
    pub historical_source_match: Option<(i64, String)>,
    pub search_preview: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub revision: i64,
    pub action: VersionAction,
    pub created: String,
    pub bytes: Vec<u8>,
    pub sha256: String,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionSummary {
    pub revision: i64,
    pub action: VersionAction,
    pub created: String,
    pub bytes: i64,
    pub sha256: String,
    pub source: String,
}

#[derive(Clone)]
pub struct Store {
    pub path: PathBuf,
}

type SearchRequest = (u64, String, Option<Kind>, bool);
type SearchResult = (u64, std::result::Result<Vec<Entry>, String>);

fn trash_table_exists(db: &Connection) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='artifact_trash')",
        [],
        |row| row.get(0),
    )?)
}

fn validate_name(name: &str) -> Result<()> {
    ensure!(
        !name.trim().is_empty()
            && name.chars().count() <= 120
            && !name.chars().any(char::is_control),
        "名称须为 1–120 字且不能包含控制字符"
    );
    Ok(())
}

fn validate_id(id: &str) -> Result<()> {
    ensure!(uuid::Uuid::parse_str(id)?.to_string() == id, "资料标识无效");
    Ok(())
}

fn validate_source(source: &str) -> Result<()> {
    if matches!(source, "本机编辑" | "工具接力") {
        return Ok(());
    }
    let (tool_id, tool_version) = source
        .split_once('@')
        .ok_or_else(|| anyhow::anyhow!("来源标记无效；不保存自由文本、路径或凭据"))?;
    ensure!(
        !tool_id.is_empty()
            && tool_id.len() <= 128
            && tool_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte)),
        "来源工具标识无效"
    );
    let version = semver::Version::parse(tool_version).context("来源工具版本无效")?;
    ensure!(
        version.pre.is_empty()
            && version.build.is_empty()
            && format!("{}.{}.{}", version.major, version.minor, version.patch) == tool_version,
        "来源工具版本必须为稳定的 x.y.z 格式"
    );
    Ok(())
}

fn digest(payload: &[u8]) -> String {
    format!("{:x}", Sha256::digest(payload))
}

fn cancel_sqlite_query_when_search_changes(
    db: &Connection,
    generation: &Arc<AtomicU64>,
    expected: u64,
) {
    let generation = Arc::clone(generation);
    db.progress_handler(
        1000,
        Some(move || generation.load(Ordering::Relaxed) != expected),
    );
}

fn search_excerpt(content: &str, lowered: &str, query_lower: &str) -> Option<String> {
    let match_start = lowered.find(query_lower)?;
    let match_end = match_start + query_lower.len();
    let mut lowered_offset = 0;
    let mut original_start = None;
    let mut original_end = None;

    for (byte_index, character) in content.char_indices() {
        let lowered_length = character.to_lowercase().map(char::len_utf8).sum::<usize>();
        let lowered_end = lowered_offset + lowered_length;
        if original_start.is_none() && match_start < lowered_end {
            original_start = Some(byte_index);
        }
        if match_end <= lowered_end {
            original_end = Some(byte_index + character.len_utf8());
            break;
        }
        lowered_offset = lowered_end;
    }

    let start = original_start?;
    let end = original_end?;
    let context_start = content[..start]
        .char_indices()
        .rev()
        .nth(SEARCH_PREVIEW_CONTEXT_CHARS)
        .map_or(0, |(index, _)| index);
    let context_end = content[end..]
        .char_indices()
        .nth(SEARCH_PREVIEW_CONTEXT_CHARS - 1)
        .map_or(content.len(), |(index, character)| {
            end + index + character.len_utf8()
        });
    let mut excerpt = String::with_capacity(context_end - context_start + 4);
    if context_start > 0 {
        excerpt.push('…');
    }
    for character in content[context_start..context_end].chars() {
        excerpt.push(if character.is_control() {
            ' '
        } else {
            character
        });
    }
    if context_end < content.len() {
        excerpt.push('…');
    }
    Some(excerpt)
}

impl Store {
    fn connect(&self, write: bool) -> Result<Connection> {
        if let Ok(meta) = fs::symlink_metadata(&self.path) {
            ensure!(
                meta.is_file() && !meta.file_type().is_symlink(),
                "本机资料库不是普通文件"
            );
            ensure!(meta.len() <= MAX_DB_BYTES, "本机资料库超过 320 MiB");
        }
        if write && let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !write && !self.path.exists() {
            anyhow::bail!("资料库尚未保存内容");
        }
        let flags = if write {
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        };
        let db = Connection::open_with_flags(&self.path, flags).context("无法打开本机资料库")?;
        db.busy_timeout(Duration::from_secs(2))?;
        db.execute_batch("PRAGMA foreign_keys=ON;")?;
        let version: i64 = db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        let app_id: i64 = db.query_row("PRAGMA application_id", [], |row| row.get(0))?;
        if version == 0 && write {
            let tables: i64 = db.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
                [],
                |row| row.get(0),
            )?;
            ensure!(
                tables == 0 && app_id == 0,
                "目标文件不是空白资料库，未修改其内容"
            );
            db.execute_batch(
                "PRAGMA auto_vacuum=FULL;
                 BEGIN IMMEDIATE;
                 CREATE TABLE artifacts(
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    revision INTEGER NOT NULL CHECK(revision > 0),
                    updated TEXT NOT NULL
                 );
                 CREATE TABLE versions(
                    artifact_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
                    revision INTEGER NOT NULL CHECK(revision > 0),
                    action TEXT NOT NULL CHECK(action IN ('create','append','replace')),
                    created TEXT NOT NULL,
                    source TEXT NOT NULL,
                    sha256 TEXT NOT NULL,
                    payload BLOB NOT NULL,
                    PRIMARY KEY(artifact_id, revision)
                 );
                 CREATE INDEX versions_created ON versions(created DESC);
                 PRAGMA application_id=1514359890;
                 PRAGMA user_version=1;
                 COMMIT;",
            )?;
        } else {
            ensure!(
                version == 1 && app_id == APPLICATION_ID,
                "资料库格式不兼容，原文件保持不变"
            );
        }
        Ok(db)
    }

    pub fn list(&self, query: &str) -> Result<Vec<Entry>> {
        self.list_filtered(query, None, false)
    }

    pub fn list_trash(&self, query: &str) -> Result<Vec<Entry>> {
        self.list_filtered(query, None, true)
    }

    fn list_filtered(
        &self,
        query: &str,
        kind_filter: Option<Kind>,
        include_trash: bool,
    ) -> Result<Vec<Entry>> {
        self.list_filtered_cancellable(query, kind_filter, include_trash, None)
    }

    fn list_filtered_cancellable(
        &self,
        query: &str,
        kind_filter: Option<Kind>,
        include_trash: bool,
        cancellation: Option<(Arc<AtomicU64>, u64)>,
    ) -> Result<Vec<Entry>> {
        let cancelled = || {
            cancellation.as_ref().is_some_and(|(generation, expected)| {
                generation.load(Ordering::Relaxed) != *expected
            })
        };
        ensure!(!cancelled(), "资料搜索已由新查询取代");
        ensure!(query.chars().count() <= 120, "搜索文字过长");
        ensure!(
            !query.chars().any(char::is_control),
            "搜索文字不能包含控制字符"
        );
        let query = query.trim();
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let db = self.connect(false)?;
        if let Some((generation, expected)) = &cancellation {
            cancel_sqlite_query_when_search_changes(&db, generation, *expected);
        }
        ensure!(!cancelled(), "资料搜索已由新查询取代");
        let has_trash_table = trash_table_exists(&db)?;
        if include_trash && !has_trash_table {
            return Ok(Vec::new());
        }
        let query_lower = query.to_lowercase();
        let kind_filter = kind_filter.map(Kind::as_str);
        let visibility = if include_trash {
            "EXISTS(SELECT 1 FROM artifact_trash t WHERE t.id=a.id)"
        } else if has_trash_table {
            "NOT EXISTS(SELECT 1 FROM artifact_trash t WHERE t.id=a.id)"
        } else {
            "1=1"
        };
        let sql = format!(
            "SELECT a.id,a.name,a.kind,a.revision,a.updated,length(v.payload),v.sha256,
                    (SELECT COUNT(*) FROM versions allv WHERE allv.artifact_id=a.id),v.source,
                    CASE WHEN ?1='' THEN X'' ELSE v.payload END,
                    COALESCE((SELECT group_concat(h.revision || char(31) || h.source, char(10))
                              FROM versions h WHERE h.artifact_id=a.id AND h.revision<>a.revision),'')
             FROM artifacts a JOIN versions v ON v.artifact_id=a.id AND v.revision=a.revision
             WHERE {visibility} AND (?2 IS NULL OR a.kind=?2)
             ORDER BY a.updated DESC,a.id LIMIT 513"
        );
        let mut statement = db.prepare(&sql)?;
        let rows = statement.query_map(params![query, kind_filter], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, Vec<u8>>(9)?,
                row.get::<_, String>(10)?,
            ))
        })?;
        let mut entries = Vec::new();
        let mut count = 0;
        for row in rows {
            ensure!(!cancelled(), "资料搜索已由新查询取代");
            count += 1;
            ensure!(count <= MAX_ARTIFACTS as usize, "资料条目超过 512 项");
            let (
                id,
                name,
                kind,
                revision,
                updated,
                bytes,
                sha256,
                versions,
                source,
                payload,
                historical_sources,
            ) = row?;
            validate_id(&id)?;
            validate_name(&name)?;
            ensure!(
                revision > 0 && bytes >= 0 && versions == revision,
                "资料元数据无效"
            );
            let kind = Kind::parse(&kind)?;
            if query_lower.is_empty() {
                entries.push(Entry {
                    id,
                    name,
                    kind,
                    revision,
                    updated,
                    bytes,
                    sha256,
                    versions,
                    source,
                    historical_source_match: None,
                    search_preview: None,
                });
                continue;
            }
            ensure!(
                payload.len() as i64 == bytes
                    && payload.len() <= MAX_VERSION_BYTES
                    && digest(&payload) == sha256,
                "资料内容校验失败"
            );
            let search_preview = if kind == Kind::Image {
                None
            } else {
                let content = String::from_utf8(payload).context("资料内容不是 UTF-8 文本")?;
                let lowered_content = content.to_lowercase();
                search_excerpt(&content, &lowered_content, &query_lower)
            };
            let source_match = source.to_lowercase().contains(&query_lower);
            let historical_source_match = historical_sources
                .split('\n')
                .filter_map(|value| {
                    let (revision, historical_source) = value.split_once('\u{1f}')?;
                    let revision = revision.parse::<i64>().ok()?;
                    historical_source
                        .to_lowercase()
                        .contains(&query_lower)
                        .then(|| (revision, historical_source.to_owned()))
                })
                .max_by_key(|(revision, _)| *revision);
            let matches = name.to_lowercase().contains(&query_lower)
                || source_match
                || historical_source_match.is_some()
                || search_preview.is_some();
            if matches {
                entries.push(Entry {
                    id,
                    name,
                    kind,
                    revision,
                    updated,
                    bytes,
                    sha256,
                    versions,
                    source,
                    historical_source_match,
                    search_preview,
                });
            }
        }
        ensure!(!cancelled(), "资料搜索已由新查询取代");
        Ok(entries)
    }

    pub fn load(&self, id: &str, revision: i64) -> Result<Version> {
        validate_id(id)?;
        ensure!(revision > 0, "资料版本无效");
        let db = self.connect(false)?;
        let version = db.query_row(
            "SELECT action,created,source,sha256,payload FROM versions WHERE artifact_id=?1 AND revision=?2",
            params![id, revision],
            |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?,row.get::<_, String>(2)?,row.get::<_, String>(3)?,row.get::<_, Vec<u8>>(4)?)),
        )?;
        let action = VersionAction::parse(&version.0)?;
        validate_source(&version.2)?;
        ensure!(
            version.4.len() <= MAX_VERSION_BYTES && digest(&version.4) == version.3,
            "资料内容校验失败"
        );
        Ok(Version {
            revision,
            action,
            created: version.1,
            source: version.2,
            sha256: version.3,
            bytes: version.4,
        })
    }

    pub fn versions(&self, id: &str) -> Result<Vec<VersionSummary>> {
        validate_id(id)?;
        let db = self.connect(false)?;
        let mut statement = db.prepare(
            "SELECT revision,action,created,length(payload),sha256,source
             FROM versions WHERE artifact_id=?1 ORDER BY revision DESC LIMIT 257",
        )?;
        let versions = statement
            .query_map([id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ensure!(
            versions.len() <= MAX_VERSIONS_PER_ARTIFACT as usize,
            "资料版本超过上限"
        );
        versions
            .into_iter()
            .map(|(revision, action, created, bytes, sha256, source)| {
                ensure!(revision > 0 && bytes >= 0, "资料版本元数据无效");
                validate_source(&source)?;
                Ok(VersionSummary {
                    revision,
                    action: VersionAction::parse(&action)?,
                    created,
                    bytes,
                    sha256,
                    source,
                })
            })
            .collect()
    }

    pub fn compare_with_current(&self, id: &str, revision: i64) -> Result<(i64, String)> {
        validate_id(id)?;
        ensure!(revision > 0, "资料版本无效");
        let mut db = self.connect(false)?;
        let tx = db.transaction()?;
        let current_revision: Option<i64> = tx
            .query_row("SELECT revision FROM artifacts WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .optional()?;
        let current_revision = current_revision.context("资料不存在或已永久删除")?;
        let kind: String = tx.query_row("SELECT kind FROM artifacts WHERE id=?1", [id], |row| {
            row.get(0)
        })?;
        ensure!(
            Kind::parse(&kind)? != Kind::Image,
            "PNG 图片版本不支持逐行文本比较"
        );
        let previous: (Vec<u8>, String) = tx.query_row(
            "SELECT payload,sha256 FROM versions WHERE artifact_id=?1 AND revision=?2",
            params![id, revision],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let current: (Vec<u8>, String) = tx.query_row(
            "SELECT payload,sha256 FROM versions WHERE artifact_id=?1 AND revision=?2",
            params![id, current_revision],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        ensure!(
            previous.0.len() + current.0.len() <= MAX_DIFF_INPUT_BYTES,
            "两个版本合计超过 2 MiB；为保持界面响应，本次只支持查看版本内容，不生成逐行差异"
        );
        ensure!(
            digest(&previous.0) == previous.1 && digest(&current.0) == current.1,
            "资料内容校验失败"
        );
        let previous = std::str::from_utf8(&previous.0).context("历史版本不是 UTF-8 文本")?;
        let current = std::str::from_utf8(&current.0).context("当前版本不是 UTF-8 文本")?;
        let mut output = String::new();
        let mut changed = false;
        for change in TextDiff::from_lines(previous, current).iter_all_changes() {
            let mark = match change.tag() {
                ChangeTag::Delete => {
                    changed = true;
                    '-'
                }
                ChangeTag::Insert => {
                    changed = true;
                    '+'
                }
                ChangeTag::Equal => ' ',
            };
            let mut line = String::from(mark);
            line.push_str(&change.to_string());
            if output.len() + line.len() > MAX_DIFF_OUTPUT_BYTES {
                const TRUNCATED: &str = "\n…差异已截断";
                let body_limit = MAX_DIFF_OUTPUT_BYTES - TRUNCATED.len();
                if output.len() > body_limit {
                    let mut cut = body_limit;
                    while !output.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    output.truncate(cut);
                } else {
                    let mut cut = (body_limit - output.len()).min(line.len());
                    while !line.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    output.push_str(&line[..cut]);
                }
                output.push_str(TRUNCATED);
                break;
            }
            output.push_str(&line);
        }
        if !changed {
            output.clear();
        }
        tx.commit()?;
        Ok((current_revision, output))
    }

    pub fn create(&self, name: &str, kind: Kind, payload: &[u8], source: &str) -> Result<Entry> {
        validate_name(name)?;
        validate_source(source)?;
        kind.validate(payload)?;
        let mut db = self.connect(true)?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (count, total): (i64, i64) = tx.query_row(
            "SELECT (SELECT COUNT(*) FROM artifacts),COALESCE((SELECT SUM(length(payload)) FROM versions),0)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        ensure!(count < MAX_ARTIFACTS, "资料库最多保存 512 项");
        ensure!(
            total + payload.len() as i64 <= MAX_TOTAL_BYTES,
            "资料库最多占用 256 MiB，请检查并删除不需要的资料"
        );
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let sha256 = digest(payload);
        tx.execute(
            "INSERT INTO artifacts(id,name,kind,revision,updated) VALUES(?1,?2,?3,1,?4)",
            params![id, name.trim(), kind.as_str(), now],
        )?;
        tx.execute(
            "INSERT INTO versions(artifact_id,revision,action,created,source,sha256,payload) VALUES(?1,1,'create',?2,?3,?4,?5)",
            params![id, now, source, sha256, payload],
        )?;
        tx.commit()?;
        Ok(Entry {
            id,
            name: name.trim().to_owned(),
            kind,
            revision: 1,
            updated: now,
            bytes: payload.len() as i64,
            sha256,
            versions: 1,
            source: source.to_owned(),
            historical_source_match: None,
            search_preview: None,
        })
    }

    pub fn add_version(
        &self,
        id: &str,
        expected_revision: i64,
        kind: Kind,
        payload: &[u8],
        source: &str,
        action: VersionAction,
    ) -> Result<Entry> {
        validate_id(id)?;
        validate_source(source)?;
        ensure!(
            matches!(action, VersionAction::Append | VersionAction::Replace),
            "新增资料必须使用新建操作"
        );
        kind.validate(payload)?;
        let mut db = self.connect(true)?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if trash_table_exists(&tx)? {
            let is_trashed: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM artifact_trash WHERE id=?1)",
                [id],
                |row| row.get(0),
            )?;
            ensure!(!is_trashed, "资料已在回收站；恢复后再编辑");
        }
        let current: Option<(String, String, i64)> = tx
            .query_row(
                "SELECT name,kind,revision FROM artifacts WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((name, kind_str, current_revision)) = current else {
            anyhow::bail!("资料已删除或不存在，请刷新后重试");
        };
        ensure!(
            current_revision == expected_revision,
            "资料在预览后已变化；未写入，请重新载入并审核"
        );
        ensure!(
            Kind::parse(&kind_str)? == kind,
            "资料类型不可在版本更新时改变"
        );
        let version_count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM versions WHERE artifact_id=?1",
            [id],
            |row| row.get(0),
        )?;
        ensure!(
            version_count < MAX_VERSIONS_PER_ARTIFACT,
            "单项资料最多保存 256 个版本"
        );
        let total: i64 = tx.query_row(
            "SELECT COALESCE(SUM(length(payload)),0) FROM versions",
            [],
            |row| row.get(0),
        )?;
        ensure!(
            total + payload.len() as i64 <= MAX_TOTAL_BYTES,
            "资料库最多占用 256 MiB，请检查并删除不需要的资料"
        );
        let revision = current_revision
            .checked_add(1)
            .context("资料版本号已到上限")?;
        let now = chrono::Utc::now().to_rfc3339();
        let sha256 = digest(payload);
        tx.execute(
            "INSERT INTO versions(artifact_id,revision,action,created,source,sha256,payload) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![id, revision, action.as_str(), now, source, sha256, payload],
        )?;
        ensure!(
            tx.execute(
                "UPDATE artifacts SET revision=?2,updated=?3 WHERE id=?1 AND revision=?4",
                params![id, revision, now, expected_revision],
            )? == 1,
            "资料在预览后已变化；未写入，请重新载入并审核"
        );
        tx.commit()?;
        Ok(Entry {
            id: id.to_owned(),
            name,
            kind,
            revision,
            updated: now,
            bytes: payload.len() as i64,
            sha256,
            versions: version_count + 1,
            source: source.to_owned(),
            historical_source_match: None,
            search_preview: None,
        })
    }

    pub fn delete(&self, id: &str, expected_revision: i64) -> Result<()> {
        validate_id(id)?;
        ensure!(expected_revision > 0, "资料版本无效");
        let db = self.connect(true)?;
        let tx = db.unchecked_transaction()?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS artifact_trash(
                 id TEXT PRIMARY KEY REFERENCES artifacts(id) ON DELETE CASCADE,
                 deleted TEXT NOT NULL
             );",
        )?;
        ensure!(
            tx.execute(
                "INSERT INTO artifact_trash(id,deleted)
                 SELECT id,?3 FROM artifacts WHERE id=?1 AND revision=?2
                 ON CONFLICT(id) DO NOTHING",
                params![id, expected_revision, chrono::Utc::now().to_rfc3339()],
            )? == 1,
            "资料在确认后已变化、已删除或已在回收站；请刷新后重试"
        );
        tx.commit()?;
        Ok(())
    }

    pub fn restore(&self, id: &str, expected_revision: i64) -> Result<()> {
        validate_id(id)?;
        ensure!(expected_revision > 0, "资料版本无效");
        let db = self.connect(true)?;
        ensure!(trash_table_exists(&db)?, "回收站中没有该资料");
        ensure!(
            db.execute(
                "DELETE FROM artifact_trash
                 WHERE id=?1 AND EXISTS(SELECT 1 FROM artifacts a WHERE a.id=?1 AND a.revision=?2)",
                params![id, expected_revision],
            )? == 1,
            "资料版本已变化或不在回收站；请刷新后重试"
        );
        Ok(())
    }

    pub fn purge(&self, id: &str, expected_revision: i64) -> Result<()> {
        validate_id(id)?;
        ensure!(expected_revision > 0, "资料版本无效");
        let db = self.connect(true)?;
        ensure!(trash_table_exists(&db)?, "回收站中没有该资料");
        ensure!(
            db.execute(
                "DELETE FROM artifacts
                 WHERE id=?1 AND revision=?2
                   AND EXISTS(SELECT 1 FROM artifact_trash t WHERE t.id=artifacts.id)",
                params![id, expected_revision],
            )? == 1,
            "资料版本已变化或不在回收站；请刷新后重试"
        );
        Ok(())
    }
}

struct PendingSave {
    id: Option<String>,
    expected_revision: i64,
    name: String,
    kind: Kind,
    payload: Vec<u8>,
    source: String,
    action: VersionAction,
}

/// UI draft state is transient. The library file is created only after an explicit save.
pub struct State {
    store: Store,
    entries: Vec<Entry>,
    selected: Option<Entry>,
    versions: Vec<VersionSummary>,
    view_revision: i64,
    comparison: Option<(i64, i64, String)>,
    draft_name: String,
    draft_kind: Kind,
    draft_text: String,
    draft_image: Option<crate::image_tools::ImageMaterial>,
    draft_image_texture: Option<egui::TextureHandle>,
    draft_source: String,
    dirty: bool,
    query: String,
    kind_filter: Option<Kind>,
    show_trash: bool,
    selected_in_trash: bool,
    search_requests: Sender<SearchRequest>,
    search_results: Receiver<SearchResult>,
    search_generation: u64,
    search_cancel_generation: Arc<AtomicU64>,
    search_pending_at: Option<Instant>,
    searching: bool,
    pending: Option<PendingSave>,
    pending_delete: Option<Entry>,
    pending_purge: bool,
    error: String,
    notice: String,
}

fn search_worker(
    store: Store,
    search_cancel_generation: Arc<AtomicU64>,
) -> (Sender<SearchRequest>, Receiver<SearchResult>) {
    let (request_tx, request_rx) = mpsc::channel::<SearchRequest>();
    let (result_tx, result_rx) = mpsc::channel::<SearchResult>();
    std::thread::spawn(move || {
        while let Ok(mut request) = request_rx.recv() {
            while let Ok(latest) = request_rx.try_recv() {
                request = latest;
            }
            let result = store
                .list_filtered_cancellable(
                    &request.1,
                    request.2,
                    request.3,
                    Some((Arc::clone(&search_cancel_generation), request.0)),
                )
                .map_err(|error| error.to_string());
            if result_tx.send((request.0, result)).is_err() {
                break;
            }
        }
    });
    (request_tx, result_rx)
}

impl State {
    pub fn new(path: PathBuf) -> Self {
        let store = Store { path };
        let search_cancel_generation = Arc::new(AtomicU64::new(0));
        let (search_requests, search_results) =
            search_worker(store.clone(), Arc::clone(&search_cancel_generation));
        let mut state = Self {
            store,
            entries: Vec::new(),
            selected: None,
            versions: Vec::new(),
            view_revision: 0,
            comparison: None,
            draft_name: String::new(),
            draft_kind: Kind::Text,
            draft_text: String::new(),
            draft_image: None,
            draft_image_texture: None,
            draft_source: "本机编辑".into(),
            dirty: false,
            query: String::new(),
            kind_filter: None,
            show_trash: false,
            selected_in_trash: false,
            search_requests,
            search_results,
            search_generation: 0,
            search_cancel_generation,
            search_pending_at: None,
            searching: false,
            pending: None,
            pending_delete: None,
            pending_purge: false,
            error: String::new(),
            notice: String::new(),
        };
        state.refresh();
        state
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_trash_fixture(&mut self) -> Result<()> {
        let entry = self.store.create(
            "回收站恢复验收",
            Kind::Text,
            b"source remains intact",
            "本机编辑",
        )?;
        self.store.delete(&entry.id, entry.revision)?;
        self.show_trash = true;
        self.refresh();
        let trashed = self
            .entries
            .first()
            .cloned()
            .context("临时资料没有进入回收站")?;
        self.load_entry(trashed, entry.revision);
        Ok(())
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_trash_state(&self) -> (bool, bool, usize, usize, usize) {
        (
            self.show_trash,
            self.selected_in_trash,
            self.entries.len(),
            self.store
                .list("")
                .map_or(usize::MAX, |entries| entries.len()),
            self.store
                .list_trash("")
                .map_or(usize::MAX, |entries| entries.len()),
        )
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_version_diff_fixture(&mut self) -> Result<()> {
        let entry = self.store.create(
            "版本差异验收",
            Kind::Text,
            b"unchanged\nold line\n",
            "本机编辑",
        )?;
        let updated = self.store.add_version(
            &entry.id,
            1,
            Kind::Text,
            b"unchanged\nnew line\nadded line\n",
            "本机编辑",
            VersionAction::Append,
        )?;
        self.refresh();
        self.load_entry(updated, 1);
        Ok(())
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_version_diff_state(&self) -> Option<(i64, i64, bool)> {
        self.comparison
            .as_ref()
            .map(|(from, current, diff)| (*from, *current, diff.contains("+new line")))
    }

    /// Receive a copy for review; no library file is opened or written here.
    pub fn receive_text(
        &mut self,
        text: &str,
        kind: Kind,
        provenance: Option<&crate::tool_provenance::ToolProvenance>,
    ) -> Result<()> {
        self.receive_text_with_reference(text, kind, provenance, None)
    }

    pub fn receive_text_with_reference(
        &mut self,
        text: &str,
        kind: Kind,
        provenance: Option<&crate::tool_provenance::ToolProvenance>,
        source_reference: Option<&str>,
    ) -> Result<()> {
        ensure!(text.len() <= MAX_VERSION_BYTES, "接力材料超过 16 MiB");
        ensure!(!self.dirty, "当前草稿尚未保存；请先保存或明确放弃草稿");
        if self.show_trash {
            self.show_trash = false;
            self.refresh();
        }
        self.selected = None;
        self.selected_in_trash = false;
        self.versions.clear();
        self.view_revision = 0;
        self.comparison = None;
        self.draft_name = "接力结果".into();
        self.draft_kind = kind;
        self.draft_text = text.to_owned();
        self.draft_image = None;
        self.draft_image_texture = None;
        if let Some(provenance) = provenance {
            provenance.validate()?;
            self.draft_source = format!("{}@{}", provenance.tool_id, provenance.tool_version);
        } else if let Some(source) = source_reference {
            validate_source(source)?;
            self.draft_source = source.to_owned();
        } else {
            self.draft_source = "工具接力".into();
        }
        self.dirty = true;
        self.pending = None;
        self.pending_delete = None;
        self.error.clear();
        self.notice = "接力内容只在当前草稿中；尚未写入本机资料库。".into();
        Ok(())
    }

    /// Receive image pixels for review. The library is written only after explicit save confirmation.
    pub fn receive_image_with_reference(
        &mut self,
        image: crate::image_tools::ImageMaterial,
        provenance: Option<&crate::tool_provenance::ToolProvenance>,
        source_reference: Option<&str>,
    ) -> Result<()> {
        ensure!(!self.dirty, "当前草稿尚未保存；请先保存或明确放弃草稿");
        ensure!(
            image.image.width() > 0
                && image.image.height() > 0
                && image.image.width() <= 12_000
                && image.image.height() <= 12_000
                && u64::from(image.image.width()) * u64::from(image.image.height())
                    <= MAX_IMAGE_PIXELS
                && image.image.as_bytes().len() <= 192 * 1024 * 1024,
            "图片尺寸或内存超过资料库限制"
        );
        let source = if let Some(provenance) = provenance {
            provenance.validate()?;
            format!("{}@{}", provenance.tool_id, provenance.tool_version)
        } else if let Some(source) = source_reference {
            validate_source(source)?;
            source.to_owned()
        } else {
            "工具接力".to_owned()
        };
        if self.show_trash {
            self.show_trash = false;
            self.refresh();
        }
        self.selected = None;
        self.selected_in_trash = false;
        self.versions.clear();
        self.view_revision = 0;
        self.comparison = None;
        self.draft_name = "图片接力结果".into();
        self.draft_kind = Kind::Image;
        self.draft_text.clear();
        self.draft_image = Some(image);
        self.draft_image_texture = None;
        self.draft_source = source;
        self.dirty = true;
        self.pending = None;
        self.pending_delete = None;
        self.error.clear();
        self.notice = "图片只在当前草稿中；预览并确认保存后，才会写入本机资料库。".into();
        Ok(())
    }

    /// Return the currently visible text as an ephemeral handoff snapshot.
    /// The caller still opens the normal preview/target dialog; this never saves data.
    pub fn handoff_snapshot(&self) -> Option<(String, String)> {
        if self.draft_kind == Kind::Image || self.draft_text.is_empty() {
            return None;
        }
        let source = match &self.selected {
            Some(entry) => format!(
                "本机资料「{}」· v{}{} · 来源 {}",
                entry.name,
                self.view_revision,
                if self.dirty {
                    "（未保存编辑）"
                } else {
                    ""
                },
                self.draft_source
            ),
            None => format!(
                "资料草稿「{}」{} · 来源 {}",
                if self.draft_name.trim().is_empty() {
                    "未命名"
                } else {
                    self.draft_name.trim()
                },
                if self.dirty { "（未保存）" } else { "" },
                self.draft_source
            ),
        };
        Some((source, self.draft_text.clone()))
    }

    /// Return image pixels as an ephemeral typed handoff; this never writes the library.
    pub fn handoff_image_snapshot(&self) -> Option<(String, crate::image_tools::ImageMaterial)> {
        if self.draft_kind != Kind::Image {
            return None;
        }
        let image = self.draft_image.clone()?;
        let source = match &self.selected {
            Some(entry) => format!(
                "本机资料「{}」· v{}{} · 来源 {}",
                entry.name,
                self.view_revision,
                if self.dirty {
                    "（未保存编辑）"
                } else {
                    ""
                },
                self.draft_source
            ),
            None => format!(
                "图片草稿「{}」{} · 来源 {}",
                if self.draft_name.trim().is_empty() {
                    "未命名"
                } else {
                    self.draft_name.trim()
                },
                if self.dirty { "（未保存）" } else { "" },
                self.draft_source
            ),
        };
        Some((source, image))
    }

    pub fn handoff_source_reference(&self) -> Option<String> {
        if self.draft_source.contains('@') && validate_source(&self.draft_source).is_ok() {
            Some(self.draft_source.clone())
        } else {
            None
        }
    }

    /// Return a validated table snapshot for typed cross-tool handoff.
    /// This reads only the current in-memory draft and never saves it.
    pub fn handoff_table_snapshot(&self) -> Result<Option<crate::workbench::Dataset>> {
        if self.draft_kind != Kind::Table {
            return Ok(None);
        }
        let dataset: crate::workbench::Dataset =
            serde_json::from_str(&self.draft_text).context("结构化表格内容无效，无法接力")?;
        crate::workbench::Dataset::from_parts(dataset.headers, dataset.rows)
            .context("结构化表格结构无效，无法接力")
            .map(Some)
    }

    fn refresh(&mut self) {
        self.search_generation = self.search_generation.wrapping_add(1);
        self.search_cancel_generation
            .store(self.search_generation, Ordering::Relaxed);
        self.search_pending_at = None;
        if self.query.trim().is_empty() && self.kind_filter.is_none() {
            self.searching = false;
            let result = if self.show_trash {
                self.store.list_trash("")
            } else {
                self.store.list("")
            };
            match result {
                Ok(entries) => {
                    self.entries = entries;
                    self.error.clear();
                }
                Err(error) => self.error = error.to_string(),
            }
        } else {
            self.searching = true;
            if self
                .search_requests
                .send((
                    self.search_generation,
                    self.query.clone(),
                    self.kind_filter,
                    self.show_trash,
                ))
                .is_err()
            {
                self.searching = false;
                self.error = "资料搜索后台不可用".into();
            }
        }
    }

    fn schedule_search(&mut self) {
        self.search_generation = self.search_generation.wrapping_add(1);
        self.search_cancel_generation
            .store(self.search_generation, Ordering::Relaxed);
        self.search_pending_at = Some(Instant::now());
        self.searching = true;
    }

    fn poll_search(&mut self, ctx: &egui::Context) {
        while let Ok((generation, result)) = self.search_results.try_recv() {
            if generation == self.search_generation {
                self.searching = false;
                match result {
                    Ok(entries) => {
                        self.entries = entries;
                        self.error.clear();
                    }
                    Err(error) => self.error = error,
                }
            }
        }
        if let Some(started) = self.search_pending_at {
            let delay = Duration::from_millis(250);
            let elapsed = started.elapsed();
            if elapsed >= delay {
                self.search_pending_at = None;
                self.refresh();
            } else {
                ctx.request_repaint_after(delay - elapsed);
            }
        }
        if self.searching {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn new_draft(&mut self) {
        self.selected = None;
        self.selected_in_trash = false;
        self.versions.clear();
        self.view_revision = 0;
        self.comparison = None;
        self.draft_name.clear();
        self.draft_kind = Kind::Text;
        self.draft_text.clear();
        self.draft_image = None;
        self.draft_image_texture = None;
        self.draft_source = "本机编辑".into();
        self.dirty = false;
        self.pending = None;
        self.pending_delete = None;
        self.error.clear();
        self.notice = "新草稿仅保留在内存中，确认保存后才会写入本机。".into();
    }

    fn load_entry(&mut self, entry: Entry, revision: i64) {
        if self.dirty {
            self.error = "当前草稿尚未保存；先保存或新建草稿后再切换资料。".into();
            return;
        }
        match self.store.load(&entry.id, revision).and_then(|version| {
            if entry.kind == Kind::Image {
                let image = decode_png(&version.bytes)?;
                Ok((
                    String::new(),
                    Some(crate::image_tools::ImageMaterial {
                        image: std::sync::Arc::new(image),
                        origins: Vec::new(),
                        kind: "本机资料库 PNG",
                    }),
                    version.source,
                ))
            } else {
                let text = String::from_utf8(version.bytes).context("资料不是 UTF-8 文本")?;
                Ok((text, None, version.source))
            }
        }) {
            Ok((text, image, source)) => match self.store.versions(&entry.id) {
                Ok(versions) => {
                    self.versions = versions;
                    self.view_revision = revision;
                    self.comparison = None;
                    self.draft_name.clone_from(&entry.name);
                    self.draft_kind = entry.kind;
                    self.draft_text = text;
                    self.draft_image = image;
                    self.draft_image_texture = None;
                    self.draft_source = source;
                    self.dirty = false;
                    self.selected = Some(entry);
                    self.selected_in_trash = self.show_trash;
                    self.pending = None;
                    self.pending_delete = None;
                    self.error.clear();
                    self.notice = format!("已打开第 {revision} 版；编辑后需明确确认保存新版本。",);
                }
                Err(error) => self.error = error.to_string(),
            },
            Err(error) => self.error = error.to_string(),
        }
    }

    fn confirm_save(&mut self) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        let result = if let Some(id) = &pending.id {
            self.store.add_version(
                id,
                pending.expected_revision,
                pending.kind,
                &pending.payload,
                &pending.source,
                pending.action,
            )
        } else {
            self.store.create(
                &pending.name,
                pending.kind,
                &pending.payload,
                &pending.source,
            )
        };
        match result {
            Ok(entry) => {
                self.selected = Some(entry.clone());
                self.draft_name.clone_from(&entry.name);
                self.draft_kind = entry.kind;
                if pending.kind == Kind::Image {
                    self.draft_text.clear();
                } else {
                    self.draft_text = String::from_utf8(pending.payload)
                        .expect("the UI stores only valid UTF-8 text");
                    self.draft_image = None;
                }
                self.draft_image_texture = None;
                self.draft_source = pending.source.clone();
                self.dirty = false;
                self.versions = self.store.versions(&entry.id).unwrap_or_default();
                self.view_revision = entry.revision;
                self.comparison = None;
                self.refresh();
                self.notice = format!("已保存第 {} 版到本机资料库。", entry.revision);
                self.error.clear();
            }
            Err(error) => {
                self.pending = Some(pending);
                self.error = error.to_string();
            }
        }
    }

    fn confirm_delete(&mut self) {
        let Some(entry) = self.pending_delete.take() else {
            return;
        };
        let result = if self.pending_purge {
            self.store.purge(&entry.id, entry.revision)
        } else {
            self.store.delete(&entry.id, entry.revision)
        };
        match result {
            Ok(()) => {
                self.selected = None;
                self.selected_in_trash = false;
                self.show_trash = self.pending_purge;
                self.versions.clear();
                self.view_revision = 0;
                self.comparison = None;
                self.draft_name.clear();
                self.draft_text.clear();
                self.draft_image = None;
                self.draft_image_texture = None;
                self.draft_kind = Kind::Text;
                self.dirty = false;
                self.refresh();
                self.notice = if self.pending_purge {
                    format!("已永久删除“{}”及其全部历史版本。", entry.name)
                } else {
                    format!("已将“{}”移入回收站，可在回收站中恢复。", entry.name)
                };
                self.error.clear();
                self.pending_purge = false;
            }
            Err(error) => {
                self.pending_delete = Some(entry);
                self.error = error.to_string();
            }
        }
    }

    fn restore_selected(&mut self) {
        let Some(entry) = self.selected.clone() else {
            return;
        };
        if !self.selected_in_trash || self.dirty {
            self.error = "请先选中回收站资料且处理未保存草稿。".into();
            return;
        }
        match self.store.restore(&entry.id, entry.revision) {
            Ok(()) => {
                self.selected = None;
                self.selected_in_trash = false;
                self.show_trash = false;
                self.versions.clear();
                self.view_revision = 0;
                self.draft_name.clear();
                self.draft_text.clear();
                self.draft_image = None;
                self.draft_image_texture = None;
                self.dirty = false;
                self.refresh();
                self.notice = format!("已恢复“{}”到本机资料列表。", entry.name);
                self.error.clear();
            }
            Err(error) => self.error = error.to_string(),
        }
    }

    fn set_trash_view(&mut self, show_trash: bool) {
        if self.show_trash == show_trash {
            return;
        }
        if self.dirty {
            self.error = "草稿尚未保存；先保存或放弃草稿后再切换资料列表。".into();
            return;
        }
        self.show_trash = show_trash;
        self.selected = None;
        self.selected_in_trash = false;
        self.versions.clear();
        self.view_revision = 0;
        self.comparison = None;
        self.draft_name.clear();
        self.draft_text.clear();
        self.draft_image = None;
        self.draft_image_texture = None;
        self.draft_kind = Kind::Text;
        self.pending = None;
        self.pending_delete = None;
        self.pending_purge = false;
        self.refresh();
        self.error.clear();
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.poll_search(ui.ctx());
        ui.heading("类型化资料与结果");
        ui.label("工具输出先作为临时草稿接力；只有你确认保存后，内容才会写入本机资料库。");
        ui.small(format!(
            "本机文件：{} · 单版上限 16 MiB · 总内容上限 256 MiB",
            self.store.path.display()
        ));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let mut requested_trash_view = self.show_trash;
            ui.selectable_value(&mut requested_trash_view, false, "资料列表");
            ui.selectable_value(&mut requested_trash_view, true, "回收站");
            if requested_trash_view != self.show_trash {
                self.set_trash_view(requested_trash_view);
            }
            ui.label("搜索");
            let mut changed = ui
                .add(
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text("名称、来源或当前/历史版本"),
                )
                .changed();
            ui.label("类型");
            egui::ComboBox::from_id_salt("artifact-search-kind")
                .selected_text(self.kind_filter.map_or("全部类型", Kind::label))
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(&mut self.kind_filter, None, "全部类型")
                        .changed();
                    for kind in [
                        Kind::Text,
                        Kind::Json,
                        Kind::Csv,
                        Kind::Tsv,
                        Kind::Table,
                        Kind::Image,
                    ] {
                        changed |= ui
                            .selectable_value(&mut self.kind_filter, Some(kind), kind.label())
                            .changed();
                    }
                });
            if changed {
                self.schedule_search();
            }
            if ui.button("刷新").clicked() {
                self.refresh();
            }
            if ui
                .add_enabled(!self.show_trash, egui::Button::new("新建草稿"))
                .clicked()
            {
                if self.dirty {
                    self.error = "草稿尚未保存；请先保存，或选中后清空内容再新建。".into();
                } else {
                    self.new_draft();
                }
            }
        });
        ui.small(
            "搜索名称、任一历史版本的来源工具/版本或当前版正文；图片内容不做 OCR，搜索和筛选在后台执行。",
        );
        if self.searching {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.small("正在搜索名称、来源与支持的文本正文…");
            });
        }
        ui.add_space(8.0);
        ui.columns(2, |columns| {
            columns[0].set_min_width(230.0);
            columns[0].heading(format!(
                "{} · {} 项",
                if self.show_trash {
                    "回收站"
                } else {
                    "本机资料"
                },
                self.entries.len()
            ));
            egui::ScrollArea::vertical()
                .id_salt("artifact-list")
                .max_height(420.0)
                .show(&mut columns[0], |ui| {
                    let entries = self.entries.clone();
                    for entry in entries {
                        let selected = self
                            .selected
                            .as_ref()
                            .is_some_and(|current| current.id == entry.id);
                        let label = format!(
                            "{}\n{} · v{} · {} B\n当前来源：{}",
                            entry.name,
                            entry.kind.label(),
                            entry.revision,
                            entry.bytes,
                            entry.source
                        );
                        let label = entry.historical_source_match.as_ref().map_or(
                            label.clone(),
                            |(revision, source)| {
                                format!("{label}\n历史来源匹配：v{revision} · {source}")
                            },
                        );
                        let label = entry
                            .search_preview
                            .as_ref()
                            .map_or(label.clone(), |preview| format!("{label}\n{preview}"));
                        let response = ui.selectable_label(selected, label);
                        if response.clicked() {
                            self.load_entry(entry.clone(), entry.revision);
                        }
                    }
                });
            columns[1].heading(if self.selected.is_some() {
                "资料内容"
            } else {
                "编辑草稿"
            });
            if self.selected_in_trash {
                columns[1]
                    .small("回收站内容为只读；恢复后可继续编辑，永久删除会移除资料及全部版本。");
                columns[1].horizontal(|ui| {
                    let restore = ui.button("恢复到资料列表");
                    #[cfg(feature = "ui-preview")]
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(
                            egui::Id::new("artifact-trash-restore"),
                            restore.rect.intersect(ui.clip_rect()),
                        )
                    });
                    if restore.clicked() {
                        self.restore_selected();
                    }
                    if ui.button("永久删除…").clicked() {
                        self.pending_delete = self.selected.clone();
                        self.pending_purge = true;
                        self.pending = None;
                    }
                });
            }
            columns[1].add_enabled_ui(
                self.pending.is_none()
                    && self.pending_delete.is_none()
                    && !self.selected_in_trash
                    && !self.show_trash,
                |ui| {
                    ui.horizontal(|ui| {
                        ui.label("名称");
                        let changed = ui
                            .add_enabled(
                                self.selected.is_none(),
                                egui::TextEdit::singleline(&mut self.draft_name),
                            )
                            .changed();
                        if changed {
                            self.mark_dirty();
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("类型");
                        egui::ComboBox::from_id_salt("artifact-kind")
                            .selected_text(self.draft_kind.label())
                            .show_ui(ui, |ui| {
                                for kind in [Kind::Text, Kind::Json, Kind::Csv, Kind::Tsv, Kind::Table] {
                                    if ui
                                        .selectable_value(&mut self.draft_kind, kind, kind.label())
                                        .changed()
                                    {
                                        self.draft_image = None;
                                        self.draft_image_texture = None;
                                        self.mark_dirty();
                                    }
                                }
                            });
                        if let Some(entry) = &self.selected {
                            ui.label(format!(
                                "当前版 v{} · 共 {} 版",
                                entry.revision, entry.versions
                            ));
                        }
                    });
                    if self.selected.is_some() && !self.versions.is_empty() {
                        let mut revision = self.view_revision;
                        egui::ComboBox::from_id_salt("artifact-version")
                            .selected_text(format!("查看第 {revision} 版"))
                            .show_ui(ui, |ui| {
                                for version in &self.versions {
                                    ui.selectable_value(
                                        &mut revision,
                                        version.revision,
                                        format!(
                                            "v{} · {} · {}",
                                            version.revision,
                                            version.action.label(),
                                            version.created
                                        ),
                                    );
                                }
                            });
                        if revision != self.view_revision {
                            let entry = self.selected.as_ref().unwrap().clone();
                            self.load_entry(entry, revision);
                        }
                        if self.draft_kind != Kind::Image
                            && self.view_revision != self.selected.as_ref().unwrap().revision
                            && !self.dirty
                            && {
                                let compare = ui.button("与当前版比较");
                                #[cfg(feature = "ui-preview")]
                                ui.ctx().data_mut(|data| {
                                    data.insert_temp(
                                        egui::Id::new("artifact-version-compare"),
                                        compare.rect.intersect(ui.clip_rect()),
                                    )
                                });
                                compare.clicked()
                            }
                        {
                            let entry = self.selected.as_ref().unwrap().clone();
                            match self
                                .store
                                .compare_with_current(&entry.id, self.view_revision)
                            {
                                Ok((current_revision, diff)) => {
                                    self.comparison =
                                        Some((self.view_revision, current_revision, diff));
                                    self.error.clear();
                                }
                                Err(error) => self.error = error.to_string(),
                            }
                        }
                    }
                    let draft_size_ok;
                    if self.draft_kind == Kind::Image {
                        if self.draft_image_texture.is_none()
                            && let Some(image) = &self.draft_image
                        {
                            self.draft_image_texture = Some(ui.ctx().load_texture(
                                "artifact-hub-image-preview",
                                image_preview(&image.image),
                                egui::TextureOptions::LINEAR,
                            ));
                        }
                        if let Some(image) = &self.draft_image {
                            ui.horizontal(|ui| {
                                ui.label(format!(
                                    "PNG 图片 · {} × {} · {} 像素 · 来源链 {} 步",
                                    image.image.width(),
                                    image.image.height(),
                                    u64::from(image.image.width())
                                        * u64::from(image.image.height()),
                                    image.origins.len()
                                ));
                                if let Some(texture) = &self.draft_image_texture {
                                    ui.image((texture.id(), texture.size_vec2()));
                                }
                            });
                            ui.small("保存时转换为 PNG；单版上限 16 MiB。图片正文不会进入文本搜索。历史图片可回传到文本操作流程。权限：仍需单独确认保存。");
                            draft_size_ok = true;
                        } else {
                            ui.label("图片资料没有可编辑像素；从图片工具接力一张图片以替换草稿。");
                            draft_size_ok = false;
                        }
                    } else {
                        let response = ui.add_sized(
                            [ui.available_width(), 250.0],
                            egui::TextEdit::multiline(&mut self.draft_text)
                                .code_editor()
                                .hint_text("粘贴或编辑文本、JSON、CSV、TSV；内容不会自动保存"),
                        );
                        if response.changed() {
                            self.mark_dirty();
                        }
                        ui.small(format!(
                            "{} / {} 字节",
                            self.draft_text.len(),
                            MAX_VERSION_BYTES
                        ));
                        draft_size_ok = self.draft_text.len() <= MAX_VERSION_BYTES;
                    }
                    ui.horizontal(|ui| {
                        if self.selected.is_none() {
                            if ui
                                .add_enabled(
                                    self.dirty && draft_size_ok,
                                    egui::Button::new("保存为新资料…"),
                                )
                                .clicked()
                            {
                                self.prepare_save(VersionAction::Create);
                            }
                        } else {
                            if ui
                                .add_enabled(
                                    self.dirty && draft_size_ok,
                                    egui::Button::new("追加版本…"),
                                )
                                .clicked()
                            {
                                self.prepare_save(VersionAction::Append);
                            }
                            if ui
                                .add_enabled(
                                    self.dirty && draft_size_ok,
                                    egui::Button::new("替换当前版…"),
                                )
                                .clicked()
                            {
                                self.prepare_save(VersionAction::Replace);
                            }
                            if ui.button("移入回收站…").clicked() {
                                self.pending_delete = self.selected.clone();
                                self.pending_purge = false;
                                self.pending = None;
                            }
                        }
                        if self.dirty && ui.button("放弃草稿").clicked() {
                            self.dirty = false;
                            if let Some(entry) = self.selected.clone() {
                                self.load_entry(entry.clone(), entry.revision);
                            } else {
                                self.new_draft();
                            }
                        }
                    });
                },
            );
        });
        if let Some((from_revision, current_revision, diff)) = &self.comparison {
            ui.separator();
            egui::CollapsingHeader::new(format!(
                "版本差异 · v{from_revision} → 当前 v{current_revision}"
            ))
            .default_open(true)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .show(ui, |ui| {
                        if diff.is_empty() {
                            ui.label("两个版本内容相同。");
                        }
                        for line in diff.lines() {
                            let color = match line.as_bytes().first() {
                                Some(b'+') => egui::Color32::from_rgb(50, 150, 80),
                                Some(b'-') => egui::Color32::from_rgb(200, 80, 80),
                                _ => ui.visuals().text_color(),
                            };
                            ui.colored_label(color, egui::RichText::new(line).monospace());
                        }
                    });
            });
        }
        let mut confirm_clicked = false;
        let mut cancel_clicked = false;
        if let Some(pending) = &self.pending {
            ui.separator();
            ui.group(|ui| {
                ui.strong(match pending.action {
                    VersionAction::Create => "确认在本机新建资料",
                    VersionAction::Append => "确认追加一个保留历史的资料版本",
                    VersionAction::Replace => "确认替换当前版并保留旧版本",
                });
                ui.label(format!(
                    "{} · {} · {} 字节",
                    pending.name,
                    pending.kind.label(),
                    pending.payload.len()
                ));
                ui.label(format!("来源：{}", pending.source));
                ui.label(format!("SHA-256 {}", digest(&pending.payload)));
                if pending.kind == Kind::Image {
                    ui.label(
                        "已校验 PNG 二进制；确认后才写入本机资料库。OCR 与图片正文索引未启用。",
                    );
                } else {
                    let preview = String::from_utf8_lossy(&pending.payload);
                    ui.add(egui::Label::new(preview.chars().take(320).collect::<String>()).wrap());
                }
                ui.horizontal(|ui| {
                    if ui.button("确认保存到本机").clicked() {
                        confirm_clicked = true;
                    }
                    if ui.button("取消").clicked() {
                        cancel_clicked = true;
                    }
                });
            });
        }
        if confirm_clicked {
            self.confirm_save();
        } else if cancel_clicked {
            self.pending = None;
        }
        let mut delete_clicked = false;
        let mut cancel_delete = false;
        if let Some(entry) = &self.pending_delete {
            ui.separator();
            ui.group(|ui| {
                if self.pending_purge {
                    ui.strong("确认永久删除回收站资料");
                    ui.label(format!(
                        "{} · {} 个版本 · 当前 v{} · 此操作不能撤销",
                        entry.name, entry.versions, entry.revision
                    ));
                } else {
                    ui.strong("确认移入回收站");
                    ui.label(format!(
                        "{} · {} 个版本 · 当前 v{} · 可在回收站中恢复",
                        entry.name, entry.versions, entry.revision
                    ));
                }
                ui.horizontal(|ui| {
                    let confirm_label = if self.pending_purge {
                        "永久删除"
                    } else {
                        "移入回收站"
                    };
                    if ui.button(confirm_label).clicked() {
                        delete_clicked = true;
                    }
                    if ui.button("取消").clicked() {
                        cancel_delete = true;
                    }
                });
            });
        }
        if delete_clicked {
            self.confirm_delete();
        } else if cancel_delete {
            self.pending_delete = None;
            self.pending_purge = false;
        }
        if !self.notice.is_empty() {
            ui.label(&self.notice);
        }
        if !self.error.is_empty() {
            ui.colored_label(egui::Color32::from_rgb(220, 80, 80), &self.error);
        }
    }

    fn prepare_save(&mut self, action: VersionAction) {
        let payload = if self.draft_kind == Kind::Image {
            match self
                .draft_image
                .as_ref()
                .context("图片草稿没有可保存像素")
                .and_then(|image| encode_png(&image.image))
            {
                Ok(payload) => payload,
                Err(error) => {
                    self.error = error.to_string();
                    return;
                }
            }
        } else {
            self.draft_text.as_bytes().to_vec()
        };
        if let Err(error) = self.draft_kind.validate(&payload) {
            self.error = error.to_string();
            return;
        }
        let selected = self.selected.as_ref();
        if action == VersionAction::Create && selected.is_some()
            || action != VersionAction::Create && selected.is_none()
        {
            self.error = "资料状态已变化，请刷新后重试。".into();
            return;
        }
        self.pending = Some(PendingSave {
            id: selected.map(|entry| entry.id.clone()),
            expected_revision: selected.map_or(0, |entry| entry.revision),
            name: self.draft_name.clone(),
            kind: self.draft_kind,
            payload,
            source: self.draft_source.clone(),
            action,
        });
        self.pending_delete = None;
        self.error.clear();
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
        self.draft_source = "本机编辑".into();
    }
}

impl Drop for State {
    fn drop(&mut self) {
        self.search_cancel_generation
            .store(self.search_generation.wrapping_add(1), Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (PathBuf, Store) {
        let root = std::env::temp_dir().join(format!("zi-artifacts-{}", uuid::Uuid::new_v4()));
        let store = Store {
            path: root.join("library.sqlite3"),
        };
        (root, store)
    }

    #[test]
    fn obsolete_search_does_not_open_or_create_the_library() {
        let (root, store) = fixture();
        let latest_generation = Arc::new(AtomicU64::new(2));
        let result = store.list_filtered_cancellable(
            "stale query",
            None,
            false,
            Some((latest_generation, 1)),
        );
        assert!(result.unwrap_err().to_string().contains("新查询取代"));
        assert!(!store.path.exists());
        assert!(!root.exists());
    }

    #[test]
    fn a_new_search_interrupts_an_active_sqlite_query() {
        let db = Connection::open_in_memory().unwrap();
        let generation = Arc::new(AtomicU64::new(1));
        let query_ready = Arc::new(std::sync::Barrier::new(2));
        let worker_generation = Arc::clone(&generation);
        let worker_ready = Arc::clone(&query_ready);
        let worker = std::thread::spawn(move || {
            cancel_sqlite_query_when_search_changes(&db, &worker_generation, 1);
            worker_ready.wait();
            db.query_row(
                "WITH RECURSIVE sequence(value) AS (
                    SELECT 0 UNION ALL SELECT value + 1 FROM sequence WHERE value < 10000000
                 ) SELECT sum(value) FROM sequence",
                [],
                |row| row.get::<_, i64>(0),
            )
        });
        query_ready.wait();
        generation.store(2, Ordering::Relaxed);
        let result = worker.join().unwrap();
        assert!(matches!(
            result,
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::OperationInterrupted
        ));
    }

    #[test]
    fn saving_is_explicit_versions_are_immutable_and_stale_writes_fail() {
        let (root, store) = fixture();
        assert!(store.list("").unwrap().is_empty());
        assert!(!root.exists());
        let first = store
            .create("部署报告", Kind::Json, br#"{"ok":true}"#, "工具接力")
            .unwrap();
        assert_eq!(first.revision, 1);
        assert_eq!(store.load(&first.id, 1).unwrap().bytes, br#"{"ok":true}"#);
        let second = store
            .add_version(
                &first.id,
                1,
                Kind::Json,
                br#"{"ok":false}"#,
                "工具接力",
                VersionAction::Append,
            )
            .unwrap();
        assert_eq!(second.revision, 2);
        assert_eq!(second.versions, 2);
        assert_eq!(store.versions(&first.id).unwrap().len(), 2);
        assert_eq!(store.load(&first.id, 1).unwrap().bytes, br#"{"ok":true}"#);
        assert!(
            store
                .add_version(
                    &first.id,
                    1,
                    Kind::Json,
                    br#"{"lost":true}"#,
                    "工具接力",
                    VersionAction::Replace,
                )
                .is_err()
        );
        assert_eq!(store.load(&first.id, 2).unwrap().bytes, br#"{"ok":false}"#);
        assert!(store.list("不存在").unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn trash_hides_entries_without_destroying_versions_and_restore_or_purge_checks_revision() {
        let (root, store) = fixture();
        let kept = store
            .create("保留资料", Kind::Text, b"stay", "本机编辑")
            .unwrap();
        let trashed = store
            .create(
                "可恢复资料",
                Kind::Json,
                br#"{"note":"recover me"}"#,
                "本机编辑",
            )
            .unwrap();
        store
            .add_version(
                &trashed.id,
                1,
                Kind::Json,
                br#"{"note":"revised"}"#,
                "本机编辑",
                VersionAction::Append,
            )
            .unwrap();

        assert_eq!(store.list_trash("").unwrap().len(), 0);
        store.delete(&trashed.id, 2).unwrap();
        assert_eq!(store.list("").unwrap()[0].id, kept.id);
        assert!(
            store
                .add_version(
                    &trashed.id,
                    2,
                    Kind::Json,
                    br#"{"note":"stale edit"}"#,
                    "本机编辑",
                    VersionAction::Append,
                )
                .is_err()
        );
        let trash = store.list_trash("可恢复").unwrap();
        assert_eq!(trash.len(), 1);
        assert_eq!(trash[0].id, trashed.id);
        assert_eq!(trash[0].versions, 2);
        assert_eq!(
            store.load(&trashed.id, 1).unwrap().bytes,
            br#"{"note":"recover me"}"#
        );
        assert!(store.restore(&trashed.id, 1).is_err());
        assert!(store.purge(&kept.id, kept.revision).is_err());

        store.restore(&trashed.id, 2).unwrap();
        assert_eq!(store.list("").unwrap().len(), 2);
        assert!(store.list_trash("").unwrap().is_empty());
        assert_eq!(
            store.load(&trashed.id, 2).unwrap().bytes,
            br#"{"note":"revised"}"#
        );

        store.delete(&trashed.id, 2).unwrap();
        assert!(store.purge(&trashed.id, 1).is_err());
        store.purge(&trashed.id, 2).unwrap();
        assert!(store.list_trash("").unwrap().is_empty());
        assert!(store.load(&trashed.id, 1).is_err());
        assert_eq!(store.list("").unwrap().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn version_comparison_is_read_only_bounded_and_targets_the_latest_revision() {
        let (root, store) = fixture();
        let original = store
            .create(
                "差异资料",
                Kind::Text,
                "第一行\n旧内容\n".as_bytes(),
                "本机编辑",
            )
            .unwrap();
        store
            .add_version(
                &original.id,
                1,
                Kind::Text,
                "第一行\n新内容\n新增行\n".as_bytes(),
                "本机编辑",
                VersionAction::Append,
            )
            .unwrap();

        let (current_revision, diff) = store.compare_with_current(&original.id, 1).unwrap();
        assert_eq!(current_revision, 2);
        assert!(diff.contains("-旧内容"));
        assert!(diff.contains("+新内容"));
        assert!(diff.contains("+新增行"));
        assert_eq!(store.versions(&original.id).unwrap().len(), 2);
        assert_eq!(
            store.load(&original.id, 1).unwrap().bytes,
            "第一行\n旧内容\n".as_bytes()
        );
        assert_eq!(
            store.load(&original.id, 2).unwrap().bytes,
            "第一行\n新内容\n新增行\n".as_bytes()
        );
        assert!(store.compare_with_current(&original.id, 0).is_err());

        let same = store
            .create("相同版本", Kind::Text, b"same\n", "本机编辑")
            .unwrap();
        store
            .add_version(
                &same.id,
                1,
                Kind::Text,
                b"same\n",
                "本机编辑",
                VersionAction::Append,
            )
            .unwrap();
        assert_eq!(store.compare_with_current(&same.id, 1).unwrap().1, "");

        let large = store
            .create("大差异", Kind::Text, b"old\n", "本机编辑")
            .unwrap();
        let large_text = format!("{}\n", "变".repeat(70_000));
        store
            .add_version(
                &large.id,
                1,
                Kind::Text,
                large_text.as_bytes(),
                "本机编辑",
                VersionAction::Append,
            )
            .unwrap();
        let diff = store.compare_with_current(&large.id, 1).unwrap().1;
        assert!(diff.len() <= MAX_DIFF_OUTPUT_BYTES);
        assert!(diff.ends_with("…差异已截断"));

        let oversize = store
            .create("超限差异", Kind::Text, &vec![b'a'; 1_100_000], "本机编辑")
            .unwrap();
        store
            .add_version(
                &oversize.id,
                1,
                Kind::Text,
                &vec![b'b'; 1_100_000],
                "本机编辑",
                VersionAction::Append,
            )
            .unwrap();
        assert!(store.compare_with_current(&oversize.id, 1).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn search_matches_names_and_current_payload_with_substrings_and_ignores_old_versions() {
        let (root, store) = fixture();
        let first = store
            .create(
                "运行记录",
                Kind::Text,
                "The DATABASE lock uses optimistic revisions.".as_bytes(),
                "本机编辑",
            )
            .unwrap();
        let second = store
            .create(
                "接口说明",
                Kind::Text,
                "错误响应包含请求追踪编号 ZC-84。".as_bytes(),
                "agent-runner@1.9.0",
            )
            .unwrap();
        let unicode_text = format!(
            "{}Fehler bei der Überprüfung des Profils.\n{}",
            "前文".repeat(30),
            "后文".repeat(30)
        );
        let unicode = store
            .create(
                "国际化日志",
                Kind::Text,
                unicode_text.as_bytes(),
                "本机编辑",
            )
            .unwrap();

        assert_eq!(store.list("DATABASE").unwrap()[0].id, first.id);
        assert_eq!(store.list("踪编号 ZC").unwrap()[0].id, second.id);
        let source_match = store.list("AGENT-RUNNER@1.9.0").unwrap().remove(0);
        assert_eq!(source_match.id, second.id);
        assert!(source_match.search_preview.is_none());
        let with_old_source = store
            .create(
                "带历史来源的资料",
                Kind::Text,
                b"current material contents",
                "agent-runner@1.8.0",
            )
            .unwrap();
        store
            .add_version(
                &with_old_source.id,
                1,
                Kind::Text,
                b"updated material contents",
                "pipeline@0.20.0",
                VersionAction::Append,
            )
            .unwrap();
        let historical_match = store.list("agent-runner@1.8.0").unwrap().remove(0);
        assert_eq!(historical_match.id, with_old_source.id);
        assert_eq!(
            historical_match.historical_source_match,
            Some((1, "agent-runner@1.8.0".into()))
        );
        let unicode_match = store.list("ÜBERPRÜFUNG").unwrap().remove(0);
        assert_eq!(unicode_match.id, unicode.id);
        let preview = unicode_match.search_preview.unwrap();
        assert!(preview.contains("Überprüfung"));
        assert!(preview.starts_with('…') && preview.ends_with('…'));
        assert!(preview.chars().count() <= SEARCH_PREVIEW_CONTEXT_CHARS * 2 + 14);
        assert!(!preview.chars().any(char::is_control));
        let name_match = store.list("国际化日志").unwrap().remove(0);
        assert!(name_match.search_preview.is_none());
        assert!(store.list("absent marker").unwrap().is_empty());

        store
            .add_version(
                &first.id,
                1,
                Kind::Text,
                b"New revision has a different searchable phrase.",
                "本机编辑",
                VersionAction::Replace,
            )
            .unwrap();
        assert!(store.list("optimistic revisions").unwrap().is_empty());
        assert_eq!(store.list("searchable phrase").unwrap()[0].id, first.id);
        assert!(store.list("\n").is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn type_filters_work_with_empty_or_body_queries() {
        let (root, store) = fixture();
        let text = store
            .create("说明", Kind::Text, b"plain text", "本机编辑")
            .unwrap();
        let json = store
            .create("配置", Kind::Json, br#"{"mode":"strict"}"#, "本机编辑")
            .unwrap();
        let csv = store
            .create("用户表", Kind::Csv, b"id,name\n1,Ada\n", "本机编辑")
            .unwrap();
        let tsv = store
            .create("数据表", Kind::Tsv, b"id\tname\n2\tGrace\n", "本机编辑")
            .unwrap();
        let table_payload = br#"{"headers":["id","name"],"rows":[[2,"Grace"]]}"#;
        let table = store
            .create("结构化数据", Kind::Table, table_payload, "pipeline@0.20.0")
            .unwrap();

        let text_entries = store.list_filtered("", Some(Kind::Text), false).unwrap();
        assert_eq!(text_entries.len(), 1);
        assert_eq!(text_entries[0].id, text.id);
        assert!(text_entries[0].search_preview.is_none());
        assert_eq!(
            store
                .list_filtered("strict", Some(Kind::Json), false)
                .unwrap()[0]
                .id,
            json.id
        );
        assert!(
            store
                .list_filtered("strict", Some(Kind::Text), false)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store.list_filtered("", Some(Kind::Csv), false).unwrap()[0].id,
            csv.id
        );
        assert_eq!(
            store.list_filtered("", Some(Kind::Tsv), false).unwrap()[0].id,
            tsv.id
        );
        assert_eq!(
            store
                .list_filtered("Grace", Some(Kind::Table), false)
                .unwrap()[0]
                .id,
            table.id
        );
        assert_eq!(store.list_filtered("", None, false).unwrap().len(), 5);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn state_trash_view_is_read_only_and_restore_preserves_unsaved_draft() {
        let (root, store) = fixture();
        let entry = store
            .create("恢复检查", Kind::Text, b"keep my source", "本机编辑")
            .unwrap();
        store.delete(&entry.id, entry.revision).unwrap();
        let mut state = State::new(store.path.clone());

        state.set_trash_view(true);
        assert!(state.show_trash);
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.entries[0].id, entry.id);
        state.load_entry(state.entries[0].clone(), 1);
        assert!(state.selected_in_trash);
        assert_eq!(state.draft_text, "keep my source");

        state.draft_text.push_str("; unsaved");
        state.mark_dirty();
        state.restore_selected();
        assert!(state.selected_in_trash);
        assert!(state.dirty);
        assert_eq!(state.draft_text, "keep my source; unsaved");

        state.dirty = false;
        state.restore_selected();
        assert!(!state.selected_in_trash);
        assert!(!state.show_trash);
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.entries[0].id, entry.id);
        assert_eq!(store.load(&entry.id, 1).unwrap().bytes, b"keep my source");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn structured_table_handoff_keeps_its_kind_through_save_reload_and_search() {
        let (root, store) = fixture();
        let dataset = crate::workbench::Dataset {
            headers: vec!["name".into(), "count".into(), "enabled".into()],
            rows: vec![vec![
                serde_json::json!("Zi"),
                serde_json::json!(7),
                serde_json::json!(true),
            ]]
            .into(),
        };
        let payload = serde_json::to_string(&dataset).unwrap();
        let mut state = State::new(store.path.clone());
        state
            .receive_text_with_reference(&payload, Kind::Table, None, Some("pipeline@0.20.0"))
            .unwrap();
        assert_eq!(state.draft_kind, Kind::Table);
        assert!(
            !store.path.exists(),
            "receiving must remain a transient draft"
        );

        state.prepare_save(VersionAction::Create);
        state.confirm_save();
        let saved = state.selected.as_ref().unwrap().clone();
        assert_eq!(saved.kind, Kind::Table);
        assert_eq!(store.load(&saved.id, 1).unwrap().bytes, payload.as_bytes());
        assert_eq!(
            store.list_filtered("Zi", Some(Kind::Table), false).unwrap()[0].id,
            saved.id
        );

        let mut reopened = State::new(store.path.clone());
        reopened.load_entry(reopened.entries[0].clone(), 1);
        assert_eq!(reopened.draft_kind, Kind::Table);
        assert_eq!(
            serde_json::from_str::<crate::workbench::Dataset>(&reopened.draft_text).unwrap(),
            dataset
        );
        assert_eq!(reopened.handoff_table_snapshot().unwrap(), Some(dataset));
        assert!(reopened.draft_source.contains("pipeline@0.20.0"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_edited_table_cannot_be_exported_as_a_typed_handoff() {
        let (root, store) = fixture();
        let mut state = State::new(store.path.clone());
        state
            .receive_text(r#"{"headers":["a"],"rows":[[1]]}"#, Kind::Table, None)
            .unwrap();
        state.draft_text = r#"{"headers":["a"],"rows":[[]]}"#.into();
        assert!(state.handoff_table_snapshot().is_err());
        assert!(!store.path.exists());
        assert!(!root.exists());
    }

    #[test]
    fn background_search_returns_current_matches_without_blocking_the_editor_state() {
        let (root, store) = fixture();
        let entry = store
            .create(
                "后台搜索",
                Kind::Text,
                "worker marker".as_bytes(),
                "本机编辑",
            )
            .unwrap();
        store
            .create(
                "JSON 搜索",
                Kind::Json,
                br#"{"text":"worker marker"}"#,
                "本机编辑",
            )
            .unwrap();
        let mut state = State::new(store.path.clone());
        state.query = "worker marker".into();
        state.kind_filter = Some(Kind::Text);
        state.dirty = true;
        state.draft_text = "unsaved draft".into();
        state.refresh();
        let (generation, result) = state
            .search_results
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert_eq!(generation, state.search_generation);
        let entries = result.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, entry.id);
        assert!(state.dirty);
        assert_eq!(state.draft_text, "unsaved draft");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_types_names_sources_and_limits_are_rejected_before_write() {
        let (_root, store) = fixture();
        assert!(store.create(" ", Kind::Text, b"x", "本机编辑").is_err());
        assert!(
            store
                .create("无效 JSON", Kind::Json, b"{", "本机编辑")
                .is_err()
        );
        assert!(
            store
                .create("含换行来源", Kind::Text, b"x", "tool\nsecret")
                .is_err()
        );
        assert!(
            store
                .create("路径来源", Kind::Text, b"x", "E:\\secret\\file.txt")
                .is_err()
        );
        assert!(
            store
                .create("二进制", Kind::Text, &[0xff], "本机编辑")
                .is_err()
        );
        assert!(
            store
                .create(
                    "超大",
                    Kind::Text,
                    &vec![b'x'; MAX_VERSION_BYTES + 1],
                    "本机编辑"
                )
                .is_err()
        );
        assert!(
            store
                .create(
                    "错误表格",
                    Kind::Table,
                    br#"{"headers":["a"],"rows":[[]]}"#,
                    "工具接力"
                )
                .is_err()
        );
        assert!(!store.path.exists());
    }

    #[test]
    fn handoff_only_fills_a_draft_and_requires_a_separate_local_save_confirmation() {
        let (root, store) = fixture();
        let mut state = State::new(store.path.clone());
        state
            .receive_text("来自另一个工具的内容", Kind::Text, None)
            .unwrap();
        assert!(state.dirty);
        assert!(!store.path.exists());
        state.prepare_save(VersionAction::Create);
        assert!(state.pending.is_some());
        assert!(!store.path.exists());
        state.confirm_save();
        assert!(store.path.exists());
        assert!(!state.dirty);
        assert_eq!(state.entries.len(), 1);
        assert_eq!(
            store.load(&state.entries[0].id, 1).unwrap().bytes,
            "来自另一个工具的内容".as_bytes()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn handoff_snapshot_exports_visible_text_without_writing_the_library() {
        let (root, store) = fixture();
        let mut state = State::new(store.path.clone());
        state
            .receive_text("给其他工具继续处理", Kind::Text, None)
            .unwrap();

        let (source, text) = state.handoff_snapshot().unwrap();
        assert!(source.contains("资料草稿「接力结果」"));
        assert!(source.contains("（未保存）"));
        assert_eq!(text, "给其他工具继续处理");
        assert!(!store.path.exists());

        state.prepare_save(VersionAction::Create);
        state.confirm_save();
        let (source, text) = state.handoff_snapshot().unwrap();
        assert!(source.contains("本机资料「接力结果」· v1"));
        assert_eq!(text, "给其他工具继续处理");
        assert!(store.path.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tool_source_reference_survives_save_reload_and_is_cleared_by_manual_edits() {
        let (root, store) = fixture();
        let mut state = State::new(store.path.clone());
        state
            .receive_text_with_reference(
                "Agent 的最终回答",
                Kind::Text,
                None,
                Some("agent-runner@1.2.3"),
            )
            .unwrap();
        state.prepare_save(VersionAction::Create);
        state.confirm_save();
        assert_eq!(
            state.handoff_source_reference().as_deref(),
            Some("agent-runner@1.2.3")
        );

        let mut reopened = State::new(store.path.clone());
        reopened.load_entry(reopened.entries[0].clone(), 1);
        assert_eq!(
            reopened.handoff_source_reference().as_deref(),
            Some("agent-runner@1.2.3")
        );
        reopened.mark_dirty();
        assert_eq!(reopened.handoff_source_reference(), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn image_material_is_explicitly_saved_versioned_searched_and_reopened() {
        let (root, store) = fixture();
        let pixels = image::RgbaImage::from_pixel(3, 2, image::Rgba([18, 90, 220, 123]));
        let image = crate::image_tools::ImageMaterial {
            image: std::sync::Arc::new(DynamicImage::ImageRgba8(pixels.clone())),
            origins: Vec::new(),
            kind: "验收图像",
        };
        let mut state = State::new(store.path.clone());
        state
            .receive_image_with_reference(image, None, Some("image-workflow@0.12.0"))
            .unwrap();
        assert!(!store.path.exists(), "接力不能隐式写入资料库");
        assert!(state.handoff_snapshot().is_none());
        assert_eq!(state.handoff_image_snapshot().unwrap().1.image.width(), 3);

        state.prepare_save(VersionAction::Create);
        assert!(state.pending.is_some());
        assert!(!store.path.exists(), "准备保存不能替代用户确认");
        state.confirm_save();
        let entry = state.selected.clone().unwrap();
        assert_eq!(entry.kind, Kind::Image);
        assert_eq!(entry.revision, 1);
        assert_eq!(entry.source, "image-workflow@0.12.0");
        assert!(store.compare_with_current(&entry.id, 1).is_err());
        assert_eq!(store.list("image-workflow").unwrap().len(), 1);
        assert!(store.list("not-in-image-pixels").unwrap().is_empty());

        let version = store.load(&entry.id, 1).unwrap();
        let decoded = decode_png(&version.bytes).unwrap();
        assert_eq!(decoded.to_rgba8(), pixels);
        let mut reopened = State::new(store.path.clone());
        reopened.load_entry(reopened.entries[0].clone(), 1);
        let (_, returned) = reopened.handoff_image_snapshot().unwrap();
        assert_eq!(returned.image.to_rgba8(), pixels);
        assert_eq!(
            reopened.handoff_source_reference().as_deref(),
            Some("image-workflow@0.12.0")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn image_versions_reject_non_png_and_oversized_images() {
        assert!(Kind::Image.validate(b"not a png").is_err());
        let (root, store) = fixture();
        let pixels = image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 0, 0]));
        let mut png = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(pixels)
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();
        let bytes = png.into_inner();
        assert!(Kind::Image.validate(&bytes).is_ok());
        let mut oversized = bytes.clone();
        oversized[16..20].copy_from_slice(&12_001_u32.to_be_bytes());
        assert!(Kind::Image.validate(&oversized).is_err());
        assert!(
            store
                .create("透明图片", Kind::Image, &bytes, "本机编辑")
                .is_ok()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn source_provenance_is_limited_to_safe_tool_ids_and_stable_versions() {
        assert!(validate_source("mcp-inspector@0.4.0").is_ok());
        for invalid in [
            "E:\\secret\\result.txt",
            "../../result",
            "tool@0.0.0-dev.1",
            "tool@01.0.0",
            "bad tool@1.0.0",
        ] {
            assert!(validate_source(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn incompatible_or_linked_library_is_never_replaced() {
        let (root, store) = fixture();
        fs::create_dir_all(&root).unwrap();
        let foreign = Connection::open(&store.path).unwrap();
        foreign.execute_batch("PRAGMA user_version=99;").unwrap();
        drop(foreign);
        let before = fs::read(&store.path).unwrap();
        assert!(store.create("保留", Kind::Text, b"x", "本机编辑").is_err());
        assert_eq!(fs::read(&store.path).unwrap(), before);
        fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn library_symlinks_are_rejected() {
        let (root, store) = fixture();
        fs::create_dir_all(&root).unwrap();
        let target = root.join("real.sqlite3");
        Connection::open(&target)
            .unwrap()
            .execute_batch("PRAGMA user_version=99;")
            .unwrap();
        std::os::windows::fs::symlink_file(&target, &store.path).unwrap();
        assert!(store.create("链接", Kind::Text, b"x", "本机编辑").is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
