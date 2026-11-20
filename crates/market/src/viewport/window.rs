//! Which bars a chart shows and where they sit in the plot's slots.

/// Which bars the plot shows and where they sit in its slots.
///
/// The plot always has `visible + reserved` equally wide slots. `reserved` are the empty slots
/// the right margin (or an overlay script) keeps past the newest bar; scrolled back by
/// `right_offset` bars they fill with the bars that follow, so the bar spacing never changes as
/// the chart scrolls. The offset is continuous: between two whole bars the first and last shown
/// bar are cut by the plot's edge (the painter clips them) and `lead` carries the fraction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Window {
    /// The first shown bar and one past the last, as indices into the series.
    pub start: usize,
    pub end: usize,
    /// Slots between the plot's left edge and the first shown bar's own slot: positive for the
    /// empty slots before a series shorter than the window, negative when the first bar is cut
    /// by the edge.
    pub lead: f32,
    /// All the slots of the plot.
    pub slots: usize,
}

impl Window {
    /// How many bars are shown.
    pub fn count(&self) -> usize {
        self.end - self.start
    }
}

/// An offset a float sum left a hair off a whole bar is that whole bar.
fn snapped(value: f64) -> f64 {
    (value * 1e6).round() / 1e6
}

/// The bars of a `len`-bar series that `visible` slots plus `reserved` empty ones show, scrolled
/// back by `right_offset` bars.
pub fn visible_window(len: usize, visible: usize, right_offset: f64, reserved: usize) -> Window {
    let slots = visible.saturating_add(reserved).max(1);
    let offset = if right_offset.is_finite() {
        right_offset
    } else {
        0.0
    };
    // The slot edge the plot's right side lies on, in bars from the first.
    let edge = snapped((len as f64) + reserved as f64 - offset).max(0.0);
    let first = snapped(edge - slots as f64);
    let start = (first.floor().max(0.0) as usize).min(len);
    let end = (edge.ceil() as usize).min(len).max(start);
    Window {
        start,
        end,
        lead: (start as f64 - first) as f32,
        slots,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_without_a_margin_is_the_last_visible_bars() {
        let window = visible_window(500, 120, 0.0, 0);
        assert_eq!((window.start, window.end), (380, 500));
        assert_eq!((window.lead, window.slots), (0.0, 120));
        assert_eq!(window.count(), 120);
    }

    #[test]
    fn a_right_margin_reserves_empty_slots_past_the_newest_bar() {
        let window = visible_window(500, 120, 0.0, 10);
        assert_eq!((window.start, window.end), (380, 500));
        assert_eq!(window.slots, 130, "the margin is part of the plot");
        // The newest bar sits ten slots in from the right edge.
        let newest_slot = window.lead as usize + window.count() - 1;
        assert_eq!(window.slots - 1 - newest_slot, 10);
    }

    #[test]
    fn scrolling_back_fills_the_margin_with_later_bars_at_the_same_spacing() {
        let at_rest = visible_window(500, 120, 0.0, 10);
        let scrolled = visible_window(500, 120, 4.0, 10);
        assert_eq!(scrolled.slots, at_rest.slots, "bar spacing does not change");
        assert_eq!(scrolled.end, 500, "still inside the margin");
        assert_eq!(scrolled.start, 376);
        let far = visible_window(500, 120, 30.0, 10);
        assert_eq!((far.start, far.end), (350, 480));
        assert_eq!(far.slots, 130);
        assert_eq!(far.lead, 0.0);
        assert_eq!(far.count(), 130, "the margin's slots show real bars now");
    }

    #[test]
    fn a_short_series_sits_at_the_right_of_the_window() {
        let window = visible_window(30, 120, 0.0, 0);
        assert_eq!((window.start, window.end), (0, 30));
        assert_eq!(window.lead, 90.0);
        let with_margin = visible_window(30, 120, 0.0, 10);
        assert_eq!(
            with_margin.lead, 90.0,
            "the margin is at the right, not the left"
        );
        assert_eq!(with_margin.slots, 130);
    }

    #[test]
    fn an_empty_or_overscrolled_series_has_an_empty_window() {
        assert_eq!(visible_window(0, 120, 0.0, 10).count(), 0);
        assert_eq!(visible_window(10, 120, 500.0, 10).count(), 0);
        let window = visible_window(10, 0, 0.0, 0);
        assert_eq!(window.slots, 1);
    }

    #[test]
    fn a_fractional_offset_cuts_the_bar_at_each_edge() {
        let window = visible_window(500, 120, 0.25, 0);
        // 119.75 bars are whole; the first bar's slot lies a quarter outside the plot.
        assert_eq!((window.start, window.end), (379, 500));
        assert!((window.lead + 0.75).abs() < 1e-6, "{}", window.lead);
        let other = visible_window(500, 120, 3.5, 0);
        assert_eq!((other.start, other.end), (376, 497));
        assert!((other.lead + 0.5).abs() < 1e-6, "{}", other.lead);
    }

    #[test]
    fn a_whole_offset_cuts_no_bar() {
        let window = visible_window(500, 120, 7.0000000001, 0);
        assert_eq!((window.start, window.end), (373, 493));
        assert_eq!(window.lead, 0.0);
    }

    #[test]
    fn a_negative_offset_leaves_empty_slots_after_the_newest_bar() {
        let window = visible_window(500, 120, -10.0, 0);
        assert_eq!((window.start, window.end), (390, 500));
        assert_eq!(window.lead, 0.0);
        assert_eq!(window.slots, 120, "the spacing does not change");
        // The newest bar's slot is 11 from the right edge: ten empty slots follow it.
        let newest_slot = window.lead + (window.count() - 1) as f32;
        assert_eq!(window.slots as f32 - 1.0 - newest_slot, 10.0);
    }

    #[test]
    fn a_fractional_whitespace_offset_shows_the_newest_bar_between_slots() {
        let window = visible_window(500, 120, -2.5, 0);
        assert_eq!(window.end, 500);
        let newest_slot = window.lead + (window.count() - 1) as f32;
        assert!((window.slots as f32 - 1.0 - newest_slot - 2.5).abs() < 1e-5);
    }

    #[test]
    fn a_non_finite_offset_is_the_newest_bars() {
        assert_eq!(
            visible_window(500, 120, f64::NAN, 0),
            visible_window(500, 120, 0.0, 0)
        );
    }
}
