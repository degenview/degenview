//! What the symbol picker asks the providers, and what it shows of the answers.
//!
//! One search asks every provider of the chosen scope at once, each on its provider's workers (so
//! rate-limited providers keep their lanes), and keeps each answer under `(query, provider)`.
//! Moving between scopes or venues therefore re-reads what was already fetched and asks only for
//! what is missing. The answers of the market tabs are merged by `service::market::search_rank`,
//! so "btc" lists every dollar market for BTC on every exchange before BTC priced in lira. A
//! prediction market has no pair to rank: its answers stay in provider order, each provider's own
//! order inside it.

use super::PickMode;
use crate::model::DataSource;
use crate::service::market::search_rank::{self, is_dollar_quote};
use crate::service::work_queue::{self, Pending};
use crate::service::{self, ProviderError, SearchResult};
use crate::ui::chart::picker_kit::suggestions::{CRYPTO_SUGGESTIONS, STOCK_SUGGESTIONS};
use eframe::egui;
use std::collections::HashMap;
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

/// The prediction markets the Predictions tab asks, in the order their answers are listed.
pub const PREDICTION_PROVIDERS: [DataSource; 2] = [DataSource::Polymarket, DataSource::Kalshi];

/// What the market tabs narrow the list to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope {
    /// Crypto and stocks together.
    All,
    Stocks,
    Crypto,
    /// Polymarket and Kalshi: events with their choices.
    Predictions,
    /// CoinMarketCap's market-wide indices and the portfolio views. Nothing is searched.
    Dashboards,
    /// The markets of the user's watchlists. Nothing is searched.
    Watchlists,
}

impl Scope {
    pub const TABS: [Self; 6] = [
        Self::All,
        Self::Stocks,
        Self::Crypto,
        Self::Predictions,
        Self::Dashboards,
        Self::Watchlists,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Stocks => "Stocks",
            Self::Crypto => "Crypto",
            Self::Predictions => "Predictions",
            Self::Dashboards => "Dashboards",
            Self::Watchlists => "Watchlists",
        }
    }

    pub fn includes(self, source: DataSource) -> bool {
        match self {
            Self::All | Self::Watchlists => PROVIDERS.contains(&source),
            Self::Stocks => source == DataSource::Alpaca,
            Self::Crypto => PROVIDERS.contains(&source) && source != DataSource::Alpaca,
            Self::Predictions => PREDICTION_PROVIDERS.contains(&source),
            Self::Dashboards => source == DataSource::CoinMarketCap,
        }
    }

    /// Whether the tab asks the providers; the others read lists the picker already holds.
    pub fn searches(self) -> bool {
        !matches!(self, Self::Watchlists | Self::Dashboards)
    }

    /// The tabs a picker opened for `mode` offers, in order. Compare draws a market over a chart,
    /// which only a crypto or stock market can be; a watchlist's own markets are already one tap
    /// away in the sidebar it is adding to.
    pub fn tabs(mode: PickMode) -> Vec<Self> {
        Self::TABS
            .into_iter()
            .filter(|tab| match mode {
                PickMode::Compare => !matches!(tab, Self::Predictions | Self::Dashboards),
                PickMode::Watchlist => *tab != Self::Watchlists,
                PickMode::Chart | PickMode::Add => true,
            })
            .collect()
    }

    /// The next tab of `mode`, for Tab.
    pub fn next(self, mode: PickMode) -> Self {
        let tabs = Self::tabs(mode);
        let at = tabs.iter().position(|tab| *tab == self).unwrap_or(0);
        tabs[(at + 1) % tabs.len()]
    }

    /// The venues the filter offers inside this scope.
    pub fn venues(self) -> Vec<DataSource> {
        PROVIDERS
            .into_iter()
            .chain(PREDICTION_PROVIDERS)
            .filter(|source| self.includes(*source))
            .collect()
    }

    /// The text the search box asks for.
    pub fn placeholder(self) -> &'static str {
        match self {
            Self::Predictions => "Search markets (e.g. Bitcoin, Fed, election)",
            Self::Dashboards => "Filter indices and portfolio views",
            Self::Watchlists => "Filter your watchlists",
            Self::All | Self::Stocks | Self::Crypto => "Search symbol…",
        }
    }
}

/// Markets offered before anything is typed and nothing has been picked yet: the largest crypto
/// dollar pairs and the biggest US stocks, from the lists in `picker_kit::suggestions`.
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
        DataSource::Polymarket | DataSource::Kalshi => "Prediction",
        DataSource::CoinMarketCap => "Index",
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

type Answer = Result<Vec<SearchResult>, ProviderError>;

/// What a search that ended without an answer (its worker panicked) shows for its provider.
const STOPPED: &str = "The search stopped unexpectedly.";

/// Where the answers come from: the providers, or canned ones under test.
#[derive(Clone, Copy)]
pub struct Backend {
    pub search: fn(DataSource, &str) -> Result<Vec<SearchResult>, ProviderError>,
    /// The other dollar markets of an asset.
    pub related: fn(&str) -> Vec<SearchResult>,
    /// The busiest prediction markets, for the Predictions tab while the box is empty.
    pub trending: fn(DataSource, usize) -> Result<Vec<SearchResult>, ProviderError>,
}

impl Backend {
    pub const LIVE: Self = Self {
        search: service::search,
        related: related_markets,
        trending: service::trending,
    };
}

/// The provider searches under way and finished.
pub struct SymbolSearch {
    answers: HashMap<(String, DataSource), Answer>,
    pending: HashMap<(String, DataSource), Pending<Answer>>,
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
    pub errors: Vec<(DataSource, ProviderError)>,
}

impl SymbolSearch {
    pub fn new(backend: Backend) -> Self {
        Self {
            answers: HashMap::new(),
            pending: HashMap::new(),
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
        let answers = &mut self.answers;
        self.pending.retain(|slot, search| {
            let Some(answer) = search.poll_or(|| Err(ProviderError::other(STOPPED))) else {
                return true;
            };
            answers.insert(slot.clone(), answer);
            false
        });
        let key = query.trim().to_lowercase();
        let sources: Vec<DataSource> = PROVIDERS
            .into_iter()
            .chain(PREDICTION_PROVIDERS)
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
                Some(Err(error)) => errors.push((*source, error.clone())),
                None => searching = true,
            }
        }
        let results = if scope == Scope::Predictions {
            groups.concat()
        } else {
            search_rank::merge_ranked(groups, query)
        };
        Listing {
            results,
            searching,
            errors,
        }
    }

    /// Asks each provider that has neither answered nor been asked this query, on the workers
    /// of that provider.
    fn start(&mut self, ctx: &egui::Context, key: &str, sources: &[DataSource]) {
        for source in sources {
            let slot = (key.to_owned(), *source);
            if self.answers.contains_key(&slot) || self.pending.contains_key(&slot) {
                continue;
            }
            let source = *source;
            let search = self.backend.search;
            let query = slot.0.clone();
            let job = work_queue::start_search(source, Some(ctx), move || search(source, &query));
            self.pending.insert(slot, job);
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

/// Related markets, fetched on a worker per asset and kept.
struct Related {
    lookup: fn(&str) -> Vec<SearchResult>,
    found: HashMap<String, Vec<SearchResult>>,
    asking: HashMap<String, Pending<Vec<SearchResult>>>,
}

impl Related {
    fn new(backend: Backend) -> Self {
        Self {
            lookup: backend.related,
            found: HashMap::new(),
            asking: HashMap::new(),
        }
    }

    fn get(&mut self, ctx: &egui::Context, of: &SearchResult) -> Option<Vec<SearchResult>> {
        let found = &mut self.found;
        self.asking.retain(|asset, lookup| {
            // A lookup that ended without an answer found nothing.
            let Some(answer) = lookup.poll_or(Vec::new) else {
                return true;
            };
            found.insert(asset.clone(), answer);
            false
        });
        if !PROVIDERS.contains(&of.source) || of.source == DataSource::Alpaca {
            return Some(Vec::new());
        }
        let asset = of.base_asset();
        if let Some(found) = self.found.get(&asset) {
            return Some(found.iter().filter(|hit| !same(hit, of)).cloned().collect());
        }
        if !self.asking.contains_key(&asset) {
            let lookup = self.lookup;
            let of = asset.clone();
            let job = work_queue::start(Some(ctx), move || lookup(&of));
            self.asking.insert(asset, job);
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
    fn the_scopes_cover_their_own_markets() {
        assert!(Scope::All.includes(DataSource::Binance));
        assert!(Scope::All.includes(DataSource::Alpaca));
        assert!(Scope::Crypto.includes(DataSource::DexScreener));
        assert!(!Scope::Crypto.includes(DataSource::Alpaca));
        assert!(Scope::Stocks.includes(DataSource::Alpaca));
        assert!(!Scope::Stocks.includes(DataSource::Coinbase));
        assert!(Scope::Predictions.includes(DataSource::Polymarket));
        assert!(Scope::Predictions.includes(DataSource::Kalshi));
        assert!(!Scope::Predictions.includes(DataSource::Binance));
        assert!(Scope::Dashboards.includes(DataSource::CoinMarketCap));
        assert!(Scope::Watchlists.includes(DataSource::Coinbase));
        assert!(!Scope::Watchlists.searches());
        assert!(!Scope::Dashboards.searches());
        assert!(Scope::Predictions.searches());
        // Prediction markets and indices never leak into the tabs that list spot markets.
        for scope in [Scope::All, Scope::Stocks, Scope::Crypto, Scope::Watchlists] {
            assert!(!scope.includes(DataSource::Polymarket));
            assert!(!scope.includes(DataSource::Kalshi));
            assert!(!scope.includes(DataSource::CoinMarketCap));
        }
    }

    #[test]
    fn a_mode_offers_only_the_tabs_it_can_use() {
        assert_eq!(Scope::tabs(PickMode::Chart), Scope::TABS);
        assert_eq!(Scope::tabs(PickMode::Add), Scope::TABS);
        // A market compared over a chart is a crypto or stock market.
        assert_eq!(
            Scope::tabs(PickMode::Compare),
            [Scope::All, Scope::Stocks, Scope::Crypto, Scope::Watchlists]
        );
        // A watchlist's own markets are already in the sidebar being added to.
        assert!(!Scope::tabs(PickMode::Watchlist).contains(&Scope::Watchlists));
        assert_eq!(Scope::Crypto.next(PickMode::Compare), Scope::Watchlists);
        assert_eq!(Scope::Dashboards.next(PickMode::Watchlist), Scope::All);
    }

    #[test]
    fn tab_cycles_through_every_scope() {
        let mut scope = Scope::All;
        let mut seen = vec![scope];
        for _ in 0..Scope::TABS.len() {
            scope = scope.next(PickMode::Chart);
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
        assert_eq!(Scope::Watchlists.venues().len(), PROVIDERS.len());
        assert_eq!(Scope::Predictions.venues(), PREDICTION_PROVIDERS);
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
            Err(ProviderError::other("Add your Alpaca API key in Settings")),
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
