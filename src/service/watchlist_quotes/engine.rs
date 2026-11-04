//! The watchlist quote state machine, free of threads and clocks of its own so every
//! rule is testable with a fake provider and explicit times.
//!
//! Sidebars (one per window) say which markets they need and whether they are on
//! screen. Needs are merged by market, so one that sits in three lists or two windows is
//! requested once. Prices come from one batched request per source per round, plus a
//! single shared Coinbase socket. A source that fails or is rate limited backs off
//! alone, keeping its last prices marked stale; a symbol the provider rejects is found
//! by splitting the batch and then left out for ten minutes.

use super::book::WatchlistQuoteBook;
use super::coinbase_ws::CoinbaseTick;
use super::provider::{QuoteFailure, QuoteProvider, QuoteRequest, chunk_size};
use crate::model::alert::AlertProviderHealth;
use crate::model::watchlist_quote::{
    Freshness, SourceQuote, WatchlistQuote, evaluate_freshness, refresh_interval_secs,
};
use crate::model::{DataSource, InstrumentId};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;
use std::sync::PoisonError;

#[derive(Clone, Debug)]
pub struct Config {
    /// How often the loop plans a round. Each source is refreshed at its own interval.
    pub poll_interval_ms: i64,
    /// A source that fails is tried again after this long (seconds), doubling up to the cap.
    pub failure_backoff: (f64, f64),
    /// A rate limit backs off further.
    pub rate_limit_backoff: (f64, f64),
    pub rejected_retry_ms: i64,
    /// Coinbase products the socket has not answered yet are filled from REST after this long...
    pub coinbase_fallback_delay_ms: i64,
    /// ...and any product the socket has been quiet about is refreshed from REST this often.
    pub coinbase_refresh_ms: i64,
    pub chain_lookups_per_round: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            poll_interval_ms: 5_000,
            failure_backoff: (5.0, 30.0),
            rate_limit_backoff: (15.0, 60.0),
            rejected_retry_ms: 600_000,
            coinbase_fallback_delay_ms: 6_000,
            coinbase_refresh_ms: 30_000,
            chain_lookups_per_round: 5,
        }
    }
}

#[derive(PartialEq)]
struct Consumer {
    instruments: BTreeMap<String, InstrumentId>,
    active: bool,
}

#[derive(Clone)]
struct SourceState {
    backoff_secs: f64,
    next_allowed_ms: i64,
    last_poll_ms: i64,
    last_successful_quote_ms: Option<i64>,
    last_error: Option<String>,
}

impl Default for SourceState {
    fn default() -> Self {
        Self {
            backoff_secs: 0.0,
            next_allowed_ms: i64::MIN,
            last_poll_ms: i64::MIN / 2,
            last_successful_quote_ms: None,
            last_error: None,
        }
    }
}

/// What a fetch round decided to ask one source.
#[derive(Clone, Debug)]
pub struct Planned {
    pub source: DataSource,
    /// `(instrument key, request)`.
    pub requests: Vec<(String, QuoteRequest)>,
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub quotes: HashMap<String, SourceQuote>,
    pub rejected: HashSet<String>,
    pub failure: Option<QuoteFailure>,
}

pub struct Engine {
    book: WatchlistQuoteBook,
    config: Config,
    consumers: HashMap<u64, Consumer>,
    known: HashMap<String, WatchlistQuote>,
    /// When each price was last confirmed by its provider; what freshness is judged by.
    received: HashMap<String, i64>,
    sources: HashMap<DataSource, SourceState>,
    rejected: HashMap<String, i64>,
    chains: HashMap<String, String>,
    unresolved_chains: HashSet<String>,
    /// Upper-case product id -> instrument key.
    coinbase_products: HashMap<String, String>,
    coinbase_subscribed_at: HashMap<String, i64>,
}

impl Engine {
    pub fn new(book: WatchlistQuoteBook, config: Config) -> Self {
        Self {
            book,
            config,
            consumers: HashMap::new(),
            known: HashMap::new(),
            received: HashMap::new(),
            sources: HashMap::new(),
            rejected: HashMap::new(),
            chains: HashMap::new(),
            unresolved_chains: HashSet::new(),
            coinbase_products: HashMap::new(),
            coinbase_subscribed_at: HashMap::new(),
        }
    }

    pub fn poll_interval_ms(&self) -> i64 {
        self.config.poll_interval_ms
    }

    // MARK: Subscriptions

    /// What `consumer` (a window's sidebar) needs now. An inactive consumer keeps its
    /// place but asks for nothing until it is active again.
    pub fn update(
        &mut self,
        consumer: u64,
        instruments: Vec<InstrumentId>,
        active: bool,
        now: i64,
    ) {
        let next = Consumer {
            instruments: instruments
                .into_iter()
                .map(|instrument| (instrument.key(), instrument))
                .collect(),
            active,
        };
        // A sidebar republishes its rows every frame: an unchanged set costs nothing.
        if self.consumers.get(&consumer) == Some(&next) {
            return;
        }
        self.consumers.insert(consumer, next);
        self.reconcile(now);
    }

    #[allow(dead_code)] // reached through the coordinator's `release`
    pub fn release(&mut self, consumer: u64, now: i64) {
        self.consumers.remove(&consumer);
        self.reconcile(now);
    }

    /// Every market an active sidebar needs, once each.
    pub fn wanted(&self) -> BTreeMap<String, InstrumentId> {
        let mut wanted = BTreeMap::new();
        for consumer in self.consumers.values().filter(|consumer| consumer.active) {
            for (key, instrument) in &consumer.instruments {
                wanted
                    .entry(key.clone())
                    .or_insert_with(|| instrument.clone());
            }
        }
        wanted
    }

    #[cfg(test)]
    pub fn subscriber_count(&self, instrument: &InstrumentId) -> usize {
        let key = instrument.key();
        self.consumers
            .values()
            .filter(|consumer| consumer.active && consumer.instruments.contains_key(&key))
            .count()
    }

    pub fn has_subscriptions(&self) -> bool {
        self.consumers
            .values()
            .any(|consumer| consumer.active && !consumer.instruments.is_empty())
    }

    pub fn provider_health(&self) -> Vec<AlertProviderHealth> {
        let wanted = self.wanted();
        let mut sources: Vec<DataSource> = wanted.values().map(|value| value.source).collect();
        sources.sort_by_key(|source| source_order(*source));
        sources.dedup();
        sources
            .into_iter()
            .map(|source| {
                let state = self.sources.get(&source).cloned().unwrap_or_default();
                let has_data_gap = wanted.iter().any(|(key, instrument)| {
                    instrument.source == source
                        && self
                            .known
                            .get(key)
                            .is_none_or(|quote| !quote.freshness.is_current())
                });
                AlertProviderHealth {
                    source,
                    last_successful_quote_ms: state.last_successful_quote_ms,
                    last_error: state.last_error,
                    retry_at_ms: (state.next_allowed_ms > 0).then_some(state.next_allowed_ms),
                    has_data_gap,
                }
            })
            .collect()
    }

    /// The Coinbase products the shared socket should carry, sorted.
    pub fn coinbase_products(&self) -> Vec<String> {
        let mut products: Vec<String> = self.coinbase_products.keys().cloned().collect();
        products.sort();
        products
    }

    fn reconcile(&mut self, now: i64) {
        let wanted = self.wanted();
        if wanted.is_empty() {
            self.coinbase_products.clear();
            return;
        }
        for (key, instrument) in &wanted {
            if instrument.source == DataSource::CoinMarketCap && !self.known.contains_key(key) {
                self.publish(
                    key,
                    WatchlistQuote::unsupported(Some(
                        if instrument.symbol.starts_with("portfolio:") {
                            "Portfolio cards have no market quote"
                        } else {
                            "Index charts have no market quote"
                        }
                        .to_owned(),
                    )),
                );
            }
        }
        // Prices that aged while nothing was polling must not read as live on return.
        self.age_quotes(now);
        self.sync_coinbase(&wanted, now);
    }

    fn sync_coinbase(&mut self, wanted: &BTreeMap<String, InstrumentId>, now: i64) {
        let mut products = HashMap::new();
        for (key, instrument) in wanted {
            if instrument.source == DataSource::Coinbase {
                products.insert(instrument.api_symbol().to_uppercase(), key.clone());
            }
        }
        for key in products.values() {
            self.coinbase_subscribed_at
                .entry(key.clone())
                .or_insert(now);
        }
        self.coinbase_products = products;
    }

    // MARK: Planning a round

    /// DEX pairs saved without a known network, to look up this round.
    pub fn missing_chains(&self) -> Vec<InstrumentId> {
        self.wanted()
            .into_iter()
            .filter(|(key, instrument)| {
                instrument.source == DataSource::DexScreener
                    && !self.chains.contains_key(key)
                    && !self.unresolved_chains.contains(key)
            })
            .map(|(_, instrument)| instrument)
            .take(self.config.chain_lookups_per_round)
            .collect()
    }

    pub fn set_chain(&mut self, instrument: &InstrumentId, chain: Option<String>) {
        let key = instrument.key();
        match chain {
            Some(chain) => {
                self.chains.insert(key, chain);
            }
            None => {
                // Don't ask again this session; the row keeps its dash and says why.
                self.unresolved_chains.insert(key.clone());
                self.publish(
                    &key,
                    WatchlistQuote {
                        freshness: Freshness::Unavailable,
                        note: Some("Couldn't find this pair's network".to_owned()),
                        ..WatchlistQuote::default()
                    },
                );
            }
        }
    }

    /// The source to poll `instrument` through, or `None` when it should not be polled now.
    fn polled(&self, key: &str, instrument: &InstrumentId, now: i64) -> Option<DataSource> {
        if self.rejected.get(key).is_some_and(|until| *until > now) {
            return None;
        }
        match instrument.source {
            DataSource::CoinMarketCap => None,
            DataSource::Coinbase => {
                // The socket supplies prices; REST fills a product it has not answered, and
                // refreshes one it has gone quiet on, so a thin market's price never ages.
                if let Some(last) = self.received.get(key) {
                    return (now - last >= self.config.coinbase_refresh_ms)
                        .then_some(DataSource::Coinbase);
                }
                let since = self.coinbase_subscribed_at.get(key)?;
                (now - since >= self.config.coinbase_fallback_delay_ms)
                    .then_some(DataSource::Coinbase)
            }
            DataSource::DexScreener => self
                .chains
                .contains_key(key)
                .then_some(DataSource::DexScreener),
            source => Some(source),
        }
    }

    fn is_due(&self, source: DataSource, now: i64) -> bool {
        let state = self.sources.get(&source).cloned().unwrap_or_default();
        if now < state.next_allowed_ms {
            return false;
        }
        // A second of slack: rounds drift a little late.
        now - state.last_poll_ms >= refresh_interval_secs(source) as i64 * 1_000 - 1_000
    }

    /// One round: every due source gets one batched request.
    pub fn plan(&mut self, now: i64) -> Vec<Planned> {
        self.age_quotes(now);
        let wanted = self.wanted();
        let mut by_source: BTreeMap<u8, Planned> = BTreeMap::new();
        for (key, instrument) in &wanted {
            let Some(source) = self.polled(key, instrument, now) else {
                continue;
            };
            let request = QuoteRequest {
                symbol: instrument.api_symbol(),
                chain: self.chains.get(key).cloned(),
            };
            by_source
                .entry(source_order(source))
                .or_insert_with(|| Planned {
                    source,
                    requests: Vec::new(),
                })
                .requests
                .push((key.clone(), request));
        }
        let plan: Vec<Planned> = by_source
            .into_values()
            .filter(|planned| self.is_due(planned.source, now))
            .collect();
        for planned in &plan {
            self.sources.entry(planned.source).or_default().last_poll_ms = now;
        }
        plan
    }

    // MARK: Applying results

    pub fn apply(&mut self, planned: &Planned, outcome: &Outcome, now: i64) {
        let source = planned.source;
        let mut state = self.sources.get(&source).cloned().unwrap_or_default();
        let mut updates: HashMap<String, WatchlistQuote> = HashMap::new();

        match &outcome.failure {
            Some(failure @ (QuoteFailure::RateLimited | QuoteFailure::Failed(_))) => {
                let (low, high) = if matches!(failure, QuoteFailure::RateLimited) {
                    self.config.rate_limit_backoff
                } else {
                    self.config.failure_backoff
                };
                state.backoff_secs = (state.backoff_secs * 2.0).max(low).min(high);
                state.next_allowed_ms = now + (state.backoff_secs * 1_000.0) as i64;
                state.last_error = Some(failure.message());
                for (key, _) in &planned.requests {
                    updates.insert(key.clone(), self.fallback(key, Some(failure.message())));
                }
            }
            Some(QuoteFailure::CredentialsMissing) => {
                state.next_allowed_ms = now + 60_000;
                state.last_error = Some(QuoteFailure::CredentialsMissing.message());
                for (key, _) in &planned.requests {
                    updates.insert(
                        key.clone(),
                        WatchlistQuote::unsupported(Some(
                            QuoteFailure::CredentialsMissing.message(),
                        )),
                    );
                }
            }
            Some(QuoteFailure::Rejected(_)) | None => {
                state.backoff_secs = 0.0;
                state.next_allowed_ms = i64::MIN;
                // A symbol is only called bad when its neighbours answered; if nothing did,
                // the source is down.
                let source_answered = !outcome.quotes.is_empty();
                if source_answered {
                    state.last_successful_quote_ms = Some(now);
                    state.last_error = None;
                } else {
                    state.last_error = Some("The provider returned no quote".to_owned());
                }
                for (key, request) in &planned.requests {
                    if let Some(quote) = outcome.quotes.get(&request.symbol) {
                        updates
                            .insert(key.clone(), WatchlistQuote::from_source(source, quote, now));
                        self.received.insert(key.clone(), now);
                        self.rejected.remove(key);
                    } else if source_answered && outcome.rejected.contains(&request.symbol) {
                        self.rejected
                            .insert(key.clone(), now + self.config.rejected_retry_ms);
                        updates.insert(
                            key.clone(),
                            WatchlistQuote {
                                freshness: Freshness::Unavailable,
                                note: Some("The provider doesn't know this market".to_owned()),
                                ..WatchlistQuote::default()
                            },
                        );
                    } else {
                        updates.insert(
                            key.clone(),
                            self.fallback(key, Some("The provider returned no quote".to_owned())),
                        );
                    }
                }
            }
        }
        self.sources.insert(source, state);
        for (key, quote) in &updates {
            self.known.insert(key.clone(), quote.clone());
        }
        self.book.apply(updates);
    }

    /// The last known price, marked stale; or an unavailable quote if there never was one.
    fn fallback(&self, key: &str, note: Option<String>) -> WatchlistQuote {
        match self.known.get(key) {
            Some(last) if last.last.is_some() => last.with_freshness(Freshness::Stale),
            _ => WatchlistQuote {
                freshness: Freshness::Unavailable,
                note,
                ..WatchlistQuote::default()
            },
        }
    }

    fn publish(&mut self, key: &str, quote: WatchlistQuote) {
        self.known.insert(key.to_owned(), quote.clone());
        self.book.apply(HashMap::from([(key.to_owned(), quote)]));
    }

    /// Downgrades prices whose window has passed since they were stamped.
    pub fn age_quotes(&mut self, now: i64) {
        let mut updates = HashMap::new();
        for (key, quote) in &self.known {
            if !quote.freshness.is_current() {
                continue;
            }
            let (Some(confirmed), Some(source)) = (self.received.get(key), self.source_of(key))
            else {
                continue;
            };
            if evaluate_freshness(source, *confirmed, now) == Freshness::Stale {
                updates.insert(key.clone(), quote.with_freshness(Freshness::Stale));
            }
        }
        if updates.is_empty() {
            return;
        }
        for (key, quote) in &updates {
            self.known.insert(key.clone(), quote.clone());
        }
        self.book.apply(updates);
    }

    /// The source behind an instrument key (`SLUG:SYMBOL`).
    fn source_of(&self, key: &str) -> Option<DataSource> {
        let slug = key.split(':').next()?;
        ALL_SOURCES
            .into_iter()
            .find(|source| source.watchlist_slug() == slug)
    }

    // MARK: Coinbase ticks

    /// Publishes the latest tick of each product; the coordinator drains its channel on a
    /// short interval, so a busy feed becomes a few updates a second.
    pub fn receive_ticks(&mut self, ticks: Vec<CoinbaseTick>, now: i64) {
        let mut latest: HashMap<String, WatchlistQuote> = HashMap::new();
        for tick in ticks {
            let Some(key) = self.coinbase_products.get(&tick.product_id.to_uppercase()) else {
                continue;
            };
            latest.insert(
                key.clone(),
                WatchlistQuote::from_source(DataSource::Coinbase, &tick.as_source_quote(), now),
            );
        }
        if latest.is_empty() {
            return;
        }
        for (key, quote) in &latest {
            self.received.insert(key.clone(), now);
            self.known.insert(key.clone(), quote.clone());
        }
        let state = self.sources.entry(DataSource::Coinbase).or_default();
        state.last_successful_quote_ms = Some(now);
        state.last_error = None;
        state.next_allowed_ms = i64::MIN;
        self.book.apply(latest);
    }
}

const ALL_SOURCES: [DataSource; 8] = [
    DataSource::Binance,
    DataSource::Coinbase,
    DataSource::CoinGecko,
    DataSource::DexScreener,
    DataSource::Alpaca,
    DataSource::Polymarket,
    DataSource::Kalshi,
    DataSource::CoinMarketCap,
];

/// A stable order for sources, so rounds are deterministic.
fn source_order(source: DataSource) -> u8 {
    ALL_SOURCES
        .iter()
        .position(|candidate| *candidate == source)
        .unwrap_or(usize::MAX) as u8
}

// MARK: Fetching

/// Fetches a batch. When the provider rejects it outright, halves it until the bad
/// symbols are found, so one delisted market does not blank the list.
pub fn fetch(
    provider: &dyn QuoteProvider,
    source: DataSource,
    requests: &[QuoteRequest],
) -> Outcome {
    fetch_chunked(provider, source, requests, chunk_size(source))
}

fn fetch_chunked(
    provider: &dyn QuoteProvider,
    source: DataSource,
    requests: &[QuoteRequest],
    size: usize,
) -> Outcome {
    if requests.len() > size {
        let mut merged = Outcome::default();
        for page in requests.chunks(size) {
            let part = fetch_chunked(provider, source, page, size);
            for (symbol, quote) in part.quotes {
                merged.quotes.entry(symbol).or_insert(quote);
            }
            merged.rejected.extend(part.rejected);
            merged.failure = merged.failure.or(part.failure);
        }
        return merged;
    }
    match provider.quotes(source, requests) {
        Ok(quotes) => Outcome {
            quotes,
            ..Outcome::default()
        },
        Err(QuoteFailure::Rejected(_)) => {
            if requests.len() <= 1 {
                return Outcome {
                    rejected: requests
                        .iter()
                        .map(|request| request.symbol.clone())
                        .collect(),
                    ..Outcome::default()
                };
            }
            let (first, second) = requests.split_at(requests.len() / 2);
            let first = fetch_chunked(provider, source, first, size);
            let second = fetch_chunked(provider, source, second, size);
            let mut merged = Outcome {
                quotes: first.quotes,
                rejected: first.rejected,
                failure: first.failure,
            };
            for (symbol, quote) in second.quotes {
                merged.quotes.entry(symbol).or_insert(quote);
            }
            merged.rejected.extend(second.rejected);
            merged.failure = merged.failure.or(second.failure);
            merged
        }
        Err(failure) => Outcome {
            failure: Some(failure),
            ..Outcome::default()
        },
    }
}

fn lock(engine: &Mutex<Engine>) -> std::sync::MutexGuard<'_, Engine> {
    engine.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One full round against a shared engine: look up missing DEX networks, plan, fetch every
/// due source in parallel, apply. The engine lock is never held across network calls.
pub fn poll(engine: &Mutex<Engine>, provider: &dyn QuoteProvider, now: &dyn Fn() -> i64) {
    let missing = lock(engine).missing_chains();
    for instrument in missing {
        let chain = provider.resolve_chain(&instrument.symbol);
        lock(engine).set_chain(&instrument, chain);
    }

    let plan = lock(engine).plan(now());
    if plan.is_empty() {
        return;
    }
    let outcomes: Vec<Outcome> = std::thread::scope(|scope| {
        let handles: Vec<_> = plan
            .iter()
            .map(|planned| {
                let requests: Vec<QuoteRequest> = planned
                    .requests
                    .iter()
                    .map(|(_, request)| request.clone())
                    .collect();
                scope.spawn(move || fetch(provider, planned.source, &requests))
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle.join().unwrap_or_else(|_| Outcome {
                    failure: Some(QuoteFailure::Failed("Quote worker crashed".to_owned())),
                    ..Outcome::default()
                })
            })
            .collect()
    });

    let received_at = now();
    let mut guard = lock(engine);
    for (planned, outcome) in plan.iter().zip(&outcomes) {
        guard.apply(planned, outcome, received_at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::watchlist_quote::Freshness;
    use std::sync::Arc;

    /// Answers from a table and records every call.
    #[derive(Default)]
    struct FakeProvider {
        prices: Mutex<HashMap<String, f64>>,
        reject: Mutex<HashSet<String>>,
        failure: Mutex<Option<QuoteFailure>>,
        calls: Mutex<Vec<(DataSource, Vec<String>)>>,
        chains: Mutex<HashMap<String, String>>,
    }

    impl FakeProvider {
        fn price(&self, symbol: &str, price: f64) {
            self.prices.lock().unwrap().insert(symbol.to_owned(), price);
        }
        fn calls(&self) -> Vec<(DataSource, Vec<String>)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl QuoteProvider for FakeProvider {
        fn quotes(
            &self,
            source: DataSource,
            requests: &[QuoteRequest],
        ) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
            let mut symbols: Vec<String> = requests.iter().map(|r| r.symbol.clone()).collect();
            symbols.sort();
            self.calls.lock().unwrap().push((source, symbols));
            if let Some(failure) = self.failure.lock().unwrap().clone() {
                return Err(failure);
            }
            let rejects = self.reject.lock().unwrap();
            if requests.iter().any(|r| rejects.contains(&r.symbol)) {
                return Err(QuoteFailure::Rejected("unknown symbol".to_owned()));
            }
            let prices = self.prices.lock().unwrap();
            Ok(requests
                .iter()
                .filter_map(|r| {
                    prices.get(&r.symbol).map(|price| {
                        (
                            r.symbol.clone(),
                            SourceQuote {
                                price: *price,
                                previous_day_price: Some(*price / 2.0),
                                ..SourceQuote::default()
                            },
                        )
                    })
                })
                .collect())
        }

        fn resolve_chain(&self, address: &str) -> Option<String> {
            self.chains.lock().unwrap().get(address).cloned()
        }
    }

    fn id(source: DataSource, symbol: &str) -> InstrumentId {
        InstrumentId::new(source, symbol)
    }

    fn setup() -> (Arc<Mutex<Engine>>, WatchlistQuoteBook, Arc<FakeProvider>) {
        let book = WatchlistQuoteBook::default();
        let engine = Arc::new(Mutex::new(Engine::new(book.clone(), Config::default())));
        (engine, book, Arc::new(FakeProvider::default()))
    }

    fn round(engine: &Mutex<Engine>, provider: &FakeProvider, now: i64) {
        poll(engine, provider, &|| now);
    }

    fn last(book: &WatchlistQuoteBook, instrument: &InstrumentId) -> Option<f64> {
        book.get(instrument).and_then(|quote| quote.last)
    }

    const T0: i64 = 1_700_000_000_000;

    #[test]
    fn one_batched_request_per_source_merging_every_window_and_list() {
        let (engine, book, provider) = setup();
        provider.price("BTCUSDT", 100.0);
        provider.price("ETHUSDT", 50.0);
        provider.price("bitcoin", 7.0);
        let btc = id(DataSource::Binance, "BTCUSDT");
        let eth = id(DataSource::Binance, "ETHUSDT");
        let gecko = id(DataSource::CoinGecko, "bitcoin");
        // Two windows both show BTC; one also shows ETH and a CoinGecko coin.
        lock(&engine).update(1, vec![btc.clone(), eth.clone()], true, T0);
        lock(&engine).update(
            2,
            vec![id(DataSource::Binance, "btcusdt"), gecko.clone()],
            true,
            T0,
        );

        round(&engine, &provider, T0);
        // Sources are fetched in parallel, so the order they reach the provider is not fixed.
        let mut calls = provider.calls();
        calls.sort_by_key(|(source, _)| source_order(*source));
        assert_eq!(calls.len(), 2, "one request per source: {calls:?}");
        assert_eq!(
            calls[0],
            (
                DataSource::Binance,
                vec!["BTCUSDT".into(), "ETHUSDT".into()]
            )
        );
        assert_eq!(calls[1], (DataSource::CoinGecko, vec!["bitcoin".into()]));
        assert_eq!(last(&book, &btc), Some(100.0));
        assert_eq!(last(&book, &gecko), Some(7.0));
        assert_eq!(lock(&engine).subscriber_count(&btc), 2);
        let quote = book.get(&btc).unwrap();
        assert_eq!(quote.freshness, Freshness::Live);
        assert_eq!(
            quote.change_percent,
            Some(100.0),
            "price/previous-1 with previous = price/2"
        );
    }

    #[test]
    fn nothing_is_requested_while_no_sidebar_is_visible() {
        let (engine, book, provider) = setup();
        provider.price("BTCUSDT", 100.0);
        let btc = id(DataSource::Binance, "BTCUSDT");
        lock(&engine).update(1, vec![btc.clone()], false, T0);
        round(&engine, &provider, T0);
        assert!(provider.calls().is_empty());
        assert!(!lock(&engine).has_subscriptions());

        // Becoming visible starts polling; hiding again keeps the last prices in the book.
        lock(&engine).update(1, vec![btc.clone()], true, T0);
        round(&engine, &provider, T0);
        assert_eq!(provider.calls().len(), 1);
        lock(&engine).update(1, vec![btc.clone()], false, T0 + 1_000);
        round(&engine, &provider, T0 + 60_000);
        assert_eq!(provider.calls().len(), 1);
        assert_eq!(last(&book, &btc), Some(100.0));

        lock(&engine).release(1, T0 + 2_000);
        assert!(lock(&engine).wanted().is_empty());
    }

    #[test]
    fn each_source_refreshes_at_its_own_interval() {
        let (engine, _book, provider) = setup();
        provider.price("BTCUSDT", 1.0);
        provider.price("bitcoin", 1.0);
        lock(&engine).update(
            1,
            vec![
                id(DataSource::Binance, "BTCUSDT"),
                id(DataSource::CoinGecko, "bitcoin"),
            ],
            true,
            T0,
        );
        round(&engine, &provider, T0);
        assert_eq!(provider.calls().len(), 2);
        // 5 s later Binance is due again (5 s source), CoinGecko (30 s) is not.
        round(&engine, &provider, T0 + 5_000);
        let calls = provider.calls();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[2].0, DataSource::Binance);
        round(&engine, &provider, T0 + 30_000);
        assert!(
            provider
                .calls()
                .iter()
                .any(|(s, _)| *s == DataSource::CoinGecko)
        );
        assert_eq!(
            provider
                .calls()
                .iter()
                .filter(|(s, _)| *s == DataSource::CoinGecko)
                .count(),
            2
        );
    }

    #[test]
    fn a_failing_source_backs_off_alone_keeping_its_last_price_stale() {
        let (engine, book, provider) = setup();
        provider.price("BTCUSDT", 100.0);
        provider.price("bitcoin", 7.0);
        let btc = id(DataSource::Binance, "BTCUSDT");
        let gecko = id(DataSource::CoinGecko, "bitcoin");
        lock(&engine).update(1, vec![btc.clone(), gecko.clone()], true, T0);
        round(&engine, &provider, T0);

        *provider.failure.lock().unwrap() = Some(QuoteFailure::Failed("boom".into()));
        round(&engine, &provider, T0 + 5_000);
        let quote = book.get(&btc).unwrap();
        assert_eq!(quote.last, Some(100.0), "last price kept");
        assert_eq!(quote.freshness, Freshness::Stale);
        let health = lock(&engine).provider_health();
        let binance = health
            .iter()
            .find(|health| health.source == DataSource::Binance)
            .unwrap();
        assert_eq!(binance.last_error.as_deref(), Some("boom"));
        assert_eq!(binance.last_successful_quote_ms, Some(T0));
        assert!(binance.retry_at_ms.is_some());
        assert!(binance.has_data_gap);
        let before = provider.calls().len();

        // Backoff 5 s: not due at +7 s (5 + 5 = 10), due at +10 s.
        round(&engine, &provider, T0 + 7_000);
        assert_eq!(provider.calls().len(), before);
        round(&engine, &provider, T0 + 10_000);
        assert_eq!(provider.calls().len(), before + 1);
        // The second failure doubles to 10 s.
        round(&engine, &provider, T0 + 15_000);
        assert_eq!(provider.calls().len(), before + 1);
        round(&engine, &provider, T0 + 20_000);
        assert_eq!(provider.calls().len(), before + 2);

        // Recovery resets everything.
        *provider.failure.lock().unwrap() = None;
        round(&engine, &provider, T0 + 40_000);
        let quote = book.get(&btc).unwrap();
        assert_eq!(quote.freshness, Freshness::Live);
        assert_eq!(quote.note, None);
        let health = lock(&engine).provider_health();
        let binance = health
            .iter()
            .find(|health| health.source == DataSource::Binance)
            .unwrap();
        assert_eq!(binance.last_error, None);
        assert_eq!(binance.last_successful_quote_ms, Some(T0 + 40_000));
    }

    #[test]
    fn a_failure_with_no_history_says_why_instead_of_a_bare_dash() {
        let (engine, book, provider) = setup();
        *provider.failure.lock().unwrap() = Some(QuoteFailure::RateLimited);
        let btc = id(DataSource::Binance, "BTCUSDT");
        lock(&engine).update(1, vec![btc.clone()], true, T0);
        round(&engine, &provider, T0);
        let quote = book.get(&btc).unwrap();
        assert_eq!(quote.last, None);
        assert_eq!(quote.freshness, Freshness::Unavailable);
        assert_eq!(quote.note.as_deref(), Some("Rate limited by the provider"));
    }

    #[test]
    fn a_rate_limit_backs_off_further_than_a_plain_failure() {
        let (engine, _book, provider) = setup();
        *provider.failure.lock().unwrap() = Some(QuoteFailure::RateLimited);
        lock(&engine).update(1, vec![id(DataSource::Binance, "BTCUSDT")], true, T0);
        round(&engine, &provider, T0);
        round(&engine, &provider, T0 + 14_000);
        assert_eq!(provider.calls().len(), 1, "15 s minimum");
        round(&engine, &provider, T0 + 15_000);
        assert_eq!(provider.calls().len(), 2);
    }

    #[test]
    fn a_bad_symbol_is_found_by_halving_and_left_out_for_ten_minutes() {
        let (engine, book, provider) = setup();
        for symbol in ["AAAUSDT", "BBBUSDT", "CCCUSDT", "DDDUSDT"] {
            provider.price(symbol, 10.0);
        }
        provider.reject.lock().unwrap().insert("CCCUSDT".to_owned());
        let instruments: Vec<InstrumentId> = ["AAAUSDT", "BBBUSDT", "CCCUSDT", "DDDUSDT"]
            .iter()
            .map(|s| id(DataSource::Binance, s))
            .collect();
        lock(&engine).update(1, instruments.clone(), true, T0);
        round(&engine, &provider, T0);

        assert_eq!(last(&book, &instruments[0]), Some(10.0));
        assert_eq!(last(&book, &instruments[1]), Some(10.0));
        assert_eq!(last(&book, &instruments[3]), Some(10.0));
        let bad = book.get(&instruments[2]).unwrap();
        assert_eq!(bad.last, None);
        assert_eq!(
            bad.note.as_deref(),
            Some("The provider doesn't know this market")
        );

        // The next rounds skip the bad one: every call after the first omits it.
        let first_round_calls = provider.calls().len();
        round(&engine, &provider, T0 + 5_000);
        for (_, symbols) in &provider.calls()[first_round_calls..] {
            assert!(!symbols.contains(&"CCCUSDT".to_owned()), "{symbols:?}");
        }
        // After ten minutes it is tried again.
        let before = provider.calls().len();
        round(&engine, &provider, T0 + 601_000);
        assert!(
            provider.calls()[before..]
                .iter()
                .any(|(_, symbols)| symbols.contains(&"CCCUSDT".to_owned()))
        );
    }

    #[test]
    fn if_every_symbol_is_rejected_the_source_is_treated_as_down_not_the_symbols() {
        let (engine, book, provider) = setup();
        provider.reject.lock().unwrap().insert("AAAUSDT".to_owned());
        let only = id(DataSource::Binance, "AAAUSDT");
        lock(&engine).update(1, vec![only.clone()], true, T0);
        round(&engine, &provider, T0);
        assert_eq!(
            book.get(&only).unwrap().note.as_deref(),
            Some("The provider returned no quote"),
            "no neighbour answered, so the symbol is not blamed"
        );
        round(&engine, &provider, T0 + 5_000);
        assert_eq!(
            provider.calls().len(),
            2,
            "and it is retried at the normal pace"
        );
    }

    #[test]
    fn missing_credentials_mark_stocks_unsupported_and_retry_after_a_minute() {
        let (engine, book, provider) = setup();
        *provider.failure.lock().unwrap() = Some(QuoteFailure::CredentialsMissing);
        let aapl = id(DataSource::Alpaca, "AAPL");
        lock(&engine).update(1, vec![aapl.clone()], true, T0);
        round(&engine, &provider, T0);
        let quote = book.get(&aapl).unwrap();
        assert_eq!(quote.freshness, Freshness::Unsupported);
        assert!(quote.note.unwrap().contains("Alpaca"));
        round(&engine, &provider, T0 + 30_000);
        assert_eq!(provider.calls().len(), 1);
        round(&engine, &provider, T0 + 61_000);
        assert_eq!(provider.calls().len(), 2);
    }

    #[test]
    fn prices_age_into_stale_when_nothing_refreshes_them() {
        let (engine, book, provider) = setup();
        provider.price("BTCUSDT", 100.0);
        let btc = id(DataSource::Binance, "BTCUSDT");
        lock(&engine).update(1, vec![btc.clone()], true, T0);
        round(&engine, &provider, T0);
        assert_eq!(book.get(&btc).unwrap().freshness, Freshness::Live);
        lock(&engine).age_quotes(T0 + 44_000);
        assert_eq!(book.get(&btc).unwrap().freshness, Freshness::Live);
        lock(&engine).age_quotes(T0 + 46_000);
        let quote = book.get(&btc).unwrap();
        assert_eq!(quote.freshness, Freshness::Stale);
        assert_eq!(quote.last, Some(100.0), "dimmed, not dropped");
        // Becoming visible again ages quotes that went unpolled meanwhile.
        lock(&engine).update(1, vec![btc.clone()], true, T0 + 100_000);
        assert_eq!(book.get(&btc).unwrap().freshness, Freshness::Stale);
    }

    #[test]
    fn coinbase_is_fed_by_the_socket_with_rest_as_a_fallback() {
        let (engine, book, provider) = setup();
        provider.price("BTC-USD", 100.0);
        let btc = id(DataSource::Coinbase, "BTC-USD");
        lock(&engine).update(1, vec![btc.clone()], true, T0);
        assert_eq!(
            lock(&engine).coinbase_products(),
            vec!["BTC-USD".to_owned()]
        );

        // The socket has not answered yet: REST waits out the fallback delay.
        round(&engine, &provider, T0 + 1_000);
        assert!(provider.calls().is_empty());
        round(&engine, &provider, T0 + 6_000);
        assert_eq!(
            provider.calls().len(),
            1,
            "REST fills a product the socket has not answered"
        );

        // A tick lands: the socket now owns the price and REST stays quiet...
        lock(&engine).receive_ticks(
            vec![CoinbaseTick {
                product_id: "btc-usd".to_owned(),
                price: 105.0,
                open_24h: Some(100.0),
                volume_24h: Some(2.0),
                time_ms: None,
            }],
            T0 + 7_000,
        );
        let quote = book.get(&btc).unwrap();
        assert_eq!(quote.last, Some(105.0));
        assert!((quote.change_percent.unwrap() - 5.0).abs() < 1e-9);
        round(&engine, &provider, T0 + 20_000);
        assert_eq!(provider.calls().len(), 1);
        // ...until it has been quiet for 30 s, when REST refreshes it.
        round(&engine, &provider, T0 + 37_000);
        assert_eq!(provider.calls().len(), 2);
    }

    #[test]
    fn ticks_for_unwatched_products_are_ignored_and_hiding_closes_the_socket_set() {
        let (engine, book, _provider) = setup();
        let btc = id(DataSource::Coinbase, "BTC-USD");
        lock(&engine).update(1, vec![btc.clone()], true, T0);
        lock(&engine).receive_ticks(
            vec![CoinbaseTick {
                product_id: "DOGE-USD".to_owned(),
                price: 1.0,
                open_24h: None,
                volume_24h: None,
                time_ms: None,
            }],
            T0,
        );
        assert!(
            book.get(&InstrumentId::new(DataSource::Coinbase, "DOGE-USD"))
                .is_none()
        );
        lock(&engine).update(1, vec![btc], false, T0 + 1);
        assert!(lock(&engine).coinbase_products().is_empty());
    }

    #[test]
    fn bare_coinbase_symbols_subscribe_under_their_product_id() {
        let (engine, _book, _provider) = setup();
        lock(&engine).update(1, vec![id(DataSource::Coinbase, "eth")], true, T0);
        assert_eq!(
            lock(&engine).coinbase_products(),
            vec!["ETH-USD".to_owned()]
        );
    }

    #[test]
    fn dex_pairs_wait_for_their_network_then_batch_by_it() {
        let (engine, book, provider) = setup();
        provider.price("0xabc", 3.0);
        provider
            .chains
            .lock()
            .unwrap()
            .insert("0xabc".to_owned(), "base".to_owned());
        let pair = id(DataSource::DexScreener, "0xabc");
        let lost = id(DataSource::DexScreener, "0xlost");
        lock(&engine).update(1, vec![pair.clone(), lost.clone()], true, T0);

        round(&engine, &provider, T0);
        assert_eq!(
            last(&book, &pair),
            Some(3.0),
            "chain resolved, then priced in the same round"
        );
        let calls = provider.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, vec!["0xabc".to_owned()]);
        let lost_quote = book.get(&lost).unwrap();
        assert_eq!(
            lost_quote.note.as_deref(),
            Some("Couldn't find this pair's network")
        );
        // Not asked again this session.
        assert!(lock(&engine).missing_chains().is_empty());
    }

    #[test]
    fn index_charts_are_unsupported_not_polled() {
        let (engine, book, provider) = setup();
        let cmc = id(DataSource::CoinMarketCap, "fear-greed");
        lock(&engine).update(1, vec![cmc.clone()], true, T0);
        round(&engine, &provider, T0);
        assert!(provider.calls().is_empty());
        assert_eq!(book.get(&cmc).unwrap().freshness, Freshness::Unsupported);
    }

    #[test]
    fn large_batches_are_split_to_the_providers_limit() {
        let (engine, _book, provider) = setup();
        let instruments: Vec<InstrumentId> = (0..45)
            .map(|index| id(DataSource::Coinbase, &format!("A{index}-USD")))
            .collect();
        for instrument in &instruments {
            provider.price(&instrument.api_symbol(), 1.0);
        }
        lock(&engine).update(1, instruments, true, T0);
        round(&engine, &provider, T0 + 10_000);
        let sizes: Vec<usize> = provider
            .calls()
            .iter()
            .map(|(_, symbols)| symbols.len())
            .collect();
        assert_eq!(sizes, vec![20, 20, 5]);
    }
}
