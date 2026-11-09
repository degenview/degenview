//! The realised-equity curve and drawdown.
//!
//! The curve steps at every trade **close**: cumulative net P&L of closed trades, plus the
//! account's initial balance when known. It is *realised* equity, not account equity: open
//! positions' unrealised P&L is not in it and no cash movements exist for paper accounts.
//!
//! Drawdown is measured from the running high-water mark of cumulative P&L, which starts at zero
//! (the start of the record). A percentage is `drawdown / (initial balance + peak P&L)` and exists
//! only when an initial balance is known and positive (a zero or negative start has no meaningful
//! percentage).

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::metric::{Metric, Reason};
use super::model::{Trade, TradeStatus};

#[derive(Clone, Debug, PartialEq)]
pub struct EquityPoint {
    pub ts_ms: i64,
    /// Cumulative net P&L of closed trades at this close.
    pub cum_net: Decimal,
    /// `initial balance + cum_net`; `None` without an initial balance.
    pub equity: Option<Decimal>,
    pub trade_key: String,
}

/// Closed trades in close order (ties by key), as steps of cumulative net P&L.
pub fn equity_curve(trades: &[&Trade], initial_balance: Option<Decimal>) -> Vec<EquityPoint> {
    let mut closed: Vec<&&Trade> = trades
        .iter()
        .filter(|trade| trade.status == TradeStatus::Closed && trade.close_ts_ms.is_some())
        .collect();
    closed.sort_by(|a, b| (a.close_ts_ms, &a.key).cmp(&(b.close_ts_ms, &b.key)));
    let mut cum = Decimal::ZERO;
    closed
        .into_iter()
        .map(|trade| {
            cum += trade.net_pnl();
            EquityPoint {
                ts_ms: trade.close_ts_ms.unwrap_or_default(),
                cum_net: cum,
                equity: initial_balance.map(|initial| initial + cum),
                trade_key: trade.key.clone(),
            }
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct DrawdownPoint {
    pub ts_ms: i64,
    /// Distance below the running peak; zero at a new high.
    pub depth: Decimal,
    /// `depth / (initial + peak)`; `None` when that base is not positive or unknown.
    pub pct: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DrawdownStats {
    pub max: Decimal,
    /// The percentage at the deepest point. `None` without an initial balance, and when there was
    /// no drawdown.
    pub max_pct: Option<f64>,
    pub peak: Decimal,
    pub current: Decimal,
    /// Number of separate episodes (a fall below a peak until it is regained).
    pub episodes: usize,
    /// Mean of each episode's deepest point.
    pub avg_episode: Metric<Decimal>,
}

pub fn drawdown_series(
    curve: &[EquityPoint],
    initial_balance: Option<Decimal>,
) -> Vec<DrawdownPoint> {
    let mut peak = Decimal::ZERO;
    curve
        .iter()
        .map(|point| {
            if point.cum_net > peak {
                peak = point.cum_net;
            }
            let depth = peak - point.cum_net;
            DrawdownPoint {
                ts_ms: point.ts_ms,
                depth,
                pct: pct_of(depth, initial_balance, peak),
            }
        })
        .collect()
}

fn pct_of(depth: Decimal, initial_balance: Option<Decimal>, peak: Decimal) -> Option<f64> {
    let initial = initial_balance.filter(|initial| *initial > Decimal::ZERO)?;
    let base = initial + peak;
    if base <= Decimal::ZERO {
        return None;
    }
    (depth / base).to_f64()
}

pub fn drawdown_stats(curve: &[EquityPoint], initial_balance: Option<Decimal>) -> DrawdownStats {
    let mut peak = Decimal::ZERO;
    let mut max = Decimal::ZERO;
    let mut max_pct = None;
    let mut episodes = 0usize;
    let mut episode_depth = Decimal::ZERO;
    let mut episode_total = Decimal::ZERO;
    let mut in_episode = false;
    let mut current = Decimal::ZERO;
    for point in curve {
        if point.cum_net > peak {
            peak = point.cum_net;
        }
        let depth = peak - point.cum_net;
        current = depth;
        if depth > Decimal::ZERO {
            if !in_episode {
                in_episode = true;
                episodes += 1;
                episode_depth = Decimal::ZERO;
            }
            episode_depth = episode_depth.max(depth);
            if depth > max {
                max = depth;
                max_pct = pct_of(depth, initial_balance, peak);
            }
        } else if in_episode {
            in_episode = false;
            episode_total += episode_depth;
        }
    }
    if in_episode {
        episode_total += episode_depth;
    }
    let avg_episode = if episodes == 0 {
        Metric::Unavailable(Reason::NotApplicable)
    } else {
        Metric::Value(episode_total / Decimal::from(episodes))
    };
    DrawdownStats {
        max,
        max_pct,
        peak,
        current,
        episodes,
        avg_episode,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::journal::model::{CostBasis, Side};
    use crate::model::journal::reconstruct::reconstruct;
    use crate::model::journal::testing::{d, exec};

    /// One round trip per `pnl`, closing one hour apart, each of size 1 (entry 1000).
    fn trades_with(pnls: &[i64]) -> Vec<Trade> {
        let mut executions = Vec::new();
        for (index, pnl) in pnls.iter().enumerate() {
            let t = (index as i64 + 1) * 10_000;
            executions.push(exec(&format!("{index}a"), Side::Buy, "1", "1000", t));
            executions.push(exec(
                &format!("{index}b"),
                Side::Sell,
                "1",
                &(1000 + pnl).to_string(),
                t + 5_000,
            ));
        }
        reconstruct(&executions, |_| CostBasis::AverageCost)
    }

    fn refs(trades: &[Trade]) -> Vec<&Trade> {
        trades.iter().collect()
    }

    #[test]
    fn the_curve_steps_at_each_close() {
        let trades = trades_with(&[10, -4, 6]);
        let curve = equity_curve(&refs(&trades), Some(d("1000")));
        let cum: Vec<_> = curve.iter().map(|p| p.cum_net).collect();
        assert_eq!(cum, [d("10"), d("6"), d("12")]);
        assert_eq!(curve[2].equity, Some(d("1012")));
        assert!(curve[0].ts_ms < curve[1].ts_ms);
    }

    #[test]
    fn without_an_initial_balance_there_is_no_equity_only_pnl() {
        let trades = trades_with(&[10]);
        let curve = equity_curve(&refs(&trades), None);
        assert_eq!(curve[0].equity, None);
        assert_eq!(curve[0].cum_net, d("10"));
    }

    #[test]
    fn open_trades_are_not_on_the_curve() {
        let trades = reconstruct(&[exec("1", Side::Buy, "1", "100", 1_000)], |_| {
            CostBasis::AverageCost
        });
        assert!(equity_curve(&refs(&trades), None).is_empty());
    }

    #[test]
    fn drawdown_is_measured_from_the_running_peak() {
        // cum: 10, 6, 2, 12, 7  -> peak 10, depth 8 at the third; then a new high; then 5 down.
        let trades = trades_with(&[10, -4, -4, 10, -5]);
        let curve = equity_curve(&refs(&trades), Some(d("100")));
        let series = drawdown_series(&curve, Some(d("100")));
        let depths: Vec<_> = series.iter().map(|p| p.depth).collect();
        assert_eq!(depths, [d("0"), d("4"), d("8"), d("0"), d("5")]);
        let stats = drawdown_stats(&curve, Some(d("100")));
        assert_eq!(stats.max, d("8"));
        assert_eq!(stats.peak, d("12"));
        assert_eq!(stats.current, d("5"));
        assert_eq!(stats.episodes, 2);
        assert_eq!(stats.avg_episode, Metric::Value(d("6.5")));
        // 8 / (100 + 10)
        assert!((stats.max_pct.unwrap() - 8.0 / 110.0).abs() < 1e-12);
        assert!((series[4].pct.unwrap() - 5.0 / 112.0).abs() < 1e-12);
    }

    #[test]
    fn percentages_need_an_initial_balance() {
        let trades = trades_with(&[10, -4]);
        let curve = equity_curve(&refs(&trades), None);
        let stats = drawdown_stats(&curve, None);
        assert_eq!(stats.max, d("4"));
        assert_eq!(stats.max_pct, None);
        assert!(
            drawdown_series(&curve, None)
                .iter()
                .all(|p| p.pct.is_none())
        );
        let zero = drawdown_stats(&curve, Some(Decimal::ZERO));
        assert_eq!(zero.max_pct, None);
    }

    #[test]
    fn a_curve_that_only_rises_has_no_drawdown() {
        let trades = trades_with(&[5, 5, 5]);
        let curve = equity_curve(&refs(&trades), Some(d("100")));
        let stats = drawdown_stats(&curve, Some(d("100")));
        assert_eq!(stats.max, Decimal::ZERO);
        assert_eq!(stats.max_pct, None);
        assert_eq!(stats.episodes, 0);
        assert_eq!(
            stats.avg_episode,
            Metric::Unavailable(Reason::NotApplicable)
        );
    }

    #[test]
    fn a_losing_start_is_a_drawdown_from_zero() {
        let trades = trades_with(&[-5, -5]);
        let curve = equity_curve(&refs(&trades), Some(d("100")));
        let stats = drawdown_stats(&curve, Some(d("100")));
        assert_eq!(stats.max, d("10"));
        assert!((stats.max_pct.unwrap() - 0.1).abs() < 1e-12);
    }

    #[test]
    fn an_account_wiped_out_has_no_percentage_instead_of_infinity() {
        let trades = trades_with(&[-100]);
        let curve = equity_curve(&refs(&trades), Some(d("100")));
        // peak 0, base 100 -> 100%; then nothing below zero base can divide.
        let stats = drawdown_stats(&curve, Some(d("100")));
        assert!((stats.max_pct.unwrap() - 1.0).abs() < 1e-12);
        let negative = drawdown_stats(&curve, Some(d("-5")));
        assert_eq!(negative.max_pct, None);
    }

    #[test]
    fn an_empty_curve_has_nothing() {
        let stats = drawdown_stats(&[], Some(d("100")));
        assert_eq!(stats.max, Decimal::ZERO);
        assert_eq!(stats.episodes, 0);
        assert!(drawdown_series(&[], None).is_empty());
    }
}
