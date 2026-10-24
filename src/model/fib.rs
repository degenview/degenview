//! The Fibonacci drawings: which exist, how many anchors each takes, which controls each offers, and
//! the arithmetic of their levels.
//!
//! **Levels are worked from stored prices and bar indexes, never from pixels**, so a retracement is
//! the same on a Regular, Percent, Indexed to 100 or Logarithmic axis and after any zoom. The
//! tools that TradingView builds in screen space (circles, arcs, the spiral, the wedge) are worked
//! in the projection (`ui::fib_geometry`), and only their ratios live here. Pure: no egui types.

use super::drawing_tool::Tool;
use super::gann::GannKind;
use super::pattern::format_ratio;
use serde::{Deserialize, Serialize};

/// Which Fibonacci drawing a [`Geometry::Fib`](super::drawings::Geometry::Fib) is. Its anchors are
/// in placement order and there are exactly [`FibKind::points`] of them.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FibKind {
    /// Horizontal levels between two prices.
    Retracement,
    /// Horizontal levels projecting a first move from the end of its retracement.
    Extension,
    /// Parallel lines at ratios of a channel's width.
    Channel,
    /// Vertical lines at Fibonacci multiples of the time between two anchors.
    TimeZone,
    /// Vertical lines at ratios of the time between two anchors, from a third.
    TrendTime,
    /// Rays from the first anchor through ratios of the box the two anchors make.
    SpeedFan,
    /// Ellipses about the middle of the two anchors, at ratios of the way to either.
    Circles,
    /// The golden spiral through the second anchor, about the first.
    Spiral,
    /// Arcs about the second anchor at ratios of its distance to the first.
    SpeedArcs,
    /// Arcs about an apex between two arms, at ratios of the first arm's length.
    Wedge,
    /// Rays from the first anchor through ratios of the span between the other two.
    Pitchfan,
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
    /// The name of a grid or a set of angle lines with a look of its own; empty for none.
    pub grid: &'static str,
    pub full_circles: bool,
    pub counterclockwise: bool,
    pub readout: bool,
    pub scale: bool,
    /// A level's ratio is printed in the labels (the Levels section of the Labels page).
    pub labels: bool,
    /// What names a level in its row of the level editor, for each list.
    pub captions: [Caption; 3],
    /// Arcs that can be centred on either corner.
    pub corners: bool,
    /// The name of a second background (a box's vertical one, a square's arcs'); empty for none.
    pub second_fill: &'static str,
    /// Titles of the level lists the tool has, in order; an empty title is no list.
    pub lists: [&'static str; 3],
}

/// What names a level in the level editor beside its ratio.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Caption {
    /// Nothing: the ratio is the name.
    #[default]
    None,
    /// A Gann angle's: `8x1`.
    Angle,
    /// A Gann arc's: `1.5x0`.
    Arc,
}

impl Features {
    /// A tool with none of the optional controls.
    pub const NONE: Self = Self {
        trend_line: false,
        extend: false,
        reverse: false,
        log_levels: false,
        prices: false,
        percent: false,
        align: false,
        sides: false,
        grid: "",
        full_circles: false,
        counterclockwise: false,
        readout: false,
        scale: false,
        labels: false,
        captions: [Caption::None; 3],
        corners: false,
        second_fill: "",
        lists: ["", "", ""],
    };
}

impl FibKind {
    #[cfg(test)]
    pub const ALL: [Self; 11] = [
        Self::Retracement,
        Self::Extension,
        Self::Channel,
        Self::TimeZone,
        Self::SpeedFan,
        Self::TrendTime,
        Self::Circles,
        Self::Spiral,
        Self::SpeedArcs,
        Self::Wedge,
        Self::Pitchfan,
    ];

    pub const fn tool(self) -> Tool {
        match self {
            Self::Retracement => Tool::FibRetracement,
            Self::Extension => Tool::FibExtension,
            Self::Channel => Tool::FibChannel,
            Self::TimeZone => Tool::FibTimeZone,
            Self::TrendTime => Tool::FibTrendTime,
            Self::SpeedFan => Tool::FibSpeedFan,
            Self::Circles => Tool::FibCircles,
            Self::Spiral => Tool::FibSpiral,
            Self::SpeedArcs => Tool::FibSpeedArcs,
            Self::Wedge => Tool::FibWedge,
            Self::Pitchfan => Tool::Pitchfan,
        }
    }

    pub const fn of_tool(tool: Tool) -> Option<Self> {
        match tool {
            Tool::FibRetracement => Some(Self::Retracement),
            Tool::FibExtension => Some(Self::Extension),
            Tool::FibChannel => Some(Self::Channel),
            Tool::FibTimeZone => Some(Self::TimeZone),
            Tool::FibTrendTime => Some(Self::TrendTime),
            Tool::FibSpeedFan => Some(Self::SpeedFan),
            Tool::FibCircles => Some(Self::Circles),
            Tool::FibSpiral => Some(Self::Spiral),
            Tool::FibSpeedArcs => Some(Self::SpeedArcs),
            Tool::FibWedge => Some(Self::Wedge),
            Tool::Pitchfan => Some(Self::Pitchfan),
            _ => None,
        }
    }

    /// How many anchors the tool takes.
    pub const fn points(self) -> usize {
        match self {
            Self::Retracement
            | Self::TimeZone
            | Self::SpeedFan
            | Self::Circles
            | Self::Spiral
            | Self::SpeedArcs => 2,
            Self::Extension | Self::Channel | Self::TrendTime | Self::Wedge | Self::Pitchfan => 3,
        }
    }

    pub const fn features(self) -> Features {
        let none = Features::NONE;
        match self {
            Self::Retracement => Features {
                trend_line: true,
                extend: true,
                reverse: true,
                log_levels: true,
                prices: true,
                percent: true,
                align: true,
                labels: true,
                lists: ["Levels", "", ""],
                ..none
            },
            Self::Extension => Features {
                trend_line: true,
                extend: true,
                reverse: true,
                log_levels: true,
                prices: true,
                percent: true,
                align: true,
                labels: true,
                lists: ["Levels", "", ""],
                ..none
            },
            Self::Channel => Features {
                extend: true,
                prices: true,
                percent: true,
                align: true,
                labels: true,
                lists: ["Levels", "", ""],
                ..none
            },
            Self::TimeZone | Self::TrendTime => Features {
                trend_line: true,
                align: true,
                labels: true,
                lists: ["Levels", "", ""],
                ..none
            },
            Self::SpeedFan => Features {
                reverse: true,
                sides: true,
                grid: "Grid",
                labels: true,
                lists: ["Price levels", "Time levels", ""],
                ..none
            },
            Self::Circles | Self::Wedge => Features {
                trend_line: true,
                percent: true,
                labels: true,
                lists: ["Levels", "", ""],
                ..none
            },
            Self::Spiral => Features {
                counterclockwise: true,
                ..none
            },
            Self::SpeedArcs => Features {
                trend_line: true,
                percent: true,
                full_circles: true,
                labels: true,
                lists: ["Levels", "", ""],
                ..none
            },
            Self::Pitchfan => Features {
                lists: ["Levels", "", ""],
                ..none
            },
        }
    }
}

/// The controls of `tool`, or `None` when it is not a Fibonacci or Gann tool.
pub fn features_of(tool: Tool) -> Option<Features> {
    FibKind::of_tool(tool)
        .map(FibKind::features)
        .or_else(|| GannKind::of_tool(tool).map(GannKind::features))
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

/// The price of an extension level: 0 sits on the third anchor and 1 is a move as long as the
/// first two anchors' from it, so level r is `p3 + r * (p2 - p1)`; with `log`, in the logarithm of
/// the prices (the same percentage move), which needs all three above zero.
pub fn extension_price(p1: f64, p2: f64, p3: f64, ratio: f64, log: bool) -> Option<f64> {
    if !(p1.is_finite() && p2.is_finite() && p3.is_finite() && ratio.is_finite()) {
        return None;
    }
    let price = if log && p1 > 0.0 && p2 > 0.0 && p3 > 0.0 {
        (p3.ln() + (p2.ln() - p1.ln()) * ratio).exp()
    } else {
        p3 + (p2 - p1) * ratio
    };
    price.is_finite().then_some(price)
}

/// The bar index of a time line: `origin + ratio * interval`, all in the chart's bar index (so a
/// weekend with no bars is no distance). `None` for anything that is not a finite number.
pub fn time_line(origin: f64, interval: f64, ratio: f64) -> Option<f64> {
    let index = origin + interval * ratio;
    (origin.is_finite() && interval.is_finite() && ratio.is_finite() && index.is_finite())
        .then_some(index)
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
    fn an_extension_projects_the_first_move_from_the_third_anchor() {
        // 100 to 150, back to 130: 1.0 is another 50 up from 130, 0.618 is 30.9 above it.
        assert_eq!(
            extension_price(100.0, 150.0, 130.0, 0.0, false),
            Some(130.0)
        );
        assert_eq!(
            extension_price(100.0, 150.0, 130.0, 1.0, false),
            Some(180.0)
        );
        let p = extension_price(100.0, 150.0, 130.0, 0.618, false).unwrap();
        assert!((p - 160.9).abs() < 1e-9);
        // A falling first move projects downward, and a negative ratio the other way.
        assert_eq!(extension_price(150.0, 100.0, 120.0, 1.0, false), Some(70.0));
        assert_eq!(
            extension_price(100.0, 150.0, 130.0, -1.0, false),
            Some(80.0)
        );
        // Swapping the first two anchors is the extension turned upside down.
        assert_eq!(extension_price(150.0, 100.0, 130.0, 1.0, false), Some(80.0));
    }

    #[test]
    fn a_log_extension_repeats_the_first_moves_percentage() {
        // 100 to 200 doubles; from 150 the 1.0 level doubles again, and 0.5 is a rise of 41.4 %.
        let p = extension_price(100.0, 200.0, 150.0, 1.0, true).unwrap();
        assert!((p - 300.0).abs() < 1e-9);
        let half = extension_price(100.0, 200.0, 150.0, 0.5, true).unwrap();
        assert!((half - 150.0 * 2f64.sqrt()).abs() < 1e-9);
        // A price at or below zero cannot be a ratio, so the difference is used.
        assert_eq!(extension_price(-10.0, 10.0, 0.0, 1.0, true), Some(20.0));
        assert_eq!(extension_price(f64::NAN, 1.0, 1.0, 1.0, false), None);
    }

    #[test]
    fn a_time_line_is_counted_in_bars_from_its_origin() {
        assert_eq!(time_line(10.0, 4.0, 3.0), Some(22.0));
        assert_eq!(time_line(10.0, -4.0, 2.0), Some(2.0));
        assert_eq!(time_line(10.0, 4.0, 0.0), Some(10.0));
        assert_eq!(time_line(10.0, f64::INFINITY, 1.0), None);
        assert_eq!(time_line(f64::NAN, 4.0, 1.0), None);
        assert_eq!(time_line(1e308, 1e308, 10.0), None);
    }

    #[test]
    fn every_kind_has_the_levels_its_tool_draws_and_only_the_controls_it_honours() {
        for kind in FibKind::ALL {
            let features = kind.features();
            // Every tool but the spiral has levels to edit.
            assert_eq!(
                features.lists[0].is_empty(),
                kind == FibKind::Spiral,
                "{kind:?}"
            );
            // A tool that cannot show prices or percentages on its lines offers neither.
            let on_the_price_axis = matches!(
                kind,
                FibKind::Retracement | FibKind::Extension | FibKind::Channel
            );
            assert_eq!(features.prices, on_the_price_axis, "{kind:?}");
            assert_eq!(features.extend, on_the_price_axis, "{kind:?}");
        }
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
