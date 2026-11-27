//! The symbol picker: a large modal opened from the symbol button at the left of the toolbar, with
//! ⌘K, from the toolbar's Add Chart button and from a watchlist's plus. It searches crypto and
//! stocks together (or either alone, or one exchange), prediction markets, and lists
//! CoinMarketCap's indices and the user's portfolio views. It lists the dollar markets of an asset
//! on every exchange first, shows the highlighted market's price on the right, and replaces the
//! focused chart's market with the one picked. A pick is reported as an [`Output`]; the workspace
//! applies it.
//!
//! It opens searching for the market the focused chart shows, so a different exchange or quote
//! of the same asset is one keystroke away. See `docs/symbol-picker.md`.

pub mod columns;
pub mod dashboards;
mod describe;
mod detail;
mod footer;
mod list;
mod lookup;
mod names;
mod notice;
mod offer;
pub mod predictions;
mod results;
pub mod search;
mod tabs;
pub mod watchlists;

use super::picker_kit::recents::{self, RecentMarket};
use crate::i18n::{tr, tr_n};
use crate::model::portfolio::Portfolio;
use crate::model::portfolio::card::PortfolioCard;
use crate::model::watchlist::quote::WatchlistQuote;
use crate::model::{DataSource, InstrumentId};
use crate::service::{self, SearchResult};
use crate::store::SharedStore;
use crate::ui::icons::Icon;
use crate::ui::kit;
use crate::ui::theme;
use dashboards::{Tile, TilePick};
use detail::PreviewChart;
use detail::action::Actions;
use eframe::egui::{self, Color32, Id, Key, Modifiers, Rect, Stroke, Ui, Vec2, pos2, vec2};
use footer::Footer;
use lookup::Lookup;
use names::Names;
use notice::{Notice, SettingsPage};
use offer::{Offer, Overlay};
use predictions::EventHead;
use results::Rows;
use search::{Backend, Listing, Scope, SymbolSearch};
use tabs::{Filter, FilterMenu};
use watchlists::WatchlistGroup;
#[cfg(test)]
use {
    footer::market_label,
    notice::{NOTICE_HEIGHT, notice_button_rect},
    tabs::{FILTER_COMPACT, FILTER_GAP, TabFit, fit_tabs, tab_room},
};

const SIZE: Vec2 = vec2(1000.0, 660.0);
/// The narrowest the picker gets: still wide enough for every tab plus the pull-down's icon.
const MIN_WIDTH: f32 = 560.0;
const PADDING: f32 = 20.0;
const TABS: f32 = 52.0;
const FOOTER: f32 = 52.0;
/// The detail panel's share of the sheet, and the narrowest and widest it gets.
const DETAIL_SHARE: f32 = 0.344;
const DETAIL_MIN: f32 = 300.0;
const DETAIL_MAX: f32 = 344.0;
/// The detail panel is dropped when the sheet is narrower than this, so the list keeps room.
const DETAIL_MIN_WIDTH: f32 = 760.0;
/// The search field asks for focus this many frames: the first only measures the modal.
const FOCUS_FRAMES: u8 = 3;

/// What the picker asks the workspace to do.
#[derive(Clone, Debug)]
pub enum Output {
    /// Show this market on the focused chart (or add a chart when none is focused). The picker
    /// has closed itself. A prediction market carries the choices ticked to chart together in
    /// its `event.series`.
    Replace(SearchResult),
    /// Add this market as a chart of its own. The picker has closed itself.
    Add(SearchResult),
    /// Draw this market on the focused chart, on top of its own. The picker has closed itself,
    /// unless it was opened to compare (then it stays, so several can be added in a row).
    Overlay(SearchResult),
    /// Add this portfolio view as a card of the grid. The picker has closed itself.
    AddCard(PortfolioCard),
    /// Add this market to the watchlist the picker was opened for. The picker stays open.
    Watchlist(SearchResult),
    /// Add this portfolio view to the watchlist the picker was opened for. The picker stays open.
    WatchlistCard(PortfolioCard),
    /// Open Settings on the Alpaca page; the picker stays open.
    OpenAlpacaSettings,
    /// Open Settings on the Oanda page; the picker stays open.
    OpenOandaSettings,
}

/// What the picker was opened for.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PickMode {
    /// Change the focused chart's market (⌘K, the symbol button).
    #[default]
    Chart,
    /// Add a chart (the Add Chart button, the new-tab page's search): a pick never replaces one.
    Add,
    /// Compare markets on the focused chart (the Compare button): a pick draws it over the chart.
    Compare,
    /// Add markets to a watchlist (its plus): a pick follows the market, and the picker stays open
    /// for the next. [`SymbolPicker::open_for_watchlist`] names the list.
    Watchlist,
}

/// What a pick does with the market.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PickKind {
    Replace,
    Add,
    Overlay,
    Watchlist,
}

/// What a pick picks: a market, or a view of the user's portfolios.
#[derive(Clone, Debug)]
enum Pick {
    Market(SearchResult),
    Card(PortfolioCard),
}

impl Pick {
    /// The identity a watchlist keeps the pick under.
    fn instrument(&self) -> InstrumentId {
        match self {
            Self::Market(result) => InstrumentId::new(result.source, &result.full_symbol),
            Self::Card(card) => card.instrument_id(),
        }
    }
}

/// A row of the list.
#[derive(Clone, Debug, PartialEq)]
enum Item {
    /// A market, a lone prediction choice or a CoinMarketCap index.
    Market(SearchResult),
    /// A prediction event with several choices, folded or open.
    Event(EventHead),
    /// One choice of an open prediction event.
    Choice(SearchResult),
    /// An index or a portfolio view on the Dashboards tab.
    Tile(Tile),
}

impl Item {
    /// The market the row stands for, when it has one to price and show.
    fn market(&self) -> Option<&SearchResult> {
        match self {
            Self::Market(result) | Self::Choice(result) => Some(result),
            Self::Event(head) => Some(&head.lead),
            Self::Tile(tile) => match &tile.pick {
                TilePick::Gauge(result) => Some(result),
                TilePick::Card(_) => None,
            },
        }
    }
}

/// What the workspace tells the picker each frame.
pub struct Env<'a> {
    /// The markets already on the tab's charts, which the list marks with a check.
    pub on_tab: &'a [InstrumentId],
    /// The price of [`SymbolPicker::highlighted`], when the quote feed has one.
    pub quote: Option<&'a WatchlistQuote>,
    /// The focused chart's market, which a pick replaces; `None` when a pick adds a chart.
    pub replacing: Option<&'a str>,
    /// The focused chart's own market and the markets already compared on it: a pick of one of
    /// these cannot be drawn again, and the list marks them in compare mode.
    pub overlaid: &'a [InstrumentId],
    /// Whether the focused chart can have a market drawn over it.
    pub can_overlay: bool,
    /// The portfolio cards already on the tab, which the Dashboards tab marks with a check.
    pub cards_on_tab: &'a [PortfolioCard],
    /// What the watchlist being added to already holds, in [`PickMode::Watchlist`]: its rows are
    /// checked, and a pick of one adds nothing. Cards are held by their `instrument_id`.
    pub in_list: &'a [InstrumentId],
}

/// The quote-demand owner of the picker in tab `tab`: clear of the sidebar's (the tab id), the
/// alert sheet's (`1 << 62`) and the alert runtime's.
pub fn quote_owner(tab: u64) -> u64 {
    (1 << 61) + tab
}

pub struct SymbolPicker {
    store: SharedStore,
    backend: Backend,
    open: bool,
    mode: PickMode,
    /// The watchlist a pick is added to, in [`PickMode::Watchlist`].
    list_name: String,
    query: String,
    scope: Scope,
    venue: Option<DataSource>,
    /// The portfolio the Dashboards tab's cards cover; `None` covers them all.
    card_scope: Option<u64>,
    search: SymbolSearch,
    caches: Caches,
    /// The prediction events listed, and which show their choices.
    events: predictions::Events,
    /// The choices ticked to chart together, as the pick that charts them.
    ticked: Option<SearchResult>,
    trending: predictions::Trending,
    portfolios: Vec<Portfolio>,
    /// What the last pick added to a watchlist, for the footer to acknowledge.
    added: Option<String>,
    recents: Vec<RecentMarket>,
    /// The listed row the keyboard (or the pointer) is on.
    highlight: usize,
    /// The market that row held in the last frame, for the quote feed and the detail panel.
    highlighted: Option<Item>,
    /// The highlight moved by key: scroll its row into view.
    reveal: bool,
    focus_frames: u8,
    select_all: bool,
    /// The user's watchlists, fed by the workspace while the Watchlists tab shows.
    watchlists: Vec<WatchlistGroup>,
    pick_needs_credentials: Option<SettingsPage>,
    /// The Settings page a notice's button asked to open.
    settings_request: Option<SettingsPage>,
}

/// What survives closing the picker: reopening keeps what was read and asks again for what
/// failed.
struct Caches {
    /// What the markets' assets are called.
    names: Names,
    /// The preview's chart of the highlighted market.
    chart: PreviewChart,
}

impl Caches {
    fn new(backend: Backend) -> Self {
        Self {
            names: Names::default(),
            chart: PreviewChart::new(backend.candles),
        }
    }

    /// The same caches for a freshly opened picker.
    fn reopened(&self) -> Self {
        Self {
            names: self.names.reopened(),
            chart: self.chart.reopened(),
        }
    }
}

impl SymbolPicker {
    pub fn new(store: SharedStore) -> Self {
        Self::with_backend(store, Backend::LIVE)
    }

    fn with_backend(store: SharedStore, backend: Backend) -> Self {
        Self::fresh(store, backend, Caches::new(backend))
    }

    /// A closed picker with nothing typed, over `caches`.
    fn fresh(store: SharedStore, backend: Backend, caches: Caches) -> Self {
        Self {
            store,
            backend,
            open: false,
            mode: PickMode::Chart,
            list_name: String::new(),
            query: String::new(),
            scope: Scope::All,
            venue: None,
            card_scope: None,
            search: SymbolSearch::new(backend),
            caches,
            events: predictions::Events::default(),
            ticked: None,
            trending: predictions::Trending::new(backend.trending),
            portfolios: Vec::new(),
            added: None,
            recents: Vec::new(),
            highlight: 0,
            highlighted: None,
            reveal: false,
            focus_frames: 0,
            select_all: false,
            watchlists: Vec::new(),
            pick_needs_credentials: None,
            settings_request: None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opens a fresh picker searching for `current`, the market the focused chart shows, with the
    /// text selected so typing replaces it.
    pub fn open(&mut self, current: &str) {
        self.open_for(current, PickMode::Chart);
    }

    /// Opens the picker to add a chart: nothing is searched for at first (recent and popular
    /// markets show), and a pick adds a chart rather than replacing one.
    pub fn open_add(&mut self) {
        self.open_for("", PickMode::Add);
    }

    /// Opens the picker to compare markets on the focused chart: nothing is searched for at first
    /// (recent and popular markets show), and a pick keeps the picker open.
    pub fn open_compare(&mut self) {
        self.open_for("", PickMode::Compare);
    }

    /// Opens the picker to add markets to the watchlist called `name`. A pick keeps the picker
    /// open so several can be added in a row.
    pub fn open_for_watchlist(&mut self, name: &str) {
        self.open_for("", PickMode::Watchlist);
        self.list_name = name.to_owned();
    }

    #[cfg(test)]
    pub fn mode(&self) -> PickMode {
        self.mode
    }

    fn open_for(&mut self, current: &str, mode: PickMode) {
        *self = Self::fresh(self.store.clone(), self.backend, self.caches.reopened());
        self.recents = self
            .store
            .lock()
            .ok()
            .map(|store| recents::load_recents(&store))
            .unwrap_or_default();
        self.query = current.to_owned();
        self.search.ask_now();
        self.select_all = !current.is_empty();
        self.focus_frames = FOCUS_FRAMES;
        self.mode = mode;
        self.open = true;
    }

    /// The workspace should hand over the user's watchlists (see [`Self::set_watchlists`]).
    pub fn wants_watchlists(&self) -> bool {
        self.open && self.scope == Scope::Watchlists
    }

    pub fn set_watchlists(&mut self, groups: Vec<WatchlistGroup>) {
        if self.watchlists != groups {
            self.watchlists = groups;
        }
    }

    /// The workspace should hand over the user's portfolios (see [`Self::set_portfolios`]).
    pub fn wants_portfolios(&self) -> bool {
        self.open && self.scope == Scope::Dashboards
    }

    pub fn set_portfolios(&mut self, portfolios: Vec<Portfolio>) {
        if self.portfolios != portfolios {
            self.portfolios = portfolios;
        }
    }

    /// What the search box holds.
    #[cfg(test)]
    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn close(&mut self) {
        self.open = false;
        self.search.forget();
        self.highlighted = None;
    }

    /// The market whose price the detail panel wants: the workspace holds a demand on it. An
    /// index or a portfolio view has none.
    pub fn highlighted(&self) -> Option<InstrumentId> {
        self.highlighted
            .as_ref()
            .and_then(Item::market)
            .filter(|result| result.source != DataSource::CoinMarketCap)
            .map(|result| InstrumentId::new(result.source, &result.full_symbol))
    }

    pub fn show(&mut self, ctx: &egui::Context, env: &Env) -> Option<Output> {
        if !self.open {
            return None;
        }
        let search_id = Id::new("symbol_picker_search");
        self.claim_keys(ctx, search_id);
        self.caches.names.poll(ctx, self.backend);
        let keys = Keys::read(ctx);

        let listing = self.listing(ctx);
        let rows = self.rows(&listing);
        let mut output = None;
        if keys.scope {
            self.set_scope(self.scope.next(self.mode));
        }
        if keys.down || keys.up {
            self.step(rows.items.len(), keys.down);
        }
        self.highlight = self.highlight.min(rows.items.len().saturating_sub(1));
        self.highlighted = rows.items.get(self.highlight).cloned();
        if keys.pick
            && let Some(item) = self.highlighted.clone()
        {
            output = self.use_item(&item, &keys, env);
        }

        let screen = ctx.content_rect().size();
        let size = vec2(
            SIZE.x.min(screen.x - 48.0).max(MIN_WIDTH),
            SIZE.y.min(screen.y - 48.0).max(400.0),
        );
        let related = self
            .highlighted
            .as_ref()
            .and_then(Item::market)
            .cloned()
            .and_then(|result| self.search.related(ctx, &result));
        let mut picked: Option<(Pick, PickKind)> = None;
        let modal = egui::Modal::new(Id::new("symbol_picker"))
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
                let full = Rect::from_min_size(ui.min_rect().min, size);
                picked = self.layout(
                    ui,
                    full,
                    search_id,
                    &listing,
                    &rows,
                    related.as_deref(),
                    env,
                );
            });
        // A click on the dimmed window outside the sheet closes it.
        let clicked_outside = modal.backdrop_response.clicked();

        if let Some((pick, kind)) = picked {
            output = output.or_else(|| self.commit(pick, kind, env));
        }
        if self.select_all && ctx.memory(|memory| memory.has_focus(search_id)) {
            select_all(ctx, search_id, &self.query);
            self.select_all = false;
        }
        if let Some(page) = std::mem::take(&mut self.settings_request) {
            output = Some(match page {
                SettingsPage::Alpaca => Output::OpenAlpacaSettings,
                SettingsPage::Oanda => Output::OpenOandaSettings,
            });
        }
        if keys.close || clicked_outside {
            self.close();
        }
        let stays_open = match output {
            Some(Output::Overlay(_)) => self.mode == PickMode::Compare,
            Some(Output::Watchlist(_) | Output::WatchlistCard(_)) => true,
            _ => false,
        };
        if matches!(
            output,
            Some(
                Output::Replace(_)
                    | Output::Add(_)
                    | Output::Overlay(_)
                    | Output::AddCard(_)
                    | Output::Watchlist(_)
                    | Output::WatchlistCard(_)
            )
        ) && !stays_open
        {
            self.close();
        }
        output
    }

    /// Keeps Tab and the arrows with the picker while the box has focus, instead of moving focus.
    fn claim_keys(&self, ctx: &egui::Context, id: Id) {
        ctx.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: false,
                    vertical_arrows: true,
                    escape: false,
                },
            );
        });
    }

    /// What the providers answered for the box, and (on the Predictions tab) the events that
    /// answer is grouped into. An empty box on that tab lists the trending ones instead.
    fn listing(&mut self, ctx: &egui::Context) -> Listing {
        let lookup = self.lookup();
        let nothing = |searching| Listing {
            results: Vec::new(),
            searching,
            errors: Vec::new(),
        };
        if self.scope == Scope::Predictions && lookup.is_empty() {
            // Only a provider that can rank its markets by activity has a trending list.
            let trending = lookup.venue.is_none_or(service::supports_trending);
            if trending {
                self.trending.refresh(ctx);
            }
            let hits: Vec<SearchResult> = if trending {
                self.trending.results().to_vec()
            } else {
                Vec::new()
            };
            self.events.set(hits.iter());
            self.keep_ticks_listed();
            return nothing(trending && self.trending.is_loading());
        }
        if !self.scope.searches() {
            return nothing(false);
        }
        let listing = self
            .search
            .listing(ctx, &lookup.query, self.scope, lookup.venue);
        if self.scope == Scope::Predictions {
            self.events.set(listing.results.iter());
            self.keep_ticks_listed();
        }
        listing
    }

    /// What the box asks for: its text, or a `VENUE:SYMBOL` read as a venue and a symbol.
    fn lookup(&self) -> Lookup {
        Lookup::of(&self.query, self.scope, self.venue)
    }

    /// Drops ticks whose event is no longer listed: they would chart something not on screen.
    fn keep_ticks_listed(&mut self) {
        if let Some(ticked) = &self.ticked
            && self
                .events
                .group(predictions::event_title(ticked))
                .is_none()
        {
            self.ticked = None;
        }
    }

    /// The rows in the list with the captions above groups of them: the answers to the query,
    /// the watchlists' markets, the events and tiles of the other tabs, or the recent (else
    /// popular) picks while the box is empty.
    fn rows(&self, listing: &Listing) -> Rows {
        let lookup = self.lookup();
        let in_scope = |result: &SearchResult| {
            self.scope.includes(result.source)
                && lookup.venue.is_none_or(|venue| venue == result.source)
        };
        match self.scope {
            Scope::Watchlists => {
                let mut rows = Rows::default();
                for group in &self.watchlists {
                    let before = rows.items.len();
                    rows.items.extend(
                        group
                            .markets
                            .iter()
                            .filter(|market| {
                                in_scope(market) && watchlists::matches(market, &lookup.query)
                            })
                            .cloned()
                            .map(Item::Market),
                    );
                    if rows.items.len() > before {
                        rows.heads.push((before, group.name.clone()));
                    }
                }
                return rows;
            }
            Scope::Dashboards => {
                let mut rows = Rows::default();
                let card_scope = self.card_scope_in_use();
                for group in dashboards::groups(&self.portfolios, card_scope, &self.query) {
                    rows.heads
                        .push((rows.items.len(), group.heading.to_owned()));
                    rows.items.extend(group.tiles.into_iter().map(Item::Tile));
                }
                if self.portfolios.is_empty() {
                    rows.note = Some(
                        tr!("Portfolio views appear here once you create a portfolio in the Portfolio tab.")
                            .to_owned(),
                    );
                }
                return rows;
            }
            Scope::Predictions => {
                let items = predictions::items(&self.events, self.ticked.as_ref());
                let head = lookup.is_empty().then(|| {
                    tr!(
                        "Trending on {provider}",
                        provider = predictions::TRENDING_PROVIDER.display_name()
                    )
                });
                return Rows::of_items(items, head.as_deref());
            }
            Scope::All | Scope::Stocks | Scope::Crypto | Scope::Forex => {}
        }
        if !lookup.is_empty() {
            let head = lookup.unsupported.as_ref().map(|prefix| {
                tr!(
                    "{prefix} isn't supported · matches from other venues",
                    prefix = prefix
                )
            });
            return Rows::of(listing.results.clone(), head.as_deref());
        }
        let recent: Vec<SearchResult> = self
            .recents
            .iter()
            .map(RecentMarket::result)
            .filter(in_scope)
            .collect();
        if recent.is_empty() {
            Rows::of(
                search::popular().into_iter().filter(in_scope).collect(),
                Some(tr!("Popular")),
            )
        } else {
            Rows::of(recent, Some(tr!("Recent")))
        }
    }

    /// The portfolio the cards cover: the chosen one while it still exists, else all of them.
    fn card_scope_in_use(&self) -> Option<u64> {
        self.card_scope
            .filter(|id| self.portfolios.iter().any(|portfolio| portfolio.id == *id))
    }

    fn step(&mut self, rows: usize, down: bool) {
        if rows == 0 {
            return;
        }
        self.highlight = if down {
            (self.highlight + 1) % rows
        } else {
            (self.highlight + rows - 1) % rows
        };
        self.reveal = true;
    }

    fn set_scope(&mut self, scope: Scope) {
        if self.scope == scope {
            return;
        }
        self.scope = scope;
        // A venue of another scope no longer applies, nor do ticks made in another list.
        if self.venue.is_some_and(|venue| !scope.includes(venue)) {
            self.venue = None;
        }
        self.ticked = None;
        self.highlight = 0;
        self.search.ask_now();
    }

    fn set_venue(&mut self, venue: Option<DataSource>) {
        if self.venue != venue {
            self.venue = venue;
            self.highlight = 0;
            self.search.ask_now();
        }
    }

    fn set_filter(&mut self, filter: Filter) {
        match filter {
            Filter::Venue(venue) => self.set_venue(venue),
            Filter::Portfolio(scope) => {
                self.card_scope = scope;
                self.highlight = 0;
            }
        }
    }

    /// What a pick in this mode does by default (a click, Return).
    fn default_kind(&self) -> PickKind {
        match self.mode {
            PickMode::Chart => PickKind::Replace,
            PickMode::Add => PickKind::Add,
            PickMode::Compare => PickKind::Overlay,
            PickMode::Watchlist => PickKind::Watchlist,
        }
    }

    /// What ⇧Return and a row's plus do: add a chart, or, for a watchlist, follow the market.
    fn add_kind(&self) -> PickKind {
        match self.mode {
            PickMode::Watchlist => PickKind::Watchlist,
            _ => PickKind::Add,
        }
    }

    /// The market a pick would replace: only a picker opened to change a chart's market has one.
    fn replacing<'a>(&self, env: &Env<'a>) -> Option<&'a str> {
        env.replacing.filter(|_| self.mode == PickMode::Chart)
    }

    /// What Return, ⇧Return or ⌥Return does on `item`: fold an event, or use what the row stands
    /// for.
    fn use_item(&mut self, item: &Item, keys: &Keys, env: &Env) -> Option<Output> {
        if let Item::Event(head) = item {
            self.events.fold(&head.title);
            return None;
        }
        let offer = Offer::of(self.mode, Some(item), env);
        let kind = if keys.add {
            self.add_kind()
        } else if keys.overlay && offer.overlay != Overlay::Unavailable {
            PickKind::Overlay
        } else {
            // A prediction market or an index is not drawn over a chart.
            self.default_kind()
        };
        // A market already drawn on the chart is not drawn twice.
        if kind == PickKind::Overlay
            && item
                .market()
                .is_some_and(|result| offer::overlaid(env, result))
        {
            return None;
        }
        let pick = self.pick_of(item)?;
        self.commit(pick, kind, env)
    }

    /// What using `item` picks; an event is only folded open.
    fn pick_of(&self, item: &Item) -> Option<Pick> {
        match item {
            Item::Event(_) => None,
            Item::Market(result) | Item::Choice(result) => {
                Some(Pick::Market(self.selection(result)))
            }
            Item::Tile(tile) => Some(Self::tile_pick(tile)),
        }
    }

    /// What Return takes for `result`: the choices ticked to chart together when it belongs to
    /// their event, else the choice alone.
    fn selection(&self, result: &SearchResult) -> SearchResult {
        match &self.ticked {
            Some(ticked)
                if predictions::ticked_count(Some(ticked)) >= 1
                    && predictions::event_title(ticked) == predictions::event_title(result) =>
            {
                ticked.clone()
            }
            _ => result.clone(),
        }
    }

    fn tile_pick(tile: &Tile) -> Pick {
        match &tile.pick {
            TilePick::Gauge(result) => Pick::Market(result.clone()),
            TilePick::Card(card) => Pick::Card(*card),
        }
    }

    /// Ticks or unticks the choice a row's checkbox belongs to, or every choice of an event.
    fn tick(&mut self, item: &Item) {
        let ticked = self.ticked.as_ref();
        self.ticked = match item {
            Item::Choice(result) => self
                .events
                .group(predictions::event_title(result))
                .and_then(|group| predictions::toggle_choice(ticked, group, &result.full_symbol)),
            Item::Event(head) => self
                .events
                .group(&head.title)
                .and_then(|group| predictions::toggle_group(ticked, group)),
            Item::Market(_) | Item::Tile(_) => return,
        };
    }

    /// Validates a pick, records it as recent and hands it to the host. A watchlist's own pick
    /// of what the list already holds is refused: nothing is added, so nothing is acknowledged.
    fn commit(&mut self, pick: Pick, kind: PickKind, env: &Env) -> Option<Output> {
        if kind == PickKind::Watchlist && offer::holds_market(self.mode, &pick.instrument(), env) {
            self.added = None;
            return None;
        }
        self.pick_needs_credentials = None;
        let output = match pick {
            Pick::Card(card) => {
                if kind == PickKind::Watchlist {
                    Output::WatchlistCard(card)
                } else {
                    Output::AddCard(card)
                }
            }
            Pick::Market(result) => {
                if let Some(page) = SettingsPage::for_source(result.source)
                    && !page.has_credentials()
                {
                    self.pick_needs_credentials = Some(page);
                    return None;
                }
                if self.mode != PickMode::Watchlist
                    && recents::record_recent(&mut self.recents, &result)
                    && let Ok(store) = self.store.lock()
                {
                    recents::store_recents(&store, &self.recents);
                }
                match kind {
                    PickKind::Add => Output::Add(result),
                    PickKind::Overlay => Output::Overlay(result),
                    PickKind::Replace => Output::Replace(result),
                    PickKind::Watchlist => Output::Watchlist(result),
                }
            }
        };
        if self.mode == PickMode::Watchlist {
            self.added = Some(match &output {
                Output::Watchlist(result) => pick_name(result),
                Output::WatchlistCard(card) => card.kind.display_title().to_owned(),
                _ => String::new(),
            });
        }
        Some(output)
    }

    // MARK: Layout

    #[allow(clippy::too_many_arguments)]
    fn layout(
        &mut self,
        ui: &mut Ui,
        full: Rect,
        search_id: Id,
        listing: &Listing,
        rows: &Rows,
        related: Option<&[SearchResult]>,
        env: &Env,
    ) -> Option<(Pick, PickKind)> {
        let mut picked = None;
        let search_area = Rect::from_min_size(
            full.min + vec2(PADDING, PADDING),
            vec2(full.width() - 2.0 * PADDING, kit::search_box::LARGE_HEIGHT),
        );
        let mut field = ui.new_child(egui::UiBuilder::new().max_rect(search_area));
        let focus = self.focus_frames > 0;
        self.focus_frames = self.focus_frames.saturating_sub(1);
        let placeholder = self.placeholder();
        let event = kit::search_box::large_search_box(
            &mut field,
            Icon::Magnifier,
            &mut self.query,
            listing.searching,
            &placeholder,
            search_id,
            focus,
        );
        if event.changed || event.cleared {
            if event.cleared {
                self.query.clear();
            }
            self.search.edited();
            self.highlight = 0;
            self.pick_needs_credentials = None;
            self.added = None;
        }

        let tabs = Rect::from_min_size(
            pos2(full.left(), search_area.bottom() + 4.0),
            vec2(full.width(), TABS),
        );
        let (scope, filter) = tabs::tab_row(
            ui,
            tabs,
            Scope::tabs(self.mode),
            self.scope,
            self.filter_menu(),
        );
        if let Some(scope) = scope {
            self.set_scope(scope);
        }
        if let Some(filter) = filter {
            self.set_filter(filter);
        }
        let body_top = tabs.bottom();
        ui.painter().hline(
            full.x_range(),
            body_top,
            Stroke::new(1.0, theme::separator()),
        );
        let footer_top = full.bottom() - FOOTER;
        ui.painter().hline(
            full.x_range(),
            footer_top,
            Stroke::new(1.0, theme::separator()),
        );

        let detail_width = if full.width() >= DETAIL_MIN_WIDTH {
            (full.width() * DETAIL_SHARE)
                .clamp(DETAIL_MIN, DETAIL_MAX)
                .round()
        } else {
            0.0
        };
        let list_area = Rect::from_min_max(
            pos2(full.left(), body_top + 1.0),
            pos2(full.right() - detail_width, footer_top),
        );
        if let Some(hit) = self.list(ui, list_area, listing, rows, env) {
            picked = Some(hit);
        }
        if detail_width > 0.0 {
            let panel = Rect::from_min_max(
                pos2(list_area.right(), body_top + 1.0),
                pos2(full.right(), footer_top),
            );
            ui.painter().vline(
                panel.left(),
                panel.y_range(),
                Stroke::new(1.0, theme::separator()),
            );
            if let Some(item) = self.highlighted.clone() {
                let names = item
                    .market()
                    .map(|result| self.caches.names.of(result))
                    .unwrap_or_default();
                let offer = Offer::of(self.mode, Some(&item), env);
                let shown = detail::Detail {
                    item: &item,
                    names,
                    quote: env.quote,
                    related,
                    actions: Actions::of(&offer, self.default_kind()),
                };
                let out = detail::show(ui, panel, &shown, &mut self.caches.chart);
                if let Some(other) = out.related {
                    picked = Some((Pick::Market(other), self.default_kind()));
                }
                if let Some(kind) = out.action {
                    picked = self.pick_of(&item).map(|pick| (pick, kind));
                }
            }
        }
        let footer_area = Rect::from_min_max(pos2(full.left(), footer_top + 1.0), full.max);
        if footer::show(ui, footer_area, self.footer(env))
            && let Some(ticked) = &self.ticked
        {
            picked = Some((Pick::Market(ticked.clone()), self.default_kind()));
        }
        picked
    }

    /// What the footer says about the highlighted row.
    fn footer(&self, env: &Env) -> Footer<'_> {
        let offer = Offer::of(self.mode, self.highlighted.as_ref(), env);
        let count = predictions::ticked_count(self.ticked.as_ref());
        Footer {
            offer,
            return_hint: footer::return_hint(
                offer,
                self.highlighted.as_ref(),
                env.replacing,
                &self.list_name,
            ),
            ticked: (count >= 1).then(|| self.ticked_label(count)),
            added: self.added.as_deref(),
        }
    }

    /// What the footer's button for `count` ticked choices says.
    fn ticked_label(&self, count: usize) -> String {
        match self.mode {
            PickMode::Watchlist => tr_n!(count, "Add {count} choice", "Add {count} choices"),
            _ => tr_n!(count, "Chart {count} choice", "Chart {count} choices"),
        }
    }

    /// The card under the list, if a provider needs a login or could not be searched.
    fn notice(&self, listing: &Listing, lookup: &Lookup) -> Option<Notice> {
        notice::notice_for(listing, lookup, self.scope, self.pick_needs_credentials)
    }

    /// What the search box asks for: a watchlist's picker names the list it adds to.
    fn placeholder(&self) -> String {
        if self.mode == PickMode::Watchlist && self.scope.searches() {
            tr!("Search to add to {list}…", list = self.list_name)
        } else {
            self.scope.placeholder().to_owned()
        }
    }

    /// What the pull-down at the right of the tabs offers in this tab: the exchanges, or on the
    /// Dashboards tab the portfolios. `None` when there is nothing to choose between.
    fn filter_menu(&self) -> Option<FilterMenu> {
        if self.scope == Scope::Dashboards {
            FilterMenu::portfolios(&self.portfolios, self.card_scope_in_use())
        } else {
            // A venue typed as a prefix wins over the pull-down, so the pull-down says what is
            // searched.
            FilterMenu::venues(&self.scope.venues(), self.lookup().venue)
        }
    }
}

/// What a market is called when a watchlist took it: the event, for choices charted together.
fn pick_name(result: &SearchResult) -> String {
    match &result.event {
        Some(event) if event.series.len() > 1 && !event.title.is_empty() => event.title.clone(),
        _ => list::name(result).to_owned(),
    }
}

/// Selects all of `text` in the text field `id`, replacing a cursor left over from an earlier sheet.
fn select_all(ctx: &egui::Context, id: Id, text: &str) {
    use egui::text::{CCursor, CCursorRange};
    let mut edit = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    edit.cursor.set_char_range(Some(CCursorRange::two(
        CCursor::new(0),
        CCursor::new(text.chars().count()),
    )));
    edit.store(ctx, id);
}

/// What the keyboard asked for this frame.
#[derive(Default)]
struct Keys {
    up: bool,
    down: bool,
    /// Return: use the highlighted market.
    pick: bool,
    /// ⇧Return: add it as a new chart rather than replacing.
    add: bool,
    /// ⌥Return: draw it over the focused chart.
    overlay: bool,
    scope: bool,
    close: bool,
}

impl Keys {
    fn read(ctx: &egui::Context) -> Self {
        ctx.input_mut(|input| {
            let add = input.consume_key(Modifiers::SHIFT, Key::Enter);
            let overlay = input.consume_key(Modifiers::ALT, Key::Enter);
            Self {
                up: input.consume_key(Modifiers::NONE, Key::ArrowUp),
                down: input.consume_key(Modifiers::NONE, Key::ArrowDown),
                pick: add || overlay || input.consume_key(Modifiers::NONE, Key::Enter),
                add,
                overlay,
                scope: input.consume_key(Modifiers::NONE, Key::Tab),
                close: input.consume_key(Modifiers::NONE, Key::Escape),
            }
        })
    }
}

#[cfg(test)]
mod tests;
