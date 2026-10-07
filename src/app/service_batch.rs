use crate::{
    config::ServiceSpec,
    tasks::{Job, Phase},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Done,
    Skipped,
    Failed,
}
impl Outcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::Done => "成功",
            Self::Skipped => "跳过",
            Self::Failed => "失败",
        }
    }
}
pub(super) struct Item {
    pub id: String,
    pub outcome: Outcome,
    pub message: String,
}
pub(super) enum Event {
    Current(String),
    Item(Item),
    Finished(bool),
}
#[derive(Default)]
pub(super) struct State {
    pub job: Job,
    pub generation: u64,
    pub total: usize,
    pub current: Option<String>,
    pub rows: Vec<Item>,
    pub cancel: Arc<AtomicBool>,
}
impl State {
    pub fn busy(&self) -> bool {
        self.job.phase.active()
    }
    pub fn begin(&mut self, total: usize) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.total = total;
        self.current = None;
        self.rows.clear();
        self.cancel = Arc::new(AtomicBool::new(false));
        self.job.begin();
        self.generation
    }
    pub fn cancel(&mut self, generation: u64) {
        if generation == self.generation && self.busy() {
            self.cancel.store(true, Ordering::Release);
            self.job.cancelling();
        }
    }
    pub fn apply(&mut self, generation: u64, event: Event) -> bool {
        if generation != self.generation || !self.busy() {
            return false;
        }
        match event {
            Event::Current(id) => self.current = Some(id),
            Event::Item(item) => {
                self.rows.push(item);
                self.job.progress = Some(self.rows.len() as f32 / self.total.max(1) as f32);
            }
            Event::Finished(cancelled) => {
                self.current = None;
                let done = self
                    .rows
                    .iter()
                    .filter(|r| r.outcome == Outcome::Done)
                    .count();
                let skipped = self
                    .rows
                    .iter()
                    .filter(|r| r.outcome == Outcome::Skipped)
                    .count();
                let failed = self
                    .rows
                    .iter()
                    .filter(|r| r.outcome == Outcome::Failed)
                    .count();
                self.job.finish(
                    if cancelled {
                        Phase::Cancelled
                    } else if failed > 0 {
                        Phase::Failed
                    } else {
                        Phase::Done
                    },
                    format!(
                        "成功 {done} · 跳过 {skipped} · 失败 {failed} · 未执行 {}",
                        self.total.saturating_sub(self.rows.len())
                    ),
                );
                return true;
            }
        }
        false
    }
}
pub(super) fn run(
    specs: Vec<ServiceSpec>,
    cancel: &AtomicBool,
    mut emit: impl FnMut(Event),
    mut execute: impl FnMut(&ServiceSpec) -> Result<Option<String>, String>,
) {
    let mut cancelled = false;
    for spec in specs {
        if cancel.load(Ordering::Acquire) {
            cancelled = true;
            break;
        }
        emit(Event::Current(spec.id.clone()));
        let (outcome, message) = match execute(&spec) {
            Ok(Some(message)) => (Outcome::Done, message),
            Ok(None) => (
                Outcome::Skipped,
                "当前状态无需执行；外部进程保持不动".into(),
            ),
            Err(error) => (Outcome::Failed, error),
        };
        let mut bounded: String = message.chars().take(1024).collect();
        if bounded.len() < message.len() {
            bounded.push_str("…（摘要截断）");
        }
        emit(Event::Item(Item {
            id: spec.id,
            outcome,
            message: bounded,
        }));
    }
    emit(Event::Finished(cancelled));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn worker_cancels_between_items_and_keeps_bounded_partial_failures() {
        let mut spec: ServiceSpec =
            serde_yaml_ng::from_str("repo: .\ncommand: echo fixture\n").unwrap();
        let specs: Vec<_> = (0..3)
            .map(|i| {
                spec.id = format!("fixture-{i}");
                spec.clone()
            })
            .collect();
        let cancel = AtomicBool::new(false);
        let mut events = Vec::new();
        run(
            specs.clone(),
            &cancel,
            |e| events.push(e),
            |_| {
                cancel.store(true, Ordering::Release);
                Ok(Some("first done".into()))
            },
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, Event::Item(_)))
                .count(),
            1
        );
        assert!(matches!(events.last(), Some(Event::Finished(true))));
        cancel.store(false, Ordering::Release);
        events.clear();
        let mut calls = 0;
        run(
            specs,
            &cancel,
            |e| events.push(e),
            |_| {
                calls += 1;
                match calls {
                    1 => Ok(Some("done".into())),
                    2 => Ok(None),
                    _ => Err("错".repeat(5000)),
                }
            },
        );
        let items: Vec<_> = events
            .iter()
            .filter_map(|e| {
                if let Event::Item(item) = e {
                    Some(item)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            items.iter().map(|r| r.outcome).collect::<Vec<_>>(),
            [Outcome::Done, Outcome::Skipped, Outcome::Failed]
        );
        assert!(items[2].message.len() < 4096);
        assert!(items[2].message.ends_with("…（摘要截断）"));
        assert!(matches!(events.last(), Some(Event::Finished(false))));
    }
    #[test]
    fn cancellation_keeps_completed_results_and_rejects_stale_events() {
        let mut state = State::default();
        let generation = state.begin(3);
        state.apply(
            generation,
            Event::Item(Item {
                id: "first".into(),
                outcome: Outcome::Done,
                message: "done".into(),
            }),
        );
        state.cancel(generation);
        assert!(state.busy());
        assert!(state.cancel.load(Ordering::Acquire));
        assert!(state.apply(generation, Event::Finished(true)));
        assert_eq!(state.job.phase, Phase::Cancelled);
        assert_eq!(state.rows.len(), 1);
        let next = state.begin(1);
        assert!(!state.apply(generation, Event::Finished(false)));
        assert!(state.busy());
        state.apply(
            next,
            Event::Item(Item {
                id: "second".into(),
                outcome: Outcome::Failed,
                message: "failure".into(),
            }),
        );
        assert!(state.apply(next, Event::Finished(false)));
        assert_eq!(state.job.phase, Phase::Failed);
    }
}
