//! A settings page's connection test: what was tried, and what it said about it.

use crate::service::work_queue::Pending;
use crate::ui::settings::card::StatusMessage;

/// A test answers for the input it was given and for nothing else. It remembers that input, shows
/// its answer only while the page still holds it, and is forgotten, running or answered, once the
/// page holds something else.
pub struct ConnectionTest<S, R> {
    attempt: Option<(S, Attempt<R>)>,
}

enum Attempt<R> {
    Running(Pending<R>),
    Answered(StatusMessage),
}

impl<S, R> Default for ConnectionTest<S, R> {
    fn default() -> Self {
        Self { attempt: None }
    }
}

impl<S, R> ConnectionTest<S, R> {
    pub fn is_running(&self) -> bool {
        matches!(self.attempt, Some((_, Attempt::Running(_))))
    }

    /// Drops the test: for a change the subject does not show, such as another saved key.
    pub fn forget(&mut self) {
        self.attempt = None;
    }

    /// Launches a test of `subject`, unless one is running.
    pub fn start(&mut self, subject: S, launch: impl FnOnce() -> Pending<R>) {
        if !self.is_running() {
            self.attempt = Some((subject, Attempt::Running(launch())));
        }
    }
}

impl<S: PartialEq, R> ConnectionTest<S, R> {
    /// Run every frame with what the page holds: forgets a test of anything else and takes a
    /// finished answer, worded by `say` (`None`: the job ended without one).
    pub fn follow(&mut self, current: &S, say: impl FnOnce(Option<R>) -> StatusMessage) {
        if self
            .attempt
            .as_ref()
            .is_some_and(|(subject, _)| subject != current)
        {
            self.forget();
        }
        let Some((_, attempt)) = &mut self.attempt else {
            return;
        };
        let Attempt::Running(pending) = attempt else {
            return;
        };
        let outcome = pending.poll();
        if outcome.is_some() || !pending.is_pending() {
            *attempt = Attempt::Answered(say(outcome));
        }
    }

    /// What the test said, if it was about `current`.
    pub fn answer(&self, current: &S) -> Option<&StatusMessage> {
        match &self.attempt {
            Some((subject, Attempt::Answered(status))) if subject == current => Some(status),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::work_queue;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    type Test = ConnectionTest<&'static str, bool>;

    fn say(outcome: Option<bool>) -> StatusMessage {
        match outcome {
            Some(true) => StatusMessage::ok("passed"),
            Some(false) => StatusMessage::error("refused"),
            None => StatusMessage::error("lost"),
        }
    }

    fn finish(test: &mut Test, current: &'static str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while test.is_running() {
            assert!(Instant::now() < deadline, "the test never finished");
            test.follow(&current, say);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn an_answer_is_shown_only_for_the_input_that_was_tested() {
        let mut test = Test::default();
        test.start("live", || work_queue::SNAPSHOTS.start(None, || true));
        finish(&mut test, "live");
        assert_eq!(test.answer(&"live"), Some(&StatusMessage::ok("passed")));
        assert_eq!(test.answer(&"practice"), None);
        test.follow(&"practice", say);
        assert_eq!(test.answer(&"live"), None, "forgotten, not kept for later");
    }

    #[test]
    fn a_test_still_running_when_the_input_changes_never_answers() {
        let mut test = Test::default();
        let (release, wait) = mpsc::channel::<()>();
        test.start("live", || {
            work_queue::SNAPSHOTS.start(None, move || wait.recv().is_ok())
        });
        test.follow(&"practice", say);
        assert!(!test.is_running(), "the new input can be tested at once");
        release.send(()).ok();
        test.follow(&"practice", say);
        assert_eq!(test.answer(&"practice"), None);
        assert_eq!(test.answer(&"live"), None);
    }

    #[test]
    fn a_job_that_ends_without_an_answer_says_so() {
        let mut test = Test::default();
        test.start("live", || {
            work_queue::SNAPSHOTS.start(None, || panic!("the test job fails"))
        });
        finish(&mut test, "live");
        assert_eq!(test.answer(&"live"), Some(&StatusMessage::error("lost")));
    }
}
