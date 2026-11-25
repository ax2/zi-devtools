//! Strict, in-memory reader for a standalone ZiDevTools knowledge index v1.
use rusqlite::{
    Connection, DatabaseName, Error, ErrorCode,
    config::DbConfig,
    ffi,
    hooks::{AuthAction, AuthContext, Authorization},
    limits::Limit,
    serialize::OwnedData,
};
use std::{
    ptr::NonNull,
    time::{Duration, Instant},
};

pub const MAX_SNAPSHOT: usize = 128 * 1024;
pub const MAX_CHUNKS: u32 = 32;
const MAX_CORPUS_TEXT: u32 = 4096;
const MAX_TERM_WORK: u32 = 12_288;
const PAGE: usize = 4096;
const SCHEMA: &str = include_str!("schema-v1.json");
type SchemaObject = (String, String, String, Option<String>);

#[derive(Clone, Copy)]
pub enum SnapshotError {
    Invalid,
    NonStatic,
    Schema,
    Integrity,
    RowLimit,
    Budget,
}

fn run_error(error: Error, fallback: SnapshotError) -> SnapshotError {
    if error.sqlite_error_code() == Some(ErrorCode::OperationInterrupted) {
        SnapshotError::Budget
    } else {
        fallback
    }
}

fn check_header(bytes: &[u8]) -> Result<(), SnapshotError> {
    if bytes.len() < PAGE
        || bytes.len() > MAX_SNAPSHOT
        || !bytes.starts_with(b"SQLite format 3\0")
        || bytes[16..18] != [16, 0]
        || !bytes.len().is_multiple_of(PAGE)
        || bytes[56..60] != [0, 0, 0, 1]
    {
        return Err(SnapshotError::Invalid);
    }
    if bytes[18..20] != [1, 1] {
        return Err(SnapshotError::NonStatic);
    }
    let declared = u32::from_be_bytes(bytes[28..32].try_into().unwrap()) as usize;
    if declared != bytes.len() / PAGE || bytes[24..28] != bytes[92..96] {
        return Err(SnapshotError::Invalid);
    }
    Ok(())
}

pub fn open(bytes: &[u8], fts_term_count: u32) -> Result<Connection, SnapshotError> {
    check_header(bytes)?;
    let mut db = Connection::open_in_memory().map_err(|_| SnapshotError::Invalid)?;
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)
        .map_err(|_| SnapshotError::Invalid)?;
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)
        .map_err(|_| SnapshotError::Invalid)?;
    db.set_limit(Limit::SQLITE_LIMIT_LENGTH, 16 * 1024);
    db.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0);
    let allocation =
        NonNull::new(unsafe { ffi::sqlite3_malloc64(bytes.len() as u64) }.cast::<u8>())
            .ok_or(SnapshotError::Invalid)?;
    // SQLite owns this memory. The provider never resolves a database path or companion file.
    let owned = unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), allocation.as_ptr(), bytes.len());
        OwnedData::from_raw_nonnull(allocation, bytes.len())
    };
    db.deserialize(DatabaseName::Main, owned, false)
        .map_err(|_| SnapshotError::Invalid)?;
    db.pragma_update(None, "query_only", true)
        .map_err(|_| SnapshotError::Invalid)?;
    db.authorizer(Some(|context: AuthContext<'_>| authorize(context, false)));
    let started = Instant::now();
    let mut steps = 0;
    db.progress_handler(
        100,
        Some(move || {
            steps += 100;
            steps > 40_000 || started.elapsed() > Duration::from_millis(800)
        }),
    );
    validate_schema(&db)?;
    validate_data(&db, fts_term_count)?;
    validate_postings(&db)?;
    let same = db
        .serialize(DatabaseName::Main)
        .map_err(|_| SnapshotError::Integrity)?;
    if &*same != bytes {
        return Err(SnapshotError::Integrity);
    }
    drop(same);
    Ok(db)
}

fn authorize(context: AuthContext<'_>, checking: bool) -> Authorization {
    if context.accessor.is_some() {
        return Authorization::Deny;
    }
    let allowed = match context.action {
        AuthAction::Select => true,
        AuthAction::Read { table_name, .. } => {
            context.database_name == Some("main")
                && matches!(
                    table_name,
                    "sqlite_master"
                        | "sqlite_schema"
                        | "indexed_files"
                        | "chunks"
                        | "chunks_fts"
                        | "chunks_fts_data"
                        | "chunks_fts_idx"
                        | "chunks_fts_docsize"
                        | "chunks_fts_config"
                )
        }
        AuthAction::Function { function_name } => matches!(
            function_name.to_ascii_lowercase().as_str(),
            "count" | "sum" | "length" | "typeof" | "min" | "instr" | "lower" | "bm25" | "match"
        ),
        AuthAction::Pragma {
            pragma_name,
            pragma_value,
        } => match pragma_name {
            "user_version" | "data_version" => pragma_value.is_none(),
            "quick_check" => matches!(
                pragma_value,
                Some(
                    "chunks"
                        | "indexed_files"
                        | "chunks_fts_data"
                        | "chunks_fts_idx"
                        | "chunks_fts_docsize"
                        | "chunks_fts_config"
                )
            ),
            "query_only" => checking && matches!(pragma_value, Some("0" | "1")),
            _ => false,
        },
        AuthAction::Insert { table_name } => {
            checking && context.database_name == Some("main") && table_name == "chunks_fts"
        }
        _ => false,
    };
    if allowed {
        Authorization::Allow
    } else {
        Authorization::Deny
    }
}

fn validate_postings(db: &Connection) -> Result<(), SnapshotError> {
    let inconsistent: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM main.chunks c LEFT JOIN main.chunks_fts_docsize d ON d.id=c.id WHERE d.id IS NULL)
         OR EXISTS(SELECT 1 FROM main.chunks_fts_docsize d LEFT JOIN main.chunks c ON c.id=d.id WHERE c.id IS NULL)",
        [],
        |row| row.get(0),
    ).map_err(|error| run_error(error, SnapshotError::Integrity))?;
    if inconsistent {
        return Err(SnapshotError::Integrity);
    }
    // FTS5 only exposes its external-content checksum as a special INSERT.
    // This single virtual-table command operates on the isolated memory copy.
    // Return to the normal authorizer and query-only mode on every result.
    db.authorizer(Some(|context: AuthContext<'_>| authorize(context, true)));
    let checked = db.pragma_update(None, "query_only", false).and_then(|()| {
        db.execute(
            "INSERT INTO main.chunks_fts(chunks_fts, rank) VALUES('integrity-check', 1)",
            [],
        )
    });
    let restored = db.pragma_update(None, "query_only", true);
    db.authorizer(Some(|context: AuthContext<'_>| authorize(context, false)));
    restored.map_err(|_| SnapshotError::Integrity)?;
    checked.map_err(|error| run_error(error, SnapshotError::Integrity))?;
    Ok(())
}

fn validate_schema(db: &Connection) -> Result<(), SnapshotError> {
    let schema_error = |error| run_error(error, SnapshotError::Schema);
    let version: i64 = db
        .query_row("PRAGMA main.user_version", [], |row| row.get(0))
        .map_err(schema_error)?;
    if version != 1 {
        return Err(SnapshotError::Schema);
    }
    let expected: Vec<SchemaObject> = serde_json::from_str(SCHEMA).expect("fixed v1 inventory");
    let mut statement = db
        .prepare("SELECT type,name,tbl_name,sql FROM main.sqlite_schema ORDER BY name LIMIT 13")
        .map_err(schema_error)?;
    let objects = statement
        .query_map([], |row| {
            let sql: Option<String> = row.get(3)?;
            let normalized =
                sql.map(|sql| sql.split_ascii_whitespace().collect::<Vec<_>>().join(" "));
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, normalized))
        })
        .map_err(schema_error)?
        .collect::<rusqlite::Result<Vec<SchemaObject>>>()
        .map_err(schema_error)?;
    // Pin ordinary tables, external FTS content mapping, shadow tables and native triggers.
    if objects != expected {
        return Err(SnapshotError::Schema);
    }
    let fts_config: Option<i64> = db.query_row(
        "SELECT CASE WHEN count(*)=1 AND min(k)='version' AND typeof(min(v))='integer' THEN min(v) END
         FROM main.chunks_fts_config",
        [],
        |row| row.get(0),
    ).map_err(schema_error)?;
    if fts_config != Some(4) {
        return Err(SnapshotError::Schema);
    }
    Ok(())
}

fn validate_data(db: &Connection, fts_term_count: u32) -> Result<(), SnapshotError> {
    let row_error = |error| run_error(error, SnapshotError::RowLimit);
    let (chunks, corpus): (u32, Option<u32>) = db
        .query_row(
            "SELECT count(*),sum(length(CAST(text AS BLOB))) FROM main.chunks",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(row_error)?;
    let corpus = corpus.unwrap_or(0);
    if chunks > MAX_CHUNKS || corpus > MAX_CORPUS_TEXT {
        return Err(SnapshotError::RowLimit);
    }
    if corpus.saturating_mul(fts_term_count) > MAX_TERM_WORK {
        return Err(SnapshotError::Budget);
    }
    let too_large: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM main.chunks WHERE
                      typeof(text)!='text' OR length(CAST(text AS BLOB))>4096
                      OR typeof(relative)!='text' OR length(CAST(relative AS BLOB))>512
                      OR typeof(location)!='text' OR length(CAST(location AS BLOB))>160
                      OR typeof(source_id)!='text' OR length(CAST(source_id AS BLOB))>64
                      OR typeof(sha256)!='text' OR length(sha256)!=64
                      OR typeof(ordinal)!='integer' OR ordinal NOT BETWEEN 0 AND 100000)
         OR EXISTS(SELECT 1 FROM main.indexed_files WHERE
                      typeof(source_id)!='text' OR length(CAST(source_id AS BLOB))>64
                      OR typeof(relative)!='text' OR length(CAST(relative AS BLOB))>512
                      OR typeof(sha256)!='text' OR length(sha256)!=64)",
            [],
            |row| row.get(0),
        )
        .map_err(row_error)?;
    if too_large {
        return Err(SnapshotError::RowLimit);
    }
    // Check each pinned ordinary B-tree here. The stronger external-content
    // check below covers the FTS virtual table without repeating its scan.
    for statement in [
        "PRAGMA main.quick_check('indexed_files')",
        "PRAGMA main.quick_check('chunks')",
        "PRAGMA main.quick_check('chunks_fts_data')",
        "PRAGMA main.quick_check('chunks_fts_idx')",
        "PRAGMA main.quick_check('chunks_fts_docsize')",
        "PRAGMA main.quick_check('chunks_fts_config')",
    ] {
        let check: String = db
            .query_row(statement, [], |row| row.get(0))
            .map_err(|error| run_error(error, SnapshotError::Integrity))?;
        if check != "ok" {
            return Err(SnapshotError::Integrity);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_authorizer_keeps_the_diagnostic_window_narrow() {
        let allowed = |action, db, accessor, checking| {
            authorize(
                AuthContext {
                    action,
                    database_name: db,
                    accessor,
                },
                checking,
            ) == Authorization::Allow
        };
        let insert = AuthAction::Insert {
            table_name: "chunks_fts",
        };
        assert!(!allowed(insert, Some("main"), None, false));
        assert!(allowed(insert, Some("main"), None, true));
        assert!(!allowed(
            insert,
            Some("main"),
            Some("unexpected_trigger"),
            true
        ));
        assert!(!allowed(insert, Some("temp"), None, true));
        for action in [
            AuthAction::Attach {
                filename: "fixture-only.sqlite",
            },
            AuthAction::Insert {
                table_name: "chunks",
            },
            AuthAction::Delete {
                table_name: "chunks",
            },
            AuthAction::Function {
                function_name: "load_extension",
            },
            AuthAction::Pragma {
                pragma_name: "writable_schema",
                pragma_value: Some("1"),
            },
        ] {
            assert!(!allowed(action, Some("main"), None, false));
            assert!(!allowed(action, Some("main"), None, true));
        }
    }
}
