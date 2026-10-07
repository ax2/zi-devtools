//! A reviewed table snapshot is published as a new SQLite file, never an existing database.
use super::*;
use rusqlite::{Connection, OpenFlags, params_from_iter, types::Value as SqlValue};
use std::{fs, path::Path};

const MAX_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct Review {
    data: Dataset,
    table: String,
    path: PathBuf,
    pub(super) scope: &'static str,
    bools: usize,
    big_ints: usize,
    nested: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("zi-sqlite-export-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }
        fn target(&self) -> PathBuf {
            self.0.join("result.sqlite")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            for entry in fs::read_dir(&self.0).unwrap() {
                let path = entry.unwrap().path();
                assert!(path.parent() == Some(self.0.as_path()));
                fs::remove_file(path).unwrap();
            }
            fs::remove_dir(&self.0).unwrap();
        }
    }
    fn dataset() -> Dataset {
        Dataset::parse(r#"[{"id":"001","nested":{"中文":"换行\n引号\""},"flag":true,"large":18446744073709551615,"empty":null,"decimal":0.25},{"id":"002","nested":[1,2],"flag":false,"large":1,"empty":"","decimal":1.5}]"#, DataFormat::Json, b',').unwrap()
    }
    #[test]
    fn typed_values_and_selection_order_survive_independent_sqlite_read() {
        let fixture = Fixture::new();
        let data = dataset();
        let review = Review::prepare(
            &data,
            &[1, 0],
            "报告\"; DROP TABLE x;--",
            &fixture.target(),
            "筛选",
        )
        .unwrap();
        assert_eq!((review.bools, review.big_ints, review.nested), (2, 1, 2));
        let receipt = write(&review, &AtomicBool::new(false)).unwrap();
        assert_eq!(receipt.rows, 2);
        let conn =
            Connection::open_with_flags(receipt.path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let table = identifier(&review.table).unwrap();
        let first: String = conn
            .query_row(
                &format!("SELECT id FROM {table} ORDER BY rowid LIMIT 1"),
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(first, "002");
        let original: (String, i64, String, String, Option<String>, f64) = conn
            .query_row(
                &format!("SELECT id,flag,large,nested,empty,decimal FROM {table} WHERE rowid=2"),
                [],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(original.0, "001");
        assert_eq!(original.1, 1);
        assert_eq!(original.2, u64::MAX.to_string());
        assert_eq!(
            serde_json::from_str::<Value>(&original.3).unwrap(),
            serde_json::json!({"中文":"换行\n引号\""})
        );
        assert_eq!(original.4, None);
        assert_eq!(original.5, 0.25);
        let types: (String, String) = conn
            .query_row(
                &format!("SELECT typeof(id),typeof(large) FROM {table} WHERE rowid=2"),
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(types, ("text".into(), "text".into()));
        assert_eq!(data, dataset(), "source must remain intact");
    }
    #[test]
    fn csv_keeps_leading_zero_strings_and_saves_more_than_preview_rows() {
        let fixture = Fixture::new();
        let csv = format!(
            "code,empty\n{}",
            (0..501).map(|n| format!("{n:04},\n")).collect::<String>()
        );
        let data = Dataset::parse(&csv, DataFormat::Csv, b',').unwrap();
        let review = Review::prepare(
            &data,
            &(0..501).rev().collect::<Vec<_>>(),
            "data",
            &fixture.target(),
            "筛选",
        )
        .unwrap();
        write(&review, &AtomicBool::new(false)).unwrap();
        let conn = Connection::open(fixture.target()).unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM data", [], |r| r.get::<_, usize>(0))
                .unwrap(),
            501
        );
        assert_eq!(
            conn.query_row("SELECT code FROM data WHERE rowid=501", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "0000"
        );
        assert_eq!(
            conn.query_row("SELECT typeof(code),empty FROM data LIMIT 1", [], |r| Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?
            )))
            .unwrap(),
            ("text".into(), "".into())
        );
    }
    #[test]
    fn a_competing_target_is_not_overwritten_and_temporary_files_are_removed() {
        let fixture = Fixture::new();
        let review = Review::prepare(&dataset(), &[0], "data", &fixture.target(), "筛选").unwrap();
        fs::write(fixture.target(), b"existing user file").unwrap();
        assert!(write(&review, &AtomicBool::new(false)).is_err());
        assert_eq!(fs::read(fixture.target()).unwrap(), b"existing user file");
        assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 1);
        assert!(Review::prepare(&dataset(), &[0], "data", &fixture.target(), "筛选").is_err());
    }
    #[test]
    fn cancellation_never_publishes_and_zero_rows_creates_an_empty_table() {
        let fixture = Fixture::new();
        let review = Review::prepare(&dataset(), &[], "data", &fixture.target(), "筛选").unwrap();
        assert!(
            write(&review, &AtomicBool::new(true))
                .err()
                .unwrap()
                .downcast_ref::<Cancelled>()
                .is_some()
        );
        assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 0);
        assert_eq!(write(&review, &AtomicBool::new(false)).unwrap().rows, 0);
    }
    #[test]
    fn invalid_identifiers_or_selection_never_create_files() {
        let fixture = Fixture::new();
        for indices in [vec![99], vec![0, 0]] {
            assert!(
                Review::prepare(&dataset(), &indices, "data", &fixture.target(), "筛选").is_err()
            );
        }
        for table in ["", "sqlite_internal", "has\0nul"] {
            assert!(Review::prepare(&dataset(), &[0], table, &fixture.target(), "筛选").is_err());
        }
        let bad = Dataset {
            headers: vec!["A".into(), "a".into()],
            rows: vec![vec![Value::Null, Value::Null]],
        };
        assert!(Review::prepare(&bad, &[0], "data", &fixture.target(), "筛选").is_err());
        assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 0);
    }
    #[test]
    fn cancellation_after_transaction_still_removes_temporary_database() {
        let fixture = Fixture::new();
        let review =
            Review::prepare(&dataset(), &[0, 1], "data", &fixture.target(), "筛选").unwrap();
        let cancel = AtomicBool::new(false);
        let result =
            write_before_publish(&review, &cancel, || cancel.store(true, Ordering::Relaxed));
        assert!(result.err().unwrap().downcast_ref::<Cancelled>().is_some());
        assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 0);
    }
    #[test]
    fn all_128_columns_can_be_opened_by_the_existing_browser() {
        let fixture = Fixture::new();
        let mut data = Dataset {
            headers: (0..128).map(|n| format!("field_{n}")).collect(),
            rows: vec![vec![Value::Null; 128]],
        };
        data.headers[0] = "quoted\"; DROP TABLE x;--".into();
        let review = Review::prepare(&data, &[0], "data", &fixture.target(), "全部").unwrap();
        write(&review, &AtomicBool::new(false)).unwrap();
        let page = crate::sqlite_browser::load_page(
            &fixture.target(),
            "data",
            0,
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(page.columns.len(), 128);
        assert_eq!(page.columns[0].name, data.headers[0]);
        assert_eq!(page.rows.len(), 1);
    }

    #[test]
    fn background_save_uses_reviewed_snapshot_and_finishes_without_auto_open() {
        let fixture = Fixture::new();
        let mut source = dataset();
        let review = Review::prepare(&source, &[0], "data", &fixture.target(), "筛选").unwrap();
        source.rows[0][0] = Value::String("changed after review".into());
        let mut state = State::default();
        state.review.replace(review);
        state.start();
        assert!(state.job.phase.active());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while state.receiver.is_some() {
            state.poll();
            assert!(
                std::time::Instant::now() < deadline,
                "background save timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(matches!(state.job.phase, Phase::Done));
        assert!(state.open_request.is_none());
        assert_eq!(state.receipt.as_ref().unwrap().rows, 1);
        let conn = Connection::open_with_flags(fixture.target(), OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
        let id: String = conn
            .query_row("SELECT id FROM data", [], |row| row.get(0))
            .unwrap();
        assert_eq!(id, "001");
        assert_eq!(source.rows[0][0], "changed after review");
    }

    #[test]
    fn oversized_review_is_rejected_before_any_file_is_created() {
        let fixture = Fixture::new();
        let data = Dataset {
            headers: vec!["text".into()],
            rows: vec![vec![Value::String("x".repeat(4096))]; 8192],
        };
        let indices: Vec<_> = (0..data.rows.len()).collect();
        let error = Review::prepare(&data, &indices, "data", &fixture.target(), "全部")
            .err()
            .unwrap();
        assert!(error.to_string().contains("32MiB"), "{error}");
        assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 0);
    }
}

fn identifier(name: &str) -> Result<String> {
    anyhow::ensure!(
        !name.trim().is_empty() && name.len() <= 512 && !name.contains('\0'),
        "表名或列名为空、含空字符或超过512字节"
    );
    Ok(format!("\"{}\"", name.replace('"', "\"\"")))
}

impl Review {
    pub(super) fn prepare(
        data: &Dataset,
        indices: &[usize],
        table: &str,
        path: &Path,
        scope: &'static str,
    ) -> Result<Self> {
        data.validate_saved()?;
        identifier(table)?;
        anyhow::ensure!(
            !table.to_ascii_lowercase().starts_with("sqlite_"),
            "表名不能使用SQLite保留前缀sqlite_"
        );
        let mut columns = BTreeSet::new();
        for name in &data.headers {
            identifier(name)?;
            anyhow::ensure!(
                columns.insert(name.to_ascii_lowercase()),
                "SQLite列名不能仅大小写不同"
            );
        }
        anyhow::ensure!(indices.len() <= ROW_LIMIT, "保存行数超过限制");
        let mut seen = BTreeSet::new();
        let mut bytes = serde_json::to_vec(&data.headers)?.len();
        let mut rows = Vec::with_capacity(indices.len());
        for &index in indices {
            anyhow::ensure!(seen.insert(index), "保存范围含重复行索引");
            let row = data.rows.get(index).context("保存范围已无效")?;
            bytes = bytes
                .checked_add(serde_json::to_vec(row)?.len())
                .context("保存大小超限")?;
            anyhow::ensure!(bytes <= MAX_BYTES, "SQLite另存快照超过32MiB");
            rows.push(row.clone());
        }
        anyhow::ensure!(
            path.is_absolute() && path.file_name().is_some(),
            "请选择新文件的完整路径"
        );
        let parent = path
            .parent()
            .context("目标目录无效")?
            .canonicalize()
            .context("目标目录不存在")?;
        anyhow::ensure!(parent.is_dir(), "目标父路径不是目录");
        let path = parent.join(path.file_name().unwrap());
        anyhow::ensure!(
            fs::symlink_metadata(&path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
            "目标已存在或无法确认，不覆盖任何文件"
        );
        let data = Dataset {
            headers: data.headers.clone(),
            rows,
        };
        let mut review = Self {
            data,
            table: table.into(),
            path,
            scope,
            bools: 0,
            big_ints: 0,
            nested: 0,
        };
        for value in review.data.rows.iter().flatten() {
            match value {
                Value::Bool(_) => review.bools += 1,
                Value::Number(n) if n.is_u64() && n.as_i64().is_none() => review.big_ints += 1,
                Value::Array(_) | Value::Object(_) => review.nested += 1,
                _ => {}
            }
        }
        Ok(review)
    }
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

#[derive(Debug)]
pub(super) struct Cancelled;
impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("已取消，尚未发布新文件")
    }
}
impl std::error::Error for Cancelled {}
fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(Cancelled.into());
    }
    Ok(())
}

pub(super) struct Temporary(pub(super) PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub(super) struct Receipt {
    pub(super) path: PathBuf,
    pub(super) rows: usize,
    pub(super) bytes: u64,
}
pub(super) fn write(review: &Review, cancel: &AtomicBool) -> Result<Receipt> {
    write_before_publish(review, cancel, || {})
}
fn write_before_publish(
    review: &Review,
    cancel: &AtomicBool,
    before_publish: impl FnOnce(),
) -> Result<Receipt> {
    check_cancel(cancel)?;
    let parent = review.path.parent().context("目标目录无效")?;
    anyhow::ensure!(
        parent.canonicalize()? == parent,
        "目标目录位置已改变，请重新预览"
    );
    let temporary_path = parent.join(format!(".zi-sqlite-export-{}.sqlite", uuid::Uuid::new_v4()));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_path)?;
    let temporary = Temporary(temporary_path);
    let mut conn = Connection::open_with_flags(&temporary.0, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;")?;
    let table = identifier(&review.table)?;
    let columns = review
        .data
        .headers
        .iter()
        .map(|h| identifier(h))
        .collect::<Result<Vec<_>>>()?;
    let transaction = conn.transaction()?;
    // No declared affinity: CSV strings and mixed JSON cell types are not coerced.
    transaction.execute_batch(&format!("CREATE TABLE {table} ({})", columns.join(",")))?;
    {
        let placeholders = vec!["?"; columns.len()].join(",");
        let mut insert =
            transaction.prepare(&format!("INSERT INTO {table} VALUES ({placeholders})"))?;
        for row in &review.data.rows {
            check_cancel(cancel)?;
            let values = row.iter().map(sql_value).collect::<Result<Vec<_>>>()?;
            insert.execute(params_from_iter(values))?;
        }
    }
    check_cancel(cancel)?;
    transaction.commit()?;
    let rows: usize = conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })?;
    let integrity: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    anyhow::ensure!(
        rows == review.data.rows.len() && integrity == "ok",
        "写入校验失败，未发布目标文件"
    );
    conn.close().map_err(|(_, error)| error)?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(&temporary.0)?
        .sync_all()?;
    let bytes = fs::metadata(&temporary.0)?.len();
    before_publish();
    check_cancel(cancel)?;
    // Atomic creation without replacement. Unsupported filesystems fail rather than overwrite/copy partially.
    fs::hard_link(&temporary.0, &review.path)
        .context("无法无覆盖发布新文件：目标可能已出现，或文件系统不支持硬链接")?;
    // Publication is the commit point: a late cancellation cannot undo a completed write.
    Ok(Receipt {
        path: review.path.clone(),
        rows,
        bytes,
    })
}

enum Reply {
    Done(Receipt),
    Cancelled,
    Failed(String),
}
#[derive(Default)]
pub(super) struct State {
    #[cfg(feature = "ui-preview")]
    pub(super) preview_rects: [Option<(egui::Rect, egui::Rect)>; 4],
    pub(super) reveal: bool,
    scroll_until: Option<std::time::Instant>,
    pub(super) job: Job,
    review: Option<Review>,
    receiver: Option<Receiver<Reply>>,
    cancel: Arc<AtomicBool>,
    receipt: Option<Receipt>,
    pub(super) open_request: Option<PathBuf>,
    path: String,
    table: String,
    all: bool,
    status: String,
}
impl Drop for State {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl State {
    pub(super) fn modal_open(&self) -> bool {
        self.review.is_some()
    }
    #[cfg(feature = "ui-preview")]
    pub(super) fn preview_review(&mut self, data: &Dataset, indices: &[usize], path: PathBuf) {
        self.path = path.display().to_string();
        self.table = "本地资料".into();
        self.review = Some(
            Review::prepare(data, indices, &self.table, &path, "筛选结果 · 当前排序").unwrap(),
        );
        self.reveal = true;
    }
    pub(super) fn ui(&mut self, ui: &mut egui::Ui, data: Option<&Dataset>, visible: &[usize]) {
        if self.reveal {
            self.scroll_until =
                Some(std::time::Instant::now() + std::time::Duration::from_millis(500));
        }
        let panel = egui::CollapsingHeader::new("另存为 SQLite")
            .id_salt("sqlite-export")
            .open(self.reveal.then_some(true))
            .show(ui, |ui| {
                ui.label("新文件 · 原数据保留 · 先预览再确认 · 不追加或覆盖已有数据库");
                if self.table.is_empty() {
                    self.table = "data".into();
                }
                ui.add_enabled_ui(!self.job.phase.active() && data.is_some(), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("表名");
                        ui.text_edit_singleline(&mut self.table);
                        ui.checkbox(&mut self.all, "保存全部行（忽略筛选与排序）");
                    });
                    ui.add(
                        egui::TextEdit::singleline(&mut self.path)
                            .hint_text("新数据库完整路径，例如 D:\\exports\\result.sqlite")
                            .desired_width(ui.available_width()),
                    );
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("选择新文件…").clicked()
                            && let Some(path) = rfd::FileDialog::new()
                                .add_filter("SQLite", &["sqlite", "db"])
                                .set_file_name("result.sqlite")
                                .save_file()
                        {
                            self.path = path.display().to_string();
                        }
                        let preview = ui.button("预览保存…");
                        #[cfg(feature = "ui-preview")]
                        {
                            self.preview_rects[0] = Some((preview.rect, ui.clip_rect()));
                        }
                        if preview.clicked()
                            && let Some(data) = data
                        {
                            let all = (0..data.rows.len()).collect::<Vec<_>>();
                            let (indices, scope) = if self.all {
                                (all.as_slice(), "全部行 · 原始顺序")
                            } else {
                                (visible, "筛选结果 · 当前排序")
                            };
                            match Review::prepare(
                                data,
                                indices,
                                self.table.trim(),
                                Path::new(self.path.trim()),
                                scope,
                            ) {
                                Ok(review) => {
                                    self.review = Some(review);
                                    self.status.clear();
                                }
                                Err(error) => self.status = error.to_string(),
                            }
                        }
                    });
                });
                if self.job.phase.active() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("正在写入独立临时数据库并校验…");
                        if ui.button("取消保存").clicked() {
                            self.cancel();
                        }
                    });
                }
                if !self.status.is_empty() {
                    ui.label(&self.status);
                }
                if let Some(receipt) = &self.receipt {
                    ui.label(format!("上次保存：{}", receipt.path.display()));
                    let open = ui.button("用 SQLite 浏览器查看…");
                    #[cfg(feature = "ui-preview")]
                    {
                        self.preview_rects[3] = Some((open.rect, ui.clip_rect()));
                    }
                    if open.clicked() {
                        self.open_request = Some(receipt.path.clone());
                    }
                }
            });
        if self
            .scroll_until
            .is_some_and(|until| std::time::Instant::now() < until)
        {
            if let Some(body) = &panel.body_response {
                body.scroll_to_me(Some(egui::Align::Max));
            } else {
                panel.header_response.scroll_to_me(Some(egui::Align::Min));
            }
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        } else {
            self.scroll_until = None;
        }
        self.reveal = false;
        let mut action = 0;
        if let Some(review) = &self.review {
            let response = egui::Modal::new(ui.id().with("sqlite-export-review")).show(ui.ctx(), |ui| {
                ui.set_max_width(620.0);
                ui.heading("确认另存新的 SQLite 文件");
                ui.label(review.path.display().to_string());
                ui.label(format!("{} · {} 行 / {} 列 · 表名：{}", review.scope, review.data.rows.len(), review.data.headers.len(), review.table));
                ui.label("保存的是打开预览时的完整结果快照，不只保存屏幕上的200行。按每个单元格的实际类型存储；CSV字符串不自动转数字。");
                ui.label(format!("类型转换：{} 个布尔值 → 整数0/1；{} 个超大整数 → 十进制文本；{} 个数组/对象 → JSON文本。NULL、普通整数、浮点和字符串按值保留。", review.bools, review.big_ints, review.nested));
                egui::ScrollArea::both().max_height(180.0).show(ui, |ui| {
                    ui.label(format!("字段：{}", review.data.headers.join(" · ")));
                    for row in review.data.rows.iter().take(5) {
                        ui.monospace(row.iter().map(|v| cell_text(v).chars().take(70).collect::<String>()).collect::<Vec<_>>().join(" | "));
                    }
                });
                ui.label("仅创建新文件；目标已存在时拒绝保存。文件生成前可取消，成功后保留文件。");
                ui.small("请选择 NTFS 等支持硬链接的磁盘。其他格式的磁盘可能无法保存，详情见使用说明。");
                ui.horizontal(|ui| {
                    let back = ui.button("返回编辑");
                    let confirm = ui.button("确认保存新文件");
                    #[cfg(feature = "ui-preview")]
                    {
                        self.preview_rects[1] = Some((back.rect, ui.clip_rect()));
                        self.preview_rects[2] = Some((confirm.rect, ui.clip_rect()));
                    }
                    if back.clicked() { action = 1; }
                    if confirm.clicked() { action = 2; }
                });
            });
            if response.should_close() {
                action = 1;
            }
        }
        if action == 1 {
            self.review = None;
        }
        if action == 2 {
            self.start();
        }
    }
    pub(super) fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.job.cancelling();
    }
    #[cfg(feature = "ui-preview")]
    pub(super) fn preview_check(&self, phase: u8) -> bool {
        match phase {
            1 => {
                assert!(self.review.is_none());
                assert!(!Path::new(&self.path).exists());
            }
            2 => {
                assert!(self.review.is_some());
                assert!(!Path::new(&self.path).exists());
            }
            3 => {
                if self.job.phase.active() {
                    return false;
                }
                assert!(matches!(self.job.phase, Phase::Done), "{}", self.status);
                let receipt = self.receipt.as_ref().unwrap();
                assert_eq!(receipt.rows, 2);
                assert!(self.open_request.is_none());
                let conn =
                    Connection::open_with_flags(&receipt.path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                        .unwrap();
                let actual: (String, i64) = conn
                    .query_row(
                        "SELECT \"编号\", \"启用\" FROM \"本地资料\" ORDER BY rowid LIMIT 1",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .unwrap();
                assert_eq!(actual, ("001".into(), 1));
            }
            _ => panic!("invalid SQLite preview phase"),
        }
        true
    }
    pub(super) fn poll(&mut self) {
        let reply = self.receiver.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(r) => Some(r),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Reply::Failed(
                "写入线程意外结束，请检查目标文件后重试".into(),
            )),
        });
        let Some(reply) = reply else { return };
        self.receiver = None;
        match reply {
            Reply::Done(receipt) => {
                self.status = format!(
                    "已保存 {} 行 · {} 字节 · {}",
                    receipt.rows,
                    receipt.bytes,
                    receipt.path.display()
                );
                self.job
                    .finish(Phase::Done, format!("已另存 {} 行", receipt.rows));
                self.receipt = Some(receipt);
                self.reveal = true;
            }
            Reply::Cancelled => {
                self.status = "已取消，未发布新文件".into();
                self.job.finish(Phase::Cancelled, "保存已取消");
            }
            Reply::Failed(error) => {
                self.status = error;
                self.job.finish(Phase::Failed, "保存失败，原表格保留");
            }
        }
    }
    fn start(&mut self) {
        let Some(review) = self.review.take() else {
            return;
        };
        self.cancel = Arc::new(AtomicBool::new(false));
        self.receipt = None;
        self.status.clear();
        self.job.begin();
        let signal = self.cancel.clone();
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        std::thread::spawn(move || {
            let reply = match write(&review, &signal) {
                Ok(receipt) => Reply::Done(receipt),
                Err(error) if error.downcast_ref::<Cancelled>().is_some() => Reply::Cancelled,
                Err(error) => Reply::Failed(format!("{error:#}")),
            };
            let _ = tx.send(reply);
        });
    }
}
