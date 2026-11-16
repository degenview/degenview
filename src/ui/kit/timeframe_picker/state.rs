//! What the field holds and which timeframe it points at. Pure: no drawing.

use crate::model::{Timeframe, TimeframeParseError};
use eframe::egui::Pos2;

/// The field's text, read as a timeframe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Typed {
    Empty,
    Valid(Timeframe),
    Invalid(TimeframeParseError),
}

impl Typed {
    pub(super) fn read(query: &str) -> Self {
        if query.trim().is_empty() {
            return Self::Empty;
        }
        match Timeframe::parse(query) {
            Ok(timeframe) => Self::Valid(timeframe),
            Err(error) => Self::Invalid(error),
        }
    }
}

/// What the open picker remembers between frames.
#[derive(Clone, Debug, Default)]
pub(super) struct State {
    pub(super) query: String,
    /// A chip the arrow keys moved to; typing takes the choice back to what the text says.
    pub(super) moved: Option<Timeframe>,
    /// The field has been given the keyboard once; after that it keeps it or loses it as the
    /// person decides.
    pub(super) focused_once: bool,
    /// The pinned chip whose Unpin menu is open, and where the menu was asked for.
    pub(super) unpin: Option<(Timeframe, Pos2)>,
}

impl State {
    /// The timeframe Return would chart: where the arrows left off, else what is typed, else the
    /// current one. Nothing while the text is no timeframe.
    pub(super) fn highlighted(&self, current: Timeframe) -> Option<Timeframe> {
        if let Some(moved) = self.moved {
            return Some(moved);
        }
        match Typed::read(&self.query) {
            Typed::Empty => Some(current),
            Typed::Valid(timeframe) => Some(timeframe),
            Typed::Invalid(_) => None,
        }
    }
}
