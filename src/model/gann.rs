//! The Gann drawings: which exist, which controls each offers, and the arithmetic of a Gann angle.
//!
//! **A Gann angle is a ratio of price to time, not a screen angle.** The 1×1 line climbs one unit
//! of price for one of time, where the user's second anchor says how much a unit of each is: it
//! passes through that anchor, whatever the chart's scales are, and 2×1 climbs twice as fast. So the
//! lines are worked from the two anchors' own positions (their price and their bar), never from a
//! fixed 45 degrees, and they move with the chart. Pure: no egui types.

use super::drawing_tool::Tool;
use super::fib::{Caption, Features};
use super::fib_style::FibLevel;
use super::pattern::format_ratio;
use serde::{Deserialize, Serialize};

/// Which Gann drawing a [`Geometry::Gann`](super::drawings::Geometry::Gann) is. Each is placed with
/// two anchors.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GannKind {
    /// A box of price levels and time levels.
    Box,
    /// A square with levels, fan lines and arcs, held square on the screen when it is placed.
    SquareFixed,
    /// The same in a box of any shape, with its ratio of price to time.
    Square,
    /// Rays from a point at ratios of price to time.
    Fan,
}

impl GannKind {
    #[cfg(test)]
    pub const ALL: [Self; 4] = [Self::Box, Self::SquareFixed, Self::Square, Self::Fan];

    pub const fn tool(self) -> Tool {
        match self {
            Self::Box => Tool::GannBox,
            Self::SquareFixed => Tool::GannSquareFixed,
            Self::Square => Tool::GannSquare,
            Self::Fan => Tool::GannFan,
        }
    }

    pub const fn of_tool(tool: Tool) -> Option<Self> {
        match tool {
            Tool::GannBox => Some(Self::Box),
            Tool::GannSquareFixed => Some(Self::SquareFixed),
            Tool::GannSquare => Some(Self::Square),
            Tool::GannFan => Some(Self::Fan),
            _ => None,
        }
    }

    /// How many anchors the tool takes.
    pub const fn points(self) -> usize {
        2
    }

    pub const fn features(self) -> Features {
        let none = Features::NONE;
        match self {
            Self::Box => Features {
                reverse: true,
                sides: true,
                grid: "Angles",
                labels: true,
                second_fill: "Vertical background",
                lists: ["Price levels", "Time levels", ""],
                ..none
            },
            Self::Fan => Features {
                labels: true,
                lists: ["Angles", "", ""],
                captions: [Caption::Angle, Caption::None, Caption::None],
                ..none
            },
            Self::Square => Features {
                reverse: true,
                readout: true,
                scale: true,
                labels: true,
                second_fill: "Arcs background",
                lists: ["Levels", "Fan lines", "Arcs"],
                captions: [Caption::None, Caption::Angle, Caption::Arc],
                corners: true,
                ..none
            },
            Self::SquareFixed => Features {
                reverse: true,
                second_fill: "Arcs background",
                lists: ["Levels", "Fan lines", "Arcs"],
                captions: [Caption::None, Caption::Angle, Caption::Arc],
                corners: true,
                ..none
            },
        }
    }
}

/// An angle's name: price over time, as `8x1` for eight units of price to one of time and `1x8`
/// for one to eight. A ratio that is not a whole number either way is written out (`2.5x1`).
pub fn angle_label(ratio: f32) -> String {
    let ratio = f64::from(ratio);
    if !ratio.is_finite() || ratio <= 0.0 {
        return String::new();
    }
    if ratio >= 1.0 {
        format!("{}x1", format_ratio(ratio, 2))
    } else {
        format!("1x{}", format_ratio(1.0 / ratio, 2))
    }
}

/// An arc's name: its radius in sides of the square, and `0` for the arcs about the first corner
/// or `1` for those about the far one (`1.5x0`).
pub fn arc_label(level: &FibLevel) -> String {
    format!(
        "{}x{}",
        format_ratio(f64::from(level.ratio), 2),
        u8::from(level.alt)
    )
}

/// The price per bar between two anchors: the scale of a Gann box, how many units of price are
/// one bar of time. `None` for no time between them or a price that is not a number.
pub fn price_per_bar(from_price: f64, to_price: f64, bars: f64) -> Option<f64> {
    let rate = (to_price - from_price).abs() / bars.abs();
    (rate.is_finite() && bars != 0.0).then_some(rate)
}

/// The price of the far corner of a Gann square whose price per bar is `scale`: as many bars
/// across as the anchors are apart, and `bars × scale` of price up (or down, as the second anchor
/// is below the first). `None` for a scale or a distance that is not a number.
pub fn scaled_price(from_price: f64, to_price: f64, bars: f64, scale: f64) -> Option<f64> {
    let price = from_price + (to_price - from_price).signum() * bars.abs() * scale;
    (price.is_finite() && scale.is_finite() && scale >= 0.0).then_some(price)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_is_one_tool_with_two_anchors() {
        for kind in GannKind::ALL {
            assert_eq!(GannKind::of_tool(kind.tool()), Some(kind));
            assert_eq!(kind.points(), 2);
            assert!(!kind.features().lists[0].is_empty());
        }
        assert_eq!(GannKind::of_tool(Tool::FibChannel), None);
    }

    #[test]
    fn an_angle_is_named_price_over_time() {
        assert_eq!(angle_label(1.0), "1x1");
        assert_eq!(angle_label(8.0), "8x1");
        assert_eq!(angle_label(2.0), "2x1");
        assert_eq!(angle_label(0.5), "1x2");
        assert_eq!(angle_label(1.0 / 3.0), "1x3");
        assert_eq!(angle_label(0.125), "1x8");
        assert_eq!(angle_label(2.5), "2.5x1");
        assert_eq!(angle_label(0.4), "1x2.5");
        assert_eq!(angle_label(0.0), "");
        assert_eq!(angle_label(-1.0), "");
        assert_eq!(angle_label(f32::NAN), "");
    }

    #[test]
    fn an_arc_is_named_by_its_size_and_corner() {
        let arc = |ratio: f32, alt: bool| FibLevel {
            ratio,
            alt,
            ..FibLevel::default()
        };
        assert_eq!(arc_label(&arc(1.0, false)), "1x0");
        assert_eq!(arc_label(&arc(1.5, false)), "1.5x0");
        assert_eq!(arc_label(&arc(5.0, true)), "5x1");
    }

    #[test]
    fn the_scale_is_price_per_bar_between_the_anchors() {
        // 30 of price over 6 bars is 5 a bar, whichever way round.
        assert_eq!(price_per_bar(100.0, 130.0, 6.0), Some(5.0));
        assert_eq!(price_per_bar(130.0, 100.0, -6.0), Some(5.0));
        assert_eq!(price_per_bar(100.0, 100.0, 4.0), Some(0.0));
        assert_eq!(price_per_bar(100.0, 130.0, 0.0), None);
        assert_eq!(price_per_bar(f64::NAN, 130.0, 6.0), None);
    }

    #[test]
    fn a_square_of_a_given_scale_is_as_many_bars_tall_as_it_is_wide_times_the_scale() {
        // 6 bars across at 5 a bar: 30 up from 100, or down from it when the second is below.
        assert_eq!(scaled_price(100.0, 105.0, 6.0, 5.0), Some(130.0));
        assert_eq!(scaled_price(100.0, 90.0, 6.0, 5.0), Some(70.0));
        assert_eq!(scaled_price(100.0, 105.0, -6.0, 5.0), Some(130.0));
        assert_eq!(scaled_price(100.0, 105.0, 6.0, f64::NAN), None);
        assert_eq!(scaled_price(100.0, 105.0, 6.0, -1.0), None);
    }
}
