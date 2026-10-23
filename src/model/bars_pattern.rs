//! The Bars pattern drawing: a run of the chart's own bars, copied and set down somewhere else.
//!
//! The copy is **kept in the drawing**, as price offsets from the first bar's close, not read from
//! the chart again: a drawing that follows the data would change when history is paged in, when the
//! chart type changes, or when another Mac that does not hold those bars opens the layout. It is
//! bounded ([`MAX_BARS`]) so it stays a small record. It is a presentation-layer picture: never fed
//! back into the series, a script, an alert or a backtest.
//!
//! Pure: no egui types.

use super::drawings::Anchor;
use super::kline::Kline;
use serde::{Deserialize, Serialize};

/// The most bars one pattern keeps: a longer selection keeps its first `MAX_BARS`.
pub const MAX_BARS: usize = 500;

/// How the bars are shown.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BarsMode {
    /// Open-high-low-close bars.
    #[default]
    Bars,
    Candles,
    /// A line through one price of each bar.
    LineClose,
    LineOpen,
    LineHigh,
    LineLow,
    LineHl2,
}

impl BarsMode {
    pub const ALL: [Self; 7] = [
        Self::Bars,
        Self::Candles,
        Self::LineClose,
        Self::LineOpen,
        Self::LineHigh,
        Self::LineLow,
        Self::LineHl2,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Bars => "Bars",
            Self::Candles => "Candles",
            Self::LineClose => "Line - Close",
            Self::LineOpen => "Line - Open",
            Self::LineHigh => "Line - High",
            Self::LineLow => "Line - Low",
            Self::LineHl2 => "Line - HL/2",
        }
    }

    /// The price of a bar a line mode follows, or `None` for the two that draw whole bars.
    pub fn line_price(self, bar: &[f64; 4]) -> Option<f64> {
        let [open, high, low, close] = *bar;
        match self {
            Self::Bars | Self::Candles => None,
            Self::LineClose => Some(close),
            Self::LineOpen => Some(open),
            Self::LineHigh => Some(high),
            Self::LineLow => Some(low),
            Self::LineHl2 => Some((high + low) / 2.0),
        }
    }
}

/// A copied run of bars and where it is set down.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BarsPattern {
    /// Where the first bar's close sits: its time is the first bar's, its price the close's.
    pub at: Anchor,
    /// When the copied bars began and ended on the chart they were copied from, for the
    /// dialog; nothing is read from them.
    pub from_ms: i64,
    pub to_ms: i64,
    /// Each bar as `[open, high, low, close]` relative to the first bar's close.
    pub bars: Vec<[f64; 4]>,
    #[serde(default)]
    pub mode: BarsMode,
    /// Reflected vertically: higher becomes lower.
    #[serde(default)]
    pub mirrored: bool,
    /// Reflected horizontally: run backwards in time.
    #[serde(default)]
    pub flipped: bool,
}

impl BarsPattern {
    /// Copies the bars `first..=last` (indexes into `source`, either order, clamped to the data) to
    /// be set down at `at`. The first [`MAX_BARS`] are kept. `None` when there is nothing there to
    /// copy or it is not a number.
    pub fn capture(source: &[Kline], first: usize, last: usize, at: Anchor) -> Option<Self> {
        if source.is_empty() || !at.is_finite() {
            return None;
        }
        let end = source.len() - 1;
        let (low, high) = (first.min(last).min(end), first.max(last).min(end));
        let kept = &source[low..=high.min(low + MAX_BARS - 1)];
        let base = kept[0].close;
        let bars: Vec<[f64; 4]> = kept
            .iter()
            .map(|bar| {
                [
                    bar.open - base,
                    bar.high - base,
                    bar.low - base,
                    bar.close - base,
                ]
            })
            .collect();
        let pattern = Self {
            at,
            from_ms: kept[0].open_time,
            to_ms: kept[kept.len() - 1].open_time,
            bars,
            mode: BarsMode::default(),
            mirrored: false,
            flipped: false,
        };
        pattern.is_valid().then_some(pattern)
    }

    /// Whether the pattern could have been captured: at least one bar, at most [`MAX_BARS`], every
    /// number a number.
    pub fn is_valid(&self) -> bool {
        (1..=MAX_BARS).contains(&self.bars.len())
            && self.at.is_finite()
            && self
                .bars
                .iter()
                .all(|bar| bar.iter().all(|value| value.is_finite()))
    }

    /// The bars as they are shown: each `[open, high, low, close]` as an offset from the price the
    /// pattern is set down at, with the reflections applied.
    ///
    /// Reflected vertically, every price is negated (and a bar's high and low swap). Reflected
    /// horizontally, the bars run in the opposite order and each bar's open and close swap, as
    /// time running backwards would make them. Either way the first bar shown closes on the anchor.
    pub fn shown(&self) -> Vec<[f64; 4]> {
        let mut bars = self.bars.clone();
        if self.mirrored {
            for bar in &mut bars {
                let [open, high, low, close] = *bar;
                *bar = [-open, -low, -high, -close];
            }
        }
        if self.flipped {
            bars.reverse();
            for bar in &mut bars {
                bar.swap(0, 3);
            }
        }
        if let Some(first) = bars.first() {
            let shift = first[3];
            if shift != 0.0 {
                for bar in &mut bars {
                    for value in bar {
                        *value -= shift;
                    }
                }
            }
        }
        bars
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(open: f64, high: f64, low: f64, close: f64, time: i64) -> Kline {
        Kline {
            open_time: time,
            close_time: time + 59,
            open,
            high,
            low,
            close,
            volume: 1.0,
        }
    }

    fn source() -> Vec<Kline> {
        vec![
            bar(99.0, 102.0, 98.0, 100.0, 0),
            bar(100.0, 106.0, 99.0, 105.0, 60),
            bar(105.0, 107.0, 101.0, 102.0, 120),
            bar(102.0, 103.0, 96.0, 97.0, 180),
        ]
    }

    fn at() -> Anchor {
        Anchor {
            time_ms: 1_000,
            price: 50.0,
        }
    }

    #[test]
    fn a_capture_is_the_bars_as_offsets_from_the_first_close() {
        let pattern = BarsPattern::capture(&source(), 0, 2, at()).unwrap();
        assert_eq!(pattern.bars.len(), 3);
        // The first close (100) is zero.
        assert_eq!(pattern.bars[0], [-1.0, 2.0, -2.0, 0.0]);
        assert_eq!(pattern.bars[1], [0.0, 6.0, -1.0, 5.0]);
        assert_eq!(pattern.bars[2], [5.0, 7.0, 1.0, 2.0]);
        assert_eq!((pattern.from_ms, pattern.to_ms), (0, 120));
        assert_eq!(pattern.at, at());
        assert!(pattern.is_valid());
    }

    #[test]
    fn the_range_may_be_selected_either_way_round_and_is_kept_in_the_data() {
        let forward = BarsPattern::capture(&source(), 1, 3, at()).unwrap();
        let backward = BarsPattern::capture(&source(), 3, 1, at()).unwrap();
        assert_eq!(forward, backward);
        assert_eq!(forward.bars.len(), 3);
        // Past the data is the data there is.
        let clamped = BarsPattern::capture(&source(), 2, 99, at()).unwrap();
        assert_eq!(clamped.bars.len(), 2);
        let beyond = BarsPattern::capture(&source(), 50, 99, at()).unwrap();
        assert_eq!(beyond.bars.len(), 1, "the last bar");
    }

    #[test]
    fn a_long_selection_keeps_its_first_bars_only() {
        let long: Vec<Kline> = (0..MAX_BARS as i64 + 100)
            .map(|i| bar(1.0, 2.0, 0.5, 1.5, i * 60))
            .collect();
        let pattern = BarsPattern::capture(&long, 0, long.len() - 1, at()).unwrap();
        assert_eq!(pattern.bars.len(), MAX_BARS);
        assert_eq!(pattern.to_ms, (MAX_BARS as i64 - 1) * 60);
    }

    #[test]
    fn nothing_to_copy_is_none() {
        assert!(BarsPattern::capture(&[], 0, 3, at()).is_none());
        let bad_anchor = Anchor {
            time_ms: 0,
            price: f64::NAN,
        };
        assert!(BarsPattern::capture(&source(), 0, 2, bad_anchor).is_none());
        let mut broken = source();
        broken[1].high = f64::NAN;
        assert!(BarsPattern::capture(&broken, 0, 2, at()).is_none());
    }

    #[test]
    fn shown_as_copied_it_is_what_was_captured() {
        let pattern = BarsPattern::capture(&source(), 0, 2, at()).unwrap();
        assert_eq!(pattern.shown(), pattern.bars);
    }

    #[test]
    fn mirrored_turns_every_price_over_the_anchor() {
        let mut pattern = BarsPattern::capture(&source(), 0, 2, at()).unwrap();
        pattern.mirrored = true;
        let shown = pattern.shown();
        // [open, high, low, close] = [-1, 2, -2, 0] becomes [1, 2, -2, 0]: high and low swap.
        assert_eq!(shown[0], [1.0, 2.0, -2.0, 0.0]);
        assert_eq!(shown[1], [0.0, 1.0, -6.0, -5.0]);
        // Every bar is still a bar: high above low, and both enclose open and close.
        for bar in &shown {
            assert!(bar[1] >= bar[2]);
            assert!(bar[1] >= bar[0].max(bar[3]) && bar[2] <= bar[0].min(bar[3]));
        }
    }

    #[test]
    fn flipped_runs_the_bars_backwards_and_swaps_each_open_and_close() {
        let mut pattern = BarsPattern::capture(&source(), 0, 2, at()).unwrap();
        pattern.flipped = true;
        let shown = pattern.shown();
        // The last bar [5, 7, 1, 2] comes first, with open and close swapped: [2, 7, 1, 5]; the
        // first bar shown closes on the anchor, so everything moves down by 5.
        assert_eq!(shown[0], [-3.0, 2.0, -4.0, 0.0]);
        assert_eq!(shown.len(), 3);
        assert_eq!(shown[0][3], 0.0);
        // The shape is the same walked the other way: the last bar shown is the first one,
        // opened and closed the other way round, and shifted the same 5.
        // First bar [-1, 2, -2, 0] swapped is [0, 2, -2, -1], shifted by -5: [-5, -3, -7, -6].
        assert_eq!(shown[2], [-5.0, -3.0, -7.0, -6.0]);
    }

    #[test]
    fn flipping_twice_is_the_pattern_again_and_both_reflections_commute() {
        let mut pattern = BarsPattern::capture(&source(), 0, 3, at()).unwrap();
        pattern.flipped = true;
        let once = pattern.shown();
        // Flip what was shown by hand: running it back restores the bars it started from.
        let mut again = BarsPattern {
            bars: once.clone(),
            flipped: true,
            ..pattern.clone()
        };
        again.mirrored = false;
        let twice = again.shown();
        let original = BarsPattern::capture(&source(), 0, 3, at()).unwrap().bars;
        for (a, b) in twice.iter().zip(&original) {
            for (x, y) in a.iter().zip(b) {
                assert!((x - y).abs() < 1e-9, "{a:?} vs {b:?}");
            }
        }
        // Mirrored then flipped is the same picture as flipped then mirrored, and as both at once.
        let capture = || BarsPattern::capture(&source(), 0, 3, at()).unwrap();
        let (mut only_flipped, mut only_mirrored, mut both) = (capture(), capture(), capture());
        only_flipped.flipped = true;
        only_mirrored.mirrored = true;
        both.mirrored = true;
        both.flipped = true;
        let flipped_then_mirrored = BarsPattern {
            bars: only_flipped.shown(),
            flipped: false,
            mirrored: true,
            ..capture()
        }
        .shown();
        let mirrored_then_flipped = BarsPattern {
            bars: only_mirrored.shown(),
            flipped: true,
            mirrored: false,
            ..capture()
        }
        .shown();
        let at_once = both.shown();
        assert_eq!(at_once.len(), 4);
        assert_eq!(at_once[0][3], 0.0);
        for ((a, b), c) in flipped_then_mirrored
            .iter()
            .zip(&mirrored_then_flipped)
            .zip(&at_once)
        {
            for ((x, y), z) in a.iter().zip(b).zip(c) {
                assert!(
                    (x - y).abs() < 1e-9 && (x - z).abs() < 1e-9,
                    "{a:?} {b:?} {c:?}"
                );
            }
        }
    }

    #[test]
    fn a_line_mode_follows_one_price_of_each_bar() {
        let bar = [-1.0, 2.0, -2.0, 0.5];
        assert_eq!(BarsMode::Bars.line_price(&bar), None);
        assert_eq!(BarsMode::Candles.line_price(&bar), None);
        assert_eq!(BarsMode::LineClose.line_price(&bar), Some(0.5));
        assert_eq!(BarsMode::LineOpen.line_price(&bar), Some(-1.0));
        assert_eq!(BarsMode::LineHigh.line_price(&bar), Some(2.0));
        assert_eq!(BarsMode::LineLow.line_price(&bar), Some(-2.0));
        assert_eq!(BarsMode::LineHl2.line_price(&bar), Some(0.0));
        assert_eq!(BarsMode::ALL.len(), 7);
    }

    #[test]
    fn a_pattern_round_trips_through_json_and_old_rows_default_their_options() {
        let mut pattern = BarsPattern::capture(&source(), 0, 3, at()).unwrap();
        pattern.mode = BarsMode::LineHl2;
        pattern.mirrored = true;
        let json = serde_json::to_string(&pattern).unwrap();
        assert_eq!(serde_json::from_str::<BarsPattern>(&json).unwrap(), pattern);
        let bare =
            r#"{"at":{"time_ms":1,"price":2.0},"from_ms":0,"to_ms":1,"bars":[[0.0,1.0,-1.0,0.5]]}"#;
        let decoded: BarsPattern = serde_json::from_str(bare).unwrap();
        assert_eq!(decoded.mode, BarsMode::Bars);
        assert!(!decoded.mirrored && !decoded.flipped);
    }

    #[test]
    fn a_damaged_pattern_is_not_valid() {
        let mut pattern = BarsPattern::capture(&source(), 0, 2, at()).unwrap();
        pattern.bars.clear();
        assert!(!pattern.is_valid());
        let mut nan = BarsPattern::capture(&source(), 0, 2, at()).unwrap();
        nan.bars[1][2] = f64::INFINITY;
        assert!(!nan.is_valid());
        let mut huge = BarsPattern::capture(&source(), 0, 2, at()).unwrap();
        huge.bars = vec![[0.0; 4]; MAX_BARS + 1];
        assert!(!huge.is_valid());
    }
}
