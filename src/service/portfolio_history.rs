//! Candle fetching behind the portfolio value chart: the daily history rebuild and the 1D
//! chart's intraday replay. The arithmetic is `model::portfolio_history`; this is the
//! blocking provider boundary and the worker bookkeeping around it.
//!
//! Both run on `service::work_queue` threads and report over `mpsc` channels the UI drains each
//! frame, like the quote feed.

use crate::model::DataSource;
use crate::model::Kline;
use crate::model::portfolio::{
    Portfolio, PortfolioAsset, PortfolioCurrency, PortfolioSnapshot, PortfolioTransaction,
};
use crate::model::portfolio_accounting::{holdings, unique_assets};
use crate::model::portfolio_history::{
    DAY_MS, IntradayBars, LOOKBACK_MS, build_snapshots, intraday_snapshots, sum_intraday,
};
use crate::service::portfolio_quotes::PortfolioFxRates;
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};

/// The most daily bars one request asks for; Binance caps a page at 1000.
const MAX_BARS: u32 = 1_000;
/// Providers are asked about this many assets at once.
const PARALLEL_FETCHES: usize = 6;
/// A failed rebuild waits this long before the next attempt.
const RETRY_MS: i64 = 60_000;
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
/// A failed request is an empty series: that asset simply goes unpriced.
pub(crate) fn daily_bars(asset: &PortfolioAsset, start_ms: i64, now_ms: i64) -> Vec<Kline> {
    let limit = days(now_ms - (start_ms - LOOKBACK_MS)).min(MAX_BARS);
    let mut bars = crate::service::fetch_klines(asset.source, &market_symbol(asset), "1d", limit)
        .unwrap_or_default();
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
}

/// Rebuilds portfolios' daily history off the UI thread.
pub struct HistoryRebuilder {
    sender: Sender<HistoryResult>,
    receiver: Receiver<HistoryResult>,
    in_flight: HashSet<u64>,
    retry_after: HashMap<u64, i64>,
}

impl Default for HistoryRebuilder {
    fn default() -> Self {
        let (sender, receiver) = channel();
        Self {
            sender,
            receiver,
            in_flight: HashSet::new(),
            retry_after: HashMap::new(),
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
            }
        }
        finished
    }

    /// Starts rebuilding `portfolio`'s history from `from`. `transactions` are its own.
    pub fn start(
        &mut self,
        ctx: &egui::Context,
        portfolio: &Portfolio,
        transactions: Vec<PortfolioTransaction>,
        from: i64,
        now_ms: i64,
        utc_offset_secs: i64,
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
                utc_offset_secs,
            );
            let _ = sender.send(result);
            ctx.request_repaint();
        });
    }
}

fn rebuild(
    portfolio_id: u64,
    base: PortfolioCurrency,
    transactions: &[PortfolioTransaction],
    from: i64,
    now_ms: i64,
    utc_offset_secs: i64,
) -> HistoryResult {
    // A ledger that cannot be replayed would come out as a column of zeros. Keep the history as
    // it is and look again later, once the transactions are fixed.
    let own_ids = HashSet::from([portfolio_id]);
    if holdings(transactions, &own_ids, None, &HashMap::new(), 0).is_err() {
        return HistoryResult {
            portfolio_id,
            from,
            snapshots: None,
            hopeless: false,
        };
    }
    let assets = unique_assets(transactions);
    // A candle prices in its market's quote currency, so only markets quoted in the
    // portfolio's own currency can value it.
    let priceable: Vec<PortfolioAsset> = assets
        .iter()
        .filter(|asset| asset.quote_currency == base)
        .cloned()
        .collect();
    let bars = fetch_all(&priceable, |asset| {
        Some(daily_bars(asset, from, now_ms)).filter(|bars| !bars.is_empty())
    });
    if bars.is_empty() {
        return HistoryResult {
            portfolio_id,
            from,
            snapshots: None,
            hopeless: priceable.is_empty(),
        };
    }
    let snapshots = build_snapshots(
        portfolio_id,
        transactions,
        &assets,
        &bars,
        from,
        now_ms,
        utc_offset_secs,
    );
    HistoryResult {
        portfolio_id,
        from,
        snapshots: Some(snapshots),
        hopeless: false,
    }
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
        let bucket = now_ms.div_euclid(crate::model::portfolio_history::INTRADAY_STEP_MS);
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
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
    let lock = || shared.lock().unwrap_or_else(PoisonError::into_inner);
    let replayable = !request.portfolios.is_empty()
        && request
            .portfolios
            .iter()
            .all(|(_, base)| *base == PortfolioCurrency::Usd);
    if !replayable {
        let mut state = lock();
        state.points.clear();
        state.computed_for = Some((request.clone(), bucket));
        state.working = false;
        return;
    }

    let wanted: Vec<PortfolioAsset> = unique_assets(&request.transactions)
        .into_iter()
        .filter(|asset| asset.quote_currency == PortfolioCurrency::Usd)
        .filter(|asset| {
            let state = lock();
            !state.in_flight.contains(&asset.key)
                && state
                    .bars
                    .get(&asset.key)
                    .is_none_or(|(fetched, _)| now_ms - fetched >= INTRADAY_FRESH_MS)
        })
        .collect();
    lock()
        .in_flight
        .extend(wanted.iter().map(|asset| asset.key.clone()));
    let fetched = fetch_all(&wanted, intraday_bars);
    {
        let mut state = lock();
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

    let bars: HashMap<String, IntradayBars> = lock()
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
    let mut state = lock();
    state.points = points;
    state.computed_for = Some((request.clone(), bucket));
    state.working = false;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::portfolio::{PortfolioCurrency, PortfolioSource, PortfolioTransactionType};

    #[test]
    fn a_ledger_that_cannot_be_replayed_is_not_rebuilt_into_zeros() {
        let sale = PortfolioTransaction {
            id: 1,
            portfolio_id: 1,
            asset: PortfolioAsset {
                key: "binance:BTCUSDT".into(),
                symbol: "BTC".into(),
                name: "Bitcoin".into(),
                source: DataSource::Binance,
                quote_currency: PortfolioCurrency::Usd,
                metadata: HashMap::new(),
            },
            kind: PortfolioTransactionType::Sell,
            quantity: 1.0,
            price: Some(100.0),
            price_currency: PortfolioCurrency::Usd,
            fee: 0.0,
            fee_currency: PortfolioCurrency::Usd,
            timestamp: 1_000,
            notes: String::new(),
            source: PortfolioSource::Manual,
            external_transaction_id: None,
            created_at: 1_000,
            updated_at: 1_000,
        };
        // Sells what was never bought: no snapshots to store, and a later retry (not "hopeless").
        let result = rebuild(1, PortfolioCurrency::Usd, &[sale], 0, 10 * DAY_MS, 0);
        assert!(result.snapshots.is_none());
        assert!(!result.hopeless);
    }
}
