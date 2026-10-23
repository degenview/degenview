//! The Long and Short position drawings: the risk, reward and size of a **hypothetical** trade.
//!
//! A position drawing is an analytical tool. It never places, changes or cancels an order, and
//! nothing here knows of the paper-trading engine: it turns an entry, a stop, a target and the
//! settings into numbers, and reads the bars the chart draws to say whether the price has since
//! reached the target or the stop.
//!
//! The formulas are the ones on TradingView's page "How to use long and short position drawing
//! tools". Everything is worked from the stored prices (never pixels), the direction of the trade is
//! carried by [`Side`] and not by sign tricks, and a configuration that makes no sense (a stop on
//! the wrong side of the entry, a zero distance) reports `None` for what it cannot say instead of a
//! number that would mislead. Pure: no egui types.

use super::kline::Kline;
use serde::{Deserialize, Serialize};

/// Which way the trade is taken.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Long,
    Short,
}

impl Side {
    /// `+1` for a long, `-1` for a short: the direction a profit lies in.
    pub const fn sign(self) -> f64 {
        match self {
            Self::Long => 1.0,
            Self::Short => -1.0,
        }
    }
}

/// The settings a position is sized by. Account and risk are in the account's currency.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sizing {
    pub account: f64,
    /// The smallest amount of the instrument traded: a quantity is in lots of this.
    pub lot: f64,
    /// Risked per trade: a percentage of the account when `risk_is_percent`, else an amount.
    pub risk: f64,
    pub risk_is_percent: bool,
    pub leverage: f64,
    /// What one unit of price movement is worth per lot: `1` for an instrument priced in the
    /// currency it is traded in (no contract multiplier is known for any market here).
    pub point_value: f64,
    /// Decimal places the quantity is rounded to.
    pub qty_precision: u8,
}

/// What a position works out to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stats {
    /// How far the target is in the direction of profit, in price (negative when it is on the
    /// losing side of the entry).
    pub target_distance: f64,
    /// The same as a percentage of the entry price.
    pub target_percent: Option<f64>,
    /// How far the stop is in the direction of loss, in price (negative when it is on the
    /// winning side).
    pub stop_distance: f64,
    pub stop_percent: Option<f64>,
    /// Reward over risk: the target's distance over the stop's. `None` unless the stop is on the
    /// losing side of the entry (there is no risk to divide by otherwise).
    pub risk_reward: Option<f64>,
    /// The amount at risk.
    pub risk_size: f64,
    /// The quantity in lots, the smaller of what the risk and the leverage allow, rounded to the
    /// precision. `None` when the stop is not on the losing side or a setting is not usable.
    pub quantity: Option<f64>,
    /// The profit at the target and the loss at the stop (negative), in the account's currency.
    pub profit: Option<f64>,
    pub loss: Option<f64>,
    /// The account after the target and after the stop.
    pub balance_at_target: Option<f64>,
    pub balance_at_stop: Option<f64>,
}

/// `value` when it is a finite number.
fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

/// Rounds to `places` decimals (at most 8).
fn round_to(value: f64, places: u8) -> f64 {
    let scale = 10f64.powi(i32::from(places.min(8)));
    (value * scale).round() / scale
}

/// What a `side` position with these prices and `sizing` works out to.
///
/// - Reward over risk: `(target − entry) / (entry − stop)` for a long, the same with the entry
///   first for a short.
/// - Quantity: `min(risk quantity, leverage quantity)`, where the risk quantity is
///   `risk size / (stop distance × point value) / lot` and the leverage quantity
///   `account × leverage / (entry × point value) / lot`. (TradingView's page words the second as
///   "divided by entry price, times point value, divided by lot size"; with a point value of one
///   the two readings agree, and this one keeps the unit of a quantity.)
/// - Profit at the target `distance × quantity × point value × lot`, the loss at the stop the same
///   with its distance, and the balances add them to the account.
pub fn stats(side: Side, entry: f64, stop: f64, target: f64, sizing: &Sizing) -> Stats {
    let sign = side.sign();
    let target_distance = sign * (target - entry);
    let stop_distance = sign * (entry - stop);
    let percent = |distance: f64| {
        (entry != 0.0 && entry.is_finite())
            .then(|| distance / entry.abs() * 100.0)
            .and_then(finite)
    };
    let risk_size = if sizing.risk_is_percent {
        sizing.account * sizing.risk / 100.0
    } else {
        sizing.risk
    };
    let usable = |value: f64| value.is_finite() && value > 0.0;
    // The risk quantity needs somewhere for the stop to be hit.
    let by_risk = (stop_distance > 0.0 && usable(sizing.point_value) && usable(sizing.lot))
        .then(|| risk_size / (stop_distance * sizing.point_value) / sizing.lot)
        .and_then(finite)
        .filter(|quantity| *quantity >= 0.0);
    let by_leverage = (usable(entry.abs())
        && usable(sizing.leverage)
        && usable(sizing.point_value)
        && usable(sizing.lot))
    .then(|| sizing.account * sizing.leverage / (entry.abs() * sizing.point_value) / sizing.lot)
    .and_then(finite)
    .filter(|quantity| *quantity >= 0.0);
    let quantity = match (by_risk, by_leverage) {
        (Some(risk), Some(leverage)) => Some(risk.min(leverage)),
        _ => None,
    }
    .map(|quantity| round_to(quantity, sizing.qty_precision));
    let money = |distance: f64| {
        quantity.and_then(|quantity| finite(distance * quantity * sizing.point_value * sizing.lot))
    };
    let profit = money(target_distance);
    let loss = money(-stop_distance);
    Stats {
        target_distance,
        target_percent: percent(target_distance),
        stop_distance,
        stop_percent: percent(stop_distance),
        risk_reward: (stop_distance > 0.0)
            .then(|| target_distance / stop_distance)
            .and_then(finite),
        risk_size,
        quantity,
        profit,
        loss,
        balance_at_target: profit.map(|profit| sizing.account + profit),
        balance_at_stop: loss.map(|loss| sizing.account + loss),
    }
}

/// Where the price went after the entry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    /// Neither level has been reached by the last bar: `last` is the last close, which the open
    /// profit or loss is measured to.
    Open { last: f64 },
    /// The target was reached, at this bar (an index into the bars given).
    Target { bar: usize },
    /// The stop was reached, at this bar.
    Stop { bar: usize },
}

/// Whether the price reached the target or the stop between the bar after `from` and the bar `to`
/// (both indexes into `bars`, `to` clamped to the last), whichever came first.
///
/// A bar that reaches both is counted as hitting the stop (**inferred**: the bars cannot say which
/// came first, and the cautious reading is the one a trader would not be surprised by). The bar the
/// position starts on is not examined: it holds the entry. Nothing is read from a bar the chart
/// does not have, so during a replay it is whatever the replay has shown. `None` when there are no
/// bars.
pub fn outcome(
    side: Side,
    stop: f64,
    target: f64,
    bars: &[Kline],
    from: usize,
    to: usize,
) -> Option<Outcome> {
    let last_bar = bars.last()?;
    let end = to.min(bars.len() - 1);
    for (index, bar) in bars
        .iter()
        .enumerate()
        .take(end + 1)
        .skip(from.saturating_add(1))
    {
        let (hit_stop, hit_target) = match side {
            Side::Long => (bar.low <= stop, bar.high >= target),
            Side::Short => (bar.high >= stop, bar.low <= target),
        };
        if hit_stop {
            return Some(Outcome::Stop { bar: index });
        }
        if hit_target {
            return Some(Outcome::Target { bar: index });
        }
    }
    Some(Outcome::Open {
        last: last_bar.close,
    })
}

/// How a position forecast turned out.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForecastResult {
    /// The price reached the target.
    Success,
    /// The time to the target has passed and the price never did.
    Failure,
}

/// Whether a forecast from `source` (a price at bar `from`) to `target` (at bar `to`) came true,
/// reading the bars after `from` up to `to` (both indexes into `bars`).
///
/// It is a success the moment a bar reaches the target (its high for a target above the source, its
/// low for one below), a failure once the bars have run past `to` without one, and `None` while
/// either could still happen: the target time is in the future, or there are no bars. TradingView
/// says only that the forecast "will automatically be deemed a success or a failure"; the rule is
/// **inferred**. A target level with the source has no direction and no result.
pub fn forecast_result(
    source: f64,
    target: f64,
    bars: &[Kline],
    from: usize,
    to: usize,
) -> Option<ForecastResult> {
    if bars.is_empty() || !(source.is_finite() && target.is_finite()) || target == source {
        return None;
    }
    let up = target > source;
    let last = bars.len() - 1;
    for bar in bars
        .iter()
        .take(to.min(last) + 1)
        .skip(from.saturating_add(1))
    {
        let reached = if up {
            bar.high >= target
        } else {
            bar.low <= target
        };
        if reached {
            return Some(ForecastResult::Success);
        }
    }
    (to > from && last >= to).then_some(ForecastResult::Failure)
}

/// The profit or loss at `price` (negative for a loss), for the open position and the closed one.
pub fn profit_at(
    side: Side,
    entry: f64,
    price: f64,
    quantity: f64,
    sizing: &Sizing,
) -> Option<f64> {
    finite(side.sign() * (price - entry) * quantity * sizing.point_value * sizing.lot)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sizing() -> Sizing {
        Sizing {
            account: 1_000.0,
            lot: 1.0,
            risk: 25.0,
            risk_is_percent: true,
            leverage: 1.0,
            point_value: 1.0,
            qty_precision: 2,
        }
    }

    fn bar(low: f64, high: f64, close: f64) -> Kline {
        Kline {
            open_time: 0,
            close_time: 0,
            open: close,
            high,
            low,
            close,
            volume: 1.0,
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn a_long_with_a_two_to_one_reward_works_out_by_hand() {
        // Entry 100, stop 95 (risk 5), target 110 (reward 10). Account 1,000, risking 25 %: 250.
        // Risk quantity 250 / 5 = 50; leverage quantity 1,000 / 100 = 10: the smaller is 10.
        let stats = stats(Side::Long, 100.0, 95.0, 110.0, &sizing());
        assert!(close(stats.target_distance, 10.0));
        assert!(close(stats.stop_distance, 5.0));
        assert!(close(stats.target_percent.unwrap(), 10.0));
        assert!(close(stats.stop_percent.unwrap(), 5.0));
        assert!(close(stats.risk_reward.unwrap(), 2.0));
        assert!(close(stats.risk_size, 250.0));
        assert!(close(stats.quantity.unwrap(), 10.0));
        // 10 lots: +100 at the target, -50 at the stop.
        assert!(close(stats.profit.unwrap(), 100.0));
        assert!(close(stats.loss.unwrap(), -50.0));
        assert!(close(stats.balance_at_target.unwrap(), 1_100.0));
        assert!(close(stats.balance_at_stop.unwrap(), 950.0));
    }

    #[test]
    fn leverage_lifts_the_cap_on_the_quantity_and_the_risk_sets_it_when_smaller() {
        // The same trade at 10 x leverage: the leverage quantity is 100, so the risk quantity of 50
        // is the smaller.
        let mut sizing = sizing();
        sizing.leverage = 10.0;
        let stats = stats(Side::Long, 100.0, 95.0, 110.0, &sizing);
        assert!(close(stats.quantity.unwrap(), 50.0));
        assert!(close(stats.profit.unwrap(), 500.0));
        assert!(close(stats.loss.unwrap(), -250.0), "the whole risk, lost");
    }

    #[test]
    fn a_short_is_the_mirror_of_a_long() {
        // Entry 100, stop 105, target 90: the same distances the other way.
        let short = stats(Side::Short, 100.0, 105.0, 90.0, &sizing());
        let long = stats(Side::Long, 100.0, 95.0, 110.0, &sizing());
        assert!(close(short.target_distance, 10.0));
        assert!(close(short.stop_distance, 5.0));
        assert!(close(short.risk_reward.unwrap(), 2.0));
        assert_eq!(short.quantity, long.quantity);
        assert!(close(short.profit.unwrap(), long.profit.unwrap()));
        assert!(close(short.loss.unwrap(), long.loss.unwrap()));
        assert!(close(short.balance_at_target.unwrap(), 1_100.0));
    }

    #[test]
    fn a_short_loses_when_the_price_rises_and_a_long_when_it_falls() {
        let sizing = sizing();
        assert!(close(
            profit_at(Side::Long, 100.0, 103.0, 10.0, &sizing).unwrap(),
            30.0
        ));
        assert!(close(
            profit_at(Side::Long, 100.0, 97.0, 10.0, &sizing).unwrap(),
            -30.0
        ));
        assert!(close(
            profit_at(Side::Short, 100.0, 103.0, 10.0, &sizing).unwrap(),
            -30.0
        ));
        assert!(close(
            profit_at(Side::Short, 100.0, 97.0, 10.0, &sizing).unwrap(),
            30.0
        ));
    }

    #[test]
    fn a_risk_amount_is_used_as_it_is_and_the_point_value_and_lot_scale_the_money() {
        // Risk a flat 100 on a 5 stop: 20 contracts at one lot; with lots of 0.5 it is 40 lots,
        // and each lot is worth half a unit per price point (point value 2 doubles the move).
        let mut sizing = sizing();
        sizing.risk = 100.0;
        sizing.risk_is_percent = false;
        sizing.leverage = 100.0;
        let plain = stats(Side::Long, 100.0, 95.0, 110.0, &sizing);
        assert!(close(plain.risk_size, 100.0));
        assert!(close(plain.quantity.unwrap(), 20.0));
        assert!(close(plain.loss.unwrap(), -100.0));
        sizing.lot = 0.5;
        sizing.point_value = 2.0;
        let scaled = stats(Side::Long, 100.0, 95.0, 110.0, &sizing);
        // 100 / (5 x 2) / 0.5 = 20 lots; each lot is 0.5 x 2 = 1 per point: the loss is 5 x 20.
        assert!(close(scaled.quantity.unwrap(), 20.0));
        assert!(
            close(scaled.loss.unwrap(), -100.0),
            "the risk is still the risk"
        );
    }

    #[test]
    fn the_quantity_is_rounded_to_its_precision() {
        let mut sizing = sizing();
        sizing.risk = 10.0; // 100 at risk on a 3 stop: 33.33...
        sizing.leverage = 100.0;
        sizing.qty_precision = 2;
        let two = stats(Side::Long, 100.0, 97.0, 103.0, &sizing);
        assert_eq!(two.quantity, Some(33.33));
        sizing.qty_precision = 0;
        let none = stats(Side::Long, 100.0, 97.0, 103.0, &sizing);
        assert_eq!(none.quantity, Some(33.0));
    }

    #[test]
    fn a_stop_on_the_wrong_side_has_no_risk_to_size() {
        // A long with its stop above the entry: no reward over risk, no quantity, no money.
        let stats = stats(Side::Long, 100.0, 105.0, 110.0, &sizing());
        assert!(close(stats.stop_distance, -5.0));
        assert_eq!(stats.risk_reward, None);
        assert_eq!(stats.quantity, None);
        assert_eq!((stats.profit, stats.loss), (None, None));
        assert_eq!(stats.balance_at_target, None);
        // The distances are still honest.
        assert!(close(stats.target_distance, 10.0));
    }

    #[test]
    fn a_target_on_the_wrong_side_is_a_loss_not_an_error() {
        // A long whose target is below the entry: reward over risk is negative and the "profit"
        // at the target is a loss.
        let stats = stats(Side::Long, 100.0, 95.0, 90.0, &sizing());
        assert!(close(stats.target_distance, -10.0));
        assert!(close(stats.risk_reward.unwrap(), -2.0));
        assert!(stats.profit.unwrap() < 0.0);
    }

    #[test]
    fn nothing_is_nan_or_infinite_whatever_the_inputs() {
        let degenerate = [
            (100.0, 100.0, 100.0),
            (0.0, 0.0, 0.0),
            (100.0, f64::NAN, 110.0),
            (f64::INFINITY, 95.0, 110.0),
            (-100.0, -95.0, -110.0),
            (1e-300, 0.0, 1e300),
        ];
        for (entry, stop, target) in degenerate {
            for side in [Side::Long, Side::Short] {
                let s = stats(side, entry, stop, target, &sizing());
                for value in [
                    s.target_percent,
                    s.stop_percent,
                    s.risk_reward,
                    s.quantity,
                    s.profit,
                    s.loss,
                    s.balance_at_target,
                    s.balance_at_stop,
                ]
                .into_iter()
                .flatten()
                {
                    assert!(value.is_finite(), "{side:?} {entry} {stop} {target}");
                }
            }
        }
    }

    #[test]
    fn a_zero_distance_stop_has_no_reward_over_risk() {
        let stats = stats(Side::Long, 100.0, 100.0, 110.0, &sizing());
        assert_eq!(stats.risk_reward, None);
        assert_eq!(stats.quantity, None);
    }

    #[test]
    fn unusable_settings_give_no_quantity() {
        for change in [
            |s: &mut Sizing| s.lot = 0.0,
            |s: &mut Sizing| s.leverage = 0.0,
            |s: &mut Sizing| s.point_value = -1.0,
            |s: &mut Sizing| s.account = f64::NAN,
        ] {
            let mut sizing = sizing();
            change(&mut sizing);
            let stats = stats(Side::Long, 100.0, 95.0, 110.0, &sizing);
            assert_eq!(stats.quantity, None);
        }
        // Zero risk is a valid setting: a quantity of nothing.
        let mut none = sizing();
        none.risk = 0.0;
        assert_eq!(
            stats(Side::Long, 100.0, 95.0, 110.0, &none).quantity,
            Some(0.0)
        );
    }

    #[test]
    fn the_first_level_reached_after_the_entry_bar_decides() {
        // A long from 100, stop 95, target 110, entered on bar 0.
        let bars = [
            bar(99.0, 101.0, 100.0), // the entry bar: not examined even though it dips under 100
            bar(98.0, 104.0, 103.0),
            bar(100.0, 111.0, 110.0), // reaches the target
            bar(90.0, 100.0, 92.0),   // and later the stop: too late
        ];
        assert_eq!(
            outcome(Side::Long, 95.0, 110.0, &bars, 0, 3),
            Some(Outcome::Target { bar: 2 })
        );
        // The stop first when the bars say so.
        let falling = [
            bar(99.0, 101.0, 100.0),
            bar(94.0, 100.0, 95.0),
            bar(100.0, 112.0, 111.0),
        ];
        assert_eq!(
            outcome(Side::Long, 95.0, 110.0, &falling, 0, 2),
            Some(Outcome::Stop { bar: 1 })
        );
    }

    #[test]
    fn a_bar_that_reaches_both_counts_as_the_stop() {
        let bars = [bar(99.0, 101.0, 100.0), bar(90.0, 115.0, 100.0)];
        assert_eq!(
            outcome(Side::Long, 95.0, 110.0, &bars, 0, 1),
            Some(Outcome::Stop { bar: 1 })
        );
    }

    #[test]
    fn a_short_reaches_its_target_below_and_its_stop_above() {
        let bars = [
            bar(99.0, 101.0, 100.0),
            bar(96.0, 103.0, 97.0),
            bar(89.0, 98.0, 90.0),
        ];
        assert_eq!(
            outcome(Side::Short, 105.0, 90.0, &bars, 0, 2),
            Some(Outcome::Target { bar: 2 })
        );
        let rising = [bar(99.0, 101.0, 100.0), bar(99.0, 106.0, 105.0)];
        assert_eq!(
            outcome(Side::Short, 105.0, 90.0, &rising, 0, 1),
            Some(Outcome::Stop { bar: 1 })
        );
    }

    #[test]
    fn a_position_nothing_has_reached_is_open_at_the_last_close() {
        let bars = [
            bar(99.0, 101.0, 100.0),
            bar(98.0, 104.0, 103.0),
            bar(100.0, 106.0, 104.5),
        ];
        assert_eq!(
            outcome(Side::Long, 95.0, 110.0, &bars, 0, 2),
            Some(Outcome::Open { last: 104.5 })
        );
        // The right edge of the drawing past the last bar looks no further than the bars go.
        assert_eq!(
            outcome(Side::Long, 95.0, 110.0, &bars, 0, 99),
            Some(Outcome::Open { last: 104.5 })
        );
        // A position drawn in the future has no bar to examine: open at the last close.
        assert_eq!(
            outcome(Side::Long, 95.0, 110.0, &bars, 50, 60),
            Some(Outcome::Open { last: 104.5 })
        );
        assert_eq!(outcome(Side::Long, 95.0, 110.0, &[], 0, 5), None);
    }

    #[test]
    fn only_the_bars_given_are_read() {
        // Hands the function the bars a replay has shown: the target the full data reaches on
        // bar 3 is not found in the first three.
        let all = [
            bar(99.0, 101.0, 100.0),
            bar(98.0, 104.0, 103.0),
            bar(100.0, 106.0, 104.5),
            bar(104.0, 120.0, 118.0),
        ];
        assert_eq!(
            outcome(Side::Long, 95.0, 110.0, &all[..3], 0, 10),
            Some(Outcome::Open { last: 104.5 })
        );
        assert_eq!(
            outcome(Side::Long, 95.0, 110.0, &all, 0, 10),
            Some(Outcome::Target { bar: 3 })
        );
    }

    #[test]
    fn a_forecast_succeeds_when_a_bar_reaches_the_target_and_fails_when_time_runs_out() {
        let bars = [
            bar(99.0, 101.0, 100.0),
            bar(98.0, 103.0, 102.0),
            bar(100.0, 108.0, 107.0),
            bar(104.0, 112.0, 110.0),
        ];
        // From 100 to 106 by bar 3: bar 2 reaches 108 first.
        assert_eq!(
            forecast_result(100.0, 106.0, &bars, 0, 3),
            Some(ForecastResult::Success)
        );
        // To 115: no bar gets there, and bar 3 is the target bar: a failure.
        assert_eq!(
            forecast_result(100.0, 115.0, &bars, 0, 3),
            Some(ForecastResult::Failure)
        );
        // To 115 with the target bar still to come: neither yet.
        assert_eq!(forecast_result(100.0, 115.0, &bars, 0, 9), None);
        // A target below the source is reached by a low.
        assert_eq!(
            forecast_result(100.0, 98.5, &bars, 0, 3),
            Some(ForecastResult::Success)
        );
        assert_eq!(
            forecast_result(100.0, 90.0, &bars, 0, 3),
            Some(ForecastResult::Failure)
        );
    }

    #[test]
    fn a_forecast_with_nothing_to_judge_has_no_result() {
        let bars = [bar(99.0, 101.0, 100.0), bar(98.0, 103.0, 102.0)];
        assert_eq!(
            forecast_result(100.0, 100.0, &bars, 0, 1),
            None,
            "no direction"
        );
        assert_eq!(forecast_result(100.0, 110.0, &[], 0, 1), None);
        assert_eq!(forecast_result(f64::NAN, 110.0, &bars, 0, 1), None);
        // No time between the two: nothing to judge by.
        assert_eq!(forecast_result(100.0, 110.0, &bars, 1, 1), None);
        // Wholly in the future.
        assert_eq!(forecast_result(100.0, 110.0, &bars, 5, 9), None);
    }

    #[test]
    fn a_forecast_reads_only_the_bars_it_is_given() {
        let all = [
            bar(99.0, 101.0, 100.0),
            bar(98.0, 103.0, 102.0),
            bar(100.0, 120.0, 118.0),
        ];
        // A replay that has shown two bars cannot know the third reaches 110.
        assert_eq!(forecast_result(100.0, 110.0, &all[..2], 0, 2), None);
        assert_eq!(
            forecast_result(100.0, 110.0, &all, 0, 2),
            Some(ForecastResult::Success)
        );
    }
}
