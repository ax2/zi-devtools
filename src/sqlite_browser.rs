//! Explicit, bounded, read-only browsing of a selected SQLite file.
use anyhow::{Context, Result, ensure};
use eframe::egui;
use rusqlite::{Connection, OpenFlags, types::ValueRef};
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};

use crate::disk_inspector::is_link;
mod backup;
mod row;
mod transfer;
mod write;

const PAGE_SIZE: usize = 50;
const MAX_PAGES: usize = 200;
const MAX_TABLES: usize = 200;
const MAX_COLUMNS: usize = 128;
const MAX_CELL_BYTES: usize = 4096;
const MAX_PAGE_BYTES: usize = 2 * 1024 * 1024;
const QUERY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub struct Table {
    pub name: String,
    pub kind: String,
}

#[derive(Clone, Debug)]
pub struct Catalog {
    pub path: PathBuf,
    pub tables: Vec<Table>,
    pub more_tables: bool,
}

#[derive(Clone, Debug)]
pub struct Column {
    pub name: String,
    pub declared_type: String,
    pub not_null: bool,
    pub primary_key: bool,
    pub hidden: bool,
}

#[derive(Clone, Debug)]
pub struct Cell {
    pub display: String,
    pub export: String,
    pub kind: &'static str,
    pub truncated: bool,
    /// Exact representable SQLite value; None for truncated/invalid text, blobs, or nonfinite reals.
    pub value: Option<serde_json::Value>,
}

#[derive(Clone, Debug)]
pub struct PageData {
    pub table: String,
    pub page: usize,
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Cell>>,
    pub has_next: bool,
    pub truncated_cells: usize,
}

fn open_read_only(path: &Path, cancelled: &Arc<AtomicBool>) -> Result<Connection> {
    ensure!(path.is_absolute(), "请选择绝对路径的本机 SQLite 文件");
    let metadata = fs::symlink_metadata(path).context("无法读取数据库文件信息")?;
    ensure!(
        metadata.is_file() && !is_link(&metadata),
        "请选择普通文件，不支持符号链接或重解析点"
    );
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .context("无法以只读模式打开 SQLite 文件")?;
    connection.busy_timeout(Duration::from_secs(1))?;
    connection.pragma_update(None, "query_only", true)?;
    let start = Instant::now();
    let signal = Arc::clone(cancelled);
    connection.progress_handler(
        1000,
        Some(move || signal.load(Ordering::Relaxed) || start.elapsed() >= QUERY_TIMEOUT),
    );
    Ok(connection)
}

pub fn list_tables(path: &Path, cancelled: &Arc<AtomicBool>) -> Result<Catalog> {
    ensure!(!cancelled.load(Ordering::Relaxed), "已取消");
    let connection = open_read_only(path, cancelled)?;
    let mut statement = connection.prepare(
        "SELECT name, type FROM sqlite_schema WHERE type IN ('table','view') \
         AND name NOT LIKE 'sqlite_%' ORDER BY type, name LIMIT 201",
    )?;
    let mut tables = Vec::new();
    let rows = statement.query_map([], |row| {
        Ok(Table {
            name: row.get(0)?,
            kind: row.get(1)?,
        })
    })?;
    for row in rows {
        ensure!(!cancelled.load(Ordering::Relaxed), "已取消");
        tables.push(row.context("无法读取表清单")?);
    }
    let more_tables = tables.len() > MAX_TABLES;
    tables.truncate(MAX_TABLES);
    Ok(Catalog {
        path: path.to_path_buf(),
        tables,
        more_tables,
    })
}

fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn bounded_text(bytes: &[u8], remaining: &mut usize) -> (String, bool) {
    let limit = bytes.len().min(MAX_CELL_BYTES).min(*remaining);
    *remaining = remaining.saturating_sub(limit);
    let truncated = limit < bytes.len();
    let content = String::from_utf8_lossy(&bytes[..limit]).into_owned();
    (
        if truncated {
            format!("{content}… [已截断]")
        } else {
            content
        },
        truncated,
    )
}

fn cell(value: ValueRef<'_>, original_length: Option<i64>, remaining: &mut usize) -> Cell {
    match value {
        ValueRef::Null => Cell {
            value: Some(serde_json::Value::Null),
            display: "NULL".into(),
            export: String::new(),
            kind: "NULL",
            truncated: false,
        },
        ValueRef::Integer(value) => Cell {
            value: Some(value.into()),
            display: value.to_string(),
            export: value.to_string(),
            kind: "整数",
            truncated: false,
        },
        ValueRef::Real(value) => Cell {
            value: serde_json::Number::from_f64(value).map(serde_json::Value::Number),
            display: value.to_string(),
            export: value.to_string(),
            kind: "浮点",
            truncated: false,
        },
        ValueRef::Text(bytes) => {
            let (content, truncated) = bounded_text(bytes, remaining);
            Cell {
                value: if truncated {
                    None
                } else {
                    std::str::from_utf8(bytes).ok().map(|text| text.into())
                },
                display: content.clone(),
                export: content,
                kind: "文本",
                truncated,
            }
        }
        ValueRef::Blob(bytes) => {
            let length = original_length.unwrap_or(bytes.len() as i64).max(0) as usize;
            let shown = bytes.len().min(24).min(*remaining / 2);
            *remaining = remaining.saturating_sub(shown * 2);
            let prefix: String = bytes[..shown]
                .iter()
                .map(|byte| format!("{byte:02X}"))
                .collect();
            let content = format!(
                "BLOB {} B · {}{}",
                length,
                prefix,
                if shown < length { "…" } else { "" }
            );
            Cell {
                value: None,
                display: content.clone(),
                export: content,
                kind: "二进制预览",
                truncated: shown < length,
            }
        }
    }
}

pub fn load_page(
    path: &Path,
    table: &str,
    page: usize,
    cancelled: &Arc<AtomicBool>,
) -> Result<PageData> {
    ensure!(page < MAX_PAGES, "最多预览前 10000 行");
    ensure!(!cancelled.load(Ordering::Relaxed), "已取消");
    let connection = open_read_only(path, cancelled)?;
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name=?1 AND type IN ('table','view'))",
        [table],
        |row| row.get(0),
    )?;
    ensure!(exists, "表或视图已不存在，请重新打开数据库");
    let mut columns = Vec::new();
    let mut structure = connection
        .prepare("SELECT name, type, \"notnull\", pk, hidden FROM pragma_table_xinfo(?1)")?;
    let structure_rows = structure.query_map([table], |row| {
        Ok(Column {
            name: row.get(0)?,
            declared_type: row.get(1)?,
            not_null: row.get::<_, i64>(2)? != 0,
            primary_key: row.get::<_, i64>(3)? != 0,
            hidden: row.get::<_, i64>(4)? != 0,
        })
    })?;
    for entry in structure_rows {
        columns.push(entry?);
        ensure!(columns.len() <= MAX_COLUMNS, "列数超过128，暂不支持预览");
    }
    ensure!(!columns.is_empty(), "无法获取列结构");
    let header_sql = format!("SELECT * FROM {} LIMIT 0", quote_identifier(table));
    let header_statement = connection
        .prepare(&header_sql)
        .context("无法准备列清单查询")?;
    ensure!(
        header_statement.column_count() <= MAX_COLUMNS,
        "结果列数超过128"
    );
    let headers: Vec<String> = header_statement
        .column_names()
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    drop(header_statement);
    // Hidden virtual-table columns can be in xinfo but absent from SELECT *.
    // Match the actual result order rather than relying on pragma order.
    let structure = std::mem::take(&mut columns);
    columns = headers
        .iter()
        .map(|name| {
            structure
                .iter()
                .find(|column| column.name == *name)
                .cloned()
                .unwrap_or_else(|| Column {
                    name: name.clone(),
                    declared_type: String::new(),
                    not_null: false,
                    primary_key: false,
                    hidden: false,
                })
        })
        .collect();
    // SELECT * can materialize an entire multi-gigabyte value in the client.
    // Ask SQLite for bounded text/blob prefixes and actual lengths instead.
    let projections = headers
        .iter()
        .map(|name| {
            let quoted = quote_identifier(name);
            format!(
                "CASE typeof({quoted}) WHEN 'text' THEN CAST(substr(CAST({quoted} AS BLOB),1,4097) AS TEXT) \
                 WHEN 'blob' THEN substr({quoted},1,25) ELSE {quoted} END, length(CAST({quoted} AS BLOB))"
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT {projections} FROM {} LIMIT ?1 OFFSET ?2",
        quote_identifier(table)
    );
    let mut statement = connection.prepare(&sql).context("无法准备只读预览查询")?;
    let mut remaining = MAX_PAGE_BYTES;
    let mut rows = Vec::new();
    let mut has_next = false;
    let mut query = statement.query(rusqlite::params![
        PAGE_SIZE as i64 + 1,
        (page * PAGE_SIZE) as i64
    ])?;
    let mut truncated_cells = 0;
    while let Some(row) = query.next().context("预览查询失败或已超时")? {
        ensure!(!cancelled.load(Ordering::Relaxed), "已取消");
        if rows.len() == PAGE_SIZE {
            has_next = true;
            break;
        }
        let mut cells = Vec::with_capacity(columns.len());
        for index in 0..columns.len() {
            let value = cell(
                row.get_ref(index * 2)?,
                row.get(index * 2 + 1)?,
                &mut remaining,
            );
            truncated_cells += usize::from(value.truncated);
            cells.push(value);
        }
        rows.push(cells);
    }
    has_next &= page + 1 < MAX_PAGES;
    Ok(PageData {
        table: table.to_owned(),
        page,
        columns,
        rows,
        has_next,
        truncated_cells,
    })
}

fn csv_safe(value: &str) -> String {
    if value.trim_start().starts_with(['=', '+', '-', '@']) || value.starts_with(['\t', '\r']) {
        format!("'{value}")
    } else {
        value.to_owned()
    }
}

pub fn export_csv(data: &PageData, path: &Path) -> Result<()> {
    ensure!(!data.rows.is_empty(), "当前页没有可导出的行");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .context("无法新建 CSV；不会覆盖已有文件")?;
    let mut writer = csv::Writer::from_writer(file);
    writer.write_record(data.columns.iter().map(|column| csv_safe(&column.name)))?;
    for row in &data.rows {
        writer.write_record(row.iter().map(|cell| {
            if cell.kind == "文本" {
                csv_safe(&cell.export)
            } else {
                cell.export.clone()
            }
        }))?;
    }
    writer.flush()?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditOperation {
    New,
    Undo,
    Redo,
}

impl EditOperation {
    fn title(self) -> &'static str {
        match self {
            Self::New => "确认写入 SQLite",
            Self::Undo => "确认撤销 SQLite 修改",
            Self::Redo => "确认重做 SQLite 修改",
        }
    }
    fn button(self) -> &'static str {
        match self {
            Self::New => "确认写入",
            Self::Undo => "确认撤销",
            Self::Redo => "确认重做",
        }
    }
    fn completed(self) -> &'static str {
        match self {
            Self::New => "修改已提交",
            Self::Undo => "撤销已提交",
            Self::Redo => "重做已提交",
        }
    }
}

enum Response {
    Catalog(Result<Catalog, String>),
    Page(Result<PageData, String>),
    Backup(Result<PathBuf, String>),
    EditPreview(Result<MutationReview, String>),
    EditApplied(
        EditOperation,
        std::result::Result<MutationReview, ApplyFailure>,
    ),
}

#[derive(Debug)]
enum ApplyFailure {
    NotCommitted(String),
    CommittedWithoutHistory(String),
}

impl std::fmt::Display for ApplyFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotCommitted(message) | Self::CommittedWithoutHistory(message) => {
                formatter.write_str(message)
            }
        }
    }
}

#[derive(Clone, Debug)]
enum MutationReview {
    Cell(write::Review),
    Row(row::Review),
}

impl MutationReview {
    fn apply(&self) -> std::result::Result<Self, ApplyFailure> {
        match self {
            Self::Cell(review) => write::apply(review).map(Self::Cell),
            Self::Row(review) => row::apply(review).map(Self::Row),
        }
    }
    fn refresh_source(&mut self) -> Result<()> {
        match self {
            Self::Cell(review) => review.refresh_source(),
            Self::Row(review) => review.refresh_source(),
        }
    }
    fn table(&self) -> &str {
        match self {
            Self::Cell(review) => review.table(),
            Self::Row(review) => review.table(),
        }
    }
    fn change_label(&self) -> String {
        match self {
            Self::Cell(review) => format!(
                "单元格 {}：{} → {}",
                review.column(),
                review.old_label(),
                review.new_label()
            ),
            Self::Row(review) => review.change_label(),
        }
    }
    fn preview_details(&self) -> Vec<String> {
        match self {
            Self::Cell(review) => vec![format!(
                "{}：{} → {}",
                review.column(),
                review.old_label(),
                review.new_label()
            )],
            Self::Row(review) => review.preview_details(),
        }
    }
    fn title(&self, operation: EditOperation) -> &'static str {
        match (self, operation) {
            (Self::Cell(_), _) => operation.title(),
            (Self::Row(_), EditOperation::New) => "预览删除整行",
            (Self::Row(_), EditOperation::Undo) => "预览撤销删除",
            (Self::Row(_), EditOperation::Redo) => "预览重做删除",
        }
    }
    fn button(&self, operation: EditOperation) -> &'static str {
        match (self, operation) {
            (Self::Row(_), EditOperation::New) => "确认删除行",
            _ => operation.button(),
        }
    }
    fn key_summary(&self) -> String {
        match self {
            Self::Cell(review) => review.key_summary(),
            Self::Row(review) => review.key_summary(),
        }
    }
    fn warning(&self, operation: EditOperation) -> &'static str {
        match self {
            Self::Cell(_) if operation == EditOperation::New => {
                "上述预览已在事务中执行并回滚。确认后会再次核对文件身份、数据库结构、主键和旧值，再提交单条 UPDATE；若数据已变化，将拒绝写入。"
            }
            Self::Cell(_) => {
                "该撤销/重做只针对本机当前会话的最近一次单格修改。确认后会再次核对数据库文件身份、结构、主键和当前值；若数据已变化，将拒绝写入。"
            }
            Self::Row(_) if operation == EditOperation::New => {
                "上述删除已在事务中执行并回滚。确认后会再次核对文件身份、数据库结构、主键和整行内容，再提交单行 DELETE。整行数据仅保存在当前会话内以支持撤销。"
            }
            Self::Row(_) => {
                "该撤销/重做只针对本机当前会话中的整行删除。确认后会核对文件身份、数据库结构、主键和行内容，再恢复或删除该行；发生冲突时会拒绝操作。"
            }
        }
    }
}

pub struct State {
    transfer: transfer::State,
    path: String,
    catalog: Option<Catalog>,
    table: String,
    data: Option<PageData>,
    message: String,
    receiver: Option<Receiver<Response>>,
    cancel: Arc<AtomicBool>,
    backup_progress: Option<Arc<backup::Progress>>,
    selected_cell: Option<(usize, usize)>,
    edit_input: String,
    edit_dialog: bool,
    edit_review: Option<MutationReview>,
    edit_operation: EditOperation,
    undo_stack: Vec<MutationReview>,
    redo_stack: Vec<MutationReview>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            transfer: transfer::State::default(),
            path: String::new(),
            catalog: None,
            table: String::new(),
            data: None,
            message: String::new(),
            receiver: None,
            cancel: Arc::new(AtomicBool::new(false)),
            backup_progress: None,
            selected_cell: None,
            edit_input: String::new(),
            edit_dialog: false,
            edit_review: None,
            edit_operation: EditOperation::New,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }
}

impl State {
    #[cfg(feature = "ui-preview")]
    pub fn preview_transfer_position(&self, index: usize) -> egui::Pos2 {
        let rect = self.transfer.buttons[index].expect("SQLite transfer button missing");
        assert!(
            rect.is_positive(),
            "SQLite transfer button {index} is clipped"
        );
        rect.center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_workflow_result_check(&self, phase: u8) {
        assert!(self.receiver.is_none());
        let data = self.data.as_ref().unwrap();
        assert_eq!(data.table, "workflow_result");
        assert_eq!(data.rows.len(), 2);
        assert_eq!(data.rows[0][0].value, Some(serde_json::json!("001")));
        assert_eq!(data.rows[0][1].value, Some(serde_json::json!("Zi Tools")));
        assert_eq!(data.rows[0][2].value, Some(serde_json::json!(2)));
        assert_eq!(self.transfer.review.is_some(), phase == 1);
    }
    pub(crate) fn take_workbench_transfer(
        &mut self,
    ) -> Option<(String, crate::workbench::Dataset)> {
        self.transfer.pending.take()
    }
    pub(crate) fn transfer_failed(&mut self, error: String) {
        self.message = error;
    }
    pub(crate) fn background_active(&self) -> bool {
        self.receiver.is_some()
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_saved_export_ready(&self) -> bool {
        if self.receiver.is_some() {
            return false;
        }
        let data = self.data.as_ref().expect("saved SQLite page loaded");
        assert_eq!(data.table, "本地资料");
        assert_eq!(data.rows.len(), 2);
        assert_eq!(data.columns.len(), 4);
        true
    }
    pub fn open_path(&mut self, path: PathBuf) -> Result<()> {
        ensure!(self.receiver.is_none(), "SQLite浏览器正在读取，请稍后重试");
        self.path = path.to_str().context("数据库路径不是有效Unicode")?.into();
        self.start_catalog();
        Ok(())
    }
    fn poll(&mut self) {
        let Some(receiver) = &self.receiver else {
            return;
        };
        match receiver.try_recv() {
            Ok(Response::Catalog(Ok(catalog))) => {
                self.table = catalog
                    .tables
                    .first()
                    .map_or(String::new(), |t| t.name.clone());
                self.message = format!("已发现 {} 个表/视图", catalog.tables.len());
                self.catalog = Some(catalog);
                self.receiver = None;
                if !self.table.is_empty() {
                    self.start_page(0);
                }
            }
            Ok(Response::Page(Ok(data))) => {
                self.message = format!("第 {} 页 · {} 行", data.page + 1, data.rows.len());
                self.data = Some(data);
                self.selected_cell = None;
                self.receiver = None;
            }
            Ok(Response::Backup(Ok(path))) => {
                self.message = format!("备份已完成：{}", path.display());
                self.receiver = None;
                self.backup_progress = None;
            }
            Ok(Response::Backup(Err(error))) => {
                self.message = format!("备份失败：{error}");
                self.receiver = None;
                self.backup_progress = None;
            }
            Ok(Response::EditPreview(Ok(review))) => {
                self.edit_review = Some(review);
                self.edit_dialog = false;
                self.message = "变更预览已准备；数据库尚未提交修改".into();
                self.receiver = None;
            }
            Ok(Response::EditPreview(Err(error))) => {
                self.message = format!("无法预览写入：{error}");
                self.receiver = None;
            }
            Ok(Response::EditApplied(operation, Ok(inverse))) => {
                let page = self.data.as_ref().map_or(0, |data| data.page);
                self.edit_review = None;
                self.edit_dialog = false;
                match operation {
                    EditOperation::New => {
                        if self.undo_stack.len() == 20 {
                            self.undo_stack.remove(0);
                        }
                        self.undo_stack.push(inverse);
                        self.redo_stack.clear();
                    }
                    EditOperation::Undo => {
                        self.undo_stack.pop();
                        self.redo_stack.push(inverse);
                    }
                    EditOperation::Redo => {
                        self.redo_stack.pop();
                        if self.undo_stack.len() == 20 {
                            self.undo_stack.remove(0);
                        }
                        self.undo_stack.push(inverse);
                    }
                }
                let history_ok = self
                    .undo_stack
                    .iter_mut()
                    .chain(self.redo_stack.iter_mut())
                    .all(|review| review.refresh_source().is_ok());
                let history_message = if !history_ok {
                    self.undo_stack.clear();
                    self.redo_stack.clear();
                    format!(
                        "{}；无法继续核验历史，撤销/重做记录已清空",
                        operation.completed()
                    )
                } else {
                    format!("{}，正在刷新当前页…", operation.completed())
                };
                self.receiver = None;
                self.start_page(page);
                self.message = history_message;
            }
            Ok(Response::EditApplied(_, Err(error))) => {
                let page = self.data.as_ref().map_or(0, |data| data.page);
                self.edit_review = None;
                self.undo_stack.clear();
                self.redo_stack.clear();
                self.receiver = None;
                self.start_page(page);
                self.message = match error {
                    ApplyFailure::NotCommitted(error) => {
                        format!("未写入：{error}；正在刷新当前页…")
                    }
                    ApplyFailure::CommittedWithoutHistory(error) => format!(
                        "{}；撤销/重做快照刷新失败，历史记录已清空（{error}）；正在刷新当前页…",
                        self.edit_operation.completed()
                    ),
                };
            }
            Ok(Response::Catalog(Err(error)) | Response::Page(Err(error))) => {
                self.message = error;
                self.receiver = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.message = "SQLite 读取线程意外结束".into();
                self.receiver = None;
                self.backup_progress = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }

    fn start_catalog(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.edit_review = None;
        self.catalog = None;
        self.data = None;
        self.message = "正在只读打开数据库…".into();
        self.cancel = Arc::new(AtomicBool::new(false));
        self.backup_progress = None;
        let signal = Arc::clone(&self.cancel);
        let path = PathBuf::from(self.path.trim().trim_matches('"'));
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        std::thread::spawn(move || {
            let result =
                list_tables(&path, &signal).map_err(|error| format!("打开失败：{error:#}"));
            let _ = sender.send(Response::Catalog(result));
        });
    }

    fn start_page(&mut self, page: usize) {
        let Some(catalog) = &self.catalog else { return };
        self.data = None;
        self.selected_cell = None;
        self.message = format!("正在读取第 {} 页…", page + 1);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.backup_progress = None;
        let signal = Arc::clone(&self.cancel);
        let path = catalog.path.clone();
        let table = self.table.clone();
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        std::thread::spawn(move || {
            let result = load_page(&path, &table, page, &signal)
                .map_err(|error| format!("读取失败：{error:#}"));
            let _ = sender.send(Response::Page(result));
        });
    }

    fn start_edit_preview(&mut self, row: usize, column: usize) {
        let Some(catalog) = &self.catalog else { return };
        let Some(data) = self.data.clone() else {
            return;
        };
        let path = catalog.path.clone();
        let input = self.edit_input.clone();
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.message = "正在执行回滚预览…".into();
        std::thread::spawn(move || {
            let result = write::prepare(&path, &data, row, column, &input)
                .map(MutationReview::Cell)
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(Response::EditPreview(result));
        });
    }

    fn start_delete_preview(&mut self, row_index: usize) {
        let Some(catalog) = &self.catalog else { return };
        let Some(data) = self.data.clone() else {
            return;
        };
        let path = catalog.path.clone();
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.message = "正在执行整行删除回滚预览…".into();
        std::thread::spawn(move || {
            let result = row::prepare_delete(&path, &data, row_index)
                .map(MutationReview::Row)
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(Response::EditPreview(result));
        });
    }

    fn start_backup(&mut self, destination: PathBuf) {
        let source = PathBuf::from(self.path.trim().trim_matches('"'));
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = Arc::clone(&cancelled);
        let progress = Arc::new(backup::Progress::default());
        self.cancel = cancelled;
        self.backup_progress = Some(Arc::clone(&progress));
        self.message = "正在创建数据库备份…".into();
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        std::thread::spawn(move || {
            let result = backup::create(&source, &destination, &signal, &progress)
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(Response::Backup(result));
        });
    }

    fn start_edit_apply(&mut self) {
        let Some(review) = self.edit_review.clone() else {
            return;
        };
        let operation = self.edit_operation;
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.message = "正在复核原值并提交事务…".into();
        std::thread::spawn(move || {
            let result = review.apply();
            let _ = sender.send(Response::EditApplied(operation, result));
        });
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self) {
        self.path = r"C:\Projects\demo\sample.sqlite".into();
        self.table = "notes".into();
        self.catalog = Some(Catalog {
            path: PathBuf::from(&self.path),
            tables: vec![
                Table {
                    name: "notes".into(),
                    kind: "table".into(),
                },
                Table {
                    name: "projects".into(),
                    kind: "table".into(),
                },
                Table {
                    name: "recent_activity".into(),
                    kind: "view".into(),
                },
            ],
            more_tables: false,
        });
        self.data = Some(PageData {
            table: "notes".into(),
            page: 0,
            columns: vec![
                Column {
                    name: "id".into(),
                    declared_type: "INTEGER".into(),
                    not_null: true,
                    primary_key: true,
                    hidden: false,
                },
                Column {
                    name: "title".into(),
                    declared_type: "TEXT".into(),
                    not_null: true,
                    primary_key: false,
                    hidden: false,
                },
                Column {
                    name: "status".into(),
                    declared_type: "TEXT".into(),
                    not_null: false,
                    primary_key: false,
                    hidden: false,
                },
                Column {
                    name: "updated_at".into(),
                    declared_type: "TEXT".into(),
                    not_null: false,
                    primary_key: false,
                    hidden: false,
                },
            ],
            rows: [
                ["1", "发布检查清单", "进行中", "2026-10-01 09:30"],
                ["2", "资料索引", "完成", "2026-09-29 18:15"],
                ["3", "界面评审", "待处理", "2026-09-27 14:00"],
            ]
            .into_iter()
            .map(|values| {
                values
                    .into_iter()
                    .map(|value| Cell {
                        value: Some(value.into()),
                        display: value.into(),
                        export: value.into(),
                        kind: "文本",
                        truncated: false,
                    })
                    .collect()
            })
            .collect(),
            has_next: true,
            truncated_cells: 0,
        });
        self.selected_cell = Some((0, 1));
        self.message = "合成 SQLite 预览，未读取本机数据库".into();
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.poll();
        self.transfer.ui(ui);
        ui.heading("SQLite 浏览器");
        ui.label("选择本机数据库，按页查看数据；需要编辑时，先查看变更预览，再明确确认写入。");
        ui.add_space(8.0);
        let mut backup_destination = None;
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::TextEdit::singleline(&mut self.path)
                        .hint_text("数据库绝对路径")
                        .desired_width(520.0)
                        .interactive(self.receiver.is_none()),
                )
                .changed()
            {
                self.catalog = None;
                self.data = None;
            }
            if ui
                .add_enabled(self.receiver.is_none(), egui::Button::new("选择文件…"))
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("SQLite", &["sqlite", "sqlite3", "db"])
                    .pick_file()
            {
                self.path = path.display().to_string();
                self.catalog = None;
                self.data = None;
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.receiver.is_none() && !self.path.trim().is_empty(),
                    egui::Button::new("打开数据库"),
                )
                .clicked()
            {
                self.start_catalog();
            }
            let backup_button = ui
                .add_enabled(
                    self.receiver.is_none() && !self.path.trim().is_empty(),
                    egui::Button::new("备份数据库…"),
                )
                .on_hover_text("创建一致的本机 SQLite 副本；目标已存在时拒绝覆盖，可随时取消。");
            if backup_button.clicked() {
                let source = PathBuf::from(self.path.trim().trim_matches('"'));
                let stem = source.file_stem().map_or_else(
                    || "database".to_owned(),
                    |value| value.to_string_lossy().into_owned(),
                );
                backup_destination = rfd::FileDialog::new()
                    .add_filter("SQLite", &["sqlite", "sqlite3", "db"])
                    .set_file_name(format!("{stem}-backup.sqlite"))
                    .save_file();
            }
            if self.receiver.is_some() {
                let label = if self.backup_progress.is_some() {
                    "停止备份"
                } else {
                    "停止读取"
                };
                if ui.button(label).clicked() {
                    self.cancel.store(true, Ordering::Relaxed);
                }
                ui.spinner();
                ui.ctx().request_repaint_after(Duration::from_millis(120));
            }
            if let Some(progress) = &self.backup_progress {
                ui.label(format!(
                    "{} · {}%",
                    progress.phase_label(),
                    progress.percent.load(Ordering::Relaxed)
                ));
            }
            if !self.message.is_empty() {
                ui.label(&self.message);
            }
        });
        if let Some(destination) = backup_destination {
            self.start_backup(destination);
        }
        let Some(catalog) = &self.catalog else { return };
        if catalog.more_tables {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "仅列出前 200 个表/视图。可改用其他数据库文件；本版没有任意 SQL 查询。",
            );
        }
        if catalog.tables.is_empty() {
            ui.weak("数据库中没有可浏览的普通表或视图。");
            return;
        }
        ui.add_space(10.0);
        let mut changed = false;
        let mut refresh = false;
        ui.horizontal(|ui| {
            ui.label("表 / 视图");
            egui::ComboBox::from_id_salt("sqlite-table")
                .selected_text(&self.table)
                .width(300.0)
                .show_ui(ui, |ui| {
                    for table in &catalog.tables {
                        changed |= ui
                            .selectable_value(
                                &mut self.table,
                                table.name.clone(),
                                format!("{}  ·  {}", table.name, table.kind),
                            )
                            .changed();
                    }
                });
            if ui
                .add_enabled(self.receiver.is_none(), egui::Button::new("刷新当前表"))
                .clicked()
            {
                refresh = true;
            }
        });
        if (changed || refresh) && self.receiver.is_none() {
            self.edit_review = None;
            self.start_page(0);
        }
        let Some(data) = &self.data else { return };
        ui.horizontal(|ui| {
            ui.heading(&data.table);
            ui.weak(format!(
                "第 {} 页 · {} 行 · {} 列",
                data.page + 1,
                data.rows.len(),
                data.columns.len()
            ));
        });
        ui.collapsing("列结构", |ui| {
            egui::Grid::new("sqlite-schema")
                .striped(true)
                .show(ui, |ui| {
                    ui.strong("列名");
                    ui.strong("声明类型");
                    ui.strong("约束");
                    ui.end_row();
                    for column in &data.columns {
                        ui.monospace(&column.name);
                        ui.label(&column.declared_type);
                        ui.label(format!(
                            "{}{}{}",
                            if column.primary_key { "主键 " } else { "" },
                            if column.not_null { "非空 " } else { "" },
                            if column.hidden { "隐藏" } else { "" }
                        ));
                        ui.end_row();
                    }
                });
        });
        let mut requested_page = None;
        let mut edit_action = 0;
        ui.horizontal(|ui| {
            let send = ui.add_enabled(
                self.receiver.is_none(),
                egui::Button::new("当前页送到数据工作台…"),
            );
            #[cfg(feature = "ui-preview")]
            {
                self.transfer.buttons[0] = Some(send.rect.intersect(ui.clip_rect()));
            }
            if send.clicked() {
                match transfer::Review::prepare(data, &self.path) {
                    Ok(review) => self.transfer.review = Some(review),
                    Err(error) => self.message = format!("无法接力当前页：{error:#}"),
                }
            }
            let selected = self.selected_cell;
            let edit = ui.add_enabled(
                self.receiver.is_none() && selected.is_some() && self.edit_review.is_none(),
                egui::Button::new("编辑选中单元格…"),
            );
            if edit.clicked()
                && let Some((row, column)) = selected
                && let Some(cell) = data.rows.get(row).and_then(|row| row.get(column))
            {
                self.edit_operation = EditOperation::New;
                self.edit_input = cell
                    .value
                    .as_ref()
                    .map_or_else(String::new, ToString::to_string);
                self.edit_dialog = true;
            }
            let delete_row = ui.add_enabled(
                self.receiver.is_none() && selected.is_some() && self.edit_review.is_none(),
                egui::Button::new("删除选中行…"),
            );
            if delete_row.clicked() && selected.is_some() {
                edit_action = 5;
            }
            let can_undo = self.receiver.is_none() && self.edit_review.is_none();
            let undo = ui.add_enabled(
                can_undo && !self.undo_stack.is_empty(),
                egui::Button::new(format!("撤销 ({})", self.undo_stack.len())),
            );
            if undo.clicked()
                && let Some(review) = self.undo_stack.last().cloned()
            {
                self.edit_review = Some(review);
                self.edit_operation = EditOperation::Undo;
            }
            let redo = ui.add_enabled(
                can_undo && !self.redo_stack.is_empty(),
                egui::Button::new(format!("重做 ({})", self.redo_stack.len())),
            );
            if redo.clicked()
                && let Some(review) = self.redo_stack.last().cloned()
            {
                self.edit_review = Some(review);
                self.edit_operation = EditOperation::Redo;
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.receiver.is_none() && data.page > 0,
                    egui::Button::new("上一页"),
                )
                .clicked()
            {
                requested_page = Some(data.page - 1);
            }
            if ui
                .add_enabled(
                    self.receiver.is_none() && data.has_next,
                    egui::Button::new("下一页"),
                )
                .clicked()
            {
                requested_page = Some(data.page + 1);
            }
            if ui
                .add_enabled(
                    self.receiver.is_none() && !data.rows.is_empty(),
                    egui::Button::new("导出当前页 CSV…"),
                )
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_file_name("sqlite-page.csv")
                    .save_file()
            {
                self.message = match export_csv(data, &path) {
                    Ok(()) => format!("当前页已导出：{}", path.display()),
                    Err(error) => format!("导出失败：{error:#}"),
                };
            }
        });
        if data.truncated_cells > 0 {
            ui.colored_label(ui.visuals().warn_fg_color, format!("本页 {} 个值因单格 4 KiB 或整页 2 MiB 预算被截断；CSV 导出也只包含当前预览值。", data.truncated_cells));
        }
        if data.rows.is_empty() {
            ui.weak("当前页没有行。");
        }
        let column_width =
            (ui.available_width() / data.columns.len().max(1) as f32 - 14.0).clamp(120.0, 280.0);
        egui::ScrollArea::both().max_height(430.0).show(ui, |ui| {
            egui::Grid::new("sqlite-rows")
                .striped(true)
                .min_col_width(column_width)
                .show(ui, |ui| {
                    for column in &data.columns {
                        ui.add_sized(
                            [column_width, 26.0],
                            egui::Label::new(egui::RichText::new(&column.name).strong()).truncate(),
                        );
                    }
                    ui.end_row();
                    for (row_index, row) in data.rows.iter().enumerate() {
                        for (column_index, cell) in row.iter().enumerate() {
                            let short: String = cell.display.chars().take(80).collect();
                            let preview = if cell.display.chars().count() > 80 {
                                format!("{short}…")
                            } else {
                                short
                            };
                            let response = ui
                                .add_sized(
                                    [column_width, 30.0],
                                    egui::Button::selectable(
                                        self.selected_cell == Some((row_index, column_index)),
                                        egui::RichText::new(preview).monospace(),
                                    ),
                                )
                                .on_hover_text(format!("{} · {}", cell.kind, cell.display));
                            if response.clicked() {
                                self.selected_cell = Some((row_index, column_index));
                            }
                        }
                        ui.end_row();
                    }
                });
        });
        ui.add_space(8.0);
        ui.small("最多列出200个表/视图、128列，预览前10000行。编辑仅支持有主键的普通表单个非主键标量；含触发器、外键的数据库暂不支持写入。变更先在事务中预览后回滚，确认时复核原值再提交。CSV只导出当前页且不覆盖文件；WAL模式下SQLite可能管理辅助文件。");
        if let Some(next) = requested_page {
            self.edit_review = None;
            self.start_page(next);
        }

        if self.edit_dialog {
            let response = egui::Modal::new(ui.id().with("sqlite-cell-edit")).show(ui.ctx(), |ui| {
                ui.set_max_width((ui.ctx().screen_rect().width() - 48.0).clamp(220.0, 620.0));
                ui.heading("预览单元格修改");
                if let Some((row, column)) = self.selected_cell
                    && let Some(cell) = self.data.as_ref().and_then(|data| data.rows.get(row).and_then(|row| row.get(column)))
                {
                    let column_name = self.data.as_ref().and_then(|data| data.columns.get(column)).map_or("?", |column| column.name.as_str());
                    ui.label(format!("{} · 第{}行 / {} · 当前值：{}", self.data.as_ref().map_or("", |data| data.table.as_str()), row + 1, column_name, cell.display));
                    ui.label("新值按 JSON 标量输入：文本写成 \"文字\"，也可输入数字、true、false 或 null。");
                }
                ui.add(egui::TextEdit::singleline(&mut self.edit_input).desired_width(480.0));
                if self.receiver.is_some() {
                    ui.horizontal(|ui| { ui.spinner(); ui.label("正在检查；预览事务结束后会自动回滚…"); });
                    ui.ctx().request_repaint_after(Duration::from_millis(120));
                }
                ui.horizontal(|ui| {
                    let cancel = ui.add_enabled(self.receiver.is_none(), egui::Button::new("取消"));
                    let preview = ui.add_enabled(self.receiver.is_none(), egui::Button::new("预览变更"));
                    if cancel.clicked() { edit_action = 1; }
                    if preview.clicked() { edit_action = 2; }
                });
            });
            if response.should_close() && self.receiver.is_none() {
                edit_action = 1;
            }
        }
        if edit_action == 1 {
            self.edit_dialog = false;
        } else if edit_action == 2
            && self.receiver.is_none()
            && let Some((row, column)) = self.selected_cell
        {
            self.start_edit_preview(row, column);
        }

        if edit_action == 5
            && self.receiver.is_none()
            && let Some((row, _)) = self.selected_cell
        {
            self.edit_operation = EditOperation::New;
            self.start_delete_preview(row);
        }

        if let Some(review) = &self.edit_review {
            let response =
                egui::Modal::new(ui.id().with("sqlite-cell-edit-review")).show(ui.ctx(), |ui| {
                    ui.set_max_width((ui.ctx().screen_rect().width() - 48.0).clamp(220.0, 620.0));
                    ui.heading(review.title(self.edit_operation));
                    let database_path = self
                        .catalog
                        .as_ref()
                        .map(|catalog| catalog.path.display().to_string())
                        .unwrap_or_default();
                    ui.label(database_path);
                    ui.label(format!("表 {} · {}", review.table(), review.change_label()));
                    ui.label(format!("主键定位：{}", review.key_summary()));
                    egui::ScrollArea::vertical()
                        .max_height(190.0)
                        .show(ui, |ui| {
                            for field in review.preview_details() {
                                ui.monospace(field);
                            }
                        });
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        review.warning(self.edit_operation),
                    );
                    if self.receiver.is_some() {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("正在复核并提交；请勿关闭应用…");
                        });
                        ui.ctx().request_repaint_after(Duration::from_millis(120));
                    }
                    ui.horizontal(|ui| {
                        let cancel = ui.add_enabled(
                            self.receiver.is_none(),
                            egui::Button::new("取消，不写入"),
                        );
                        let apply = ui.add_enabled(
                            self.receiver.is_none(),
                            egui::Button::new(review.button(self.edit_operation)),
                        );
                        if cancel.clicked() {
                            edit_action = 3;
                        }
                        if apply.clicked() {
                            edit_action = 4;
                        }
                    });
                });
            if response.should_close() && self.receiver.is_none() {
                edit_action = 3;
            }
        }
        if edit_action == 3 {
            self.edit_review = None;
            self.edit_dialog = false;
        } else if edit_action == 4 && self.receiver.is_none() {
            self.start_edit_apply();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        let path = std::env::temp_dir().join(format!("zidt-sqlite-{}.db", uuid::Uuid::new_v4()));
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE \"notes\"\"demo\" (id INTEGER PRIMARY KEY, text TEXT, payload BLOB); CREATE VIEW recent AS SELECT id, text FROM \"notes\"\"demo\";").unwrap();
        for index in 0..55 {
            connection
                .execute(
                    "INSERT INTO \"notes\"\"demo\" (text,payload) VALUES (?1,?2)",
                    rusqlite::params![format!("row-{index}"), [0xABu8, 0xCD]],
                )
                .unwrap();
        }
        path
    }

    #[test]
    fn lists_and_pages_unusual_identifiers_without_writes() {
        let path = fixture();
        let signal = Arc::new(AtomicBool::new(false));
        let before = fs::read(&path).unwrap();
        let catalog = list_tables(&path, &signal).unwrap();
        assert!(
            catalog
                .tables
                .iter()
                .any(|table| table.name == "notes\"demo")
        );
        assert!(catalog.tables.iter().any(|table| table.name == "recent"));
        let first = load_page(&path, "notes\"demo", 0, &signal).unwrap();
        assert_eq!(first.rows.len(), 50);
        assert!(first.has_next);
        assert_eq!(first.columns.len(), 3);
        assert_eq!(first.rows[0][1].display, "row-0");
        assert_eq!(first.rows[0][2].kind, "二进制预览");
        let second = load_page(&path, "notes\"demo", 1, &signal).unwrap();
        assert_eq!(second.rows.len(), 5);
        assert!(!second.has_next);
        assert!(load_page(&path, "notes\"demo\"; DROP TABLE recent; --", 0, &signal).is_err());
        assert_eq!(list_tables(&path, &signal).unwrap().tables.len(), 2);
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_non_database_and_cancelled_operations() {
        let path =
            std::env::temp_dir().join(format!("zidt-not-sqlite-{}.db", uuid::Uuid::new_v4()));
        fs::write(&path, b"not a database").unwrap();
        let signal = Arc::new(AtomicBool::new(false));
        assert!(list_tables(&path, &signal).is_err());
        fs::remove_file(path).unwrap();
        let path = fixture();
        signal.store(true, Ordering::Relaxed);
        assert!(list_tables(&path, &signal).is_err());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn export_does_not_replace_existing_file_and_escapes_formula_text() {
        let path = fixture();
        let signal = Arc::new(AtomicBool::new(false));
        let mut data = load_page(&path, "notes\"demo", 0, &signal).unwrap();
        data.rows[0][1].export = "=HYPERLINK(\"x\")".into();
        let output = path.with_extension("csv");
        export_csv(&data, &output).unwrap();
        let saved = fs::read_to_string(&output).unwrap();
        assert!(saved.contains("'=HYPERLINK"));
        assert!(export_csv(&data, &output).is_err());
        fs::remove_file(output).unwrap();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn slow_view_can_be_cancelled() {
        let path = fixture();
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("CREATE VIEW slow AS WITH RECURSIVE seq(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM seq WHERE n < 10000000) SELECT n FROM seq ORDER BY n DESC;").unwrap();
        drop(connection);
        let signal = Arc::new(AtomicBool::new(false));
        let worker_signal = Arc::clone(&signal);
        let worker_path = path.clone();
        let started = Instant::now();
        let worker = std::thread::spawn(move || load_page(&worker_path, "slow", 0, &worker_signal));
        std::thread::sleep(Duration::from_millis(30));
        signal.store(true, Ordering::Relaxed);
        assert!(worker.join().unwrap().is_err());
        assert!(started.elapsed() < Duration::from_secs(6));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn large_values_are_only_returned_as_previews() {
        let path = fixture();
        let connection = Connection::open(&path).unwrap();
        let blob = vec![0xEFu8; 1024 * 1024];
        connection
            .execute(
                "INSERT INTO \"notes\"\"demo\" (text,payload) VALUES (?1,?2)",
                rusqlite::params!["多".repeat(4000), blob],
            )
            .unwrap();
        drop(connection);
        let signal = Arc::new(AtomicBool::new(false));
        let data = load_page(&path, "notes\"demo", 1, &signal).unwrap();
        let last = data.rows.last().unwrap();
        assert!(last[1].truncated);
        assert!(last[1].display.len() < 4200);
        assert!(last[2].truncated);
        assert!(last[2].display.contains("1048576 B"));
        assert!(last[2].display.len() < 100);
        fs::remove_file(path).unwrap();
    }
}
