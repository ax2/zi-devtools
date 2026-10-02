use super::*;
use rusqlite::{Connection, OpenFlags, params};
use std::path::Path;

fn connect(path: &Path, write: bool) -> Result<Connection> {
    if path.exists() {
        let meta = std::fs::symlink_metadata(path)?;
        ensure!(
            meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= 64 * 1024 * 1024,
            "日常工具数据库不是普通文件或超过 64 MiB"
        );
    } else if write && let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let connection = Connection::open_with_flags(
        path,
        if write {
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        },
    )?;
    connection.busy_timeout(std::time::Duration::from_secs(2))?;
    let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
    ensure!(
        version == 1 || (write && version == 0),
        "日常工具数据库版本不支持"
    );
    if version == 0 {
        connection.execute_batch("BEGIN IMMEDIATE; CREATE TABLE IF NOT EXISTS records(id TEXT PRIMARY KEY, revision INTEGER NOT NULL, payload TEXT NOT NULL); PRAGMA user_version=1; COMMIT;")?;
    }
    Ok(connection)
}
pub(super) fn load(path: &Path) -> Result<Vec<Item>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let conn = connect(path, false)?;
    let (count, bytes): (i64, i64) = conn.query_row(
        "SELECT count(*), coalesce(sum(length(CAST(payload AS BLOB))),0) FROM records",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    ensure!(
        count <= MAX_ITEMS as i64 && bytes <= 32 * 1024 * 1024,
        "本地记录超过容量限制"
    );
    let mut statement = conn.prepare("SELECT id, revision, payload FROM records")?;
    let rows = statement.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut result = Vec::new();
    for row in rows {
        let (id, revision, payload) = row?;
        ensure!(payload.len() <= MAX_BODY + 8192, "记录过大");
        let item: Item = serde_json::from_str(&payload)?;
        item.validate()?;
        ensure!(
            item.id == id && item.revision == revision,
            "记录标识或版本不一致"
        );
        result.push(item);
    }
    result.sort_by_key(|i| {
        (
            std::cmp::Reverse(i.pinned),
            std::cmp::Reverse(i.updated),
            i.id.clone(),
        )
    });
    Ok(result)
}
pub(super) fn save(path: &Path, mut item: Item) -> Result<()> {
    item.validate()?;
    let mut conn = connect(path, true)?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let expected = item.revision;
    item.revision = expected
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("记录版本溢出"))?;
    item.updated = Local::now().timestamp();
    let payload = serde_json::to_string(&item)?;
    let (count, bytes): (i64, i64) = tx.query_row("SELECT count(*), coalesce(sum(length(CAST(payload AS BLOB))),0) FROM records WHERE id != ?1", [&item.id], |r| Ok((r.get(0)?, r.get(1)?)))?;
    ensure!(
        count < MAX_ITEMS as i64 && bytes + payload.len() as i64 <= 32 * 1024 * 1024,
        "已达到 2000 条或 32 MiB 保存上限（含回收站）"
    );
    if expected == 0 {
        tx.execute(
            "INSERT INTO records VALUES(?1,?2,?3)",
            params![item.id, item.revision, payload],
        )?;
    } else {
        ensure!(
            tx.execute(
                "UPDATE records SET revision=?2,payload=?3 WHERE id=?1 AND revision=?4",
                params![item.id, item.revision, payload, expected]
            )? == 1,
            "记录已被另一窗口修改；请复制当前内容，放弃编辑并重新加载后重试"
        );
    }
    tx.commit()?;
    Ok(())
}
