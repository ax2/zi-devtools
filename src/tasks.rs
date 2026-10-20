//! In-memory metadata only. Task inputs, paths and credentials are never retained here.
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    Running,
    Cancelling,
    Done,
    Failed,
    Cancelled,
}
impl Phase {
    pub fn active(self) -> bool {
        matches!(self, Self::Running | Self::Cancelling)
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "未开始",
            Self::Running => "运行中",
            Self::Cancelling => "正在取消",
            Self::Done => "已完成",
            Self::Failed => "失败",
            Self::Cancelled => "已取消",
        }
    }
}
pub struct Job {
    owner_id: uuid::Uuid,
    generation: u64,
    pub phase: Phase,
    started: Option<Instant>,
    elapsed: Duration,
    pub progress: Option<f32>,
    summary: String,
}

impl Default for Job {
    fn default() -> Self {
        Self {
            owner_id: uuid::Uuid::new_v4(),
            generation: 0,
            phase: Phase::Idle,
            started: None,
            elapsed: Duration::ZERO,
            progress: None,
            summary: String::new(),
        }
    }
}

impl Job {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn begin(&mut self) {
        self.generation += 1;
        self.phase = Phase::Running;
        self.started = Some(Instant::now());
        self.elapsed = Duration::ZERO;
        self.progress = None;
        self.summary.clear();
    }
    pub fn cancelling(&mut self) {
        if self.phase == Phase::Running {
            self.phase = Phase::Cancelling;
        }
    }
    pub fn finish(&mut self, phase: Phase, summary: impl Into<String>) {
        if !self.phase.active() {
            return;
        }
        debug_assert!(!phase.active() && phase != Phase::Idle);
        self.elapsed = self.started.map_or(Duration::ZERO, |t| t.elapsed());
        self.phase = phase;
        self.summary = summary.into();
        if phase == Phase::Done {
            self.progress = Some(1.0);
        }
    }
    pub fn snapshot(
        &self,
        key: &'static str,
        title: &'static str,
        cancellable: bool,
    ) -> Option<Row> {
        self.started.map(|started| Row {
            owner_id: self.owner_id,
            key,
            instance: None,
            instance_name: None,
            generation: self.generation,
            title,
            phase: self.phase,
            elapsed: if self.phase.active() {
                started.elapsed()
            } else {
                self.elapsed
            },
            progress: self.progress,
            summary: self.summary.clone(),
            cancellable,
        })
    }
}
#[derive(Clone)]
pub struct Row {
    pub(crate) owner_id: uuid::Uuid,
    pub key: &'static str,
    pub instance: Option<String>,
    pub instance_name: Option<String>,
    pub generation: u64,
    pub title: &'static str,
    pub phase: Phase,
    pub elapsed: Duration,
    pub progress: Option<f32>,
    pub summary: String,
    pub cancellable: bool,
}

/// Identity of one observed task run. The owner UUID distinguishes separate
/// Job instances that use the same tool/instance key and local generation. It
/// is process-local and is not a durable job identifier.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RunId {
    owner_id: uuid::Uuid,
    key: &'static str,
    instance: Option<String>,
    generation: u64,
}

impl RunId {
    pub fn key(&self) -> &'static str {
        self.key
    }

    pub fn instance(&self) -> Option<&str> {
        self.instance.as_deref()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn slot(&self) -> TaskSlotId {
        TaskSlotId {
            owner_id: self.owner_id,
            key: self.key,
            instance: self.instance.clone(),
        }
    }
}

impl Row {
    pub fn run_id(&self) -> RunId {
        RunId {
            owner_id: self.owner_id,
            key: self.key,
            instance: self.instance.clone(),
            generation: self.generation,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TaskSlotId {
    owner_id: uuid::Uuid,
    key: &'static str,
    instance: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewFilter {
    #[default]
    All,
    Active,
    Failed,
    Finished,
}
impl ViewFilter {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::Active => "运行中",
            Self::Failed => "失败",
            Self::Finished => "已结束",
        }
    }
    fn matches(self, row: &Row) -> bool {
        match self {
            Self::All => true,
            Self::Active => row.phase.active(),
            Self::Failed => row.phase == Phase::Failed,
            Self::Finished => !row.phase.active(),
        }
    }
}
#[derive(Default)]
pub struct Center {
    pub rows: Vec<Row>,
    cleared: std::collections::HashMap<TaskSlotId, u64>,
}
impl Center {
    pub fn cancellable_running(&self) -> Vec<Row> {
        self.rows
            .iter()
            .filter(|row| row.phase == Phase::Running && row.cancellable)
            .cloned()
            .collect()
    }

    pub fn observe(&mut self, row: Option<Row>) {
        let Some(row) = row else {
            return;
        };
        let run_id = row.run_id();
        if self
            .cleared
            .get(&run_id.slot())
            .is_some_and(|&generation| generation >= run_id.generation())
        {
            return;
        }
        if let Some(existing) = self.rows.iter_mut().find(|r| r.run_id() == run_id) {
            *existing = row;
        } else {
            self.rows.push(row);
        }
        while self.rows.len() > 50 {
            if let Some(i) = self.rows.iter().position(|r| !r.phase.active()) {
                let removed = self.rows.remove(i);
                self.cleared
                    .entry(removed.run_id().slot())
                    .and_modify(|g| *g = (*g).max(removed.generation))
                    .or_insert(removed.generation);
            } else {
                break;
            }
        }
    }
    pub fn clear_finished(&mut self) {
        for row in &self.rows {
            if !row.phase.active() {
                self.cleared
                    .entry(row.run_id().slot())
                    .and_modify(|g| *g = (*g).max(row.generation))
                    .or_insert(row.generation);
            }
        }
        self.rows.retain(|row| row.phase.active());
    }

    /// Search only stable, user-visible task labels. Never search summaries,
    /// which may be supplied by tool implementations and can contain data.
    pub fn filtered(&self, filter: ViewFilter, query: &str) -> Vec<&Row> {
        let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        self.rows
            .iter()
            .filter(|row| filter.matches(row))
            .filter(|row| {
                if terms.is_empty() {
                    return true;
                }
                let searchable = format!(
                    "{} {}",
                    row.title,
                    row.instance_name.as_deref().unwrap_or_default()
                )
                .to_lowercase();
                terms.iter().all(|term| searchable.contains(term))
            })
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn same_tool_generations_are_separate_for_each_instance() {
        let mut center = Center::default();
        let mut first = Job::default();
        let mut second = Job::default();
        first.begin();
        second.begin();
        let row = |job: &Job, id: &str| {
            let mut r = job.snapshot("data", "数据", false).unwrap();
            r.instance = Some(id.into());
            r
        };
        center.observe(Some(row(&first, "one")));
        center.observe(Some(row(&second, "two")));
        assert_eq!(center.rows.len(), 2);
        first.finish(Phase::Done, "first result");
        center.observe(Some(row(&first, "one")));
        center.clear_finished();
        assert_eq!(center.rows.len(), 1);
        assert_eq!(center.rows[0].instance.as_deref(), Some("two"));
        center.observe(Some(row(&first, "one")));
        assert_eq!(center.rows.len(), 1);
        second.finish(Phase::Done, "second result");
        center.observe(Some(row(&second, "two")));
        assert_eq!(center.rows[0].summary, "second result");
    }

    #[test]
    fn run_id_compares_tool_instance_and_generation_as_one_identity() {
        let mut job = Job::default();
        job.begin();
        let first = job.snapshot("data", "数据", true).unwrap();
        let mut other_instance = first.clone();
        other_instance.instance = Some("other".into());
        assert_ne!(first.run_id(), other_instance.run_id());

        let mut other_tool = first.clone();
        other_tool.key = "files";
        assert_ne!(first.run_id(), other_tool.run_id());

        job.begin();
        let next_generation = job.snapshot("data", "数据", true).unwrap();
        assert_ne!(first.run_id(), next_generation.run_id());
        assert_eq!(first.run_id().key(), "data");
        assert_eq!(first.run_id().instance(), None);
        assert_eq!(next_generation.run_id().generation(), 2);
    }

    #[test]
    fn run_id_separates_job_owners_with_the_same_local_generation() {
        let mut first_job = Job::default();
        let mut second_job = Job::default();
        first_job.begin();
        second_job.begin();
        let first = first_job.snapshot("data", "数据", true).unwrap();
        let second = second_job.snapshot("data", "数据", true).unwrap();

        assert_eq!(first.generation, second.generation);
        assert_eq!(first.key, second.key);
        assert_eq!(first.instance, second.instance);
        assert_ne!(first.run_id(), second.run_id());

        let mut center = Center::default();
        center.observe(Some(first.clone()));
        center.observe(Some(second.clone()));
        assert_eq!(center.rows.len(), 2);
        first_job.finish(Phase::Done, "完成");
        second_job.finish(Phase::Done, "完成");
        center.observe(first_job.snapshot("data", "数据", true));
        center.observe(second_job.snapshot("data", "数据", true));
        center.clear_finished();
        center.observe(Some(first));
        center.observe(Some(second));
        assert!(center.rows.is_empty());
    }
    #[test]
    fn cancelling_is_not_terminal_and_history_clear_does_not_resurrect() {
        let mut job = Job::default();
        let mut center = Center::default();
        job.begin();
        job.cancelling();
        assert!(job.phase.active());
        center.observe(job.snapshot("data", "数据", true));
        center.clear_finished();
        assert_eq!(center.rows.len(), 1);
        job.finish(Phase::Cancelled, "取消完成");
        center.observe(job.snapshot("data", "数据", true));
        center.clear_finished();
        center.observe(job.snapshot("data", "数据", true));
        assert!(center.rows.is_empty());
        job.begin();
        center.observe(job.snapshot("data", "数据", true));
        assert_eq!(center.rows.len(), 1);
    }
    #[test]
    fn history_is_bounded_and_does_not_lose_running_jobs() {
        let mut center = Center::default();
        let mut active = Job::default();
        active.begin();
        center.observe(active.snapshot("active", "运行中", true));
        let mut job = Job::default();
        for _ in 0..70 {
            job.begin();
            job.finish(Phase::Done, "完成");
            center.observe(job.snapshot("data", "数据", false));
        }
        assert_eq!(center.rows.len(), 50);
        assert!(center.rows.iter().any(|r| r.key == "active"));
    }

    #[test]
    fn filters_and_search_match_task_labels_without_searching_summaries() {
        let mut center = Center::default();
        let mut running = Job::default();
        running.begin();
        let mut running_row = running.snapshot("data", "数据解析", true).unwrap();
        running_row.instance_name = Some("每月账单".into());
        running_row.summary = "private-cell-value".into();
        center.observe(Some(running_row));

        let mut failed = Job::default();
        failed.begin();
        failed.finish(Phase::Failed, "private-cell-value");
        center.observe(failed.snapshot("files", "文件校验", true));

        assert_eq!(center.filtered(ViewFilter::Active, "账单").len(), 1);
        assert_eq!(center.filtered(ViewFilter::Failed, "文件").len(), 1);
        assert!(
            center
                .filtered(ViewFilter::All, "private-cell-value")
                .is_empty()
        );
        assert_eq!(center.filtered(ViewFilter::Finished, "").len(), 1);
        assert!(center.filtered(ViewFilter::All, "数据 不存在").is_empty());
    }

    #[test]
    fn bulk_cancel_candidates_include_only_running_cancellable_rows() {
        let mut center = Center::default();
        let mut running = Job::default();
        running.begin();
        let mut cancellable = running.snapshot("files", "文件校验", true).unwrap();
        cancellable.instance = Some("folder-a".into());
        center.observe(Some(cancellable));

        let mut non_cancellable = running.snapshot("data", "数据解析", false).unwrap();
        non_cancellable.instance = Some("table-a".into());
        center.observe(Some(non_cancellable));

        running.cancelling();
        let mut cancelling = running
            .snapshot("image-workflow", "图片流程", true)
            .unwrap();
        cancelling.instance = Some("image-a".into());
        center.observe(Some(cancelling));

        let targets = center.cancellable_running();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].key, "files");
        assert_eq!(targets[0].instance.as_deref(), Some("folder-a"));
        assert_eq!(targets[0].phase, Phase::Running);
    }
}
