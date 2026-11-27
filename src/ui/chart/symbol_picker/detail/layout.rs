//! How the preview shares its height. The market's name, price and button always show; the chart
//! takes what it can between a readable minimum and its full height, and the other markets of the
//! asset only get room when at least two of them fit.

pub const CHART_MAX: f32 = 112.0;
const CHART_MIN: f32 = 48.0;
/// Below this height the panel tightens up and says whether it trades and where on one line.
const COMPACT_BELOW: f32 = 360.0;
/// One row of the other markets of the asset.
pub const RELATED_ROW: f32 = 32.0;
/// The gap to the hairline, the gap after it, the caption and two rows.
const RELATED_MIN: f32 = 12.0 + 10.0 + 22.0 + 2.0 * RELATED_ROW;
pub const BUTTON: f32 = 34.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub compact: bool,
    pub pad: f32,
    pub header: f32,
    pub price: f32,
    /// One status line when compact, two otherwise.
    pub lines: f32,
    pub chart: f32,
    /// The room for the other markets, from the chart down to the button; zero hides them.
    pub related: f32,
}

/// Shares `height` for a market with or without a button and other markets to list.
pub fn layout(height: f32, has_button: bool, has_related: bool) -> Layout {
    let compact = height < COMPACT_BELOW;
    let (pad, header, price, lines) = if compact {
        (14.0, 36.0, 36.0, 18.0)
    } else {
        (20.0, 40.0, 40.0, 40.0)
    };
    // The gaps: header to price, price to lines, lines to chart.
    let gaps = if compact { 24.0 } else { 36.0 };
    let button = if has_button { BUTTON + 10.0 } else { 0.0 };
    let free = height - 2.0 * pad - header - price - lines - gaps - button;
    let (chart, related) = if has_related && free >= CHART_MIN + 16.0 + RELATED_MIN {
        let chart = (free - RELATED_MIN).min(CHART_MAX);
        (chart, free - chart)
    } else {
        (free.clamp(CHART_MIN, CHART_MAX), 0.0)
    };
    Layout {
        compact,
        pad,
        header,
        price,
        lines,
        chart,
        related,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_full_sheet_has_a_full_chart_and_other_markets() {
        let full = layout(506.0, true, true);
        assert_eq!(full.chart, CHART_MAX);
        assert!(full.related >= RELATED_MIN);
    }

    #[test]
    fn a_shorter_sheet_gives_up_the_other_markets_before_the_chart() {
        let shorter = layout(380.0, true, true);
        assert_eq!(shorter.related, 0.0);
        assert!(shorter.chart >= CHART_MIN);
    }

    #[test]
    fn the_shortest_sheet_is_compact_and_still_charts() {
        let short = layout(246.0, true, true);
        assert!(short.compact && short.related == 0.0);
        assert!(short.chart >= CHART_MIN);
        let used = 2.0 * short.pad
            + short.header
            + short.price
            + short.lines
            + 24.0
            + short.chart
            + BUTTON
            + 10.0;
        assert!(used <= 246.0, "{used}");
    }
}
