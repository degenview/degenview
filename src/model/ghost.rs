//! The Ghost feed's candles: a hypothetical run of candles from one price to another.
//!
//! TradingView describes the tool as one that "uses previous data as well as your inputs to project
//! the likelihood of certain candles", sized by an average high-low and a variance, without saying
//! how the candles are made. This build makes them from nothing but the two anchors, the two
//! numbers and a stored seed: a random walk pinned at both ends (so the last candle closes where
//! the second anchor is), each candle's range about the average, the same every time it is drawn.
//! They are a picture of one way the move could go. They carry no probability, are never real
//! candles, and nothing but the drawing ever reads them.
//!
//! Pure: no egui types, no clock, no global random number generator.

/// The most candles one ghost feed makes.
pub const MAX_CANDLES: usize = 500;

/// One ghost candle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candle {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

/// SplitMix64: a small generator that is fully determined by its seed.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `[0, 1)`.
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// A number in `[-1, 1)`.
    fn signed(&mut self) -> f64 {
        self.unit() * 2.0 - 1.0
    }
}

/// `count` candles from `start` to `end` (a price each), with an average high-to-low range of
/// `average_range` (in price) and `variance` (0 to 1) of how much that and the path wander.
///
/// - The closes follow the straight line from `start` to `end` plus a random walk pinned to zero at
///   both ends, so the last candle closes exactly on `end`; each candle opens where the one before
///   closed, the first at `start`.
/// - A candle's range is the average times `1 ± variance`, never smaller than its body; its wicks
///   share what is left over by a random split.
/// - The same inputs and seed give the same candles.
///
/// Empty for no candles, a count over [`MAX_CANDLES`] (the drawing is bounded by its caller), or
/// numbers that are not numbers.
pub fn candles(
    start: f64,
    end: f64,
    count: usize,
    average_range: f64,
    variance: f64,
    seed: u64,
) -> Vec<Candle> {
    if count == 0
        || count > MAX_CANDLES
        || !(start.is_finite() && end.is_finite() && average_range.is_finite())
        || average_range < 0.0
        || !variance.is_finite()
    {
        return Vec::new();
    }
    let variance = variance.clamp(0.0, 1.0);
    let mut random = Random(seed);
    // The walk: a step at most half the average range each way, scaled by the variance.
    let step = average_range * 0.5 * variance;
    let mut walk = Vec::with_capacity(count + 1);
    walk.push(0.0);
    for _ in 0..count {
        let last = walk[walk.len() - 1];
        walk.push(last + random.signed() * step);
    }
    let drift = walk[count];
    let mut previous = start;
    (1..=count)
        .map(|index| {
            let along = index as f64 / count as f64;
            // Pinned: the walk minus its straight line to where it ended is zero at both ends.
            let bridge = walk[index] - drift * along;
            let close = if index == count {
                end
            } else {
                start + (end - start) * along + bridge
            };
            let open = previous;
            previous = close;
            let range = (average_range * (1.0 + variance * random.signed())).max(0.0);
            let body = (close - open).abs();
            let spare = (range - body).max(0.0);
            let split = random.unit();
            Candle {
                open,
                high: open.max(close) + spare * split,
                low: open.min(close) - spare * (1.0 - split),
                close,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_inputs_make_the_same_candles_and_another_seed_makes_others() {
        let a = candles(100.0, 120.0, 20, 2.0, 0.5, 7);
        let b = candles(100.0, 120.0, 20, 2.0, 0.5, 7);
        assert_eq!(a, b);
        assert_ne!(a, candles(100.0, 120.0, 20, 2.0, 0.5, 8));
        assert_eq!(a.len(), 20);
    }

    #[test]
    fn the_run_starts_at_the_first_price_and_closes_on_the_second() {
        for seed in [0, 1, 42, u64::MAX] {
            let run = candles(100.0, 120.0, 25, 3.0, 1.0, seed);
            assert_eq!(run[0].open, 100.0);
            assert_eq!(run.last().unwrap().close, 120.0);
            // Each candle opens where the one before it closed.
            for pair in run.windows(2) {
                assert_eq!(pair[1].open, pair[0].close);
            }
        }
    }

    #[test]
    fn every_candle_is_a_candle() {
        for seed in 0..50 {
            for candle in candles(50.0, 40.0, 30, 1.5, 1.0, seed) {
                assert!(candle.high >= candle.open.max(candle.close), "{candle:?}");
                assert!(candle.low <= candle.open.min(candle.close), "{candle:?}");
                assert!(candle.low.is_finite() && candle.high.is_finite());
            }
        }
    }

    #[test]
    fn with_no_variance_the_path_is_a_straight_line_of_candles_of_the_average_range() {
        // 10 to 20 in 5 candles: each closes 2 higher; the body (2) is under the range (3), so every
        // candle is exactly 3 from low to high.
        let run = candles(10.0, 20.0, 5, 3.0, 0.0, 99);
        for (index, candle) in run.iter().enumerate() {
            assert!(
                (candle.close - (12.0 + 2.0 * index as f64)).abs() < 1e-9,
                "{candle:?}"
            );
            assert!((candle.high - candle.low - 3.0).abs() < 1e-9, "{candle:?}");
        }
    }

    #[test]
    fn a_big_body_is_not_squeezed_into_a_small_range() {
        // A jump of 10 in one candle with an average range of 1: the candle is its body, no wicks.
        let run = candles(100.0, 110.0, 1, 1.0, 0.0, 3);
        assert_eq!(run.len(), 1);
        assert!((run[0].high - 110.0).abs() < 1e-9 && (run[0].low - 100.0).abs() < 1e-9);
    }

    #[test]
    fn it_runs_down_as_well_as_up() {
        let run = candles(100.0, 80.0, 10, 1.0, 0.0, 5);
        assert!(run.windows(2).all(|pair| pair[1].close < pair[0].close));
        assert_eq!(run.last().unwrap().close, 80.0);
    }

    #[test]
    fn a_wider_variance_wanders_further_from_the_line() {
        let wander = |variance: f64| {
            candles(100.0, 100.0, 50, 4.0, variance, 11)
                .iter()
                .map(|candle| (candle.close - 100.0).abs())
                .fold(0.0, f64::max)
        };
        assert!(wander(0.0) < 1e-9);
        assert!(wander(1.0) > wander(0.2));
    }

    #[test]
    fn no_range_means_no_wicks() {
        for candle in candles(10.0, 20.0, 5, 0.0, 1.0, 1) {
            assert_eq!(candle.high, candle.open.max(candle.close));
            assert_eq!(candle.low, candle.open.min(candle.close));
        }
    }

    #[test]
    fn nothing_that_is_not_a_number_makes_candles() {
        assert!(candles(1.0, 2.0, 0, 1.0, 0.5, 1).is_empty());
        assert!(candles(1.0, 2.0, MAX_CANDLES + 1, 1.0, 0.5, 1).is_empty());
        assert_eq!(
            candles(1.0, 2.0, MAX_CANDLES, 1.0, 0.5, 1).len(),
            MAX_CANDLES
        );
        assert!(candles(f64::NAN, 2.0, 5, 1.0, 0.5, 1).is_empty());
        assert!(candles(1.0, f64::INFINITY, 5, 1.0, 0.5, 1).is_empty());
        assert!(candles(1.0, 2.0, 5, -1.0, 0.5, 1).is_empty());
        assert!(candles(1.0, 2.0, 5, 1.0, f64::NAN, 1).is_empty());
        // A variance out of range is brought in, not refused.
        assert_eq!(candles(1.0, 2.0, 5, 1.0, 9.0, 1).len(), 5);
    }

    #[test]
    fn the_candles_are_never_more_than_the_inputs_can_explain() {
        // Even at full variance each close stays within one walk's reach of the line.
        let run = candles(100.0, 100.0, 40, 2.0, 1.0, 77);
        assert!(
            run.iter()
                .all(|candle| (candle.close - 100.0).abs() <= 40.0 * 1.0)
        );
    }
}
