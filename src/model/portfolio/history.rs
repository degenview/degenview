//! Pure portfolio-history builders: the daily rebuild (one snapshot per local day, priced from
//! the nearest prior daily candle) and the 1D chart's intraday replay. Ported from
//! `PortfolioStore.buildSnapshots` / `historyRebuildStart` / `intradaySnapshots`.
//!
//! Everything here is a function of its arguments, so the host runs it on a worker thread and
//! the tests need no clock, network or database.

use crate::model::Kline;
use crate::model::portfolio::accounting::{holdings_and_closed, net_contributions};
use crate::model::portfolio::{
    PortfolioAsset, PortfolioLedgerSnapshot, PortfolioQuote, PortfolioSnapshot,
    PortfolioTransaction,
};
use std::collections::{BTreeMap, HashMap, HashSet};

pub const DAY_MS: i64 = 86_400_000;
/// A candle older than this is no quote at all: the market was not traded then.
pub const LOOKBACK_MS: i64 = 3 * DAY_MS;
/// Spacing of the 1D chart's points.
pub const INTRADAY_STEP_MS: i64 = 30 * 60 * 1_000;
/// How far back the 1D chart looks.
pub const INTRADAY_WINDOW_MS: i64 = DAY_MS;

/// The UTC offset, in seconds east, in force at an instant. A bare number is a fixed offset;
/// a function of the instant follows daylight saving, so a spring day is 23 hours long.
pub trait ZoneOffset {
    fn offset_secs(&self, at_ms: i64) -> i64;
}

impl ZoneOffset for i64 {
    fn offset_secs(&self, _: i64) -> i64 {
        *self
    }
}

impl<F: Fn(i64) -> i64> ZoneOffset for F {
    fn offset_secs(&self, at_ms: i64) -> i64 {
        self(at_ms)
    }
}

/// Midnight (local time) at or before `ms`.
pub fn start_of_day(ms: i64, zone: impl ZoneOffset) -> i64 {
    let local = ms + zone.offset_secs(ms) * 1_000;
    let midnight = local.div_euclid(DAY_MS) * DAY_MS;
    // The offset in force at that midnight can differ from the one at `ms`.
    let guess = midnight - zone.offset_secs(ms) * 1_000;
    midnight - zone.offset_secs(guess) * 1_000
}

/// Identifies the transactions of one portfolio as a history was computed from them: an edit,
/// an addition or a deletion changes it, the order they are listed in does not.
pub fn transactions_fingerprint(transactions: &[PortfolioTransaction]) -> u64 {
    use std::hash::{Hash, Hasher};
    transactions
        .iter()
        .map(|tx| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            tx.id.hash(&mut hasher);
            tx.portfolio_id.hash(&mut hasher);
            tx.asset.key.hash(&mut hasher);
            std::mem::discriminant(&tx.kind).hash(&mut hasher);
            tx.quantity.to_bits().hash(&mut hasher);
            tx.price.map(f64::to_bits).hash(&mut hasher);
            tx.fee.to_bits().hash(&mut hasher);
            tx.timestamp.hash(&mut hasher);
            tx.updated_at.hash(&mut hasher);
            hasher.finish()
        })
        .fold(0u64, u64::wrapping_add)
}

/// Where a portfolio's daily history must be (re)built from, or `None` when it is up to date.
/// An edited ledger invalidates from the edit's day; otherwise only the days after the newest
/// stored point are missing; with nothing stored the history starts at the first transaction.
pub fn rebuild_start(
    portfolio_id: u64,
    snapshot: &PortfolioLedgerSnapshot,
    now_ms: i64,
    zone: impl ZoneOffset + Copy,
) -> Option<i64> {
    let today = start_of_day(now_ms, zone);
    if let Some(invalidated) = snapshot.invalidated_after.get(&portfolio_id) {
        return Some(start_of_day(*invalidated, zone).min(today));
    }
    let latest = snapshot
        .historical_snapshots
        .iter()
        .filter(|point| point.portfolio_id == portfolio_id)
        .map(|point| point.timestamp)
        .max();
    if let Some(latest) = latest {
        // The next midnight, found from the calendar: a day is not always 24 hours.
        let next = start_of_day(start_of_day(latest, zone) + DAY_MS + DAY_MS / 2, zone);
        return (next <= today).then_some(next);
    }
    snapshot
        .transactions
        .iter()
        .filter(|transaction| transaction.portfolio_id == portfolio_id)
        .map(|transaction| start_of_day(transaction.timestamp, zone))
        .min()
}

/// The last daily bar that had finished by `ms`, in bars sorted by open time. A bar still
/// forming at `ms` closes later than the holdings it would be paired with.
fn last_finished_bar_at(bars: &[Kline], ms: i64) -> Option<&Kline> {
    let after = bars.partition_point(|bar| bar.open_time + DAY_MS <= ms);
    after.checked_sub(1).map(|index| &bars[index])
}

/// One snapshot per day from `start` through `end`, each at that day's local midnight: the
/// holdings after every transaction before it, priced at the close of the last daily candle
/// that had finished by then. `transactions` are the portfolio's own; `bars` are daily candles
/// by asset key. A day with an unpriced holding is flagged incomplete.
pub fn build_snapshots(
    portfolio_id: u64,
    transactions: &[PortfolioTransaction],
    assets: &[PortfolioAsset],
    bars: &HashMap<String, Vec<Kline>>,
    start: i64,
    end: i64,
    zone: impl ZoneOffset + Copy,
) -> Vec<PortfolioSnapshot> {
    let owned: HashMap<&str, Vec<Kline>> = bars
        .iter()
        .map(|(key, series)| {
            let mut series = series.clone();
            series.sort_by_key(|bar| bar.open_time);
            (key.as_str(), series)
        })
        .collect();
    let ids = HashSet::from([portfolio_id]);
    let mut points = Vec::new();
    let mut cursor = start_of_day(start, zone);
    while cursor <= end {
        let quotes: HashMap<String, PortfolioQuote> = assets
            .iter()
            .filter_map(|asset| {
                let bar = last_finished_bar_at(owned.get(asset.key.as_str())?, cursor)?;
                (cursor - bar.open_time <= LOOKBACK_MS + DAY_MS).then(|| {
                    (
                        asset.key.clone(),
                        PortfolioQuote {
                            price: bar.close,
                            previous_day_price: None,
                            timestamp: bar.open_time,
                        },
                    )
                })
            })
            .collect();
        points.push(snapshot_at(
            portfolio_id,
            &ids,
            transactions,
            &quotes,
            cursor,
        ));
        // Midnight to midnight: recomputed so a daylight-saving day is 23 or 25 hours long.
        cursor = start_of_day(cursor + DAY_MS + DAY_MS / 2, zone);
    }
    points
}

fn snapshot_at(
    portfolio_id: u64,
    ids: &HashSet<u64>,
    transactions: &[PortfolioTransaction],
    quotes: &HashMap<String, PortfolioQuote>,
    at: i64,
) -> PortfolioSnapshot {
    let (held, closed) = match holdings_and_closed(transactions, ids, Some(at), quotes, at) {
        Ok((held, closed)) => (Some(held), closed),
        Err(_) => (None, Vec::new()),
    };
    let is_complete = held
        .as_ref()
        .is_some_and(|held| held.iter().all(|holding| holding.current_price.is_some()));
    let value: f64 = held
        .iter()
        .flatten()
        .filter_map(|holding| holding.current_value)
        .sum();
    let cost: f64 = held
        .iter()
        .flatten()
        .map(|holding| holding.cost_basis)
        .sum();
    // Positions closed by `at` still earned what they earned.
    let realized: f64 = held
        .iter()
        .flatten()
        .map(|holding| holding.realized_pnl)
        .sum::<f64>()
        + closed
            .iter()
            .map(|position| position.realized_pnl)
            .sum::<f64>();
    PortfolioSnapshot {
        portfolio_id,
        timestamp: at,
        value,
        net_contributions: net_contributions(transactions, ids, Some(at)),
        realized_pnl: realized,
        unrealized_pnl: value - cost,
        is_complete,
    }
}

/// Recent fine-grained candles of one asset, for the 1D chart.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IntradayBars {
    pub bars: Vec<Kline>,
    /// The length of one candle.
    pub span_ms: i64,
}

/// Portfolio value at every `step` boundary of the trailing `window`, replaying the holdings of
/// that moment against the close of the last candle that had finished by then, in the reporting
/// currency (`rate_to_reporting` converts the portfolio's own).
pub fn intraday_snapshots(
    transactions: &[PortfolioTransaction],
    portfolio_id: u64,
    bars: &HashMap<String, IntradayBars>,
    rate_to_reporting: f64,
    now_ms: i64,
) -> Vec<PortfolioSnapshot> {
    let own: Vec<PortfolioTransaction> = transactions
        .iter()
        .filter(|transaction| transaction.portfolio_id == portfolio_id)
        .cloned()
        .collect();
    let Some(first) = own.iter().map(|transaction| transaction.timestamp).min() else {
        return Vec::new();
    };
    let assets = crate::model::portfolio::accounting::unique_assets(&own);
    let ids = HashSet::from([portfolio_id]);
    let end = now_ms.div_euclid(INTRADAY_STEP_MS) * INTRADAY_STEP_MS;
    let mut points = Vec::new();
    let mut slot = end - INTRADAY_WINDOW_MS;
    while slot <= end {
        if slot >= first {
            let quotes: HashMap<String, PortfolioQuote> = assets
                .iter()
                .filter_map(|asset| {
                    let series = bars.get(&asset.key)?;
                    let bar = series
                        .bars
                        .iter()
                        .rev()
                        .find(|bar| bar.open_time + series.span_ms <= slot)?;
                    (slot - bar.open_time <= LOOKBACK_MS).then(|| {
                        (
                            asset.key.clone(),
                            PortfolioQuote {
                                price: bar.close * rate_to_reporting,
                                previous_day_price: None,
                                timestamp: bar.open_time,
                            },
                        )
                    })
                })
                .collect();
            points.push(snapshot_at(portfolio_id, &ids, &own, &quotes, slot));
        }
        slot += INTRADAY_STEP_MS;
    }
    points
}

/// Several portfolios' intraday series added up, point for point.
pub fn sum_intraday(series: &[Vec<PortfolioSnapshot>]) -> Vec<PortfolioSnapshot> {
    let mut by_time: BTreeMap<i64, Vec<&PortfolioSnapshot>> = BTreeMap::new();
    for point in series.iter().flatten() {
        by_time.entry(point.timestamp).or_default().push(point);
    }
    by_time
        .into_iter()
        .map(|(timestamp, points)| PortfolioSnapshot {
            portfolio_id: 0,
            timestamp,
            value: points.iter().map(|point| point.value).sum(),
            net_contributions: points.iter().map(|point| point.net_contributions).sum(),
            realized_pnl: points.iter().map(|point| point.realized_pnl).sum(),
            unrealized_pnl: points.iter().map(|point| point.unrealized_pnl).sum(),
            is_complete: points.iter().all(|point| point.is_complete),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::{PortfolioCurrency, PortfolioSource, PortfolioTransactionType};

    fn asset() -> PortfolioAsset {
        PortfolioAsset {
            key: "binance:BTCUSDT".into(),
            symbol: "BTC".into(),
            name: "Bitcoin".into(),
            source: DataSource::Binance,
            quote_currency: PortfolioCurrency::Usd,
            metadata: HashMap::new(),
        }
    }

    fn buy(id: u64, timestamp: i64, quantity: f64, price: f64) -> PortfolioTransaction {
        PortfolioTransaction {
            id,
            portfolio_id: 1,
            asset: asset(),
            kind: PortfolioTransactionType::Buy,
            quantity,
            price: Some(price),
            price_currency: PortfolioCurrency::Usd,
            fee: 0.0,
            fee_currency: PortfolioCurrency::Usd,
            timestamp,
            notes: String::new(),
            source: PortfolioSource::Manual,
            external_transaction_id: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn bar(open_time: i64, close: f64) -> Kline {
        Kline {
            open_time,
            close_time: open_time + DAY_MS - 1,
            open: close,
            high: close,
            low: close,
            close,
            volume: 1.0,
        }
    }

    fn snapshot_point(timestamp: i64) -> PortfolioSnapshot {
        PortfolioSnapshot {
            portfolio_id: 1,
            timestamp,
            value: 1.0,
            net_contributions: 1.0,
            realized_pnl: 0.0,
            unrealized_pnl: 0.0,
            is_complete: true,
        }
    }

    #[test]
    fn start_of_day_follows_the_offset() {
        // 2024-01-02 01:00 UTC is still Jan 1 in UTC-5, whose midnight is 05:00 UTC.
        let ms = 1_704_157_200_000;
        assert_eq!(start_of_day(ms, 0), 1_704_153_600_000);
        assert_eq!(start_of_day(ms, -5 * 3_600), 1_704_085_200_000);
    }

    #[test]
    fn rebuild_starts_where_the_history_is_missing() {
        let day = |n: i64| n * DAY_MS;
        let mut ledger = PortfolioLedgerSnapshot::default();
        ledger.transactions.push(buy(1, day(10) + 5_000, 1.0, 10.0));
        let now = day(20) + 7 * 3_600_000;

        // Nothing stored yet: the first transaction's day.
        assert_eq!(rebuild_start(1, &ledger, now, 0), Some(day(10)));
        // Another portfolio has no transactions at all.
        assert_eq!(rebuild_start(2, &ledger, now, 0), None);

        // Stored through day 15: the day after.
        ledger.historical_snapshots.push(snapshot_point(day(15)));
        assert_eq!(rebuild_start(1, &ledger, now, 0), Some(day(16)));

        // Stored through today: nothing to do.
        ledger.historical_snapshots.push(snapshot_point(day(20)));
        assert_eq!(rebuild_start(1, &ledger, now, 0), None);

        // An edit invalidates from its day, never past today.
        ledger.invalidated_after.insert(1, day(12) + 9_999);
        assert_eq!(rebuild_start(1, &ledger, now, 0), Some(day(12)));
        ledger.invalidated_after.insert(1, day(40));
        assert_eq!(rebuild_start(1, &ledger, now, 0), Some(day(20)));
    }

    #[test]
    fn snapshots_keep_the_realized_pnl_of_a_closed_position() {
        let day = |n: i64| n * DAY_MS;
        let mut sell = buy(2, day(3) + 1_000, 1.0, 15.0);
        sell.kind = PortfolioTransactionType::Sell;
        let transactions = vec![buy(1, day(1) + 1_000, 1.0, 10.0), sell];
        let bars = HashMap::from([(
            "binance:BTCUSDT".to_owned(),
            (1..=6).map(|n| bar(day(n), 15.0)).collect(),
        )]);
        let points = build_snapshots(1, &transactions, &[asset()], &bars, day(1), day(6), 0);
        // Closed on day 3: nothing held, but the 5 earned stays in every later point.
        assert_eq!(points[3].value, 0.0);
        assert_eq!(points[3].realized_pnl, 5.0);
        assert_eq!(points[5].realized_pnl, 5.0);
        assert_eq!(points[1].realized_pnl, 0.0);
    }

    #[test]
    fn daily_snapshots_price_holdings_from_the_last_finished_bar() {
        let day = |n: i64| n * DAY_MS;
        let transactions = vec![buy(1, day(2) + 1_000, 2.0, 10.0)];
        let bars = HashMap::from([(
            "binance:BTCUSDT".to_owned(),
            vec![bar(day(1), 10.0), bar(day(3), 12.0), bar(day(4), 15.0)],
        )]);
        let points = build_snapshots(1, &transactions, &[asset()], &bars, day(1), day(9), 0);
        assert_eq!(points.len(), 9);

        // Day 1: not bought yet, and no bar has finished.
        assert_eq!(points[0].value, 0.0);
        assert_eq!(points[0].net_contributions, 0.0);
        // Day 2 midnight is before the purchase. Day 3 midnight holds 2 BTC at the close of
        // the day-1 bar, the last one to have finished (day 2 has no bar).
        assert_eq!(points[1].value, 0.0);
        assert_eq!(points[2].value, 20.0);
        assert_eq!(points[2].net_contributions, 20.0);
        assert!(points[2].is_complete);
        assert_eq!(points[2].unrealized_pnl, 0.0);
        // Day 4 midnight sees the day-3 close, day 5 the day-4 close, and that bar prices the
        // days after it until it is more than 3 days old.
        assert_eq!(points[3].value, 24.0);
        assert_eq!(points[4].value, 30.0);
        assert_eq!(points[7].value, 30.0);
        assert_eq!(points[8].value, 0.0);
        assert!(!points[8].is_complete);
    }

    #[test]
    fn a_point_never_uses_a_candle_that_is_still_forming() {
        let day = |n: i64| n * DAY_MS;
        // Bought during day 3, whose own candle closes at 50 after the buy at 10.
        let transactions = vec![buy(1, day(3) + 3_600_000, 1.0, 10.0)];
        let bars = HashMap::from([(
            "binance:BTCUSDT".to_owned(),
            vec![bar(day(2), 10.0), bar(day(3), 50.0)],
        )]);
        let points = build_snapshots(1, &transactions, &[asset()], &bars, day(3), day(5), 0);
        // Midnight of day 3: nothing held yet, however the day ended.
        assert_eq!(points[0].value, 0.0);
        // Midnight of day 4 holds it, priced at day 3's finished close, and counts its cost.
        assert_eq!(points[1].value, 50.0);
        assert_eq!(points[1].net_contributions, 10.0);
        assert_eq!(points[1].unrealized_pnl, 40.0);
    }

    /// New York in 2024: 5 hours behind, 4 hours from 2 am on day 10 (07:00 UTC).
    fn spring_forward(ms: i64) -> i64 {
        if ms < 10 * DAY_MS + 7 * 3_600_000 {
            -5 * 3_600
        } else {
            -4 * 3_600
        }
    }

    #[test]
    fn a_daylight_saving_day_is_23_hours_between_two_midnights() {
        let day = |n: i64| n * DAY_MS;
        let points = build_snapshots(
            1,
            &[],
            &[],
            &HashMap::new(),
            day(8),
            day(13),
            spring_forward,
        );
        let gaps: Vec<i64> = points
            .windows(2)
            .map(|pair| (pair[1].timestamp - pair[0].timestamp) / 3_600_000)
            .collect();
        assert!(gaps.contains(&23), "{gaps:?}");
        assert!(gaps.iter().all(|gap| *gap == 24 || *gap == 23), "{gaps:?}");
        // Every point is a local midnight in the offset of its own day.
        for point in &points {
            let local = point.timestamp + spring_forward(point.timestamp) * 1_000;
            assert_eq!(local.rem_euclid(DAY_MS), 0, "{}", point.timestamp);
        }
    }

    #[test]
    fn rebuild_start_steps_over_a_daylight_saving_day() {
        let day = |n: i64| n * DAY_MS;
        let mut ledger = PortfolioLedgerSnapshot::default();
        ledger.transactions.push(buy(1, day(5), 1.0, 10.0));
        // Stored through the midnight before the 23-hour day.
        let before = start_of_day(day(10) + 12 * 3_600_000, spring_forward);
        ledger.historical_snapshots.push(snapshot_point(before));
        let now = day(14);
        let next = rebuild_start(1, &ledger, now, spring_forward).unwrap();
        assert_eq!(next - before, 23 * 3_600_000);
    }

    #[test]
    fn fingerprints_change_with_an_edit_but_not_with_order() {
        let a = buy(1, 100, 1.0, 10.0);
        let b = buy(2, 200, 2.0, 11.0);
        let both = transactions_fingerprint(&[a.clone(), b.clone()]);
        assert_eq!(both, transactions_fingerprint(&[b.clone(), a.clone()]));
        let mut edited = a.clone();
        edited.quantity = 1.5;
        assert_ne!(both, transactions_fingerprint(&[edited, b.clone()]));
        assert_ne!(both, transactions_fingerprint(&[a]));
    }

    #[test]
    fn intraday_slots_use_the_last_finished_candle() {
        let step = INTRADAY_STEP_MS;
        let now = 100 * DAY_MS + 3 * step + 1_234;
        let transactions = vec![buy(1, now - 2 * DAY_MS, 2.0, 10.0)];
        // 15-minute candles over the last day, closing at their index.
        let span = 15 * 60 * 1_000;
        let first_open = (now - INTRADAY_WINDOW_MS) / span * span - span;
        let candles: Vec<Kline> = (0..120)
            .map(|index| Kline {
                open_time: first_open + index * span,
                close_time: first_open + (index + 1) * span - 1,
                open: 0.0,
                high: 0.0,
                low: 0.0,
                close: index as f64,
                volume: 0.0,
            })
            .collect();
        let bars = HashMap::from([(
            "binance:BTCUSDT".to_owned(),
            IntradayBars {
                bars: candles.clone(),
                span_ms: span,
            },
        )]);
        let points = intraday_snapshots(&transactions, 1, &bars, 2.0, now);
        assert_eq!(points.len(), (INTRADAY_WINDOW_MS / step + 1) as usize);
        let last = points.last().unwrap();
        assert_eq!(last.timestamp, now.div_euclid(step) * step);
        // The candle that opens at slot - span has just finished: close × quantity × rate.
        let finished = candles
            .iter()
            .rev()
            .find(|bar| bar.open_time + span <= last.timestamp)
            .unwrap();
        assert_eq!(last.value, finished.close * 2.0 * 2.0);
        assert!(
            points
                .windows(2)
                .all(|pair| pair[0].timestamp < pair[1].timestamp)
        );
    }

    #[test]
    fn intraday_skips_slots_before_the_first_transaction_and_sums_series() {
        let now = 50 * DAY_MS;
        let transactions = vec![buy(1, now - 3_600_000, 1.0, 10.0)];
        let points = intraday_snapshots(&transactions, 1, &HashMap::new(), 1.0, now);
        assert!(!points.is_empty() && points.len() <= 3);
        assert!(points.iter().all(|point| !point.is_complete));

        let a = vec![snapshot_point(1), snapshot_point(2)];
        let b = vec![snapshot_point(2), snapshot_point(3)];
        let sum = sum_intraday(&[a, b]);
        assert_eq!(
            sum.iter().map(|point| point.timestamp).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(sum[1].value, 2.0);
    }
}
