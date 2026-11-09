//! A hand-off between whoever produces reports and the thread that writes them to a link.
//!
//! A link carries only so many reports a second. While the writer waits for it, a newer
//! pointer position replaces an older one, so the host always gets the latest position instead
//! of a growing backlog. Keys, buttons, wheel and relative motion are never dropped.
use crate::Report;
use std::{
    collections::VecDeque,
    sync::{Condvar, Mutex, MutexGuard},
    time::Duration,
};

/// More reports than this waiting means the link has stalled; the backlog is discarded.
pub const MAX_QUEUED: usize = 1024;

#[derive(Debug, PartialEq)]
pub enum Taken {
    Reports(Vec<Report>),
    Timeout,
    Closed,
}

#[derive(Default)]
struct State {
    reports: VecDeque<Report>,
    closed: bool,
}

#[derive(Default)]
pub struct ReportQueue {
    state: Mutex<State>,
    wake: Condvar,
}

impl ReportQueue {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Queues `report`, or replaces the newest queued report it supersedes. Returns false when
    /// the queue was full and its backlog was discarded to make room.
    pub fn push(&self, report: Report) -> bool {
        let mut state = self.lock();
        let mut kept = true;
        match state.reports.back_mut() {
            Some(last) if report.supersedes(last) => *last = report,
            _ => {
                if state.reports.len() >= MAX_QUEUED {
                    state.reports.clear();
                    kept = false;
                }
                state.reports.push_back(report);
            }
        }
        drop(state);
        self.wake.notify_one();
        kept
    }

    /// Waits up to `timeout` for reports and takes all of them.
    pub fn take(&self, timeout: Duration) -> Taken {
        let state = self.lock();
        let (mut state, _) = self
            .wake
            .wait_timeout_while(state, timeout, |s| s.reports.is_empty() && !s.closed)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.closed {
            Taken::Closed
        } else if state.reports.is_empty() {
            Taken::Timeout
        } else {
            Taken::Reports(state.reports.drain(..).collect())
        }
    }

    /// Drops everything queued, e.g. when the link goes down: stale input must not replay.
    pub fn clear(&self) {
        self.lock().reports.clear();
    }

    /// Wakes the writer so it can stop.
    pub fn close(&self) {
        self.lock().closed = true;
        self.wake.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(id: u8, byte: u8, position_only: bool) -> Report {
        Report {
            id,
            data: vec![byte],
            position_only,
        }
    }

    #[test]
    fn only_the_newest_position_waits() {
        let queue = ReportQueue::new();
        queue.push(report(2, 1, true));
        queue.push(report(2, 2, true));
        queue.push(report(3, 0, false)); // a button: kept, and positions never jump it
        queue.push(report(2, 3, true));
        queue.push(report(2, 4, true));
        assert_eq!(
            queue.take(Duration::ZERO),
            Taken::Reports(vec![
                report(2, 2, true),
                report(3, 0, false),
                report(2, 4, true)
            ])
        );
        assert_eq!(queue.take(Duration::ZERO), Taken::Timeout);
    }

    #[test]
    fn positions_of_different_pointers_do_not_replace_each_other() {
        let queue = ReportQueue::new();
        queue.push(report(2, 1, true));
        queue.push(report(4, 1, true));
        let Taken::Reports(reports) = queue.take(Duration::ZERO) else {
            panic!()
        };
        assert_eq!(reports.len(), 2);
    }

    #[test]
    fn a_stalled_link_discards_its_backlog() {
        let queue = ReportQueue::new();
        for i in 0..MAX_QUEUED {
            assert!(queue.push(report(1, i as u8, false)));
        }
        assert!(!queue.push(report(1, 9, false)));
        assert_eq!(
            queue.take(Duration::ZERO),
            Taken::Reports(vec![report(1, 9, false)])
        );
    }

    #[test]
    fn closing_wakes_the_writer() {
        let queue = std::sync::Arc::new(ReportQueue::new());
        let waiter = {
            let queue = queue.clone();
            std::thread::spawn(move || queue.take(Duration::from_secs(10)))
        };
        std::thread::sleep(Duration::from_millis(20));
        queue.close();
        assert_eq!(waiter.join().unwrap(), Taken::Closed);
    }
}
