//! What the symbol picker asks the providers, and what it shows of the answers.
//!
//! One search asks every provider of the chosen scope at once, each on its own thread, and keeps
//! each answer under `(query, provider)`. Moving between scopes or venues therefore re-reads what
//! was already fetched and asks only for what is missing. The answers are merged by
//! `service::search_rank`, so "btc" lists every dollar market for BTC on every exchange before
//! BTC priced in lira.

use crate::model::DataSource;
use crate::service::search_rank::{self, is_dollar_quote};
use crate::service::{self, SearchResult};
use crate::ui::add_chart_sheet::{CRYPTO_SUGGESTIONS, STOCK_SUGGESTIONS};
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

/// How long the box waits after a keystroke before it asks.
const DEBOUNCE: Duration = Duration::from_millis(220);
/// Related markets listed under a market.
const RELATED_LIMIT: usize = 6;

/// The markets the picker lists, in the order the providers are asked (and so the order exchanges
/// that list the same market appear in).
pub const PROVIDERS: [DataSource; 5] = [
    DataSource::Binance,
    DataSource::Coinbase,
    DataSource::DexScreener,
    DataSource::CoinGecko,
    DataSource::Alpaca,
];

/// What the market tabs narrow the list to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope {
    /// Crypto and stocks together.
    All,
    Stocks,
    Crypto,
}

impl Scope {
    pub const TABS: [Self; 3] = [Self::All, Self::Stocks, Self::Crypto];

    pub fn title(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Stocks => "Stocks",
            Self::Crypto => "Crypto",
        }
    }

    pub fn includes(self, source: DataSource) -> bool {
        match self {
            Self::All => PROVIDERS.contains(&source),
            Self::Stocks => source == DataSource::Alpaca,
            Self::Crypto => PROVIDERS.contains(&source) && source != DataSource::Alpaca,
        }
    }

    /// The next tab, for Tab.
    pub fn next(self) -> Self {
        let at = Self::TABS.iter().position(|tab| *tab == self).unwrap_or(0);
        Self::TABS[(at + 1) % Self::TABS.len()]
    }

    /// The venues the filter offers inside this scope.
    pub fn venues(self) -> Vec<DataSource> {
        PROVIDERS
            .into_iter()
            .filter(|source| self.includes(*source))
            .collect()
    }
}

/// Markets offered before anything is typed and nothing has been picked yet: the largest crypto
/// dollar pairs and the biggest US stocks, from the same lists Add Chart suggests.
pub fn popular() -> Vec<SearchResult> {
    let crypto = CRYPTO_SUGGESTIONS.iter().map(|base| {
        let symbol = format!("{base}USDT");
        SearchResult::new(DataSource::Binance, symbol.clone(), symbol).pair(*base, "USDT")
    });
    let stocks = STOCK_SUGGESTIONS
        .iter()
        .map(|ticker| SearchResult::new(DataSource::Alpaca, *ticker, *ticker));
    crypto.chain(stocks).collect()
}

/// What kind of market a provider lists, as the list's right-hand column says it.
pub fn market_kind(source: DataSource) -> &'static str {
    match source {
        DataSource::Alpaca => "Stock",
        _ => "Crypto",
    }
}

/// How a market trades, for the "also trades as" column.
pub fn trading_kind(source: DataSource) -> &'static str {
    match source {
        DataSource::Binance | DataSource::Coinbase => "Spot",
        DataSource::DexScreener => "DEX",
        DataSource::CoinGecko => "Aggregate",
        DataSource::Alpaca => "Stock",
        _ => "",
    }
}

type Answer = (String, DataSource, Result<Vec<SearchResult>, String>);

/// Where the answers come from: the providers, or canned ones under test.
#[derive(Clone, Copy)]
pub struct Backend {
    pub search: fn(DataSource, &str) -> Result<Vec<SearchResult>, String>,
    /// The other dollar markets of an asset.
    pub related: fn(&str) -> Vec<SearchResult>,
}

impl Backend {
    pub const LIVE: Self = Self {
        search: service::search,
        related: related_markets,
    };
}

/// The provider searches under way and finished.
pub struct SymbolSearch {
    answers: HashMap<(String, DataSource), Result<Vec<SearchResult>, String>>,
    pending: HashSet<(String, DataSource)>,
    sender: Sender<Answer>,
    inbox: Receiver<Answer>,
    /// When the box last changed; the search starts once it has been still for `DEBOUNCE`.
    edited: Option<Instant>,
    related: Related,
    backend: Backend,
}

impl Default for SymbolSearch {
    fn default() -> Self {
        Self::new(Backend::LIVE)
    }
}

/// What the list shows for one query.
pub struct Listing {
    /// Best first, narrowed to the scope and venue.
    pub results: Vec<SearchResult>,
    /// A provider is still answering.
    pub searching: bool,
    /// Why a provider gave nothing, by provider.
    pub errors: Vec<(DataSource, String)>,
}

impl SymbolSearch {
    pub fn new(backend: Backend) -> Self {
        let (sender, inbox) = mpsc::channel();
        Self {
            answers: HashMap::new(),
            pending: HashSet::new(),
            sender,
            inbox,
            edited: None,
            related: Related::new(backend),
            backend,
        }
    }

    /// The next search starts without waiting (a pick of a scope, the first open).
    pub fn ask_now(&mut self) {
        self.edited = Some(Instant::now() - DEBOUNCE);
    }

    /// The box changed.
    pub fn edited(&mut self) {
        self.edited = Some(Instant::now());
    }

    /// Reads what arrived, starts the searches that are due and returns what to list.
    pub fn listing(
        &mut self,
        ctx: &egui::Context,
        query: &str,
        scope: Scope,
        venue: Option<DataSource>,
    ) -> Listing {
        while let Ok((query, source, answer)) = self.inbox.try_recv() {
            self.pending.remove(&(query.clone(), source));
            self.answers.insert((query, source), answer);
        }
        let key = query.trim().to_lowercase();
        let sources: Vec<DataSource> = PROVIDERS
            .into_iter()
            .filter(|source| scope.includes(*source) && venue.is_none_or(|only| only == *source))
            .collect();
        if key.is_empty() {
            return Listing {
                results: Vec::new(),
                searching: false,
                errors: Vec::new(),
            };
        }
        if let Some(edited) = self.edited {
            let wait = DEBOUNCE.saturating_sub(edited.elapsed());
            if wait.is_zero() {
                self.edited = None;
                self.start(ctx, &key, &sources);
            } else {
                ctx.request_repaint_after(wait);
            }
        }
        let mut groups = Vec::new();
        let mut errors = Vec::new();
        let mut searching = self.edited.is_some();
        for source in &sources {
            let slot = (key.clone(), *source);
            match self.answers.get(&slot) {
                Some(Ok(results)) => groups.push(results.clone()),
                Some(Err(message)) => errors.push((*source, message.clone())),
                None => searching = true,
            }
        }
        let results = search_rank::merge_ranked(groups, query);
        Listing {
            results,
            searching,
            errors,
        }
    }

    /// Asks each provider that has neither answered nor been asked this query.
    fn start(&mut self, ctx: &egui::Context, key: &str, sources: &[DataSource]) {
        for source in sources {
            let slot = (key.to_owned(), *source);
            if self.answers.contains_key(&slot) || !self.pending.insert(slot.clone()) {
                continue;
            }
            let sender = self.sender.clone();
            let repaint = ctx.clone();
            let source = *source;
            let search = self.backend.search;
            std::thread::spawn(move || {
                let answer = search(source, &slot.0);
                let _ = sender.send((slot.0, source, answer));
                repaint.request_repaint();
            });
        }
    }

    /// The other markets that trade `of`'s asset in dollars, from the exchanges whose lists are
    /// held locally. `None` while they are being read; asks once per asset.
    pub fn related(&mut self, ctx: &egui::Context, of: &SearchResult) -> Option<Vec<SearchResult>> {
        self.related.get(ctx, of)
    }

    pub fn forget(&mut self) {
        *self = Self::new(self.backend);
    }
}

/// An asset and the other dollar markets found for it.
type RelatedAnswer = (String, Vec<SearchResult>);

/// Related markets, fetched on a thread per asset and kept.
struct Related {
    lookup: fn(&str) -> Vec<SearchResult>,
    found: HashMap<String, Vec<SearchResult>>,
    asking: HashSet<String>,
    channel: Option<(Sender<RelatedAnswer>, Receiver<RelatedAnswer>)>,
}

impl Related {
    fn new(backend: Backend) -> Self {
        Self {
            lookup: backend.related,
            found: HashMap::new(),
            asking: HashSet::new(),
            channel: None,
        }
    }

    fn get(&mut self, ctx: &egui::Context, of: &SearchResult) -> Option<Vec<SearchResult>> {
        let (_, inbox) = self.channel.get_or_insert_with(mpsc::channel);
        while let Ok((asset, found)) = inbox.try_recv() {
            self.asking.remove(&asset);
            self.found.insert(asset, found);
        }
        if of.source == DataSource::Alpaca {
            return Some(Vec::new());
        }
        let asset = of.base_asset();
        if let Some(found) = self.found.get(&asset) {
            return Some(found.iter().filter(|hit| !same(hit, of)).cloned().collect());
        }
        if self.asking.insert(asset.clone()) {
            let sender = self.channel.as_ref().map(|(sender, _)| sender.clone())?;
            let repaint = ctx.clone();
            let lookup = self.lookup;
            std::thread::spawn(move || {
                let found = lookup(&asset);
                let _ = sender.send((asset, found));
                repaint.request_repaint();
            });
        }
        None
    }
}

fn same(a: &SearchResult, b: &SearchResult) -> bool {
    a.source == b.source && a.full_symbol == b.full_symbol
}

/// Every dollar market of `asset` on the exchanges whose lists are held locally.
fn related_markets(asset: &str) -> Vec<SearchResult> {
    let groups: Vec<Vec<SearchResult>> = [DataSource::Binance, DataSource::Coinbase]
        .into_iter()
        .map(|source| {
            service::search(source, asset)
                .unwrap_or_default()
                .into_iter()
                .filter(|hit| {
                    hit.base_asset().eq_ignore_ascii_case(asset)
                        && hit.quote.as_deref().is_some_and(is_dollar_quote)
                })
                .collect()
        })
        .collect();
    search_rank::merge_ranked(groups, asset)
        .into_iter()
        .take(RELATED_LIMIT + 1)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scopes_cover_crypto_and_stocks() {
        assert!(Scope::All.includes(DataSource::Binance));
        assert!(Scope::All.includes(DataSource::Alpaca));
        assert!(Scope::Crypto.includes(DataSource::DexScreener));
        assert!(!Scope::Crypto.includes(DataSource::Alpaca));
        assert!(Scope::Stocks.includes(DataSource::Alpaca));
        assert!(!Scope::Stocks.includes(DataSource::Coinbase));
        // Prediction markets and CoinMarketCap gauges are added from Add Chart, not searched here.
        for scope in Scope::TABS {
            assert!(!scope.includes(DataSource::Polymarket));
            assert!(!scope.includes(DataSource::CoinMarketCap));
        }
    }

    #[test]
    fn tab_cycles_through_every_scope() {
        let mut scope = Scope::All;
        let mut seen = vec![scope];
        for _ in 0..Scope::TABS.len() {
            scope = scope.next();
            seen.push(scope);
        }
        assert_eq!(seen.first(), seen.last());
        assert_eq!(seen.len(), Scope::TABS.len() + 1);
    }

    #[test]
    fn a_scope_offers_only_its_own_venues() {
        assert_eq!(Scope::Stocks.venues(), [DataSource::Alpaca]);
        assert!(!Scope::Crypto.venues().contains(&DataSource::Alpaca));
        assert_eq!(Scope::All.venues().len(), PROVIDERS.len());
    }

    #[test]
    fn answers_are_listed_best_first_and_kept_per_scope() {
        let ctx = egui::Context::default();
        let mut search = SymbolSearch::default();
        let binance = vec![
            SearchResult::new(DataSource::Binance, "BTCTRY", "BTCTRY").pair("BTC", "TRY"),
            SearchResult::new(DataSource::Binance, "BTCUSDT", "BTCUSDT").pair("BTC", "USDT"),
        ];
        let share = vec![SearchResult::new(
            DataSource::Alpaca,
            "BTC — Grayscale Bitcoin Mini Trust",
            "BTC",
        )];
        for (source, results) in [(DataSource::Binance, binance), (DataSource::Alpaca, share)] {
            search.answers.insert(("btc".into(), source), Ok(results));
        }
        for source in [
            DataSource::Coinbase,
            DataSource::DexScreener,
            DataSource::CoinGecko,
        ] {
            search
                .answers
                .insert(("btc".into(), source), Ok(Vec::new()));
        }
        let all = search.listing(&ctx, "BTC", Scope::All, None);
        let names: Vec<&str> = all.results.iter().map(|r| r.symbol.as_str()).collect();
        assert_eq!(
            names,
            ["BTCUSDT", "BTC — Grayscale Bitcoin Mini Trust", "BTCTRY"]
        );
        assert!(!all.searching);

        let stocks = search.listing(&ctx, "btc", Scope::Stocks, None);
        assert_eq!(stocks.results.len(), 1);
        let venue = search.listing(&ctx, "btc", Scope::All, Some(DataSource::Binance));
        assert_eq!(venue.results.len(), 2);
    }

    #[test]
    fn a_provider_that_has_not_answered_keeps_the_list_searching() {
        let ctx = egui::Context::default();
        let mut search = SymbolSearch::default();
        search
            .answers
            .insert(("eth".into(), DataSource::Binance), Ok(Vec::new()));
        let listing = search.listing(&ctx, "eth", Scope::Crypto, Some(DataSource::Binance));
        assert!(!listing.searching);
        let listing = search.listing(&ctx, "eth", Scope::Crypto, Some(DataSource::Coinbase));
        assert!(listing.searching);
    }

    #[test]
    fn a_failing_provider_is_reported_while_the_others_answer() {
        let ctx = egui::Context::default();
        let mut search = SymbolSearch::default();
        search.answers.insert(
            ("aapl".into(), DataSource::Alpaca),
            Err("Add your Alpaca API key in Settings".into()),
        );
        let listing = search.listing(&ctx, "aapl", Scope::Stocks, None);
        assert_eq!(listing.errors.len(), 1);
        assert!(listing.results.is_empty());
    }

    #[test]
    fn related_markets_leave_out_the_one_asked_about_and_other_assets() {
        let mut related = Related::new(Backend::LIVE);
        let ctx = egui::Context::default();
        let btc = SearchResult::new(DataSource::Binance, "BTCUSDT", "BTCUSDT").pair("BTC", "USDT");
        related.found.insert(
            "BTC".into(),
            vec![
                btc.clone(),
                SearchResult::new(DataSource::Coinbase, "BTC-USD", "BTC-USD").pair("BTC", "USD"),
            ],
        );
        let found = related.get(&ctx, &btc).expect("held");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].symbol, "BTC-USD");
    }
}
