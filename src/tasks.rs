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
#[derive(Default)]
pub struct Job {
    generation: u64,
    pub phase: Phase,
    started: Option<Instant>,
    elapsed: Duration,
    pub progress: Option<f32>,
    summary: String,
}
impl Job {
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
#[derive(Default)]
pub struct Center {
    pub rows: Vec<Row>,
    cleared: std::collections::HashMap<(&'static str, Option<String>), u64>,
}
impl Center {
    pub fn observe(&mut self, row: Option<Row>) {
        let Some(row) = row else {
            return;
        };
        if self
            .cleared
            .get(&(row.key, row.instance.clone()))
            .is_some_and(|&generation| generation >= row.generation)
        {
            return;
        }
        if let Some(existing) = self.rows.iter_mut().find(|r| {
            r.key == row.key && r.instance == row.instance && r.generation == row.generation
        }) {
            *existing = row;
        } else {
            self.rows.push(row);
        }
        while self.rows.len() > 50 {
            if let Some(i) = self.rows.iter().position(|r| !r.phase.active()) {
                let removed = self.rows.remove(i);
                self.cleared
                    .entry((removed.key, removed.instance))
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
                    .entry((row.key, row.instance.clone()))
                    .and_modify(|g| *g = (*g).max(row.generation))
                    .or_insert(row.generation);
            }
        }
        self.rows.retain(|row| row.phase.active());
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
}
