//! Heikin Ashi: one smoothed bar per source candle.
//!
//! ```text
//! close = (open + high + low + close) / 4
//! open  = (previous open + previous close) / 2      first bar: (open + close) / 2
//! high  = max(high, open, close)
//! low   = min(low, open, close)
//! ```
//!
//! The open is recursive, so the series is computed over every candle that was loaded, never from
//! the first visible one: a different left edge would give different bars. Seeding the first bar
//! with `(open + close) / 2` is the example in Pine's own documentation for `ticker.heikinashi`.
//! A deeper history changes the earliest bars slightly and converges quickly, as on TradingView.

use super::{BarShape, SyntheticBar};
use crate::model::Kline;

pub(super) fn build(source: &[Kline], forming: Option<usize>) -> Vec<SyntheticBar> {
    let mut bars: Vec<SyntheticBar> = Vec::with_capacity(source.len());
    for (index, bar) in source.iter().enumerate() {
        let close = (bar.open + bar.high + bar.low + bar.close) / 4.0;
        let open = match bars.last() {
            Some(previous) => (previous.open + previous.close) / 2.0,
            None => (bar.open + bar.close) / 2.0,
        };
        bars.push(SyntheticBar {
            open,
            high: bar.high.max(open).max(close),
            low: bar.low.min(open).min(close),
            close,
            volume: bar.volume,
            first_source: index,
            last_source: index,
            open_time: bar.open_time,
            close_time: bar.close_time,
            confirmed: forming != Some(index),
            shape: BarShape::Ohlc,
        });
    }
    bars
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;

    #[test]
    fn follows_the_recursive_definition() {
        let source = vec![
            candle(0, 10.0, 14.0, 8.0, 12.0),
            candle(1, 12.0, 18.0, 11.0, 16.0),
            candle(2, 16.0, 17.0, 9.0, 10.0),
        ];
        let bars = build(&source, None);
        // Bar 0: close (10+14+8+12)/4 = 11, open (10+12)/2 = 11.
        assert_eq!((bars[0].open, bars[0].close), (11.0, 11.0));
        assert_eq!((bars[0].high, bars[0].low), (14.0, 8.0));
        // Bar 1: close (12+18+11+16)/4 = 14.25, open (11+11)/2 = 11.
        assert_eq!((bars[1].open, bars[1].close), (11.0, 14.25));
        assert_eq!((bars[1].high, bars[1].low), (18.0, 11.0));
        // Bar 2: close (16+17+9+10)/4 = 13, open (11+14.25)/2 = 12.625.
        assert_eq!((bars[2].open, bars[2].close), (12.625, 13.0));
        assert_eq!((bars[2].high, bars[2].low), (17.0, 9.0));
    }

    #[test]
    fn extremes_stretch_to_the_body_when_the_candle_is_tight() {
        let source = vec![
            candle(0, 10.0, 10.0, 10.0, 10.0),
            candle(1, 20.0, 20.0, 20.0, 20.0),
        ];
        let bars = build(&source, None);
        // The second bar's open (10) is below the candle's low (20): the low drops to it.
        assert_eq!(bars[1].open, 10.0);
        assert_eq!(bars[1].low, 10.0);
        assert_eq!(bars[1].high, 20.0);
    }

    #[test]
    fn one_bar_per_candle_and_the_forming_one_is_unconfirmed() {
        let source = closes(&[1.0, 2.0, 3.0]);
        let bars = build(&source, Some(2));
        assert_eq!(bars.len(), 3);
        assert!(bars[0].confirmed && bars[1].confirmed && !bars[2].confirmed);
        assert_eq!(bars[1].first_source, 1);
        assert_eq!(bars[1].open_time, source[1].open_time);
    }

    #[test]
    fn a_prefix_of_the_candles_gives_a_prefix_of_the_bars() {
        let source = vec![
            candle(0, 10.0, 14.0, 8.0, 12.0),
            candle(1, 12.0, 18.0, 11.0, 16.0),
            candle(2, 16.0, 17.0, 9.0, 10.0),
        ];
        let full = build(&source, None);
        let prefix = build(&source[..2], None);
        assert_eq!(&full[..2], &prefix[..]);
    }
}
