//! Read-only presentation of one reviewed copy plan's execution results.
use super::{CopyExecution, CopyOutcome, CopyPlan, pagination};
use eframe::egui;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Copied,
    Conflict,
    Failed,
    Pending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Filter {
    All,
    Uncopied,
    Copied,
    Conflict,
    Failed,
    Pending,
}
impl Filter {
    const ALL: [Self; 6] = [
        Self::All,
        Self::Uncopied,
        Self::Copied,
        Self::Conflict,
        Self::Failed,
        Self::Pending,
    ];
    fn includes(self, kind: Kind) -> bool {
        match self {
            Self::All => true,
            Self::Uncopied => kind != Kind::Copied,
            Self::Copied => kind == Kind::Copied,
            Self::Conflict => kind == Kind::Conflict,
            Self::Failed => kind == Kind::Failed,
            Self::Pending => kind == Kind::Pending,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::Uncopied => "未复制",
            Self::Copied => "成功",
            Self::Conflict => "冲突",
            Self::Failed => "失败",
            Self::Pending => "未完成",
        }
    }
    #[cfg(feature = "ui-preview")]
    fn id(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Uncopied => "uncopied",
            Self::Copied => "copied",
            Self::Conflict => "conflicts",
            Self::Failed => "failed",
            Self::Pending => "pending",
        }
    }
}

struct Row {
    relative: PathBuf,
    kind: Kind,
    detail: String,
    search: String,
    bytes: u64,
}

pub(super) struct Review {
    rows: Vec<Row>,
    filter: Filter,
    query: String,
    page: usize,
    cancelled: bool,
    reveal_header: bool,
    source_root: PathBuf,
    destination_root: PathBuf,
}

impl Review {
    pub fn new(plan: &CopyPlan, result: &CopyExecution) -> Self {
        let mut copied = BTreeSet::new();
        let mut outcomes = BTreeMap::new();
        for outcome in &result.outcomes {
            match outcome {
                CopyOutcome::Copied(path) => {
                    copied.insert(path);
                }
                CopyOutcome::Conflict(path)
                | CopyOutcome::Unavailable(path)
                | CopyOutcome::Failed(path, _) => {
                    outcomes.insert(path, outcome);
                }
                CopyOutcome::Cancelled => {}
            }
        }
        let rows: Vec<_> = plan
            .items
            .iter()
            .map(|item| {
                let (kind, detail) = if copied.contains(&item.destination) {
                    (Kind::Copied, "已提交目标文件".to_owned())
                } else {
                    match outcomes.get(&item.relative) {
                        Some(CopyOutcome::Conflict(_)) => {
                            (Kind::Conflict, "目标已存在，未覆盖".to_owned())
                        }
                        Some(CopyOutcome::Unavailable(_)) => {
                            (Kind::Failed, "目标路径不可用，未写入".to_owned())
                        }
                        Some(CopyOutcome::Failed(_, error)) => (Kind::Failed, error.clone()),
                        _ => (
                            Kind::Pending,
                            if result.cancelled {
                                "取消后尚未提交；可能包含已开始暂存的文件"
                            } else {
                                "未收到该文件的完成结果"
                            }
                            .to_owned(),
                        ),
                    }
                };
                let search = format!("{} {detail}", item.relative.display()).to_lowercase();
                Row {
                    relative: item.relative.clone(),
                    kind,
                    detail,
                    search,
                    bytes: item.bytes,
                }
            })
            .collect();
        let filter = if rows.iter().any(|r| r.kind != Kind::Copied) {
            Filter::Uncopied
        } else {
            Filter::All
        };
        Self {
            rows,
            filter,
            query: String::new(),
            page: 0,
            cancelled: result.cancelled,
            reveal_header: true,
            source_root: plan.source_root.clone(),
            destination_root: plan.destination_root.clone(),
        }
    }

    fn count(&self, filter: Filter) -> usize {
        self.rows.iter().filter(|r| filter.includes(r.kind)).count()
    }

    fn transfer(&self) -> anyhow::Result<super::transfer::Review> {
        let query = self.query.trim().to_lowercase();
        let source = super::transfer::path_value(&self.source_root)?;
        let destination = super::transfer::path_value(&self.destination_root)?;
        let rows = self
            .rows
            .iter()
            .filter(|r| self.filter.includes(r.kind) && r.search.contains(&query))
            .map(|r| {
                Ok(vec![
                    super::transfer::path_value(&r.relative)?,
                    serde_json::json!(match r.kind {
                        Kind::Copied => "copied",
                        Kind::Conflict => "conflict",
                        Kind::Failed => "failed",
                        Kind::Pending => "pending",
                    }),
                    serde_json::json!(r.detail),
                    serde_json::json!(r.bytes),
                    source.clone(),
                    destination.clone(),
                    serde_json::json!(self.cancelled),
                ])
            });
        super::transfer::Review::new(
            "目录复制结果",
            format!(
                "分类“{}”、查询“{query}”的全部匹配行，跨分页；原计划{}项，成功{}、未复制{}。取消状态{}；未完成可能包含已开始暂存但未提交的项目。",
                self.filter.label(),
                self.rows.len(),
                self.count(Filter::Copied),
                self.count(Filter::Uncopied),
                self.cancelled
            ),
            &[
                "relative_path",
                "status",
                "detail",
                "planned_bytes",
                "source_root",
                "destination_root",
                "execution_cancelled",
            ],
            rows,
        )
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, transfer: &mut super::transfer::State) {
        let original_page = self.page;
        let header = ui.strong(format!(
            "执行结果 · 共 {} 项 · 成功 {} · 未复制 {}",
            self.rows.len(),
            self.count(Filter::Copied),
            self.count(Filter::Uncopied)
        ));
        #[cfg(feature = "ui-preview")]
        ui.ctx().data_mut(|d| {
            d.insert_temp(egui::Id::new("directory-review-header"), header.rect);
            d.insert_temp(egui::Id::new("directory-review-clip"), ui.clip_rect());
        });
        if self.cancelled {
            ui.weak("任务已取消，已成功文件保留；未完成项需重新预览后再决定是否复制。");
        }
        ui.horizontal_wrapped(|ui| {
            for filter in Filter::ALL {
                let button = ui.selectable_label(
                    self.filter == filter,
                    format!("{} {}", filter.label(), self.count(filter)),
                );
                #[cfg(feature = "ui-preview")]
                ui.ctx().data_mut(|d| {
                    d.insert_temp(
                        egui::Id::new(format!("directory-review-{}", filter.id())),
                        button.rect,
                    )
                });
                if button.clicked() {
                    self.filter = filter;
                    self.page = 0;
                    self.reveal_header = true;
                }
            }
        });
        let query_response = ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .id(egui::Id::new("directory-review-query-input"))
                .hint_text("筛选结果路径或错误内容")
                .desired_width(ui.available_width()),
        );
        #[cfg(feature = "ui-preview")]
        ui.ctx().data_mut(|d| {
            d.insert_temp(egui::Id::new("directory-review-query"), query_response.rect)
        });
        if query_response.changed() {
            self.page = 0;
            self.reveal_header = true;
        }
        let query = self.query.trim().to_lowercase();
        let matches: Vec<_> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| self.filter.includes(r.kind) && r.search.contains(&query))
            .map(|(i, _)| i)
            .collect();
        let send = ui.button(format!(
            "发送筛选复制结果到数据工作台…（{}项）",
            matches.len()
        ));
        #[cfg(feature = "ui-preview")]
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new("directory-transfer-results"), send.rect));
        if send.clicked() {
            transfer.prepare(self.transfer());
        }
        let range = pagination::controls(
            ui,
            &mut self.page,
            matches.len(),
            "directory-copy-results",
            "执行结果",
        );
        if matches.is_empty() {
            ui.weak("没有匹配的执行结果；筛选不会改变复制计划或选择。");
        }
        for index in &matches[range] {
            let row = &self.rows[*index];
            let (label, color) = match row.kind {
                Kind::Copied => ("已复制", ui.visuals().text_color()),
                Kind::Conflict => ("冲突", ui.visuals().warn_fg_color),
                Kind::Failed => ("失败", ui.visuals().error_fg_color),
                Kind::Pending => ("未完成", ui.visuals().weak_text_color()),
            };
            ui.colored_label(color, label);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(row.relative.display().to_string()).monospace(),
                )
                .wrap(),
            );
            ui.label(&row.detail);
            ui.add_space(4.0);
        }
        if self.reveal_header || self.page != original_page {
            // Page/filter changes can replace hundreds of rows at once. Jump
            // directly instead of animating toward a target in the old layout.
            header.scroll_to_me_animation(
                Some(egui::Align::Min),
                egui::style::ScrollAnimation::none(),
            );
            self.reveal_header = false;
        }
    }
}

#[cfg(feature = "ui-preview")]
impl super::State {
    pub(crate) fn preview_result_review_plan(&mut self) {
        self.preview_retry_plan();
        assert_eq!(self.copy_plan.as_ref().unwrap().items.len(), 205);
        self.copy_preview_page = 2;
    }
    pub(crate) fn preview_result_review_ready(&self) -> bool {
        self.copy_result.is_some() && !self.background_active()
    }
    pub(crate) fn preview_result_review_check(&self, step: u8) {
        let review = self.copy_review.as_ref().unwrap();
        assert_eq!(review.count(Filter::All), 205);
        assert_eq!(review.count(Filter::Copied), 199);
        assert_eq!(review.count(Filter::Conflict), 5);
        assert_eq!(review.count(Filter::Failed), 1);
        assert_eq!(review.count(Filter::Pending), 0);
        assert_eq!(self.selected.len(), 205);
        assert_eq!(self.copy_plan.as_ref().unwrap().items.len(), 205);
        assert!(!self.background_active());
        assert_eq!(
            review.filter,
            match step {
                0 => Filter::Uncopied,
                1 => Filter::Conflict,
                2 => Filter::Failed,
                3 | 4 => Filter::All,
                _ => panic!("unknown review step"),
            }
        );
        assert_eq!(review.page, if step == 3 { 2 } else { 0 });
        assert_eq!(review.query, if step == 4 { "file-0204" } else { "" });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directory_compare::{
        CopySide, Limits, compare, execute_copy_plan, execute_copy_plan_with_progress,
        prepare_copy_plan,
    };
    use std::{
        fs,
        sync::atomic::{AtomicBool, Ordering},
    };

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("zi-copy-review-{}", uuid::Uuid::new_v4()));
            for name in ["left", "right", "output"] {
                fs::create_dir_all(root.join(name)).unwrap();
            }
            for name in ["a.txt", "b.txt", "c.txt"] {
                fs::write(root.join("left").join(name), b"source").unwrap();
            }
            Self(root)
        }
        fn plan(&self) -> CopyPlan {
            let report = compare(
                &self.0.join("left"),
                &self.0.join("right"),
                &AtomicBool::new(false),
                Limits::default(),
                |_| {},
            )
            .unwrap();
            prepare_copy_plan(
                &report,
                CopySide::Left,
                &["a.txt".into(), "b.txt".into(), "c.txt".into()],
                &self.0.join("output"),
            )
            .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Ok(root) = self.0.canonicalize() {
                let parent = std::env::temp_dir().canonicalize().unwrap();
                assert_eq!(root.parent(), Some(parent.as_path()));
                assert!(
                    root.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("zi-copy-review-")
                );
                let _ = fs::remove_dir_all(root);
            }
        }
    }

    #[test]
    fn actual_success_conflict_and_changed_source_are_distinct_without_writes_from_review() {
        let f = Fixture::new();
        fs::write(f.0.join("output/b.txt"), b"keep existing").unwrap();
        let plan = f.plan();
        fs::write(f.0.join("left/c.txt"), b"edited").unwrap();
        let result = execute_copy_plan(&plan, &AtomicBool::new(false));
        let review = Review::new(&plan, &result);
        assert_eq!(review.count(Filter::All), 3);
        assert_eq!(review.count(Filter::Copied), 1);
        assert_eq!(review.count(Filter::Conflict), 1);
        assert_eq!(review.count(Filter::Failed), 1);
        assert_eq!(review.count(Filter::Uncopied), 2);
        assert_eq!(review.filter, Filter::Uncopied);
        let table = review.transfer().unwrap();
        assert_eq!(table.data().rows.len(), 2);
        assert_eq!(table.data().rows[0][1], serde_json::json!("conflict"));
        assert_eq!(table.data().rows[1][1], serde_json::json!("failed"));
        assert_eq!(table.data().rows[0][3], serde_json::json!(6));
        assert_eq!(
            fs::read(f.0.join("output/b.txt")).unwrap(),
            b"keep existing"
        );
        assert!(!f.0.join("output/c.txt").exists());
    }

    #[test]
    fn cancelled_uncommitted_items_are_visible_and_unrelated_outcomes_do_not_add_rows() {
        let f = Fixture::new();
        let plan = f.plan();
        let cancel = AtomicBool::new(false);
        let mut result = execute_copy_plan_with_progress(&plan, &cancel, |progress| {
            if progress.copied_files == 1 {
                cancel.store(true, Ordering::Relaxed);
            }
        });
        assert!(result.cancelled);
        result
            .outcomes
            .push(CopyOutcome::Copied(f.0.join("unrelated.txt")));
        let review = Review::new(&plan, &result);
        assert_eq!(review.rows.len(), 3);
        assert_eq!(review.count(Filter::Copied), 1);
        assert_eq!(review.count(Filter::Pending), 2);
        let table = review.transfer().unwrap();
        assert_eq!(table.data().rows.len(), 2);
        assert!(
            table
                .data()
                .rows
                .iter()
                .all(|r| r[1] == "pending" && r[6] == true)
        );
        assert!(!f.0.join("output/b.txt").exists());
        assert!(!f.0.join("output/c.txt").exists());
    }
}
