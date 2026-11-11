//! The price step of a market and the footprint's automatic row size.
//!
//! An exchange's tick size is used where the services know it (`service::tick_size`). Where they do
//! not, a step is **inferred** from the prices the chart holds: the coarsest power of ten every one of
//! them is a multiple of. That is the decimals the feed prints, not the exchange's true step (a
//! market quoted in 0.5s infers 0.1), and the chart says which it used ([`TickSource`]).

use crate::model::kline::Kline;

/// Where a tick size came from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TickSource {
    Exchange,
    /// Worked out from the decimals of the prices.
    Inferred,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tick {
    pub size: f64,
    pub source: TickSource,
}

impl Tick {
    /// `exchange` when it is a usable step, else a step inferred from `bars`; `None` when neither.
    pub fn resolve(exchange: Option<f64>, bars: &[Kline]) -> Option<Self> {
        match exchange {
            Some(size) if size.is_finite() && size > 0.0 => Some(Self {
                size,
                source: TickSource::Exchange,
            }),
            _ => infer(bars).map(|size| Self {
                size,
                source: TickSource::Inferred,
            }),
        }
    }
}

/// Bars looked at when inferring (the newest ones).
const INFER_BARS: usize = 200;
/// The finest step tried (ten decimals).
const MAX_DECIMALS: i32 = 10;

/// The coarsest `10^-d` (`d` from 0 to 10) that every open, high, low and close of the newest bars
/// is a multiple of; `None` when there are no usable prices or none fits.
pub fn infer(bars: &[Kline]) -> Option<f64> {
    let start = bars.len().saturating_sub(INFER_BARS);
    let prices: Vec<f64> = bars[start..]
        .iter()
        .flat_map(|bar| [bar.open, bar.high, bar.low, bar.close])
        .filter(|price| price.is_finite() && *price > 0.0)
        .collect();
    if prices.is_empty() {
        return None;
    }
    (0..=MAX_DECIMALS).map(|d| 10f64.powi(-d)).find(|step| {
        prices.iter().all(|price| {
            let multiples = price / step;
            (multiples - multiples.round()).abs() <= 1e-6 + multiples.abs() * 1e-12
        })
    })
}

/// TradingView's automatic row size: a fifth of the average true range, in whole ticks, at least
/// one. `None` for an average that is not a positive number.
pub fn auto_ticks_per_row(average_true_range: f64, tick: f64) -> Option<u32> {
    if !(average_true_range.is_finite()
        && average_true_range > 0.0
        && tick.is_finite()
        && tick > 0.0)
    {
        return None;
    }
    let ticks = (0.2 * average_true_range / tick).round();
    Some(ticks.clamp(1.0, f64::from(u32::MAX)) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(open: f64, high: f64, low: f64, close: f64) -> Kline {
        Kline {
            open_time: 0,
            close_time: 0,
            open,
            high,
            low,
            close,
            volume: 1.0,
        }
    }

    #[test]
    fn the_step_is_the_decimals_the_prices_use() {
        assert_eq!(infer(&[bar(100.0, 105.0, 99.0, 101.0)]), Some(1.0));
        assert_eq!(infer(&[bar(100.5, 105.0, 99.0, 101.0)]), Some(0.1));
        assert_eq!(infer(&[bar(100.25, 105.0, 99.0, 101.0)]), Some(0.01));
        let step = infer(&[bar(0.000_123_4, 0.000_125, 0.000_12, 0.000_124)]).unwrap();
        assert!((step - 1e-7).abs() < 1e-15, "{step}");
    }

    #[test]
    fn float_noise_in_a_price_does_not_hide_its_step() {
        // 0.1 + 0.2 is not exactly 0.3.
        assert_eq!(infer(&[bar(0.1 + 0.2, 0.7, 0.1, 0.3)]), Some(0.1));
    }

    #[test]
    fn nothing_usable_infers_nothing() {
        assert_eq!(infer(&[]), None);
        assert_eq!(infer(&[bar(f64::NAN, f64::NAN, f64::NAN, f64::NAN)]), None);
        // More than ten decimals.
        assert_eq!(infer(&[bar(1.234_567_890_123_4, 2.0, 1.0, 1.5)]), None);
    }

    #[test]
    fn only_the_newest_bars_count() {
        let mut bars = vec![bar(1.234_5, 1.3, 1.2, 1.25)];
        bars.extend((0..INFER_BARS).map(|_| bar(100.0, 101.0, 99.0, 100.0)));
        assert_eq!(infer(&bars), Some(1.0));
    }

    #[test]
    fn an_exchange_tick_wins_and_a_bad_one_falls_back() {
        let bars = [bar(100.25, 101.0, 99.0, 100.0)];
        let exchange = Tick::resolve(Some(0.05), &bars).unwrap();
        assert_eq!(
            (exchange.size, exchange.source),
            (0.05, TickSource::Exchange)
        );
        for bad in [Some(0.0), Some(-1.0), Some(f64::NAN), None] {
            let inferred = Tick::resolve(bad, &bars).unwrap();
            assert_eq!(
                (inferred.size, inferred.source),
                (0.01, TickSource::Inferred)
            );
        }
        assert!(Tick::resolve(None, &[]).is_none());
    }

    #[test]
    fn the_automatic_row_is_a_fifth_of_the_atr_in_whole_ticks() {
        assert_eq!(auto_ticks_per_row(10.0, 0.5), Some(4)); // 0.2 * 10 / 0.5
        assert_eq!(auto_ticks_per_row(10.0, 0.01), Some(200));
        assert_eq!(
            auto_ticks_per_row(0.001, 1.0),
            Some(1),
            "never under one tick"
        );
        assert_eq!(auto_ticks_per_row(0.0, 1.0), None);
        assert_eq!(auto_ticks_per_row(f64::NAN, 1.0), None);
        assert_eq!(auto_ticks_per_row(1.0, 0.0), None);
    }
}
