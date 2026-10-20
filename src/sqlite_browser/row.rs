//! Reviewed, single-row SQLite deletion with process-local restoration data.
use super::write::value_label;
use super::*;
use rusqlite::{TransactionBehavior, types::Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Delete,
    Restore,
}

#[derive(Clone, Debug)]
pub(super) struct Review {
    path: PathBuf,
    source: crate::material_files::FileMaterial,
    table: String,
    schema_version: i64,
    keys: Vec<(String, Value)>,
    values: Vec<(String, Value)>,
    rowid: Option<(String, i64)>,
    action: Action,
}

impl Review {
    pub(super) fn table(&self) -> &str {
        &self.table
    }
    pub(super) fn change_label(&self) -> String {
        match self.action {
            Action::Delete => format!("删除1行（{}列；快照仅用于本次会话撤销）", self.values.len()),
            Action::Restore => format!("恢复1行（{}列）", self.values.len()),
        }
    }
    pub(super) fn preview_details(&self) -> Vec<String> {
        let mut fields = self
            .values
            .iter()
            .map(|(name, value)| {
                let text = value_label(value);
                let preview = text.chars().take(160).collect::<String>();
                if text.chars().count() > 160 {
                    format!("{name} = {preview}… [截断，仅供确认]")
                } else {
                    format!("{name} = {preview}")
                }
            })
            .collect::<Vec<_>>();
        if let Some((alias, rowid)) = &self.rowid {
            fields.push(format!("内部行身份 {alias} = {rowid}"));
        }
        fields
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

fn row_columns(connection: &Connection, table: &str, page: &PageData) -> Result<Vec<String>> {
    let mut statement =
        connection.prepare("SELECT name, hidden FROM pragma_table_xinfo(?1) ORDER BY cid")?;
    let columns = statement
        .query_map([table], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ensure!(!columns.is_empty(), "无法获取表列结构");
    ensure!(
        columns.len() <= MAX_COLUMNS,
        "列数超过128，暂不支持整行删除"
    );
    ensure!(
        columns.iter().all(|(_, hidden)| *hidden == 0),
        "含有生成列或隐藏列的表暂不支持整行删除，以确保撤销可完整恢复"
    );
    let names = columns
        .into_iter()
        .map(|(name, _)| name)
        .collect::<Vec<_>>();
    ensure!(
        names.len() == page.columns.len()
            && names
                .iter()
                .zip(&page.columns)
                .all(|(name, column)| name == &column.name),
        "当前列结构与预览不一致，请重新读取"
    );
    Ok(names)
}

fn where_keys(keys: &[(String, Value)]) -> String {
    keys.iter()
        .enumerate()
        .map(|(index, (name, _))| format!("{} IS ?{}", quote_identifier(name), index + 1))
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn find_row(
    connection: &Connection,
    table: &str,
    columns: &[String],
    keys: &[(String, Value)],
) -> Result<Option<Vec<Value>>> {
    let projection = columns
        .iter()
        .map(|name| quote_identifier(name))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT {projection} FROM {} WHERE {}",
        quote_identifier(table),
        where_keys(keys)
    );
    let params = rusqlite::params_from_iter(keys.iter().map(|(_, value)| value));
    let mut statement = connection.prepare(&sql)?;
    let mut rows = statement.query(params)?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let values = (0..columns.len())
        .map(|index| row.get(index))
        .collect::<rusqlite::Result<Vec<Value>>>()?;
    ensure!(rows.next()?.is_none(), "主键未能唯一定位一行");
    Ok(Some(values))
}

fn rowid_alias(connection: &Connection, table: &str, columns: &[String]) -> Result<Option<String>> {
    let without_rowid: i64 = connection.query_row(
        "SELECT wr FROM pragma_table_list WHERE name=?1 AND type='table' LIMIT 1",
        [table],
        |row| row.get(0),
    )?;
    if without_rowid != 0 {
        return Ok(None);
    }
    Ok(["_rowid_", "rowid", "oid"]
        .into_iter()
        .find(|candidate| {
            columns
                .iter()
                .all(|column| !column.eq_ignore_ascii_case(candidate))
        })
        .map(str::to_owned))
}

fn find_rowid(
    connection: &Connection,
    table: &str,
    alias: &str,
    keys: &[(String, Value)],
) -> Result<Option<i64>> {
    let sql = format!(
        "SELECT {} FROM {} WHERE {}",
        quote_identifier(alias),
        quote_identifier(table),
        where_keys(keys)
    );
    let params = rusqlite::params_from_iter(keys.iter().map(|(_, value)| value));
    let mut statement = connection.prepare(&sql)?;
    let mut rows = statement.query(params)?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let rowid = row.get(0)?;
    ensure!(rows.next()?.is_none(), "主键未能唯一定位一行");
    Ok(Some(rowid))
}

fn row_matches(review: &Review, actual: &[Value]) -> bool {
    actual.len() == review.values.len()
        && actual
            .iter()
            .zip(&review.values)
            .all(|(value, (_, expected))| value == expected)
}

fn delete_row(connection: &Connection, review: &Review) -> Result<()> {
    let columns = review
        .values
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    let actual = find_row(connection, &review.table, &columns, &review.keys)?
        .context("当前行已不存在，请重新读取")?;
    ensure!(
        row_matches(review, &actual),
        "当前行内容已变化，拒绝删除；请重新读取并再次预览"
    );
    if let Some((alias, expected_rowid)) = &review.rowid {
        ensure!(
            find_rowid(connection, &review.table, alias, &review.keys)? == Some(*expected_rowid),
            "目标行 rowid 已变化，拒绝删除"
        );
    }
    let sql = format!(
        "DELETE FROM {} WHERE {}",
        quote_identifier(&review.table),
        where_keys(&review.keys)
    );
    let changed = connection.execute(
        &sql,
        rusqlite::params_from_iter(review.keys.iter().map(|(_, value)| value)),
    )?;
    ensure!(changed == 1, "数据已变化，未删除任何行");
    ensure!(
        find_row(connection, &review.table, &columns, &review.keys)?.is_none(),
        "删除后仍能读取目标行"
    );
    Ok(())
}

fn restore_row(connection: &Connection, review: &Review) -> Result<()> {
    let columns = review
        .values
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    ensure!(
        find_row(connection, &review.table, &columns, &review.keys)?.is_none(),
        "主键已被其他数据占用，拒绝恢复"
    );
    let mut insert_columns =
        Vec::with_capacity(columns.len() + usize::from(review.rowid.is_some()));
    let mut insert_values = Vec::with_capacity(columns.len() + usize::from(review.rowid.is_some()));
    if let Some((alias, rowid)) = &review.rowid {
        insert_columns.push(quote_identifier(alias));
        insert_values.push(Value::Integer(*rowid));
    }
    insert_columns.extend(columns.iter().map(|name| quote_identifier(name)));
    insert_values.extend(review.values.iter().map(|(_, value)| value.clone()));
    let names = insert_columns.to_vec().join(", ");
    let placeholders = (1..=insert_values.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "INSERT INTO {} ({names}) VALUES ({placeholders})",
        quote_identifier(&review.table)
    );
    let changed = connection.execute(&sql, rusqlite::params_from_iter(insert_values.iter()))?;
    ensure!(changed == 1, "未能恢复目标行");
    let restored = find_row(connection, &review.table, &columns, &review.keys)?
        .context("恢复后无法读取目标行")?;
    ensure!(row_matches(review, &restored), "恢复结果与原行快照不一致");
    if let Some((alias, rowid)) = &review.rowid {
        ensure!(
            find_rowid(connection, &review.table, alias, &review.keys)? == Some(*rowid),
            "恢复后的 rowid 与原行不一致，已回滚"
        );
    }
    Ok(())
}

pub(super) fn prepare_delete(path: &Path, page: &PageData, row: usize) -> Result<Review> {
    ensure!(row < page.rows.len(), "所选行已不在当前页");
    let source = crate::material_files::FileMaterial::selected(path, usize::MAX)?;
    let mut connection = write::open_writable(source.path())?;
    source.validate(usize::MAX)?;
    let primary_keys = write::ensure_supported_schema(&connection, &page.table)?;
    let columns = row_columns(&connection, &page.table, page)?;
    let rowid_column = rowid_alias(&connection, &page.table, &columns)?;
    let without_rowid: i64 = connection.query_row(
        "SELECT wr FROM pragma_table_list WHERE name=?1 AND type='table' LIMIT 1",
        [&page.table],
        |row| row.get(0),
    )?;
    ensure!(
        without_rowid != 0 || rowid_column.is_some(),
        "表的 rowid 名称被列遮蔽，无法安全保留行身份"
    );
    let row_cells = &page.rows[row];
    ensure!(row_cells.len() == columns.len(), "当前行列数与数据库不一致");
    let values = columns
        .iter()
        .zip(row_cells)
        .map(|(name, cell)| Ok((name.clone(), write::page_value(cell)?)))
        .collect::<Result<Vec<_>>>()?;
    let mut keys = Vec::with_capacity(primary_keys.len());
    for (_, name) in primary_keys {
        let (_, value) = values
            .iter()
            .find(|(column, _)| column == &name)
            .context("当前页缺少主键列，请重新读取")?;
        ensure!(!matches!(value, Value::Null), "主键值为空，不能安全定位行");
        keys.push((name, value.clone()));
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    write::ensure_supported_schema(&transaction, &page.table)?;
    let schema_version = transaction.query_row("PRAGMA schema_version", [], |row| row.get(0))?;
    let rowid = match &rowid_column {
        Some(alias) => Some((
            alias.clone(),
            find_rowid(&transaction, &page.table, alias, &keys)?
                .context("当前行已被删除，请重新读取")?,
        )),
        None => None,
    };
    let review = Review {
        path: source.path().to_path_buf(),
        source,
        table: page.table.clone(),
        schema_version,
        keys,
        values,
        rowid,
        action: Action::Delete,
    };
    let actual = find_row(&transaction, &review.table, &columns, &review.keys)?
        .context("当前行已被删除，请重新读取")?;
    ensure!(
        row_matches(&review, &actual),
        "当前行内容已变化，请重新读取后再次预览"
    );
    delete_row(&transaction, &review)?;
    transaction.rollback()?;
    Ok(review)
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
        let mut connection = write::open_writable(&review.path)?;
        review.source.validate(usize::MAX)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        write::ensure_supported_schema(&transaction, &review.table)?;
        ensure!(
            transaction.query_row("PRAGMA schema_version", [], |row| row.get::<_, i64>(0))?
                == review.schema_version,
            "数据库结构已变化，请重新读取并预览"
        );
        match review.action {
            Action::Delete => delete_row(&transaction, review)?,
            Action::Restore => restore_row(&transaction, review)?,
        }
        transaction.commit()?;
        committed = true;
        let mut inverse = review.clone();
        inverse.action = match review.action {
            Action::Delete => Action::Restore,
            Action::Restore => Action::Delete,
        };
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
                std::env::temp_dir().join(format!("zi-sqlite-row-{}.db", uuid::Uuid::new_v4()));
            Connection::open(&path)
                .unwrap()
                .execute_batch(schema)
                .unwrap();
            Self(path)
        }
        fn page(&self, table: &str) -> PageData {
            load_page(&self.0, table, 0, &Arc::new(AtomicBool::new(false))).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
            let _ = fs::remove_file(self.0.with_extension("db-wal"));
            let _ = fs::remove_file(self.0.with_extension("db-shm"));
        }
    }

    #[test]
    fn delete_preview_rolls_back_and_delete_can_be_undone_and_redone() {
        let fixture = Fixture::new(
            "CREATE TABLE notes(id INTEGER PRIMARY KEY, title TEXT, enabled INTEGER); INSERT INTO notes VALUES(1,'first',1),(2,'second',0);",
        );
        let review = prepare_delete(&fixture.0, &fixture.page("notes"), 0).unwrap();
        let db = Connection::open(&fixture.0).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM notes", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            2
        );
        drop(db);

        let undo = apply(&review).unwrap();
        let db = Connection::open(&fixture.0).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM notes", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
        drop(db);

        let redo = apply(&undo).unwrap();
        let db = Connection::open(&fixture.0).unwrap();
        let row: (i64, String, i64) = db
            .query_row("SELECT id,title,enabled FROM notes", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .unwrap();
        assert_eq!(row, (1, "first".into(), 1));
        drop(db);

        apply(&redo).unwrap();
        let db = Connection::open(&fixture.0).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM notes", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn committed_row_delete_reports_lost_history_if_snapshot_refresh_fails() {
        let fixture = Fixture::new(
            "CREATE TABLE notes(id INTEGER PRIMARY KEY, title TEXT); INSERT INTO notes VALUES(1,'before');",
        );
        let review = prepare_delete(&fixture.0, &fixture.page("notes"), 0).unwrap();
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
                .query_row("SELECT count(*) FROM notes", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0,
            "the result must accurately report that the transaction committed"
        );
    }

    #[test]
    fn delete_and_undo_preserve_rowid_for_composite_primary_keys() {
        let fixture = Fixture::new(
            "CREATE TABLE notes(part TEXT, key TEXT, title TEXT, PRIMARY KEY(part,key)); INSERT INTO notes(rowid,part,key,title) VALUES(812,'a','b','preserve rowid');",
        );
        let review = prepare_delete(&fixture.0, &fixture.page("notes"), 0).unwrap();
        let undo = apply(&review).unwrap();
        apply(&undo).unwrap();
        assert_eq!(
            Connection::open(&fixture.0)
                .unwrap()
                .query_row(
                    "SELECT rowid FROM notes WHERE part='a' AND key='b'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            812
        );
    }

    #[test]
    fn delete_and_undo_support_without_rowid_tables() {
        let fixture = Fixture::new(
            "CREATE TABLE notes(id TEXT PRIMARY KEY, title TEXT) WITHOUT ROWID; INSERT INTO notes VALUES('k','value');",
        );
        let review = prepare_delete(&fixture.0, &fixture.page("notes"), 0).unwrap();
        let undo = apply(&review).unwrap();
        apply(&undo).unwrap();
        assert_eq!(
            Connection::open(&fixture.0)
                .unwrap()
                .query_row("SELECT title FROM notes WHERE id='k'", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "value"
        );
    }

    #[test]
    fn delete_apply_rejects_a_row_changed_after_preview() {
        let fixture = Fixture::new(
            "CREATE TABLE notes(id INTEGER PRIMARY KEY, title TEXT); INSERT INTO notes VALUES(1,'before');",
        );
        let review = prepare_delete(&fixture.0, &fixture.page("notes"), 0).unwrap();
        Connection::open(&fixture.0)
            .unwrap()
            .execute("UPDATE notes SET title='external' WHERE id=1", [])
            .unwrap();
        assert!(apply(&review).is_err());
        assert_eq!(
            Connection::open(&fixture.0)
                .unwrap()
                .query_row("SELECT title FROM notes WHERE id=1", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "external"
        );
    }

    #[test]
    fn restore_refuses_to_overwrite_a_reused_primary_key() {
        let fixture = Fixture::new(
            "CREATE TABLE notes(id INTEGER PRIMARY KEY, title TEXT); INSERT INTO notes VALUES(1,'before');",
        );
        let review = prepare_delete(&fixture.0, &fixture.page("notes"), 0).unwrap();
        let restore = apply(&review).unwrap();
        Connection::open(&fixture.0)
            .unwrap()
            .execute("INSERT INTO notes VALUES(1,'new owner')", [])
            .unwrap();
        assert!(apply(&restore).is_err());
        assert_eq!(
            Connection::open(&fixture.0)
                .unwrap()
                .query_row("SELECT title FROM notes WHERE id=1", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "new owner"
        );
    }

    #[test]
    fn delete_preview_rejects_generated_columns_and_truncated_values() {
        let generated = Fixture::new(
            "CREATE TABLE notes(id INTEGER PRIMARY KEY, title TEXT, title_upper TEXT GENERATED ALWAYS AS (upper(title)) STORED); INSERT INTO notes(id,title) VALUES(1,'before');",
        );
        assert!(
            prepare_delete(&generated.0, &generated.page("notes"), 0)
                .unwrap_err()
                .to_string()
                .contains("生成列")
        );

        let large = Fixture::new(
            "CREATE TABLE notes(id INTEGER PRIMARY KEY, body TEXT); INSERT INTO notes VALUES(1, printf('%05000d', 1));",
        );
        assert!(prepare_delete(&large.0, &large.page("notes"), 0).is_err());
    }
}
