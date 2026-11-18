//! The marks the Demonstration cursor leaves while someone explains a chart: dots and freehand
//! strokes that stay a moment and then fade away.
//!
//! The marks are pinned to time and price like a drawing, so a wheel zoom during the fade keeps
//! them on the bars they point at, but they are not drawings: never stored, synced, templated or
//! put in a snapshot, and a chart that changes market drops them.
//!
//! Time is the caller's clock in seconds, so the fade does not depend on the frame rate.

use crate::model::Anchor;

/// How long a finished mark stays fully visible.
pub const HOLD_SECS: f64 = 2.0;
/// How long it then takes to fade out.
pub const FADE_SECS: f64 = 1.0;
/// Most marks kept at once; the oldest go first.
pub const MAX_MARKS: usize = 48;
/// Most points in one stroke; the stroke stops growing there.
pub const MAX_POINTS: usize = 4096;

#[derive(Clone, Debug, PartialEq)]
pub enum MarkShape {
    Dot(Anchor),
    Stroke(Vec<Anchor>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub shape: MarkShape,
    /// When the mark was finished; `None` while a stroke is still being drawn.
    ended: Option<f64>,
}

#[derive(Clone, Debug, Default)]
pub struct Marks {
    marks: Vec<Mark>,
}

impl Marks {
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }

    pub fn marks(&self) -> &[Mark] {
        &self.marks
    }

    /// A stroke is being drawn: the press that began it is still down.
    pub fn is_drawing(&self) -> bool {
        self.marks.last().is_some_and(|mark| mark.ended.is_none())
    }

    /// Puts a dot, finished at once.
    pub fn dot(&mut self, at: Anchor, now: f64) {
        self.push(Mark {
            shape: MarkShape::Dot(at),
            ended: Some(now),
        });
    }

    /// Starts a stroke at `at`. A stroke left open is finished first.
    pub fn begin(&mut self, at: Anchor, now: f64) {
        self.end(now);
        self.push(Mark {
            shape: MarkShape::Stroke(vec![at]),
            ended: None,
        });
    }

    /// Adds a point to the stroke being drawn.
    pub fn extend(&mut self, at: Anchor) {
        if let Some(Mark {
            shape: MarkShape::Stroke(points),
            ended: None,
        }) = self.marks.last_mut()
            && points.len() < MAX_POINTS
            && points.last() != Some(&at)
        {
            points.push(at);
        }
    }

    /// Finishes the stroke being drawn: its fade starts now.
    pub fn end(&mut self, now: f64) {
        if let Some(mark) = self.marks.last_mut()
            && mark.ended.is_none()
        {
            mark.ended = Some(now);
        }
    }

    pub fn clear(&mut self) {
        self.marks.clear();
    }

    /// Drops the marks that have faded away.
    pub fn prune(&mut self, now: f64) {
        self.marks.retain(|mark| {
            mark.ended
                .is_none_or(|ended| now - ended < HOLD_SECS + FADE_SECS)
        });
    }

    /// How opaque `mark` is at `now`: whole while drawn and held, then fading to nothing.
    pub fn alpha(mark: &Mark, now: f64) -> f32 {
        let Some(ended) = mark.ended else {
            return 1.0;
        };
        let faded = (now - ended - HOLD_SECS) / FADE_SECS;
        (1.0 - faded).clamp(0.0, 1.0) as f32
    }

    /// Seconds until a repaint changes anything, or `None` when nothing is on screen: zero while a
    /// stroke is drawn or a mark fades, the rest of the hold while every mark is just waiting.
    pub fn next_change(&self, now: f64) -> Option<f64> {
        let mut wait: Option<f64> = None;
        for mark in &self.marks {
            let due = match mark.ended {
                None => 0.0,
                Some(ended) => (ended + HOLD_SECS - now).max(0.0),
            };
            wait = Some(wait.map_or(due, |soonest| soonest.min(due)));
        }
        wait
    }

    fn push(&mut self, mark: Mark) {
        self.marks.push(mark);
        if self.marks.len() > MAX_MARKS {
            self.marks.remove(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(time_ms: i64) -> Anchor {
        Anchor {
            time_ms,
            price: 100.0,
        }
    }

    #[test]
    fn a_stroke_is_whole_while_drawn_and_fades_from_when_it_ends() {
        let mut marks = Marks::default();
        marks.begin(at(0), 10.0);
        marks.extend(at(1));
        marks.extend(at(2));
        assert!(marks.is_drawing());
        assert_eq!(
            Marks::alpha(&marks.marks()[0], 500.0),
            1.0,
            "held while the button is down"
        );
        marks.end(20.0);
        assert!(!marks.is_drawing());
        let mark = marks.marks()[0].clone();
        assert_eq!(Marks::alpha(&mark, 20.0 + HOLD_SECS), 1.0);
        let half = Marks::alpha(&mark, 20.0 + HOLD_SECS + FADE_SECS / 2.0);
        assert!((half - 0.5).abs() < 1e-6, "{half}");
        assert_eq!(Marks::alpha(&mark, 20.0 + HOLD_SECS + FADE_SECS), 0.0);
    }

    #[test]
    fn faded_marks_are_dropped_and_the_rest_kept() {
        let mut marks = Marks::default();
        marks.dot(at(0), 0.0);
        marks.dot(at(1), 2.0);
        marks.prune(HOLD_SECS + FADE_SECS - 0.1);
        assert_eq!(marks.marks().len(), 2);
        marks.prune(HOLD_SECS + FADE_SECS);
        assert_eq!(marks.marks().len(), 1, "the first is gone");
        marks.prune(2.0 + HOLD_SECS + FADE_SECS);
        assert!(marks.is_empty());
    }

    #[test]
    fn a_stroke_still_being_drawn_is_never_pruned() {
        let mut marks = Marks::default();
        marks.begin(at(0), 0.0);
        marks.prune(1.0e6);
        assert_eq!(marks.marks().len(), 1);
    }

    #[test]
    fn a_new_stroke_finishes_one_left_open() {
        let mut marks = Marks::default();
        marks.begin(at(0), 1.0);
        marks.begin(at(5), 2.0);
        assert_eq!(marks.marks().len(), 2);
        assert_eq!(marks.marks()[0].ended, Some(2.0));
        assert!(marks.is_drawing());
    }

    #[test]
    fn extending_ignores_repeats_a_finished_stroke_and_the_point_cap() {
        let mut marks = Marks::default();
        marks.extend(at(1));
        assert!(marks.is_empty(), "nothing is being drawn");
        marks.begin(at(0), 0.0);
        marks.extend(at(0));
        assert_eq!(stroke_len(&marks), 1, "the same point twice adds nothing");
        for time in 1..(MAX_POINTS as i64 + 10) {
            marks.extend(at(time));
        }
        assert_eq!(stroke_len(&marks), MAX_POINTS);
        marks.end(1.0);
        marks.extend(at(-5));
        assert_eq!(
            stroke_len(&marks),
            MAX_POINTS,
            "a finished stroke is closed"
        );
    }

    fn stroke_len(marks: &Marks) -> usize {
        match &marks.marks()[0].shape {
            MarkShape::Stroke(points) => points.len(),
            MarkShape::Dot(_) => 1,
        }
    }

    #[test]
    fn the_oldest_marks_go_first_past_the_cap() {
        let mut marks = Marks::default();
        for time in 0..(MAX_MARKS as i64 + 5) {
            marks.dot(at(time), 0.0);
        }
        assert_eq!(marks.marks().len(), MAX_MARKS);
        assert_eq!(marks.marks()[0].shape, MarkShape::Dot(at(5)));
    }

    #[test]
    fn the_next_repaint_is_now_while_drawing_and_after_the_hold_when_idle() {
        let mut marks = Marks::default();
        assert_eq!(marks.next_change(0.0), None);
        marks.dot(at(0), 10.0);
        let wait = marks.next_change(10.5).expect("a mark is on screen");
        assert!((wait - (HOLD_SECS - 0.5)).abs() < 1e-9, "{wait}");
        assert_eq!(
            marks.next_change(10.0 + HOLD_SECS + 0.2),
            Some(0.0),
            "fading"
        );
        marks.begin(at(1), 11.0);
        assert_eq!(marks.next_change(11.0), Some(0.0));
    }
}
