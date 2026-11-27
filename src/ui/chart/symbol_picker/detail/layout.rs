//! Where everything in the preview sits. The renderer only draws into these rects, so what the
//! tests measure is what is on screen.
//!
//! A market's name, price and button always show; the chart takes what it can between a readable
//! minimum and its full height, and the other markets of the asset only get room when at least two
//! of them fit.

use eframe::egui::{Rect, pos2, vec2};

/// From the panel's edges to its content.
pub const PAD: f32 = 20.0;
const BUTTON_HEIGHT: f32 = 34.0;
/// From the button to the panel's foot, and from the content above to the button.
const BUTTON_FOOT: f32 = 16.0;
const BUTTON_GAP: f32 = 10.0;
const CHART_MAX: f32 = 112.0;
const CHART_MIN: f32 = 48.0;
/// Below this height the panel tightens up and says whether it trades and where on one line.
const COMPACT_BELOW: f32 = 360.0;
/// The second of the two status lines, from the top of the first.
const VENUE_LINE: f32 = 21.0;
/// "Also trades as": from the chart to its hairline, from the hairline to its caption, the
/// caption, and one row.
pub const RELATED_RULE: f32 = 12.0;
pub const RELATED_CAPTION_GAP: f32 = 10.0;
pub const RELATED_CAPTION: f32 = 22.0;
pub const RELATED_ROW: f32 = 32.0;
const RELATED_MIN: f32 = RELATED_RULE + RELATED_CAPTION_GAP + RELATED_CAPTION + 2.0 * RELATED_ROW;

/// The button at the foot of the panel.
pub fn button(panel: Rect) -> Rect {
    let bottom = panel.bottom() - BUTTON_FOOT;
    Rect::from_min_max(
        pos2(panel.left() + PAD, bottom - BUTTON_HEIGHT),
        pos2(panel.right() - PAD, bottom),
    )
}

/// The parts of a crypto or stock market's preview, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Market {
    pub header: Rect,
    pub price: Rect,
    /// Whether it trades now; also where, when there is no `venue` line.
    pub status: Rect,
    pub venue: Option<Rect>,
    pub chart: Rect,
    /// The room for the other markets of the asset, when there is enough.
    pub related: Option<Rect>,
}

/// Shares `panel` out for a market with or without a button and other markets to list.
pub fn market(panel: Rect, has_button: bool, has_related: bool) -> Market {
    let compact = panel.height() < COMPACT_BELOW;
    let (pad, header, price, lines, gap) = if compact {
        (14.0, 36.0, 36.0, 18.0, 8.0)
    } else {
        (20.0, 40.0, 40.0, 40.0, 14.0)
    };
    let bottom = if has_button {
        button(panel).top() - BUTTON_GAP
    } else {
        panel.bottom() - pad
    };
    let mut y = panel.top() + pad;
    let mut take = |height: f32, after: f32| {
        let rect = Rect::from_min_size(
            pos2(panel.left() + PAD, y),
            vec2(panel.width() - 2.0 * PAD, height),
        );
        y += height + after;
        rect
    };
    let header = take(header, gap);
    let price = take(price, gap / 2.0);
    let status = take(lines, gap);
    let free = bottom - status.bottom() - gap;
    let (chart, related) = if has_related && free >= CHART_MIN + 16.0 + RELATED_MIN {
        let chart = (free - RELATED_MIN).min(CHART_MAX);
        (chart, Some(free - chart))
    } else {
        (free.clamp(CHART_MIN, CHART_MAX), None)
    };
    let chart = take(chart, 0.0);
    let related = related.map(|height| take(height, 0.0));
    Market {
        header,
        price,
        status,
        venue: (!compact).then(|| status.with_min_y(status.top() + VENUE_LINE)),
        chart,
        related,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel(height: f32) -> Rect {
        Rect::from_min_size(pos2(600.0, 100.0), vec2(320.0, height))
    }

    /// The parts in reading order, each below the one before and all clear of the button.
    fn assert_stacked(panel: Rect, fit: &Market) {
        let mut parts = vec![fit.header, fit.price, fit.status, fit.chart];
        parts.extend(fit.related);
        for pair in parts.windows(2) {
            assert!(pair[0].bottom() <= pair[1].top(), "{pair:?}");
        }
        let last = parts[parts.len() - 1];
        assert!(last.bottom() <= button(panel).top() - BUTTON_GAP, "{fit:?}");
        assert!(parts.iter().all(|part| panel.contains_rect(*part)));
    }

    #[test]
    fn the_full_sheet_has_a_full_chart_and_other_markets() {
        let panel = panel(506.0);
        let fit = market(panel, true, true);
        assert_stacked(panel, &fit);
        assert_eq!(fit.chart.height(), CHART_MAX);
        assert!(fit.related.is_some_and(|rect| rect.height() >= RELATED_MIN));
        assert!(fit.venue.is_some());
    }

    #[test]
    fn a_shorter_sheet_gives_up_the_other_markets_before_the_chart() {
        let panel = panel(380.0);
        let fit = market(panel, true, true);
        assert_stacked(panel, &fit);
        assert_eq!(fit.related, None);
        assert!(fit.chart.height() >= CHART_MIN);
    }

    #[test]
    fn the_shortest_sheet_is_compact_and_still_charts() {
        let panel = panel(246.0);
        let fit = market(panel, true, true);
        assert_stacked(panel, &fit);
        assert_eq!((fit.venue, fit.related), (None, None));
        assert!(fit.chart.height() >= CHART_MIN);
    }
}
