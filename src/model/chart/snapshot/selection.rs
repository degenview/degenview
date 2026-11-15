//! The rectangle the user drags over a chart to say which portion to capture (Select portion).
//!
//! Everything is in points, in the coordinates of the area the drag may cover, so a HiDPI screen
//! changes nothing here: pixels appear only when the composition is fitted to an image.

use eframe::egui::{Pos2, Rect};

/// The shortest side, in points, of a selection that counts. A smaller one is a click or a slip
/// of the hand, and the selection is cancelled.
pub const MIN_SIDE: f32 = 24.0;

/// The rectangle between the press and the pointer, dragged in any direction and kept inside
/// `area`: what is drawn while the drag is under way, however small. A corner that is not a
/// number counts as the area's top-left.
pub fn dragged(from: Pos2, to: Pos2, area: Rect) -> Rect {
    let inside = |point: Pos2| {
        if point.x.is_finite() && point.y.is_finite() {
            point.clamp(area.min, area.max)
        } else {
            area.min
        }
    };
    Rect::from_two_pos(inside(from), inside(to))
}

/// The selection the release makes: [`dragged`], unless it is under [`MIN_SIDE`] on a side (or
/// `area` is empty), which cancels it.
pub fn released(from: Pos2, to: Pos2, area: Rect) -> Option<Rect> {
    if !area.is_positive() {
        return None;
    }
    let rect = dragged(from, to, area);
    (rect.width() >= MIN_SIDE && rect.height() >= MIN_SIDE).then_some(rect)
}
