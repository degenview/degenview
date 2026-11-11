//! The editor's find bar: Find (⌘F / Ctrl+F) opens it above the code, Find Next / Previous step
//! through the matches, Esc closes it. Stands in for the native `NSTextFinder` bar Swift's editor uses; matching is
//! plain-text and case-insensitive, like its default.

use crate::ui::icons::{self, Icon};
use crate::ui::shortcut;
use crate::ui::theme;
use eframe::egui::{
    self, Align, Id, Key, Layout, Margin, Rect, RichText, Stroke, Ui, UiBuilder, Vec2,
};
use std::ops::Range;

pub const BAR_HEIGHT: f32 = 34.0;

#[derive(Default)]
pub struct FindState {
    pub open: bool,
    pub query: String,
    /// Index into the current matches.
    pub current: usize,
    pub needs_focus: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub enum FindEvent {
    None,
    /// The query text changed: select the first match.
    QueryChanged,
    Next,
    Previous,
    Close,
}

/// Every non-overlapping, case-insensitive occurrence of `query` in `text`, as char ranges.
pub fn matches(text: &str, query: &str) -> Vec<Range<usize>> {
    // Lowercase one char to one char, so positions stay char indexes of the original text.
    let hay: Vec<char> = text
        .chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect();
    let needle: Vec<char> = query
        .chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        if hay[i..i + needle.len()] == needle[..] {
            found.push(i..i + needle.len());
            i += needle.len();
        } else {
            i += 1;
        }
    }
    found
}

/// The next (or previous) match index with wraparound; `None` when there are no matches.
pub fn step(current: usize, count: usize, forward: bool) -> Option<usize> {
    if count == 0 {
        None
    } else if forward {
        Some((current + 1) % count)
    } else {
        Some((current + count - 1) % count)
    }
}

/// The first match at or after `caret`, wrapping to the first.
pub fn nearest(matches: &[Range<usize>], caret: usize) -> usize {
    matches.iter().position(|m| m.start >= caret).unwrap_or(0)
}

/// Draws the bar in `rect` and reports what the user did.
pub fn show(ui: &mut Ui, rect: Rect, state: &mut FindState, count: usize) -> FindEvent {
    ui.painter().rect_filled(rect, 0.0, theme::chrome());
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, theme::separator()),
    );
    let mut event = FindEvent::None;
    let mut inner = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(12.0, 5.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    inner.spacing_mut().item_spacing.x = 6.0;
    icons::inline(&mut inner, Icon::Magnifier, 13.0, theme::text_secondary());
    let id = Id::new("script-find-field");
    let field = inner.add(
        egui::TextEdit::singleline(&mut state.query)
            .id(id)
            .hint_text("Find")
            .desired_width(220.0)
            .margin(Margin::symmetric(6, 3)),
    );
    if state.needs_focus {
        field.request_focus();
        state.needs_focus = false;
    }
    if field.changed() {
        event = FindEvent::QueryChanged;
    }
    if field.lost_focus() && inner.input(|i| i.key_pressed(Key::Enter)) {
        field.request_focus();
        event = if inner.input(|i| i.modifiers.shift) {
            FindEvent::Previous
        } else {
            FindEvent::Next
        };
    }
    let summary = if state.query.is_empty() {
        String::new()
    } else if count == 0 {
        "No results".to_owned()
    } else {
        format!("{} of {count}", state.current.min(count - 1) + 1)
    };
    inner.label(
        RichText::new(summary)
            .size(10.0)
            .color(theme::text_secondary()),
    );
    if icons::button(
        &mut inner,
        Icon::ChevronUp,
        24.0,
        theme::text_primary(),
        &shortcut::FIND_PREVIOUS.hint("Previous match"),
    )
    .clicked()
    {
        event = FindEvent::Previous;
    }
    if icons::button(
        &mut inner,
        Icon::ChevronDown,
        24.0,
        theme::text_primary(),
        &shortcut::FIND_NEXT.hint("Next match"),
    )
    .clicked()
    {
        event = FindEvent::Next;
    }
    inner.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if icons::button(ui, Icon::Xmark, 24.0, theme::text_secondary(), "Close").clicked() {
            event = FindEvent::Close;
        }
    });
    if inner.input(|i| i.key_pressed(Key::Escape)) {
        event = FindEvent::Close;
    }
    event
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_is_case_insensitive_and_non_overlapping() {
        assert_eq!(matches("Close close CLOSE", "close"), [0..5, 6..11, 12..17]);
        assert_eq!(matches("aaaa", "aa"), [0..2, 2..4]);
        assert!(matches("abc", "").is_empty());
        assert!(matches("abc", "z").is_empty());
        // Positions are char indexes, not bytes.
        let found = matches("日本語 foo", "foo");
        assert_eq!((found[0].start, found[0].end, found.len()), (4, 7, 1));
    }

    #[test]
    fn stepping_wraps_both_ways() {
        assert_eq!(step(0, 3, true), Some(1));
        assert_eq!(step(2, 3, true), Some(0));
        assert_eq!(step(0, 3, false), Some(2));
        assert_eq!(step(0, 0, true), None);
    }

    #[test]
    fn the_first_match_at_or_after_the_caret_is_current() {
        let found = matches("x x x", "x");
        assert_eq!(nearest(&found, 0), 0);
        assert_eq!(nearest(&found, 1), 1);
        assert_eq!(nearest(&found, 5), 0, "past the last wraps to the first");
    }
}
