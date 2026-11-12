//! When to ask a venue what an order would do, and whether its answer may still be acted on.
//!
//! The sheet asks once what is typed is complete and has stopped changing for a moment, asks again
//! when the answer is too old, and never lets an order be reviewed on an answer that is missing,
//! failed, refused it, or is stale. (A venue's order endpoint may not require a preview; this is
//! this application's rule, so a real order is never sent on a guess.)

use std::time::{Duration, Instant};

use super::draft::PreviewResult;
use super::task::Task;
use degenview_brokers::model::preview::{PREVIEW_FRESH, PreviewAnswer, PreviewGate, preview_gate};

type Answer = Result<PreviewResult, String>;

#[derive(Default)]
pub struct PreviewSlot {
    key: String,
    due: Option<Instant>,
    task: Option<Task<Answer>>,
    answer: Option<(Answer, Instant)>,
}

impl PreviewSlot {
    /// Called every frame. `complete` is whether the draft can be asked about; `key` identifies
    /// exactly what would be asked (the venue, the market and what is typed). `ask` starts the
    /// question. Returns whether something is waiting on the venue, so the caller keeps repainting.
    pub fn sync(
        &mut self,
        key: &str,
        complete: bool,
        now: Instant,
        debounce: Duration,
        ask: impl FnOnce() -> Task<Answer>,
    ) -> bool {
        if !complete {
            *self = Self::default();
            return false;
        }
        if key != self.key {
            *self = Self {
                key: key.to_owned(),
                due: Some(now + debounce),
                ..Self::default()
            };
        }
        if let Some(task) = &mut self.task
            && let Some(answer) = task.poll()
        {
            self.answer = Some((answer.clone(), now));
            self.task = None;
        }
        // An accepted answer that has aged is asked for again.
        if let Some((Ok(preview), at)) = &self.answer
            && preview.accepted
            && now.duration_since(*at) > PREVIEW_FRESH
            && self.task.is_none()
            && self.due.is_none()
        {
            self.due = Some(now);
        }
        if self.task.is_none() && self.due.is_some_and(|due| now >= due) {
            self.task = Some(ask());
            self.due = None;
        }
        self.task.is_some() || self.due.is_some()
    }

    /// Asks again at once ("Try again").
    pub fn retry(&mut self, now: Instant) {
        self.task = None;
        self.answer = None;
        self.due = Some(now);
    }

    pub fn result(&self) -> Option<&PreviewResult> {
        match &self.answer {
            Some((Ok(preview), _)) => Some(preview),
            _ => None,
        }
    }

    pub fn gate(&self, now: Instant) -> PreviewGate {
        match &self.answer {
            None => PreviewGate::Waiting,
            Some((Err(text), _)) => preview_gate(PreviewAnswer::Failed(text.clone()), None),
            Some((Ok(preview), at)) if preview.accepted => {
                preview_gate(PreviewAnswer::Accepted, Some(now.duration_since(*at)))
            }
            Some((Ok(_), _)) => preview_gate(PreviewAnswer::Problems, None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accepted() -> Answer {
        Ok(PreviewResult {
            accepted: true,
            ..PreviewResult::default()
        })
    }

    const DEBOUNCE: Duration = Duration::from_millis(400);

    #[test]
    fn nothing_is_asked_until_the_order_is_complete_and_has_settled() {
        let t0 = Instant::now();
        let mut slot = PreviewSlot::default();
        let mut asked = 0;
        // Incomplete: never asked, and a gate that waits.
        assert!(!slot.sync("a", false, t0, DEBOUNCE, || unreachable!()));
        assert_eq!(slot.gate(t0), PreviewGate::Waiting);
        // Complete, but still within the debounce.
        assert!(slot.sync("a", true, t0, DEBOUNCE, || {
            asked += 1;
            Task::ready(accepted())
        }));
        assert_eq!(asked, 0);
        // After it: asked once, answered at once, and not asked again.
        slot.sync("a", true, t0 + DEBOUNCE, DEBOUNCE, || {
            asked += 1;
            Task::ready(accepted())
        });
        slot.sync("a", true, t0 + DEBOUNCE * 2, DEBOUNCE, || {
            asked += 1;
            Task::ready(accepted())
        });
        assert_eq!(asked, 1);
        assert_eq!(slot.gate(t0 + DEBOUNCE * 2), PreviewGate::Ready);
    }

    #[test]
    fn changing_what_is_typed_makes_the_old_answer_about_another_order() {
        let t0 = Instant::now();
        let mut slot = PreviewSlot::default();
        slot.sync("a", true, t0, Duration::ZERO, || Task::ready(accepted()));
        slot.sync("a", true, t0, Duration::ZERO, || unreachable!());
        assert_eq!(slot.gate(t0), PreviewGate::Ready);
        slot.sync("b", true, t0, DEBOUNCE, || unreachable!());
        assert_eq!(
            slot.gate(t0),
            PreviewGate::Waiting,
            "never reviewed on the answer for a"
        );
        assert!(slot.result().is_none());
    }

    #[test]
    fn an_answer_goes_stale_and_is_asked_for_again() {
        let t0 = Instant::now();
        let mut slot = PreviewSlot::default();
        slot.sync("a", true, t0, Duration::ZERO, || Task::ready(accepted()));
        slot.sync("a", true, t0, Duration::ZERO, || unreachable!());
        let late = t0 + PREVIEW_FRESH + Duration::from_secs(1);
        let mut asked = 0;
        slot.sync("a", true, late, DEBOUNCE, || {
            asked += 1;
            Task::new(|| None)
        });
        assert_eq!(asked, 1, "asked again");
        assert_eq!(
            slot.gate(late),
            PreviewGate::Stale,
            "and not reviewable meanwhile"
        );
    }

    #[test]
    fn a_refusal_and_a_failure_block_review_and_a_retry_asks_again() {
        let t0 = Instant::now();
        let mut slot = PreviewSlot::default();
        slot.sync("a", true, t0, Duration::ZERO, || {
            Task::ready(Ok(PreviewResult {
                accepted: false,
                problems: vec!["Insufficient funds".to_owned()],
                ..PreviewResult::default()
            }))
        });
        slot.sync("a", true, t0, Duration::ZERO, || unreachable!());
        assert_eq!(slot.gate(t0), PreviewGate::Blocked);
        assert_eq!(slot.result().unwrap().problems, vec!["Insufficient funds"]);
        slot.retry(t0);
        slot.sync("a", true, t0, Duration::ZERO, || {
            Task::ready(Err("The connection dropped".to_owned()))
        });
        slot.sync("a", true, t0, Duration::ZERO, || unreachable!());
        assert_eq!(
            slot.gate(t0),
            PreviewGate::Failed("The connection dropped".to_owned())
        );
        assert!(!slot.gate(t0).allows_review());
    }
}
