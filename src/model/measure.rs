//! The three measurers (Price range, Date range, Date and price range): what each one reads off
//! the two anchors it is placed with.
//!
//! Prices are the anchors' real prices, never pixels or an axis label, so a measurement is the
//! same on a linear, logarithmic, percent or indexed axis. Bars and time are counted by the
//! caller from the bars the chart draws (they depend on the chart type and the replay cut). Pure:
//! no egui types.

use super::drawing_tool::Tool;
use super::kline::Kline;
use serde::{Deserialize, Serialize};

/// Which measurer a [`Geometry::Measure`](super::drawings::Geometry::Measure) is.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasureKind {
    /// Two prices: a shaded box between them across the width of the two anchors.
    Price,
    /// Two times: a shaded box between them across the height of the plot.
    Date,
    /// Both: a shaded rectangle between the two anchors.
    DateAndPrice,
}

impl MeasureKind {
    #[cfg(test)]
    pub const ALL: [Self; 3] = [Self::Price, Self::Date, Self::DateAndPrice];

    pub const fn tool(self) -> Tool {
        match self {
            Self::Price => Tool::PriceRange,
            Self::Date => Tool::DateRange,
            Self::DateAndPrice => Tool::DateAndPriceRange,
        }
    }

    pub const fn of_tool(tool: Tool) -> Option<Self> {
        match tool {
            Tool::PriceRange => Some(Self::Price),
            Tool::DateRange => Some(Self::Date),
            Tool::DateAndPriceRange => Some(Self::DateAndPrice),
            _ => None,
        }
    }

    /// Whether the measurement has a price part.
    pub const fn has_price(self) -> bool {
        !matches!(self, Self::Date)
    }

    /// Whether the measurement has a time part.
    pub const fn has_time(self) -> bool {
        !matches!(self, Self::Price)
    }
}

/// The percentage change from `from` to `to`, or `None` when `from` is nothing to measure against
/// (zero or not a number).
pub fn percent_change(from: f64, to: f64) -> Option<f64> {
    if !(from.is_finite() && to.is_finite()) || from == 0.0 {
        return None;
    }
    let change = (to - from) / from.abs() * 100.0;
    change.is_finite().then_some(change)
}

/// How many `tick`s there are between two prices, or `None` for a tick that is not a size.
pub fn ticks_between(from: f64, to: f64, tick: f64) -> Option<f64> {
    if !(from.is_finite() && to.is_finite() && tick.is_finite()) || tick <= 0.0 {
        return None;
    }
    let ticks = (to - from) / tick;
    ticks.is_finite().then_some(ticks)
}

/// The volume of the bars from index `first` to index `second` (fractional indexes: each is
/// rounded to the bar it falls on, and either may be the earlier), both included, or `None` when
/// there are no bars there or they traded nothing (a market with no volume, a gap). The bars are
/// the ones the chart draws.
pub fn volume_over(bars: &[Kline], first: f64, second: f64) -> Option<f64> {
    if bars.is_empty() || !(first.is_finite() && second.is_finite()) {
        return None;
    }
    let last = bars.len() - 1;
    let clamp = |index: f64| index.round().clamp(0.0, last as f64) as usize;
    let (low, high) = (clamp(first.min(second)), clamp(first.max(second)));
    let total: f64 = bars[low..=high].iter().map(|bar| bar.volume).sum();
    (total.is_finite() && total > 0.0).then_some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(volume: f64) -> Kline {
        Kline {
            open_time: 0,
            close_time: 0,
            open: 1.0,
            high: 2.0,
            low: 0.5,
            close: 1.5,
            volume,
        }
    }

    #[test]
    fn every_kind_is_one_tool_and_says_what_it_measures() {
        for kind in MeasureKind::ALL {
            assert_eq!(MeasureKind::of_tool(kind.tool()), Some(kind));
        }
        assert!(MeasureKind::Price.has_price() && !MeasureKind::Price.has_time());
        assert!(!MeasureKind::Date.has_price() && MeasureKind::Date.has_time());
        assert!(MeasureKind::DateAndPrice.has_price() && MeasureKind::DateAndPrice.has_time());
    }

    #[test]
    fn a_percentage_is_the_change_over_the_start() {
        assert_eq!(percent_change(100.0, 110.0), Some(10.0));
        assert_eq!(percent_change(100.0, 90.0), Some(-10.0));
        // The same move, measured the other way, is not the same percentage.
        let back = percent_change(110.0, 100.0).unwrap();
        assert!((back - (-100.0 / 11.0)).abs() < 1e-12);
        // A fall to nothing is -100 %, and a rise from a negative price keeps its direction.
        assert_eq!(percent_change(50.0, 0.0), Some(-100.0));
        assert_eq!(percent_change(-10.0, -5.0), Some(50.0));
    }

    #[test]
    fn a_percentage_of_nothing_is_nothing() {
        assert_eq!(percent_change(0.0, 5.0), None);
        assert_eq!(percent_change(f64::NAN, 5.0), None);
        assert_eq!(percent_change(5.0, f64::INFINITY), None);
        // Tiny starts do not overflow into infinity.
        assert_eq!(percent_change(f64::MIN_POSITIVE, f64::MAX), None);
    }

    #[test]
    fn ticks_count_the_steps_between_two_prices() {
        assert_eq!(ticks_between(100.0, 101.5, 0.25), Some(6.0));
        assert_eq!(ticks_between(101.5, 100.0, 0.25), Some(-6.0));
        assert_eq!(ticks_between(1.0, 2.0, 0.0), None);
        assert_eq!(ticks_between(1.0, 2.0, -0.5), None);
        assert_eq!(ticks_between(f64::NAN, 2.0, 1.0), None);
    }

    #[test]
    fn volume_is_summed_over_the_bars_both_anchors_fall_on() {
        let bars = [bar(1.0), bar(2.0), bar(4.0), bar(8.0), bar(16.0)];
        // Bars 1 to 3.
        assert_eq!(volume_over(&bars, 1.0, 3.0), Some(14.0));
        // The other way round is the same bars.
        assert_eq!(volume_over(&bars, 3.0, 1.0), Some(14.0));
        // Between bars rounds to the nearest: 0.6 is bar 1 and 2.4 is bar 2.
        assert_eq!(volume_over(&bars, 0.6, 2.4), Some(6.0));
        // One bar is one bar.
        assert_eq!(volume_over(&bars, 2.0, 2.0), Some(4.0));
    }

    #[test]
    fn volume_beyond_the_data_is_the_data_there_is() {
        let bars = [bar(1.0), bar(2.0), bar(4.0)];
        // Anchors in the past and the future of the chart: all three bars.
        assert_eq!(volume_over(&bars, -50.0, 90.0), Some(7.0));
        // Wholly in the future: the last bar (the index is clamped, as the drawing is).
        assert_eq!(volume_over(&bars, 10.0, 12.0), Some(4.0));
    }

    #[test]
    fn a_market_with_no_volume_has_none_to_report() {
        let bars = [bar(0.0), bar(0.0), bar(0.0)];
        assert_eq!(volume_over(&bars, 0.0, 2.0), None);
        assert_eq!(volume_over(&[], 0.0, 2.0), None);
        assert_eq!(volume_over(&[bar(1.0)], f64::NAN, 2.0), None);
        let broken = [bar(f64::NAN), bar(1.0)];
        assert_eq!(volume_over(&broken, 0.0, 1.0), None);
    }
}
