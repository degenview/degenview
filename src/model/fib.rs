//! The Fibonacci drawings: which exist, how many anchors each takes, which controls each offers, and
//! the arithmetic of their levels.
//!
//! **Levels are worked from stored prices and bar indexes, never from pixels**, so a retracement is
//! the same on a Regular, Percent, Indexed to 100 or Logarithmic axis and after any zoom. The
//! tools that TradingView builds in screen space (circles, arcs, the spiral, the wedge) are worked
//! in the projection (`ui::fib_geometry`), and only their ratios live here. Pure: no egui types.

use super::drawing_tool::Tool;
use super::pattern::format_ratio;
use serde::{Deserialize, Serialize};

/// Which Fibonacci drawing a [`Geometry::Fib`](super::drawings::Geometry::Fib) is. Its anchors are
/// in placement order and there are exactly [`FibKind::points`] of them.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FibKind {
    /// Horizontal levels between two prices.
    Retracement,
}

/// The controls a Fibonacci or Gann tool really has. A control a tool lacks is neither in its
/// dialog nor in its toolbar, so nothing offered is inert.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Features {
    /// The line joining the anchors, which can be hidden.
    pub trend_line: bool,
    /// Level lines that run on to the plot's edges.
    pub extend: bool,
    pub reverse: bool,
    /// Levels placed by the ratio of the prices on a log chart.
    pub log_levels: bool,
    /// A level's price next to its ratio.
    pub prices: bool,
    /// Ratios as percentages.
    pub percent: bool,
    /// Where along the line, and on which side, the label sits.
    pub align: bool,
    /// Labels on the four sides of a fan or box.
    pub sides: bool,
    pub grid: bool,
    pub full_circles: bool,
    pub counterclockwise: bool,
    pub angles: bool,
    pub readout: bool,
    pub scale: bool,
    /// Arcs that can be centred on either corner.
    pub corners: bool,
    /// The name of a second background (a box's vertical one, a square's arcs'); empty for none.
    pub second_fill: &'static str,
    /// Titles of the level lists the tool has, in order; an empty title is no list.
    pub lists: [&'static str; 3],
}

impl FibKind {
    #[cfg(test)]
    pub const ALL: [Self; 1] = [Self::Retracement];

    pub const fn tool(self) -> Tool {
        match self {
            Self::Retracement => Tool::FibRetracement,
        }
    }

    pub const fn of_tool(tool: Tool) -> Option<Self> {
        match tool {
            Tool::FibRetracement => Some(Self::Retracement),
            _ => None,
        }
    }

    /// How many anchors the tool takes.
    pub const fn points(self) -> usize {
        match self {
            Self::Retracement => 2,
        }
    }

    pub const fn features(self) -> Features {
        let none = Features {
            trend_line: false,
            extend: false,
            reverse: false,
            log_levels: false,
            prices: false,
            percent: false,
            align: false,
            sides: false,
            grid: false,
            full_circles: false,
            counterclockwise: false,
            angles: false,
            readout: false,
            scale: false,
            corners: false,
            second_fill: "",
            lists: ["", "", ""],
        };
        match self {
            Self::Retracement => Features {
                trend_line: true,
                extend: true,
                reverse: true,
                log_levels: true,
                prices: true,
                percent: true,
                align: true,
                lists: ["Levels", "", ""],
                ..none
            },
        }
    }
}

/// The controls of `tool`, or `None` when it is not a Fibonacci or Gann tool.
pub fn features_of(tool: Tool) -> Option<Features> {
    FibKind::of_tool(tool).map(FibKind::features)
}

/// The price of a retracement level: 0 sits on the end, 1 on the start, so that dragging from a
/// low to a high puts the 0.618 retracement 61.8 % of the way back down. With `log` the ratio
/// divides the move of the *logarithm* of the price (a level halfway down a doubling is the
/// geometric mean), which is TradingView's "Fib levels based on log scale"; it needs both prices
/// above zero and otherwise falls back to the plain difference. `None` for a ratio or a price
/// that is not finite.
pub fn retracement_price(start: f64, end: f64, ratio: f64, log: bool) -> Option<f64> {
    interpolate(end, start, ratio, log)
}

/// `from + (to - from) * ratio`, in the logarithm of the prices when `log` and they allow it.
fn interpolate(from: f64, to: f64, ratio: f64, log: bool) -> Option<f64> {
    if !(from.is_finite() && to.is_finite() && ratio.is_finite()) {
        return None;
    }
    let price = if log && from > 0.0 && to > 0.0 {
        (from.ln() + (to.ln() - from.ln()) * ratio).exp()
    } else {
        from + (to - from) * ratio
    };
    price.is_finite().then_some(price)
}

/// A level's ratio as printed: `0.618`, or `61.8%` when `percent`.
pub fn level_label(ratio: f64, percent: bool) -> String {
    if percent {
        format!("{}%", format_ratio(ratio * 100.0, 1))
    } else {
        format_ratio(ratio, 3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_is_one_tool_with_its_anchors() {
        for kind in FibKind::ALL {
            assert_eq!(FibKind::of_tool(kind.tool()), Some(kind));
            assert!(kind.points() >= 2);
        }
    }

    #[test]
    fn a_retracement_puts_zero_on_the_end_and_one_on_the_start() {
        // A rise from 100 to 200: the 0.5 retracement is 150, the 0.618 is 138.2.
        assert_eq!(retracement_price(100.0, 200.0, 0.0, false), Some(200.0));
        assert_eq!(retracement_price(100.0, 200.0, 1.0, false), Some(100.0));
        assert_eq!(retracement_price(100.0, 200.0, 0.5, false), Some(150.0));
        let p = retracement_price(100.0, 200.0, 0.618, false).unwrap();
        assert!((p - 138.2).abs() < 1e-9);
        // Falling from 200 to 100 mirrors it.
        let q = retracement_price(200.0, 100.0, 0.618, false).unwrap();
        assert!((q - 161.8).abs() < 1e-9);
        // Extensions run past either end.
        assert_eq!(retracement_price(100.0, 200.0, -1.0, false), Some(300.0));
        assert_eq!(retracement_price(100.0, 200.0, 2.0, false), Some(0.0));
    }

    #[test]
    fn a_log_retracement_is_the_geometric_mean_halfway() {
        let half = retracement_price(100.0, 400.0, 0.5, true).unwrap();
        assert!((half - 200.0).abs() < 1e-9, "{half}");
        let plain = retracement_price(100.0, 400.0, 0.5, false).unwrap();
        assert_eq!(plain, 250.0);
        // Levels at 0 and 1 are the anchors either way.
        assert!((retracement_price(100.0, 400.0, 0.0, true).unwrap() - 400.0).abs() < 1e-9);
        assert!((retracement_price(100.0, 400.0, 1.0, true).unwrap() - 100.0).abs() < 1e-9);
    }

    #[test]
    fn a_log_level_needs_positive_prices_and_otherwise_uses_the_difference() {
        assert_eq!(retracement_price(-100.0, 100.0, 0.5, true), Some(0.0));
        assert_eq!(retracement_price(0.0, 100.0, 0.5, true), Some(50.0));
    }

    #[test]
    fn a_level_that_is_not_a_number_has_no_price() {
        assert_eq!(retracement_price(f64::NAN, 1.0, 0.5, false), None);
        assert_eq!(retracement_price(1.0, 2.0, f64::INFINITY, false), None);
        assert_eq!(retracement_price(f64::MAX, -f64::MAX, 1e300, false), None);
    }

    #[test]
    fn a_ratio_prints_as_a_value_or_a_percentage() {
        assert_eq!(level_label(0.618, false), "0.618");
        assert_eq!(level_label(1.0, false), "1");
        assert_eq!(level_label(0.5, true), "50%");
        assert_eq!(level_label(0.236, true), "23.6%");
        assert_eq!(level_label(4.236, true), "423.6%");
        assert_eq!(level_label(-0.5, false), "-0.5");
    }
}
