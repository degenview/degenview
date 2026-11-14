//! Portfolio valuation over the app-wide watchlist quote stream.
//!
//! A portfolio is another consumer of `WatchlistQuoteCoordinator`: it contributes its markets to
//! the same merged demand set as every watchlist and reads the same transient quote book. Currency
//! projection can touch the network, so it runs off the UI thread and publishes an immutable
//! snapshot when every historical/current conversion is ready.

use crate::lock::lock;
use crate::model::InstrumentId;
use crate::model::portfolio::accounting::{holdings, unique_assets};
use crate::model::portfolio::history::DAY_MS;
use crate::model::portfolio::{
    PortfolioAsset, PortfolioCurrency, PortfolioLedgerSnapshot, PortfolioQuote, PortfolioSnapshot,
    PortfolioTransaction,
};
use crate::model::watchlist::quote::Freshness;
use crate::service::market::fx_rates::{Conversion, FxRateService};
use crate::service::watchlist_quotes::{WatchlistQuoteBook, WatchlistQuoteCoordinator};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long the first valuation waits for every market to report before it publishes with
/// what it has. Publishing quote by quote makes the total climb and the holdings reshuffle as
/// each price lands; a market that never answers must not hold the portfolio back forever.
const SETTLE: Duration = Duration::from_secs(8);

/// The most often a valued portfolio is revalued for price ticks alone. The quote book changes
/// several times a second while markets are busy, and every revaluation republishes holdings,
/// history and cards; a total that moves every two seconds reads as live and costs a fraction.
/// A change to the ledger, the selection or the currency is never held back.
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

pub trait PortfolioFxRates: Send + Sync {
    fn conversion(
        &self,
        from: PortfolioCurrency,
        to: PortfolioCurrency,
        on: Option<i64>,
    ) -> Result<Conversion, String>;

    fn conversions(
        &self,
        from: PortfolioCurrency,
        to: PortfolioCurrency,
        dates: &[i64],
    ) -> HashMap<i64, Result<Conversion, String>>;
}

impl PortfolioFxRates for FxRateService {
    fn conversion(
        &self,
        from: PortfolioCurrency,
        to: PortfolioCurrency,
        on: Option<i64>,
    ) -> Result<Conversion, String> {
        FxRateService::conversion(self, from, to, on).map_err(|error| error.to_string())
    }

    fn conversions(
        &self,
        from: PortfolioCurrency,
        to: PortfolioCurrency,
        dates: &[i64],
    ) -> HashMap<i64, Result<Conversion, String>> {
        FxRateService::conversions(self, from, to, dates)
            .into_iter()
            .map(|(date, result)| (date, result.map_err(|error| error.to_string())))
            .collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PortfolioProjection {
    pub transactions: Vec<PortfolioTransaction>,
    pub quotes: HashMap<String, PortfolioQuote>,
    pub history: Vec<PortfolioSnapshot>,
    pub uses_cached_exchange_rates: bool,
    pub unpriced_asset_count: usize,
    /// Assets no source has reported on yet (not even to say it has no quote).
    pub awaiting_asset_count: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PortfolioQuoteState {
    pub projection: Option<PortfolioProjection>,
    /// A valuation is being built where there was none: the first load, or after the ledger,
    /// selection or currency changed. A refresh for price ticks alone never sets it, so nothing
    /// that reads it flashes while markets move.
    pub is_updating: bool,
    /// Prices changed inside the refresh interval: the next frame after it revalues them.
    pub refresh_pending: bool,
    pub error: Option<String>,
}

#[derive(Clone, PartialEq)]
struct ProjectionBasis {
    snapshot: PortfolioLedgerSnapshot,
    portfolio_ids: HashSet<u64>,
    reporting_currency: PortfolioCurrency,
}

struct FeedState {
    basis: Option<ProjectionBasis>,
    book_revision: u64,
    generation: u64,
    published: PortfolioQuoteState,
    pending: Option<(ProjectionBasis, u64, i64)>,
    worker_running: bool,
    /// When the current basis first asked for prices.
    settle_started: Option<Instant>,
    /// When the last valuation was started.
    last_started: Option<Instant>,
    /// The settle window ran out: publish whatever is priced.
    settle_expired: bool,
}

type CurrencyPair = (PortfolioCurrency, PortfolioCurrency);
type HistoricalConversions = HashMap<CurrencyPair, HashMap<i64, Result<Conversion, String>>>;

impl Default for FeedState {
    fn default() -> Self {
        Self {
            basis: None,
            book_revision: u64::MAX,
            generation: 0,
            published: PortfolioQuoteState::default(),
            pending: None,
            worker_running: false,
            settle_started: None,
            last_started: None,
            settle_expired: false,
        }
    }
}

struct Inner {
    book: WatchlistQuoteBook,
    coordinator: WatchlistQuoteCoordinator,
    fx: Arc<dyn PortfolioFxRates>,
    repaint: Arc<dyn Fn() + Send + Sync>,
    state: Mutex<FeedState>,
}

/// Cheap shared handle. Each visible portfolio tab supplies one demand owner id; expensive
/// projections are generation-checked so a late result can never replace a newer selection.
#[derive(Clone)]
pub struct PortfolioQuoteFeed {
    inner: Arc<Inner>,
}

impl PortfolioQuoteFeed {
    pub fn new(
        book: WatchlistQuoteBook,
        coordinator: WatchlistQuoteCoordinator,
        fx: Arc<dyn PortfolioFxRates>,
        repaint: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                book,
                coordinator,
                fx,
                repaint,
                state: Mutex::new(FeedState::default()),
            }),
        }
    }

    /// A feed of its own over the same prices and rates. A feed projects one selection at a time,
    /// so every consumer with its own selection (each portfolio card) needs its own.
    pub fn sibling(&self) -> Self {
        Self::new(
            self.inner.book.clone(),
            self.inner.coordinator.clone(),
            Arc::clone(&self.inner.fx),
            Arc::clone(&self.inner.repaint),
        )
    }

    /// The FX service the feed converts with, for callers that need rates of their own.
    pub fn fx_rates(&self) -> Arc<dyn PortfolioFxRates> {
        Arc::clone(&self.inner.fx)
    }

    pub fn update(
        &self,
        owner: u64,
        snapshot: &PortfolioLedgerSnapshot,
        portfolio_ids: &HashSet<u64>,
        reporting_currency: PortfolioCurrency,
        visible: bool,
        now_ms: i64,
    ) {
        let assets = eligible_assets(snapshot, portfolio_ids);
        let instruments = assets.iter().map(instrument_for).collect();
        self.inner
            .coordinator
            .set_demand(owner, instruments, visible);
        if !visible {
            return;
        }

        let basis = ProjectionBasis {
            snapshot: snapshot.clone(),
            portfolio_ids: portfolio_ids.clone(),
            reporting_currency,
        };
        let book_revision = self.inner.book.revision();
        let start_worker = {
            let mut state = lock(&self.inner.state);
            let same_basis = state.basis.as_ref() == Some(&basis);
            let unchanged = same_basis && state.book_revision == book_revision;
            let expired_while_waiting = !state.settle_expired
                && state.published.projection.is_none()
                && state
                    .settle_started
                    .is_some_and(|started| started.elapsed() >= SETTLE);
            if unchanged && !expired_while_waiting {
                return;
            }
            if same_basis
                && state.published.projection.is_some()
                && !refresh_due(state.last_started, Instant::now())
            {
                // Only prices moved, and the last valuation is recent: leave the revision
                // unrecorded so the first frame after the interval picks it up.
                state.published.refresh_pending = true;
                return;
            }
            state.published.refresh_pending = false;
            state.last_started = Some(Instant::now());
            if expired_while_waiting {
                state.settle_expired = true;
            }
            if state.basis.as_ref() != Some(&basis) {
                state.published.projection = None;
                state.settle_started = Some(Instant::now());
                state.settle_expired = false;
            }
            state.basis = Some(basis.clone());
            state.book_revision = book_revision;
            state.generation = state.generation.wrapping_add(1);
            state.published.is_updating = state.published.projection.is_none();
            state.published.error = None;
            let generation = state.generation;
            state.pending = Some((basis, generation, now_ms));
            if state.worker_running {
                false
            } else {
                state.worker_running = true;
                true
            }
        };
        if start_worker {
            let inner = Arc::clone(&self.inner);
            crate::service::work_queue::submit(move || run_projections(&inner));
        }
    }

    pub fn state(&self) -> PortfolioQuoteState {
        lock(&self.inner.state).published.clone()
    }

    pub fn release(&self, owner: u64) {
        self.inner.coordinator.release(owner);
    }
}

fn run_projections(inner: &Inner) {
    loop {
        let request = {
            let mut state = lock(&inner.state);
            match state.pending.take() {
                Some(request) => request,
                None => {
                    state.worker_running = false;
                    return;
                }
            }
        };
        let (basis, generation, now_ms) = request;
        let result = build_projection(
            &basis.snapshot,
            &basis.portfolio_ids,
            basis.reporting_currency,
            &inner.book,
            inner.fx.as_ref(),
            now_ms,
        );
        let mut state = lock(&inner.state);
        if state.generation == generation {
            // An error is news too; so is a valuation that differs from the one on screen.
            let mut changed = true;
            match result {
                Ok(projection) => {
                    let settling = projection.awaiting_asset_count > 0
                        && state.published.projection.is_none()
                        && !state.settle_expired;
                    if settling {
                        // Keep waiting: the next book change (or the settle timeout) re-runs.
                        continue;
                    }
                    changed = state.published.projection.as_ref() != Some(&projection);
                    state.published.projection = Some(projection);
                    state.published.error = None;
                }
                Err(error) => state.published.error = Some(error),
            }
            state.published.is_updating = false;
            drop(state);
            if changed {
                (inner.repaint)();
            }
        }
    }
}

/// Whether a valuation started at `last` is old enough to be redone for a price tick.
fn refresh_due(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|last| now.saturating_duration_since(last) >= REFRESH_INTERVAL)
}

pub fn instrument_for(asset: &PortfolioAsset) -> InstrumentId {
    let market_symbol = asset
        .key
        .split_once(':')
        .map(|(_, symbol)| symbol)
        .filter(|symbol| !symbol.is_empty())
        .unwrap_or(&asset.symbol);
    InstrumentId::new(asset.source, market_symbol)
}

fn eligible_assets(
    snapshot: &PortfolioLedgerSnapshot,
    portfolio_ids: &HashSet<u64>,
) -> Vec<PortfolioAsset> {
    let transactions: Vec<PortfolioTransaction> = snapshot
        .transactions
        .iter()
        .filter(|transaction| portfolio_ids.contains(&transaction.portfolio_id))
        .cloned()
        .collect();
    open_assets(&transactions, portfolio_ids)
}

/// The assets the portfolios hold now. A position sold down to nothing needs no price: asking
/// for one only adds a market that may never answer and a "partly unpriced" warning for
/// exposure that is gone. (History looks at every asset ever traded; that is another path.)
fn open_assets(
    transactions: &[PortfolioTransaction],
    portfolio_ids: &HashSet<u64>,
) -> Vec<PortfolioAsset> {
    match holdings(transactions, portfolio_ids, None, &HashMap::new(), 0) {
        Ok(held) => held.into_iter().map(|holding| holding.asset).collect(),
        // A ledger that cannot be replayed is reported elsewhere; price all it mentions.
        Err(_) => unique_assets(transactions),
    }
}

fn build_projection(
    snapshot: &PortfolioLedgerSnapshot,
    portfolio_ids: &HashSet<u64>,
    reporting_currency: PortfolioCurrency,
    book: &WatchlistQuoteBook,
    fx: &dyn PortfolioFxRates,
    now_ms: i64,
) -> Result<PortfolioProjection, String> {
    let portfolios: HashMap<u64, PortfolioCurrency> = snapshot
        .portfolios
        .iter()
        .filter(|portfolio| portfolio_ids.contains(&portfolio.id))
        .map(|portfolio| (portfolio.id, portfolio.base_currency))
        .collect();
    let selected_transactions: Vec<PortfolioTransaction> = snapshot
        .transactions
        .iter()
        .filter(|transaction| portfolio_ids.contains(&transaction.portfolio_id))
        .cloned()
        .collect();
    let selected_history: Vec<PortfolioSnapshot> = snapshot
        .historical_snapshots
        .iter()
        .filter(|point| portfolio_ids.contains(&point.portfolio_id))
        .cloned()
        .collect();

    let mut dates_by_pair: HashMap<CurrencyPair, Vec<i64>> = HashMap::new();
    for transaction in &selected_transactions {
        if transaction.price.is_some() && transaction.price_currency != reporting_currency {
            dates_by_pair
                .entry((transaction.price_currency, reporting_currency))
                .or_default()
                .push(transaction.timestamp);
        }
        if transaction.fee != 0.0 && transaction.fee_currency != reporting_currency {
            dates_by_pair
                .entry((transaction.fee_currency, reporting_currency))
                .or_default()
                .push(transaction.timestamp);
        }
    }
    for point in &selected_history {
        if let Some(base) = portfolios.get(&point.portfolio_id)
            && *base != reporting_currency
        {
            dates_by_pair
                .entry((*base, reporting_currency))
                .or_default()
                .push(point.timestamp);
        }
    }

    // The change over the last day compares today's price with yesterday's, so yesterday's
    // price is converted at yesterday's rate: a flat dollar asset is not flat in euros when the
    // exchange rate moved.
    let day_ago_ms = now_ms - DAY_MS;
    let assets = open_assets(&selected_transactions, portfolio_ids);
    for asset in &assets {
        if asset.quote_currency != reporting_currency {
            dates_by_pair
                .entry((asset.quote_currency, reporting_currency))
                .or_default()
                .push(day_ago_ms);
        }
    }

    let mut historical = HashMap::new();
    for ((from, to), dates) in dates_by_pair {
        historical.insert((from, to), fx.conversions(from, to, &dates));
    }
    let mut used_cache = false;
    let mut transactions = Vec::with_capacity(selected_transactions.len());
    for mut transaction in selected_transactions {
        if transaction.price.is_some() && transaction.price_currency != reporting_currency {
            let conversion = historical_conversion(
                &historical,
                transaction.price_currency,
                reporting_currency,
                transaction.timestamp,
            )?;
            transaction.price = transaction.price.map(|price| price * conversion.rate);
            used_cache |= conversion.is_cached;
        }
        if transaction.fee != 0.0 && transaction.fee_currency != reporting_currency {
            let conversion = historical_conversion(
                &historical,
                transaction.fee_currency,
                reporting_currency,
                transaction.timestamp,
            )?;
            transaction.fee *= conversion.rate;
            used_cache |= conversion.is_cached;
        }
        transaction.price_currency = reporting_currency;
        transaction.fee_currency = reporting_currency;
        transactions.push(transaction);
    }

    let mut current_rates: HashMap<PortfolioCurrency, Conversion> = HashMap::new();
    let mut quotes = HashMap::new();
    let mut awaiting_asset_count = 0;
    for asset in &assets {
        let Some(source) = book.get(&instrument_for(asset)) else {
            awaiting_asset_count += 1;
            continue;
        };
        if matches!(
            source.freshness,
            Freshness::Unavailable | Freshness::Unsupported
        ) {
            continue;
        }
        let Some(price) = source
            .last
            .filter(|price| price.is_finite() && *price > 0.0)
        else {
            continue;
        };
        let conversion = if asset.quote_currency == reporting_currency {
            Conversion {
                rate: 1.0,
                observation_ms: now_ms,
                is_cached: false,
            }
        } else if let Some(conversion) = current_rates.get(&asset.quote_currency) {
            *conversion
        } else {
            let conversion = fx.conversion(asset.quote_currency, reporting_currency, None)?;
            current_rates.insert(asset.quote_currency, conversion);
            conversion
        };
        used_cache |= conversion.is_cached;
        // Yesterday's rate, or today's when that day's is unavailable.
        let previous_rate = if asset.quote_currency == reporting_currency {
            conversion.rate
        } else {
            match historical_conversion(
                &historical,
                asset.quote_currency,
                reporting_currency,
                day_ago_ms,
            ) {
                Ok(previous) => {
                    used_cache |= previous.is_cached;
                    previous.rate
                }
                Err(_) => conversion.rate,
            }
        };
        let timestamp = if matches!(source.freshness, Freshness::Live | Freshness::MarketClosed) {
            now_ms
        } else {
            source.timestamp_ms.unwrap_or(0)
        };
        quotes.insert(
            asset.key.clone(),
            PortfolioQuote {
                price: price * conversion.rate,
                previous_day_price: source.change.map(|change| (price - change) * previous_rate),
                timestamp,
            },
        );
    }

    let unpriced_asset_count = assets
        .iter()
        .filter(|asset| !quotes.contains_key(&asset.key))
        .count();
    let mut history = Vec::with_capacity(selected_history.len());
    for mut point in selected_history {
        let base = portfolios
            .get(&point.portfolio_id)
            .copied()
            .unwrap_or(reporting_currency);
        if base != reporting_currency {
            let conversion =
                historical_conversion(&historical, base, reporting_currency, point.timestamp)?;
            point.value *= conversion.rate;
            point.net_contributions *= conversion.rate;
            point.realized_pnl *= conversion.rate;
            point.unrealized_pnl *= conversion.rate;
            used_cache |= conversion.is_cached;
        }
        history.push(point);
    }
    history.sort_by_key(|point| point.timestamp);
    if portfolio_ids.len() > 1 {
        history = aggregate_history(&history);
    }

    Ok(PortfolioProjection {
        transactions,
        quotes,
        history,
        uses_cached_exchange_rates: used_cache,
        unpriced_asset_count,
        awaiting_asset_count,
    })
}

fn historical_conversion(
    conversions: &HistoricalConversions,
    from: PortfolioCurrency,
    to: PortfolioCurrency,
    timestamp: i64,
) -> Result<Conversion, String> {
    conversions
        .get(&(from, to))
        .and_then(|by_date| by_date.get(&timestamp))
        .cloned()
        .unwrap_or_else(|| {
            Err(format!(
                "Exchange rate from {} to {} is unavailable.",
                from.display_name(),
                to.display_name()
            ))
        })
}

fn aggregate_history(points: &[PortfolioSnapshot]) -> Vec<PortfolioSnapshot> {
    let mut by_timestamp: BTreeMap<i64, Vec<&PortfolioSnapshot>> = BTreeMap::new();
    for point in points {
        by_timestamp.entry(point.timestamp).or_default().push(point);
    }
    by_timestamp
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
    use crate::model::portfolio::{Portfolio, PortfolioSource, PortfolioTransactionType};
    use crate::model::watchlist::quote::WatchlistQuote;

    struct FakeFx;

    impl PortfolioFxRates for FakeFx {
        fn conversion(
            &self,
            from: PortfolioCurrency,
            to: PortfolioCurrency,
            on: Option<i64>,
        ) -> Result<Conversion, String> {
            Ok(Conversion {
                rate: rate(from, to),
                observation_ms: on.unwrap_or(9_000),
                is_cached: false,
            })
        }

        fn conversions(
            &self,
            from: PortfolioCurrency,
            to: PortfolioCurrency,
            dates: &[i64],
        ) -> HashMap<i64, Result<Conversion, String>> {
            dates
                .iter()
                .map(|date| {
                    (
                        *date,
                        Ok(Conversion {
                            rate: rate(from, to),
                            observation_ms: *date,
                            is_cached: false,
                        }),
                    )
                })
                .collect()
        }
    }

    fn rate(from: PortfolioCurrency, to: PortfolioCurrency) -> f64 {
        match (from, to) {
            (PortfolioCurrency::Usd, PortfolioCurrency::Eur) => 0.9,
            _ if from == to => 1.0,
            _ => panic!("unexpected conversion"),
        }
    }

    fn asset() -> PortfolioAsset {
        PortfolioAsset {
            key: "binance:BTCUSDT".into(),
            symbol: "BTC/USDT".into(),
            name: "Bitcoin".into(),
            source: DataSource::Binance,
            quote_currency: PortfolioCurrency::Usd,
            metadata: HashMap::new(),
        }
    }

    fn snapshot() -> PortfolioLedgerSnapshot {
        PortfolioLedgerSnapshot {
            portfolios: vec![Portfolio {
                base_currency: PortfolioCurrency::Usd,
                ..Portfolio::new(1, "Main", 1_000)
            }],
            transactions: vec![PortfolioTransaction {
                id: 2,
                portfolio_id: 1,
                asset: asset(),
                kind: PortfolioTransactionType::Buy,
                quantity: 2.0,
                price: Some(10_000.0),
                price_currency: PortfolioCurrency::Usd,
                fee: 10.0,
                fee_currency: PortfolioCurrency::Usd,
                timestamp: 2_000,
                notes: String::new(),
                source: PortfolioSource::Manual,
                external_transaction_id: None,
                created_at: 2_000,
                updated_at: 2_000,
            }],
            ..PortfolioLedgerSnapshot::default()
        }
    }

    #[test]
    fn price_ticks_revalue_at_most_once_per_interval() {
        let started = Instant::now();
        assert!(refresh_due(None, started), "never valued: go now");
        assert!(!refresh_due(Some(started), started));
        assert!(!refresh_due(
            Some(started),
            started + REFRESH_INTERVAL - Duration::from_millis(1)
        ));
        assert!(refresh_due(Some(started), started + REFRESH_INTERVAL));
        // A clock that reads earlier than the start never panics or holds the refresh back forever.
        assert!(!refresh_due(Some(started + REFRESH_INTERVAL), started));
    }

    #[test]
    fn asset_keys_supply_the_provider_symbol() {
        let instrument = instrument_for(&asset());
        assert_eq!(instrument.source, DataSource::Binance);
        assert_eq!(instrument.symbol, "BTCUSDT");
    }

    #[test]
    fn projection_converts_costs_and_shared_live_quotes() {
        let book = WatchlistQuoteBook::default();
        let instrument = instrument_for(&asset());
        book.apply(HashMap::from([(
            instrument.key(),
            WatchlistQuote {
                last: Some(20_000.0),
                change: Some(2_000.0),
                timestamp_ms: Some(8_000),
                freshness: Freshness::Live,
                ..WatchlistQuote::default()
            },
        )]));

        let projection = build_projection(
            &snapshot(),
            &HashSet::from([1]),
            PortfolioCurrency::Eur,
            &book,
            &FakeFx,
            10_000,
        )
        .expect("projection");
        assert_eq!(projection.transactions[0].price, Some(9_000.0));
        assert_eq!(projection.transactions[0].fee, 9.0);
        assert_eq!(projection.quotes["binance:BTCUSDT"].price, 18_000.0);
        assert_eq!(
            projection.quotes["binance:BTCUSDT"].previous_day_price,
            Some(16_200.0)
        );
        assert_eq!(projection.unpriced_asset_count, 0);
    }

    /// Dollars are worth 0.8 euros a day ago and 0.9 now.
    struct MovingFx;

    impl PortfolioFxRates for MovingFx {
        fn conversion(
            &self,
            _: PortfolioCurrency,
            _: PortfolioCurrency,
            on: Option<i64>,
        ) -> Result<Conversion, String> {
            Ok(Conversion {
                rate: 0.9,
                observation_ms: on.unwrap_or(0),
                is_cached: false,
            })
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
                    let rate = if *date == NOW - DAY_MS { 0.8 } else { 0.9 };
                    (
                        *date,
                        Ok(Conversion {
                            rate,
                            observation_ms: *date,
                            is_cached: false,
                        }),
                    )
                })
                .collect()
        }
    }

    const NOW: i64 = 200_000_000;

    fn live_quote(book: &WatchlistQuoteBook, last: f64, change: f64) {
        book.apply(HashMap::from([(
            instrument_for(&asset()).key(),
            WatchlistQuote {
                last: Some(last),
                change: Some(change),
                timestamp_ms: Some(NOW),
                freshness: Freshness::Live,
                ..WatchlistQuote::default()
            },
        )]));
    }

    #[test]
    fn a_flat_asset_moves_in_the_reporting_currency_when_the_exchange_rate_does() {
        let book = WatchlistQuoteBook::default();
        live_quote(&book, 20_000.0, 0.0);
        let projection = build_projection(
            &snapshot(),
            &HashSet::from([1]),
            PortfolioCurrency::Eur,
            &book,
            &MovingFx,
            NOW,
        )
        .expect("projection");
        let quote = &projection.quotes["binance:BTCUSDT"];
        assert!((quote.price - 18_000.0).abs() < 1e-6, "today at 0.9");
        let previous = quote.previous_day_price.unwrap();
        assert!((previous - 16_000.0).abs() < 1e-6, "yesterday at 0.8");
    }

    fn sold_out() -> PortfolioLedgerSnapshot {
        let mut snapshot = snapshot();
        let mut sell = snapshot.transactions[0].clone();
        sell.id = 3;
        sell.kind = PortfolioTransactionType::Sell;
        sell.timestamp = 3_000;
        sell.created_at = 3_000;
        snapshot.transactions.push(sell);
        snapshot
    }

    #[test]
    fn a_sold_out_asset_is_not_priced_or_awaited() {
        let ids = HashSet::from([1]);
        assert!(eligible_assets(&snapshot(), &ids).len() == 1);
        assert!(eligible_assets(&sold_out(), &ids).is_empty());
        let projection = build_projection(
            &sold_out(),
            &ids,
            PortfolioCurrency::Usd,
            &WatchlistQuoteBook::default(),
            &FakeFx,
            10_000,
        )
        .expect("projection");
        assert_eq!(projection.unpriced_asset_count, 0);
        assert_eq!(projection.awaiting_asset_count, 0);
    }

    #[test]
    fn absent_quotes_remain_unpriced_instead_of_becoming_zero() {
        let projection = build_projection(
            &snapshot(),
            &HashSet::from([1]),
            PortfolioCurrency::Usd,
            &WatchlistQuoteBook::default(),
            &FakeFx,
            10_000,
        )
        .expect("projection");
        assert!(projection.quotes.is_empty());
        assert_eq!(projection.unpriced_asset_count, 1);
    }

    #[test]
    fn a_market_that_answered_no_quote_is_not_awaited() {
        let build = |book: &WatchlistQuoteBook| {
            build_projection(
                &snapshot(),
                &HashSet::from([1]),
                PortfolioCurrency::Usd,
                book,
                &FakeFx,
                10_000,
            )
            .expect("projection")
        };
        let book = WatchlistQuoteBook::default();
        assert_eq!(build(&book).awaiting_asset_count, 1, "nothing reported yet");

        book.apply(HashMap::from([(
            instrument_for(&asset()).key(),
            WatchlistQuote {
                freshness: Freshness::Unavailable,
                ..WatchlistQuote::default()
            },
        )]));
        let answered = build(&book);
        assert_eq!(answered.awaiting_asset_count, 0);
        assert_eq!(answered.unpriced_asset_count, 1);
    }
}
