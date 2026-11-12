//! A search sheet that picks one market for a portfolio: the first step of "Add Transaction"
//! (which asset is it for?) and of "Match…" in the CoinMarketCap import. Searches the crypto
//! sources or the stock source as you type (debounced, off the UI thread) and hands back the
//! chosen `SearchResult`; the host turns it into a `PortfolioAsset`.

// Transient: the portfolio host opens this once it is wired.
#![allow(dead_code)]

use crate::model::DataSource;
use crate::service::SearchResult;
use crate::store::SharedStore;
use crate::ui::chart::add::art;
use crate::ui::chart::add::sheet::{
    self, CRYPTO_SUGGESTIONS, Chip, PREDICTION_TOPICS, RecentMarket, STOCK_SUGGESTIONS,
};
use crate::ui::icons::Icon;
use crate::ui::kit;
use crate::ui::theme;
use eframe::egui::{self, Color32, Id, Rect, Stroke, Ui, pos2, vec2};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant};

pub const DEBOUNCE: Duration = Duration::from_millis(300);
pub const CRYPTO_SOURCES: [DataSource; 4] = [
    DataSource::Binance,
    DataSource::Coinbase,
    DataSource::CoinGecko,
    DataSource::DexScreener,
];
pub const STOCK_SOURCES: [DataSource; 1] = [DataSource::Alpaca];

/// A provider's ticker search: results, or why there are none.
pub type SearchFn =
    Arc<dyn Fn(DataSource, &str) -> Result<Vec<SearchResult>, String> + Send + Sync>;

type Batch = Vec<(DataSource, Result<Vec<SearchResult>, String>)>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PickerTab {
    Crypto,
    Stocks,
    Predictions,
}

impl PickerTab {
    const ALL: [Self; 3] = [Self::Crypto, Self::Stocks, Self::Predictions];

    pub fn title(self) -> &'static str {
        match self {
            Self::Crypto => "Crypto",
            Self::Stocks => "Stocks",
            Self::Predictions => "Predictions",
        }
    }

    /// The picture at the head of the sheet: the one Add Chart shows for the same section.
    fn scene(self) -> art::Scene {
        match self {
            Self::Crypto => art::Scene::Crypto,
            Self::Stocks => art::Scene::Stocks,
            Self::Predictions => art::Scene::Predictions,
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Crypto => Icon::BitcoinCircle,
            Self::Stocks => Icon::BuildingColumns,
            Self::Predictions => Icon::Percent,
        }
    }

    pub fn placeholder(self) -> &'static str {
        match self {
            Self::Crypto => "Ticker symbol (e.g. BTC or PEPE)",
            Self::Stocks => "US stock symbol or company name (e.g. AAPL)",
            Self::Predictions => "Search markets (e.g. Bitcoin, Fed, election)",
        }
    }
}

pub struct AssetPickerState {
    pub title: String,
    pub subtitle: String,
    pub action_label: String,
    pub tab: PickerTab,
    /// The prediction-market provider the Predictions section searches.
    pub provider: DataSource,
    pub query: String,
    pub selected: Option<SearchResult>,
    results: Vec<(DataSource, Result<Vec<SearchResult>, String>)>,
    searched_query: String,
    searched_tab: Option<PickerTab>,
    inbox: Option<Receiver<Batch>>,
    debounce: Option<Instant>,
    search: SearchFn,
    focus: bool,
    /// The markets picked before (Add Chart's list); loaded when the sheet first draws.
    recents: Vec<RecentMarket>,
    recents_loaded: bool,
    /// The list changed in the sheet and still has to be saved.
    recents_dirty: bool,
}

#[derive(Clone, Debug)]
pub enum AssetPickerOutcome {
    Choose(Box<SearchResult>),
    Cancel,
}

impl AssetPickerState {
    pub fn new(title: &str, subtitle: &str, action_label: &str) -> Self {
        Self::with_search(
            title,
            subtitle,
            action_label,
            Arc::new(crate::service::search),
        )
    }

    pub fn with_search(title: &str, subtitle: &str, action_label: &str, search: SearchFn) -> Self {
        Self {
            title: title.to_owned(),
            subtitle: subtitle.to_owned(),
            action_label: action_label.to_owned(),
            tab: PickerTab::Crypto,
            provider: DataSource::Polymarket,
            query: String::new(),
            selected: None,
            results: Vec::new(),
            searched_query: String::new(),
            searched_tab: None,
            inbox: None,
            debounce: None,
            search,
            focus: true,
            recents: Vec::new(),
            recents_loaded: false,
            recents_dirty: false,
        }
    }

    /// The providers the current section asks.
    pub fn sources(&self) -> Vec<DataSource> {
        match self.tab {
            PickerTab::Crypto => CRYPTO_SOURCES.to_vec(),
            PickerTab::Stocks => STOCK_SOURCES.to_vec(),
            PickerTab::Predictions => vec![self.provider],
        }
    }

    /// Switches the prediction-market provider and searches it again.
    pub fn select_provider(&mut self, provider: DataSource) {
        if self.provider != provider {
            self.provider = provider;
            self.selected = None;
            self.results.clear();
            self.searched_tab = None;
            self.inbox = None;
            self.touch();
        }
    }

    pub fn is_loading(&self) -> bool {
        self.inbox.is_some()
    }

    pub fn results(&self) -> &[(DataSource, Result<Vec<SearchResult>, String>)] {
        &self.results
    }

    /// Call after the query or tab changed: a search starts once typing pauses.
    pub fn touch(&mut self) {
        if self.query.trim() == self.searched_query && self.searched_tab == Some(self.tab) {
            self.debounce = None;
            return;
        }
        self.selected = None;
        self.results.clear();
        if self.query.trim().is_empty() {
            self.debounce = None;
            self.inbox = None;
            self.searched_query.clear();
        } else {
            self.debounce = Some(Instant::now());
        }
    }

    /// Selects a recent market on the section that owns its source, switching to it.
    fn pick_recent(&mut self, result: SearchResult) {
        self.select_tab(if result.source == DataSource::Alpaca {
            PickerTab::Stocks
        } else {
            PickerTab::Crypto
        });
        self.selected = Some(result);
    }

    pub fn select_tab(&mut self, tab: PickerTab) {
        if self.tab != tab {
            self.tab = tab;
            self.selected = None;
            self.results.clear();
            self.searched_tab = None;
            self.inbox = None;
            self.touch();
        }
    }

    /// Whether a search should start now: the query or tab changed and typing has paused.
    pub fn is_due(&self, now: Instant) -> bool {
        self.inbox.is_none()
            && self
                .debounce
                .is_some_and(|since| now.duration_since(since) >= DEBOUNCE)
            && (self.query.trim() != self.searched_query || self.searched_tab != Some(self.tab))
    }

    /// Starts the search for the current query on a worker thread.
    pub fn start(&mut self, ctx: &egui::Context) {
        self.debounce = None;
        let query = self.query.trim().to_owned();
        self.searched_query = query.clone();
        self.searched_tab = Some(self.tab);
        if query.is_empty() {
            self.results.clear();
            return;
        }
        let (sender, receiver) = channel();
        self.inbox = Some(receiver);
        let search = Arc::clone(&self.search);
        let sources = self.sources();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let batch: Batch = std::thread::scope(|scope| {
                let handles: Vec<_> = sources
                    .iter()
                    .map(|source| {
                        let (search, query) = (&search, &query);
                        scope.spawn(move || (*source, search(*source, query)))
                    })
                    .collect();
                handles
                    .into_iter()
                    .filter_map(|handle| handle.join().ok())
                    .collect()
            });
            let _ = sender.send(batch);
            ctx.request_repaint();
        });
    }

    /// Takes a finished search, if any. Results arrive in source priority order.
    pub fn receive(&mut self) {
        let Some(inbox) = &self.inbox else {
            return;
        };
        match inbox.try_recv() {
            Ok(mut batch) => {
                if self.query.trim() != self.searched_query || self.searched_tab != Some(self.tab) {
                    self.inbox = None;
                    return;
                }
                let sources = self.sources();
                let order = |source: DataSource| {
                    sources
                        .iter()
                        .position(|candidate| *candidate == source)
                        .unwrap_or(usize::MAX)
                };
                batch.sort_by_key(|(source, _)| order(*source));
                let query = self.searched_query.clone();
                self.results = batch
                    .into_iter()
                    .map(|(source, result)| {
                        (
                            source,
                            result.map(|hits| crate::service::rank_results(hits, &query)),
                        )
                    })
                    .collect();
                self.inbox = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.inbox = None,
        }
    }
}

// MARK: Pure text

pub fn hit_summary(hit: &SearchResult) -> String {
    match hit.price {
        Some(price) => format!("{}  ·  {}", hit.symbol, crate::format::price(price)),
        None => hit.symbol.clone(),
    }
}

/// What a source's section says when it has no rows.
pub fn empty_note(result: &Result<Vec<SearchResult>, String>) -> Option<String> {
    match result {
        Err(message) => Some(message.clone()),
        Ok(hits) if hits.is_empty() => Some("No matches".to_owned()),
        Ok(_) => None,
    }
}

// MARK: View

const FOOTER_HEIGHT: f32 = 60.0;
const SEARCH_ID: &str = "portfolio_asset_search_field";

/// The sheet is the Add Chart sheet's twin: same size, header, section switcher, search box,
/// suggestion chips, result list and footer, so the two never drift apart.
pub fn show(
    ctx: &egui::Context,
    state: &mut AssetPickerState,
    store: &SharedStore,
) -> Option<AssetPickerOutcome> {
    if !state.recents_loaded {
        state.recents_loaded = true;
        state.recents = store
            .lock()
            .ok()
            .map(|store| sheet::load_recents(&store))
            .unwrap_or_default();
    }
    state.receive();
    if state.is_due(Instant::now()) {
        state.start(ctx);
    }
    if state.debounce.is_some() || state.is_loading() {
        ctx.request_repaint_after(Duration::from_millis(100));
    }
    let screen = ctx.content_rect().size();
    let size = vec2(
        sheet::SHEET_SIZE.x.min(screen.x - 32.0).max(560.0),
        sheet::SHEET_SIZE.y.min(screen.y - 32.0).max(440.0),
    );
    let search_id = Id::new(SEARCH_ID);
    let field_focused = ctx.memory(|memory| memory.has_focus(search_id));
    let mut outcome = None;

    egui::Modal::new(Id::new("portfolio-asset-picker"))
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
            let padding = sheet::PADDING;
            let body = Rect::from_min_size(ui.min_rect().min, vec2(size.x, size.y - FOOTER_HEIGHT))
                .shrink(padding);
            let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body));
            body_ui.set_clip_rect(body.expand(2.0));
            show_body(&mut body_ui, state, search_id, &mut outcome);

            let divider_y = ui.min_rect().top() + size.y - FOOTER_HEIGHT;
            ui.painter().hline(
                ui.min_rect().x_range(),
                divider_y,
                Stroke::new(1.0, theme::separator()),
            );
            let footer = Rect::from_min_size(
                pos2(ui.min_rect().left() + padding, divider_y + 1.0),
                vec2(size.x - padding * 2.0, FOOTER_HEIGHT - 1.0),
            );
            let mut footer_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(footer)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            show_footer(&mut footer_ui, state, &mut outcome);
        });

    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        outcome = Some(AssetPickerOutcome::Cancel);
    } else if outcome.is_none()
        && !field_focused
        && ctx.input(|input| input.key_pressed(egui::Key::Enter))
        && let Some(selected) = state.selected.clone()
    {
        outcome = Some(AssetPickerOutcome::Choose(Box::new(selected)));
    }
    if let Some(AssetPickerOutcome::Choose(result)) = &outcome
        && sheet::record_recent(&mut state.recents, result)
    {
        state.recents_dirty = true;
    }
    if std::mem::take(&mut state.recents_dirty)
        && let Ok(store) = store.lock()
    {
        sheet::store_recents(&store, &state.recents);
    }
    outcome
}

fn show_body(
    ui: &mut Ui,
    state: &mut AssetPickerState,
    search_id: Id,
    outcome: &mut Option<AssetPickerOutcome>,
) {
    sheet::header(ui, state.tab.scene(), &state.title, &state.subtitle);
    ui.add_space(16.0);
    let items = PickerTab::ALL.map(|tab| (tab, tab.title(), tab.icon()));
    if let Some(tab) = kit::icon_tabs::icon_tabs(ui, &items, state.tab) {
        state.select_tab(tab);
        state.focus = true;
    }
    ui.add_space(16.0);

    let content = Rect::from_min_size(
        ui.cursor().min,
        vec2(ui.available_width(), ui.available_height().max(80.0)),
    );
    let mut content_ui = ui.new_child(egui::UiBuilder::new().max_rect(content));
    let ui = &mut content_ui;

    let focus = std::mem::take(&mut state.focus);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if state.tab == PickerTab::Predictions
            && let Some(provider) = sheet::provider_menu(ui, state.provider)
        {
            state.select_provider(provider);
        }
        let searching = state.is_loading() || state.debounce.is_some();
        let event = sheet::search_box(
            ui,
            &mut state.query,
            searching,
            state.tab.placeholder(),
            search_id,
            focus,
        );
        if event.changed {
            state.touch();
        }
        if event.cleared {
            state.query.clear();
            state.touch();
        }
        if event.submitted {
            // Return picks the first result; on a selection that already is the first, it adds.
            if let Some(first) = state.first_hit() {
                if state
                    .selected
                    .as_ref()
                    .is_some_and(|selected| sheet::same_market(selected, &first))
                {
                    *outcome = Some(AssetPickerOutcome::Choose(Box::new(first)));
                } else {
                    state.selected = Some(first);
                }
            }
        }
    });
    ui.add_space(16.0);

    if state.query.trim().is_empty() {
        idle(ui, state, outcome);
        return;
    }
    let searching = state.is_loading() || state.debounce.is_some();
    let sections = state.sections();
    if !sections.is_empty() {
        if let Some(chosen) = sheet::source_groups(ui, &sections, &mut state.selected) {
            *outcome = Some(AssetPickerOutcome::Choose(Box::new(chosen)));
        }
    } else if !searching && state.has_answer() {
        if let Some(error) = state.first_error() {
            sheet::no_results(ui, "Search Failed", &error);
        } else {
            let query = state.query.trim().to_owned();
            sheet::no_results(
                ui,
                &format!("No Results for “{query}”"),
                "Check the spelling or try a new search.",
            );
        }
    }
}

/// Suggestion chips while the box is empty; they scroll rather than push the sheet.
fn idle(ui: &mut Ui, state: &mut AssetPickerState, outcome: &mut Option<AssetPickerOutcome>) {
    let mut search = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            match state.tab {
                PickerTab::Crypto => {
                    let items =
                        CRYPTO_SUGGESTIONS.map(|ticker| Chip::Logo(ticker, DataSource::Binance));
                    if let Some(index) = sheet::suggestion_grid(ui, "Suggestions", &items) {
                        search = Some(CRYPTO_SUGGESTIONS[index]);
                    }
                }
                PickerTab::Stocks => {
                    let items =
                        STOCK_SUGGESTIONS.map(|ticker| Chip::Logo(ticker, DataSource::Alpaca));
                    if let Some(index) = sheet::suggestion_grid(
                        ui,
                        "Popular US stocks and ETFs · free IEX feed",
                        &items,
                    ) {
                        search = Some(STOCK_SUGGESTIONS[index]);
                    }
                }
                PickerTab::Predictions => {
                    let items = PREDICTION_TOPICS.map(|(title, _, icon)| Chip::Symbol(title, icon));
                    if let Some(index) = sheet::suggestion_grid(ui, "Popular topics", &items) {
                        search = Some(PREDICTION_TOPICS[index].1);
                    }
                }
            }
            if state.tab != PickerTab::Predictions && !state.recents.is_empty() {
                ui.add_space(16.0);
                let recents = sheet::recents_card(ui, &mut state.recents, state.selected.as_ref());
                if let Some(result) = recents.commit {
                    *outcome = Some(AssetPickerOutcome::Choose(Box::new(result)));
                } else if let Some(result) = recents.pick {
                    state.pick_recent(result);
                }
                state.recents_dirty |= recents.changed;
            }
        });
    if let Some(query) = search {
        state.query = query.to_owned();
        state.touch();
        state.debounce = Some(Instant::now() - DEBOUNCE);
        state.focus = true;
    }
}

fn show_footer(
    ui: &mut Ui,
    state: &mut AssetPickerState,
    outcome: &mut Option<AssetPickerOutcome>,
) {
    ui.spacing_mut().item_spacing.x = 12.0;
    if let Some(selected) = &state.selected {
        sheet::selected_banner(ui, selected, (ui.available_width() - 200.0).min(380.0));
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let label = state.action_label.clone();
        if sheet::sheet_button(ui, &label, true, state.selected.is_some()).clicked()
            && let Some(selected) = state.selected.clone()
        {
            *outcome = Some(AssetPickerOutcome::Choose(Box::new(selected)));
        }
        if sheet::sheet_button(ui, "Cancel", false, true).clicked() {
            *outcome = Some(AssetPickerOutcome::Cancel);
        }
    });
}

impl AssetPickerState {
    /// The answered sources that have hits, as the result list draws them.
    fn sections(&self) -> Vec<(DataSource, Vec<SearchResult>)> {
        self.results
            .iter()
            .filter_map(|(source, result)| match result {
                Ok(hits) if !hits.is_empty() => {
                    Some((*source, hits.iter().take(20).cloned().collect()))
                }
                _ => None,
            })
            .collect()
    }

    fn first_hit(&self) -> Option<SearchResult> {
        self.sections()
            .into_iter()
            .flat_map(|(_, hits)| hits)
            .next()
    }

    /// Whether a search has come back for the current query and section.
    fn has_answer(&self) -> bool {
        !self.results.is_empty()
            && self.query.trim() == self.searched_query
            && self.searched_tab == Some(self.tab)
    }

    fn first_error(&self) -> Option<String> {
        self.results
            .iter()
            .find_map(|(_, result)| result.as_ref().err().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Rect;
    use std::sync::Mutex;

    fn hit(source: DataSource, symbol: &str, full: &str) -> SearchResult {
        SearchResult {
            symbol: symbol.into(),
            full_symbol: full.into(),
            source,
            price: None,
            event: None,
            base: None,
            quote: None,
        }
    }

    #[allow(clippy::type_complexity)]
    fn fake() -> (SearchFn, Arc<Mutex<Vec<(DataSource, String)>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&calls);
        let search: SearchFn = Arc::new(move |source, query| {
            recorded.lock().unwrap().push((source, query.to_owned()));
            match source {
                DataSource::Binance => Ok(vec![
                    hit(source, "BTC/USDT", "BTCUSDT"),
                    hit(source, "WBTC/USDT", "WBTCUSDT"),
                ]),
                DataSource::Coinbase => Ok(vec![]),
                DataSource::CoinGecko => Err("rate limited".to_owned()),
                _ => Ok(vec![hit(source, "BTC/WETH", "0xabc")]),
            }
        });
        (search, calls)
    }

    fn wait(state: &mut AssetPickerState) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while state.is_loading() && Instant::now() < deadline {
            state.receive();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_search_waits_for_typing_to_pause_and_never_repeats_itself() {
        let (search, calls) = fake();
        let mut state = AssetPickerState::with_search("Add", "Pick", "Use Asset", search);
        let ctx = egui::Context::default();
        assert!(
            !state.is_due(Instant::now() + Duration::from_secs(1)),
            "nothing typed yet"
        );
        state.query = "btc".into();
        state.touch();
        let typed = Instant::now();
        assert!(!state.is_due(typed), "still inside the debounce");
        assert!(state.is_due(typed + DEBOUNCE + Duration::from_millis(10)));
        state.start(&ctx);
        assert!(state.is_loading());
        wait(&mut state);
        assert_eq!(
            calls.lock().unwrap().len(),
            4,
            "one request per crypto source"
        );
        assert!(
            !state.is_due(Instant::now() + Duration::from_secs(5)),
            "the same query is not searched twice"
        );
        state.query = "btc ".into();
        state.touch();
        assert!(
            !state.is_due(Instant::now() + Duration::from_secs(5)),
            "whitespace is not a new query"
        );
        state.query = "eth".into();
        state.touch();
        assert!(state.is_due(Instant::now() + Duration::from_secs(5)));
    }

    #[test]
    fn results_keep_source_priority_and_are_ranked_within_a_source() {
        let (search, _) = fake();
        let mut state = AssetPickerState::with_search("Add", "Pick", "Use Asset", search);
        state.query = "BTC".into();
        state.start(&egui::Context::default());
        wait(&mut state);
        let sources: Vec<DataSource> = state.results().iter().map(|(source, _)| *source).collect();
        assert_eq!(sources, CRYPTO_SOURCES);
        let binance = state.results()[0].1.as_ref().unwrap();
        assert_eq!(
            binance[0].full_symbol, "BTCUSDT",
            "the exact base ranks first"
        );
        assert_eq!(
            empty_note(&state.results()[1].1).as_deref(),
            Some("No matches")
        );
        assert_eq!(
            empty_note(&state.results()[2].1).as_deref(),
            Some("rate limited")
        );
        assert_eq!(empty_note(&state.results()[0].1), None);
    }

    #[test]
    fn switching_tabs_clears_the_choice_and_searches_the_stock_source() {
        let (search, calls) = fake();
        let mut state = AssetPickerState::with_search("Add", "Pick", "Use Asset", search);
        state.query = "aapl".into();
        state.selected = Some(hit(DataSource::Binance, "BTC/USDT", "BTCUSDT"));
        state.select_tab(PickerTab::Stocks);
        assert!(state.selected.is_none() && state.results().is_empty());
        assert!(state.is_due(Instant::now() + Duration::from_secs(1)));
        state.start(&egui::Context::default());
        wait(&mut state);
        assert_eq!(
            *calls.lock().unwrap(),
            vec![(DataSource::Alpaca, "aapl".to_owned())]
        );
        state.select_tab(PickerTab::Stocks);
        assert!(
            state.results().len() == 1,
            "re-selecting the same tab changes nothing"
        );
        assert_eq!(state.sources(), STOCK_SOURCES);
        state.select_tab(PickerTab::Predictions);
        assert!(state.is_due(Instant::now() + Duration::from_secs(1)));
        state.start(&egui::Context::default());
        wait(&mut state);
        assert_eq!(
            state
                .results()
                .iter()
                .map(|(source, _)| *source)
                .collect::<Vec<_>>(),
            [DataSource::Polymarket]
        );
        state.select_provider(DataSource::Kalshi);
        assert!(state.results().is_empty(), "a new provider starts clean");
        assert!(state.is_due(Instant::now() + Duration::from_secs(1)));
        assert_eq!(state.sources(), [DataSource::Kalshi]);
    }

    fn memory_store() -> SharedStore {
        Arc::new(Mutex::new(
            crate::store::Store::open_in_memory().expect("open store"),
        ))
    }

    fn draw(state: &mut AssetPickerState) -> Option<AssetPickerOutcome> {
        draw_with(state, &memory_store())
    }

    fn draw_with(state: &mut AssetPickerState, store: &SharedStore) -> Option<AssetPickerOutcome> {
        let ctx = egui::Context::default();
        let mut outcome = None;
        for _ in 0..2 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 800.0))),
                    ..Default::default()
                },
                |ui| outcome = show(ui.ctx(), state, store),
            );
            output.textures_delta.clear();
        }
        outcome
    }

    #[test]
    fn the_sheet_draws_idle_searching_results_and_empty_states_in_each_section() {
        let (search, _) = fake();
        let mut state = AssetPickerState::with_search("Add Asset", "Pick", "Continue", search);
        for tab in PickerTab::ALL {
            state.select_tab(tab);
            assert!(draw(&mut state).is_none());
        }
        state.select_tab(PickerTab::Crypto);
        state.query = "BTC".into();
        state.touch();
        assert!(draw(&mut state).is_none(), "waiting out the debounce");
        state.start(&egui::Context::default());
        wait(&mut state);
        state.selected = state.first_hit();
        assert!(state.selected.is_some());
        assert!(draw(&mut state).is_none());
        state.query = "zzzz".into();
        state.results.clear();
        state.searched_query = "zzzz".into();
        assert!(draw(&mut state).is_none(), "no results for a typed query");
    }

    #[test]
    fn results_group_by_source_and_the_first_hit_is_the_best_match() {
        let (search, _) = fake();
        let mut state = AssetPickerState::with_search("Add", "Pick", "Use Asset", search);
        state.query = "BTC".into();
        state.start(&egui::Context::default());
        wait(&mut state);
        let first = state.first_hit().unwrap();
        assert_eq!(first.full_symbol, "BTCUSDT");
        let sections = state.sections();
        assert_eq!(sections.len(), 2, "empty and failed sources draw no group");
    }

    #[test]
    fn an_empty_query_clears_results_without_a_request() {
        let (search, calls) = fake();
        let mut state = AssetPickerState::with_search("Add", "Pick", "Use Asset", search);
        state.query = "  ".into();
        state.touch();
        assert!(!state.is_due(Instant::now() + Duration::from_secs(1)));
        assert!(!state.is_loading());
        assert!(calls.lock().unwrap().is_empty());
        assert_eq!(
            hit_summary(&hit(DataSource::Binance, "BTC/USDT", "x")),
            "BTC/USDT"
        );
        let mut priced = hit(DataSource::Binance, "BTC/USDT", "x");
        priced.price = Some(67_432.19);
        assert!(hit_summary(&priced).starts_with("BTC/USDT  ·  "));
    }

    #[test]
    fn a_pick_is_remembered_and_offered_again() {
        let store = memory_store();
        let mut state = AssetPickerState::new("Add Asset", "", "Continue");
        draw_with(&mut state, &store);
        assert!(state.recents.is_empty());
        let btc = hit(DataSource::Binance, "BTC/USDT", "BTCUSDT");
        state.recents_dirty = false;
        // What the sheet does when a row is chosen.
        assert!(sheet::record_recent(&mut state.recents, &btc));
        sheet::store_recents(&store.lock().unwrap(), &state.recents);

        let mut next = AssetPickerState::new("Add Asset", "", "Continue");
        draw_with(&mut next, &store);
        assert_eq!(next.recents.len(), 1);
        assert_eq!(next.recents[0].full_symbol, "BTCUSDT");
    }

    #[test]
    fn a_recent_stock_switches_to_stocks_and_selects_it() {
        let mut state = AssetPickerState::new("Add Asset", "", "Continue");
        assert_eq!(state.tab, PickerTab::Crypto);
        state.pick_recent(hit(DataSource::Alpaca, "AAPL", "AAPL"));
        assert_eq!(state.tab, PickerTab::Stocks);
        assert_eq!(
            state.selected.as_ref().map(|s| s.symbol.as_str()),
            Some("AAPL")
        );
        state.pick_recent(hit(DataSource::Binance, "BTC/USDT", "BTCUSDT"));
        assert_eq!(state.tab, PickerTab::Crypto);
        assert!(state.selected.is_some());
    }

    #[test]
    fn prediction_markets_are_not_kept_as_recents() {
        let mut recents = Vec::new();
        assert!(!sheet::record_recent(
            &mut recents,
            &hit(DataSource::Polymarket, "Fed", "fed")
        ));
        assert!(recents.is_empty());
    }
}
