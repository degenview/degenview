//! The watchlist quote state machine, free of threads and clocks of its own so every
//! rule is testable with a fake provider and explicit times.
//!
//! Sidebars (one per window) say which markets they need and whether they are on
//! screen. Needs are merged by market, so one that sits in three lists or two windows is
//! requested once. Prices come from one batched request per source per round, plus a
//! one shared ticker socket per streamed source. A source that fails or is rate limited backs off
//! alone, keeping its last prices marked stale; a symbol the provider rejects is found
//! by splitting the batch and then left out for ten minutes.

use super::book::WatchlistQuoteBook;
use super::provider::{QuoteFailure, QuoteProvider, QuoteRequest};
use super::quote_stream::StreamTick;
use crate::lock::lock;
use crate::model::alert::AlertProviderHealth;
use crate::model::market::profile::Class;
use crate::model::watchlist::quote::{Freshness, SourceQuote, WatchlistQuote, evaluate_freshness};
use crate::model::{DataSource, InstrumentId};
use crate::service::market::venue;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;

#[derive(Clone, Debug)]
pub struct Config {
    /// How often the loop plans a round. Each source is refreshed at its own interval.
    pub poll_interval_ms: i64,
    /// A source that fails is tried again after this long (seconds), doubling up to the cap.
    pub failure_backoff: (f64, f64),
    /// A rate limit backs off further.
    pub rate_limit_backoff: (f64, f64),
    pub rejected_retry_ms: i64,
    /// What the engine remembers about a market nobody asks for is dropped after this long.
    pub idle_grace_ms: i64,
    /// Products of a streamed source the socket has not answered yet are filled from REST after
    /// this long...
    pub stream_fallback_delay_ms: i64,
    /// ...and any product the socket has been quiet about is refreshed from REST this often.
    pub stream_refresh_ms: i64,
    /// The price a day ago of a streamed market, as its last tick or REST answer gave it, is lent to
    /// the next answer that gives none for this long: a quiet market keeps its change instead of
    /// losing it every refresh, and a venue whose ticks are only trades keeps it between REST calls.
    /// A venue whose ticks give none has it asked of REST again at half this age.
    pub stream_reference_ms: i64,
    pub chain_lookups_per_round: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            poll_interval_ms: 5_000,
            failure_backoff: (5.0, 30.0),
            rate_limit_backoff: (15.0, 60.0),
            rejected_retry_ms: 600_000,
            idle_grace_ms: 600_000,
            stream_fallback_delay_ms: 6_000,
            stream_refresh_ms: 30_000,
            stream_reference_ms: 600_000,
            chain_lookups_per_round: 5,
        }
    }
}

/// How long a sidebar's demand lasts without being renewed. A sidebar draws every
/// frame it is on screen; one on a background tab or in a hidden or covered window
/// draws none, so its demand lapses instead of polling for nobody.
pub const SIDEBAR_LEASE_MS: i64 = 3_000;

/// The least time between two passes that forget idle markets.
const PRUNE_EVERY_MS: i64 = 5_000;

/// What a streamed market's last answer said of the price a day ago, and when REST is to be asked.
struct StreamReference {
    /// `None` is an answer that gave none, noted so that a market with nothing to find is not asked
    /// about every round.
    price: Option<f64>,
    taken: i64,
    /// A refused or empty REST answer waits `Config::stream_refresh_ms`, an answer half of
    /// `Config::stream_reference_ms`.
    next_ask: i64,
}

struct Consumer {
    instruments: BTreeMap<String, InstrumentId>,
    active: bool,
    /// Demand that lapses unless renewed (see `SIDEBAR_LEASE_MS`).
    leased: bool,
    /// When the consumer last asked.
    seen_ms: i64,
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

/// What a fetch brought back: prices, the symbols the provider refused by name, and the
/// requests that failed for a reason that is not a symbol's fault (with the symbols they
/// carried). A batch can hold all three, so one bad page never costs the good ones.
#[derive(Debug, Default)]
pub struct Outcome {
    pub quotes: HashMap<String, SourceQuote>,
    pub rejected: HashSet<String>,
    pub failed: Vec<(Vec<String>, QuoteFailure)>,
}

impl Outcome {
    pub fn from_quotes(quotes: HashMap<String, SourceQuote>) -> Self {
        Self {
            quotes,
            ..Self::default()
        }
    }

    pub fn failing(symbols: Vec<String>, failure: QuoteFailure) -> Self {
        Self {
            failed: vec![(symbols, failure)],
            ..Self::default()
        }
    }

    fn absorb(&mut self, part: Outcome) {
        for (symbol, quote) in part.quotes {
            self.quotes.entry(symbol).or_insert(quote);
        }
        self.rejected.extend(part.rejected);
        self.failed.extend(part.failed);
    }

    /// The failure that decides the source's backoff: a rate limit outranks a plain failure,
    /// which outranks a missing key.
    fn worst_failure(&self) -> Option<&QuoteFailure> {
        self.failed
            .iter()
            .map(|(_, failure)| failure)
            .max_by_key(|failure| match failure {
                QuoteFailure::RateLimited => 3,
                QuoteFailure::Failed(_) | QuoteFailure::Rejected(_) => 2,
                QuoteFailure::CredentialsMissing => 1,
            })
    }
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
    /// Per streamed source: upper-case product id -> instrument key.
    stream_products: HashMap<DataSource, HashMap<String, String>>,
    /// When each streamed market was first asked for, by instrument key.
    stream_subscribed_at: HashMap<String, i64>,
    /// What the last answer said of the price a day ago of each streamed market, by instrument key.
    stream_references: HashMap<String, StreamReference>,
    /// When a streamed market's 24h volume was last given, by instrument key.
    stream_volumes: HashMap<String, i64>,
    /// When each remembered market stopped being asked for (see `Config::idle_grace_ms`).
    idle_since: HashMap<String, i64>,
    last_prune_ms: i64,
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
            stream_products: HashMap::new(),
            stream_subscribed_at: HashMap::new(),
            stream_references: HashMap::new(),
            stream_volumes: HashMap::new(),
            idle_since: HashMap::new(),
            last_prune_ms: i64::MIN / 2,
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
        self.put(consumer, instruments, active, false, now);
    }

    /// Like `update`, for a sidebar: the demand also lapses when it stops being renewed
    /// (a background tab, a hidden or covered window), see `expire_leases`.
    pub fn update_leased(
        &mut self,
        consumer: u64,
        instruments: Vec<InstrumentId>,
        active: bool,
        now: i64,
    ) {
        self.put(consumer, instruments, active, true, now);
    }

    fn put(
        &mut self,
        consumer: u64,
        instruments: Vec<InstrumentId>,
        active: bool,
        leased: bool,
        now: i64,
    ) {
        let instruments: BTreeMap<String, InstrumentId> = instruments
            .into_iter()
            .map(|instrument| (instrument.key(), instrument))
            .collect();
        // A sidebar republishes its rows every frame: an unchanged set only renews.
        if let Some(held) = self.consumers.get_mut(&consumer)
            && held.instruments == instruments
            && held.active == active
            && held.leased == leased
        {
            held.seen_ms = now;
            return;
        }
        self.consumers.insert(
            consumer,
            Consumer {
                instruments,
                active,
                leased,
                seen_ms: now,
            },
        );
        self.reconcile(now);
    }

    /// Silences leased consumers that stopped asking; true when any did. They keep their
    /// place and wake on their next `update_leased`.
    pub fn expire_leases(&mut self, now: i64) -> bool {
        // The loop calls this every tick, even with nothing subscribed: the moment to
        // forget markets that nobody has asked for in a while.
        let wanted = self.wanted();
        self.prune(&wanted, now);
        let mut expired = false;
        for consumer in self.consumers.values_mut() {
            if consumer.leased && consumer.active && now - consumer.seen_ms > SIDEBAR_LEASE_MS {
                consumer.active = false;
                expired = true;
            }
        }
        if expired {
            self.reconcile(now);
        }
        expired
    }

    /// A leased consumer is half way to lapsing: the caller asks the UI to draw so a
    /// visible sidebar renews (a hidden one will not, and lapses).
    pub fn lease_aging(&self, now: i64) -> bool {
        self.consumers.values().any(|consumer| {
            consumer.leased && consumer.active && now - consumer.seen_ms > SIDEBAR_LEASE_MS / 2
        })
    }

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

    /// The products the shared socket of `source` should carry, sorted.
    pub fn stream_products(&self, source: DataSource) -> Vec<String> {
        let mut products: Vec<String> = self
            .stream_products
            .get(&source)
            .map(|products| products.keys().cloned().collect())
            .unwrap_or_default();
        products.sort();
        products
    }

    fn reconcile(&mut self, now: i64) {
        let wanted = self.wanted();
        self.prune(&wanted, now);
        if wanted.is_empty() {
            self.stream_products.clear();
            return;
        }
        for (key, instrument) in &wanted {
            if instrument.source.profile().class == Class::Index && !self.known.contains_key(key) {
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
        self.sync_streams(&wanted, now);
    }

    /// Forgets markets nobody has asked for during the grace period: their prices, the
    /// moments they were confirmed, their refusals and socket bookkeeping. Without this a
    /// long session that imports, deletes and browses many markets only ever grows, and a
    /// market that returns hours later would show an hours-old price at once.
    fn prune(&mut self, wanted: &BTreeMap<String, InstrumentId>, now: i64) {
        // Reconciling and planning are frequent; forgetting is not urgent.
        if now - self.last_prune_ms < PRUNE_EVERY_MS {
            return;
        }
        self.last_prune_ms = now;
        let remembered: Vec<String> = self
            .known
            .keys()
            .chain(self.received.keys())
            .chain(self.rejected.keys())
            .chain(self.stream_subscribed_at.keys())
            .chain(self.stream_references.keys())
            .chain(self.stream_volumes.keys())
            .chain(self.unresolved_chains.iter())
            .cloned()
            .collect();
        for key in &remembered {
            if wanted.contains_key(key) {
                self.idle_since.remove(key);
            } else {
                self.idle_since.entry(key.clone()).or_insert(now);
            }
        }
        let expired: Vec<String> = self
            .idle_since
            .iter()
            .filter(|(_, since)| now - **since > self.config.idle_grace_ms)
            .map(|(key, _)| key.clone())
            .collect();
        if expired.is_empty() {
            return;
        }
        for key in &expired {
            self.idle_since.remove(key);
            self.known.remove(key);
            self.received.remove(key);
            self.rejected.remove(key);
            self.stream_subscribed_at.remove(key);
            self.stream_references.remove(key);
            self.stream_volumes.remove(key);
            self.unresolved_chains.remove(key);
        }
        self.book.remove(&expired);
    }

    #[cfg(test)]
    pub fn remembered_markets(&self) -> usize {
        self.known.len()
            + self.received.len()
            + self.rejected.len()
            + self.stream_subscribed_at.len()
            + self.stream_references.len()
            + self.stream_volumes.len()
            + self.unresolved_chains.len()
            + self.idle_since.len()
    }

    fn sync_streams(&mut self, wanted: &BTreeMap<String, InstrumentId>, now: i64) {
        let mut streams: HashMap<DataSource, HashMap<String, String>> = HashMap::new();
        for (key, instrument) in wanted {
            if instrument.source.streams_quotes() {
                streams
                    .entry(instrument.source)
                    .or_default()
                    .insert(instrument.api_symbol(), key.clone());
                self.stream_subscribed_at.entry(key.clone()).or_insert(now);
            }
        }
        self.stream_products = streams;
    }

    // MARK: Planning a round

    /// DEX pairs saved without a known network, to look up this round.
    pub fn missing_chains(&self) -> Vec<InstrumentId> {
        self.wanted()
            .into_iter()
            .filter(|(key, instrument)| {
                instrument.source.profile().class == Class::Dex
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
            // Index charts and portfolio cards have no quote to poll.
            source if source.profile().class == Class::Index => None,
            source if source.streams_quotes() => {
                // The socket supplies prices; REST fills a product it has not answered, and
                // refreshes one it has gone quiet on, so a thin market's price never ages.
                if let Some(last) = self.received.get(key) {
                    return (now - last >= self.config.stream_refresh_ms
                        || self.reference_wanted(source, key, now))
                    .then_some(source);
                }
                let since = self.stream_subscribed_at.get(key)?;
                (now - since >= self.config.stream_fallback_delay_ms).then_some(source)
            }
            // A DEX pair is asked for by its network, once that is known.
            source if source.profile().class == Class::Dex => {
                self.chains.contains_key(key).then_some(source)
            }
            source => Some(source),
        }
    }

    fn is_due(&self, source: DataSource, now: i64) -> bool {
        let state = self.sources.get(&source).cloned().unwrap_or_default();
        if now < state.next_allowed_ms {
            return false;
        }
        // A second of slack: rounds drift a little late.
        now - state.last_poll_ms >= source.profile().quotes.refresh_secs as i64 * 1_000 - 1_000
    }

    /// One round: every due source gets one batched request.
    pub fn plan(&mut self, now: i64) -> Vec<Planned> {
        let wanted = self.wanted();
        self.prune(&wanted, now);
        self.age_quotes(now);
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

        // Why each failed symbol failed; the first reason wins.
        let mut failed: HashMap<&str, &QuoteFailure> = HashMap::new();
        for (symbols, failure) in &outcome.failed {
            for symbol in symbols {
                failed.entry(symbol.as_str()).or_insert(failure);
            }
        }
        let source_answered = !outcome.quotes.is_empty();
        match outcome.worst_failure() {
            Some(QuoteFailure::CredentialsMissing) => {
                state.next_allowed_ms = now + 60_000;
                state.last_error = Some(QuoteFailure::CredentialsMissing.message());
            }
            Some(failure) => {
                let (low, high) = if matches!(failure, QuoteFailure::RateLimited) {
                    self.config.rate_limit_backoff
                } else {
                    self.config.failure_backoff
                };
                state.backoff_secs = (state.backoff_secs * 2.0).max(low).min(high);
                state.next_allowed_ms = now + (state.backoff_secs * 1_000.0) as i64;
                state.last_error = Some(failure.message());
            }
            None => {
                state.backoff_secs = 0.0;
                state.next_allowed_ms = i64::MIN;
                // A symbol is only called bad when its neighbours answered; if nothing did,
                // the source is down.
                state.last_error =
                    (!source_answered).then(|| "The provider returned no quote".to_owned());
            }
        }
        if source_answered {
            state.last_successful_quote_ms = Some(now);
        }

        for (key, request) in &planned.requests {
            // A market whose socket is ticking is asked of REST only for what its ticks lack: REST
            // never replaces its newer price, and a call that fails or omits it never marks it stale.
            let ticking = self.is_ticking(source, key, now);
            if let Some(answer) = outcome.quotes.get(&request.symbol) {
                let mut quote = *answer;
                if source.streams_quotes() {
                    self.remember_reference(key, quote.previous_day_price, now);
                    self.remember_volume(key, quote.volume_24h, now);
                }
                quote = self.with_stream_reference(source, key, quote, now);
                let held = ticking
                    .then(|| self.known.get(key))
                    .flatten()
                    .and_then(|held| held.last.map(|price| (price, held.timestamp_ms)));
                match held {
                    Some((price, timestamp_ms)) => {
                        quote.price = price;
                        quote.timestamp_ms = timestamp_ms;
                    }
                    None => {
                        self.received.insert(key.clone(), now);
                    }
                }
                let mut row = WatchlistQuote::from_source(source, &quote, now);
                if ticking {
                    self.keep_volume(key, quote.volume_24h, &mut row, now);
                }
                updates.insert(key.clone(), row);
                self.rejected.remove(key);
                continue;
            }
            if source.streams_quotes() {
                self.note_unanswered(key, now);
            }
            let row = if let Some(failure) = failed.get(request.symbol.as_str()) {
                match failure {
                    QuoteFailure::CredentialsMissing => {
                        WatchlistQuote::unsupported(Some(failure.message()))
                    }
                    _ => self.fallback(key, Some(failure.message())),
                }
            } else if source_answered && outcome.rejected.contains(&request.symbol) {
                self.rejected
                    .insert(key.clone(), now + self.config.rejected_retry_ms);
                WatchlistQuote {
                    freshness: Freshness::Unavailable,
                    note: Some("The provider doesn't know this market".to_owned()),
                    ..WatchlistQuote::default()
                }
            } else {
                self.fallback(key, Some("The provider returned no quote".to_owned()))
            };
            if !ticking {
                updates.insert(key.clone(), row);
            }
        }
        self.sources.insert(source, state);
        for (key, quote) in &updates {
            self.known.insert(key.clone(), quote.clone());
        }
        self.book.apply(updates);
    }

    /// `quote` of a streamed market with the price a day ago its last tick or REST answer gave,
    /// when it gives none itself and that answer is recent enough to still say what a day ago was.
    fn with_stream_reference(
        &self,
        source: DataSource,
        key: &str,
        quote: SourceQuote,
        now: i64,
    ) -> SourceQuote {
        if quote.previous_day_price.is_some() || !source.streams_quotes() {
            return quote;
        }
        SourceQuote {
            previous_day_price: self.stream_reference(key, now),
            ..quote
        }
    }

    /// The price a day ago an answer gave for a streamed market, while it is recent enough to still
    /// say what a day ago was.
    fn stream_reference(&self, key: &str, now: i64) -> Option<f64> {
        self.stream_references
            .get(key)
            .filter(|held| now - held.taken <= self.config.stream_reference_ms)
            .and_then(|held| held.price)
    }

    /// Notes what an answer said of the price a day ago of a streamed market. An answer that gives
    /// none only notes that it was asked, and never replaces a price that is still good.
    fn remember_reference(&mut self, key: &str, reference: Option<f64>, now: i64) {
        let next_ask = now + self.config.stream_reference_ms / 2;
        let held = self.stream_reference(key, now).is_some();
        match self.stream_references.get_mut(key) {
            Some(entry) if reference.is_none() && held => entry.next_ask = next_ask,
            _ => {
                self.stream_references.insert(
                    key.to_owned(),
                    StreamReference {
                        price: reference,
                        taken: now,
                        next_ask,
                    },
                );
            }
        }
    }

    /// Notes that REST was asked for a streamed market and gave nothing (the call failed or left it
    /// out), so it is asked again after `Config::stream_refresh_ms` and not in every round.
    fn note_unanswered(&mut self, key: &str, now: i64) {
        let next_ask = now + self.config.stream_refresh_ms;
        self.stream_references
            .entry(key.to_owned())
            .and_modify(|entry| entry.next_ask = next_ask)
            .or_insert(StreamReference {
                price: None,
                taken: now,
                next_ask,
            });
    }

    /// Whether REST is to be asked for the price a day ago of a streamed market whose venue's ticks
    /// do not say it: none was asked for yet, or the next ask is due.
    fn reference_wanted(&self, source: DataSource, key: &str, now: i64) -> bool {
        let ticks_say_it = venue::of(source)
            .quotes()
            .is_none_or(|protocol| protocol.ticks_carry_reference());
        !ticks_say_it
            && self
                .stream_references
                .get(key)
                .is_none_or(|held| now >= held.next_ask)
    }

    /// Notes when a streamed market's answer gave a 24h volume.
    fn remember_volume(&mut self, key: &str, volume: Option<f64>, now: i64) {
        if volume.is_some() {
            self.stream_volumes.insert(key.to_owned(), now);
        }
    }

    /// Gives `row`, built from an answer that gave no volume (`given`), the volume of the last
    /// answer that did, while it is recent enough to still say what the volume was.
    fn keep_volume(&self, key: &str, given: Option<f64>, row: &mut WatchlistQuote, now: i64) {
        let recent = self
            .stream_volumes
            .get(key)
            .is_some_and(|taken| now - taken <= self.config.stream_reference_ms);
        if let (None, true, Some(known)) = (given, recent, self.known.get(key)) {
            row.volume = known.volume;
            row.volume_kind = known.volume_kind;
        }
    }

    /// Whether a streamed market has been confirmed within `Config::stream_refresh_ms`, by a tick
    /// or an answer, and has a price to show.
    fn is_ticking(&self, source: DataSource, key: &str, now: i64) -> bool {
        source.streams_quotes()
            && self
                .received
                .get(key)
                .is_some_and(|last| now - last < self.config.stream_refresh_ms)
            && self
                .known
                .get(key)
                .is_some_and(|quote| quote.last.is_some())
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
        DataSource::ALL
            .into_iter()
            .find(|source| source.watchlist_slug() == slug)
    }

    // MARK: Ticks

    /// Publishes the latest tick of each product; the coordinator drains its channel on a
    /// short interval, so a busy feed becomes a few updates a second.
    pub fn receive_ticks(&mut self, ticks: Vec<StreamTick>, now: i64) {
        let mut latest: HashMap<String, WatchlistQuote> = HashMap::new();
        let mut heard: HashSet<DataSource> = HashSet::new();
        for tick in ticks {
            // A product is named as the venue takes it: upper case for most, but Hyperliquid's
            // names are case-sensitive (`xyz:SP500`, `kPEPE`), so a tick is looked up as it came
            // first and by its upper-case spelling after.
            let Some(key) = self
                .stream_products
                .get(&tick.source)
                .and_then(|products| {
                    products
                        .get(&tick.product_id)
                        .or_else(|| products.get(&tick.product_id.to_uppercase()))
                })
                .cloned()
            else {
                continue;
            };
            if let Some(reference) = tick.quote.previous_day_price {
                self.remember_reference(&key, Some(reference), now);
            }
            // A tick that does not say the price a day ago or the volume (a venue whose socket sends
            // trades) keeps what the last answer said.
            let quote = self.with_stream_reference(tick.source, &key, tick.quote, now);
            self.remember_volume(&key, quote.volume_24h, now);
            let mut row = WatchlistQuote::from_source(tick.source, &quote, now);
            self.keep_volume(&key, quote.volume_24h, &mut row, now);
            latest.insert(key, row);
            heard.insert(tick.source);
        }
        if latest.is_empty() {
            return;
        }
        for (key, quote) in &latest {
            self.received.insert(key.clone(), now);
            self.known.insert(key.clone(), quote.clone());
        }
        for source in heard {
            let state = self.sources.entry(source).or_default();
            state.last_successful_quote_ms = Some(now);
            state.last_error = None;
            state.next_allowed_ms = i64::MIN;
        }
        self.book.apply(latest);
    }
}

/// A stable order for sources, so rounds are deterministic.
fn source_order(source: DataSource) -> u8 {
    DataSource::ALL
        .iter()
        .position(|candidate| *candidate == source)
        .unwrap_or(usize::MAX) as u8
}

// MARK: Fetching

/// The most requests one source may spend in a round looking for the symbols a provider
/// refuses: enough for several delisted markets in a full batch, far below one request per
/// symbol. What it cannot isolate is reported as failed, not retried.
const ISOLATION_BUDGET: usize = 64;

/// Fetches a batch in provider-sized pages. When the provider rejects a page outright,
/// halves it until the bad symbols are found, so one delisted market does not blank the
/// list. A rate limit stops the round's remaining requests at once.
pub fn fetch(
    provider: &dyn QuoteProvider,
    source: DataSource,
    requests: &[QuoteRequest],
) -> Outcome {
    fetch_chunked(provider, source, requests, source.profile().quotes.batch)
}

fn fetch_chunked(
    provider: &dyn QuoteProvider,
    source: DataSource,
    requests: &[QuoteRequest],
    size: usize,
) -> Outcome {
    let mut budget = ISOLATION_BUDGET;
    let mut merged = Outcome::default();
    let mut limited: Option<QuoteFailure> = None;
    for page in requests.chunks(size.max(1)) {
        if let Some(failure) = &limited {
            merged.absorb(Outcome::failing(symbols(page), failure.clone()));
            continue;
        }
        let part = fetch_page(provider, source, page, &mut budget);
        if part
            .failed
            .iter()
            .any(|(_, failure)| matches!(failure, QuoteFailure::RateLimited))
        {
            limited = Some(QuoteFailure::RateLimited);
        }
        merged.absorb(part);
    }
    merged
}

fn symbols(requests: &[QuoteRequest]) -> Vec<String> {
    requests
        .iter()
        .map(|request| request.symbol.clone())
        .collect()
}

/// One page: asks the provider; on a rejection, isolates the refused symbols.
fn fetch_page(
    provider: &dyn QuoteProvider,
    source: DataSource,
    requests: &[QuoteRequest],
    budget: &mut usize,
) -> Outcome {
    let mut rejection = String::new();
    let outcome = bisect(provider, source, requests, budget, &mut rejection);
    // Every symbol of a page refused one by one is the page's fault, not theirs: a request
    // the provider will not take (a revoked key, a region, a changed endpoint) must back
    // the source off, not mark a whole library as unknown markets.
    if requests.len() > 1
        && outcome.quotes.is_empty()
        && outcome.failed.is_empty()
        && outcome.rejected.len() == requests.len()
    {
        return Outcome::failing(symbols(requests), QuoteFailure::Failed(rejection));
    }
    outcome
}

fn bisect(
    provider: &dyn QuoteProvider,
    source: DataSource,
    requests: &[QuoteRequest],
    budget: &mut usize,
    rejection: &mut String,
) -> Outcome {
    match provider.quotes(source, requests) {
        Ok(outcome) => outcome,
        Err(QuoteFailure::Rejected(message)) => {
            if rejection.is_empty() {
                *rejection = message.clone();
            }
            if requests.len() <= 1 {
                return Outcome {
                    rejected: requests
                        .iter()
                        .map(|request| request.symbol.clone())
                        .collect(),
                    ..Outcome::default()
                };
            }
            if *budget < 2 {
                return Outcome::failing(
                    symbols(requests),
                    QuoteFailure::Failed(format!("{message} (too many markets to isolate)")),
                );
            }
            *budget -= 2;
            let (first, second) = requests.split_at(requests.len() / 2);
            let mut merged = bisect(provider, source, first, budget, rejection);
            // A rate limit in one half means the other half would meet it too.
            if merged
                .failed
                .iter()
                .any(|(_, failure)| matches!(failure, QuoteFailure::RateLimited))
            {
                merged.absorb(Outcome::failing(symbols(second), QuoteFailure::RateLimited));
            } else {
                merged.absorb(bisect(provider, source, second, budget, rejection));
            }
            merged
        }
        Err(failure) => Outcome::failing(symbols(requests), failure),
    }
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
            .zip(&plan)
            .map(|(handle, planned)| {
                handle.join().unwrap_or_else(|_| {
                    Outcome::failing(
                        planned
                            .requests
                            .iter()
                            .map(|(_, request)| request.symbol.clone())
                            .collect(),
                        QuoteFailure::Failed("Quote worker crashed".to_owned()),
                    )
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
    use crate::model::watchlist::quote::Freshness;
    use crate::model::watchlist::quote::VolumeKind;
    use std::sync::Arc;

    /// Answers from a table and records every call.
    #[derive(Default)]
    struct FakeProvider {
        prices: Mutex<HashMap<String, f64>>,
        reject: Mutex<HashSet<String>>,
        failure: Mutex<Option<QuoteFailure>>,
        /// A request holding any of these symbols fails as a page, not as a symbol.
        page_fails: Mutex<HashSet<String>>,
        /// Every request is refused as if it named an unknown symbol.
        reject_all: Mutex<bool>,
        /// From this many requests on, the provider answers with a rate limit.
        limit_after: Mutex<Option<usize>>,
        calls: Mutex<Vec<(DataSource, Vec<String>)>>,
        chains: Mutex<HashMap<String, String>>,
        /// Answers carry no price a day ago, as Kraken's REST ticker does not.
        without_reference: Mutex<bool>,
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
        ) -> Result<Outcome, QuoteFailure> {
            let mut symbols: Vec<String> = requests.iter().map(|r| r.symbol.clone()).collect();
            symbols.sort();
            let made = {
                let mut calls = self.calls.lock().unwrap();
                calls.push((source, symbols));
                calls.len()
            };
            if let Some(failure) = self.failure.lock().unwrap().clone() {
                return Err(failure);
            }
            if self
                .limit_after
                .lock()
                .unwrap()
                .is_some_and(|after| made > after)
            {
                return Err(QuoteFailure::RateLimited);
            }
            if self
                .page_fails
                .lock()
                .unwrap()
                .iter()
                .any(|symbol| requests.iter().any(|r| &r.symbol == symbol))
            {
                return Err(QuoteFailure::Failed("page failed".to_owned()));
            }
            if *self.reject_all.lock().unwrap() {
                return Err(QuoteFailure::Rejected("HTTP 400".to_owned()));
            }
            let rejects = self.reject.lock().unwrap();
            if requests.iter().any(|r| rejects.contains(&r.symbol)) {
                return Err(QuoteFailure::Rejected("unknown symbol".to_owned()));
            }
            let prices = self.prices.lock().unwrap();
            Ok(Outcome::from_quotes(
                requests
                    .iter()
                    .filter_map(|r| {
                        prices.get(&r.symbol).map(|price| {
                            (
                                r.symbol.clone(),
                                SourceQuote {
                                    price: *price,
                                    previous_day_price: (!*self.without_reference.lock().unwrap())
                                        .then_some(*price / 2.0),
                                    ..SourceQuote::default()
                                },
                            )
                        })
                    })
                    .collect(),
            ))
        }

        fn resolve_chain(&self, address: &str) -> Option<String> {
            self.chains.lock().unwrap().get(address).cloned()
        }
    }

    fn tick(product_id: &str, price: f64, open_24h: Option<f64>) -> StreamTick {
        StreamTick {
            source: DataSource::Coinbase,
            product_id: product_id.to_owned(),
            quote: SourceQuote {
                price,
                previous_day_price: open_24h,
                ..SourceQuote::default()
            },
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
    fn a_sidebar_that_stops_drawing_lapses_and_wakes_when_it_draws_again() {
        let (engine, book, provider) = setup();
        provider.price("BTCUSDT", 100.0);
        let btc = id(DataSource::Binance, "BTCUSDT");
        lock(&engine).update_leased(1, vec![btc.clone()], true, T0);
        round(&engine, &provider, T0);
        assert_eq!(provider.calls().len(), 1);

        // Drawing every frame renews the lease however long it goes on.
        for step in 1..=10 {
            let at = T0 + step * 1_000;
            lock(&engine).update_leased(1, vec![btc.clone()], true, at);
            assert!(!lock(&engine).expire_leases(at));
        }
        assert!(lock(&engine).has_subscriptions());

        // Half way to lapsing the loop asks the UI to draw; a covered window will not.
        let last_draw = T0 + 10_000;
        assert!(!lock(&engine).lease_aging(last_draw + 1_000));
        assert!(lock(&engine).lease_aging(last_draw + SIDEBAR_LEASE_MS / 2 + 1));
        assert!(lock(&engine).expire_leases(last_draw + SIDEBAR_LEASE_MS + 1));
        assert!(!lock(&engine).has_subscriptions());
        assert!(!lock(&engine).lease_aging(last_draw + SIDEBAR_LEASE_MS + 2));
        let calls = provider.calls().len();
        round(&engine, &provider, last_draw + 60_000);
        assert_eq!(
            provider.calls().len(),
            calls,
            "a lapsed sidebar polls nothing"
        );
        assert_eq!(
            last(&book, &btc),
            Some(100.0),
            "its prices stay in the book"
        );

        // Drawing again wakes it.
        lock(&engine).update_leased(1, vec![btc.clone()], true, last_draw + 61_000);
        assert!(lock(&engine).has_subscriptions());
        round(&engine, &provider, last_draw + 61_000);
        assert_eq!(provider.calls().len(), calls + 1);
    }

    #[test]
    fn demand_that_is_not_leased_never_lapses() {
        let (engine, _book, _provider) = setup();
        let btc = id(DataSource::Binance, "BTCUSDT");
        lock(&engine).update(1, vec![btc], true, T0);
        assert!(!lock(&engine).expire_leases(T0 + 10 * SIDEBAR_LEASE_MS));
        assert!(lock(&engine).has_subscriptions());
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

    /// `count` Binance markets `S000USDT`.., all priced, subscribed by window 1.
    fn many_binance(
        engine: &Arc<Mutex<Engine>>,
        provider: &FakeProvider,
        count: usize,
    ) -> Vec<InstrumentId> {
        let markets: Vec<InstrumentId> = (0..count)
            .map(|index| id(DataSource::Binance, &format!("S{index:03}USDT")))
            .collect();
        for market in &markets {
            provider.price(&market.symbol, 10.0);
        }
        lock(engine).update(1, markets.clone(), true, T0);
        markets
    }

    #[test]
    fn a_failed_page_keeps_the_prices_of_the_pages_that_answered() {
        let (engine, book, provider) = setup();
        let markets = many_binance(&engine, &provider, 250);
        // Three pages of 100/100/50; the middle one fails.
        provider
            .page_fails
            .lock()
            .unwrap()
            .insert("S150USDT".into());
        round(&engine, &provider, T0);
        assert_eq!(provider.calls().len(), 3, "every page is asked");
        assert_eq!(book.get(&markets[0]).unwrap().freshness, Freshness::Live);
        assert_eq!(book.get(&markets[249]).unwrap().freshness, Freshness::Live);
        let failed = book.get(&markets[150]).unwrap();
        assert_eq!(failed.freshness, Freshness::Unavailable);
        assert_eq!(failed.note.as_deref(), Some("page failed"));
        let health = lock(&engine).provider_health();
        let binance = health
            .iter()
            .find(|health| health.source == DataSource::Binance)
            .unwrap();
        assert_eq!(binance.last_error.as_deref(), Some("page failed"));
        assert_eq!(
            binance.last_successful_quote_ms,
            Some(T0),
            "a page answered"
        );
        assert!(binance.retry_at_ms.is_some(), "the source still backs off");
    }

    #[test]
    fn a_rate_limit_stops_the_rest_of_the_round() {
        let (engine, book, provider) = setup();
        let markets = many_binance(&engine, &provider, 250);
        *provider.limit_after.lock().unwrap() = Some(1);
        round(&engine, &provider, T0);
        assert_eq!(provider.calls().len(), 2, "no request after the 429");
        assert_eq!(book.get(&markets[0]).unwrap().freshness, Freshness::Live);
        assert_eq!(
            book.get(&markets[249]).unwrap().note.as_deref(),
            Some("Rate limited by the provider")
        );
        let health = lock(&engine).provider_health();
        let binance = health
            .iter()
            .find(|health| health.source == DataSource::Binance)
            .unwrap();
        assert_eq!(binance.retry_at_ms, Some(T0 + 15_000), "rate-limit backoff");
    }

    #[test]
    fn a_batch_the_provider_refuses_whole_is_a_source_failure_not_a_search() {
        let (engine, book, provider) = setup();
        let markets = many_binance(&engine, &provider, 100);
        *provider.reject_all.lock().unwrap() = true;
        round(&engine, &provider, T0);
        assert!(
            provider.calls().len() <= ISOLATION_BUDGET + 1,
            "{} requests for one batch",
            provider.calls().len()
        );
        let health = lock(&engine).provider_health();
        let binance = health
            .iter()
            .find(|health| health.source == DataSource::Binance)
            .unwrap();
        assert!(binance.last_error.is_some());
        assert!(
            binance.retry_at_ms.is_some(),
            "backed off, not retried at once"
        );
        assert!(
            !book.get(&markets[0]).unwrap().freshness.is_current(),
            "nothing reads live"
        );
        let before = provider.calls().len();
        round(&engine, &provider, T0 + 1_000);
        assert_eq!(provider.calls().len(), before, "the backoff holds");
    }

    #[test]
    fn a_few_bad_symbols_in_a_full_batch_are_found_within_the_budget() {
        let (engine, book, provider) = setup();
        let markets = many_binance(&engine, &provider, 100);
        for bad in [3, 47, 90] {
            provider
                .reject
                .lock()
                .unwrap()
                .insert(markets[bad].symbol.clone());
        }
        round(&engine, &provider, T0);
        assert!(provider.calls().len() <= ISOLATION_BUDGET + 1);
        for (index, market) in markets.iter().enumerate() {
            let quote = book.get(market).unwrap();
            if [3, 47, 90].contains(&index) {
                assert_eq!(quote.freshness, Freshness::Unavailable, "{index}");
            } else {
                assert_eq!(quote.freshness, Freshness::Live, "{index}");
            }
        }
    }

    #[test]
    fn markets_nobody_asks_for_are_forgotten_after_the_grace_period() {
        let (engine, book, provider) = setup();
        let grace = Config::default().idle_grace_ms;
        provider.price("BTCUSDT", 100.0);
        let btc = id(DataSource::Binance, "BTCUSDT");
        lock(&engine).update(1, vec![btc.clone()], true, T0);
        round(&engine, &provider, T0);
        assert_eq!(last(&book, &btc), Some(100.0));

        // Dropped from every sidebar: kept through the grace period, then forgotten.
        lock(&engine).update(1, Vec::new(), true, T0 + 10_000);
        round(&engine, &provider, T0 + 20_000);
        assert_eq!(last(&book, &btc), Some(100.0), "still remembered");
        lock(&engine).expire_leases(T0 + 10_000 + grace + 1);
        assert!(book.get(&btc).is_none());
        assert_eq!(lock(&engine).remembered_markets(), 0);

        // It comes back with no old price to show.
        lock(&engine).update(1, vec![btc.clone()], true, T0 + 2 * grace);
        assert!(book.get(&btc).is_none(), "nothing live until it is fetched");
    }

    #[test]
    fn a_market_asked_for_again_inside_the_grace_period_keeps_its_price() {
        let (engine, book, provider) = setup();
        provider.price("BTCUSDT", 100.0);
        let btc = id(DataSource::Binance, "BTCUSDT");
        lock(&engine).update(1, vec![btc.clone()], true, T0);
        round(&engine, &provider, T0);
        lock(&engine).update(1, Vec::new(), true, T0 + 10_000);
        lock(&engine).update(1, vec![btc.clone()], true, T0 + 60_000);
        lock(&engine).update(1, Vec::new(), true, T0 + 70_000);
        // The idle clock restarted at +70 s, so +10 min after the first drop is too soon.
        lock(&engine).expire_leases(T0 + 10_000 + 600_001);
        assert_eq!(last(&book, &btc), Some(100.0));
        lock(&engine).expire_leases(T0 + 70_000 + 600_001);
        assert!(book.get(&btc).is_none());
    }

    #[test]
    fn churning_through_many_markets_keeps_the_engine_bounded() {
        let (engine, book, provider) = setup();
        let grace = Config::default().idle_grace_ms;
        let mut now = T0;
        for batch in 0..20 {
            let markets: Vec<InstrumentId> = (0..50)
                .map(|index| id(DataSource::Binance, &format!("B{batch:02}M{index:02}USDT")))
                .collect();
            for market in &markets {
                provider.price(&market.symbol, 1.0);
            }
            lock(&engine).update(1, markets, true, now);
            round(&engine, &provider, now);
            now += grace / 2;
        }
        // 1,000 markets passed through; only those of the last grace period remain.
        let remembered = lock(&engine).remembered_markets();
        assert!(
            remembered < 4 * 150,
            "{remembered} entries retained for 1,000 markets"
        );
        lock(&engine).update(1, Vec::new(), true, now);
        lock(&engine).expire_leases(now + 2 * grace);
        assert_eq!(lock(&engine).remembered_markets(), 0);
        assert!(
            book.get(&id(DataSource::Binance, "B00M00USDT")).is_none(),
            "the book is pruned too"
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
            lock(&engine).stream_products(DataSource::Coinbase),
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
        lock(&engine).receive_ticks(vec![tick("btc-usd", 105.0, Some(100.0))], T0 + 7_000);
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
        lock(&engine).receive_ticks(vec![tick("DOGE-USD", 1.0, None)], T0);
        assert!(
            book.get(&InstrumentId::new(DataSource::Coinbase, "DOGE-USD"))
                .is_none()
        );
        lock(&engine).update(1, vec![btc], false, T0 + 1);
        assert!(
            lock(&engine)
                .stream_products(DataSource::Coinbase)
                .is_empty()
        );
    }

    #[test]
    fn a_rest_refresh_of_a_streamed_market_keeps_the_sockets_reference_for_a_while() {
        let (engine, book, provider) = setup();
        *provider.without_reference.lock().unwrap() = true;
        provider.price("BTC-USD", 110.0);
        let btc = id(DataSource::Coinbase, "BTC-USD");
        lock(&engine).update(1, vec![btc.clone()], true, T0);
        lock(&engine).receive_ticks(vec![tick("BTC-USD", 105.0, Some(100.0))], T0 + 7_000);

        // The socket goes quiet; REST brings a new price and no price a day ago: the change is
        // still measured against the reference the socket gave.
        round(&engine, &provider, T0 + 37_000);
        let quote = book.get(&btc).unwrap();
        assert_eq!(quote.last, Some(110.0));
        assert!((quote.change_percent.unwrap() - 10.0).abs() < 1e-9);

        // Ten minutes on with no tick, the reference is too old to say what a day ago was.
        lock(&engine).update(1, vec![btc.clone()], true, T0 + 700_000);
        round(&engine, &provider, T0 + 700_000);
        let quote = book.get(&btc).unwrap();
        assert_eq!(quote.last, Some(110.0));
        assert_eq!(quote.change_percent, None);
    }

    #[test]
    fn a_streamed_market_the_socket_never_answered_has_no_change_from_rest() {
        let (engine, book, provider) = setup();
        *provider.without_reference.lock().unwrap() = true;
        provider.price("ETH-USD", 50.0);
        let eth = id(DataSource::Coinbase, "ETH-USD");
        lock(&engine).update(1, vec![eth.clone()], true, T0);
        round(&engine, &provider, T0 + 6_000);
        let quote = book.get(&eth).unwrap();
        assert_eq!(quote.last, Some(50.0));
        assert_eq!(quote.change, None);
    }

    /// A tick of a venue whose socket sends trades: a price and nothing else.
    fn trade_tick(product_id: &str, price: f64) -> StreamTick {
        StreamTick {
            source: DataSource::Bitstamp,
            ..tick(product_id, price, None)
        }
    }

    #[test]
    fn a_venue_whose_ticks_are_trades_has_its_reference_from_rest_and_keeps_it_between_calls() {
        let (engine, book, provider) = setup();
        provider.price("BTCUSD", 110.0);
        let btc = id(DataSource::Bitstamp, "BTCUSD");
        lock(&engine).update(1, vec![btc.clone()], true, T0);

        // The first trade arrives before REST was asked anything: a price and no change yet.
        lock(&engine).receive_ticks(vec![trade_tick("BTCUSD", 105.0)], T0 + 1_000);
        let quote = book.get(&btc).unwrap();
        assert_eq!((quote.last, quote.change), (Some(105.0), None));

        // The socket is alive, and REST is still asked once for the price a day ago (the fake gives
        // half the price): the change is measured against it from then on, and REST's newer or
        // older price does not replace the trade's.
        round(&engine, &provider, T0 + 6_000);
        assert_eq!(provider.calls().len(), 1);
        let quote = book.get(&btc).unwrap();
        assert_eq!(quote.last, Some(105.0));
        assert!((quote.change_percent.unwrap() - (105.0 / 55.0 - 1.0) * 100.0).abs() < 1e-9);
        lock(&engine).receive_ticks(vec![trade_tick("BTCUSD", 66.0)], T0 + 7_000);
        let quote = book.get(&btc).unwrap();
        assert_eq!(quote.last, Some(66.0));
        assert!((quote.change_percent.unwrap() - 20.0).abs() < 1e-9);

        // No more calls while the reference is young and the socket is heard...
        for now in [T0 + 17_000, T0 + 40_000, T0 + 200_000] {
            lock(&engine).receive_ticks(vec![trade_tick("BTCUSD", 66.0)], now);
            round(&engine, &provider, now);
        }
        assert_eq!(provider.calls().len(), 1);

        // ...and one again once it is half as old as it may get, before it is too old to use.
        let half = Config::default().stream_reference_ms / 2;
        lock(&engine).receive_ticks(vec![trade_tick("BTCUSD", 70.0)], T0 + 6_000 + half);
        round(&engine, &provider, T0 + 6_000 + half);
        assert_eq!(provider.calls().len(), 2);
        assert!(book.get(&btc).unwrap().change_percent.is_some());
    }

    #[test]
    fn a_trade_venue_market_rest_has_no_reference_for_is_asked_again_only_after_a_while() {
        let (engine, book, provider) = setup();
        *provider.without_reference.lock().unwrap() = true;
        provider.price("STRKEUR", 3.0);
        let strk = id(DataSource::Bitstamp, "STRKEUR");
        lock(&engine).update(1, vec![strk.clone()], true, T0);
        lock(&engine).receive_ticks(vec![trade_tick("STRKEUR", 3.1)], T0 + 1_000);
        round(&engine, &provider, T0 + 6_000);
        assert_eq!(provider.calls().len(), 1);
        assert_eq!(book.get(&strk).unwrap().change, None);

        // Nothing was found, and that is noted: the next rounds do not ask again.
        for now in [T0 + 17_000, T0 + 40_000, T0 + 200_000] {
            lock(&engine).receive_ticks(vec![trade_tick("STRKEUR", 3.1)], now);
            round(&engine, &provider, now);
        }
        assert_eq!(provider.calls().len(), 1);
    }

    #[test]
    fn a_rest_call_that_leaves_out_a_ticking_market_neither_marks_it_stale_nor_repeats_every_round()
    {
        let (engine, book, provider) = setup();
        provider.price("ETHUSD", 1.0);
        let btc = id(DataSource::Bitstamp, "BTCUSD");
        let eth = id(DataSource::Bitstamp, "ETHUSD");
        lock(&engine).update(1, vec![btc.clone(), eth.clone()], true, T0);
        lock(&engine).receive_ticks(
            vec![trade_tick("BTCUSD", 105.0), trade_tick("ETHUSD", 2.0)],
            T0 + 1_000,
        );
        round(&engine, &provider, T0 + 6_000);
        assert_eq!(provider.calls().len(), 1);
        let quote = book.get(&btc).unwrap();
        assert_eq!(quote.last, Some(105.0));
        assert!(quote.freshness.is_current());
        assert_eq!(last(&book, &eth), Some(2.0));

        // The omission is noted: the next rounds do not ask again until the refresh has passed.
        let refresh = Config::default().stream_refresh_ms;
        for now in [T0 + 11_000, T0 + 21_000] {
            lock(&engine).receive_ticks(vec![trade_tick("BTCUSD", 106.0)], now);
            round(&engine, &provider, now);
        }
        assert_eq!(provider.calls().len(), 1);
        let now = T0 + 6_000 + refresh;
        lock(&engine).receive_ticks(vec![trade_tick("BTCUSD", 107.0)], now);
        round(&engine, &provider, now);
        assert_eq!(provider.calls().len(), 2);
        let quote = book.get(&btc).unwrap();
        assert_eq!(quote.last, Some(107.0));
        assert!(quote.freshness.is_current());
    }

    #[test]
    fn a_tick_keeps_the_last_volume_only_while_it_is_recent() {
        let (engine, book, _provider) = setup();
        let btc = id(DataSource::Coinbase, "BTC-USD");
        lock(&engine).update(1, vec![btc.clone()], true, T0);
        let mut with_volume = tick("BTC-USD", 105.0, Some(100.0));
        with_volume.quote.volume_24h = Some(5.0);
        lock(&engine).receive_ticks(vec![with_volume], T0 + 1_000);
        let age = Config::default().stream_reference_ms;
        lock(&engine).receive_ticks(vec![tick("BTC-USD", 106.0, Some(100.0))], T0 + age);
        assert_eq!(book.get(&btc).unwrap().volume, Some(5.0));
        lock(&engine).receive_ticks(vec![tick("BTC-USD", 107.0, Some(100.0))], T0 + age + 2_000);
        assert_eq!(book.get(&btc).unwrap().volume, None);
    }

    #[test]
    fn a_tick_that_gives_no_volume_keeps_the_volume_of_the_last_answer() {
        let (engine, book, _provider) = setup();
        let btc = id(DataSource::Coinbase, "BTC-USD");
        lock(&engine).update(1, vec![btc.clone()], true, T0);
        let mut with_volume = tick("BTC-USD", 105.0, Some(100.0));
        with_volume.quote.volume_24h = Some(5.0);
        with_volume.quote.volume_kind = Some(VolumeKind::Base);
        lock(&engine).receive_ticks(vec![with_volume], T0 + 1_000);
        lock(&engine).receive_ticks(vec![tick("BTC-USD", 106.0, Some(100.0))], T0 + 2_000);
        let quote = book.get(&btc).unwrap();
        assert_eq!(quote.last, Some(106.0));
        assert_eq!(
            (quote.volume, quote.volume_kind),
            (Some(5.0), Some(VolumeKind::Base))
        );
    }

    #[test]
    fn a_reference_is_forgotten_with_its_market() {
        let (engine, _book, _provider) = setup();
        let btc = id(DataSource::Coinbase, "BTC-USD");
        lock(&engine).update(1, vec![btc], true, T0);
        lock(&engine).receive_ticks(vec![tick("BTC-USD", 105.0, Some(100.0))], T0 + 1_000);
        assert!(lock(&engine).remembered_markets() > 0);

        // Nobody asks for it any more: marked idle, then forgotten once the grace has passed.
        let grace = Config::default().idle_grace_ms;
        lock(&engine).update(1, Vec::new(), true, T0 + 10_000);
        lock(&engine).expire_leases(T0 + 10_000 + grace + 1);
        assert_eq!(lock(&engine).remembered_markets(), 0);
    }

    #[test]
    fn a_case_sensitive_product_is_subscribed_and_matched_as_the_venue_names_it() {
        let (engine, book, provider) = setup();
        provider.price("xyz:SP500", 6_900.0);
        let index = id(DataSource::Hyperliquid, "xyz:SP500");
        let small = id(DataSource::Hyperliquid, "kPEPE");
        lock(&engine).update(1, vec![index.clone(), small.clone()], true, T0);
        assert_eq!(
            lock(&engine).stream_products(DataSource::Hyperliquid),
            vec!["kPEPE".to_owned(), "xyz:SP500".to_owned()]
        );
        let mut tick = tick("xyz:SP500", 6_950.0, Some(7_000.0));
        tick.source = DataSource::Hyperliquid;
        lock(&engine).receive_ticks(vec![tick], T0 + 1_000);
        assert_eq!(book.get(&index).unwrap().last, Some(6_950.0));
        assert!(book.get(&small).is_none());
    }

    #[test]
    fn each_streamed_source_has_its_own_products_and_takes_only_its_own_ticks() {
        let (engine, book, provider) = setup();
        provider.price("BTC-USDT", 100.0);
        let okx = id(DataSource::Okx, "btc-usdt");
        let coinbase = id(DataSource::Coinbase, "BTC-USD");
        lock(&engine).update(1, vec![okx.clone(), coinbase.clone()], true, T0);
        assert_eq!(
            lock(&engine).stream_products(DataSource::Okx),
            vec!["BTC-USDT".to_owned()]
        );
        assert_eq!(
            lock(&engine).stream_products(DataSource::Coinbase),
            vec!["BTC-USD".to_owned()]
        );

        // A Coinbase tick under an OKX product's name does not reach the OKX market.
        let mut stray = tick("BTC-USDT", 1.0, None);
        stray.source = DataSource::Coinbase;
        let mut own = tick("BTC-USDT", 105.0, Some(100.0));
        own.source = DataSource::Okx;
        lock(&engine).receive_ticks(vec![stray], T0 + 1_000);
        assert!(book.get(&okx).is_none());
        lock(&engine).receive_ticks(vec![own], T0 + 2_000);
        assert_eq!(book.get(&okx).unwrap().last, Some(105.0));
        assert!(book.get(&coinbase).is_none());

        // The socket owns the OKX price: REST waits, and refreshes it after 30 quiet seconds.
        round(&engine, &provider, T0 + 20_000);
        assert!(
            provider
                .calls()
                .iter()
                .all(|call| call.0 != DataSource::Okx)
        );
        round(&engine, &provider, T0 + 33_000);
        assert!(
            provider
                .calls()
                .iter()
                .any(|call| call.0 == DataSource::Okx)
        );
    }

    #[test]
    fn bare_coinbase_symbols_subscribe_under_their_product_id() {
        let (engine, _book, _provider) = setup();
        lock(&engine).update(1, vec![id(DataSource::Coinbase, "eth")], true, T0);
        assert_eq!(
            lock(&engine).stream_products(DataSource::Coinbase),
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
