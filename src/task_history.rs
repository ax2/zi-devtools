//! Optional, privacy-bounded history of completed background tasks.
//!
//! Only fixed tool keys/titles, terminal state, elapsed time, and completion
//! time are stored. Instance names, summaries, inputs, outputs and paths are
//! deliberately excluded.
use crate::tasks::{Phase, Row};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, params};
use std::{fs, path::Path, time::Duration};

const SCHEMA_VERSION: i64 = 1;
pub const MAX_RECORDS: usize = 1000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub key: String,
    pub title: String,
    pub phase: Phase,
    pub elapsed: Duration,
    pub completed_at: i64,
}

fn validate_path(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(metadata.file_type().is_file(), "任务历史目标不是普通文件");
            ensure!(metadata.len() <= 1024 * 1024, "任务历史数据库超过 1 MiB");
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn open(path: &Path, create: bool) -> Result<Option<Connection>> {
    if !validate_path(path)? && !create {
        return Ok(None);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(path).context("无法打开本机任务历史")?;
    connection.busy_timeout(Duration::from_secs(2))?;
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if create && version == 0 {
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS task_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_key TEXT NOT NULL CHECK(length(task_key) BETWEEN 1 AND 96),
                title TEXT NOT NULL CHECK(length(title) BETWEEN 1 AND 128),
                phase TEXT NOT NULL CHECK(phase IN ('done', 'failed', 'cancelled')),
                elapsed_ms INTEGER NOT NULL CHECK(elapsed_ms >= 0),
                completed_at INTEGER NOT NULL
            );
            PRAGMA user_version = 1;",
        )?;
    }
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    ensure!(
        version == SCHEMA_VERSION,
        "任务历史版本不受支持，原文件未修改"
    );
    Ok(Some(connection))
}

fn phase_name(phase: Phase) -> Result<&'static str> {
    match phase {
        Phase::Done => Ok("done"),
        Phase::Failed => Ok("failed"),
        Phase::Cancelled => Ok("cancelled"),
        _ => anyhow::bail!("只允许保存已结束任务"),
    }
}

fn parse_phase(value: &str) -> Result<Phase> {
    match value {
        "done" => Ok(Phase::Done),
        "failed" => Ok(Phase::Failed),
        "cancelled" => Ok(Phase::Cancelled),
        _ => anyhow::bail!("任务历史状态无效"),
    }
}

fn validate_labels(key: &str, title: &str) -> Result<()> {
    ensure!(
        !key.is_empty()
            && key.len() <= 96
            && key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
        "任务历史标识无效"
    );
    ensure!(
        !title.trim().is_empty()
            && title.chars().count() <= 128
            && !title.chars().any(char::is_control),
        "任务历史标题无效"
    );
    Ok(())
}

pub fn load(path: &Path) -> Result<Vec<Record>> {
    let Some(connection) = open(path, false)? else {
        return Ok(Vec::new());
    };
    let mut statement = connection.prepare(
        "SELECT task_key, title, phase, elapsed_ms, completed_at
         FROM task_history ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = statement.query_map([MAX_RECORDS as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;
    let mut records = Vec::new();
    for row in rows {
        let (key, title, phase, elapsed_ms, completed_at) = row?;
        validate_labels(&key, &title)?;
        ensure!(elapsed_ms >= 0, "任务历史耗时无效");
        records.push(Record {
            key,
            title,
            phase: parse_phase(&phase)?,
            elapsed: Duration::from_millis(elapsed_ms as u64),
            completed_at,
        });
    }
    Ok(records)
}

pub fn append(path: &Path, row: &Row) -> Result<Record> {
    let phase = phase_name(row.phase)?;
    validate_labels(row.key, row.title)?;
    let connection = open(path, true)?.expect("create=true always returns a connection");
    let transaction = connection.unchecked_transaction()?;
    let completed_at = chrono::Utc::now().timestamp();
    let elapsed_ms = row.elapsed.as_millis().min(i64::MAX as u128) as i64;
    transaction.execute(
        "INSERT INTO task_history (task_key, title, phase, elapsed_ms, completed_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![row.key, row.title, phase, elapsed_ms, completed_at],
    )?;
    transaction.execute(
        "DELETE FROM task_history WHERE id NOT IN
         (SELECT id FROM task_history ORDER BY id DESC LIMIT ?1)",
        [MAX_RECORDS as i64],
    )?;
    transaction.commit()?;
    Ok(Record {
        key: row.key.to_owned(),
        title: row.title.to_owned(),
        phase: row.phase,
        elapsed: Duration::from_millis(elapsed_ms as u64),
        completed_at,
    })
}

pub fn clear(path: &Path) -> Result<()> {
    let Some(connection) = open(path, false)? else {
        return Ok(());
    };
    connection.pragma_update(None, "secure_delete", "ON")?;
    connection.execute("DELETE FROM task_history", [])?;
    connection.execute_batch("VACUUM")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path() -> std::path::PathBuf {
        std::env::temp_dir()
            .join(format!("zi-task-history-{}", uuid::Uuid::new_v4()))
            .join("history.sqlite3")
    }

    fn completed_row() -> Row {
        Row {
            owner_id: uuid::Uuid::new_v4(),
            key: "files",
            instance: Some("private-folder-name".into()),
            instance_name: Some("private instance".into()),
            generation: 12,
            title: "批量文件校验",
            phase: Phase::Done,
            elapsed: Duration::from_millis(1375),
            progress: Some(1.0),
            summary: "private output and path".into(),
            cancellable: false,
        }
    }

    #[test]
    fn stores_only_bounded_non_sensitive_completion_metadata() {
        let path = temp_path();
        let written = append(&path, &completed_row()).unwrap();
        assert_eq!(written.phase, Phase::Done);
        assert_eq!(written.elapsed, Duration::from_millis(1375));
        let loaded = load(&path).unwrap();
        assert_eq!(loaded, vec![written]);
        let raw = fs::read(&path).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("private"));
        clear(&path).unwrap();
        assert!(load(&path).unwrap().is_empty());
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn rejects_nonterminal_rows_and_unexpected_schema() {
        let path = temp_path();
        let mut row = completed_row();
        row.phase = Phase::Running;
        assert!(append(&path, &row).is_err());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let connection = Connection::open(&path).unwrap();
        connection.pragma_update(None, "user_version", 99).unwrap();
        drop(connection);
        assert!(load(&path).is_err());
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn retains_only_the_newest_one_thousand_rows() {
        let path = temp_path();
        let mut row = completed_row();
        for _ in 0..MAX_RECORDS + 7 {
            append(&path, &row).unwrap();
        }
        row.phase = Phase::Failed;
        append(&path, &row).unwrap();
        let records = load(&path).unwrap();
        assert_eq!(records.len(), MAX_RECORDS);
        assert_eq!(records[0].phase, Phase::Failed);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
