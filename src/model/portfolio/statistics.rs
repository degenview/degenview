//! Figures the Statistics and Overview tabs show, derived from the reporting-currency
//! holdings, the value history and the (converted) transactions. Pure value types: no
//! store, no formatting.

// Transient: the portfolio screens read these once they are hosted.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

use crate::model::portfolio::accounting::ClosedPosition;
use crate::model::portfolio::{
    PortfolioAsset, PortfolioHolding, PortfolioSnapshot, PortfolioTransaction,
    PortfolioTransactionType,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Up,
    Down,
    Unchanged,
}

/// How a value moved between two points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PortfolioValueChange {
    pub amount: f64,
    /// `None` when it started from zero.
    pub percentage: Option<f64>,
}

impl PortfolioValueChange {
    pub fn new(from: f64, to: f64) -> Self {
        let amount = to - from;
        Self {
            amount,
            percentage: (from != 0.0).then(|| amount / from),
        }
    }

    pub fn direction(&self) -> Direction {
        direction_of(self.amount)
    }
}

fn direction_of(value: f64) -> Direction {
    if value > 0.0 {
        Direction::Up
    } else if value < 0.0 {
        Direction::Down
    } else {
        Direction::Unchanged
    }
}

/// How far back the Overview's value chart looks.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum PortfolioHistoryRange {
    OneDay,
    OneWeek,
    #[default]
    OneMonth,
    OneYear,
    All,
}

impl PortfolioHistoryRange {
    pub const ALL: [Self; 5] = [
        Self::OneDay,
        Self::OneWeek,
        Self::OneMonth,
        Self::OneYear,
        Self::All,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::OneDay => "1D",
            Self::OneWeek => "1W",
            Self::OneMonth => "1M",
            Self::OneYear => "1Y",
            Self::All => "ALL",
        }
    }

    /// Milliseconds back from now, or `None` for everything.
    pub fn duration_ms(self) -> Option<i64> {
        const DAY: i64 = 86_400_000;
        match self {
            Self::OneDay => Some(DAY),
            Self::OneWeek => Some(7 * DAY),
            Self::OneMonth => Some(31 * DAY),
            Self::OneYear => Some(365 * DAY),
            Self::All => None,
        }
    }

    /// The span in words, for captions beside a change figure.
    pub fn period_name(self) -> &'static str {
        match self {
            Self::OneDay => "24h",
            Self::OneWeek => "7 days",
            Self::OneMonth => "30 days",
            Self::OneYear => "1 year",
            Self::All => "all time",
        }
    }

    pub fn filter(self, snapshots: &[PortfolioSnapshot], now_ms: i64) -> Vec<PortfolioSnapshot> {
        match self.duration_ms() {
            None => snapshots.to_vec(),
            Some(duration) => {
                let cutoff = now_ms - duration;
                snapshots
                    .iter()
                    .filter(|snapshot| snapshot.timestamp >= cutoff)
                    .cloned()
                    .collect()
            }
        }
    }
}

/// Profit or loss over a stretch of history, separate from money moved in or out.
///
/// A value change alone would call a deposit a gain, so this follows
/// `value − net_contributions`: the P&L at the end minus the P&L at the start. The
/// percentage is against the capital in play — the starting value plus anything
/// contributed since.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PortfolioPeriodChange {
    pub profit: f64,
    pub percentage: Option<f64>,
}

impl PortfolioPeriodChange {
    /// `snapshots` are the history inside the range, oldest first. `current_value` and
    /// `current_net_contributions` are today's, taken from the ledger: they stand in for the
    /// end of the range, so money moved after the last stored point is not read as profit.
    pub fn new(
        snapshots: &[PortfolioSnapshot],
        current_value: f64,
        current_net_contributions: f64,
    ) -> Option<Self> {
        let first = snapshots.first()?;
        let start_profit = first.value - first.net_contributions;
        let end_profit = current_value - current_net_contributions;
        let profit = end_profit - start_profit;
        let deposited = (current_net_contributions - first.net_contributions).max(0.0);
        let capital = first.value + deposited;
        Some(Self {
            profit,
            percentage: (capital > 0.0).then(|| profit / capital),
        })
    }

    pub fn direction(&self) -> Direction {
        direction_of(self.profit)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssetResult {
    pub asset: PortfolioAsset,
    pub total_pnl: f64,
    pub pnl_percent: Option<f64>,
}

/// The highest or lowest point of the value history.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extreme {
    pub value: f64,
    pub timestamp: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PortfolioStatistics {
    pub total_value: f64,
    pub cost_basis: f64,
    pub realized_pnl: f64,
    pub unrealized_pnl: f64,
    /// Unrealized P&L over the cost basis of the assets that have a price.
    pub unrealized_percent: Option<f64>,
    /// Value now against value 24 hours ago, at today's quantities. `None` when nothing has
    /// both prices.
    pub day_change: Option<PortfolioValueChange>,
    pub best: Option<AssetResult>,
    pub worst: Option<AssetResult>,
    /// Every priced asset, best total P&L first.
    pub asset_results: Vec<AssetResult>,
    pub winners: usize,
    pub losers: usize,
    pub high: Option<Extreme>,
    pub low: Option<Extreme>,
    pub asset_count: usize,
    pub transaction_count: usize,
    pub buy_count: usize,
    pub sell_count: usize,
    pub first_transaction: Option<i64>,
    /// Fees paid, in the reporting currency (pass converted transactions).
    pub fees_paid: f64,
}

impl PortfolioStatistics {
    pub fn total_pnl(&self) -> f64 {
        self.realized_pnl + self.unrealized_pnl
    }

    pub fn new(
        holdings: &[PortfolioHolding],
        history: &[PortfolioSnapshot],
        transactions: &[PortfolioTransaction],
    ) -> Self {
        Self::with_closed(holdings, &[], history, transactions)
    }

    /// Statistics that also count the positions closed completely: their realized profit is
    /// part of the total, and each is a win or a loss in the results.
    pub fn with_closed(
        holdings: &[PortfolioHolding],
        closed: &[ClosedPosition],
        history: &[PortfolioSnapshot],
        transactions: &[PortfolioTransaction],
    ) -> Self {
        let total_value = holdings.iter().filter_map(|h| h.current_value).sum();
        let cost_basis = holdings.iter().map(|h| h.cost_basis).sum();
        let realized_pnl = holdings.iter().map(|h| h.realized_pnl).sum::<f64>()
            + closed.iter().map(|c| c.realized_pnl).sum::<f64>();
        let unrealized_pnl: f64 = holdings.iter().filter_map(|h| h.unrealized_pnl).sum();

        let priced_cost: f64 = holdings
            .iter()
            .filter(|h| h.unrealized_pnl.is_some())
            .map(|h| h.cost_basis)
            .sum();

        let (mut previous, mut current) = (0.0, 0.0);
        for holding in holdings {
            let (Some(price), Some(before)) = (holding.current_price, holding.previous_day_price)
            else {
                continue;
            };
            previous += holding.quantity * before;
            current += holding.quantity * price;
        }

        let mut results: Vec<AssetResult> = holdings
            .iter()
            .filter_map(|holding| {
                Some(AssetResult {
                    asset: holding.asset.clone(),
                    total_pnl: holding.total_pnl()?,
                    pnl_percent: holding.pnl_percent(),
                })
            })
            .chain(closed.iter().map(|position| AssetResult {
                asset: position.asset.clone(),
                total_pnl: position.realized_pnl,
                pnl_percent: None,
            }))
            .collect();
        results.sort_by(|a, b| b.total_pnl.total_cmp(&a.total_pnl));
        let ranked = || results.iter().filter(|result| result.pnl_percent.is_some());
        let by_percent = |result: &&AssetResult| result.pnl_percent.unwrap_or(0.0);
        let best = ranked()
            .max_by(|a, b| by_percent(a).total_cmp(&by_percent(b)))
            .cloned();
        let worst = ranked()
            .min_by(|a, b| by_percent(a).total_cmp(&by_percent(b)))
            .cloned();
        let winners = results.iter().filter(|r| r.total_pnl > 0.0).count();
        let losers = results.iter().filter(|r| r.total_pnl < 0.0).count();

        let extreme = |snapshot: &PortfolioSnapshot| Extreme {
            value: snapshot.value,
            timestamp: snapshot.timestamp,
        };
        // Days before the first purchase are an empty portfolio, not a low of zero.
        let held = || {
            history
                .iter()
                .filter(|point| point.value != 0.0 || point.net_contributions != 0.0)
        };
        let high = held()
            .max_by(|a, b| a.value.total_cmp(&b.value))
            .map(extreme);
        let low = held()
            .min_by(|a, b| a.value.total_cmp(&b.value))
            .map(extreme);

        Self {
            total_value,
            cost_basis,
            realized_pnl,
            unrealized_pnl,
            unrealized_percent: (priced_cost != 0.0).then(|| unrealized_pnl / priced_cost),
            day_change: (previous > 0.0).then(|| PortfolioValueChange::new(previous, current)),
            best,
            worst,
            winners,
            losers,
            high,
            low,
            asset_results: results,
            asset_count: holdings.len(),
            transaction_count: transactions.len(),
            buy_count: transactions
                .iter()
                .filter(|t| t.kind == PortfolioTransactionType::Buy)
                .count(),
            sell_count: transactions
                .iter()
                .filter(|t| t.kind == PortfolioTransactionType::Sell)
                .count(),
            first_transaction: transactions.iter().map(|t| t.timestamp).min(),
            fees_paid: transactions.iter().map(|t| t.fee).sum(),
        }
    }
}

/// Geometry of one P&L bar around the zero line: gains grow right, losses left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PnlBar {
    /// Left edge as a fraction of the track width.
    pub start: f64,
    /// Length as a fraction of the track width (a sliver is kept so zero is visible).
    pub length: f64,
    pub is_gain: bool,
}

/// Bars for `results`, relative to the widest reach on each side, plus where zero sits.
pub fn pnl_bars(results: &[AssetResult]) -> (Vec<PnlBar>, f64) {
    let negative_reach = results
        .iter()
        .map(|r| (-r.total_pnl).max(0.0))
        .fold(0.0, f64::max);
    let positive_reach = results
        .iter()
        .map(|r| r.total_pnl.max(0.0))
        .fold(0.0, f64::max);
    let span = (negative_reach + positive_reach).max(1e-8);
    let zero = negative_reach / span;
    let bars = results
        .iter()
        .map(|result| {
            let length = (result.total_pnl.abs() / span).max(0.0);
            let is_gain = result.total_pnl >= 0.0;
            PnlBar {
                start: if is_gain { zero } else { zero - length },
                length,
                is_gain,
            }
        })
        .collect();
    (bars, zero)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::PortfolioCurrency;
    use std::collections::HashMap;

    fn asset(ticker: &str) -> PortfolioAsset {
        PortfolioAsset {
            key: format!("binance:{ticker}"),
            symbol: format!("{ticker}/USDT"),
            name: ticker.to_owned(),
            source: DataSource::Binance,
            quote_currency: PortfolioCurrency::Usd,
            metadata: HashMap::new(),
        }
    }

    fn holding(
        ticker: &str,
        qty: f64,
        cost: f64,
        price: Option<f64>,
        previous: Option<f64>,
    ) -> PortfolioHolding {
        let value = price.map(|p| p * qty);
        PortfolioHolding {
            asset: asset(ticker),
            quantity: qty,
            cost_basis: cost,
            average_cost: cost / qty,
            realized_pnl: 0.0,
            current_price: price,
            previous_day_price: previous,
            current_value: value,
            unrealized_pnl: value.map(|v| v - cost),
            allocation: 0.0,
            is_price_stale: false,
        }
    }

    #[test]
    fn closed_positions_count_towards_realized_pnl_and_the_win_rate() {
        let open = [holding("A", 1.0, 100.0, Some(110.0), None)];
        let closed = [
            ClosedPosition {
                asset: asset("B"),
                realized_pnl: 50.0,
            },
            ClosedPosition {
                asset: asset("C"),
                realized_pnl: -20.0,
            },
        ];
        let stats = PortfolioStatistics::with_closed(&open, &closed, &[], &[]);
        assert_eq!(stats.realized_pnl, 30.0);
        assert_eq!(stats.unrealized_pnl, 10.0);
        assert_eq!(stats.total_pnl(), 40.0);
        assert_eq!((stats.winners, stats.losers), (2, 1));
        assert_eq!(stats.asset_results.len(), 3);
        // A closed position has no cost basis, so it is not ranked by percentage.
        assert_eq!(
            stats.best.map(|r| r.asset.key),
            Some("binance:A".to_owned())
        );
        // `asset_count` is what is held.
        assert_eq!(stats.asset_count, 1);
    }

    fn snapshot(timestamp: i64, value: f64, contributions: f64) -> PortfolioSnapshot {
        PortfolioSnapshot {
            portfolio_id: 1,
            timestamp,
            value,
            net_contributions: contributions,
            realized_pnl: 0.0,
            unrealized_pnl: 0.0,
            is_complete: true,
        }
    }

    #[test]
    fn value_changes_report_amount_percent_and_direction() {
        let up = PortfolioValueChange::new(100.0, 110.0);
        assert_eq!(
            (up.amount, up.percentage, up.direction()),
            (10.0, Some(0.1), Direction::Up)
        );
        let down = PortfolioValueChange::new(200.0, 150.0);
        assert_eq!(down.percentage, Some(-0.25));
        assert_eq!(down.direction(), Direction::Down);
        let from_zero = PortfolioValueChange::new(0.0, 5.0);
        assert_eq!(from_zero.percentage, None);
        assert_eq!(
            PortfolioValueChange::new(3.0, 3.0).direction(),
            Direction::Unchanged
        );
    }

    #[test]
    fn a_deposit_is_not_a_gain() {
        // Worth $1,000 with $1,000 in; later worth $2,100 after another $1,000 went in.
        let history = [
            snapshot(0, 1_000.0, 1_000.0),
            snapshot(10, 2_000.0, 2_000.0),
        ];
        let change = PortfolioPeriodChange::new(&history, 2_100.0, 2_000.0).unwrap();
        assert!(
            (change.profit - 100.0).abs() < 1e-9,
            "only the $100 above the money put in"
        );
        // Capital in play: the $1,000 at the start plus the $1,000 deposited since.
        assert!((change.percentage.unwrap() - 0.05).abs() < 1e-9);
        assert_eq!(change.direction(), Direction::Up);
    }

    #[test]
    fn a_withdrawal_is_not_a_loss_and_an_empty_range_has_no_change() {
        let history = [snapshot(0, 2_000.0, 1_000.0), snapshot(10, 1_500.0, 500.0)];
        let change = PortfolioPeriodChange::new(&history, 1_500.0, 500.0).unwrap();
        // P&L went from 1,000 to 1,000: nothing gained or lost despite value falling.
        assert!(change.profit.abs() < 1e-9);
        assert_eq!(change.direction(), Direction::Unchanged);
        assert_eq!(PortfolioPeriodChange::new(&[], 100.0, 0.0), None);
    }

    #[test]
    fn money_moved_after_the_last_point_is_not_profit_or_loss() {
        // The last stored point is worth 1,000 with 1,000 in.
        let history = [
            snapshot(0, 1_000.0, 1_000.0),
            snapshot(10, 1_000.0, 1_000.0),
        ];
        // A 500 buy since then: worth 1,500 with 1,500 in.
        let bought = PortfolioPeriodChange::new(&history, 1_500.0, 1_500.0).unwrap();
        assert!(bought.profit.abs() < 1e-9, "a buy is not a gain");
        // A 400 withdrawal since then: worth 600 with 600 in.
        let sold = PortfolioPeriodChange::new(&history, 600.0, 600.0).unwrap();
        assert!(sold.profit.abs() < 1e-9, "a sale is not a loss");
        // A gain on top of the new money still shows.
        let gained = PortfolioPeriodChange::new(&history, 1_600.0, 1_500.0).unwrap();
        assert!((gained.profit - 100.0).abs() < 1e-9);
    }

    #[test]
    fn a_range_that_started_empty_has_no_percentage() {
        let history = [snapshot(0, 0.0, 0.0)];
        let change = PortfolioPeriodChange::new(&history, 0.0, 0.0).unwrap();
        assert_eq!(change.percentage, None);
    }

    #[test]
    fn history_ranges_cut_off_older_snapshots() {
        let day = 86_400_000;
        let now = 400 * day;
        let history: Vec<PortfolioSnapshot> = [0, 100, 369, 390, 399]
            .iter()
            .map(|d| snapshot(*d * day, 1.0, 0.0))
            .collect();
        let count = |range: PortfolioHistoryRange| range.filter(&history, now).len();
        assert_eq!(
            count(PortfolioHistoryRange::OneDay),
            1,
            "399d is 1 day back, inclusive"
        );
        assert_eq!(
            count(PortfolioHistoryRange::OneWeek),
            1,
            "390d is ten days back"
        );
        assert_eq!(count(PortfolioHistoryRange::OneMonth), 3);
        assert_eq!(count(PortfolioHistoryRange::OneYear), 4);
        assert_eq!(count(PortfolioHistoryRange::All), 5);
        assert_eq!(
            PortfolioHistoryRange::default(),
            PortfolioHistoryRange::OneMonth
        );
        assert_eq!(PortfolioHistoryRange::OneMonth.period_name(), "30 days");
        assert_eq!(PortfolioHistoryRange::All.title(), "ALL");
    }

    #[test]
    fn totals_percentages_and_the_day_change_come_from_the_holdings() {
        let holdings = vec![
            holding("BTC", 1.0, 100.0, Some(150.0), Some(140.0)),
            holding("ETH", 2.0, 100.0, Some(40.0), Some(50.0)),
            holding("DOGE", 10.0, 20.0, None, None),
        ];
        let stats = PortfolioStatistics::new(&holdings, &[], &[]);
        assert_eq!(stats.total_value, 230.0);
        assert_eq!(stats.cost_basis, 220.0);
        assert_eq!(stats.unrealized_pnl, 30.0);
        // Percent is over the cost of the priced assets only (200), not the unpriced one.
        assert_eq!(stats.unrealized_percent, Some(0.15));
        // Day change: now 150 + 80 = 230 against 140 + 100 = 240.
        let day = stats.day_change.unwrap();
        assert_eq!(day.amount, -10.0);
        assert!((day.percentage.unwrap() + 10.0 / 240.0).abs() < 1e-12);
        assert_eq!(stats.asset_count, 3);
        assert_eq!((stats.winners, stats.losers), (1, 1));
        assert_eq!(stats.total_pnl(), 30.0);
    }

    #[test]
    fn best_and_worst_are_by_percentage_and_results_by_amount() {
        let holdings = vec![
            holding("A", 1.0, 10.0, Some(30.0), None), // +200 %, +20
            holding("B", 1.0, 1_000.0, Some(1_100.0), None), // +10 %, +100
            holding("C", 1.0, 100.0, Some(50.0), None), // -50 %, -50
            holding("D", 1.0, 10.0, None, None),       // unpriced
        ];
        let stats = PortfolioStatistics::new(&holdings, &[], &[]);
        assert_eq!(stats.best.as_ref().unwrap().asset.key, "binance:A");
        assert_eq!(stats.worst.as_ref().unwrap().asset.key, "binance:C");
        let order: Vec<&str> = stats
            .asset_results
            .iter()
            .map(|r| r.asset.key.as_str())
            .collect();
        assert_eq!(
            order,
            ["binance:B", "binance:A", "binance:C"],
            "biggest gain in money first"
        );
    }

    #[test]
    fn nothing_priced_leaves_the_derived_figures_empty() {
        let stats = PortfolioStatistics::new(&[holding("X", 1.0, 5.0, None, None)], &[], &[]);
        assert_eq!(stats.unrealized_percent, None);
        assert_eq!(stats.day_change, None);
        assert!(stats.best.is_none() && stats.worst.is_none() && stats.asset_results.is_empty());
        let empty = PortfolioStatistics::new(&[], &[], &[]);
        assert_eq!(empty.total_value, 0.0);
        assert_eq!(empty.asset_count, 0);
    }

    #[test]
    fn history_extremes_and_activity_counts() {
        let history = [
            snapshot(1, 50.0, 0.0),
            snapshot(2, 90.0, 0.0),
            snapshot(3, 10.0, 0.0),
        ];
        let stats = PortfolioStatistics::new(&[], &history, &[]);
        assert_eq!(
            stats.high,
            Some(Extreme {
                value: 90.0,
                timestamp: 2
            })
        );
        assert_eq!(
            stats.low,
            Some(Extreme {
                value: 10.0,
                timestamp: 3
            })
        );
        assert_eq!(PortfolioStatistics::new(&[], &[], &[]).high, None);
    }

    #[test]
    fn days_before_the_first_purchase_are_not_the_low() {
        let history = [
            snapshot(1, 0.0, 0.0),
            snapshot(2, 0.0, 0.0),
            snapshot(3, 40.0, 40.0),
            snapshot(4, 55.0, 40.0),
        ];
        let stats = PortfolioStatistics::new(&[], &history, &[]);
        assert_eq!(stats.low.map(|extreme| extreme.value), Some(40.0));
        // A portfolio that really fell to nothing after funding still reports it.
        let wiped = [snapshot(1, 40.0, 40.0), snapshot(2, 0.0, 40.0)];
        let stats = PortfolioStatistics::new(&[], &wiped, &[]);
        assert_eq!(stats.low.map(|extreme| extreme.value), Some(0.0));
    }

    #[test]
    fn bars_grow_either_side_of_a_shared_zero_line() {
        let result = |pnl: f64| AssetResult {
            asset: asset("X"),
            total_pnl: pnl,
            pnl_percent: None,
        };
        let (bars, zero) = pnl_bars(&[result(75.0), result(-25.0), result(0.0)]);
        // Reach: 25 left, 75 right, span 100: zero at 25 %.
        assert!((zero - 0.25).abs() < 1e-12);
        assert!(
            bars[0].is_gain
                && (bars[0].start - 0.25).abs() < 1e-12
                && (bars[0].length - 0.75).abs() < 1e-12
        );
        assert!(
            !bars[1].is_gain
                && bars[1].start.abs() < 1e-12
                && (bars[1].length - 0.25).abs() < 1e-12
        );
        assert!(bars[2].is_gain && bars[2].length == 0.0);
        let (none, zero) = pnl_bars(&[]);
        assert!(none.is_empty() && zero == 0.0);
        // All losses: zero sits at the far right.
        let (_, zero) = pnl_bars(&[result(-10.0)]);
        assert!((zero - 1.0).abs() < 1e-12);
    }
}
