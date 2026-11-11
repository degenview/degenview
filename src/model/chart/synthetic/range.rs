//! Range bars: a bar closes when its high-to-low span reaches the range, whatever time that took.
//!
//! * A bar that breaks upward closes at `low + range`; one that breaks downward closes at
//!   `high - range`. Every finished bar spans exactly the range.
//! * The next bar opens where the last one closed, so there are no gaps, and a candle that moves
//!   several ranges makes several bars while a quiet one makes none.
//! * The bar still short of the range is shown as the live, unconfirmed bar.
//!
//! A candle does not record the order of its prices, so each is read as open, low, high, close
//! when it rose and open, high, low, close when it fell. Two candles with the same OHLC but a
//! different real path could make different bars on a tick chart; no tick-level parity is
//! claimed. The range is in price units (candles carry no tick size).

use super::{BarShape, SyntheticBar, price_path, tolerance};
use crate::model::Kline;
use crate::model::chart::kind::ConstructionSource;

struct Current {
    open: f64,
    high: f64,
    low: f64,
    first_source: usize,
    volume: f64,
}

pub(super) fn build(source: &[Kline], range: f64, forming: Option<usize>) -> Vec<SyntheticBar> {
    let slack = tolerance(range);
    let mut bars: Vec<SyntheticBar> = Vec::new();
    let mut current: Option<Current> = None;

    for (index, candle) in source.iter().enumerate() {
        if let Some(current) = current.as_mut() {
            current.volume += candle.volume;
        }
        let (path, count) = price_path(candle, ConstructionSource::HighLow);
        for &price in &path[..count] {
            let bar = current.get_or_insert(Current {
                open: price,
                high: price,
                low: price,
                first_source: index,
                volume: candle.volume,
            });
            loop {
                let (high, low) = (bar.high.max(price), bar.low.min(price));
                if high - low < range - slack {
                    bar.high = high;
                    bar.low = low;
                    break;
                }
                let up = price > bar.high;
                let (top, bottom) = if up {
                    (bar.low + range, bar.low)
                } else {
                    (bar.high, bar.high - range)
                };
                let close = if up { top } else { bottom };
                bars.push(SyntheticBar {
                    open: bar.open,
                    high: top,
                    low: bottom,
                    close,
                    volume: bar.volume,
                    first_source: bar.first_source,
                    last_source: index,
                    open_time: source[bar.first_source].open_time,
                    close_time: candle.close_time,
                    confirmed: forming != Some(index),
                    shape: BarShape::Ohlc,
                });
                *bar = Current {
                    open: close,
                    high: close,
                    low: close,
                    first_source: index,
                    volume: 0.0,
                };
            }
        }
    }
    if let Some(bar) = current
        && bar.high > bar.low
    {
        let last = source.len() - 1;
        let close = source[last].close;
        bars.push(SyntheticBar {
            open: bar.open,
            high: bar.high,
            low: bar.low,
            close: close.clamp(bar.low, bar.high),
            volume: bar.volume,
            first_source: bar.first_source,
            last_source: last,
            open_time: source[bar.first_source].open_time,
            close_time: source[last].close_time,
            confirmed: false,
            shape: BarShape::Ohlc,
        });
    }
    bars
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;

    fn run(source: &[Kline], range: f64) -> Vec<(f64, f64, f64, f64)> {
        build(source, range, None)
            .iter()
            .map(|bar| (bar.open, bar.high, bar.low, bar.close))
            .collect()
    }

    #[test]
    fn a_rising_candle_that_covers_two_ranges_makes_two_gapless_bars() {
        // Read as 10 → 10 (low) → 15 (high) → 14.
        let source = vec![candle(0, 10.0, 15.0, 10.0, 14.0)];
        let bars = run(&source, 2.0);
        assert_eq!(bars[0], (10.0, 12.0, 10.0, 12.0));
        assert_eq!(bars[1], (12.0, 14.0, 12.0, 14.0));
        // 14 → 15 is one more unit: the live bar.
        assert_eq!(bars.len(), 3);
        assert_eq!((bars[2].0, bars[2].1, bars[2].2), (14.0, 15.0, 14.0));
    }

    #[test]
    fn a_falling_break_closes_at_the_low_of_the_range() {
        // Read as 20 → 24 (high) → 18 (low) → 19.
        let source = vec![candle(0, 20.0, 24.0, 18.0, 19.0)];
        let bars = run(&source, 5.0);
        // High 24 reached first; falling to 18 spans 6 ≥ 5: closes at 24 - 5 = 19 (down bar).
        assert_eq!(bars[0], (20.0, 24.0, 19.0, 19.0));
    }

    #[test]
    fn a_reversal_needs_a_full_range_from_the_last_close() {
        let source = vec![
            candle(0, 10.0, 12.0, 10.0, 12.0),
            candle(1, 12.0, 12.0, 10.5, 10.5),
        ];
        let bars = run(&source, 2.0);
        // Bar 1 up 10→12. Next bar opens at 12; falling to 10.5 is only 1.5: live bar, no flip.
        assert_eq!(bars[0], (10.0, 12.0, 10.0, 12.0));
        assert_eq!(bars.len(), 2);
        assert_eq!((bars[1].0, bars[1].1, bars[1].2), (12.0, 12.0, 10.5));
        let source = vec![
            candle(0, 10.0, 12.0, 10.0, 12.0),
            candle(1, 12.0, 12.0, 10.0, 10.0),
        ];
        let bars = run(&source, 2.0);
        assert_eq!(bars[1], (12.0, 12.0, 10.0, 10.0));
    }

    #[test]
    fn a_quiet_candle_makes_no_completed_bar() {
        let source = vec![
            candle(0, 10.0, 10.5, 9.8, 10.2),
            candle(1, 10.2, 10.6, 10.0, 10.3),
        ];
        let bars = build(&source, 5.0, None);
        assert_eq!(bars.len(), 1);
        assert!(!bars[0].confirmed, "the bar short of the range is live");
        assert!(bars[0].high - bars[0].low < 5.0);
    }

    #[test]
    fn every_completed_bar_spans_exactly_the_range() {
        let source: Vec<Kline> = (0..40)
            .map(|index| {
                let base = 100.0 + (index as f64 * 0.7).sin() * 9.0;
                candle(index, base, base + 3.0, base - 2.5, base + 1.0)
            })
            .collect();
        let bars = build(&source, 2.5, None);
        assert!(bars.len() > 4);
        for bar in &bars[..bars.len() - 1] {
            assert!((bar.high - bar.low - 2.5).abs() < 1e-9, "{bar:?}");
        }
        for pair in bars[..bars.len() - 1].windows(2) {
            assert!(
                (pair[1].open - pair[0].close).abs() < 1e-9,
                "gap in {pair:?}"
            );
        }
    }

    #[test]
    fn bars_made_on_the_forming_candle_are_unconfirmed() {
        let source = vec![
            candle(0, 10.0, 12.0, 10.0, 12.0),
            candle(1, 12.0, 16.0, 12.0, 16.0),
        ];
        let bars = build(&source, 2.0, Some(1));
        assert!(bars[0].confirmed, "completed on candle 0, which had closed");
        assert!(bars[1..].iter().all(|bar| !bar.confirmed));
        assert!(bars.len() >= 3);
    }

    #[test]
    fn a_prefix_of_the_candles_gives_a_prefix_of_the_completed_bars() {
        let source: Vec<Kline> = (0..30)
            .map(|index| {
                let base = 50.0 + (index as f64 * 0.5).cos() * 6.0;
                candle(index, base, base + 2.0, base - 2.0, base - 0.5)
            })
            .collect();
        let full = build(&source, 1.5, None);
        let prefix = build(&source[..20], 1.5, None);
        let settled = prefix.len() - 1;
        assert_eq!(&full[..settled], &prefix[..settled]);
    }
}
