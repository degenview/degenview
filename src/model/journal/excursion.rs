//! Maximum favourable and adverse excursion: how far a trade went for and against the trader
//! while it was open, estimated from candles.
//!
//! This is an estimate and says so. Candles only give each bar's high and low, so the figures are
//! bounds taken from complete bars. The rules, in order of what they refuse to do:
//!
//! - A bar that contains a fill is never used for its high or low, because it is unknown whether
//!   the extreme came before or after the fill (it could be a move the trade was not in). Its
//!   contribution is the fill price alone.
//! - Only bars that lie wholly between the first and the last fill count. With none, the answer is
//!   "unavailable", not a number built from the fills alone.
//! - The position is valued at its weighted-average cost, with what earlier exits realised added
//!   in, so scaling in and out is followed. (An account on FIFO or LIFO realises slightly
//!   different amounts along the way, and the same total at the end.)
//! - Gross of fees: fees are not a price excursion.
//!
//! Nothing here fetches anything.

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::accounting::from_f64;
use super::metric::{Metric, Reason};
use super::model::Trade;
use crate::model::Kline;

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Excursion {
    /// The best the trade's gross P&L got to, never below zero.
    pub mfe: Decimal,
    /// The worst it got to, as a positive amount, never below zero.
    pub mae: Decimal,
    /// The trade's gross result, for reference.
    pub gross: Decimal,
    /// How much of the best excursion the exit kept: gross / MFE. Negative when the trade ended
    /// at a loss after a profitable stretch. `None` when the trade was never profitable.
    pub exit_efficiency: Option<f64>,
    /// Bars that contributed their high or low.
    pub bars: usize,
    /// The candle interval, as the provider names it ("5m").
    pub interval: String,
}

impl Excursion {
    /// What the figures are: said wherever they are shown.
    pub fn provenance(&self) -> String {
        format!(
            "Estimated from {} {} candles between the fills; bars containing a fill are not used.",
            self.bars, self.interval
        )
    }
}

/// Excursion of a closed `trade`, from `candles` of `interval_ms` each (any order, any extent;
/// bars outside the hold are ignored). `interval` is only the label.
pub fn excursion(
    trade: &Trade,
    candles: &[Kline],
    interval: &str,
    interval_ms: i64,
) -> Metric<Excursion> {
    if trade.is_open() || trade.close_ts_ms.is_none() {
        return Metric::Unavailable(Reason::OpenTrade);
    }
    if candles.is_empty() || interval_ms <= 0 {
        return Metric::Unavailable(Reason::NoCandles);
    }
    let sign = trade.direction.sign();
    let mut legs: Vec<_> = trade.legs.iter().collect();
    legs.sort_by_key(|leg| leg.ts_ms);

    // The state of the position after each leg: (time, size, average cost, realised so far).
    struct State {
        ts: i64,
        size: Decimal,
        cost: Decimal,
        realised: Decimal,
    }
    let mut states = Vec::with_capacity(legs.len());
    let (mut size, mut cost, mut realised) = (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
    let mut best = Decimal::ZERO;
    let mut worst = Decimal::ZERO;
    for leg in &legs {
        if leg.is_entry {
            let total = size + leg.quantity;
            if total.is_zero() {
                continue;
            }
            cost = (cost * size + leg.price * leg.quantity) / total;
            size = total;
        } else {
            realised += sign * (leg.price - cost) * leg.quantity * trade.multiplier;
            size = (size - leg.quantity).max(Decimal::ZERO);
        }
        // The fill itself is a point on the path: valued at the fill price.
        let at_fill = realised + sign * (leg.price - cost) * size * trade.multiplier;
        best = best.max(at_fill);
        worst = worst.min(at_fill);
        states.push(State {
            ts: leg.ts_ms,
            size,
            cost,
            realised,
        });
    }
    let (Some(first), Some(last)) = (states.first().map(|s| s.ts), states.last().map(|s| s.ts))
    else {
        return Metric::Unavailable(Reason::NoCandles);
    };

    let mut used = 0usize;
    for bar in candles {
        let end = bar.open_time + interval_ms;
        if bar.open_time <= first || end > last {
            continue;
        }
        // A bar holding any fill is out: its extremes cannot be placed against the position.
        if states
            .iter()
            .any(|state| state.ts >= bar.open_time && state.ts < end)
        {
            continue;
        }
        let Some(state) = states.iter().rev().find(|state| state.ts < bar.open_time) else {
            continue;
        };
        let (Some(high), Some(low)) = (from_f64(bar.high), from_f64(bar.low)) else {
            continue;
        };
        if high < low {
            continue;
        }
        let (favourable, adverse) = if sign.is_sign_positive() {
            (high, low)
        } else {
            (low, high)
        };
        let value = |price: Decimal| {
            state.realised + sign * (price - state.cost) * state.size * trade.multiplier
        };
        best = best.max(value(favourable));
        worst = worst.min(value(adverse));
        used += 1;
    }
    if used == 0 {
        return Metric::Unavailable(Reason::NoCompleteBar);
    }
    let gross = trade.gross_pnl;
    let exit_efficiency = (best > Decimal::ZERO)
        .then(|| (gross / best).to_f64())
        .flatten()
        .filter(|value| value.is_finite());
    Metric::Value(Excursion {
        mfe: best,
        mae: -worst,
        gross,
        exit_efficiency,
        bars: used,
        interval: interval.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::journal::model::{Direction, Side, TradeLeg, TradeStatus};
    use crate::model::journal::testing::d;

    const MIN: i64 = 60_000;

    fn leg(ts: i64, side: Side, quantity: &str, price: &str, entry: bool, after: &str) -> TradeLeg {
        TradeLeg {
            exec_uid: format!("e{ts}"),
            ts_ms: ts,
            side,
            quantity: d(quantity),
            price: d(price),
            fee: Some(Decimal::ZERO),
            is_entry: entry,
            realized_gross: Decimal::ZERO,
            position_after: d(after),
        }
    }

    fn trade(direction: Direction, legs: Vec<TradeLeg>, gross: &str) -> Trade {
        Trade {
            key: "t".into(),
            account_uid: "a".into(),
            instrument: crate::model::journal::testing::instrument("BTC"),
            direction,
            status: TradeStatus::Closed,
            open_ts_ms: legs[0].ts_ms,
            close_ts_ms: Some(legs.last().unwrap().ts_ms),
            entry_quantity: d("1"),
            exit_quantity: d("1"),
            open_quantity: Decimal::ZERO,
            max_quantity: d("1"),
            avg_entry: legs[0].price,
            avg_exit: Some(legs.last().unwrap().price),
            multiplier: Decimal::ONE,
            gross_pnl: d(gross),
            fees: Decimal::ZERO,
            fees_known: true,
            open_fees: Decimal::ZERO,
            currency: Some("USD".into()),
            initial_stop: None,
            legs,
        }
    }

    fn bar(minute: i64, high: f64, low: f64) -> Kline {
        Kline {
            open_time: minute * MIN,
            close_time: minute * MIN + MIN - 1,
            open: low,
            high,
            low,
            close: high,
            volume: 1.0,
        }
    }

    fn long_trade() -> Trade {
        // Buy 1 at 100 at minute 0; sell 1 at 104 at minute 10.
        trade(
            Direction::Long,
            vec![
                leg(0, Side::Buy, "1", "100", true, "1"),
                leg(10 * MIN, Side::Sell, "1", "104", false, "0"),
            ],
            "4",
        )
    }

    #[test]
    fn a_long_trade_takes_its_best_and_worst_from_the_bars_between_the_fills() {
        let candles = [
            bar(0, 101.0, 99.0),  // holds the entry: not used
            bar(1, 103.0, 98.0),  // low 98 -> -2
            bar(2, 108.0, 102.0), // high 108 -> +8
            bar(9, 106.0, 103.0), // inside
            bar(10, 200.0, 1.0),  // holds the exit: not used
        ];
        let Metric::Value(result) = excursion(&long_trade(), &candles, "1m", MIN) else {
            panic!("available");
        };
        assert_eq!(result.mfe, d("8"));
        assert_eq!(result.mae, d("2"));
        assert_eq!(result.bars, 3);
        assert_eq!(result.exit_efficiency, Some(0.5));
        assert!(result.provenance().contains("1m"));
    }

    #[test]
    fn a_short_trade_is_mirrored() {
        // Sell 1 at 100, buy back at 97.
        let short = trade(
            Direction::Short,
            vec![
                leg(0, Side::Sell, "1", "100", true, "1"),
                leg(10 * MIN, Side::Buy, "1", "97", false, "0"),
            ],
            "3",
        );
        let candles = [bar(1, 102.0, 96.0), bar(2, 99.0, 95.0)];
        let Metric::Value(result) = excursion(&short, &candles, "1m", MIN) else {
            panic!("available");
        };
        assert_eq!(result.mfe, d("5"));
        assert_eq!(result.mae, d("2"));
    }

    #[test]
    fn scaling_in_follows_the_changing_size_and_cost() {
        // Buy 1 at 100 (min 0), buy 1 more at 110 (min 5): size 2, cost 105. Sell 2 at 112 (min 10).
        let scaled = trade(
            Direction::Long,
            vec![
                leg(0, Side::Buy, "1", "100", true, "1"),
                leg(5 * MIN, Side::Buy, "1", "110", true, "2"),
                leg(10 * MIN, Side::Sell, "2", "112", false, "0"),
            ],
            "14",
        );
        // Bar 2 (size 1, cost 100): high 108 -> +8. Bar 7 (size 2, cost 105): low 101 -> -8.
        let candles = [bar(2, 108.0, 100.0), bar(7, 113.0, 101.0)];
        let Metric::Value(result) = excursion(&scaled, &candles, "1m", MIN) else {
            panic!("available");
        };
        // Bar 7 high 113: (113 - 105) * 2 = 16.
        assert_eq!(result.mfe, d("16"));
        assert_eq!(result.mae, d("8"));
        assert_eq!(result.exit_efficiency, Some(14.0 / 16.0));
    }

    #[test]
    fn a_partial_exit_realises_before_the_rest_runs() {
        // Buy 2 at 100; sell 1 at 106 (min 4); the last 1 runs and exits at 103 (min 10).
        let t = trade(
            Direction::Long,
            vec![
                leg(0, Side::Buy, "2", "100", true, "2"),
                leg(4 * MIN, Side::Sell, "1", "106", false, "1"),
                leg(10 * MIN, Side::Sell, "1", "103", false, "0"),
            ],
            "9",
        );
        // After the first exit: realised 6, size 1. Bar at min 6 high 108 -> 6 + 8 = 14.
        let candles = [bar(6, 108.0, 102.0)];
        let Metric::Value(result) = excursion(&t, &candles, "1m", MIN) else {
            panic!("available");
        };
        assert_eq!(result.mfe, d("14"));
        assert_eq!(result.mae, Decimal::ZERO);
    }

    #[test]
    fn the_unavailable_cases_say_why() {
        let t = long_trade();
        assert_eq!(
            excursion(&t, &[], "1m", MIN),
            Metric::Unavailable(Reason::NoCandles)
        );
        // Only bars holding a fill, or outside the hold.
        let candles = [
            bar(0, 101.0, 99.0),
            bar(10, 105.0, 103.0),
            bar(30, 1.0, 1.0),
        ];
        assert_eq!(
            excursion(&t, &candles, "1m", MIN),
            Metric::Unavailable(Reason::NoCompleteBar)
        );
        let mut open = long_trade();
        open.status = TradeStatus::Open;
        open.close_ts_ms = None;
        assert_eq!(
            excursion(&open, &candles, "1m", MIN),
            Metric::Unavailable(Reason::OpenTrade)
        );
    }

    #[test]
    fn a_trade_that_never_profited_has_no_efficiency_and_bad_bars_are_skipped() {
        let loser = trade(
            Direction::Long,
            vec![
                leg(0, Side::Buy, "1", "100", true, "1"),
                leg(10 * MIN, Side::Sell, "1", "95", false, "0"),
            ],
            "-5",
        );
        let candles = [
            bar(1, 99.0, 94.0),
            Kline {
                high: f64::NAN,
                ..bar(2, 0.0, 0.0)
            },
            Kline {
                high: 90.0,
                low: 99.0,
                ..bar(3, 0.0, 0.0)
            },
        ];
        let Metric::Value(result) = excursion(&loser, &candles, "1m", MIN) else {
            panic!("available");
        };
        assert_eq!(result.mfe, Decimal::ZERO);
        assert_eq!(result.mae, d("6"));
        assert_eq!(result.exit_efficiency, None);
        assert_eq!(result.bars, 1);
    }
}
