use super::State;
use crate::tasks::{Phase, Row, RunId};
use std::sync::atomic::Ordering;

#[derive(Default, Clone, Copy)]
pub(super) enum Operation {
    #[default]
    Scan,
    Preview,
    Copy,
}

impl State {
    #[cfg(feature = "ui-preview")]
    pub(crate) fn preview_copy_progress_start(&mut self) {
        self.preview_retry_plan();
        self.copy_preview_pause = true;
        self.execute_copy();
    }

    #[cfg(feature = "ui-preview")]
    pub(crate) fn preview_task_execute(&mut self) {
        self.execute_copy();
    }

    #[cfg(feature = "ui-preview")]
    pub(crate) fn preview_wait_for_cancel(&mut self) {
        self.begin_task(Operation::Preview);
        self.copy_cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel = self.copy_cancel.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        self.copy_receiver = Some(receiver);
        std::thread::spawn(move || {
            let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while !cancel.load(Ordering::Relaxed) && std::time::Instant::now() < end {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let _ = sender.send(super::CopyTaskResult::Prepared(Err(
                "controlled preview worker stopped".into(),
            )));
        });
    }

    pub(crate) fn task_snapshot(&self) -> Option<Row> {
        let title = match self.operation {
            Operation::Scan => "目录扫描与对比",
            Operation::Preview => "目录复制预览",
            Operation::Copy => "目录文件复制",
        };
        let mut row = self
            .job
            .snapshot("file-compare", title, self.background_active())?;
        if row.phase.active() {
            row.summary = match self.operation {
                Operation::Scan => self
                    .progress
                    .lock()
                    .map(|p| {
                        format!(
                            "左 {} 项 · 右 {} 项 · 已校验 {} 对",
                            p.left_entries, p.right_entries, p.hashed_pairs
                        )
                    })
                    .unwrap_or_else(|_| "正在读取扫描进度".into()),
                Operation::Preview => "正在校验来源并生成预览，尚未写入目标".into(),
                Operation::Copy => match self.copy_progress.lock() {
                    Ok(progress) => {
                        row.progress = Some(progress.fraction());
                        progress.summary()
                    }
                    Err(_) => "正在读取复制进度；取消后保留已完成文件".into(),
                },
            };
        }
        Some(row)
    }

    pub(crate) fn cancel_task(&mut self, run_id: &RunId) -> bool {
        if !self.background_active()
            || !self.job.phase.active()
            || self
                .task_snapshot()
                .is_none_or(|row| row.run_id() != *run_id)
        {
            return false;
        }
        match self.operation {
            Operation::Scan => self.cancel.store(true, Ordering::Relaxed),
            Operation::Preview | Operation::Copy => self.copy_cancel.store(true, Ordering::Relaxed),
        }
        self.job.cancelling();
        true
    }

    pub(super) fn cancel_current(&mut self) {
        if let Some(row) = self.task_snapshot() {
            self.cancel_task(&row.run_id());
        }
    }

    pub(super) fn begin_task(&mut self, operation: Operation) {
        self.operation = operation;
        self.job.begin();
    }

    /// Poll once in the application loop before rendering and observing completion.
    pub fn poll_background(&mut self) {
        self.poll();
        self.poll_copy();
    }

    pub(super) fn finish_task(&mut self, phase: Phase, summary: impl Into<String>) {
        self.job.finish(phase, summary);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directory_compare::*;

    struct Fixture {
        root: PathBuf,
        state: State,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("zi-directory-tasks-{}", uuid::Uuid::new_v4()));
            for part in ["left", "right", "output"] {
                fs::create_dir_all(root.join(part)).unwrap();
            }
            fs::write(root.join("left/a.txt"), b"source a").unwrap();
            fs::write(root.join("left/b.txt"), b"source b").unwrap();
            fs::write(root.join("output/b.txt"), b"keep me").unwrap();
            let mut state = State::default();
            state.left_path = root.join("left").display().to_string();
            state.right_path = root.join("right").display().to_string();
            Self { root, state }
        }
        fn scan(&mut self) {
            self.state.start();
            assert!(self.state.background_active());
            self.wait();
        }
        fn wait(&mut self) {
            let end = Instant::now() + Duration::from_secs(5);
            while self.state.background_active() && Instant::now() < end {
                self.state.poll_background();
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(!self.state.background_active());
        }
        fn plan(&mut self) {
            self.state.prepare_copy(
                CopySide::Left,
                vec!["a.txt".into(), "b.txt".into()],
                self.root.join("output"),
            );
            assert!(
                self.state.background_active(),
                "preview also blocks upgrade"
            );
            self.wait();
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
                    .starts_with("zi-directory-tasks-")
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn actual_scan_preview_and_copy_finish_without_rendering_the_directory_page() {
        let mut f = Fixture::new();
        assert!(!f.state.background_active() && f.state.task_snapshot().is_none());
        f.scan();
        let scan = f.state.task_snapshot().unwrap();
        assert_eq!(scan.phase, Phase::Done);
        assert_eq!(scan.title, "目录扫描与对比");
        f.plan();
        let preview = f.state.task_snapshot().unwrap();
        assert_eq!(preview.phase, Phase::Done);
        assert_eq!(preview.title, "目录复制预览");
        assert_ne!(scan.run_id(), preview.run_id());
        assert!(!f.root.join("output/a.txt").exists());
        f.state.execute_copy();
        assert!(f.state.background_active(), "copy also blocks upgrade");
        let active = f.state.task_snapshot().unwrap().run_id();
        f.state.start();
        f.state
            .prepare_copy(CopySide::Left, vec![], f.root.join("output"));
        assert_eq!(
            active,
            f.state.task_snapshot().unwrap().run_id(),
            "busy task cannot be replaced"
        );
        f.wait();
        let copy = f.state.task_snapshot().unwrap();
        assert_eq!(copy.phase, Phase::Failed, "conflict is not all-success");
        assert!(copy.summary.contains("成功 1，冲突 1"));
        assert!(!copy.summary.contains(&f.root.display().to_string()));
        assert!(!copy.cancellable);
        assert_eq!(fs::read(f.root.join("output/a.txt")).unwrap(), b"source a");
        assert_eq!(fs::read(f.root.join("output/b.txt")).unwrap(), b"keep me");
        assert!(!f.state.cancel_task(&active));
        let fresh = f.root.join("fresh-output");
        fs::create_dir(&fresh).unwrap();
        f.state
            .prepare_copy(CopySide::Left, vec!["a.txt".into()], fresh.clone());
        f.wait();
        f.state.execute_copy();
        f.wait();
        assert_eq!(f.state.task_snapshot().unwrap().phase, Phase::Done);
        assert_eq!(fs::read(fresh.join("a.txt")).unwrap(), b"source a");
    }

    #[test]
    fn cancel_rejects_stale_generation_and_foreign_owner_then_cancels_preview() {
        let mut f = Fixture::new();
        f.scan();
        f.plan();
        let plan = f.state.copy_plan.take().unwrap();
        let old = f.state.task_snapshot().unwrap().run_id();
        f.state.begin_task(Operation::Preview);
        let (sender, receiver) = mpsc::channel();
        f.state.copy_receiver = Some(receiver);
        assert!(!f.state.cancel_task(&old));
        let mut other = State::default();
        for _ in 0..f.state.job.generation() {
            other.begin_task(Operation::Preview);
        }
        assert_eq!(other.job.generation(), f.state.job.generation());
        assert!(
            !f.state
                .cancel_task(&other.task_snapshot().unwrap().run_id())
        );
        let current = f.state.task_snapshot().unwrap().run_id();
        assert!(f.state.cancel_task(&current));
        assert!(f.state.background_active());
        assert_eq!(f.state.job.phase, Phase::Cancelling);
        sender.send(CopyTaskResult::Prepared(Ok(plan))).unwrap();
        f.state.poll_background();
        assert_eq!(f.state.job.phase, Phase::Cancelled);
        assert!(f.state.copy_plan.is_none());
        assert!(!f.root.join("output/a.txt").exists());
    }

    #[test]
    fn cancelled_copy_keeps_successes_and_disconnection_consumes_plan() {
        let mut f = Fixture::new();
        f.scan();
        f.plan();
        let mut first = f.state.copy_plan.clone().unwrap();
        first.items.truncate(1);
        let result = execute_copy_plan(&first, &AtomicBool::new(false));
        f.state.begin_task(Operation::Copy);
        let (sender, receiver) = mpsc::channel();
        f.state.copy_receiver = Some(receiver);
        let run = f.state.task_snapshot().unwrap().run_id();
        assert!(f.state.cancel_task(&run));
        let mut result = result;
        result.cancelled = true;
        result.outcomes.push(CopyOutcome::Cancelled);
        sender.send(CopyTaskResult::Executed(result)).unwrap();
        f.state.poll_background();
        assert_eq!(f.state.job.phase, Phase::Cancelled);
        assert_eq!(fs::read(f.root.join("output/a.txt")).unwrap(), b"source a");
        assert_eq!(fs::read(f.root.join("output/b.txt")).unwrap(), b"keep me");
        f.state.begin_task(Operation::Copy);
        let (sender, receiver) = mpsc::channel();
        f.state.copy_receiver = Some(receiver);
        drop(sender);
        f.state.poll_background();
        assert_eq!(f.state.job.phase, Phase::Failed);
        assert!(f.state.copy_plan.is_none());
        assert!(!f.state.background_active());
    }

    #[test]
    fn scan_cancellation_and_partial_coverage_are_not_success() {
        let mut f = Fixture::new();
        f.scan();
        for (status, expected) in [
            (ScanStatus::Cancelled, Phase::Cancelled),
            (ScanStatus::Partial, Phase::Failed),
        ] {
            let mut report = f.state.report.clone().unwrap();
            report.status = status;
            f.state.begin_task(Operation::Scan);
            f.state.cancel = Arc::new(AtomicBool::new(false));
            let (sender, receiver) = mpsc::channel();
            f.state.receiver = Some(receiver);
            let run = f.state.task_snapshot().unwrap().run_id();
            assert!(f.state.cancel_task(&run));
            assert!(f.state.cancel.load(Ordering::Relaxed));
            sender.send(Ok(report)).unwrap();
            f.state.poll_background();
            assert_eq!(f.state.job.phase, expected);
            assert!(!f.state.background_active());
        }
    }

    #[test]
    fn rendering_cannot_consume_a_completion_before_the_application_observes_it() {
        let mut f = Fixture::new();
        f.scan();
        f.plan();
        let result =
            execute_copy_plan(f.state.copy_plan.as_ref().unwrap(), &AtomicBool::new(false));
        f.state.begin_task(Operation::Copy);
        let (sender, receiver) = mpsc::channel();
        f.state.copy_receiver = Some(receiver);
        sender.send(CopyTaskResult::Executed(result)).unwrap();
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| f.state.ui(ui));
        });
        assert_eq!(f.state.job.phase, Phase::Running);
        assert!(
            f.state.background_active(),
            "render must leave queued result for app loop"
        );
        let before = f.state.task_snapshot().unwrap();
        f.state.poll_background();
        let after = f.state.task_snapshot().unwrap();
        assert_eq!(before.run_id(), after.run_id());
        assert!(before.phase.active() && !after.phase.active());
        assert_eq!(after.phase, Phase::Failed);
    }
}
