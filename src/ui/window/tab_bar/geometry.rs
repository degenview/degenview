//! Where the strip's tabs go: their width, scroll and slots, where a dragged tab sits and lands,
//! the close button's place and when a drag tears a tab off. Pure functions, so the host shares
//! them while it drives a drag.

use super::{
    CLOSE_SIZE, HEIGHT, MAX_TAB_WIDTH, MIN_TAB_WIDTH, NEW_BUTTON_WIDTH, SIDE_PADDING, TAB_GAP,
};
use eframe::egui::{Pos2, Rect, Vec2, pos2};

/// Width of each tab: they share the room left of the `+`, between the min and max.
pub fn tab_width(count: usize, available: f32) -> f32 {
    if count == 0 {
        return MAX_TAB_WIDTH;
    }
    let room = (available - NEW_BUTTON_WIDTH - SIDE_PADDING * 2.0).max(0.0);
    let gaps = TAB_GAP * (count - 1) as f32;
    ((room - gaps) / count as f32).clamp(MIN_TAB_WIDTH, MAX_TAB_WIDTH)
}

/// How far the tabs run past the strip: zero while they all fit, else how far the row can be
/// scrolled so that its last tab and the `+` end at the strip's right edge.
pub fn max_scroll(count: usize, strip_width: f32) -> f32 {
    let width = tab_width(count, strip_width);
    (tab_left(count, width) + NEW_BUTTON_WIDTH - strip_width).max(0.0)
}

/// The scroll that shows tab `index` whole (with the strip's padding beside it) when it is not,
/// else `scroll` as it is.
pub fn reveal(index: usize, count: usize, strip_width: f32, scroll: f32) -> f32 {
    let width = tab_width(count, strip_width);
    let room = strip_width - NEW_BUTTON_WIDTH;
    let left = tab_left(index, width);
    let right = left + width;
    let target = if left - SIDE_PADDING < scroll {
        left - SIDE_PADDING
    } else if right + SIDE_PADDING > scroll + room {
        right + SIDE_PADDING - room
    } else {
        scroll
    };
    target.clamp(0.0, max_scroll(count, strip_width))
}

/// Left edge of tab `index`, relative to the strip's left edge.
pub fn tab_left(index: usize, width: f32) -> f32 {
    SIDE_PADDING + index as f32 * (width + TAB_GAP)
}

/// The slot under `x` (relative to the strip's left edge), for reordering an existing tab.
pub fn slot_at(x: f32, count: usize, width: f32) -> usize {
    if count == 0 {
        return 0;
    }
    let slot = ((x - SIDE_PADDING) / (width + TAB_GAP)).floor();
    (slot.max(0.0) as usize).min(count - 1)
}

/// The slot tab `index` occupies when a tab dragged in from elsewhere holds a gap at `gap`.
pub fn slot_of(index: usize, gap: Option<usize>) -> usize {
    match gap {
        Some(gap) if index >= gap => index + 1,
        _ => index,
    }
}

/// Left edge (relative to the strip) of a tab of `count` dragged by the pointer, kept between
/// the first and last slot. `grab_x` is where in the tab the pointer took hold of it.
pub fn dragged_left(pointer_x: f32, grab_x: f32, count: usize, width: f32) -> f32 {
    (pointer_x - grab_x).clamp(tab_left(0, width), tab_left(count.saturating_sub(1), width))
}

/// The slot a dragged tab sits over: the one under its centre, so it swaps with a neighbour once
/// it is half across.
pub fn dragged_slot(left: f32, count: usize, width: f32) -> usize {
    slot_at(left + width / 2.0, count, width)
}

/// The gap nearest `x`, for dropping a tab from elsewhere: 0 is before the first tab and
/// `count` after the last.
pub fn insertion_at(x: f32, count: usize, width: f32) -> usize {
    let gap = ((x - SIDE_PADDING) / (width + TAB_GAP)).round();
    (gap.max(0.0) as usize).min(count)
}

pub(super) fn close_rect(tab: Rect) -> Rect {
    Rect::from_center_size(
        pos2(tab.right() - 6.0 - CLOSE_SIZE / 2.0, tab.center().y + 1.0),
        Vec2::splat(CLOSE_SIZE),
    )
}

/// Where the pointer is relative to the strip, for deciding between reordering and tearing off.
pub fn is_outside_tear_off_zone(pointer: Pos2, strip: Rect) -> bool {
    pointer.y < strip.top() - HEIGHT
        || pointer.y > strip.bottom() + HEIGHT
        || pointer.x < strip.left() - HEIGHT
        || pointer.x > strip.right() + HEIGHT
}
