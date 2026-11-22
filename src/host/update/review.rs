//! Which window shows the review sheet of an available update, and what that sheet has drawn.
//!
//! The sheet exists only while an update is on offer: `observe` drops it once the phase leaves
//! `Available`, and the host calls that where the phase changes, so asking whether a window shows it
//! (`is_open_in`) never changes anything.

use super::Phase;
use eframe::egui;

/// The open sheet: the window that shows it, and the version it drew on its previous frame.
struct Open {
    viewport: egui::ViewportId,
    shown: Option<String>,
}

/// The review sheet, if one is open. Only one window shows it at a time.
#[derive(Default)]
pub(super) struct ReviewState {
    open: Option<Open>,
}

impl ReviewState {
    /// Opens the sheet in `viewport`, closing it in any other window.
    pub(super) fn open(&mut self, viewport: egui::ViewportId) {
        self.open = Some(Open {
            viewport,
            shown: None,
        });
    }

    pub(super) fn close(&mut self) {
        self.open = None;
    }

    /// Closes the sheet when `viewport` is the window showing it; one shown elsewhere is left
    /// alone.
    pub(super) fn close_in(&mut self, viewport: egui::ViewportId) {
        if self.is_open_in(viewport) {
            self.open = None;
        }
    }

    /// Ends the sheet unless an update is on offer in `phase`: the choice is dropped, so a later
    /// update does not open it unasked.
    pub(super) fn observe(&mut self, phase: &Phase) {
        if !matches!(phase, Phase::Available { .. }) {
            self.open = None;
        }
    }

    pub(super) fn is_open_in(&self, viewport: egui::ViewportId) -> bool {
        self.open
            .as_ref()
            .is_some_and(|open| open.viewport == viewport)
    }

    /// Records that the sheet draws `version` this frame and tells whether it drew that same
    /// version on its previous frame; false when no sheet is open.
    pub(super) fn note_shown(&mut self, version: &str) -> bool {
        let Some(open) = self.open.as_mut() else {
            return false;
        };
        let seen = open.shown.as_deref() == Some(version);
        if !seen {
            open.shown = Some(version.to_owned());
        }
        seen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::changelog::{Coverage, ReleaseNotes};
    use std::sync::Arc;

    const MAIN: egui::ViewportId = egui::ViewportId::ROOT;

    fn other() -> egui::ViewportId {
        egui::ViewportId::from_hash_of("other")
    }

    fn available() -> Phase {
        Phase::Available {
            version: "9.9.9".to_owned(),
            url: "https://example.test/release".to_owned(),
            installable: true,
            note: None,
            notes: Arc::new(ReleaseNotes::default()),
            coverage: Coverage::default(),
        }
    }

    #[test]
    fn a_sheet_shows_in_one_window_and_asking_changes_nothing() {
        let mut review = ReviewState::default();
        assert!(!review.is_open_in(MAIN));
        review.open(MAIN);
        review.open(other());
        assert!(!review.is_open_in(MAIN));
        assert!(review.is_open_in(other()));
        assert!(
            review.is_open_in(other()),
            "asking twice gives the same answer"
        );
        review.close();
        assert!(!review.is_open_in(other()));
    }

    #[test]
    fn closing_in_a_window_ends_only_the_sheet_it_shows() {
        let mut review = ReviewState::default();
        review.open(MAIN);
        review.close_in(other());
        assert!(review.is_open_in(MAIN));
        review.close_in(MAIN);
        assert!(!review.is_open_in(MAIN));
    }

    #[test]
    fn the_sheet_ends_when_no_update_is_on_offer_and_stays_while_one_is() {
        let mut review = ReviewState::default();
        review.open(MAIN);
        review.observe(&available());
        assert!(review.is_open_in(MAIN));
        for phase in [Phase::UpToDate, Phase::Checking, Phase::Unchecked] {
            review.open(MAIN);
            review.observe(&phase);
            assert!(!review.is_open_in(MAIN), "{phase:?}");
        }
    }

    #[test]
    fn a_version_counts_as_seen_from_the_second_frame_it_is_drawn() {
        let mut review = ReviewState::default();
        assert!(!review.note_shown("9.9.9"), "no sheet is open");
        review.open(MAIN);
        assert!(!review.note_shown("9.9.9"), "its first frame");
        assert!(review.note_shown("9.9.9"));
        assert!(
            !review.note_shown("9.9.10"),
            "a different release starts over"
        );
        assert!(review.note_shown("9.9.10"));
        review.open(MAIN);
        assert!(!review.note_shown("9.9.10"), "a reopened sheet starts over");
    }
}
