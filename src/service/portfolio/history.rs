//! Candle fetching behind the portfolio value chart: the daily history rebuild and the 1D
//! chart's intraday replay. The arithmetic is `model::portfolio::history`; this is the
//! blocking provider boundary and the worker bookkeeping around it.
//!
//! Both run on `service::work_queue` threads and report over `mpsc` channels the UI drains each
//! frame, like the quote feed.

use crate::lock::lock;
use crate::model::DataSource;
use crate::model::Kline;
use crate::model::portfolio::accounting::{holdings, unique_assets};
use crate::model::portfolio::history::{
    DAY_MS, IntradayBars, LOOKBACK_MS, build_snapshots, intraday_snapshots, sum_intraday,
    transactions_fingerprint,
};
use crate::model::portfolio::{
    Portfolio, PortfolioAsset, PortfolioCurrency, PortfolioSnapshot, PortfolioTransaction,
};
use crate::service::portfolio::quotes::PortfolioFxRates;
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

/// The most daily bars one request asks for; Binance caps a page at 1000.
const MAX_BARS: u32 = 1_000;
/// Providers are asked about this many assets at once.
const PARALLEL_FETCHES: usize = 6;
/// A failed rebuild waits this long before the next attempt.
const RETRY_MS: i64 = 60_000;
/// A rebuild that priced only some of the assets is stored, and tried again after this long.
const PARTIAL_RETRY_MS: i64 = 10 * 60_000;
/// Fresh enough that a 1D refresh reuses the candles it already has.
const INTRADAY_FRESH_MS: i64 = 5 * 60 * 1_000;

/// The symbol a data source understands: the key minus its `source:` prefix.
pub fn market_symbol(asset: &PortfolioAsset) -> String {
    asset
        .key
        .split_once(':')
        .map(|(_, symbol)| symbol)
        .filter(|symbol| !symbol.is_empty())
        .unwrap_or(&asset.symbol)
        .to_owned()
}

/// Whole days spanned, plus slack for a partial first and current day.
pub(crate) fn days(span_ms: i64) -> u32 {
    (span_ms.max(0) as f64 / DAY_MS as f64).ceil() as u32 + 2
}

/// Ascending daily candles covering `start` (and the days just before it) through now.
/// A failed request is an empty series: that asset simply goes unpriced. A span of more than
/// `MAX_BARS` days asks the provider for a time range where it has one; otherwise the newest
/// `MAX_BARS` are all there is, and the caller sees where they begin.
pub(crate) fn daily_bars(asset: &PortfolioAsset, start_ms: i64, now_ms: i64) -> Vec<Kline> {
    let first = start_ms - LOOKBACK_MS;
    let wanted = days(now_ms - first);
    let symbol = market_symbol(asset);
    let recent = |limit: u32| crate::service::fetch_klines(asset.source, &symbol, "1d", limit);
    let mut bars = if wanted > MAX_BARS {
        crate::service::fetch_klines_range(
            asset.source,
            &symbol,
            "1d",
            first - DAY_MS,
            now_ms + DAY_MS,
            wanted as usize,
        )
        .ok()
        .filter(|bars| !bars.is_empty())
        .or_else(|| recent(MAX_BARS).ok())
        .unwrap_or_default()
    } else {
        recent(wanted).unwrap_or_default()
    };
    bars.sort_by_key(|bar| bar.open_time);
    bars
}

/// The last day of fine candles. Alpaca has no 15-minute timeframe, so it is sampled hourly.
fn intraday_bars(asset: &PortfolioAsset) -> Option<IntradayBars> {
    let hourly = asset.source == DataSource::Alpaca;
    let (interval, limit, span_ms) = if hourly {
        ("1h", 48, 3_600_000)
    } else {
        ("15m", 120, 900_000)
    };
    let mut bars =
        crate::service::fetch_klines(asset.source, &market_symbol(asset), interval, limit).ok()?;
    if bars.is_empty() {
        return None;
    }
    bars.sort_by_key(|bar| bar.open_time);
    Some(IntradayBars { bars, span_ms })
}

/// Runs `fetch` for every asset, a few at a time, collecting what came back.
pub(crate) fn fetch_all<T: Send>(
    assets: &[PortfolioAsset],
    fetch: impl Fn(&PortfolioAsset) -> Option<T> + Sync,
) -> HashMap<String, T> {
    let mut result = HashMap::new();
    for chunk in assets.chunks(PARALLEL_FETCHES) {
        std::thread::scope(|scope| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|asset| {
                    let fetch = &fetch;
                    (asset.key.clone(), scope.spawn(move || fetch(asset)))
                })
                .collect();
            for (key, handle) in handles {
                if let Some(value) = handle.join().ok().flatten() {
                    result.insert(key, value);
                }
            }
        });
    }
    result
}

// MARK: Daily rebuild

/// A finished rebuild. `snapshots` is `None` when nothing could be priced, so the caller keeps
/// the history as it is and tries again later instead of storing a column of zeros.
pub struct HistoryResult {
    pub portfolio_id: u64,
    pub from: i64,
    pub snapshots: Option<Vec<PortfolioSnapshot>>,
    /// Never retry: no asset of the portfolio can be priced in its currency.
    pub hopeless: bool,
    /// Some asset that could have been priced had no candles (a failed request, usually), so
    /// the snapshots leave it out and the rebuild is worth repeating.
    pub incomplete: bool,
    /// [`transactions_fingerprint`] of the transactions it was computed from: the result is
    /// stored only while the portfolio still has exactly these.
    pub basis: u64,
}

/// Rebuilds portfolios' daily history off the UI thread.
pub struct HistoryRebuilder {
    sender: Sender<HistoryResult>,
    receiver: Receiver<HistoryResult>,
    in_flight: HashSet<u64>,
    retry_after: HashMap<u64, i64>,
    /// Portfolios whose stored history is missing an asset: where to rebuild from, and when.
    partial: HashMap<u64, (i64, i64)>,
}

impl Default for HistoryRebuilder {
    fn default() -> Self {
        let (sender, receiver) = channel();
        Self {
            sender,
            receiver,
            in_flight: HashSet::new(),
            retry_after: HashMap::new(),
            partial: HashMap::new(),
        }
    }
}

impl HistoryRebuilder {
    pub fn is_rebuilding(&self, portfolio_id: u64) -> bool {
        self.in_flight.contains(&portfolio_id)
    }

    /// Whether a rebuild may start for this portfolio now.
    pub fn can_start(&self, portfolio_id: u64, now_ms: i64) -> bool {
        !self.in_flight.contains(&portfolio_id)
            && self
                .retry_after
                .get(&portfolio_id)
                .is_none_or(|after| now_ms >= *after)
    }

    /// Where to rebuild `portfolio_id` from to fill in an asset a stored history lacks, once
    /// it is time to look again.
    pub fn partial_due(&self, portfolio_id: u64, now_ms: i64) -> Option<i64> {
        self.partial
            .get(&portfolio_id)
            .filter(|(_, after)| now_ms >= *after)
            .map(|(from, _)| *from)
    }

    /// Holds back the next rebuild of `portfolio_id` for a while, after its result could not be
    /// stored.
    pub fn defer(&mut self, portfolio_id: u64, now_ms: i64) {
        self.retry_after.insert(portfolio_id, now_ms + RETRY_MS);
    }

    /// Finished rebuilds, with their bookkeeping settled.
    pub fn poll(&mut self, now_ms: i64) -> Vec<HistoryResult> {
        let finished: Vec<HistoryResult> = self.receiver.try_iter().collect();
        for result in &finished {
            self.in_flight.remove(&result.portfolio_id);
            if result.snapshots.is_none() {
                let after = if result.hopeless {
                    i64::MAX
                } else {
                    now_ms + RETRY_MS
                };
                self.retry_after.insert(result.portfolio_id, after);
            } else {
                self.retry_after.remove(&result.portfolio_id);
                if result.incomplete {
                    self.partial.insert(
                        result.portfolio_id,
                        (result.from, now_ms + PARTIAL_RETRY_MS),
                    );
                } else {
                    self.partial.remove(&result.portfolio_id);
                }
            }
        }
        finished
    }

    /// Starts rebuilding `portfolio`'s history from `from`. `transactions` are its own.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        &mut self,
        ctx: &egui::Context,
        portfolio: &Portfolio,
        transactions: Vec<PortfolioTransaction>,
        from: i64,
        now_ms: i64,
        zone: fn(i64) -> i64,
        fx: Arc<dyn PortfolioFxRates>,
    ) {
        self.in_flight.insert(portfolio.id);
        let sender = self.sender.clone();
        let ctx = ctx.clone();
        let portfolio_id = portfolio.id;
        let base = portfolio.base_currency;
        crate::service::work_queue::submit(move || {
            let result = rebuild(
                portfolio_id,
                base,
                &transactions,
                from,
                now_ms,
                zone,
                &*fx,
                &daily_bars,
            );
            let _ = sender.send(result);
            ctx.request_repaint();
        });
    }
}

/// Rebuilds one portfolio's daily history. `candles` fetches an asset's daily candles from
/// `from` (a function, so a test can serve its own). Candles quoted in another currency than
/// the portfolio's are converted at each day's rate, as the live valuation converts.
#[allow(clippy::too_many_arguments)]
fn rebuild(
    portfolio_id: u64,
    base: PortfolioCurrency,
    transactions: &[PortfolioTransaction],
    from: i64,
    now_ms: i64,
    zone: fn(i64) -> i64,
    fx: &dyn PortfolioFxRates,
    candles: &(dyn Fn(&PortfolioAsset, i64, i64) -> Vec<Kline> + Sync),
) -> HistoryResult {
    let basis = transactions_fingerprint(transactions);
    let nothing = |incomplete| HistoryResult {
        portfolio_id,
        from,
        snapshots: None,
        hopeless: false,
        incomplete,
        basis,
    };
    // A ledger that cannot be replayed would come out as a column of zeros. Keep the history as
    // it is and look again later, once the transactions are fixed.
    let own_ids = HashSet::from([portfolio_id]);
    if holdings(transactions, &own_ids, None, &HashMap::new(), 0).is_err() {
        return nothing(false);
    }
    let assets = unique_assets(transactions);
    let mut bars = fetch_all(&assets, |asset| {
        let series = candles(asset, from, now_ms);
        let series = if asset.quote_currency == base {
            series
        } else {
            in_currency(series, asset.quote_currency, base, fx)
        };
        Some(series).filter(|series| !series.is_empty())
    });
    if bars.is_empty() {
        return nothing(false);
    }
    // Where an asset's candles begin, when that is later than its first transaction needs: the
    // days before are unpriced, not worth nothing, so no point is made for them.
    let covered_from = assets
        .iter()
        .filter_map(|asset| {
            let series = bars.get(&asset.key)?;
            let first_bar = series.first()?.open_time;
            let first_held = transactions
                .iter()
                .filter(|tx| tx.asset.key == asset.key)
                .map(|tx| tx.timestamp)
                .min()?;
            (first_bar > first_held + LOOKBACK_MS + DAY_MS).then_some(first_bar + DAY_MS)
        })
        .max();
    let incomplete = bars.len() < assets.len();
    let mut snapshots = build_snapshots(
        portfolio_id,
        transactions,
        &assets,
        &bars,
        from,
        now_ms,
        zone,
    );
    if let Some(covered_from) = covered_from {
        snapshots.retain(|point| point.timestamp >= covered_from);
    }
    bars.clear();
    HistoryResult {
        portfolio_id,
        from,
        snapshots: Some(snapshots),
        hopeless: false,
        incomplete,
        basis,
    }
}

/// `series` priced in `to` instead of `from_currency`, each candle at its own day's rate. A day
/// with no rate has no candle: the asset goes unpriced for it rather than at a wrong price.
fn in_currency(
    series: Vec<Kline>,
    from_currency: PortfolioCurrency,
    to: PortfolioCurrency,
    fx: &dyn PortfolioFxRates,
) -> Vec<Kline> {
    let dates: Vec<i64> = series.iter().map(|bar| bar.open_time).collect();
    let rates = fx.conversions(from_currency, to, &dates);
    series
        .into_iter()
        .filter_map(|bar| {
            let rate = rates.get(&bar.open_time)?.as_ref().ok()?.rate;
            Some(Kline {
                open: bar.open * rate,
                high: bar.high * rate,
                low: bar.low * rate,
                close: bar.close * rate,
                ..bar
            })
        })
        .collect()
}

// MARK: Intraday

/// The inputs an intraday series is computed from; a change in any of them recomputes it.
#[derive(Clone, Debug, PartialEq)]
pub struct IntradayRequest {
    /// Portfolios in scope with their base currency.
    pub portfolios: Vec<(u64, PortfolioCurrency)>,
    pub reporting_currency: PortfolioCurrency,
    /// The scope's transactions, already in the reporting currency.
    pub transactions: Vec<PortfolioTransaction>,
}

#[derive(Default)]
struct IntradayState {
    bars: HashMap<String, (i64, IntradayBars)>,
    in_flight: HashSet<String>,
    /// The series for `computed_for`, at the 30-minute `bucket` it was computed in.
    points: Vec<PortfolioSnapshot>,
    computed_for: Option<(IntradayRequest, i64)>,
    working: bool,
}

/// The last 24 hours of portfolio value at 30-minute spacing, for the 1D chart. Empty when the
/// selection can't be replayed (a non-USD portfolio, no candles yet), so callers fall back to
/// the daily history.
#[derive(Clone, Default)]
pub struct IntradayFeed {
    state: Arc<Mutex<IntradayState>>,
}

impl IntradayFeed {
    /// The current series, kicking off a fetch/recompute when it is stale. Call every frame
    /// while the 1D range is showing.
    pub fn points(
        &self,
        ctx: &egui::Context,
        request: &IntradayRequest,
        fx: &Arc<dyn PortfolioFxRates>,
        now_ms: i64,
    ) -> Vec<PortfolioSnapshot> {
        let bucket = now_ms.div_euclid(crate::model::portfolio::history::INTRADAY_STEP_MS);
        let mut state = lock(&self.state);
        let up_to_date = state
            .computed_for
            .as_ref()
            .is_some_and(|(computed, at)| computed == request && *at == bucket);
        let needs_bars = request
            .transactions
            .iter()
            .map(|transaction| &transaction.asset)
            .any(|asset| {
                asset.quote_currency == PortfolioCurrency::Usd
                    && !state.in_flight.contains(&asset.key)
                    && state
                        .bars
                        .get(&asset.key)
                        .is_none_or(|(fetched, _)| now_ms - fetched >= INTRADAY_FRESH_MS)
            });
        if !state.working && (!up_to_date || needs_bars) {
            state.working = true;
            let shared = Arc::clone(&self.state);
            let request = request.clone();
            let fx = Arc::clone(fx);
            let ctx = ctx.clone();
            crate::service::work_queue::submit(move || {
                refresh(&shared, &request, fx.as_ref(), now_ms, bucket);
                ctx.request_repaint();
            });
        }
        state.points.clone()
    }
}

fn refresh(
    shared: &Mutex<IntradayState>,
    request: &IntradayRequest,
    fx: &dyn PortfolioFxRates,
    now_ms: i64,
    bucket: i64,
) {
    let replayable = !request.portfolios.is_empty()
        && request
            .portfolios
            .iter()
            .all(|(_, base)| *base == PortfolioCurrency::Usd);
    if !replayable {
        let mut state = lock(shared);
        state.points.clear();
        state.computed_for = Some((request.clone(), bucket));
        state.working = false;
        return;
    }

    let wanted: Vec<PortfolioAsset> = unique_assets(&request.transactions)
        .into_iter()
        .filter(|asset| asset.quote_currency == PortfolioCurrency::Usd)
        .filter(|asset| {
            let state = lock(shared);
            !state.in_flight.contains(&asset.key)
                && state
                    .bars
                    .get(&asset.key)
                    .is_none_or(|(fetched, _)| now_ms - fetched >= INTRADAY_FRESH_MS)
        })
        .collect();
    lock(shared)
        .in_flight
        .extend(wanted.iter().map(|asset| asset.key.clone()));
    let fetched = fetch_all(&wanted, intraday_bars);
    {
        let mut state = lock(shared);
        for asset in &wanted {
            state.in_flight.remove(&asset.key);
        }
        // A failed fetch is remembered as no candles, so it is retried after the freshness
        // window rather than on every frame.
        for asset in &wanted {
            let bars = fetched.get(&asset.key).cloned().unwrap_or_default();
            state.bars.insert(asset.key.clone(), (now_ms, bars));
        }
    }

    let bars: HashMap<String, IntradayBars> = lock(shared)
        .bars
        .iter()
        .map(|(key, (_, bars))| (key.clone(), bars.clone()))
        .collect();
    let rate = if request.reporting_currency == PortfolioCurrency::Usd {
        Some(1.0)
    } else {
        fx.conversion(PortfolioCurrency::Usd, request.reporting_currency, None)
            .ok()
            .map(|conversion| conversion.rate)
    };
    let series: Vec<Vec<PortfolioSnapshot>> = match rate {
        Some(rate) => request
            .portfolios
            .iter()
            .map(|(id, _)| intraday_snapshots(&request.transactions, *id, &bars, rate, now_ms))
            .collect(),
        None => Vec::new(),
    };
    let points = match series.len() {
        0 => Vec::new(),
        1 => series.into_iter().next().unwrap_or_default(),
        _ => sum_intraday(&series),
    };
    let mut state = lock(shared);
    state.points = points;
    state.computed_for = Some((request.clone(), bucket));
    state.working = false;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::portfolio::{PortfolioSource, PortfolioTransactionType};
    use crate::service::market::fx_rates::Conversion;

    /// Dollars are worth 0.9 euros, except on `no_rate_on`.
    struct Fx {
        no_rate_on: Option<i64>,
    }

    impl PortfolioFxRates for Fx {
        fn conversion(
            &self,
            _: PortfolioCurrency,
            _: PortfolioCurrency,
            _: Option<i64>,
        ) -> Result<Conversion, String> {
            unreachable!("history asks for dated rates")
        }

        fn conversions(
            &self,
            _: PortfolioCurrency,
            _: PortfolioCurrency,
            dates: &[i64],
        ) -> HashMap<i64, Result<Conversion, String>> {
            dates
                .iter()
                .map(|date| {
                    let answer = if Some(*date) == self.no_rate_on {
                        Err("no rate".to_owned())
                    } else {
                        Ok(Conversion {
                            rate: 0.9,
                            observation_ms: *date,
                            is_cached: false,
                        })
                    };
                    (*date, answer)
                })
                .collect()
        }
    }

    fn asset(key: &str, quote_currency: PortfolioCurrency) -> PortfolioAsset {
        PortfolioAsset {
            key: key.into(),
            symbol: key.into(),
            name: key.into(),
            source: DataSource::Binance,
            quote_currency,
            metadata: HashMap::new(),
        }
    }

    fn trade(
        id: u64,
        asset: PortfolioAsset,
        kind: PortfolioTransactionType,
        quantity: f64,
        timestamp: i64,
    ) -> PortfolioTransaction {
        PortfolioTransaction {
            id,
            portfolio_id: 1,
            asset,
            kind,
            quantity,
            price: Some(100.0),
            price_currency: PortfolioCurrency::Usd,
            fee: 0.0,
            fee_currency: PortfolioCurrency::Usd,
            timestamp,
            notes: String::new(),
            source: PortfolioSource::Manual,
            external_transaction_id: None,
            created_at: timestamp,
            updated_at: timestamp,
        }
    }

    fn buy(id: u64, asset: PortfolioAsset, timestamp: i64) -> PortfolioTransaction {
        trade(id, asset, PortfolioTransactionType::Buy, 1.0, timestamp)
    }

    /// One candle a day at 100, from day `first` through day `last`.
    fn flat(first: i64, last: i64) -> Vec<Kline> {
        (first..=last)
            .map(|day| Kline {
                open_time: day * DAY_MS,
                close_time: (day + 1) * DAY_MS - 1,
                open: 100.0,
                high: 100.0,
                low: 100.0,
                close: 100.0,
                volume: 1.0,
            })
            .collect()
    }

    fn run(
        base: PortfolioCurrency,
        transactions: &[PortfolioTransaction],
        from: i64,
        now: i64,
        fx: &Fx,
        candles: &(dyn Fn(&PortfolioAsset, i64, i64) -> Vec<Kline> + Sync),
    ) -> HistoryResult {
        rebuild(1, base, transactions, from, now, |_| 0, fx, candles)
    }

    #[test]
    fn a_ledger_that_cannot_be_replayed_is_not_rebuilt_into_zeros() {
        let sale = trade(
            1,
            asset("btc", PortfolioCurrency::Usd),
            PortfolioTransactionType::Sell,
            1.0,
            1_000,
        );
        // Sells what was never bought: no snapshots to store, and a later retry (not "hopeless").
        let result = run(
            PortfolioCurrency::Usd,
            &[sale],
            0,
            10 * DAY_MS,
            &Fx { no_rate_on: None },
            &|_, _, _| panic!("no candles are asked for"),
        );
        assert!(result.snapshots.is_none());
        assert!(!result.hopeless);
    }

    #[test]
    fn an_asset_quoted_in_another_currency_is_converted_not_dropped() {
        let btc = asset("btc", PortfolioCurrency::Usd);
        let result = run(
            PortfolioCurrency::Eur,
            &[buy(1, btc, 2 * DAY_MS)],
            2 * DAY_MS,
            10 * DAY_MS,
            &Fx { no_rate_on: None },
            &|_, _, _| flat(0, 10),
        );
        let points = result.snapshots.expect("snapshots");
        assert!(!result.hopeless && !result.incomplete);
        // 1 BTC at 100 dollars is 90 euros, from the day after the purchase.
        let held = points.iter().find(|p| p.timestamp == 4 * DAY_MS).unwrap();
        assert!((held.value - 90.0).abs() < 1e-9, "{}", held.value);
        assert!(held.is_complete);
    }

    #[test]
    fn a_day_without_a_rate_is_unpriced_not_priced_wrongly() {
        let btc = asset("btc", PortfolioCurrency::Usd);
        let result = run(
            PortfolioCurrency::Eur,
            &[buy(1, btc, 2 * DAY_MS)],
            2 * DAY_MS,
            10 * DAY_MS,
            &Fx {
                no_rate_on: Some(5 * DAY_MS),
            },
            &|_, _, _| flat(0, 10),
        );
        let points = result.snapshots.unwrap();
        let at = |day: i64| points.iter().find(|p| p.timestamp == day * DAY_MS).unwrap();
        // The day-5 candle is gone, so day 6 falls back to the day-4 candle (inside the
        // look-back) rather than inventing a price.
        assert!((at(6).value - 90.0).abs() < 1e-9);
        assert!(at(6).is_complete);
    }

    #[test]
    fn history_older_than_the_candles_begins_where_they_do() {
        let btc = asset("btc", PortfolioCurrency::Usd);
        // Bought on day 500, but the provider's newest 1,000 days begin on day 1000.
        let now = 2_000 * DAY_MS;
        let result = run(
            PortfolioCurrency::Usd,
            &[buy(1, btc, 500 * DAY_MS)],
            500 * DAY_MS,
            now,
            &Fx { no_rate_on: None },
            &|_, _, _| flat(1_000, 2_000),
        );
        let points = result.snapshots.unwrap();
        let first = points.first().unwrap();
        assert_eq!(first.timestamp, 1_001 * DAY_MS, "first finished candle");
        assert!(
            points.iter().all(|p| p.is_complete && p.value > 0.0),
            "no leading run of zeros"
        );
    }

    #[test]
    fn one_asset_missing_marks_the_rebuild_incomplete_but_keeps_the_rest() {
        let btc = asset("btc", PortfolioCurrency::Usd);
        let eth = asset("eth", PortfolioCurrency::Usd);
        let result = run(
            PortfolioCurrency::Usd,
            &[buy(1, btc, 2 * DAY_MS), buy(2, eth, 2 * DAY_MS)],
            2 * DAY_MS,
            10 * DAY_MS,
            &Fx { no_rate_on: None },
            &|asset, _, _| {
                if asset.key == "btc" {
                    flat(0, 10)
                } else {
                    Vec::new()
                }
            },
        );
        assert!(result.incomplete);
        let points = result.snapshots.unwrap();
        let last = points.last().unwrap();
        assert!((last.value - 100.0).abs() < 1e-9, "btc priced, eth not");
        assert!(!last.is_complete);
    }

    fn finished(portfolio_id: u64, incomplete: bool) -> HistoryResult {
        HistoryResult {
            portfolio_id,
            from: 5_000,
            snapshots: Some(Vec::new()),
            hopeless: false,
            incomplete,
            basis: 0,
        }
    }

    #[test]
    fn a_rebuild_missing_an_asset_is_stored_and_tried_again_later() {
        let mut history = HistoryRebuilder::default();
        history.sender.send(finished(1, true)).unwrap();
        assert_eq!(history.poll(1_000).len(), 1);
        assert_eq!(history.partial_due(1, 1_000), None, "not yet");
        assert_eq!(
            history.partial_due(1, 1_000 + PARTIAL_RETRY_MS),
            Some(5_000),
            "from where it was rebuilt"
        );
        // Another portfolio is unaffected.
        assert_eq!(history.partial_due(2, i64::MAX), None);
        // A complete rebuild ends the retrying.
        history.sender.send(finished(1, false)).unwrap();
        history.poll(2_000);
        assert_eq!(history.partial_due(1, i64::MAX), None);
    }
}
