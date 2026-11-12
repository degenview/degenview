//! The Add Chart sheet (Swift `AddTickerSheet`): a fixed-size sheet with a header, a section
//! switcher (Crypto, Stocks, Predictions, CoinMarketCap), a search pane per section with
//! suggestion chips and recent markets while the box is empty, and a footer that shows what is
//! selected beside Cancel and Add.

use crate::model::DataSource;
use crate::model::PmSeries;
use crate::model::portfolio::Portfolio;
use crate::model::portfolio::card::{CardKind, PortfolioCard};
use crate::service::{self, EventInfo, SearchResult};
use crate::store::SharedStore;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::loading::LoadingIndicator;
use crate::ui::{
    chart::add::art,
    kit::{loading, source_logo},
    theme,
};
use eframe::egui::{
    self, Color32, CornerRadius, FontId, Galley, Id, Rect, RichText, Sense, Stroke, StrokeKind, Ui,
    Vec2, pos2, text::LayoutJob, vec2,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// Fixed rather than content-hugging, so switching sections never changes the sheet's size.
pub(crate) const SHEET_SIZE: Vec2 = vec2(900.0, 620.0);
pub(crate) const PADDING: f32 = 24.0;
/// The room the trending list's loading state takes.
const TRENDING_LOADING_HEIGHT: f32 = 120.0;
const SEARCH_FIELD_HEIGHT: f32 = 36.0;
const RESULT_ROW_HEIGHT: f32 = 38.0;
const RESULT_COUNT_HEIGHT: f32 = 18.0;
const RESULT_COUNT_MIN_WIDTH: f32 = 30.0;
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(300);
const RECENT_LIMIT: usize = 10;
/// Events listed under "Trending", and how long that list is reused (as in the Swift app).
const TRENDING_LIMIT: usize = 8;
const TRENDING_TTL: Duration = Duration::from_secs(600);
/// How long a failed trending fetch waits before the idle pane asks again.
const TRENDING_RETRY: Duration = Duration::from_secs(30);
const RECENT_KEY: &str = "addTicker.recentMarkets";

const CRYPTO_SOURCES: [DataSource; 4] = [
    DataSource::Binance,
    DataSource::Coinbase,
    DataSource::DexScreener,
    DataSource::CoinGecko,
];
const STOCK_SOURCES: [DataSource; 1] = [DataSource::Alpaca];
pub(crate) const PREDICTION_PROVIDERS: [DataSource; 2] =
    [DataSource::Polymarket, DataSource::Kalshi];

pub(crate) const CRYPTO_SUGGESTIONS: [&str; 10] = [
    "BTC", "ETH", "SOL", "BNB", "XRP", "ADA", "DOGE", "AVAX", "DOT", "LINK",
];
pub(crate) const STOCK_SUGGESTIONS: [&str; 10] = [
    "AAPL", "MSFT", "NVDA", "AMZN", "GOOGL", "META", "TSLA", "SPY", "QQQ", "AMD",
];
/// Starting points for the Predictions section; each searches the selected provider for its query.
pub(crate) const PREDICTION_TOPICS: [(&str, &str, Icon); 10] = [
    ("Fed decision", "fed", Icon::BuildingColumns),
    ("Bitcoin price", "bitcoin", Icon::BitcoinCircle),
    ("Ethereum price", "ethereum", Icon::Diamond),
    ("Inflation", "inflation", Icon::ChartUptrend),
    ("Recession", "recession", Icon::ArrowDownRightCircle),
    ("Election", "election", Icon::CheckmarkSeal),
    ("Trump", "trump", Icon::PersonCircle),
    ("Oil & gold", "oil", Icon::Flame),
    ("Super Bowl", "super bowl", Icon::SportsCourt),
    ("Weather", "temperature", Icon::CloudSun),
];
/// Icon and description per entry of `service::market::cmc::CHART_TYPES`, in the same order.
const CMC_DETAILS: [(Icon, &str); 4] = [
    (Icon::ChartLine, "Historical 0–100 index chart"),
    (Icon::GaugeHigh, "Current Bitcoin/Altcoin season reading"),
    (Icon::Waveform, "Historical market-sentiment chart"),
    (Icon::Gauge, "Current sentiment gauge"),
];

// MARK: Type

/// SF Pro weights on its `wght` axis.
const MEDIUM: f32 = 510.0;
const SEMIBOLD: f32 = 590.0;
const BOLD: f32 = 700.0;

/// The macOS text styles the Swift sheet uses, in points.
mod size {
    pub const TITLE2: f32 = 17.0;
    pub const BODY: f32 = 13.0;
    pub const CALLOUT: f32 = 12.0;
    pub const SUBHEADLINE: f32 = 11.0;
    pub const CAPTION: f32 = 10.0;
}

fn text(value: impl Into<String>, size: f32, weight: f32, color: Color32) -> RichText {
    RichText::new(value)
        .size(size)
        .variation("wght", weight)
        .color(color)
}

/// A one-line galley at `weight`, cut with an ellipsis past `max_width`.
fn galley(
    ui: &Ui,
    value: &str,
    size: f32,
    weight: f32,
    color: Color32,
    max_width: f32,
) -> Arc<Galley> {
    galley_rows(ui, value, size, weight, color, max_width, 1)
}

/// Like [`galley`], wrapping onto up to `max_rows` lines before the ellipsis.
fn galley_rows(
    ui: &Ui,
    value: &str,
    size: f32,
    weight: f32,
    color: Color32,
    max_width: f32,
    max_rows: usize,
) -> Arc<Galley> {
    let mut job = LayoutJob::default();
    job.append(
        value,
        0.0,
        egui::TextFormat {
            font_id: FontId::proportional(size),
            color,
            coords: egui::epaint::text::VariationCoords::new([("wght", weight)]),
            ..Default::default()
        },
    );
    job.wrap.max_width = max_width.max(1.0);
    job.wrap.max_rows = max_rows;
    job.wrap.overflow_character = Some('…');
    ui.painter().layout_job(job)
}

// MARK: Sections

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Tab {
    Crypto,
    Stocks,
    Predictions,
    CoinMarketCap,
    Portfolio,
}

impl Tab {
    const ALL: [Self; 5] = [
        Self::Crypto,
        Self::Stocks,
        Self::Predictions,
        Self::CoinMarketCap,
        Self::Portfolio,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::Crypto => "Crypto",
            Self::Stocks => "Stocks",
            Self::Predictions => "Predictions",
            Self::CoinMarketCap => "CoinMarketCap",
            Self::Portfolio => "Portfolio",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Crypto => Icon::BitcoinCircle,
            Self::Stocks => Icon::BuildingColumns,
            Self::Predictions => Icon::Percent,
            Self::CoinMarketCap => Icon::Gauge,
            Self::Portfolio => Icon::Briefcase,
        }
    }

    fn placeholder(self) -> &'static str {
        match self {
            Self::Crypto => "Ticker symbol (e.g. BTC or PEPE)",
            Self::Stocks => "US stock symbol or company name (e.g. AAPL)",
            Self::Predictions | Self::CoinMarketCap | Self::Portfolio => {
                "Search markets (e.g. Bitcoin, Fed, election)"
            }
        }
    }
}

// MARK: Search

type SearchInbox = Receiver<(String, Vec<(DataSource, Result<Vec<SearchResult>, String>)>)>;

/// Debounced search state for one section of the sheet.
struct SearchSession {
    query: String,
    inbox: Option<SearchInbox>,
    results: Vec<(DataSource, Vec<SearchResult>)>,
    /// `results` grouped by prediction-market event, in the order the Predictions section shows.
    events: EventList,
    selected: Option<SearchResult>,
    debounce: Option<Instant>,
    /// The query the current results answer.
    answered: String,
    error: Option<String>,
}

impl SearchSession {
    fn new() -> Self {
        Self {
            query: String::new(),
            inbox: None,
            results: Vec::new(),
            events: EventList::default(),
            selected: None,
            debounce: None,
            answered: String::new(),
            error: None,
        }
    }

    fn trimmed(&self) -> &str {
        self.query.trim()
    }

    fn is_idle(&self) -> bool {
        self.trimmed().is_empty()
    }

    fn is_searching(&self) -> bool {
        !self.is_idle() && (self.inbox.is_some() || self.debounce.is_some())
    }

    fn has_results(&self) -> bool {
        self.results.iter().any(|(_, list)| !list.is_empty())
    }

    fn first_result(&self) -> Option<&SearchResult> {
        self.results.iter().flat_map(|(_, list)| list).next()
    }

    /// The result Return picks. Events list their likeliest choice first, so the Predictions
    /// section's first row is not necessarily the provider's first hit.
    fn first_visible(&self, grouped: bool) -> Option<&SearchResult> {
        if grouped {
            self.events.first()
        } else {
            self.first_result()
        }
    }

    fn set_results(&mut self, results: Vec<(DataSource, Vec<SearchResult>)>) {
        self.events
            .set(results.iter().flat_map(|(_, list)| list.iter()));
        self.results = results;
    }

    /// Fills the box and searches right away, as picking a suggestion does.
    fn search_now(&mut self, query: &str) {
        self.query = query.to_owned();
        self.debounce = Some(Instant::now() - SEARCH_DEBOUNCE);
    }

    fn edited(&mut self) {
        self.error = None;
        if self.is_idle() {
            self.set_results(Vec::new());
            self.answered.clear();
            self.debounce = None;
        } else {
            self.debounce = Some(Instant::now());
        }
    }

    fn clear(&mut self) {
        self.query.clear();
        self.edited();
        self.selected = None;
    }

    /// Starts the debounced search once it is due; results land through `receive`.
    fn tick(&mut self, sources: &[DataSource], ctx: &egui::Context) {
        self.receive();
        let Some(debounce) = self.debounce else {
            return;
        };
        if self.inbox.is_some() || debounce.elapsed() < SEARCH_DEBOUNCE {
            ctx.request_repaint_after(SEARCH_DEBOUNCE);
            return;
        }
        self.debounce = None;
        let query = self.trimmed().to_owned();
        if query.is_empty() || query == self.answered {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        let sources = sources.to_vec();
        std::thread::spawn(move || {
            let results = sources
                .iter()
                .map(|source| (*source, service::search(*source, &query)))
                .collect();
            let _ = sender.send((query, results));
            repaint.request_repaint();
        });
        self.inbox = Some(receiver);
    }

    fn receive(&mut self) {
        let Some(receiver) = &self.inbox else {
            return;
        };
        let Ok((query, results)) = receiver.try_recv() else {
            return;
        };
        self.inbox = None;
        // A reply to a query the box no longer holds is dropped; the next tick asks again.
        if query != self.trimmed() {
            if self.debounce.is_none() {
                self.debounce = Some(Instant::now() - SEARCH_DEBOUNCE);
            }
            return;
        }
        let mut error = None;
        let mut collected = Vec::new();
        for (source, result) in results {
            match result {
                Ok(list) if !list.is_empty() => collected.push((source, list)),
                Ok(_) => {}
                Err(message) => {
                    error.get_or_insert(message);
                }
            }
        }
        self.answered = query;
        self.set_results(collected);
        // A selection the new results no longer contain would commit something not on screen.
        if let Some(selected) = &self.selected
            && !self
                .results
                .iter()
                .flat_map(|(_, list)| list)
                .any(|result| same_market(result, selected))
        {
            self.selected = None;
        }
        self.error = if self.has_results() { None } else { error };
    }

    /// Drops the results so the next tick searches again (after a provider switch).
    fn reset_results(&mut self) {
        self.set_results(Vec::new());
        self.answered.clear();
        self.selected = None;
        self.error = None;
        self.inbox = None;
        if !self.is_idle() {
            self.debounce = Some(Instant::now() - SEARCH_DEBOUNCE);
        }
    }
}

pub(crate) fn same_market(a: &SearchResult, b: &SearchResult) -> bool {
    a.source == b.source && a.full_symbol == b.full_symbol
}

// MARK: Prediction events

/// The choices of one prediction-market event, listed under the event's title.
struct EventGroup {
    title: String,
    /// Likeliest choice first.
    results: Vec<SearchResult>,
}

/// Prediction-market results grouped by event, with which events show their choices.
#[derive(Default)]
struct EventList {
    groups: Vec<EventGroup>,
    /// Titles of the events whose choices are showing.
    expanded: HashSet<String>,
}

impl EventList {
    /// Regroups `results`. A new set of markets starts collapsed, unless it is a single event;
    /// the same markets with fresh prices keep what the user opened.
    fn set<'a>(&mut self, results: impl Iterator<Item = &'a SearchResult>) {
        let groups = group_by_event(results);
        if !self.has_same_markets(&groups) {
            self.expanded = match groups.as_slice() {
                [only] => HashSet::from([only.title.clone()]),
                _ => HashSet::new(),
            };
        }
        self.groups = groups;
    }

    fn has_same_markets(&self, groups: &[EventGroup]) -> bool {
        let markets = |groups: &[EventGroup]| -> Vec<(String, String)> {
            groups
                .iter()
                .flat_map(|group| {
                    group
                        .results
                        .iter()
                        .map(|result| (group.title.clone(), result.full_symbol.clone()))
                })
                .collect()
        };
        markets(&self.groups) == markets(groups)
    }

    fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    fn first(&self) -> Option<&SearchResult> {
        self.groups.first()?.results.first()
    }
}

/// Buckets results by event without reordering events, then stably sorts each event's choices by
/// descending YES probability, choices without one last. A result with no event title stands
/// alone under its own label.
fn group_by_event<'a>(results: impl Iterator<Item = &'a SearchResult>) -> Vec<EventGroup> {
    let mut groups: Vec<EventGroup> = Vec::new();
    for result in results {
        let title = result
            .event
            .as_ref()
            .map(|event| event.title.as_str())
            .filter(|title| !title.is_empty())
            .unwrap_or(&result.symbol);
        match groups.iter_mut().find(|group| group.title == title) {
            Some(group) => group.results.push(result.clone()),
            None => groups.push(EventGroup {
                title: title.to_owned(),
                results: vec![result.clone()],
            }),
        }
    }
    for group in &mut groups {
        group.results.sort_by(|a, b| match (a.price, b.price) {
            (Some(a), Some(b)) => b.total_cmp(&a),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
    }
    groups
}

// MARK: Recents

/// A market picked before, kept so it can be picked again without searching. The price is
/// dropped, since it would be stale.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentMarket {
    pub symbol: String,
    pub full_symbol: String,
    pub source: DataSource,
}

/// The markets picked before, newest first (shared with an empty tab's shortcuts).
pub fn load_recents(store: &crate::store::Store) -> Vec<RecentMarket> {
    store
        .setting::<Vec<RecentMarket>>(RECENT_KEY)
        .unwrap_or_default()
}

pub fn store_recents(store: &crate::store::Store, recents: &[RecentMarket]) {
    let _ = if recents.is_empty() {
        store.remove_setting(RECENT_KEY)
    } else {
        store.set_setting(RECENT_KEY, &recents)
    };
}

/// Puts `result` first in `recents`, the one place both sheets record a pick. Prediction markets
/// and CoinMarketCap indices are not kept; returns whether the list changed.
pub(crate) fn record_recent(recents: &mut Vec<RecentMarket>, result: &SearchResult) -> bool {
    if result.source.is_prediction_market() || result.source == DataSource::CoinMarketCap {
        return false;
    }
    recents.retain(|recent| {
        recent.source != result.source || recent.full_symbol != result.full_symbol
    });
    recents.insert(
        0,
        RecentMarket {
            symbol: result.symbol.clone(),
            full_symbol: result.full_symbol.clone(),
            source: result.source,
        },
    );
    recents.truncate(RECENT_LIMIT);
    true
}

impl RecentMarket {
    pub fn result(&self) -> SearchResult {
        SearchResult {
            symbol: self.symbol.clone(),
            full_symbol: self.full_symbol.clone(),
            source: self.source,
            price: None,
            event: None,
            base: None,
            quote: None,
        }
    }
}

// MARK: Sheet

/// What the sheet asks its host to do.
#[derive(Clone, Debug)]
pub enum AddChartAction {
    /// Add (or focus) a chart for this market; the sheet has closed itself.
    Add(SearchResult),
    /// Add (or focus) this portfolio card; the sheet has closed itself.
    AddCard(PortfolioCard),
    /// Open Settings on the Alpaca page; the sheet stays open.
    OpenAlpacaSettings,
}

type TrendingInbox = Receiver<(DataSource, Result<Vec<SearchResult>, String>)>;

/// The provider's busiest markets, shown while the prediction search box is empty.
#[derive(Default)]
struct Trending {
    provider: Option<DataSource>,
    events: EventList,
    /// When the list stops being fresh; `None` before the first answer.
    expires: Option<Instant>,
    inbox: Option<TrendingInbox>,
}

impl Trending {
    fn is_loading(&self) -> bool {
        self.inbox.is_some()
    }

    /// The events for `provider`, empty while another provider's are still held.
    fn events_for(&mut self, provider: DataSource) -> Option<&mut EventList> {
        (self.provider == Some(provider) && !self.events.is_empty()).then_some(&mut self.events)
    }

    fn receive(&mut self) {
        let Some(receiver) = &self.inbox else {
            return;
        };
        let Ok((provider, answer)) = receiver.try_recv() else {
            return;
        };
        self.inbox = None;
        self.provider = Some(provider);
        // A failure is quiet: trending is an extra, not something the user asked for.
        let (results, lifetime) = match answer {
            Ok(results) => (results, TRENDING_TTL),
            Err(_) => (Vec::new(), TRENDING_RETRY),
        };
        self.events.set(results.iter());
        self.expires = Some(Instant::now() + lifetime);
    }

    /// Starts a fetch when `provider` can list trending markets and the held list is stale.
    fn refresh(&mut self, provider: DataSource, ctx: &egui::Context) {
        self.receive();
        if self.inbox.is_some() || !service::supports_trending(provider) {
            return;
        }
        let fresh = self.provider == Some(provider)
            && self.expires.is_some_and(|expires| Instant::now() < expires);
        if fresh {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let _ = sender.send((provider, service::trending(provider, TRENDING_LIMIT)));
            repaint.request_repaint();
        });
        self.inbox = Some(receiver);
    }
}

pub struct AddChartSheet {
    store: SharedStore,
    open: bool,
    tab: Tab,
    crypto: SearchSession,
    stocks: SearchSession,
    predictions: SearchSession,
    trending: Trending,
    provider: DataSource,
    cmc_index: usize,
    /// The portfolios a card can cover, set by the host while the sheet is open.
    portfolios: Vec<Portfolio>,
    card_kind: usize,
    card_scope: Option<u64>,
    recents: Vec<RecentMarket>,
    needs_alpaca_setup: bool,
    focus_search: bool,
    watchlist_name: Option<String>,
}

impl AddChartSheet {
    pub fn new(store: SharedStore) -> Self {
        Self {
            store,
            open: false,
            tab: Tab::Crypto,
            crypto: SearchSession::new(),
            stocks: SearchSession::new(),
            predictions: SearchSession::new(),
            trending: Trending::default(),
            provider: DataSource::Polymarket,
            cmc_index: 0,
            portfolios: Vec::new(),
            card_kind: 0,
            card_scope: None,
            recents: Vec::new(),
            needs_alpaca_setup: false,
            focus_search: false,
            watchlist_name: None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The portfolios the Portfolio section offers; call while the sheet is open.
    pub fn set_portfolios(&mut self, portfolios: Vec<Portfolio>) {
        if self.portfolios != portfolios {
            self.portfolios = portfolios;
        }
    }

    pub fn is_watchlist_mode(&self) -> bool {
        self.watchlist_name.is_some()
    }

    /// Opens a fresh sheet on the Crypto section, as the Swift sheet starts anew each time.
    pub fn open(&mut self) {
        let store = self.store.clone();
        *self = Self::new(store);
        self.recents = self
            .store
            .lock()
            .ok()
            .map(|store| load_recents(&store))
            .unwrap_or_default();
        self.open = true;
        self.focus_search = true;
    }

    pub fn open_for_watchlist(&mut self, name: &str) {
        self.open();
        self.watchlist_name = Some(name.to_owned());
    }

    /// Runs due searches; called every frame while the workspace is shown.
    pub fn tick(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        self.crypto.tick(&CRYPTO_SOURCES, ctx);
        self.stocks.tick(&STOCK_SOURCES, ctx);
        let provider = [self.provider];
        self.predictions.tick(&provider, ctx);
        if self.tab == Tab::Predictions && self.predictions.is_idle() {
            self.trending.refresh(self.provider, ctx);
        }
    }

    fn session(&mut self, tab: Tab) -> Option<&mut SearchSession> {
        match tab {
            Tab::Crypto => Some(&mut self.crypto),
            Tab::Stocks => Some(&mut self.stocks),
            Tab::Predictions => Some(&mut self.predictions),
            Tab::CoinMarketCap | Tab::Portfolio => None,
        }
    }

    /// Whichever section is showing owns the selection Add commits.
    fn active_selection(&self) -> Option<SearchResult> {
        match self.tab {
            Tab::Crypto => self.crypto.selected.clone(),
            Tab::Stocks => self.stocks.selected.clone(),
            Tab::Predictions => self.predictions.selected.clone(),
            Tab::Portfolio => None,
            Tab::CoinMarketCap => {
                let (key, title) = service::market::cmc::CHART_TYPES[self.cmc_index];
                Some(SearchResult {
                    symbol: title.into(),
                    full_symbol: key.into(),
                    source: DataSource::CoinMarketCap,
                    price: None,
                    event: None,
                    base: None,
                    quote: None,
                })
            }
        }
    }

    fn set_tab(&mut self, tab: Tab) {
        if self.tab != tab {
            self.tab = tab;
            self.needs_alpaca_setup = false;
            self.focus_search = true;
        }
    }

    /// Validates and hands `result` to the host, recording it as recent.
    fn commit(&mut self, result: SearchResult) -> Option<AddChartAction> {
        if result.source == DataSource::Alpaca
            && service::keychain::api_keys::alpaca_credentials().is_none()
        {
            self.needs_alpaca_setup = true;
            return None;
        }
        if self.watchlist_name.is_none() && record_recent(&mut self.recents, &result) {
            self.save_recents();
        }
        if self.watchlist_name.is_none() {
            self.open = false;
        }
        Some(AddChartAction::Add(result))
    }

    /// The card the Portfolio section has picked, for the host to add.
    fn commit_card(&mut self) -> Option<AddChartAction> {
        if self.portfolios.is_empty() {
            return None;
        }
        let kind = CardKind::ALL[self.card_kind.min(CardKind::ALL.len() - 1)];
        let scope = self
            .card_scope
            .filter(|id| self.portfolios.iter().any(|portfolio| portfolio.id == *id));
        if self.watchlist_name.is_none() {
            self.open = false;
        }
        Some(AddChartAction::AddCard(PortfolioCard::new(kind, scope)))
    }

    fn save_recents(&self) {
        if let Ok(store) = self.store.lock() {
            store_recents(&store, &self.recents);
        }
    }

    /// Selects a recent market on the section that owns its source, switching to it.
    fn pick_recent(&mut self, result: SearchResult) {
        if result.source == DataSource::Alpaca {
            self.set_tab(Tab::Stocks);
            self.stocks.selected = Some(result);
        } else {
            self.set_tab(Tab::Crypto);
            self.crypto.selected = Some(result);
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<AddChartAction> {
        if !self.open {
            return None;
        }
        let screen = ctx.content_rect().size();
        let size = vec2(
            SHEET_SIZE.x.min(screen.x - 32.0).max(560.0),
            SHEET_SIZE.y.min(screen.y - 32.0).max(440.0),
        );
        let search_id = Id::new("add_chart_search_field");
        let field_focused = ctx.memory(|memory| memory.has_focus(search_id));
        let mut action = None;
        let mut request: Option<Request> = None;

        let shown = egui::Modal::new(Id::new("add_chart_sheet"))
            .backdrop_color(Color32::from_black_alpha(110))
            .frame(
                egui::Frame::new()
                    .fill(theme::chrome())
                    .corner_radius(12.0)
                    .stroke(Stroke::new(1.0, theme::ink(22)))
                    .shadow(egui::Shadow {
                        offset: [0, 12],
                        blur: 40,
                        spread: 0,
                        color: Color32::from_black_alpha(130),
                    }),
            )
            .show(ctx, |ui| {
                ui.set_width(size.x);
                ui.set_height(size.y);
                ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
                let footer_height = 60.0;
                let body =
                    Rect::from_min_size(ui.min_rect().min, vec2(size.x, size.y - footer_height))
                        .shrink(PADDING);
                let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body));
                body_ui.set_clip_rect(body.expand(2.0));
                self.show_body(&mut body_ui, search_id, &mut request);

                let divider_y = ui.min_rect().top() + size.y - footer_height;
                ui.painter().hline(
                    ui.min_rect().x_range(),
                    divider_y,
                    Stroke::new(1.0, theme::separator()),
                );
                let footer = Rect::from_min_size(
                    pos2(ui.min_rect().left() + PADDING, divider_y + 1.0),
                    vec2(size.x - PADDING * 2.0, footer_height - 1.0),
                );
                let mut footer_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(footer)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                self.show_footer(&mut footer_ui, &mut request);
            });
        // A click on the dimmed area around the sheet cancels it, like Escape. A click that only
        // closed a pop-up menu does not.
        if shown.backdrop_response.clicked() && !shown.any_popup_open {
            request = Some(Request::Cancel);
        }

        let enter = ctx.input(|input| input.key_pressed(egui::Key::Enter));
        let escape = ctx.input(|input| input.key_pressed(egui::Key::Escape));
        if escape {
            request = Some(Request::Cancel);
        } else if enter && !field_focused && request.is_none() {
            request = Some(Request::CommitSelection);
        }

        match request {
            Some(Request::Cancel) => self.open = false,
            Some(Request::CommitSelection) if self.tab == Tab::Portfolio => {
                action = self.commit_card();
            }
            Some(Request::CommitSelection) => {
                if let Some(selected) = self.active_selection() {
                    action = self.commit(selected);
                }
            }
            Some(Request::Commit(result)) => action = self.commit(result),
            Some(Request::OpenAlpacaSettings) => action = Some(AddChartAction::OpenAlpacaSettings),
            None => {}
        }
        action
    }

    // MARK: Body

    fn show_body(&mut self, ui: &mut Ui, search_id: Id, request: &mut Option<Request>) {
        sheet_header(ui, self.watchlist_name.as_deref(), self.tab);
        ui.add_space(16.0);
        if let Some(tab) = tab_bar(ui, self.tab) {
            self.set_tab(tab);
        }
        ui.add_space(16.0);

        let status_height = self.status_height();
        let content_height = (ui.available_height()
            - if status_height > 0.0 {
                status_height + 16.0
            } else {
                0.0
            })
        .max(80.0);
        let content =
            Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), content_height));
        let mut content_ui = ui.new_child(egui::UiBuilder::new().max_rect(content));
        match self.tab {
            Tab::CoinMarketCap => self.show_cmc(&mut content_ui),
            Tab::Portfolio => self.show_portfolio(&mut content_ui),
            tab => self.show_search_pane(&mut content_ui, tab, search_id, request),
        }
        ui.advance_cursor_after_rect(content);

        if status_height > 0.0 {
            ui.add_space(16.0);
            self.show_status(ui, request);
        }
    }

    fn status_height(&self) -> f32 {
        let mut height = 0.0;
        if self.tab == Tab::Predictions
            && let Some(_) = &self.predictions.error
        {
            height += 50.0;
        }
        if self.needs_alpaca_setup {
            height += 54.0;
        }
        height
    }

    fn show_status(&mut self, ui: &mut Ui, request: &mut Option<Request>) {
        if self.tab == Tab::Predictions
            && let Some(error) = self.predictions.error.clone()
        {
            notice_card(ui, theme::ORANGE, &error, None);
        }
        if self.needs_alpaca_setup
            && notice_card(
                ui,
                theme::ORANGE,
                "Set up Alpaca before adding a stock chart.",
                Some("Open Settings"),
            )
        {
            *request = Some(Request::OpenAlpacaSettings);
        }
    }

    fn show_search_pane(
        &mut self,
        ui: &mut Ui,
        tab: Tab,
        search_id: Id,
        request: &mut Option<Request>,
    ) {
        let focus = std::mem::take(&mut self.focus_search);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if tab == Tab::Predictions
                && let Some(provider) = provider_menu(ui, self.provider)
                && provider != self.provider
            {
                self.provider = provider;
                self.predictions.reset_results();
            }
            let session = self.session(tab).expect("search section");
            if let Some(submit) = search_field(ui, session, tab.placeholder(), search_id, focus) {
                debug_assert!(submit);
                // Return picks the first result; on a selection that already is the first, it adds.
                if let Some(first) = session.first_visible(tab == Tab::Predictions).cloned() {
                    if session
                        .selected
                        .as_ref()
                        .is_some_and(|selected| same_market(selected, &first))
                    {
                        *request = Some(Request::Commit(first));
                    } else {
                        session.selected = Some(first);
                    }
                }
            }
        });
        ui.add_space(16.0);

        let session = self.session(tab).expect("search section");
        if session.is_idle() {
            self.show_idle(ui, tab, request);
            return;
        }
        if session.has_results() {
            let commit = if tab == Tab::Predictions {
                prediction_list(ui, session)
            } else {
                result_list(ui, session)
            };
            if commit.is_some() {
                *request = commit.map(Request::Commit);
            }
        } else if !session.is_searching() && tab != Tab::Predictions {
            if let Some(error) = session.error.clone() {
                no_results(ui, "Search Failed", &error);
            } else {
                let query = session.trimmed().to_owned();
                no_results(
                    ui,
                    &format!("No Results for “{query}”"),
                    "Check the spelling or try a new search.",
                );
            }
        } else if !session.is_searching() && session.error.is_none() {
            ui.label(text(
                "No markets found",
                size::CAPTION,
                400.0,
                theme::text_secondary(),
            ));
        }
    }

    /// Suggestions plus ten recents can outgrow the pane; they scroll rather than push the sheet.
    fn show_idle(&mut self, ui: &mut Ui, tab: Tab, request: &mut Option<Request>) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                match tab {
                    Tab::Crypto => {
                        let items = CRYPTO_SUGGESTIONS
                            .map(|ticker| Chip::Logo(ticker, DataSource::Binance));
                        if let Some(index) = suggestion_grid(ui, "Suggestions", &items) {
                            self.crypto.search_now(CRYPTO_SUGGESTIONS[index]);
                        }
                    }
                    Tab::Stocks => {
                        let items =
                            STOCK_SUGGESTIONS.map(|ticker| Chip::Logo(ticker, DataSource::Alpaca));
                        if let Some(index) = suggestion_grid(
                            ui,
                            "Popular US stocks and ETFs · free IEX feed",
                            &items,
                        ) {
                            let symbol = STOCK_SUGGESTIONS[index];
                            self.stocks.selected = Some(SearchResult {
                                symbol: symbol.into(),
                                full_symbol: symbol.into(),
                                source: DataSource::Alpaca,
                                price: None,
                                event: None,
                                base: None,
                                quote: None,
                            });
                            // Only search when there is a feed to ask: the text would hide the
                            // chips, leaving an empty pane.
                            if service::keychain::api_keys::alpaca_credentials().is_some() {
                                self.stocks.search_now(symbol);
                            }
                        }
                    }
                    Tab::Predictions => {
                        let items =
                            PREDICTION_TOPICS.map(|(title, _, icon)| Chip::Symbol(title, icon));
                        if let Some(index) = suggestion_grid(ui, "Popular topics", &items) {
                            self.predictions.search_now(PREDICTION_TOPICS[index].1);
                        }
                        self.show_trending(ui, request);
                    }
                    Tab::CoinMarketCap | Tab::Portfolio => {}
                }
                if tab != Tab::Predictions && !self.recents.is_empty() {
                    ui.add_space(16.0);
                    self.show_recents(ui, tab, request);
                }
            });
    }

    /// "Trending on Polymarket": the busiest events under the topic chips, listed like results.
    fn show_trending(&mut self, ui: &mut Ui, request: &mut Option<Request>) {
        let provider = self.provider;
        let selected = &mut self.predictions.selected;
        let Some(events) = self.trending.events_for(provider) else {
            if self.trending.is_loading() {
                let (area, _) = ui.allocate_exact_size(
                    vec2(ui.available_width(), TRENDING_LOADING_HEIGHT),
                    Sense::hover(),
                );
                loading::message(ui, area, "Loading trending markets…");
            }
            return;
        };
        ui.add_space(16.0);
        let mut commit = None;
        result_well(ui, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                caption_label(
                    ui,
                    Icon::Flame,
                    &format!("Trending on {}", provider.display_name()),
                );
            });
            ui.add_space(2.0);
            commit = event_groups(ui, events, selected);
            ui.add_space(6.0);
        });
        if let Some(result) = commit {
            *request = Some(Request::Commit(result));
        }
    }

    fn show_recents(&mut self, ui: &mut Ui, tab: Tab, request: &mut Option<Request>) {
        let selected = match tab {
            Tab::Stocks => self.stocks.selected.clone(),
            _ => self.crypto.selected.clone(),
        };
        let outcome = recents_card(ui, &mut self.recents, selected.as_ref());
        if let Some(result) = outcome.commit {
            *request = Some(Request::Commit(result));
        } else if let Some(result) = outcome.pick {
            self.pick_recent(result);
        }
        if outcome.changed {
            self.save_recents();
        }
    }

    fn show_cmc(&mut self, ui: &mut Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.label(text(
                    "Market-wide CoinMarketCap indices",
                    size::SUBHEADLINE,
                    MEDIUM,
                    theme::text_primary(),
                ));
                ui.add_space(10.0);
                let columns = if ui.available_width() < 650.0 { 1 } else { 2 };
                let width = (ui.available_width() - 10.0 * (columns - 1) as f32) / columns as f32;
                for row in 0..service::market::cmc::CHART_TYPES.len().div_ceil(columns) {
                    if row > 0 {
                        ui.add_space(10.0);
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        for column in 0..columns {
                            let index = row * columns + column;
                            if index >= service::market::cmc::CHART_TYPES.len() {
                                break;
                            }
                            let (_, title) = service::market::cmc::CHART_TYPES[index];
                            let (icon, subtitle) = CMC_DETAILS[index];
                            if choice_card(
                                ui,
                                width,
                                icon,
                                title,
                                subtitle,
                                self.cmc_index == index,
                            )
                            .clicked()
                            {
                                self.cmc_index = index;
                            }
                        }
                    });
                }
            });
    }

    /// "Portfolio": which portfolio, then what to show about it, as cards like the index list.
    fn show_portfolio(&mut self, ui: &mut Ui) {
        if self.portfolios.is_empty() {
            let area = ui.available_rect_before_wrap();
            crate::ui::kit::empty_state(
                ui,
                area,
                Icon::Briefcase,
                "No Portfolios",
                "Create a portfolio in the Portfolio tab, then add its charts here.",
            );
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.label(text(
                    "Which portfolio",
                    size::SUBHEADLINE,
                    MEDIUM,
                    theme::text_primary(),
                ));
                ui.add_space(8.0);
                let mut options: Vec<(Option<u64>, &str)> = vec![(None, "All Portfolios")];
                options.extend(
                    self.portfolios
                        .iter()
                        .map(|portfolio| (Some(portfolio.id), portfolio.name.as_str())),
                );
                if !options.iter().any(|(id, _)| *id == self.card_scope) {
                    self.card_scope = None;
                }
                if options.len() <= 4 {
                    crate::ui::kit::segmented(
                        ui,
                        Id::new("add-card-scope"),
                        &options,
                        &mut self.card_scope,
                    );
                } else {
                    let current = options
                        .iter()
                        .find(|(id, _)| *id == self.card_scope)
                        .map_or("All Portfolios", |(_, name)| *name);
                    egui::ComboBox::from_id_salt("add-card-scope")
                        .selected_text(current)
                        .width(220.0)
                        .show_ui(ui, |ui| {
                            for (id, name) in &options {
                                ui.selectable_value(&mut self.card_scope, *id, *name);
                            }
                        });
                }
                ui.add_space(18.0);
                ui.label(text(
                    "What to show",
                    size::SUBHEADLINE,
                    MEDIUM,
                    theme::text_primary(),
                ));
                ui.add_space(10.0);
                let columns = if ui.available_width() < 650.0 { 1 } else { 2 };
                let width = (ui.available_width() - 10.0 * (columns - 1) as f32) / columns as f32;
                for row in 0..CardKind::ALL.len().div_ceil(columns) {
                    if row > 0 {
                        ui.add_space(10.0);
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        for column in 0..columns {
                            let index = row * columns + column;
                            let Some(kind) = CardKind::ALL.get(index) else {
                                break;
                            };
                            if choice_card(
                                ui,
                                width,
                                crate::ui::portfolio::cards::icon(*kind),
                                kind.title(),
                                kind.summary(),
                                self.card_kind == index,
                            )
                            .clicked()
                            {
                                self.card_kind = index;
                            }
                        }
                    });
                }
            });
    }

    // MARK: Footer

    fn show_footer(&mut self, ui: &mut Ui, request: &mut Option<Request>) {
        ui.spacing_mut().item_spacing.x = 12.0;
        let selection = match self.tab {
            Tab::CoinMarketCap | Tab::Portfolio => None,
            _ => self.active_selection(),
        };
        if let Some(result) = &selection {
            selected_banner(ui, result, (ui.available_width() - 200.0).min(380.0));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let can_commit = match self.tab {
                Tab::CoinMarketCap => true,
                Tab::Portfolio => !self.portfolios.is_empty(),
                _ => selection.is_some(),
            };
            if sheet_button(ui, "Add", true, can_commit).clicked() {
                *request = Some(Request::CommitSelection);
            }
            if sheet_button(ui, "Cancel", false, true).clicked() {
                *request = Some(Request::Cancel);
            }
        });
    }
}

enum Request {
    Cancel,
    CommitSelection,
    Commit(SearchResult),
    OpenAlpacaSettings,
}

/// What the user did in the Recent card this frame.
#[derive(Default)]
pub(crate) struct RecentsOutcome {
    /// A market was clicked: select it.
    pub pick: Option<SearchResult>,
    /// A market was double-clicked: take it.
    pub commit: Option<SearchResult>,
    /// A market was removed or the list cleared: save it.
    pub changed: bool,
}

/// The "Recent" card: the markets picked before, in rows with a Clear link and a context menu to
/// remove one. Shared by the Add Chart sheet and the portfolio's asset picker.
pub(crate) fn recents_card(
    ui: &mut Ui,
    recents: &mut Vec<RecentMarket>,
    selected: Option<&SearchResult>,
) -> RecentsOutcome {
    let mut outcome = RecentsOutcome::default();
    let mut remove = None;
    let mut clear = false;
    card(ui, |ui| {
        ui.horizontal(|ui| {
            caption_label(ui, Icon::ClockArrow, "Recent");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let clear_button = ui.add(
                    egui::Label::new(text(
                        "Clear",
                        size::CAPTION + 0.5,
                        400.0,
                        theme::text_secondary(),
                    ))
                    .sense(Sense::click()),
                );
                if clear_button
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    clear = true;
                }
            });
        });
        ui.add_space(8.0);
        let column = (ui.available_width() - RECENT_GAP * (RECENT_COLUMNS - 1.0)) / RECENT_COLUMNS;
        for (row, group) in recents.chunks(RECENT_COLUMNS as usize).enumerate() {
            if row > 0 {
                ui.add_space(2.0);
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = RECENT_GAP;
                for (offset, market) in group.iter().enumerate() {
                    let result = market.result();
                    let is_selected = selected.is_some_and(|picked| same_market(picked, &result));
                    let response = result_row(ui, &result, is_selected, true, column);
                    response.context_menu(|ui| {
                        if ui.button("Remove from Recent").clicked() {
                            remove = Some(row * RECENT_COLUMNS as usize + offset);
                            ui.close();
                        }
                    });
                    if response.double_clicked() {
                        outcome.commit = Some(result);
                    } else if response.clicked() {
                        outcome.pick = Some(result);
                    }
                }
            });
        }
    });
    if let Some(index) = remove {
        recents.remove(index);
        outcome.changed = true;
    }
    if clear {
        recents.clear();
        outcome.changed = true;
    }
    outcome
}

// MARK: Components

/// The top of the sheet: a picture for the tab (or for the watchlist), the title and a one-line
/// description.
fn sheet_header(ui: &mut Ui, watchlist_name: Option<&str>, tab: Tab) {
    let scene = if watchlist_name.is_some() {
        art::Scene::Watchlist
    } else {
        match tab {
            Tab::Crypto => art::Scene::Crypto,
            Tab::Stocks => art::Scene::Stocks,
            Tab::Predictions => art::Scene::Predictions,
            Tab::CoinMarketCap => art::Scene::CoinMarketCap,
            Tab::Portfolio => art::Scene::Portfolio,
        }
    };
    let title =
        watchlist_name.map_or_else(|| "Add Chart".to_owned(), |name| format!("Add to {name}"));
    let subtitle = if watchlist_name.is_some() {
        "Search a market, stock or prediction market to follow in this watchlist."
    } else {
        "Search a market, stock or index and add it as a live chart."
    };
    header(ui, scene, &title, subtitle);
}

/// A sheet's header: `scene`'s picture, then the title and a one-line description. Add Chart and
/// the portfolio's asset picker both open with it.
pub(crate) fn header(ui: &mut Ui, scene: art::Scene, title: &str, subtitle: &str) {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), art::SIZE.y), Sense::hover());
    let art = Rect::from_min_size(row.min, art::SIZE);
    art::paint(ui, art, scene);
    paint_header_text(ui, row, art.right() + 16.0, title, subtitle);
}

/// A header's title and description, vertically centred in `row` from `left`.
fn paint_header_text(ui: &Ui, row: Rect, left: f32, title: &str, subtitle: &str) {
    let painter = ui.painter();
    let width = row.right() - left;
    let title = galley(
        ui,
        title,
        size::TITLE2 + 1.0,
        SEMIBOLD,
        theme::text_primary(),
        width,
    );
    let subtitle = galley(
        ui,
        subtitle,
        size::SUBHEADLINE + 1.0,
        400.0,
        theme::text_secondary(),
        width,
    );
    let total = title.size().y + 2.0 + subtitle.size().y;
    let top = row.center().y - total / 2.0;
    painter.galley(pos2(left, top), title.clone(), theme::text_primary());
    painter.galley(
        pos2(left, top + title.size().y + 2.0),
        subtitle,
        theme::text_secondary(),
    );
}

/// The section switcher (Swift `IconTabBar`, compact): a pill track whose segments hug an icon
/// and a title, the selected one raised on a darker plate.
fn tab_bar(ui: &mut Ui, selected: Tab) -> Option<Tab> {
    let items = Tab::ALL.map(|tab| (tab, tab.title(), tab.icon()));
    segmented_tabs(ui, &items, selected)
}

/// The pill-track section switcher over any set of `(value, title, icon)` segments.
pub(crate) fn segmented_tabs<T: Copy + PartialEq>(
    ui: &mut Ui,
    items: &[(T, &str, Icon)],
    selected: T,
) -> Option<T> {
    let segments: Vec<(T, &str, Icon, Arc<Galley>, f32)> = items
        .iter()
        .map(|&(tab, title, icon)| {
            let weight = if tab == selected { SEMIBOLD } else { MEDIUM };
            let color = if tab == selected {
                theme::text_primary()
            } else {
                theme::text_secondary()
            };
            let label = galley(ui, title, 12.5, weight, color, f32::INFINITY);
            let width = 11.0 + 13.0 + 5.0 + label.size().x + 11.0;
            (tab, title, icon, label, width)
        })
        .collect();
    let segment_height = 28.0;
    let track_width = segments.iter().map(|segment| segment.4).sum::<f32>()
        + 2.0 * (segments.len() - 1) as f32
        + 6.0;
    let (row, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), segment_height + 6.0),
        Sense::hover(),
    );
    let track = Rect::from_center_size(row.center(), vec2(track_width, segment_height + 6.0));
    let painter = ui.painter();
    painter.rect_filled(track, 11.0, theme::quaternary(0.6));
    painter.rect_stroke(
        track,
        11.0,
        Stroke::new(1.0, theme::ink(14)),
        StrokeKind::Inside,
    );

    let mut clicked = None;
    let mut x = track.left() + 3.0;
    for (tab, title, icon, label, width) in segments {
        let segment = Rect::from_min_size(pos2(x, track.top() + 3.0), vec2(width, segment_height));
        let response = ui
            .interact(
                segment,
                ui.id().with(("segmented_tab", title)),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let response = crate::ui::accessibility::label(response, title);
        let active = tab == selected;
        if active {
            painter.add(
                egui::Shadow {
                    offset: [0, 1],
                    blur: 3,
                    spread: 0,
                    color: Color32::from_black_alpha(40),
                }
                .as_shape(segment, 8.0),
            );
            painter.rect_filled(segment, 8.0, theme::editor_background());
            painter.rect_stroke(
                segment,
                8.0,
                Stroke::new(1.0, theme::ink(20)),
                StrokeKind::Inside,
            );
        } else if response.hovered() {
            painter.rect_filled(segment, 8.0, theme::ink(8));
        }
        let icon_rect = Rect::from_min_size(
            pos2(segment.left() + 11.0, segment.center().y - 6.5),
            Vec2::splat(13.0),
        );
        icons::paint(
            painter,
            icon_rect,
            icon,
            if active {
                theme::text_primary()
            } else {
                theme::text_secondary()
            },
        );
        painter.galley(
            pos2(
                icon_rect.right() + 5.0,
                segment.center().y - label.size().y / 2.0,
            ),
            label,
            theme::text_primary(),
        );
        if response.clicked() {
            clicked = Some(tab);
        }
        x += width + 2.0;
    }
    clicked
}

/// The search box: magnifier, plain text field, spinner while searching and a clear button.
/// Returns `Some(true)` when Return was pressed in it.
fn search_field(
    ui: &mut Ui,
    session: &mut SearchSession,
    placeholder: &str,
    id: Id,
    focus: bool,
) -> Option<bool> {
    let searching = session.is_searching();
    let event = search_box(ui, &mut session.query, searching, placeholder, id, focus);
    if event.changed {
        session.edited();
    }
    if event.cleared {
        session.clear();
    }
    event.submitted.then_some(true)
}

/// What happened in a [`search_box`] this frame.
#[derive(Default)]
pub(crate) struct SearchBoxEvent {
    pub changed: bool,
    pub submitted: bool,
    pub cleared: bool,
}

pub(crate) fn search_box(
    ui: &mut Ui,
    query: &mut String,
    searching: bool,
    placeholder: &str,
    id: Id,
    focus: bool,
) -> SearchBoxEvent {
    let focused = ui.memory(|memory| memory.has_focus(id));
    let mut event = SearchBoxEvent::default();
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, SEARCH_FIELD_HEIGHT), Sense::hover());
    ui.painter().rect_filled(rect, 9.0, theme::quaternary(0.5));
    ui.painter().rect_stroke(
        rect,
        9.0,
        Stroke::new(
            1.0,
            if focused {
                theme::with_alpha(theme::text_secondary(), 0.7)
            } else {
                theme::ink(31)
            },
        ),
        StrokeKind::Inside,
    );
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(12.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    inner.spacing_mut().item_spacing.x = 8.0;
    icons::inline(
        &mut inner,
        Icon::Magnifier,
        14.0,
        if focused {
            theme::text_primary()
        } else {
            theme::text_secondary()
        },
    );
    let trailing = if query.is_empty() { 0.0 } else { 24.0 } + if searching { 24.0 } else { 0.0 };
    let edit = egui::TextEdit::singleline(query)
        .id(id)
        .frame(egui::Frame::NONE)
        .font(FontId::proportional(size::BODY))
        .text_color(theme::text_primary())
        .hint_text(text(placeholder, size::BODY, 400.0, theme::text_tertiary()))
        .desired_width((inner.available_width() - trailing).max(60.0));
    let response = inner.add(edit);
    if focus {
        response.request_focus();
    }
    event.changed = response.changed();
    if response.lost_focus() && inner.input(|input| input.key_pressed(egui::Key::Enter)) {
        event.submitted = true;
        response.request_focus();
    }
    if searching {
        inner.add(LoadingIndicator::new().size(13.0));
    }
    if !query.is_empty()
        && icons::button(
            &mut inner,
            Icon::XmarkCircleFill,
            22.0,
            theme::text_tertiary(),
            "Clear search",
        )
        .clicked()
    {
        event.cleared = true;
        response.request_focus();
    }
    event
}

/// The Polymarket / Kalshi switch, styled like the search box beside it.
pub(crate) fn provider_menu(ui: &mut Ui, provider: DataSource) -> Option<DataSource> {
    let name = galley(
        ui,
        provider.display_name(),
        size::BODY,
        MEDIUM,
        theme::text_primary(),
        f32::INFINITY,
    );
    let width = 12.0 + 18.0 + 8.0 + name.size().x + 8.0 + 9.0 + 12.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, SEARCH_FIELD_HEIGHT), Sense::click());
    let response = crate::ui::accessibility::label(response, "Prediction market provider")
        .on_hover_text("Prediction market provider")
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        9.0,
        if response.hovered() {
            theme::quaternary(0.7)
        } else {
            theme::quaternary(0.5)
        },
    );
    painter.rect_stroke(
        rect,
        9.0,
        Stroke::new(1.0, theme::ink(31)),
        StrokeKind::Inside,
    );
    let logo = Rect::from_min_size(
        pos2(rect.left() + 12.0, rect.center().y - 9.0),
        Vec2::splat(18.0),
    );
    source_logo::paint(ui, provider, logo);
    painter.galley(
        pos2(logo.right() + 8.0, rect.center().y - name.size().y / 2.0),
        name,
        theme::text_primary(),
    );
    icons::paint(
        painter,
        Rect::from_center_size(pos2(rect.right() - 16.5, rect.center().y), Vec2::splat(9.0)),
        Icon::ChevronDown,
        theme::text_secondary(),
    );
    let mut picked = None;
    egui::Popup::menu(&response).gap(4.0).show(|ui| {
        ui.set_min_width(150.0);
        for source in PREDICTION_PROVIDERS {
            let row = ui
                .horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let (mark, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                    if source == provider {
                        icons::paint(ui.painter(), mark, Icon::Check, theme::text_primary());
                    }
                    source_logo::show(ui, source, 16.0);
                    ui.add(
                        egui::Button::new(text(
                            source.display_name(),
                            size::BODY,
                            400.0,
                            theme::text_primary(),
                        ))
                        .frame(false),
                    )
                })
                .inner;
            if row.clicked() {
                picked = Some(source);
            }
        }
    });
    picked
}

/// The quiet rounded panel suggestions and recents sit in.
pub(crate) fn card(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(theme::quaternary(0.3))
        .corner_radius(10.0)
        .stroke(Stroke::new(1.0, theme::ink(14)))
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            content(ui);
        });
}

/// A card's heading: a small icon and a medium-weight secondary caption.
pub(crate) fn caption_label(ui: &mut Ui, icon: Icon, caption: &str) {
    ui.spacing_mut().item_spacing.x = 5.0;
    icons::inline(ui, icon, 12.0, theme::text_secondary());
    ui.label(text(
        caption,
        size::SUBHEADLINE + 1.0,
        MEDIUM,
        theme::text_secondary(),
    ));
}

pub(crate) enum Chip {
    /// A coin or company logo, looked up for the chip's title as a ticker of the source.
    Logo(&'static str, DataSource),
    Symbol(&'static str, Icon),
}

/// Quick-fill chips use five columns at the Swift sheet width and three in a narrow window.
pub(crate) fn suggestion_grid(ui: &mut Ui, caption: &str, items: &[Chip]) -> Option<usize> {
    let mut clicked = None;
    card(ui, |ui| {
        ui.horizontal(|ui| caption_label(ui, Icon::Sparkles, caption));
        ui.add_space(10.0);
        let spacing = 8.0;
        let columns = if ui.available_width() < 650.0 { 3 } else { 5 };
        let width = (ui.available_width() - spacing * (columns - 1) as f32) / columns as f32;
        for (row, chunk) in items.chunks(columns).enumerate() {
            if row > 0 {
                ui.add_space(spacing);
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = spacing;
                for (offset, item) in chunk.iter().enumerate() {
                    if suggestion_chip(ui, item, width).clicked() {
                        clicked = Some(row * columns + offset);
                    }
                }
            });
        }
    });
    clicked
}

fn suggestion_chip(ui: &mut Ui, item: &Chip, width: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let title = match item {
        Chip::Logo(title, _) | Chip::Symbol(title, _) => *title,
    };
    let response = crate::ui::accessibility::label(response, title);
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        8.0,
        theme::ink(if response.hovered() { 23 } else { 13 }),
    );
    painter.rect_stroke(
        rect,
        8.0,
        Stroke::new(1.0, theme::ink(20)),
        StrokeKind::Inside,
    );
    let label = galley(
        ui,
        title,
        size::SUBHEADLINE + 1.0,
        MEDIUM,
        theme::text_primary(),
        width - 20.0 - 25.0,
    );
    let content_width = 18.0 + 7.0 + label.size().x;
    let left = rect.center().x - content_width / 2.0;
    let icon_rect = Rect::from_min_size(pos2(left, rect.center().y - 9.0), Vec2::splat(18.0));
    match item {
        Chip::Logo(ticker, source) => {
            // The id the source knows the ticker by: Binance lists "BTC" as "BTCUSDT".
            let market = if *source == DataSource::Binance {
                format!("{ticker}USDT")
            } else {
                (*ticker).to_owned()
            };
            ticker_icon(ui, icon_rect, ticker, *source, &market);
        }
        Chip::Symbol(_, icon) => {
            icons::paint(painter, icon_rect.shrink(2.5), *icon, theme::text_primary())
        }
    }
    painter.galley(
        pos2(
            icon_rect.right() + 7.0,
            rect.center().y - label.size().y / 2.0,
        ),
        label,
        theme::text_primary(),
    );
    response
}

/// A coin, company or market logo in a circle; a hue-seeded monogram until (or unless) the
/// shared icon cache resolves one.
fn ticker_icon(ui: &Ui, rect: Rect, symbol: &str, source: DataSource, market: &str) {
    let lookup = supports_ticker_icon_lookup(source);
    if lookup && let Some(url) = service::image::icon::get_url(source, market) {
        egui::Image::new(url)
            .fit_to_exact_size(rect.size())
            .corner_radius(CornerRadius::same((rect.width() / 2.0) as u8))
            .paint_at(ui, rect);
        return;
    }
    if lookup {
        service::image::icon::request_with_base(source, market, symbol);
    }
    monogram(ui, rect, symbol);
}

fn supports_ticker_icon_lookup(source: DataSource) -> bool {
    matches!(
        source,
        DataSource::Binance
            | DataSource::Coinbase
            | DataSource::CoinGecko
            | DataSource::DexScreener
            | DataSource::Alpaca
    )
}

/// A hue-seeded circle with the symbol's first two letters.
fn monogram(ui: &Ui, rect: Rect, symbol: &str) {
    let seed = symbol
        .to_uppercase()
        .chars()
        .fold(0u32, |sum, c| sum.wrapping_add((c as u32).wrapping_mul(31)));
    let fill: Color32 = egui::ecolor::Hsva::new((seed % 360) as f32 / 360.0, 0.55, 0.7, 1.0).into();
    let painter = ui.painter();
    painter.circle_filled(rect.center(), rect.width() / 2.0, fill);
    let initials: String = symbol
        .chars()
        .filter(|c| c.is_alphanumeric())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    let monogram = galley(
        ui,
        &initials,
        rect.width() * 0.42,
        BOLD,
        Color32::WHITE,
        f32::INFINITY,
    );
    painter.galley(
        rect.center() - monogram.size() / 2.0,
        monogram,
        Color32::WHITE,
    );
}

/// A prediction market's own artwork in a circle, a monogram until (or unless) it loads.
fn prediction_icon(ui: &Ui, rect: Rect, symbol: &str, url: Option<&str>) {
    if let Some(url) = url {
        let image = egui::Image::new(url.to_owned())
            .fit_to_exact_size(rect.size())
            .corner_radius(CornerRadius::same((rect.width() / 2.0) as u8))
            .show_loading_spinner(false);
        if image
            .load_for_size(ui.ctx(), rect.size())
            .is_ok_and(|poll| poll.is_ready())
        {
            image.paint_at(ui, rect);
            return;
        }
    }
    monogram(ui, rect, symbol);
}

/// A result's artwork. Crypto pairs share one lookup per base asset, keyed as its Binance USDT
/// market, so ETH/BTC and ETH/EUR draw the same logo as ETH/USDT without a request each.
pub(crate) fn result_icon(ui: &Ui, rect: Rect, result: &SearchResult) {
    let base = base_symbol(result);
    match result.source {
        DataSource::Binance | DataSource::Coinbase | DataSource::CoinGecko => {
            ticker_icon(ui, rect, &base, DataSource::Binance, &format!("{base}USDT"));
        }
        source => ticker_icon(ui, rect, &base, source, &result.full_symbol),
    }
}

/// The ticker without a trading pair or company name: "BTC/USDT" → "BTC".
fn base_symbol(result: &SearchResult) -> String {
    result.base_asset()
}

/// One search hit: logo (with the data source badged on its corner when `shows_source`), symbol
/// and price, and a check when selected.
pub(crate) fn result_row(
    ui: &mut Ui,
    result: &SearchResult,
    selected: bool,
    shows_source: bool,
    width: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, RESULT_ROW_HEIGHT), Sense::click());
    let response = crate::ui::accessibility::label(response, &result.symbol);
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let painter = ui.painter();
    let fill = if selected {
        theme::selected_tint(false)
    } else if response.hovered() {
        theme::ink(15)
    } else {
        Color32::TRANSPARENT
    };
    painter.rect_filled(rect, 8.0, fill);
    if selected {
        painter.rect_stroke(rect, 8.0, theme::selected_stroke(), StrokeKind::Inside);
    }
    let content = rect.shrink2(vec2(10.0, 5.0));
    let icon = Rect::from_min_size(
        pos2(content.left(), rect.center().y - 14.0),
        Vec2::splat(28.0),
    );
    result_icon(ui, icon, result);
    if shows_source {
        crate::ui::kit::source_badge(ui, icon, result.source);
    }

    let mut right = content.right();
    let check = Rect::from_center_size(pos2(right - 7.0, rect.center().y), Vec2::splat(14.0));
    if selected {
        icons::paint(painter, check, Icon::CheckCircleFill, theme::text_primary());
    }
    right = check.left() - 10.0;
    if let Some(price) = result.price {
        let label = painter.layout_no_wrap(
            format!("${}", crate::format::price(price)),
            FontId::monospace(size::CALLOUT - 0.5),
            theme::text_secondary(),
        );
        right -= label.size().x;
        painter.galley(
            pos2(right, rect.center().y - label.size().y / 2.0),
            label,
            theme::text_secondary(),
        );
        right -= 10.0;
    }
    let left = icon.right() + 10.0;
    let symbol = galley(
        ui,
        &result.symbol,
        size::BODY,
        SEMIBOLD,
        theme::text_primary(),
        right - left,
    );
    painter.galley(
        pos2(left, rect.center().y - symbol.size().y / 2.0),
        symbol,
        theme::text_primary(),
    );
    let response = if shows_source {
        response.on_hover_text(result.source.display_name())
    } else {
        response
    };
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A prediction-market search: events with their choices in a quiet rounded well. Returns a
/// double-clicked choice.
fn prediction_list(ui: &mut Ui, session: &mut SearchSession) -> Option<SearchResult> {
    let SearchSession {
        events, selected, ..
    } = session;
    let height = ui.available_height();
    let mut commit = None;
    result_well(ui, |ui| {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(height - 10.0)
            .show(ui, |ui| {
                commit = event_groups(ui, events, selected);
                ui.add_space(6.0);
            });
    });
    commit
}

/// The rounded well result lists sit in.
fn result_well(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(theme::quaternary(0.25))
        .corner_radius(10.0)
        .stroke(Stroke::new(1.0, theme::ink(18)))
        .inner_margin(egui::Margin::symmetric(6, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            content(ui);
        });
}

/// Each event as a disclosure header over its choices, a tree divided from the next event. An
/// event with several choices ticks them to chart together; a lone choice is picked. Returns a
/// double-clicked lone choice.
fn event_groups(
    ui: &mut Ui,
    list: &mut EventList,
    selected: &mut Option<SearchResult>,
) -> Option<SearchResult> {
    let mut commit = None;
    let width = ui.available_width();
    let EventList { groups, expanded } = list;
    for (index, group) in groups.iter().enumerate() {
        let open = expanded.contains(&group.title);
        let ticks = group.results.len() > 1;
        let tick = ticks.then(|| group_tick(selected.as_ref(), group));
        let header = event_header(ui, group, open, tick, width);
        if header.tick_clicked {
            *selected = toggle_group(selected.as_ref(), group);
        } else if header.clicked && !expanded.remove(&group.title) {
            expanded.insert(group.title.clone());
        }
        if open {
            let branch_top = header.bottom;
            for result in &group.results {
                if ticks {
                    let ticked = is_ticked(selected.as_ref(), result);
                    let response = choice_row(ui, result, Mark::Tick(ticked), width);
                    if response.clicked() {
                        *selected = toggle_choice(selected.as_ref(), group, &result.full_symbol);
                    }
                } else {
                    let picked = selected
                        .as_ref()
                        .is_some_and(|picked| same_market(picked, result));
                    let response = choice_row(ui, result, Mark::Pick(picked), width);
                    if response.double_clicked() {
                        commit = Some(result.clone());
                    } else if response.clicked() {
                        *selected = Some(result.clone());
                    }
                }
                ui.add_space(2.0);
            }
            // The branch the choices hang from, down the chevron's column.
            let x = ui.min_rect().left() + EVENT_PAD + EVENT_CHEVRON / 2.0;
            ui.painter().vline(
                x,
                branch_top..=ui.cursor().top() - 2.0,
                Stroke::new(1.0, theme::separator()),
            );
        }
        if index + 1 < groups.len() {
            ui.add_space(4.0);
            let (line, _) = ui.allocate_exact_size(vec2(width, 1.0), Sense::hover());
            ui.painter().hline(
                line.shrink2(vec2(4.0, 0.0)).x_range(),
                line.center().y,
                Stroke::new(1.0, theme::separator()),
            );
        }
    }
    commit
}

// MARK: Ticking choices

/// How many of an event's choices are ticked.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Tick {
    None,
    Some,
    All,
}

/// Whether the pick has `result` among the choices ticked to chart together.
fn is_ticked(picked: Option<&SearchResult>, result: &SearchResult) -> bool {
    picked
        .and_then(|picked| picked.event.as_ref())
        .is_some_and(|event| {
            event
                .series
                .iter()
                .any(|choice| choice.token_id == result.full_symbol)
        })
}

fn group_tick(picked: Option<&SearchResult>, group: &EventGroup) -> Tick {
    let ticked = group
        .results
        .iter()
        .filter(|result| is_ticked(picked, result))
        .count();
    match ticked {
        0 => Tick::None,
        count if count == group.results.len() => Tick::All,
        _ => Tick::Some,
    }
}

/// The pick for the `group` choices `keep` accepts, in the order the group lists them: the first
/// is the market the chart opens on, and all of them are its lines. `None` when none are kept.
fn pick_choices(group: &EventGroup, keep: impl Fn(&SearchResult) -> bool) -> Option<SearchResult> {
    let kept: Vec<&SearchResult> = group.results.iter().filter(|result| keep(result)).collect();
    let mut pick = (*kept.first()?).clone();
    let event = pick.event.get_or_insert_with(|| EventInfo {
        title: group.title.clone(),
        ..EventInfo::default()
    });
    event.series = kept
        .iter()
        .map(|result| PmSeries {
            token_id: result.full_symbol.clone(),
            label: result.symbol.clone(),
        })
        .collect();
    Some(pick)
}

/// The pick after ticking or unticking one choice. Ticking in another event drops the ticks of
/// the event before it: only one event is charted at a time.
fn toggle_choice(
    picked: Option<&SearchResult>,
    group: &EventGroup,
    token_id: &str,
) -> Option<SearchResult> {
    pick_choices(group, |result| {
        (result.full_symbol == token_id) != is_ticked(picked, result)
    })
}

/// The pick after the event's checkbox: every choice ticked, or none when all already were.
fn toggle_group(picked: Option<&SearchResult>, group: &EventGroup) -> Option<SearchResult> {
    match group_tick(picked, group) {
        Tick::All => None,
        Tick::None | Tick::Some => pick_choices(group, |_| true),
    }
}

/// The grey of a ticked checkbox, dark enough to keep its white tick legible in both appearances.
fn checkbox_fill() -> Color32 {
    if theme::is_light() {
        Color32::from_gray(118)
    } else {
        Color32::from_rgb(88, 98, 104)
    }
}

/// A checkbox: an outline, a grey square with a tick, or one with a dash for some.
fn paint_checkbox(painter: &egui::Painter, rect: Rect, tick: Tick) {
    if tick == Tick::None {
        painter.rect_stroke(
            rect,
            3.0,
            Stroke::new(1.2, theme::with_alpha(theme::text_secondary(), 0.7)),
            StrokeKind::Inside,
        );
        return;
    }
    painter.rect_filled(rect, 3.0, checkbox_fill());
    let at = |x: f32, y: f32| rect.min + vec2(rect.width() * x, rect.height() * y);
    let stroke = Stroke::new(1.6, Color32::WHITE);
    if tick == Tick::All {
        painter.line_segment([at(0.24, 0.52), at(0.43, 0.72)], stroke);
        painter.line_segment([at(0.43, 0.72), at(0.78, 0.30)], stroke);
    } else {
        painter.line_segment([at(0.25, 0.5), at(0.75, 0.5)], stroke);
    }
}

// MARK: Event rows

/// Size of a prediction market's artwork beside its title or choice.
/// The recent markets sit in this many columns, this far apart.
const RECENT_COLUMNS: f32 = 4.0;
const RECENT_GAP: f32 = 8.0;
const EVENT_ART_SIZE: f32 = 24.0;
const EVENT_PAD: f32 = 10.0;
const EVENT_CHEVRON: f32 = 12.0;
const EVENT_GAP: f32 = 8.0;
const CHECKBOX_SIZE: f32 = 14.0;
/// Where an event's own content starts, and how much further in its choices sit.
const EVENT_CONTENT: f32 = EVENT_PAD + EVENT_CHEVRON + EVENT_GAP;
const CHOICE_INDENT: f32 = 22.0;

/// What an event header was asked to do this frame.
struct EventHeader {
    /// The header, outside its checkbox, was clicked: show or hide the choices.
    clicked: bool,
    /// The checkbox was clicked.
    tick_clicked: bool,
    /// Where the header ends, for the branch under it.
    bottom: f32,
}

/// An event's checkbox (when it has several choices), artwork, title and choice count, with a
/// chevron for whether its choices show.
fn event_header(
    ui: &mut Ui,
    group: &EventGroup,
    open: bool,
    tick: Option<Tick>,
    width: f32,
) -> EventHeader {
    let checkbox_room = if tick.is_some() {
        CHECKBOX_SIZE + EVENT_GAP
    } else {
        0.0
    };
    let art_left = EVENT_CONTENT + checkbox_room;
    let text_left = art_left + EVENT_ART_SIZE + EVENT_GAP;
    let text_width = width - text_left - EVENT_PAD;
    let choices = group.results.len();
    let title = galley_rows(
        ui,
        &group.title,
        size::CALLOUT,
        SEMIBOLD,
        theme::text_primary(),
        text_width,
        2,
    );
    let detail = galley(
        ui,
        &format!(
            "{choices} choice{} · {}",
            if choices == 1 { "" } else { "s" },
            if open { "Hide Choices" } else { "Show Choices" },
        ),
        size::CAPTION,
        400.0,
        theme::text_secondary(),
        text_width,
    );
    let text_height = title.size().y + 2.0 + detail.size().y;
    ui.add_space(6.0);
    let (rect, response) = ui.allocate_exact_size(
        vec2(width, text_height.max(EVENT_ART_SIZE) + 8.0),
        Sense::click(),
    );
    let response =
        crate::ui::accessibility::label(response, format!("{}, {choices} choices", group.title))
            .on_hover_cursor(egui::CursorIcon::PointingHand);
    // Laid over the header, so a click on the box ticks the choices without also folding them.
    let checkbox = tick.map(|tick| {
        let rect = Rect::from_center_size(
            pos2(
                rect.left() + EVENT_CONTENT + CHECKBOX_SIZE / 2.0,
                rect.center().y,
            ),
            Vec2::splat(CHECKBOX_SIZE),
        );
        let response = ui
            .interact(rect.expand(4.0), response.id.with("tick"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let label = format!("Select all choices in {}", group.title);
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Checkbox,
                true,
                tick == Tick::All,
                label.clone(),
            )
        });
        (rect, tick, response)
    });
    let outcome = EventHeader {
        clicked: response.clicked(),
        tick_clicked: checkbox
            .as_ref()
            .is_some_and(|(_, _, response)| response.clicked()),
        bottom: rect.bottom(),
    };
    if !ui.is_rect_visible(rect) {
        return outcome;
    }
    let painter = ui.painter();
    icons::paint(
        painter,
        Rect::from_center_size(
            pos2(
                rect.left() + EVENT_PAD + EVENT_CHEVRON / 2.0,
                rect.center().y,
            ),
            Vec2::splat(EVENT_CHEVRON),
        ),
        if open {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        },
        theme::text_secondary(),
    );
    if let Some((checkbox, tick, _)) = checkbox {
        paint_checkbox(painter, checkbox, tick);
    }
    if let Some(first) = group.results.first() {
        let art = Rect::from_center_size(
            pos2(
                rect.left() + art_left + EVENT_ART_SIZE / 2.0,
                rect.center().y,
            ),
            Vec2::splat(EVENT_ART_SIZE),
        );
        let image = first
            .event
            .as_ref()
            .and_then(|event| event.image_url.as_deref());
        prediction_icon(ui, art, &first.symbol, image);
    }
    let painter = ui.painter();
    let top = rect.center().y - text_height / 2.0;
    painter.galley(
        pos2(rect.left() + text_left, top),
        title.clone(),
        theme::text_primary(),
    );
    painter.galley(
        pos2(rect.left() + text_left, top + title.size().y + 2.0),
        detail,
        theme::text_secondary(),
    );
    outcome
}

/// How a choice row shows it is chosen.
#[derive(Clone, Copy)]
enum Mark {
    /// A checkbox, for choices charted together with their siblings.
    Tick(bool),
    /// A highlight and a check, for a lone choice.
    Pick(bool),
}

/// One choice of an event, hung under its header: checkbox or pick mark, artwork, label over up
/// to two lines, and YES probability.
fn choice_row(ui: &mut Ui, result: &SearchResult, mark: Mark, width: f32) -> egui::Response {
    const CHECK: f32 = 14.0;
    let left = EVENT_CONTENT + CHOICE_INDENT;
    let art_left = match mark {
        Mark::Tick(_) => left + CHECKBOX_SIZE + EVENT_GAP,
        Mark::Pick(_) => left,
    };
    let text_left = art_left + EVENT_ART_SIZE + 10.0;
    let price = result.price.map(|price| {
        ui.painter().layout_no_wrap(
            crate::format::headline(price, true),
            FontId::monospace(size::CALLOUT - 0.5),
            theme::text_secondary(),
        )
    });
    let price_width = price.as_ref().map_or(0.0, |price| price.size().x + 10.0);
    let trailing = match mark {
        Mark::Pick(_) => CHECK + 10.0,
        Mark::Tick(_) => 0.0,
    };
    let label = galley_rows(
        ui,
        &result.symbol,
        size::BODY,
        MEDIUM,
        theme::text_primary(),
        width - text_left - price_width - trailing - EVENT_PAD,
        2,
    );
    let height = (label.size().y + 12.0).max(RESULT_ROW_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let chosen = matches!(mark, Mark::Tick(true) | Mark::Pick(true));
    let name = result.symbol.clone();
    response.widget_info(|| match mark {
        Mark::Tick(ticked) => {
            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, ticked, name.clone())
        }
        Mark::Pick(_) => egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name.clone()),
    });
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let row = Rect::from_min_max(
        pos2(rect.left() + left - 6.0, rect.top()),
        rect.right_bottom(),
    );
    let painter = ui.painter();
    if matches!(mark, Mark::Pick(true)) {
        painter.rect_filled(row, 8.0, theme::selected_tint(false));
        painter.rect_stroke(row, 8.0, theme::selected_stroke(), StrokeKind::Inside);
    } else if response.hovered() {
        painter.rect_filled(row, 8.0, theme::ink(15));
    } else if chosen {
        painter.rect_filled(row, 8.0, theme::ink(10));
    }
    if let Mark::Tick(ticked) = mark {
        paint_checkbox(
            painter,
            Rect::from_center_size(
                pos2(rect.left() + left + CHECKBOX_SIZE / 2.0, rect.center().y),
                Vec2::splat(CHECKBOX_SIZE),
            ),
            if ticked { Tick::All } else { Tick::None },
        );
    }
    let art = Rect::from_center_size(
        pos2(
            rect.left() + art_left + EVENT_ART_SIZE / 2.0,
            rect.center().y,
        ),
        Vec2::splat(EVENT_ART_SIZE),
    );
    let image = result
        .event
        .as_ref()
        .and_then(|event| event.image_url.as_deref());
    prediction_icon(ui, art, &result.symbol, image);
    let painter = ui.painter();
    let mut right = rect.right() - EVENT_PAD;
    if matches!(mark, Mark::Pick(true)) {
        icons::paint(
            painter,
            Rect::from_center_size(
                pos2(right - CHECK / 2.0, rect.center().y),
                Vec2::splat(CHECK),
            ),
            Icon::CheckCircleFill,
            theme::text_primary(),
        );
    }
    right -= trailing;
    if let Some(price) = price {
        right -= price.size().x;
        painter.galley(
            pos2(right, rect.center().y - price.size().y / 2.0),
            price,
            theme::text_secondary(),
        );
    }
    painter.galley(
        pos2(
            rect.left() + text_left,
            rect.center().y - label.size().y / 2.0,
        ),
        label,
        theme::text_primary(),
    );
    response
}

/// Results grouped by source in a quiet rounded well; returns a double-clicked result.
fn result_list(ui: &mut Ui, session: &mut SearchSession) -> Option<SearchResult> {
    let SearchSession {
        results, selected, ..
    } = session;
    source_groups(ui, results, selected)
}

/// The grouped result well over any `(source, hits)` sections.
pub(crate) fn source_groups(
    ui: &mut Ui,
    results: &[(DataSource, Vec<SearchResult>)],
    selected: &mut Option<SearchResult>,
) -> Option<SearchResult> {
    let mut commit = None;
    let height = ui.available_height();
    egui::Frame::new()
        .fill(theme::quaternary(0.25))
        .corner_radius(10.0)
        .stroke(Stroke::new(1.0, theme::ink(18)))
        .inner_margin(egui::Margin::symmetric(6, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(height - 10.0)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let width = ui.available_width();
                    let sections = results.len();
                    for (index, (source, list)) in results.iter().enumerate() {
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.add_space(4.0);
                            ui.spacing_mut().item_spacing.x = 6.0;
                            source_logo::show(ui, *source, 14.0);
                            ui.label(text(
                                source.display_name(),
                                size::CAPTION + 0.5,
                                SEMIBOLD,
                                theme::text_secondary(),
                            ));
                            result_count_badge(ui, list.len());
                        });
                        ui.add_space(4.0);
                        for result in list {
                            let is_selected = selected
                                .as_ref()
                                .is_some_and(|picked| same_market(picked, result));
                            let response = result_row(ui, result, is_selected, false, width);
                            ui.add_space(2.0);
                            if response.double_clicked() {
                                commit = Some(result.clone());
                            } else if response.clicked() {
                                *selected = Some(result.clone());
                            }
                        }
                        if index + 1 < sections {
                            ui.add_space(4.0);
                            let (line, _) =
                                ui.allocate_exact_size(vec2(width, 1.0), Sense::hover());
                            ui.painter().hline(
                                line.shrink2(vec2(4.0, 0.0)).x_range(),
                                line.center().y,
                                Stroke::new(1.0, theme::separator()),
                            );
                        }
                    }
                    ui.add_space(6.0);
                });
        });
    commit
}

/// A compact, deliberately wide capsule for a provider's result count.
fn result_count_badge(ui: &mut Ui, count: usize) {
    let label = galley(
        ui,
        &count.to_string(),
        size::CAPTION,
        SEMIBOLD,
        theme::text_secondary(),
        f32::INFINITY,
    );
    let width = result_count_width(label.size().x);
    let (rect, _) = ui.allocate_exact_size(vec2(width, RESULT_COUNT_HEIGHT), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, RESULT_COUNT_HEIGHT / 2.0, theme::quaternary(0.9));
    painter.rect_stroke(
        rect,
        RESULT_COUNT_HEIGHT / 2.0,
        Stroke::new(1.0, theme::ink(20)),
        StrokeKind::Inside,
    );
    painter.galley(
        rect.center() - label.size() / 2.0,
        label,
        theme::text_secondary(),
    );
}

fn result_count_width(label_width: f32) -> f32 {
    (label_width + 16.0).max(RESULT_COUNT_MIN_WIDTH)
}

/// `ContentUnavailableView.search`: a large magnifier, a title and a hint, centred.
pub(crate) fn no_results(ui: &mut Ui, title: &str, detail: &str) {
    let rect = ui.available_rect_before_wrap();
    let painter = ui.painter();
    let title = galley(
        ui,
        title,
        15.0,
        BOLD,
        theme::text_primary(),
        rect.width() - 40.0,
    );
    let detail = galley(
        ui,
        detail,
        size::BODY,
        400.0,
        theme::text_secondary(),
        rect.width() - 40.0,
    );
    let total = 40.0 + 14.0 + title.size().y + 6.0 + detail.size().y;
    let mut y = rect.center().y - total / 2.0 - 10.0;
    icons::paint(
        painter,
        Rect::from_center_size(pos2(rect.center().x, y + 20.0), Vec2::splat(40.0)),
        Icon::Magnifier,
        theme::text_secondary(),
    );
    y += 54.0;
    painter.galley(
        pos2(rect.center().x - title.size().x / 2.0, y),
        title.clone(),
        theme::text_primary(),
    );
    y += title.size().y + 6.0;
    painter.galley(
        pos2(rect.center().x - detail.size().x / 2.0, y),
        detail,
        theme::text_secondary(),
    );
}

/// A selectable option as a card: icon badge, title, description and a radio mark.
fn choice_card(
    ui: &mut Ui,
    width: f32,
    icon: Icon,
    title: &str,
    subtitle: &str,
    selected: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 60.0), Sense::click());
    let response = crate::ui::accessibility::label(response, title)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter();
    let fill = if selected {
        theme::selected_tint(false)
    } else if response.hovered() {
        theme::ink(18)
    } else {
        theme::ink(8)
    };
    painter.rect_filled(rect, 10.0, fill);
    painter.rect_stroke(
        rect,
        10.0,
        if selected {
            Stroke::new(1.5, theme::selected_stroke().color)
        } else {
            Stroke::new(1.0, theme::ink(31))
        },
        StrokeKind::Inside,
    );
    let tint = if selected {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    let badge = Rect::from_min_size(
        pos2(rect.left() + 12.0, rect.center().y - 18.0),
        Vec2::splat(36.0),
    );
    painter.rect_filled(badge, 9.0, theme::with_alpha(tint, 0.14));
    icons::paint(
        painter,
        Rect::from_center_size(badge.center(), Vec2::splat(16.0)),
        icon,
        tint,
    );
    let radio = Rect::from_center_size(
        pos2(rect.right() - 12.0 - 7.0, rect.center().y),
        Vec2::splat(14.0),
    );
    if selected {
        icons::paint(painter, radio, Icon::CheckCircleFill, theme::text_primary());
    } else {
        icons::paint(
            painter,
            radio,
            Icon::Circle,
            theme::with_alpha(theme::text_secondary(), 0.6),
        );
    }
    let left = badge.right() + 12.0;
    let max_width = radio.left() - 8.0 - left;
    let title = galley(
        ui,
        title,
        size::SUBHEADLINE + 1.0,
        SEMIBOLD,
        theme::text_primary(),
        max_width,
    );
    let subtitle = galley(
        ui,
        subtitle,
        size::CAPTION + 1.0,
        400.0,
        theme::text_secondary(),
        max_width,
    );
    let total = title.size().y + 2.0 + subtitle.size().y;
    let top = rect.center().y - total / 2.0;
    painter.galley(pos2(left, top), title.clone(), theme::text_primary());
    painter.galley(
        pos2(left, top + title.size().y + 2.0),
        subtitle,
        theme::text_secondary(),
    );
    response
}

/// A tinted callout with a warning glyph and an optional trailing button; true when it is clicked.
fn notice_card(ui: &mut Ui, tint: Color32, title: &str, action: Option<&str>) -> bool {
    let mut clicked = false;
    egui::Frame::new()
        .fill(theme::with_alpha(tint, 0.08))
        .corner_radius(10.0)
        .stroke(Stroke::new(1.0, theme::with_alpha(tint, 0.25)))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                icons::inline(ui, Icon::TriangleFill, 14.0, tint);
                ui.add(
                    egui::Label::new(text(
                        title,
                        size::SUBHEADLINE + 1.0,
                        SEMIBOLD,
                        theme::text_primary(),
                    ))
                    .truncate(),
                );
                if let Some(action) = action {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let button = egui::Button::new(text(
                            action,
                            size::SUBHEADLINE + 0.5,
                            400.0,
                            theme::text_primary(),
                        ))
                        .fill(theme::ink(26))
                        .corner_radius(5.0)
                        .min_size(vec2(0.0, 20.0));
                        clicked = ui.add(button).clicked();
                    });
                }
            });
        });
    ui.add_space(4.0);
    clicked
}

/// What the footer calls a pick: the event for several ticked choices, else the market.
fn banner_label(result: &SearchResult) -> &str {
    match &result.event {
        Some(event) if event.series.len() > 1 && !event.title.is_empty() => &event.title,
        _ => &result.symbol,
    }
}

/// The confirmation strip in the footer, showing what Add will commit.
pub(crate) fn selected_banner(ui: &mut Ui, result: &SearchResult, max_width: f32) {
    egui::Frame::new()
        .fill(theme::selected_tint(false))
        .corner_radius(10.0)
        .stroke(theme::selected_stroke())
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_max_width(max_width - 20.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let (icon, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
                let artwork = result
                    .event
                    .as_ref()
                    .and_then(|event| event.image_url.as_deref());
                if let Some(url) = artwork {
                    prediction_icon(ui, icon, &result.symbol, Some(url));
                } else if result.source.is_prediction_market() {
                    source_logo::paint(ui, result.source, icon);
                } else {
                    result_icon(ui, icon, result);
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(text(
                        "Selected",
                        size::CAPTION,
                        400.0,
                        theme::text_secondary(),
                    ));
                    ui.add(
                        egui::Label::new(text(
                            banner_label(result),
                            size::CALLOUT + 0.5,
                            SEMIBOLD,
                            theme::text_primary(),
                        ))
                        .truncate(),
                    );
                });
                if let Some(price) = result.price {
                    let price = if result.source.is_prediction_market() {
                        crate::format::headline(price, true)
                    } else {
                        format!("${}", crate::format::price(price))
                    };
                    ui.label(
                        RichText::new(price)
                            .font(FontId::monospace(size::CAPTION + 0.5))
                            .color(theme::text_secondary()),
                    );
                }
            });
        });
}

/// A large push button: the prominent one is accent-filled, the other a quiet fill.
pub(crate) fn sheet_button(
    ui: &mut Ui,
    label: &str,
    prominent: bool,
    enabled: bool,
) -> egui::Response {
    let galley = galley(
        ui,
        label,
        size::BODY,
        if prominent { MEDIUM } else { 400.0 },
        Color32::PLACEHOLDER,
        f32::INFINITY,
    );
    let width = (galley.size().x + 32.0).max(76.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, 28.0), sense);
    let response = crate::ui::accessibility::label(response, label);
    let pressed = response.is_pointer_button_down_on();
    let hovered = response.hovered() && enabled;
    let (fill, color) = match (prominent, enabled) {
        (true, true) => (
            theme::ink(if pressed {
                56
            } else if hovered {
                44
            } else {
                36
            }),
            theme::text_primary(),
        ),
        (true, false) => (theme::ink(14), theme::text_tertiary()),
        (false, _) => (
            theme::ink(if pressed {
                46
            } else if hovered {
                36
            } else {
                26
            }),
            theme::text_primary(),
        ),
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 7.0, fill);
    if !prominent || enabled {
        painter.rect_stroke(
            rect,
            7.0,
            if prominent {
                theme::selected_stroke()
            } else {
                Stroke::new(1.0, theme::ink(14))
            },
            StrokeKind::Inside,
        );
    }
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(symbol: &str, source: DataSource) -> SearchResult {
        SearchResult {
            symbol: symbol.into(),
            full_symbol: symbol.into(),
            source,
            price: None,
            event: None,
            base: None,
            quote: None,
        }
    }

    fn choice(event: &str, label: &str, price: Option<f64>) -> SearchResult {
        SearchResult {
            full_symbol: format!("{event}/{label}"),
            price,
            event: Some(crate::service::EventInfo {
                title: event.into(),
                image_url: None,
                series: Vec::new(),
            }),
            ..result(label, DataSource::Polymarket)
        }
    }

    fn titles(list: &EventList) -> Vec<&str> {
        list.groups
            .iter()
            .map(|group| group.title.as_str())
            .collect()
    }

    fn labels(group: &EventGroup) -> Vec<&str> {
        group
            .results
            .iter()
            .map(|result| result.symbol.as_str())
            .collect()
    }

    #[test]
    fn choices_group_under_their_event_in_search_order() {
        let results = [
            choice("Fed", "Cut", Some(0.2)),
            choice("Election", "Yes", Some(0.5)),
            choice("Fed", "Hold", Some(0.7)),
        ];
        let mut list = EventList::default();
        list.set(results.iter());
        assert_eq!(titles(&list), ["Fed", "Election"]);
        assert_eq!(labels(&list.groups[0]), ["Hold", "Cut"]);
    }

    #[test]
    fn choices_sort_by_probability_with_unpriced_last_and_ties_kept() {
        let results = [
            choice("E", "none", None),
            choice("E", "tie a", Some(0.4)),
            choice("E", "top", Some(0.9)),
            choice("E", "tie b", Some(0.4)),
        ];
        let groups = group_by_event(results.iter());
        assert_eq!(labels(&groups[0]), ["top", "tie a", "tie b", "none"]);
    }

    #[test]
    fn a_result_without_an_event_stands_under_its_own_label() {
        let results = [result("Will it rain?", DataSource::Kalshi)];
        let groups = group_by_event(results.iter());
        assert_eq!(groups[0].title, "Will it rain?");
        assert_eq!(groups[0].results.len(), 1);
    }

    #[test]
    fn events_start_collapsed_unless_there_is_only_one() {
        let mut list = EventList::default();
        list.set([choice("A", "x", None), choice("B", "y", None)].iter());
        assert!(list.expanded.is_empty());
        list.set([choice("A", "x", None)].iter());
        assert!(list.expanded.contains("A"));
    }

    #[test]
    fn refreshed_prices_keep_what_the_user_opened() {
        let mut list = EventList::default();
        list.set([choice("A", "x", Some(0.1)), choice("B", "y", None)].iter());
        list.expanded.insert("B".into());
        list.set([choice("A", "x", Some(0.3)), choice("B", "y", None)].iter());
        assert!(list.expanded.contains("B"));
        list.set([choice("C", "z", None), choice("B", "y", None)].iter());
        assert!(list.expanded.is_empty());
    }

    fn event(choices: &[(&str, Option<f64>)]) -> EventGroup {
        let results: Vec<SearchResult> = choices
            .iter()
            .map(|(label, price)| choice("Fed", label, *price))
            .collect();
        group_by_event(results.iter()).remove(0)
    }

    fn ticked_labels(picked: &SearchResult) -> Vec<&str> {
        picked
            .event
            .as_ref()
            .expect("event")
            .series
            .iter()
            .map(|choice| choice.label.as_str())
            .collect()
    }

    #[test]
    fn ticking_choices_builds_one_pick_led_by_the_first_in_event_order() {
        let group = event(&[("Cut", Some(0.2)), ("Hold", Some(0.7)), ("Hike", Some(0.1))]);
        // The event lists Hold, Cut, Hike; ticking Hike then Cut still leads with Cut.
        let picked = toggle_choice(None, &group, "Fed/Hike");
        let picked = toggle_choice(picked.as_ref(), &group, "Fed/Cut").expect("pick");
        assert_eq!(picked.symbol, "Cut");
        assert_eq!(ticked_labels(&picked), ["Cut", "Hike"]);
        assert_eq!(group_tick(Some(&picked), &group), Tick::Some);
    }

    #[test]
    fn unticking_the_last_choice_clears_the_pick() {
        let group = event(&[("Cut", Some(0.2)), ("Hold", Some(0.7))]);
        let picked = toggle_choice(None, &group, "Fed/Cut");
        assert!(toggle_choice(picked.as_ref(), &group, "Fed/Cut").is_none());
    }

    #[test]
    fn the_event_checkbox_ticks_everything_then_clears() {
        let group = event(&[("Cut", Some(0.2)), ("Hold", Some(0.7)), ("Hike", None)]);
        let some = toggle_choice(None, &group, "Fed/Hold");
        let all = toggle_group(some.as_ref(), &group).expect("all");
        assert_eq!(ticked_labels(&all), ["Hold", "Cut", "Hike"]);
        assert_eq!(group_tick(Some(&all), &group), Tick::All);
        assert!(toggle_group(Some(&all), &group).is_none());
        assert_eq!(group_tick(None, &group), Tick::None);
    }

    #[test]
    fn ticking_in_another_event_drops_the_first_events_ticks() {
        let fed = event(&[("Cut", Some(0.2)), ("Hold", Some(0.7))]);
        let other = group_by_event(
            [
                choice("Vote", "Yes", Some(0.5)),
                choice("Vote", "No", Some(0.5)),
            ]
            .iter(),
        )
        .remove(0);
        let in_fed = toggle_choice(None, &fed, "Fed/Cut");
        let in_vote = toggle_choice(in_fed.as_ref(), &other, "Vote/No").expect("pick");
        assert_eq!(ticked_labels(&in_vote), ["No"]);
        assert_eq!(group_tick(Some(&in_vote), &fed), Tick::None);
    }

    #[test]
    fn the_footer_names_the_event_only_for_several_choices() {
        let group = event(&[("Cut", Some(0.2)), ("Hold", Some(0.7))]);
        let one = toggle_choice(None, &group, "Fed/Cut").expect("one");
        assert_eq!(banner_label(&one), "Cut");
        let both = toggle_group(None, &group).expect("both");
        assert_eq!(banner_label(&both), "Fed");
    }

    #[test]
    fn return_picks_the_likeliest_choice_of_the_first_event() {
        let mut session = SearchSession::new();
        session.set_results(vec![(
            DataSource::Polymarket,
            vec![
                choice("A", "low", Some(0.1)),
                choice("A", "high", Some(0.8)),
            ],
        )]);
        assert_eq!(
            session.first_visible(true).map(|r| r.symbol.as_str()),
            Some("high")
        );
        assert_eq!(
            session.first_visible(false).map(|r| r.symbol.as_str()),
            Some("low")
        );
    }

    #[test]
    fn base_symbol_drops_pair_and_company() {
        assert_eq!(base_symbol(&result("btc/usdt", DataSource::Binance)), "BTC");
        assert_eq!(
            base_symbol(&result("AAPL — Apple Inc.", DataSource::Alpaca)),
            "AAPL"
        );
    }

    #[test]
    fn dex_searches_before_coingecko_and_resolves_pair_artwork() {
        assert_eq!(
            CRYPTO_SOURCES,
            [
                DataSource::Binance,
                DataSource::Coinbase,
                DataSource::DexScreener,
                DataSource::CoinGecko,
            ]
        );
        assert!(supports_ticker_icon_lookup(DataSource::DexScreener));
    }

    #[test]
    fn result_count_badges_are_horizontal_capsules() {
        assert_eq!(result_count_width(0.0), RESULT_COUNT_MIN_WIDTH);
        assert!(result_count_width(8.0) > RESULT_COUNT_HEIGHT * 1.5);
        assert_eq!(result_count_width(40.0), 56.0);
    }

    #[test]
    fn stale_replies_are_dropped_and_selection_follows_results() {
        let mut session = SearchSession::new();
        session.query = "eth".into();
        session.selected = Some(result("BTCUSDT", DataSource::Binance));
        let (sender, receiver) = mpsc::channel();
        session.inbox = Some(receiver);
        sender
            .send((
                "btc".into(),
                vec![(
                    DataSource::Binance,
                    Ok(vec![result("BTCUSDT", DataSource::Binance)]),
                )],
            ))
            .unwrap();
        session.receive();
        assert!(
            session.results.is_empty(),
            "a reply to an old query is ignored"
        );
        assert!(
            session.debounce.is_some(),
            "and the current query is asked again"
        );

        let (sender, receiver) = mpsc::channel();
        session.inbox = Some(receiver);
        sender
            .send((
                "eth".into(),
                vec![
                    (
                        DataSource::Binance,
                        Ok(vec![result("ETHUSDT", DataSource::Binance)]),
                    ),
                    (DataSource::Coinbase, Err("offline".into())),
                ],
            ))
            .unwrap();
        session.receive();
        assert_eq!(
            session.results.len(),
            1,
            "failed and empty sources are left out"
        );
        assert!(
            session.selected.is_none(),
            "a selection missing from the results is dropped"
        );
        assert!(
            session.error.is_none(),
            "a partial failure with results is not an error"
        );
    }

    #[test]
    fn recents_round_trip_in_swift_shape() {
        let market = RecentMarket {
            symbol: "BTC/USDT".into(),
            full_symbol: "BTCUSDT".into(),
            source: DataSource::Binance,
        };
        let json = serde_json::to_string(&market).unwrap();
        assert!(json.contains("\"fullSymbol\":\"BTCUSDT\""));
        let back: RecentMarket = serde_json::from_str(&json).unwrap();
        assert_eq!(back.result().full_symbol, "BTCUSDT");
    }
}
