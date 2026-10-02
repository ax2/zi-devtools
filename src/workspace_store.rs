//! Explicit local snapshots. Transactions and revision checks protect saved work.
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::{fs, path::PathBuf, time::Duration};

pub const MAX_SNAPSHOT: usize = 64 * 1024 * 1024;
const MAX_TOTAL: i64 = 128 * 1024 * 1024;
const MAX_ENTRIES: i64 = 256;

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub revision: i64,
    pub updated: String,
    pub bytes: i64,
}

#[derive(Clone)]
pub struct Store {
    pub path: PathBuf,
}

pub fn validate_name(name: &str) -> Result<()> {
    ensure!(
        !name.trim().is_empty()
            && name.chars().count() <= 80
            && !name.chars().any(char::is_control),
        "名称须为 1–80 字且不能包含控制字符"
    );
    Ok(())
}

fn validate_id(id: &str) -> Result<()> {
    ensure!(uuid::Uuid::parse_str(id)?.to_string() == id, "实例标识无效");
    Ok(())
}

impl Store {
    fn connect(&self, write: bool) -> Result<Connection> {
        if let Ok(meta) = fs::symlink_metadata(&self.path) {
            ensure!(
                meta.is_file() && !meta.file_type().is_symlink(),
                "工作实例库不是普通文件"
            );
            ensure!(
                meta.len() <= 192 * 1024 * 1024,
                "工作实例库超过文件大小上限"
            );
        }
        if write && let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let flags = if write {
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        };
        let db = Connection::open_with_flags(&self.path, flags).context("无法打开工作实例库")?;
        db.busy_timeout(Duration::from_secs(2))?;
        let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version == 0 && write {
            let tables: i64 = db.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
                [],
                |r| r.get(0),
            )?;
            ensure!(tables == 0, "该数据库不是工作实例库，未修改其内容");
            db.execute_batch("PRAGMA auto_vacuum=FULL; BEGIN IMMEDIATE; CREATE TABLE snapshots(id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL, revision INTEGER NOT NULL, updated TEXT NOT NULL, payload BLOB NOT NULL); PRAGMA user_version=1; COMMIT;")?;
        } else {
            ensure!(version == 1, "工作实例库版本不支持，原文件保持不变");
        }
        Ok(db)
    }

    pub fn list(&self) -> Result<Vec<Entry>> {
        if !self.path.try_exists()? {
            return Ok(Vec::new());
        }
        let db = self.connect(false)?;
        let mut query = db.prepare("SELECT id,name,kind,revision,updated,length(payload) FROM snapshots ORDER BY updated DESC,id LIMIT 257")?;
        let rows = query
            .query_map([], |r| {
                Ok(Entry {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    kind: r.get(2)?,
                    revision: r.get(3)?,
                    updated: r.get(4)?,
                    bytes: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ensure!(rows.len() <= MAX_ENTRIES as usize, "工作实例库条目超过上限");
        for row in &rows {
            validate_id(&row.id)?;
            validate_name(&row.name)?;
            ensure!(
                row.revision > 0 && (0..=MAX_SNAPSHOT as i64).contains(&row.bytes),
                "工作实例元数据无效"
            );
        }
        Ok(rows)
    }

    pub fn load(&self, id: &str) -> Result<(Entry, Vec<u8>)> {
        validate_id(id)?;
        let db = self.connect(false)?;
        let size: i64 = db.query_row(
            "SELECT length(payload) FROM snapshots WHERE id=?1",
            [id],
            |r| r.get(0),
        )?;
        ensure!((0..=MAX_SNAPSHOT as i64).contains(&size), "保存内容过大");
        let (entry, bytes) = db.query_row("SELECT name,kind,revision,updated,payload FROM snapshots WHERE id=?1 AND length(payload)<=?2", params![id, MAX_SNAPSHOT as i64], |r| Ok((Entry { id:id.into(), name:r.get(0)?, kind:r.get(1)?, revision:r.get(2)?, updated:r.get(3)?, bytes:size }, r.get::<_,Vec<u8>>(4)?)))?;
        validate_name(&entry.name)?;
        ensure!(
            entry.revision > 0 && bytes.len() <= MAX_SNAPSHOT,
            "保存版本或内容无效"
        );
        Ok((entry, bytes))
    }

    pub fn save(
        &self,
        id: &str,
        name: &str,
        kind: &str,
        payload: &[u8],
        expected: Option<i64>,
    ) -> Result<i64> {
        validate_id(id)?;
        validate_name(name)?;
        ensure!(kind == "data-v1", "不支持的工作实例类型");
        ensure!(
            payload.len() <= MAX_SNAPSHOT,
            "单个快照最多 64 MiB，请缩小材料"
        );
        let mut db = self.connect(true)?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current: Option<i64> = tx
            .query_row("SELECT revision FROM snapshots WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()?;
        ensure!(
            current == expected,
            "保存版本已变化；请另存副本或重新打开，未覆盖现有数据"
        );
        let (count, total): (i64, i64) = tx.query_row(
            "SELECT COUNT(*),COALESCE(SUM(length(payload)),0) FROM snapshots WHERE id<>?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        ensure!(
            count < MAX_ENTRIES && total + payload.len() as i64 <= MAX_TOTAL,
            "实例库最多 256 项 / 128 MiB；请先检查并删除不需要的保存项"
        );
        let next = current
            .unwrap_or(0)
            .checked_add(1)
            .context("保存版本已到上限")?;
        tx.execute("INSERT INTO snapshots(id,name,kind,revision,updated,payload) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET name=excluded.name,revision=excluded.revision,updated=excluded.updated,payload=excluded.payload", params![id,name.trim(),kind,next,chrono::Utc::now().to_rfc3339(),payload])?;
        tx.commit()?;
        Ok(next)
    }

    pub fn delete(&self, id: &str, expected: i64) -> Result<()> {
        validate_id(id)?;
        let db = self.connect(true)?;
        ensure!(
            db.execute(
                "DELETE FROM snapshots WHERE id=?1 AND revision=?2",
                params![id, expected]
            )? == 1,
            "保存项已变化或被删除，请刷新后重试"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshots_are_explicit_transactional_and_revision_checked() {
        let root = std::env::temp_dir().join(format!("zi-workspace-{}", uuid::Uuid::new_v4()));
        let store = Store {
            path: root.join("workspace.sqlite3"),
        };
        assert!(store.list().unwrap().is_empty());
        assert!(!root.exists());
        let id = uuid::Uuid::new_v4().to_string();
        assert_eq!(store.save(&id, "订单", "data-v1", b"one", None).unwrap(), 1);
        assert!(store.save(&id, "覆盖", "data-v1", b"bad", None).is_err());
        assert_eq!(
            store
                .save(&id, "订单二版", "data-v1", b"two", Some(1))
                .unwrap(),
            2
        );
        assert!(store.save(&id, "过期", "data-v1", b"bad", Some(1)).is_err());
        assert!(store.delete(&id, 1).is_err());
        assert_eq!(store.load(&id).unwrap().1, b"two");
        assert_eq!(store.list().unwrap()[0].name, "订单二版");
        store.delete(&id, 2).unwrap();
        assert!(store.list().unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn incompatible_database_and_invalid_input_are_not_overwritten() {
        let root = std::env::temp_dir().join(format!("zi-workspace-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let store = Store {
            path: root.join("workspace.sqlite3"),
        };
        let db = Connection::open(&store.path).unwrap();
        db.execute_batch("PRAGMA user_version=99;").unwrap();
        drop(db);
        let before = fs::read(&store.path).unwrap();
        assert!(
            store
                .save(
                    &uuid::Uuid::new_v4().to_string(),
                    "test",
                    "data-v1",
                    b"x",
                    None
                )
                .is_err()
        );
        assert_eq!(fs::read(&store.path).unwrap(), before);
        assert!(validate_name("\n").is_err());
        assert!(validate_id("../escape").is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
