//! Reviewed, frozen current-page handoff; never executes SQL or replaces existing work.
use super::*;
pub(super) struct Review {
    path: String,
    table: String,
    page: usize,
    has_next: bool,
    data: crate::workbench::Dataset,
}
impl Review {
    pub(super) fn prepare(page: &PageData, path: &str) -> Result<Self> {
        ensure!(
            page.rows.len() <= PAGE_SIZE && page.columns.len() <= MAX_COLUMNS,
            "当前页行列超限"
        );
        ensure!(
            page.truncated_cells == 0,
            "当前页有截断值，不能作为完整数据接力"
        );
        ensure!(
            page.columns.iter().all(|c| c.name.len() <= MAX_CELL_BYTES),
            "列名超过4KiB，未接力"
        );
        let mut bytes = page.columns.iter().map(|c| c.name.len()).sum::<usize>();
        let mut rows = Vec::with_capacity(page.rows.len());
        for (row_index, row) in page.rows.iter().enumerate() {
            ensure!(row.len() == page.columns.len(), "当前页行宽不一致");
            let mut values = Vec::with_capacity(row.len());
            for (column_index, cell) in row.iter().enumerate() {
                ensure!(
                    !cell.truncated,
                    "第{}行/{}列已截断",
                    row_index + 1,
                    column_index + 1
                );
                let value = cell.value.as_ref().context(format!(
                    "第{}行/{}列不能准确接力：BLOB、非UTF8文本或非有限数字尚未支持",
                    row_index + 1,
                    column_index + 1
                ))?;
                let length = value.to_string().len();
                ensure!(length <= MAX_CELL_BYTES * 6 + 2, "单元格JSON表示超限");
                bytes = bytes.saturating_add(length);
                ensure!(bytes <= MAX_PAGE_BYTES, "当前页JSON表示超过2MiB，未接力");
                values.push(value.clone());
            }
            rows.push(values);
        }
        let data = crate::workbench::Dataset::from_parts(
            page.columns.iter().map(|c| c.name.clone()).collect(),
            rows,
        )?;
        Ok(Self {
            path: path.into(),
            table: page.table.clone(),
            page: page.page,
            has_next: page.has_next,
            data,
        })
    }
}
#[derive(Default)]
pub(super) struct State {
    pub(super) review: Option<Review>,
    pub(super) pending: Option<(String, crate::workbench::Dataset)>,
    #[cfg(feature = "ui-preview")]
    pub(super) buttons: [Option<egui::Rect>; 3],
}
impl State {
    pub(super) fn ui(&mut self, ui: &mut egui::Ui) {
        let mut action = 0;
        if let Some(review) = &self.review {
            let response = egui::Modal::new(ui.id().with("sqlite-page-transfer")).show(ui.ctx(), |ui| {
                ui.set_max_width((ui.ctx().screen_rect().width() - 48.0).clamp(180.0, 620.0));
                ui.heading("确认当前页接力");
                ui.label(&review.path);
                ui.label(format!("{} · 第{}页 · {}行 / {}列", review.table, review.page + 1, review.data.rows.len(), review.data.headers.len()));
                ui.label(if review.has_next { "仅导入已读取的当前页，后面还有数据；不代表全表。" } else { "仅导入已读取的当前页；不自动查询其他页或表。" });
                ui.label("创建新的数据实例，保留原流程与已有工作；使用冻结快照，不修改数据库。NULL、整数、有限浮点和UTF8文本保留；0/1不还原为布尔，JSON文本不解析成对象。");
                egui::ScrollArea::both().max_height(150.0).show(ui, |ui| {
                    ui.label(review.data.headers.join(" · "));
                    for row in review.data.rows.iter().take(3) {
                        ui.monospace(row.iter().map(|v| v.to_string().chars().take(45).collect::<String>()).collect::<Vec<_>>().join(" | "));
                    }
                });
                ui.horizontal(|ui| {
                    let cancel = ui.button("取消，保留当前工作");
                    let confirm = ui.button("确认创建数据实例");
                    #[cfg(feature="ui-preview")]
                    { self.buttons[1] = Some(cancel.rect); self.buttons[2] = Some(confirm.rect); }
                    if cancel.clicked() { action = 1; } else if confirm.clicked() { action = 2; }
                });
            });
            if response.should_close() {
                action = 1;
            }
        }
        if action == 1 {
            self.review = None;
        }
        if action == 2
            && let Some(review) = self.review.take()
        {
            let short = review.table.chars().take(36).collect::<String>();
            self.pending = Some((
                format!("SQLite · {short} · 第{}页", review.page + 1),
                review.data,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            Self(
                std::env::temp_dir()
                    .join(format!("zi-page-transfer-{}.sqlite", uuid::Uuid::new_v4())),
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if self.0.exists() {
                std::fs::remove_file(&self.0).unwrap();
            }
        }
    }
    #[test]
    fn actual_page_preserves_typed_unicode_nul_leading_zero_and_source_file() {
        let fixture = Fixture::new();
        {
            let db = Connection::open(&fixture.0).unwrap();
            db.execute_batch("CREATE TABLE data(id TEXT, nullable, integer_value INTEGER, decimal REAL, text_value TEXT)").unwrap();
            db.execute(
                "INSERT INTO data VALUES(?1,NULL,?2,?3,?4)",
                rusqlite::params!["001", i64::MAX, 0.25, "中文🦀\0=1+1"],
            )
            .unwrap();
        }
        let before = std::fs::read(&fixture.0).unwrap();
        let page = load_page(&fixture.0, "data", 0, &Arc::new(AtomicBool::new(false))).unwrap();
        let review = Review::prepare(&page, fixture.0.to_str().unwrap()).unwrap();
        assert_eq!(
            review.data.rows[0],
            vec![
                serde_json::json!("001"),
                serde_json::Value::Null,
                serde_json::json!(i64::MAX),
                serde_json::json!(0.25),
                serde_json::json!("中文🦀\0=1+1")
            ]
        );
        assert_eq!(std::fs::read(&fixture.0).unwrap(), before);
    }
    #[test]
    fn truncated_binary_invalid_utf8_and_nonfinite_cells_cannot_be_passed_as_exact() {
        let fixture = Fixture::new();
        {
            let db = Connection::open(&fixture.0).unwrap();
            db.execute_batch(
                "CREATE TABLE data(value); INSERT INTO data VALUES(CAST(X'FF' AS TEXT))",
            )
            .unwrap();
        }
        let signal = Arc::new(AtomicBool::new(false));
        let mut page = load_page(&fixture.0, "data", 0, &signal).unwrap();
        assert!(Review::prepare(&page, "fixture").is_err());
        for value in [
            ValueRef::Blob(&[]),
            ValueRef::Real(f64::INFINITY),
            ValueRef::Text(b"abc"),
        ] {
            page.rows[0][0] = cell(value, None, &mut 2);
            assert!(Review::prepare(&page, "fixture").is_err());
        }
        page.rows[0][0] = cell(ValueRef::Text(b"ok"), None, &mut 100);
        page.truncated_cells = 1;
        assert!(Review::prepare(&page, "fixture").is_err());
    }
    #[test]
    fn empty_current_page_keeps_columns_and_more_pages_are_explicit() {
        let fixture = Fixture::new();
        {
            let db = Connection::open(&fixture.0).unwrap();
            db.execute_batch("CREATE TABLE data(id TEXT)").unwrap();
        }
        let mut page = load_page(&fixture.0, "data", 0, &Arc::new(AtomicBool::new(false))).unwrap();
        let review = Review::prepare(&page, "fixture").unwrap();
        assert_eq!(review.data.headers, vec!["id"]);
        assert!(review.data.rows.is_empty());
        page.has_next = true;
        page.page = 4;
        let review = Review::prepare(&page, "fixture").unwrap();
        assert!(review.has_next);
        assert_eq!(review.page, 4);
        page.columns[0].name = String::new();
        assert!(Review::prepare(&page, "fixture").is_err());
    }
}
