//! Shared one-table SQLite encoding for reviewed native saves and resource guests.
use crate::Dataset;
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, params_from_iter, types::Value as SqlValue};
use serde_json::Value;
use std::collections::BTreeSet;

fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

pub fn validate_identifiers(table: &str, headers: &[String]) -> Result<()> {
    for name in std::iter::once(table).chain(headers.iter().map(String::as_str)) {
        ensure!(
            !name.trim().is_empty() && name.len() <= 512 && !name.contains('\0'),
            "表名或列名为空、含空字符或超过512字节"
        );
    }
    ensure!(
        !table.to_ascii_lowercase().starts_with("sqlite_"),
        "表名不能使用SQLite保留前缀sqlite_"
    );
    let mut columns = BTreeSet::new();
    for name in headers {
        ensure!(
            columns.insert(name.to_ascii_lowercase()),
            "SQLite列名不能仅大小写不同"
        );
    }
    Ok(())
}

fn sql_value(value: &Value) -> Result<SqlValue> {
    Ok(match value {
        Value::Null => SqlValue::Null,
        Value::Bool(v) => SqlValue::Integer(i64::from(*v)),
        Value::String(s) => SqlValue::Text(s.clone()),
        Value::Number(n) => {
            if let Some(v) = n.as_i64() {
                SqlValue::Integer(v)
            } else if n.is_u64() {
                SqlValue::Text(n.to_string())
            } else {
                SqlValue::Real(n.as_f64().context("数值无法表示")?)
            }
        }
        Value::Array(_) | Value::Object(_) => SqlValue::Text(serde_json::to_string(value)?),
    })
}

pub fn create_verified_table(
    connection: &mut Connection,
    table: &str,
    data: &Dataset,
    check_cancel: impl Fn() -> Result<()>,
) -> Result<usize> {
    validate_identifiers(table, &data.headers)?;
    let table = quote_identifier(table);
    let columns = data
        .headers
        .iter()
        .map(|name| quote_identifier(name))
        .collect::<Vec<_>>();
    let transaction = connection.transaction()?;
    // No declared affinity: CSV strings and mixed JSON cell types are not coerced.
    transaction.execute_batch(&format!("CREATE TABLE {table} ({})", columns.join(",")))?;
    {
        let placeholders = vec!["?"; columns.len()].join(",");
        let mut insert =
            transaction.prepare(&format!("INSERT INTO {table} VALUES ({placeholders})"))?;
        for row in &data.rows {
            check_cancel()?;
            let values = row.iter().map(sql_value).collect::<Result<Vec<_>>>()?;
            insert.execute(params_from_iter(values))?;
        }
    }
    check_cancel()?;
    transaction.commit()?;
    let rows: usize =
        connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })?;
    let integrity: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    ensure!(
        rows == data.rows.len() && integrity == "ok",
        "写入校验失败，未发布目标文件"
    );
    Ok(rows)
}
