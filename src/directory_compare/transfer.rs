//! Frozen, explicitly confirmed table handoff. No filesystem access or copy authority.
use super::{Report, Row, ScanStatus};
use crate::workbench::Dataset;
use anyhow::{Context, Result, ensure};
use eframe::egui;
use serde_json::{Value, json};
use std::path::Path;

const MAX_ROWS: usize = 10_000;
const MAX_BYTES: usize = 2 * 1024 * 1024;

pub(super) fn path_value(path: &Path) -> Result<Value> {
    Ok(Value::String(
        path.to_str()
            .context("路径含无法准确转为Unicode的字符，未接力")?
            .into(),
    ))
}

pub(super) fn matches(row: &Row, query: &str) -> bool {
    query.is_empty()
        || format!("{} {}", row.relative.display(), row.difference.label())
            .to_lowercase()
            .contains(query)
}

#[derive(Debug)]
pub(super) struct Review {
    name: String,
    scope: String,
    data: Dataset,
}

impl Review {
    #[cfg(any(test, feature = "ui-preview"))]
    pub(super) fn data(&self) -> &Dataset {
        &self.data
    }
    pub(super) fn new(
        name: &str,
        scope: String,
        headers: &[&str],
        rows: impl IntoIterator<Item = Result<Vec<Value>>>,
    ) -> Result<Self> {
        let mut collected = Vec::new();
        let mut bytes = 0usize;
        for row in rows {
            ensure!(
                collected.len() < MAX_ROWS,
                "匹配结果超过10,000行，请缩小筛选范围；未截断或创建实例"
            );
            let row = row?;
            bytes = bytes.saturating_add(serde_json::to_vec(&row)?.len());
            ensure!(
                bytes <= MAX_BYTES,
                "表格超过2 MiB，请缩小筛选范围；未截断或创建实例"
            );
            collected.push(row);
        }
        let data = Dataset::from_parts(headers.iter().map(|h| (*h).into()).collect(), collected)?;
        ensure!(
            serde_json::to_vec(&data)?.len() <= MAX_BYTES,
            "表格超过2 MiB，请缩小筛选范围；未截断或创建实例"
        );
        Ok(Self {
            name: name.into(),
            scope,
            data,
        })
    }

    pub(super) fn report(report: &Report, query: &str) -> Result<Self> {
        let query = query.trim().to_lowercase();
        let matching: Vec<_> = report
            .rows
            .iter()
            .filter(|r| matches(r, &query))
            .take(MAX_ROWS + 1)
            .collect();
        ensure!(
            matching.len() <= MAX_ROWS,
            "匹配结果超过10,000行，请缩小筛选范围；未截断或创建实例"
        );
        let left = path_value(&report.left_root)?;
        let right = path_value(&report.right_root)?;
        let coverage = match report.status {
            ScanStatus::Complete => "complete",
            ScanStatus::Partial => "partial",
            ScanStatus::Cancelled => "cancelled",
        };
        let rows = matching.into_iter().map(|r| {
            Ok(vec![
                path_value(&r.relative)?,
                json!(r.difference.label()),
                json!(r.left_kind.map(super::ItemKind::label)),
                json!(r.right_kind.map(super::ItemKind::label)),
                json!(r.left_size),
                json!(r.right_size),
                json!(r.left_sha256),
                json!(r.right_sha256),
                left.clone(),
                right.clone(),
                json!(coverage),
                json!(report.limit_reason),
                json!(report.unassessed_one_side),
                json!(report.skipped_links),
                json!(report.skipped_depth),
                json!(report.read_errors),
                json!(report.changed_entries),
                json!(report.skipped_hash_pairs),
            ])
        });
        Self::new(
            "目录差异",
            format!(
                "筛选“{query}”的全部匹配行，跨分页；原报告{}行，覆盖状态{coverage}。仅含差异或未校验项，不代表目录全部内容。未判定单侧项{}、链接跳过{}、读取错误{}。",
                report.rows.len(),
                report.unassessed_one_side,
                report.skipped_links,
                report.read_errors
            ),
            &[
                "relative_path",
                "difference",
                "left_type",
                "right_type",
                "left_size",
                "right_size",
                "left_sha256",
                "right_sha256",
                "left_root",
                "right_root",
                "coverage",
                "limit_reason",
                "unassessed_one_side",
                "skipped_links",
                "skipped_depth",
                "read_errors",
                "changed_entries",
                "skipped_hash_pairs",
            ],
            rows,
        )
    }
}

#[derive(Default)]
pub(super) struct State {
    review: Option<Review>,
    pending: Option<(String, Dataset)>,
    error: String,
}
impl State {
    #[cfg(feature = "ui-preview")]
    pub(super) fn review_rows(&self) -> Option<usize> {
        self.review.as_ref().map(|r| r.data().rows.len())
    }
    pub(super) fn prepare(&mut self, result: Result<Review>) {
        self.error.clear();
        self.review = None;
        self.pending = None;
        match result {
            Ok(review) => self.review = Some(review),
            Err(error) => self.error = format!("接力未完成：{error:#}"),
        }
    }
    pub(super) fn take(&mut self) -> Option<(String, Dataset)> {
        self.pending.take()
    }
    pub(super) fn completed(&mut self, result: Result<()>) {
        match result {
            Ok(()) => {
                self.review = None;
                self.error.clear();
            }
            Err(error) => self.error = format!("接力未完成，快照保留，可重试：{error:#}"),
        }
    }
    pub(super) fn ui(&mut self, ui: &mut egui::Ui) {
        if self.review.is_none() {
            if !self.error.is_empty() {
                let mut close = false;
                let modal = egui::Modal::new(egui::Id::new("directory-transfer-error")).show(
                    ui.ctx(),
                    |ui| {
                        ui.set_max_width(
                            (ui.ctx().screen_rect().width() - 48.0).clamp(180.0, 620.0),
                        );
                        ui.heading("无法发送到数据工作台");
                        ui.label(&self.error);
                        close = ui.button("返回调整筛选").clicked();
                    },
                );
                if close || modal.should_close() {
                    self.error.clear();
                }
            }
            return;
        }
        let review = self.review.as_ref().unwrap();
        let mut action = 0;
        let modal = egui::Modal::new(egui::Id::new("directory-table-transfer")).area(egui::Modal::default_area(egui::Id::new("directory-table-transfer")).fade_in(false)).frame(egui::Frame::popup(ui.style()).fill(ui.visuals().panel_fill)).show(ui.ctx(), |ui| {
            ui.set_max_width((ui.ctx().screen_rect().width() - 48.0).clamp(180.0, 620.0));
            ui.heading("发送到 CSV/JSON 工作台");
            ui.strong(format!("{} · {}行 / {}列", review.name, review.data.rows.len(), review.data.headers.len()));
            ui.label(&review.scope);
            ui.label("使用点击时的结果快照，创建独立数据实例并保留已有工作。文件大小保留数字、缺失值保留NULL；不会重新扫描、读取或复制文件。新实例未启用自动保存，可在工作台主动保存或导出。");
            egui::ScrollArea::both().max_height(160.0).show(ui, |ui| {
                egui::Grid::new("directory-transfer-table-preview").striped(true).show(ui, |ui| {
                    for header in &review.data.headers {
                        ui.add_sized([140.0, 22.0], egui::Label::new(egui::RichText::new(header).strong()).truncate()).on_hover_text(header);
                    }
                    ui.end_row();
                    for row in review.data.rows.iter().take(3) {
                        for value in row {
                            let text = value.as_str().map(str::to_owned).unwrap_or_else(|| value.to_string());
                            ui.add_sized([140.0, 22.0], egui::Label::new(&text).truncate()).on_hover_text(text);
                        }
                        ui.end_row();
                    }
                });
            });
            ui.small("上方仅预览前3行，长值缩略显示；传递数据保留完整值。");
            if !self.error.is_empty() { ui.colored_label(ui.visuals().error_fg_color, &self.error); }
            ui.horizontal_wrapped(|ui| {
                for (id, label, value) in [("cancel", "取消", 1), ("confirm", "确认创建数据实例", 2)] {
                    let button = ui.button(label);
                    #[cfg(feature="ui-preview")]
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(format!("directory-transfer-{id}")), button.rect));
                    #[cfg(not(feature="ui-preview"))]
                    let _ = id;
                    if button.clicked() { action = value; }
                }
            });
        });
        if modal.should_close() {
            action = 1;
        }
        match action {
            1 => {
                self.review = None;
                self.error.clear();
            }
            2 => self.pending = Some((review.name.clone(), review.data.clone())),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directory_compare::{Limits, compare};
    use std::{fs, path::PathBuf, sync::atomic::AtomicBool};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .join(format!("zi-directory-transfer-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(root.join("left")).unwrap();
            fs::create_dir_all(root.join("right")).unwrap();
            Self(root)
        }
        fn report(&self, limits: Limits) -> Report {
            compare(
                &self.0.join("left"),
                &self.0.join("right"),
                &AtomicBool::new(false),
                limits,
                |_| {},
            )
            .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let root = self.0.canonicalize().unwrap();
            assert_eq!(
                root.parent(),
                Some(std::env::temp_dir().canonicalize().unwrap().as_path())
            );
            assert!(
                root.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("zi-directory-transfer-")
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn actual_report_transfers_all_matching_pages_with_types_and_frozen_values() {
        let f = Fixture::new();
        for index in 0..205 {
            fs::write(
                f.0.join("left").join(format!("=中文-{index:04}.txt")),
                b"123",
            )
            .unwrap();
        }
        let report = f.report(Limits::default());
        let review = Review::report(&report, "中文").unwrap();
        assert_eq!(review.data.rows.len(), 205);
        assert_eq!(review.data.rows[0][0], json!("=中文-0000.txt"));
        assert_eq!(review.data.rows[0][4], json!(3));
        assert!(review.data.rows[0][5].is_null());
        assert_eq!(review.data.rows[0][10], json!("complete"));
        assert_eq!(Review::report(&report, "0204").unwrap().data.rows.len(), 1);
        fs::write(f.0.join("left/=中文-0000.txt"), b"changed after report").unwrap();
        assert_eq!(
            Review::report(&report, "0000").unwrap().data.rows[0][4],
            json!(3)
        );
        assert_eq!(fs::read_dir(f.0.join("right")).unwrap().count(), 0);
    }

    #[test]
    fn actual_partial_hash_report_keeps_coverage_and_skip_evidence() {
        let f = Fixture::new();
        for side in ["left", "right"] {
            fs::write(f.0.join(side).join("a"), b"same").unwrap();
        }
        let report = f.report(Limits {
            max_hash_bytes: 0,
            ..Limits::default()
        });
        let review = Review::report(&report, "").unwrap();
        assert_eq!(review.data.rows[0][10], json!("partial"));
        assert_eq!(review.data.rows[0][17], json!(1));
        assert_eq!(review.data.rows[0][1], json!("未校验内容"));
        assert!(review.data.rows[0][6].is_null());
    }

    #[test]
    fn oversized_tables_fail_without_truncation_and_import_failure_retains_review() {
        assert!(
            Review::new(
                "rows",
                String::new(),
                &["value"],
                (0..=MAX_ROWS).map(|i| Ok(vec![json!(i)]))
            )
            .unwrap_err()
            .to_string()
            .contains("10,000")
        );
        assert!(
            Review::new(
                "bytes",
                String::new(),
                &["value"],
                [Ok(vec![json!("x".repeat(MAX_BYTES))])]
            )
            .unwrap_err()
            .to_string()
            .contains("2 MiB")
        );
        let review = Review::new("empty", String::new(), &["value"], []).unwrap();
        let mut state = State::default();
        state.prepare(Ok(review));
        state.completed(Err(anyhow::anyhow!("fixture capacity reached")));
        assert!(state.review.is_some() && state.pending.is_none());
        state.completed(Ok(()));
        assert!(state.review.is_none());
    }

    #[cfg(windows)]
    #[test]
    fn non_unicode_windows_paths_are_rejected_instead_of_replaced() {
        use std::{ffi::OsString, os::windows::ffi::OsStringExt};
        assert!(path_value(Path::new(&OsString::from_wide(&[0xd800]))).is_err());
    }
}
