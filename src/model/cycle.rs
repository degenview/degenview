//! The cycle drawings (Cyclic lines, Time cycles, Sine line): what each one's two anchors mean
//! and how their repeats are found.
//!
//! **Cycles are measured in bars, not in clock time.** A cycle's period is the distance between
//! its anchors in the chart's bar index (`TimeIndex::index_of`, fractional between bars), because
//! TradingView's three help pages position every cycle anchor by a bar number, and because a
//! market that closes at weekends has no bars there: a cycle of five bars still lands five bars
//! on, not five days. The anchors stay times, so the cycle follows a timeframe switch to the same
//! moments, and is simply counted in the new timeframe's bars. Pure: no egui types.

use super::drawing_tool::Tool;
use serde::{Deserialize, Serialize};

/// The shortest period (in bars) a cycle is drawn at: anything shorter has no period to repeat.
pub const MIN_PERIOD: f64 = 0.01;
/// The most repeats one cycle ever draws, however far the chart is zoomed out.
pub const MAX_REPEATS: usize = 2_000;

/// Which cycle a [`Geometry::Cycle`](super::drawings::Geometry::Cycle) is. Each is placed with two
/// anchors.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CycleKind {
    /// Vertical lines at the distance between the two anchors, on and on.
    CyclicLines,
    /// Arcs, one over each stretch of that distance.
    TimeCycles,
    /// A sine wave through both anchors, which are consecutive extremes: half a period apart.
    SineLine,
}

impl CycleKind {
    #[cfg(test)]
    pub const ALL: [Self; 3] = [Self::CyclicLines, Self::TimeCycles, Self::SineLine];

    pub const fn tool(self) -> Tool {
        match self {
            Self::CyclicLines => Tool::CyclicLines,
            Self::TimeCycles => Tool::TimeCycles,
            Self::SineLine => Tool::SineLine,
        }
    }

    pub const fn of_tool(tool: Tool) -> Option<Self> {
        match tool {
            Tool::CyclicLines => Some(Self::CyclicLines),
            Tool::TimeCycles => Some(Self::TimeCycles),
            Tool::SineLine => Some(Self::SineLine),
            _ => None,
        }
    }
}

/// The signed period from bar index `first` to bar index `second`, or `None` when it is not a
/// period: not finite, or shorter than [`MIN_PERIOD`] (a zero period would repeat forever).
pub fn period(first: f64, second: f64) -> Option<f64> {
    let period = second - first;
    (period.is_finite() && period.abs() >= MIN_PERIOD).then_some(period)
}

/// The bar indexes `first + k * period` for k = 0, 1, 2 .. that lie in `lo..=hi`, in order of k.
/// A negative period runs the repeats to the left. `None` when more than [`MAX_REPEATS`] would
/// show (a window that holds that many repeats of one cycle is not drawn), or for a bad input;
/// otherwise at most [`MAX_REPEATS`] long.
pub fn positions(first: f64, period: f64, lo: f64, hi: f64) -> Option<Vec<f64>> {
    if !(first.is_finite() && period.is_finite() && lo.is_finite() && hi.is_finite())
        || period.abs() < MIN_PERIOD
        || lo > hi
    {
        return None;
    }
    // The k whose position first enters the window, and the last that is still in it.
    let (start, end) = if period > 0.0 {
        ((lo - first) / period, (hi - first) / period)
    } else {
        ((hi - first) / period, (lo - first) / period)
    };
    let start = start.ceil().max(0.0);
    let end = end.floor();
    if !(start.is_finite() && end.is_finite()) || end < start {
        return Some(Vec::new());
    }
    if end - start + 1.0 > MAX_REPEATS as f64 {
        return None;
    }
    Some(
        (start as u64..=end as u64)
            .map(|k| first + k as f64 * period)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_is_one_tool() {
        for kind in CycleKind::ALL {
            assert_eq!(CycleKind::of_tool(kind.tool()), Some(kind));
        }
    }

    #[test]
    fn a_period_is_the_signed_distance_in_bars() {
        assert_eq!(period(10.0, 14.5), Some(4.5));
        assert_eq!(period(10.0, 7.0), Some(-3.0));
        assert_eq!(period(3.0, 3.0), None, "a zero period repeats forever");
        assert_eq!(period(0.0, 0.005), None, "shorter than the minimum");
        assert_eq!(period(0.0, f64::NAN), None);
        assert_eq!(period(f64::NEG_INFINITY, 0.0), None);
    }

    #[test]
    fn repeats_start_at_the_first_anchor_and_follow_the_period() {
        // Period 5 from bar 2: 2, 7, 12, 17, ... and only those in 0..=30.
        let found = positions(2.0, 5.0, 0.0, 30.0).unwrap();
        assert_eq!(found, [2.0, 7.0, 12.0, 17.0, 22.0, 27.0]);
        // Nothing before the first anchor, even when the window reaches back.
        assert_eq!(positions(2.0, 5.0, -100.0, 8.0).unwrap(), [2.0, 7.0]);
    }

    #[test]
    fn a_window_beyond_the_first_anchor_starts_at_the_first_repeat_inside_it() {
        // 2 + 5k in 100..=121 is k = 20 (102), 21 (107), 22 (112), 23 (117).
        assert_eq!(
            positions(2.0, 5.0, 100.0, 121.0).unwrap(),
            [102.0, 107.0, 112.0, 117.0]
        );
        // An exact edge counts on both sides.
        assert_eq!(
            positions(0.0, 10.0, 10.0, 30.0).unwrap(),
            [10.0, 20.0, 30.0]
        );
    }

    #[test]
    fn a_negative_period_runs_to_the_left() {
        // From 20 back by 6: 20, 14, 8, 2, and then -4 is out of the window.
        assert_eq!(
            positions(20.0, -6.0, 0.0, 30.0).unwrap(),
            [20.0, 14.0, 8.0, 2.0]
        );
        assert_eq!(
            positions(20.0, -6.0, 25.0, 40.0).unwrap(),
            Vec::<f64>::new()
        );
    }

    #[test]
    fn fractional_periods_land_between_bars() {
        let found = positions(0.0, 2.5, 0.0, 10.0).unwrap();
        assert_eq!(found, [0.0, 2.5, 5.0, 7.5, 10.0]);
    }

    #[test]
    fn a_window_too_full_of_repeats_is_refused_instead_of_drawn() {
        // 0.01 bars apart across 1000 bars is 100,000 repeats.
        assert_eq!(positions(0.0, 0.01, 0.0, 1000.0), None);
        // Exactly the limit is still drawn.
        let at_limit = positions(0.0, 1.0, 0.0, (MAX_REPEATS - 1) as f64).unwrap();
        assert_eq!(at_limit.len(), MAX_REPEATS);
        assert_eq!(positions(0.0, 1.0, 0.0, MAX_REPEATS as f64), None);
    }

    #[test]
    fn bad_input_has_no_repeats() {
        assert_eq!(positions(0.0, 0.0, 0.0, 10.0), None);
        assert_eq!(positions(f64::NAN, 1.0, 0.0, 10.0), None);
        assert_eq!(positions(0.0, 1.0, 10.0, 0.0), None, "an empty window");
        assert_eq!(positions(0.0, 1.0, 0.0, f64::INFINITY), None);
    }
}
