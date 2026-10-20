use super::{BTreeSet, CopyExecution, CopyOutcome, CopyPlan, PathBuf, ScanStatus, State};

/// Select from the reviewed plan, never from arbitrary paths in an error message.
/// Cancellation has no per-file path, so every item without a success stays in.
pub(super) fn remaining(plan: &CopyPlan, result: &CopyExecution) -> BTreeSet<PathBuf> {
    let copied: BTreeSet<_> = result
        .outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            CopyOutcome::Copied(path) => Some(path.as_path()),
            _ => None,
        })
        .collect();
    plan.items
        .iter()
        .filter(|item| !copied.contains(item.destination.as_path()))
        .map(|item| item.relative.clone())
        .collect()
}

impl State {
    #[cfg(feature = "ui-preview")]
    pub fn preview_retry_plan(&mut self) {
        let report = self.report.as_ref().unwrap();
        self.selected = report.rows.iter().map(|row| row.relative.clone()).collect();
        self.copy_plan = Some(
            super::prepare_copy_plan(
                report,
                self.copy_side,
                &self.selected.iter().cloned().collect::<Vec<_>>(),
                std::path::Path::new(&self.destination_path),
            )
            .unwrap(),
        );
    }

    pub(super) fn select_uncopied(&mut self) -> bool {
        if self.copy_receiver.is_some()
            || self.receiver.is_some()
            || !self
                .report
                .as_ref()
                .is_some_and(|report| report.status == ScanStatus::Complete)
        {
            return false;
        }
        let (Some(plan), Some(result)) = (&self.copy_plan, &self.copy_result) else {
            return false;
        };
        if plan.side != self.copy_side {
            return false;
        }
        let selected = remaining(plan, result);
        if selected.is_empty() {
            return false;
        }
        self.selected = selected;
        // Retain the last result for review. A consumed plan cannot execute again;
        // prepare_copy will replace it only after the user asks for a new preview.
        self.message = format!(
            "已仅选择 {} 个未复制项；上次结果保留。本次选择操作不会写入文件，请重新预览后再执行。",
            self.selected.len()
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    struct Fixture {
        root: PathBuf,
        state: State,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("zi-copy-retry-{}", uuid::Uuid::new_v4()));
            let left = root.join("left");
            let right = root.join("right");
            let output = root.join("output");
            for path in [&left, &right, &output] {
                fs::create_dir_all(path).unwrap();
            }
            for name in ["a.txt", "b.txt", "c.txt"] {
                fs::write(left.join(name), format!("source {name}")).unwrap();
                fs::write(right.join(name), b"different").unwrap();
            }
            fs::write(output.join("b.txt"), b"existing destination").unwrap();
            let report = compare(
                &left,
                &right,
                &AtomicBool::new(false),
                Limits::default(),
                |_| {},
            )
            .unwrap();
            let selected = vec![
                PathBuf::from("a.txt"),
                PathBuf::from("b.txt"),
                PathBuf::from("c.txt"),
            ];
            let plan = prepare_copy_plan(&report, CopySide::Left, &selected, &output).unwrap();
            let mut state = State::default();
            state.report = Some(report);
            state.selected = selected.into_iter().collect();
            state.copy_plan = Some(plan);
            state.destination_path = output.display().to_string();
            Self { root, state }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let Ok(root) = fs::canonicalize(&self.root) else {
                return;
            };
            let Ok(temp) = fs::canonicalize(std::env::temp_dir()) else {
                return;
            };
            if root.parent() == Some(temp.as_path())
                && root
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("zi-copy-retry-"))
            {
                let _ = fs::remove_dir_all(root);
            }
        }
    }
    fn wait(state: &mut State) {
        let until = Instant::now() + Duration::from_secs(5);
        while state.copy_receiver.is_some() && Instant::now() < until {
            state.poll_copy();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(state.copy_receiver.is_none(), "copy worker must finish");
    }

    #[test]
    fn cancelled_unattempted_and_failed_items_remain_but_successes_do_not() {
        let f = Fixture::new();
        let plan = f.state.copy_plan.as_ref().unwrap();
        let mut attempted = plan.clone();
        attempted.items.truncate(2);
        let mut result = execute_copy_plan(&attempted, &AtomicBool::new(false));
        result.cancelled = true;
        result.outcomes.push(CopyOutcome::Cancelled);
        assert_eq!(
            super::remaining(plan, &result),
            BTreeSet::from([PathBuf::from("b.txt"), PathBuf::from("c.txt")])
        );
        result.outcomes.push(CopyOutcome::Failed(
            "c.txt".into(),
            "synthetic failure".into(),
        ));
        result
            .outcomes
            .push(CopyOutcome::Unavailable("b.txt".into()));
        result
            .outcomes
            .push(CopyOutcome::Copied(f.root.join("unrelated.txt")));
        assert_eq!(super::remaining(plan, &result).len(), 2);
        assert_eq!(
            fs::read(f.root.join("output/a.txt")).unwrap(),
            b"source a.txt"
        );
        assert!(!f.root.join("output/c.txt").exists());
    }

    #[test]
    fn retry_only_changes_selection_and_requires_a_fresh_preview() {
        let mut f = Fixture::new();
        f.state.execute_copy();
        wait(&mut f.state);
        assert!(f.state.select_uncopied());
        assert_eq!(f.state.selected, BTreeSet::from([PathBuf::from("b.txt")]));
        assert!(f.state.copy_result.is_some() && f.state.copy_plan.is_some());
        assert!(!can_execute_copy(
            f.state.copy_plan.as_ref(),
            f.state.copy_result.as_ref(),
            false
        ));
        f.state.execute_copy();
        assert!(
            f.state.copy_receiver.is_none(),
            "consumed plan must not start another worker"
        );
        assert_eq!(
            fs::read(f.root.join("output/b.txt")).unwrap(),
            b"existing destination"
        );
        let fresh = f.root.join("fresh-output");
        fs::create_dir(&fresh).unwrap();
        change_copy_destination(
            &mut f.state.destination_path,
            &mut f.state.copy_plan,
            &mut f.state.copy_result,
            fresh.display().to_string(),
        );
        fs::write(
            f.root.join("left/b.txt"),
            b"updated source after first attempt",
        )
        .unwrap();
        f.state.prepare_copy(
            CopySide::Left,
            f.state.selected.iter().cloned().collect(),
            fresh.clone(),
        );
        wait(&mut f.state);
        let plan = f.state.copy_plan.as_ref().unwrap();
        assert_eq!(plan.items.len(), 1);
        assert_eq!(
            plan.items[0].sha256,
            format!(
                "{:x}",
                Sha256::digest(b"updated source after first attempt")
            )
        );
        assert!(
            !fresh.join("b.txt").exists(),
            "new preview remains read-only"
        );
        f.state.execute_copy();
        wait(&mut f.state);
        assert_eq!(
            fs::read(fresh.join("b.txt")).unwrap(),
            b"updated source after first attempt"
        );
        assert!(!fresh.join("a.txt").exists() && !fresh.join("c.txt").exists());
        assert!(
            !f.state.select_uncopied(),
            "all copied leaves selection untouched"
        );
    }

    #[test]
    fn busy_workers_and_cancelled_plans_require_review() {
        let mut f = Fixture::new();
        let result = execute_copy_plan(f.state.copy_plan.as_ref().unwrap(), &AtomicBool::new(true));
        assert!(result.cancelled);
        assert_eq!(
            super::remaining(f.state.copy_plan.as_ref().unwrap(), &result).len(),
            3
        );
        assert!(!f.root.join("output/a.txt").exists());
        f.state.copy_result = Some(result);
        let (sender, receiver) = mpsc::channel();
        f.state.copy_receiver = Some(receiver);
        assert!(!f.state.select_uncopied());
        f.state.copy_result = None;
        f.state.execute_copy();
        // A refused execute must retain the original channel and cancellation owner.
        sender
            .send(CopyTaskResult::Executed(CopyExecution {
                outcomes: vec![CopyOutcome::Cancelled],
                copied_bytes: 0,
                cancelled: true,
            }))
            .unwrap();
        f.state.poll_copy();
        assert!(f.state.copy_receiver.is_none());
        assert!(f.state.select_uncopied());
        assert_eq!(f.state.selected.len(), 3);
    }
}
