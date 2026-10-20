use chrono::{DateTime, Local, Utc};
use std::time::Instant;

/// One application-loop time sample shared by clocks and calendar reminders.
#[derive(Clone, Copy)]
pub(super) struct TickSnapshot {
    pub(super) monotonic: Instant,
    pub(super) utc: DateTime<Utc>,
}

impl TickSnapshot {
    pub(super) fn capture() -> Self {
        Self {
            monotonic: Instant::now(),
            utc: Utc::now(),
        }
    }

    pub(super) fn local(self) -> DateTime<Local> {
        self.utc.with_timezone(&Local)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_calendar_time_uses_the_same_captured_instant_as_utc_clock_time() {
        let tick = TickSnapshot::capture();
        assert_eq!(tick.utc, tick.local().with_timezone(&Utc));
    }
}
