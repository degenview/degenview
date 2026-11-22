//! Waiting and trying again while somebody still wants the feed.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// How often a wait looks at the shutdown flag.
const SLICE: Duration = Duration::from_millis(250);

/// Waits `span` in slices; `false` when `shutdown` was set meanwhile.
fn sleep_unless_stopped(shutdown: &AtomicBool, span: Duration) -> bool {
    let mut waited = Duration::ZERO;
    while waited < span {
        if shutdown.load(Ordering::Acquire) {
            return false;
        }
        std::thread::sleep(SLICE);
        waited += SLICE;
    }
    !shutdown.load(Ordering::Acquire)
}

/// Calls `attempt` until it answers, waiting `every` between tries; `None` when `shutdown` came
/// first. For the lookups a feed needs before it can start (a market list, a tick size), which
/// fail when the network does.
pub(super) fn retry_until_shutdown<T>(
    shutdown: &AtomicBool,
    every: Duration,
    mut attempt: impl FnMut() -> Option<T>,
) -> Option<T> {
    loop {
        if shutdown.load(Ordering::Acquire) {
            return None;
        }
        if let Some(answer) = attempt() {
            return Some(answer);
        }
        if !sleep_unless_stopped(shutdown, every) {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wait_ends_early_when_the_stream_is_stopped() {
        let stopped = AtomicBool::new(true);
        assert!(!sleep_unless_stopped(&stopped, Duration::from_secs(60)));
        let running = AtomicBool::new(false);
        assert!(sleep_unless_stopped(&running, Duration::from_millis(10)));
    }

    #[test]
    fn a_lookup_is_tried_again_until_it_answers() {
        let running = AtomicBool::new(false);
        let mut calls = 0;
        let answer = retry_until_shutdown(&running, Duration::from_millis(10), || {
            calls += 1;
            (calls == 3).then_some("done")
        });
        assert_eq!(answer, Some("done"));
        assert_eq!(calls, 3);
    }

    #[test]
    fn a_lookup_gives_up_when_the_stream_is_stopped() {
        let stopped = AtomicBool::new(true);
        let mut calls = 0;
        let answer: Option<()> = retry_until_shutdown(&stopped, Duration::from_millis(10), || {
            calls += 1;
            None
        });
        assert_eq!(answer, None);
        assert_eq!(calls, 0);

        let running = AtomicBool::new(false);
        let answer: Option<()> = retry_until_shutdown(&running, Duration::from_millis(10), || {
            running.store(true, Ordering::Release);
            None
        });
        assert_eq!(answer, None, "stopped while it waited");
    }
}
