//! Risk and R-multiples.
//!
//! **Initial risk** of a trade is `Σ entry_qty × |entry_price − stop| × multiplier` over every
//! entry leg: the money lost if the whole position, at the prices it was actually built at, were
//! stopped out at the trade's one stop. Scaling in therefore raises the denominator in proportion
//! to what was risked at each entry. The stop is the one the user entered, else the one the
//! source attached to the entry order (`Trade::initial_stop`); a stop is never guessed.
//!
//! **Realised R** is net P&L divided by initial risk, for closed trades only.

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::metric::{Metric, Reason};
use super::model::{Direction, Trade};

/// `stop` overrides the trade's own stop (a review's manual entry).
pub fn initial_risk(trade: &Trade, stop: Option<Decimal>) -> Metric<Decimal> {
    let Some(stop) = stop.or(trade.initial_stop) else {
        return Metric::Unavailable(Reason::MissingStop);
    };
    if stop <= Decimal::ZERO {
        return Metric::Unavailable(Reason::InvalidStop);
    }
    let mut risk = Decimal::ZERO;
    for leg in trade.legs.iter().filter(|leg| leg.is_entry) {
        let distance = match trade.direction {
            Direction::Long => leg.price - stop,
            Direction::Short => stop - leg.price,
        };
        if distance <= Decimal::ZERO {
            return Metric::Unavailable(Reason::InvalidStop);
        }
        risk += leg.quantity * distance * trade.multiplier;
    }
    if risk <= Decimal::ZERO {
        return Metric::Unavailable(Reason::InvalidStop);
    }
    Metric::Value(risk)
}

pub fn r_multiple(trade: &Trade, stop: Option<Decimal>) -> Metric<Decimal> {
    if trade.is_open() {
        return Metric::Unavailable(Reason::OpenTrade);
    }
    initial_risk(trade, stop).map(|risk| trade.net_pnl() / risk)
}

/// The planned reward-to-risk ratio: distance to the target over distance to the stop, from the
/// planned entry (else the average fill). `None` without a target and a valid stop; never a guess.
pub fn planned_reward_risk(
    trade: &Trade,
    planned_entry: Option<Decimal>,
    stop: Option<Decimal>,
    target: Option<Decimal>,
) -> Option<f64> {
    let entry = planned_entry
        .filter(|p| *p > Decimal::ZERO)
        .unwrap_or(trade.avg_entry);
    let stop = stop.or(trade.initial_stop)?;
    let target = target?;
    let (risk, reward) = match trade.direction {
        Direction::Long => (entry - stop, target - entry),
        Direction::Short => (stop - entry, entry - target),
    };
    if risk <= Decimal::ZERO || reward <= Decimal::ZERO {
        return None;
    }
    (reward / risk).to_f64().filter(|r| r.is_finite())
}

/// Initial risk as a fraction of an account balance; `None` without a positive balance.
pub fn risk_fraction(initial_risk: Decimal, balance: Option<Decimal>) -> Option<f64> {
    let balance = balance.filter(|b| *b > Decimal::ZERO)?;
    (initial_risk / balance).to_f64().filter(|r| r.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::journal::model::{CostBasis, Side};
    use crate::model::journal::reconstruct::reconstruct;
    use crate::model::journal::testing::{d, exec};

    fn trade(executions: &[crate::model::journal::model::Execution]) -> Trade {
        reconstruct(executions, |_| CostBasis::AverageCost)
            .into_iter()
            .next()
            .unwrap()
    }

    #[test]
    fn r_is_net_pnl_over_initial_risk() {
        let t = trade(&[
            exec("1", Side::Buy, "2", "100", 1_000).stop("95").fee("1"),
            exec("2", Side::Sell, "2", "110", 2_000).fee("1"),
        ]);
        // risk 2 * 5 = 10; net 20 - 2 = 18
        assert_eq!(initial_risk(&t, None), Metric::Value(d("10")));
        assert_eq!(r_multiple(&t, None), Metric::Value(d("1.8")));
    }

    #[test]
    fn a_short_risks_the_distance_above_its_entry() {
        let t = trade(&[
            exec("1", Side::Sell, "1", "100", 1_000).stop("104"),
            exec("2", Side::Buy, "1", "92", 2_000),
        ]);
        assert_eq!(r_multiple(&t, None), Metric::Value(d("2")));
    }

    #[test]
    fn scaling_in_adds_the_risk_taken_at_each_entry() {
        let t = trade(&[
            exec("1", Side::Buy, "1", "100", 1_000).stop("95"),
            exec("2", Side::Buy, "1", "102", 1_500),
            exec("3", Side::Sell, "2", "110", 2_000),
        ]);
        // 1*5 + 1*7
        assert_eq!(initial_risk(&t, None), Metric::Value(d("12")));
    }

    #[test]
    fn a_multiplier_scales_risk_and_pnl_alike() {
        let t = trade(&[
            exec("1", Side::Buy, "1", "100", 1_000)
                .stop("99")
                .times("10"),
            exec("2", Side::Sell, "1", "102", 2_000).times("10"),
        ]);
        assert_eq!(initial_risk(&t, None), Metric::Value(d("10")));
        assert_eq!(r_multiple(&t, None), Metric::Value(d("2")));
    }

    #[test]
    fn a_manual_stop_overrides_the_attached_one() {
        let t = trade(&[
            exec("1", Side::Buy, "1", "100", 1_000).stop("95"),
            exec("2", Side::Sell, "1", "110", 2_000),
        ]);
        assert_eq!(r_multiple(&t, Some(d("90"))), Metric::Value(d("1")));
    }

    #[test]
    fn no_stop_means_no_risk_not_zero_risk() {
        let t = trade(&[
            exec("1", Side::Buy, "1", "100", 1_000),
            exec("2", Side::Sell, "1", "110", 2_000),
        ]);
        assert_eq!(
            initial_risk(&t, None),
            Metric::Unavailable(Reason::MissingStop)
        );
        assert_eq!(
            r_multiple(&t, None),
            Metric::Unavailable(Reason::MissingStop)
        );
    }

    #[test]
    fn a_stop_on_the_wrong_side_is_invalid() {
        let t = trade(&[
            exec("1", Side::Buy, "1", "100", 1_000),
            exec("2", Side::Sell, "1", "110", 2_000),
        ]);
        for stop in ["100", "105", "0"] {
            assert_eq!(
                initial_risk(&t, Some(d(stop))),
                Metric::Unavailable(Reason::InvalidStop),
                "{stop}"
            );
        }
        // Scaling in above the stop's wrong side invalidates the whole trade's risk.
        let scaled = trade(&[
            exec("1", Side::Buy, "1", "100", 1_000).stop("101"),
            exec("2", Side::Sell, "1", "110", 2_000),
        ]);
        assert_eq!(
            initial_risk(&scaled, None),
            Metric::Unavailable(Reason::InvalidStop)
        );
    }

    #[test]
    fn an_open_trade_has_risk_but_no_r() {
        let t = trade(&[exec("1", Side::Buy, "1", "100", 1_000).stop("95")]);
        assert_eq!(initial_risk(&t, None), Metric::Value(d("5")));
        assert_eq!(r_multiple(&t, None), Metric::Unavailable(Reason::OpenTrade));
    }

    #[test]
    fn a_losing_trade_has_negative_r() {
        let t = trade(&[
            exec("1", Side::Buy, "1", "100", 1_000).stop("95"),
            exec("2", Side::Sell, "1", "95", 2_000),
        ]);
        assert_eq!(r_multiple(&t, None), Metric::Value(d("-1")));
    }

    #[test]
    fn planned_reward_risk_reads_the_plan_not_the_fill() {
        let long = trade(&[
            exec("1", Side::Buy, "1", "101", 1_000).stop("95"),
            exec("2", Side::Sell, "1", "110", 2_000),
        ]);
        // From the fill 101: risk 6, reward 9 -> 1.5. From a planned 100: risk 5, reward 10 -> 2.
        assert_eq!(
            planned_reward_risk(&long, None, None, Some(d("110"))),
            Some(1.5)
        );
        assert_eq!(
            planned_reward_risk(&long, Some(d("100")), None, Some(d("110"))),
            Some(2.0)
        );
        assert_eq!(
            planned_reward_risk(&long, Some(d("100")), Some(d("90")), Some(d("130"))),
            Some(3.0)
        );
        // No target, a target on the wrong side, or no stop: no ratio.
        assert_eq!(planned_reward_risk(&long, None, None, None), None);
        assert_eq!(planned_reward_risk(&long, None, None, Some(d("100"))), None);
        let no_stop = trade(&[
            exec("1", Side::Buy, "1", "100", 1_000),
            exec("2", Side::Sell, "1", "110", 2_000),
        ]);
        assert_eq!(
            planned_reward_risk(&no_stop, None, None, Some(d("120"))),
            None
        );
        let short = trade(&[
            exec("1", Side::Sell, "1", "100", 1_000),
            exec("2", Side::Buy, "1", "90", 2_000),
        ]);
        assert_eq!(
            planned_reward_risk(&short, None, Some(d("104")), Some(d("88"))),
            Some(3.0)
        );
    }

    #[test]
    fn risk_as_a_share_of_the_account_needs_a_balance() {
        assert_eq!(risk_fraction(d("50"), Some(d("1000"))), Some(0.05));
        assert_eq!(risk_fraction(d("50"), None), None);
        assert_eq!(risk_fraction(d("50"), Some(d("0"))), None);
        assert_eq!(risk_fraction(d("50"), Some(d("-5"))), None);
    }
}
