//! Shared, bounded SQLite row reading and spreadsheet-safe CSV serialization.
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, types::ValueRef};

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

#[derive(Clone, Copy)]
pub struct PageLimits {
    pub page_size: usize,
    pub max_pages: usize,
    pub max_columns: usize,
    pub max_cell_bytes: usize,
    pub max_page_bytes: usize,
}

pub fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn bounded_text(bytes: &[u8], max_cell_bytes: usize, remaining: &mut usize) -> (String, bool) {
    let limit = bytes.len().min(max_cell_bytes).min(*remaining);
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

pub fn cell(
    value: ValueRef<'_>,
    original_length: Option<i64>,
    max_cell_bytes: usize,
    remaining: &mut usize,
) -> Cell {
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
            let (content, truncated) = bounded_text(bytes, max_cell_bytes, remaining);
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
    connection: &Connection,
    table: &str,
    page: usize,
    limits: PageLimits,
    check_cancel: impl Fn() -> Result<()>,
) -> Result<PageData> {
    ensure!(page < limits.max_pages, "页码超出预览范围");
    check_cancel()?;
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
        ensure!(
            columns.len() <= limits.max_columns,
            "列数超过{}，暂不支持预览",
            limits.max_columns
        );
    }
    ensure!(!columns.is_empty(), "无法获取列结构");
    let header_sql = format!("SELECT * FROM {} LIMIT 0", quote_identifier(table));
    let header_statement = connection
        .prepare(&header_sql)
        .context("无法准备列清单查询")?;
    ensure!(
        header_statement.column_count() <= limits.max_columns,
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
                "CASE typeof({quoted}) WHEN 'text' THEN CAST(substr(CAST({quoted} AS BLOB),1,?3) AS TEXT) \
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
    let mut remaining = limits.max_page_bytes;
    let mut rows = Vec::new();
    let mut has_next = false;
    let mut query = statement.query(rusqlite::params![
        limits.page_size as i64 + 1,
        (page * limits.page_size) as i64,
        (limits.max_cell_bytes + 1) as i64
    ])?;
    let mut truncated_cells = 0;
    while let Some(row) = query.next().context("预览查询失败或已超时")? {
        check_cancel()?;
        if rows.len() == limits.page_size {
            has_next = true;
            break;
        }
        let mut cells = Vec::with_capacity(columns.len());
        for index in 0..columns.len() {
            let value = cell(
                row.get_ref(index * 2)?,
                row.get(index * 2 + 1)?,
                limits.max_cell_bytes,
                &mut remaining,
            );
            truncated_cells += usize::from(value.truncated);
            cells.push(value);
        }
        rows.push(cells);
    }
    has_next &= page + 1 < limits.max_pages;
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

pub fn write_csv(data: &PageData, output: impl std::io::Write) -> Result<()> {
    ensure!(!data.rows.is_empty(), "当前页没有可导出的行");
    let mut writer = csv::Writer::from_writer(output);
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
