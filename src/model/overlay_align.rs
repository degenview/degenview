//! Putting a comparison market's candles on the chart's bars. Pure.
//!
//! The chart's bars are the time axis. An overlay candle belongs to the bar whose span holds the
//! candle's *midpoint*, never to the bar at the same index and not to the bar with the same open
//! time: providers stamp the same day differently (Alpaca's daily bar opens at New York midnight,
//! a crypto day at 00:00 UTC), and matching open times would put one a day off. A bar's span runs
//! from its open to the earlier of the next bar's open and one nominal bar length later, so a
//! market with weekend gaps does not take a neighbouring market's weekend.
//!
//! A bar no candle lands on is `None`: nothing is carried forward and nothing is invented.

use super::{Kline, chart_type::PriceSource};

/// The values of one overlay on the chart's bars, `None` where it has no candle.
pub type Aligned = Vec<Option<f64>>;

/// The bar of `bars` (open times, ascending) whose span holds `at`, if any.
fn bar_of(bars: &[i64], bar_ms: i64, at: i64) -> Option<usize> {
    let index = bars.partition_point(|open| *open <= at).checked_sub(1)?;
    let end = match bars.get(index + 1) {
        Some(next) => (*next).min(bars[index].saturating_add(bar_ms)),
        None => bars[index].saturating_add(bar_ms),
    };
    (at < end).then_some(index)
}

/// The value a candle contributes: finite and positive (a ratio and a logarithm need that), else
/// nothing.
fn value_of(source: PriceSource, candle: &Kline) -> Option<f64> {
    let value = source.value(candle);
    (value.is_finite() && value > 0.0).then_some(value)
}

/// `bars` are the open times of the chart's bars, ascending; `bar_ms` is the chart's nominal
/// bar length; `candle_ms` is the overlay's (the same timeframe, so usually equal). A bar two
/// candles land on takes the later one.
pub fn align(
    bars: &[i64],
    bar_ms: i64,
    candles: &[Kline],
    candle_ms: i64,
    source: PriceSource,
) -> Aligned {
    let mut out = vec![None; bars.len()];
    place(&mut out, bars, bar_ms, candles, candle_ms, source);
    out
}

/// Re-places `candles` (the newest few of an overlay) into `out` after a live update, instead of
/// aligning the whole history again. The result equals a full [`align`] as long as only those
/// candles changed. A candle that no longer yields a value clears its bar.
pub fn patch_tail(
    out: &mut Aligned,
    bars: &[i64],
    bar_ms: i64,
    tail: &[Kline],
    candle_ms: i64,
    source: PriceSource,
) {
    for candle in tail {
        if let Some(index) = bar_of(bars, bar_ms, candle.open_time.saturating_add(candle_ms / 2)) {
            if index < out.len() {
                out[index] = value_of(source, candle);
            }
        }
    }
}

fn place(
    out: &mut Aligned,
    bars: &[i64],
    bar_ms: i64,
    candles: &[Kline],
    candle_ms: i64,
    source: PriceSource,
) {
    for candle in candles {
        let Some(index) = bar_of(bars, bar_ms, candle.open_time.saturating_add(candle_ms / 2))
        else {
            continue;
        };
        if let Some(value) = value_of(source, candle) {
            out[index] = Some(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3_600_000;
    const DAY: i64 = 24 * HOUR;

    fn candle(open_time: i64, close: f64) -> Kline {
        Kline {
            open_time,
            close_time: open_time,
            open: close,
            high: close,
            low: close,
            close,
            volume: 1.0,
        }
    }

    fn bars(count: usize, start: i64, step: i64) -> Vec<i64> {
        (0..count as i64).map(|n| start + n * step).collect()
    }

    #[test]
    fn candles_land_on_their_own_bar_by_time_not_by_index() {
        // The overlay starts three bars later than the chart: its first value is bar 3.
        let chart = bars(6, 0, HOUR);
        let overlay = vec![candle(3 * HOUR, 10.0), candle(4 * HOUR, 11.0)];
        let aligned = align(&chart, HOUR, &overlay, HOUR, PriceSource::Close);
        assert_eq!(aligned, [None, None, None, Some(10.0), Some(11.0), None]);
    }

    #[test]
    fn a_session_market_does_not_fill_the_weekend_of_a_continuous_one() {
        // Chart: 7 daily crypto bars Mon..Sun. Overlay: stock bars Mon..Fri only.
        let chart = bars(7, 0, DAY);
        let stock: Vec<_> = (0..5).map(|n| candle(n * DAY, 100.0 + n as f64)).collect();
        let aligned = align(&chart, DAY, &stock, DAY, PriceSource::Close);
        assert_eq!(aligned[4], Some(104.0));
        assert_eq!(aligned[5], None);
        assert_eq!(aligned[6], None);
    }

    #[test]
    fn crypto_weekend_candles_do_not_land_on_a_stock_chart_bar() {
        // Chart: stock days Mon..Fri (the Friday bar must not stretch over the weekend).
        let chart = bars(5, 0, DAY);
        let crypto: Vec<_> = (0..7).map(|n| candle(n * DAY, 1.0 + n as f64)).collect();
        let aligned = align(&chart, DAY, &crypto, DAY, PriceSource::Close);
        assert_eq!(
            aligned,
            [Some(1.0), Some(2.0), Some(3.0), Some(4.0), Some(5.0)]
        );
    }

    #[test]
    fn another_providers_day_stamp_still_lands_on_the_same_day() {
        // Chart: crypto days at 00:00 UTC. Overlay: US stock days opening 05:00 UTC.
        let chart = bars(4, 0, DAY);
        let stock: Vec<_> = (0..4)
            .map(|n| candle(n * DAY + 5 * HOUR, 50.0 + n as f64))
            .collect();
        let aligned = align(&chart, DAY, &stock, DAY, PriceSource::Close);
        assert_eq!(aligned, [Some(50.0), Some(51.0), Some(52.0), Some(53.0)]);
        // And the other way round: a stock chart, crypto overlay.
        let stock_chart: Vec<i64> = (0..4).map(|n| n * DAY + 5 * HOUR).collect();
        let crypto: Vec<_> = (0..4).map(|n| candle(n * DAY, 1.0 + n as f64)).collect();
        let aligned = align(&stock_chart, DAY, &crypto, DAY, PriceSource::Close);
        assert_eq!(aligned, [Some(1.0), Some(2.0), Some(3.0), Some(4.0)]);
    }

    #[test]
    fn candles_outside_the_chart_are_dropped() {
        let chart = bars(3, 10 * DAY, DAY);
        let overlay = vec![candle(0, 1.0), candle(10 * DAY, 2.0), candle(99 * DAY, 3.0)];
        let aligned = align(&chart, DAY, &overlay, DAY, PriceSource::Close);
        assert_eq!(aligned, [Some(2.0), None, None]);
    }

    #[test]
    fn missing_candles_stay_missing() {
        let chart = bars(5, 0, HOUR);
        let overlay = vec![candle(0, 1.0), candle(2 * HOUR, 3.0), candle(4 * HOUR, 5.0)];
        let aligned = align(&chart, HOUR, &overlay, HOUR, PriceSource::Close);
        assert_eq!(aligned, [Some(1.0), None, Some(3.0), None, Some(5.0)]);
    }

    #[test]
    fn invalid_prices_are_left_out_not_clamped() {
        let chart = bars(5, 0, HOUR);
        let overlay = vec![
            candle(0, 0.0),
            candle(HOUR, -4.0),
            candle(2 * HOUR, f64::NAN),
            candle(3 * HOUR, f64::INFINITY),
            candle(4 * HOUR, 2.5),
        ];
        let aligned = align(&chart, HOUR, &overlay, HOUR, PriceSource::Close);
        assert_eq!(aligned, [None, None, None, None, Some(2.5)]);
    }

    #[test]
    fn two_candles_in_one_bar_take_the_later() {
        let chart = bars(2, 0, DAY);
        let overlay = vec![candle(0, 1.0), candle(HOUR, 2.0)];
        // 1h candles into a daily bar: both midpoints are inside bar 0.
        let aligned = align(&chart, DAY, &overlay, HOUR, PriceSource::Close);
        assert_eq!(aligned, [Some(2.0), None]);
    }

    #[test]
    fn the_source_picks_the_value() {
        let chart = bars(1, 0, HOUR);
        let mut bar = candle(0, 10.0);
        bar.open = 8.0;
        bar.high = 12.0;
        bar.low = 6.0;
        let aligned = |source| align(&chart, HOUR, std::slice::from_ref(&bar), HOUR, source)[0];
        assert_eq!(aligned(PriceSource::Open), Some(8.0));
        assert_eq!(aligned(PriceSource::High), Some(12.0));
        assert_eq!(aligned(PriceSource::Hl2), Some(9.0));
    }

    #[test]
    fn calendar_bars_cap_at_the_next_open() {
        // Months of 28/31 days against a 30-day nominal length.
        let starts = [0, 31 * DAY, 59 * DAY, 90 * DAY];
        let chart: Vec<i64> = starts.to_vec();
        let month = 30 * DAY;
        let overlay: Vec<_> = starts
            .iter()
            .map(|s| candle(*s, 1.0 + (*s / DAY) as f64))
            .collect();
        let aligned = align(&chart, month, &overlay, month, PriceSource::Close);
        assert!(aligned.iter().all(Option::is_some), "{aligned:?}");
    }

    #[test]
    fn patching_the_tail_equals_aligning_everything_again() {
        let chart = bars(50, 0, HOUR);
        let mut overlay: Vec<_> = (0..50)
            .map(|n| candle(n * HOUR, 100.0 + n as f64))
            .collect();
        let mut aligned = align(&chart, HOUR, &overlay, HOUR, PriceSource::Close);
        overlay[49] = candle(49 * HOUR, 777.0);
        patch_tail(
            &mut aligned,
            &chart,
            HOUR,
            &overlay[48..],
            HOUR,
            PriceSource::Close,
        );
        assert_eq!(
            aligned,
            align(&chart, HOUR, &overlay, HOUR, PriceSource::Close)
        );
        // A new live bar the chart has not got yet is ignored until the chart has it.
        overlay.push(candle(50 * HOUR, 5.0));
        patch_tail(
            &mut aligned,
            &chart,
            HOUR,
            &overlay[49..],
            HOUR,
            PriceSource::Close,
        );
        assert_eq!(
            aligned,
            align(&chart, HOUR, &overlay, HOUR, PriceSource::Close)
        );
    }

    #[test]
    fn an_empty_chart_or_overlay_aligns_to_nothing() {
        assert!(align(&[], DAY, &[candle(0, 1.0)], DAY, PriceSource::Close).is_empty());
        assert_eq!(
            align(&bars(3, 0, DAY), DAY, &[], DAY, PriceSource::Close),
            [None, None, None]
        );
    }
}
