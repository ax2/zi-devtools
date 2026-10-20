//! Small path-free snapshot shared by the copy worker and the UI.
//! Staging bytes are not counted as committed until atomic publication succeeds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct CopyProgress {
    pub total_files: usize,
    pub processed_files: usize,
    pub copied_files: usize,
    pub conflicts: usize,
    pub failed_files: usize,
    pub committed_bytes: u64,
    pub current_bytes: u64,
    pub current_total: Option<u64>,
}

pub(super) enum Outcome {
    Copied,
    Conflict,
    Failed,
}

impl CopyProgress {
    pub fn new(total_files: usize) -> Self {
        Self {
            total_files,
            ..Self::default()
        }
    }

    pub fn start_file(&mut self, bytes: u64) {
        self.current_total = Some(bytes);
        self.current_bytes = 0;
    }

    pub fn staged(&mut self, bytes: u64) {
        self.current_bytes = bytes.min(self.current_total.unwrap_or(0));
    }

    pub fn finish_file(&mut self, outcome: Outcome, bytes: u64) {
        self.processed_files += 1;
        match outcome {
            Outcome::Copied => {
                self.copied_files += 1;
                self.committed_bytes = self.committed_bytes.saturating_add(bytes);
            }
            Outcome::Conflict => self.conflicts += 1,
            Outcome::Failed => self.failed_files += 1,
        }
        self.clear_staging();
    }

    pub fn clear_staging(&mut self) {
        self.current_total = None;
        self.current_bytes = 0;
    }

    pub fn fraction(&self) -> f32 {
        if self.total_files == 0 {
            return 0.0;
        }
        if self.processed_files >= self.total_files {
            return 1.0;
        }
        let current = self
            .current_total
            .filter(|n| *n > 0)
            .map_or(0.0, |n| self.current_bytes as f64 / n as f64);
        // A fully staged final file still needs hash/identity checks and commit.
        ((self.processed_files as f64 + current) / self.total_files as f64).min(0.99) as f32
    }

    pub fn summary(&self) -> String {
        let mut summary = format!(
            "已处理 {} / {} 项 · 成功 {} · 冲突 {} · 失败 {} · 已提交 {}",
            self.processed_files,
            self.total_files,
            self.copied_files,
            self.conflicts,
            self.failed_files,
            format_bytes(self.committed_bytes),
        );
        if let Some(total) = self.current_total {
            summary.push_str(&format!(
                " · 当前文件暂存 {} / {}",
                format_bytes(self.current_bytes),
                format_bytes(total)
            ));
        }
        summary
    }
}

fn format_bytes(value: u64) -> String {
    for (scale, unit) in [(1u64 << 30, "GiB"), (1 << 20, "MiB"), (1 << 10, "KiB")] {
        if value >= scale {
            return format!("{:.1} {unit}", value as f64 / scale as f64);
        }
    }
    format!("{value} B")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directory_compare::{
        CopySide, Limits, compare, execute_copy_plan_with_progress, prepare_copy_plan,
    };
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicBool, Ordering},
    };

    struct Fixture {
        root: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("zi-copy-progress-{}", uuid::Uuid::new_v4()));
            for part in ["left", "right", "output"] {
                fs::create_dir_all(root.join(part)).unwrap();
            }
            Self { root }
        }
        fn plan(&self, names: &[&str]) -> crate::directory_compare::CopyPlan {
            let report = compare(
                &self.root.join("left"),
                &self.root.join("right"),
                &AtomicBool::new(false),
                Limits::default(),
                |_| {},
            )
            .unwrap();
            prepare_copy_plan(
                &report,
                CopySide::Left,
                &names.iter().map(PathBuf::from).collect::<Vec<_>>(),
                &self.root.join("output"),
            )
            .unwrap()
        }
        fn assert_no_temporary_files(&self) {
            assert!(fs::read_dir(self.root.join("output")).unwrap().all(|e| {
                !e.unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".zi-local-")
            }));
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let root = fs::canonicalize(&self.root).unwrap();
            let temp = fs::canonicalize(std::env::temp_dir()).unwrap();
            assert_eq!(root.parent(), Some(temp.as_path()));
            assert!(
                root.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("zi-copy-progress-")
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn actual_copy_reports_chunks_conflicts_and_hash_failure_without_inflating_committed_bytes() {
        let f = Fixture::new();
        fs::write(f.root.join("left/a"), vec![7u8; 131072]).unwrap();
        fs::write(f.root.join("left/b"), []).unwrap();
        fs::write(f.root.join("left/c"), b"source c").unwrap();
        fs::write(f.root.join("left/d"), b"before").unwrap();
        fs::write(f.root.join("output/c"), b"existing c").unwrap();
        let plan = f.plan(&["a", "b", "c", "d"]);
        fs::write(f.root.join("left/d"), b"after!").unwrap();
        let mut updates = Vec::new();
        let result =
            execute_copy_plan_with_progress(&plan, &AtomicBool::new(false), |p| updates.push(p));
        assert_eq!(result.copied_bytes, 131072);
        let half = updates
            .iter()
            .find(|p| p.current_bytes == 65536 && p.copied_files == 0)
            .unwrap();
        assert_eq!(half.committed_bytes, 0);
        assert!(half.fraction() > 0.0 && half.fraction() < 1.0);
        let full_staging = updates.iter().find(|p| p.current_bytes == 131072).unwrap();
        assert_eq!(full_staging.committed_bytes, 0);
        let last = updates.last().unwrap();
        assert_eq!(
            (
                last.processed_files,
                last.copied_files,
                last.conflicts,
                last.failed_files
            ),
            (4, 2, 1, 1)
        );
        assert_eq!(last.committed_bytes, result.copied_bytes);
        assert_eq!(
            fs::read(f.root.join("output/a")).unwrap(),
            vec![7u8; 131072]
        );
        assert_eq!(fs::read(f.root.join("output/b")).unwrap().len(), 0);
        assert_eq!(fs::read(f.root.join("output/c")).unwrap(), b"existing c");
        assert!(!f.root.join("output/d").exists());
        f.assert_no_temporary_files();
    }

    #[test]
    fn cancelling_mid_file_preserves_prior_commits_and_removes_the_partial_file() {
        let f = Fixture::new();
        fs::write(f.root.join("left/a"), b"source a").unwrap();
        fs::write(f.root.join("left/b"), vec![9u8; 131072]).unwrap();
        fs::write(f.root.join("left/c"), b"source c").unwrap();
        let plan = f.plan(&["a", "b", "c"]);
        let cancel = AtomicBool::new(false);
        let mut last = CopyProgress::default();
        let result = execute_copy_plan_with_progress(&plan, &cancel, |p| {
            if p.copied_files == 1 && p.current_bytes >= 65536 {
                cancel.store(true, Ordering::Relaxed);
            }
            last = p;
        });
        assert!(result.cancelled);
        assert_eq!(result.copied_bytes, 8);
        assert_eq!(
            (
                last.processed_files,
                last.copied_files,
                last.committed_bytes
            ),
            (1, 1, 8)
        );
        assert_eq!(last.current_total, None);
        assert_eq!(fs::read(f.root.join("output/a")).unwrap(), b"source a");
        assert!(!f.root.join("output/b").exists() && !f.root.join("output/c").exists());
        f.assert_no_temporary_files();
    }

    #[test]
    fn staged_bytes_are_never_reported_as_committed_and_empty_files_still_need_commit() {
        let mut progress = CopyProgress::new(1);
        progress.start_file(100);
        progress.staged(100);
        assert_eq!(progress.committed_bytes, 0);
        assert!(progress.fraction() < 1.0);
        progress.clear_staging();
        assert_eq!(
            progress.processed_files, 0,
            "cancelled item remains unprocessed"
        );
        progress.start_file(0);
        assert_eq!(progress.fraction(), 0.0);
        progress.finish_file(Outcome::Copied, 0);
        assert_eq!(progress.fraction(), 1.0);
        assert_eq!(progress.copied_files, 1);
    }

    #[test]
    fn conflicts_and_failures_advance_processing_but_not_successful_bytes() {
        let mut progress = CopyProgress::new(3);
        progress.start_file(8);
        progress.finish_file(Outcome::Copied, 8);
        progress.start_file(40);
        progress.finish_file(Outcome::Conflict, 40);
        progress.start_file(80);
        progress.staged(30);
        progress.finish_file(Outcome::Failed, 80);
        assert_eq!(progress.processed_files, 3);
        assert_eq!(progress.fraction(), 1.0);
        assert_eq!(
            (
                progress.copied_files,
                progress.conflicts,
                progress.failed_files
            ),
            (1, 1, 1)
        );
        assert_eq!(progress.committed_bytes, 8);
        assert!(!progress.summary().contains("暂存"));
    }
}
