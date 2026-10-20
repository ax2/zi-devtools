//! Explicit, single-cell SQLite writes guarded by a reviewed rollback preview.
use super::*;
use rusqlite::{OptionalExtension, TransactionBehavior, types::Value};

const MAX_EDIT_BYTES: usize = MAX_CELL_BYTES;

#[derive(Clone, Debug)]
pub(super) struct Review {
    path: PathBuf,
    source: crate::material_files::FileMaterial,
    table: String,
    column: String,
    schema_version: i64,
    keys: Vec<(String, Value)>,
    old: Value,
    new: Value,
}

impl Review {
    pub(super) fn table(&self) -> &str {
        &self.table
    }
    pub(super) fn column(&self) -> &str {
        &self.column
    }
    #[cfg(test)]
    pub(super) fn old_value(&self) -> &Value {
        &self.old
    }
    #[cfg(test)]
    pub(super) fn new_value(&self) -> &Value {
        &self.new
    }
    pub(super) fn old_label(&self) -> String {
        value_label(&self.old)
    }
    pub(super) fn new_label(&self) -> String {
        value_label(&self.new)
    }
    pub(super) fn key_summary(&self) -> String {
        self.keys
            .iter()
            .map(|(name, value)| format!("{name}={}", value_label(value)))
            .collect::<Vec<_>>()
            .join(", ")
    }
    pub(super) fn refresh_source(&mut self) -> Result<()> {
        self.source = crate::material_files::FileMaterial::selected(&self.path, usize::MAX)?;
        Ok(())
    }
}

pub(super) fn value_label(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Integer(value) => value.to_string(),
        Value::Real(value) => value.to_string(),
        Value::Text(value) => serde_json::to_string(value).unwrap_or_else(|_| "\"?\"".into()),
        Value::Blob(_) => "[BLOB]".into(),
    }
}

pub(super) fn open_writable(path: &Path) -> Result<Connection> {
    ensure!(path.is_absolute(), "请选择绝对路径的本机 SQLite 文件");
    let metadata = fs::symlink_metadata(path).context("无法读取数据库文件信息")?;
    ensure!(
        metadata.is_file() && !is_link(&metadata),
        "请选择普通文件，不支持符号链接或重解析点"
    );
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .context("无法以读写模式打开已存在的 SQLite 文件")?;
    connection.busy_timeout(Duration::from_secs(1))?;
    let start = Instant::now();
    connection.progress_handler(1000, Some(move || start.elapsed() >= QUERY_TIMEOUT));
    Ok(connection)
}

fn scalar(input: &str) -> Result<Value> {
    ensure!(input.len() <= MAX_EDIT_BYTES, "单元格输入最多4KiB");
    let value: serde_json::Value = serde_json::from_str(input)
        .context("请按 JSON 标量输入：字符串用双引号，或输入数字、true、false、null")?;
    match value {
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::Bool(value) => Ok(Value::Integer(i64::from(value))),
        serde_json::Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                Ok(Value::Integer(value))
            } else if let Some(value) = value.as_u64() {
                ensure!(value <= i64::MAX as u64, "整数超出 SQLite 64 位范围");
                Ok(Value::Integer(value as i64))
            } else {
                let value = value.as_f64().context("无效数字")?;
                ensure!(value.is_finite(), "SQLite 不接受非有限数字");
                Ok(Value::Real(value))
            }
        }
        serde_json::Value::String(value) => Ok(Value::Text(value)),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            anyhow::bail!("只支持 NULL、布尔、数字和文本标量；对象/数组需在其他工具中处理")
        }
    }
}

pub(super) fn page_value(cell: &Cell) -> Result<Value> {
    ensure!(!cell.truncated, "截断单元格不能编辑");
    let value = cell
        .value
        .as_ref()
        .context("BLOB、非UTF8或非有限值不能编辑")?;
    match value {
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::Bool(value) => Ok(Value::Integer(i64::from(*value))),
        serde_json::Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                Ok(Value::Integer(value))
            } else {
                let value = value.as_f64().context("无效数字")?;
                ensure!(value.is_finite(), "无效数字");
                Ok(Value::Real(value))
            }
        }
        serde_json::Value::String(value) => Ok(Value::Text(value.clone())),
        _ => anyhow::bail!("该单元格不是 SQLite 标量"),
    }
}

pub(super) fn ensure_supported_schema(
    connection: &Connection,
    table: &str,
) -> Result<Vec<(i64, String)>> {
    let (kind, create_sql): (String, Option<String>) = connection
        .query_row(
            "SELECT type, sql FROM sqlite_schema WHERE name=?1 AND type IN ('table','view')",
            [table],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .context("表已不存在")?;
    ensure!(kind == "table", "视图不能直接编辑");
    let create_sql = create_sql.unwrap_or_default();
    ensure!(
        !create_sql
            .trim_start()
            .to_ascii_uppercase()
            .starts_with("CREATE VIRTUAL TABLE"),
        "虚拟表暂不支持直接编辑"
    );
    let triggers: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_schema WHERE type='trigger' AND tbl_name=?1",
        [table],
        |row| row.get(0),
    )?;
    ensure!(
        triggers == 0,
        "该表含有触发器，暂不支持编辑，避免产生未预览的副作用"
    );

    let mut statement = connection.prepare(
        "SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' LIMIT 201",
    )?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ensure!(names.len() <= 200, "数据库表数量超过200，暂不支持写入");
    for name in names {
        let sql = format!("PRAGMA foreign_key_list({})", quote_identifier(&name));
        let mut foreign_keys = connection.prepare(&sql)?;
        let mut rows = foreign_keys.query([])?;
        ensure!(
            rows.next()?.is_none(),
            "数据库含有外键关系，暂不支持写入，避免产生未预览的级联变更"
        );
    }

    let mut statement = connection.prepare(
        "SELECT name, pk FROM pragma_table_xinfo(?1) WHERE hidden=0 AND pk>0 ORDER BY pk",
    )?;
    let keys = statement
        .query_map([table], |row| Ok((row.get(1)?, row.get(0)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ensure!(!keys.is_empty(), "该表没有可用于唯一定位行的主键");
    Ok(keys)
}

fn schema_version(connection: &Connection) -> Result<i64> {
    Ok(connection.query_row("PRAGMA schema_version", [], |row| row.get(0))?)
}

fn lookup_value(
    connection: &Connection,
    table: &str,
    column: &str,
    keys: &[(String, Value)],
) -> Result<Option<Value>> {
    let where_clause = keys
        .iter()
        .enumerate()
        .map(|(i, (name, _))| format!("{} IS ?{}", quote_identifier(name), i + 1))
        .collect::<Vec<_>>()
        .join(" AND ");
    let sql = format!(
        "SELECT {} FROM {} WHERE {where_clause}",
        quote_identifier(column),
        quote_identifier(table)
    );
    let params = rusqlite::params_from_iter(keys.iter().map(|(_, value)| value));
    let mut statement = connection.prepare(&sql)?;
    let mut rows = statement.query(params)?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let value = row.get(0)?;
    ensure!(rows.next()?.is_none(), "主键未能唯一定位一行");
    Ok(Some(value))
}

fn perform_update(
    connection: &Connection,
    table: &str,
    column: &str,
    keys: &[(String, Value)],
    old: &Value,
    new: &Value,
) -> Result<Value> {
    let assignments = format!("{}=?1", quote_identifier(column));
    let mut predicates = Vec::new();
    for (index, (name, _)) in keys.iter().enumerate() {
        predicates.push(format!("{} IS ?{}", quote_identifier(name), index + 2));
    }
    predicates.push(format!(
        "{} IS ?{}",
        quote_identifier(column),
        keys.len() + 2
    ));
    let sql = format!(
        "UPDATE {} SET {assignments} WHERE {}",
        quote_identifier(table),
        predicates.join(" AND ")
    );
    let mut values = vec![new.clone()];
    values.extend(keys.iter().map(|(_, value)| value.clone()));
    values.push(old.clone());
    let changed = connection.execute(&sql, rusqlite::params_from_iter(values.iter()))?;
    ensure!(changed == 1, "数据已变化或目标行已删除，请重新读取后再预览");
    lookup_value(connection, table, column, keys)?.context("更新后的记录无法重新读取")
}

pub(super) fn prepare(
    path: &Path,
    page: &PageData,
    row: usize,
    column: usize,
    input: &str,
) -> Result<Review> {
    ensure!(row < page.rows.len(), "所选行已不在当前页");
    ensure!(column < page.columns.len(), "所选列不存在");
    let target = &page.columns[column];
    ensure!(
        !target.primary_key,
        "暂不支持修改主键，避免关联记录产生副作用"
    );
    let old = page_value(&page.rows[row][column])?;
    let requested = scalar(input)?;
    ensure!(old != requested, "新旧值相同，无需写入");

    let source = crate::material_files::FileMaterial::selected(path, usize::MAX)?;
    let mut connection = open_writable(source.path())?;
    source.validate(usize::MAX)?;
    let primary_keys = ensure_supported_schema(&connection, &page.table)?;
    let mut keys = Vec::with_capacity(primary_keys.len());
    for (_, name) in &primary_keys {
        let index = page
            .columns
            .iter()
            .position(|column| column.name == *name)
            .context("当前页缺少主键列，请重新读取")?;
        let value = page_value(&page.rows[row][index])?;
        ensure!(
            !matches!(value, Value::Null),
            "当前行主键值为空，无法唯一定位"
        );
        keys.push((name.clone(), value));
    }
    let current = lookup_value(&connection, &page.table, &target.name, &keys)?
        .context("当前行已被删除，请重新读取")?;
    ensure!(
        current == old,
        "当前值已被其他程序修改，请重新读取并再次预览"
    );

    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_supported_schema(&transaction, &page.table)?;
    let schema_version = schema_version(&transaction)?;
    let actual = perform_update(
        &transaction,
        &page.table,
        &target.name,
        &keys,
        &old,
        &requested,
    )?;
    transaction.rollback()?;
    ensure!(actual != old, "SQLite 应用列类型后没有产生变化");
    Ok(Review {
        path: source.path().to_path_buf(),
        source,
        table: page.table.clone(),
        column: target.name.clone(),
        schema_version,
        keys,
        old,
        new: actual,
    })
}

pub(super) fn apply(review: &Review) -> std::result::Result<Review, super::ApplyFailure> {
    apply_with_refresh(review, |inverse| inverse.refresh_source())
}

fn apply_with_refresh<F>(
    review: &Review,
    refresh: F,
) -> std::result::Result<Review, super::ApplyFailure>
where
    F: FnOnce(&mut Review) -> Result<()>,
{
    let mut committed = false;
    let result = (|| -> Result<Review> {
        review.source.validate(usize::MAX)?;
        let mut connection = open_writable(&review.path)?;
        review.source.validate(usize::MAX)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_supported_schema(&transaction, &review.table)?;
        ensure!(
            schema_version(&transaction)? == review.schema_version,
            "数据库结构已变化，请重新读取并预览"
        );
        let current = lookup_value(&transaction, &review.table, &review.column, &review.keys)?
            .context("当前行已被删除，请重新读取")?;
        ensure!(
            current == review.old,
            "当前值已变化，未写入；请重新读取并再次预览"
        );
        let actual = perform_update(
            &transaction,
            &review.table,
            &review.column,
            &review.keys,
            &review.old,
            &review.new,
        )?;
        ensure!(actual == review.new, "SQLite 返回值与预览不符，已回滚");
        transaction.commit()?;
        committed = true;
        let mut inverse = review.clone();
        inverse.old = review.new.clone();
        inverse.new = review.old.clone();
        refresh(&mut inverse)?;
        Ok(inverse)
    })();
    result.map_err(|error| {
        let message = format!("{error:#}");
        if committed {
            super::ApplyFailure::CommittedWithoutHistory(message)
        } else {
            super::ApplyFailure::NotCommitted(message)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new(schema: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("zi-sqlite-edit-{}.db", uuid::Uuid::new_v4()));
            Connection::open(&path)
                .unwrap()
                .execute_batch(schema)
                .unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
            let _ = fs::remove_file(self.0.with_extension("db-wal"));
            let _ = fs::remove_file(self.0.with_extension("db-shm"));
        }
    }
    fn page(fixture: &Fixture) -> PageData {
        load_page(&fixture.0, "notes", 0, &Arc::new(AtomicBool::new(false))).unwrap()
    }

    #[test]
    fn preview_rolls_back_and_confirmed_apply_updates_only_the_selected_cell() {
        let fixture = Fixture::new(
            "CREATE TABLE notes(id INTEGER PRIMARY KEY, title TEXT, active INTEGER); INSERT INTO notes VALUES(1,'before',0),(2,'other',1);",
        );
        let before = fs::read(&fixture.0).unwrap();
        let page = page(&fixture);
        let review = prepare(&fixture.0, &page, 0, 1, "\"after\"").unwrap();
        assert!(matches!(review.old_value(), Value::Text(value) if value == "before"));
        assert!(matches!(review.new_value(), Value::Text(value) if value == "after"));
        assert_eq!(
            fs::read(&fixture.0).unwrap(),
            before,
            "review must rollback without persistence"
        );
        let undo = apply(&review).unwrap();
        let db = Connection::open_with_flags(&fixture.0, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let values: Vec<(i64, String, i64)> = db
            .prepare("SELECT id,title,active FROM notes ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(values, vec![(1, "after".into(), 0), (2, "other".into(), 1)]);
        let redo = apply(&undo).unwrap();
        assert_eq!(
            Connection::open(&fixture.0)
                .unwrap()
                .query_row("SELECT title FROM notes WHERE id=1", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "before"
        );
        apply(&redo).unwrap();
        assert_eq!(
            Connection::open(&fixture.0)
                .unwrap()
                .query_row("SELECT title FROM notes WHERE id=1", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "after"
        );
    }

    #[test]
    fn committed_edit_reports_lost_history_if_snapshot_refresh_fails() {
        let fixture = Fixture::new(
            "CREATE TABLE notes(id INTEGER PRIMARY KEY, title TEXT); INSERT INTO notes VALUES(1,'before');",
        );
        let review = prepare(&fixture.0, &page(&fixture), 0, 1, "\"after\"").unwrap();
        let error = apply_with_refresh(&review, |_| anyhow::bail!("simulated refresh failure"))
            .unwrap_err();
        assert!(matches!(
            error,
            super::super::ApplyFailure::CommittedWithoutHistory(message)
                if message.contains("simulated refresh failure")
        ));
        assert_eq!(
            Connection::open(&fixture.0)
                .unwrap()
                .query_row("SELECT title FROM notes WHERE id=1", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "after",
            "the result must accurately report that the transaction committed"
        );
    }

    #[test]
    fn apply_rejects_stale_cell_and_preserves_concurrent_value() {
        let fixture = Fixture::new(
            "CREATE TABLE notes(id INTEGER PRIMARY KEY, title TEXT); INSERT INTO notes VALUES(1,'before');",
        );
        Connection::open(&fixture.0)
            .unwrap()
            .execute_batch("PRAGMA journal_mode=WAL")
            .unwrap();
        let review = prepare(&fixture.0, &page(&fixture), 0, 1, "\"after\"").unwrap();
        let external = Connection::open(&fixture.0).unwrap();
        external
            .execute("UPDATE notes SET title='external' WHERE id=1", [])
            .unwrap();
        let error = apply(&review).unwrap_err().to_string();
        assert!(
            error.contains("已变化"),
            "unexpected conflict error: {error}"
        );
        drop(external);
        assert_eq!(
            Connection::open(&fixture.0)
                .unwrap()
                .query_row("SELECT title FROM notes WHERE id=1", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "external"
        );
    }

    #[test]
    fn apply_rejects_schema_changes_since_the_review() {
        let fixture = Fixture::new(
            "CREATE TABLE notes(id INTEGER PRIMARY KEY, title TEXT); INSERT INTO notes VALUES(1,'before');",
        );
        let review = prepare(&fixture.0, &page(&fixture), 0, 1, "\"after\"").unwrap();
        Connection::open(&fixture.0)
            .unwrap()
            .execute_batch("CREATE INDEX notes_title ON notes(title)")
            .unwrap();
        assert!(apply(&review).is_err());
        assert_eq!(
            Connection::open(&fixture.0)
                .unwrap()
                .query_row("SELECT title FROM notes WHERE id=1", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "before"
        );
    }

    #[test]
    fn apply_rejects_a_replaced_database_even_if_path_and_cell_match() {
        let fixture = Fixture::new(
            "CREATE TABLE notes(id INTEGER PRIMARY KEY, title TEXT); INSERT INTO notes VALUES(1,'before');",
        );
        let review = prepare(&fixture.0, &page(&fixture), 0, 1, "\"after\"").unwrap();
        let previous = fixture.0.with_extension("previous.db");
        fs::rename(&fixture.0, &previous).unwrap();
        Connection::open(&fixture.0)
            .unwrap()
            .execute_batch("CREATE TABLE notes(id INTEGER PRIMARY KEY, title TEXT); INSERT INTO notes VALUES(1,'before');")
            .unwrap();
        assert!(apply(&review).unwrap_err().to_string().contains("已改变"));
        assert_eq!(
            Connection::open(&fixture.0)
                .unwrap()
                .query_row("SELECT title FROM notes WHERE id=1", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "before"
        );
        fs::remove_file(previous).unwrap();
    }

    #[test]
    fn preview_rejects_views_without_keys_triggers_and_foreign_keys() {
        let no_key =
            Fixture::new("CREATE TABLE notes(title TEXT); INSERT INTO notes VALUES('before');");
        assert!(prepare(&no_key.0, &page(&no_key), 0, 0, "\"after\"").is_err());

        let trigger = Fixture::new(
            "CREATE TABLE notes(id INTEGER PRIMARY KEY, title TEXT); CREATE TRIGGER notes_audit AFTER UPDATE ON notes BEGIN INSERT INTO audit VALUES(new.title); END; CREATE TABLE audit(title TEXT); INSERT INTO notes VALUES(1,'before');",
        );
        assert!(
            prepare(&trigger.0, &page(&trigger), 0, 1, "\"after\"")
                .unwrap_err()
                .to_string()
                .contains("触发器")
        );

        let fk = Fixture::new(
            "PRAGMA foreign_keys=ON; CREATE TABLE parent(id INTEGER PRIMARY KEY, title TEXT); CREATE TABLE child(id INTEGER PRIMARY KEY, parent_id INTEGER REFERENCES parent(id)); INSERT INTO parent VALUES(1,'before'); INSERT INTO child VALUES(2,1);",
        );
        let page = load_page(&fk.0, "parent", 0, &Arc::new(AtomicBool::new(false))).unwrap();
        assert!(
            prepare(&fk.0, &page, 0, 1, "\"after\"")
                .unwrap_err()
                .to_string()
                .contains("外键")
        );
    }

    #[test]
    fn scalar_parser_rejects_composites_and_out_of_range_integers() {
        assert!(scalar("[1,2]").is_err());
        assert!(scalar("{\"a\":1}").is_err());
        assert!(scalar("9223372036854775808").is_err());
        assert_eq!(scalar("true").unwrap(), Value::Integer(1));
        assert_eq!(scalar("null").unwrap(), Value::Null);
    }
}
