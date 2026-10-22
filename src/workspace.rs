use crate::model::alert::{AlertAsset, AlertCurrency, PriceAlert};
use crate::model::chart_sync::SyncHub;
use crate::model::history::{self, OlderHistory};
use crate::model::paper::{PaperCurrency, PaperInstrument, PaperQuote};
use crate::model::portfolio_card::PortfolioCard;
use crate::model::watchlist::{EntryId, WatchlistInstrument, new_id, now_ms};
use crate::model::watchlist_text::ImportReport;
use crate::model::{
    BuiltIn, ChartColumn, ChartConfig, ChartLayout, ChartSyncSettings, DataSource, Drawing,
    DrawingCommand, DrawingStyle, EMA_PERIODS, IndicatorConfig, InstrumentId, Kline,
    LayoutSnapshot, MagnetMode, PmSeries, ReplayEngine, ReplayInterval, ReplayStatus, SavedView,
    Timeframe, Tool,
};
use crate::paper_host::{PaperHost, PaperPanel, PaperPanelAction};
use crate::portfolio_card_host::{CardRuntime, PortfolioCardEnv};
use crate::service::live::BinanceLiveFeed;
use crate::service::watchlist_quotes::{WatchlistQuoteBook, WatchlistQuoteCoordinator};
use crate::service::{self, SearchResult};
use crate::store::SharedStore;
use crate::store::drawing_template::{DrawingPrefs, DrawingTemplateStore};
use crate::store::watchlist::WatchlistStore;
use crate::ui::add_chart_sheet::{self, AddChartAction, AddChartSheet};
use crate::ui::alert_editor::{self, PriceAlertEditorOutcome, PriceAlertEditorState};
use crate::ui::chart_settings::{
    self, ChartSettingsAction, ChartSettingsContext, ChartSettingsState,
};
use crate::ui::drawing_properties::{self, DrawingProperties};
use crate::ui::empty_state::{self, EmptyStateAction};
use crate::ui::layout_prompts::{self, NameOutcome, NamePrompt, UnsavedChoice};
use crate::ui::paper_chart_trading::PaperOverlayAction;
use crate::ui::pine_settings_popover::{self, PineSettingsPopover};
use crate::ui::replay_bar::{self, ReplayBarAction, ReplayBarInput};
use crate::ui::replay_date_sheet::{self, ReplayDateSheetState};
use crate::ui::shortcut;
use crate::ui::tool_sidebar::{self, ToolSidebarAction};
use crate::ui::watchlist_sidebar::{
    self, WatchlistAction, WatchlistSidebarInput, WatchlistSidebarState,
};
use crate::ui::workspace_toolbar::{self, ToolbarAction};
use crate::ui::{
    self, ChartCard, ChartCardAction, CrosshairInfo, DrawingFrame, ReplayChartFrame, SeriesLine,
};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod context;
mod snapshot;
use crate::model::snapshot::SnapshotAction;
use context::ContextState;
use snapshot::SnapshotState;

const CANDLE_LIMIT: u32 = 500;
const REFRESH_INTERVAL: Duration = Duration::from_secs(5);
/// How often a rate-limited provider (one request every few seconds) is asked again, and how long
/// it is left alone after an error rather than spending the limiter on retries.
const SLOW_REFRESH_INTERVAL: Duration = Duration::from_secs(15);
const SLOW_RETRY_INTERVAL: Duration = Duration::from_secs(45);
const PAPER_QUOTE_FLUSH_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, PartialEq)]
pub enum WorkspaceAction {
    OpenPortfolio,
    /// Open the Script Manager tab.
    OpenPineScripts,
    OpenAlerts,
    /// The price-alert sheet's Save: queue the alert for the runtime. `baseline` is the price
    /// the sheet last showed.
    SavePriceAlert {
        alert: Box<PriceAlert>,
        baseline: Option<f64>,
    },
    /// The price-alert sheet's "Add in Settings".
    OpenWebhookSettings,
    /// Open Settings on the Alpaca page (Add Chart's "Set up Alpaca" notice).
    OpenAlpacaSettings,
    /// Pause or resume a price alert (its line on a chart).
    SetAlertState {
        id: crate::model::alert::AlertId,
        state: crate::model::alert::AlertState,
    },
    /// Delete a price alert (its line on a chart).
    DeleteAlert(crate::model::alert::AlertId),
    OpenWatchlistInNewTab(WatchlistInstrument),
}

/// What a saved workspace holds for one chart (defined in `model`; re-exported so the store and
/// importers can keep their `crate::workspace::PersistedChart` path).
pub use crate::model::PersistedChart;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PersistedWorkspace {
    pub id: u64,
    pub timeframe: Timeframe,
    pub watchlist_visible: bool,
    pub charts: Vec<PersistedChart>,
    pub columns: Vec<ChartColumn>,
    pub focused: Option<u64>,
    /// Which parts of horizontal navigation the charts share. Rows from before linked charts
    /// have none and open linked, the default.
    #[serde(default)]
    pub sync: ChartSyncSettings,
}

impl crate::store::Document for PersistedWorkspace {
    fn document_id(&self) -> String {
        self.id.to_string()
    }
}

/// The focused chart's identity, read by the Pine integration (alert subscription + tick loop).
pub struct PineChartTarget {
    pub chart_id: u64,
    pub instrument: InstrumentId,
    pub timeframe: Timeframe,
    pub instances: Vec<crate::model::pine_instance::PineInstance>,
}

/// What a remote caller (the MCP server) may see of one chart: identity and state, never candles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChartSummary {
    pub id: u64,
    pub instrument: InstrumentId,
    pub display_symbol: String,
    pub is_line_chart: bool,
    /// How the chart draws its bars (candles for a market that draws a line).
    pub chart_type: crate::model::ChartType,
    pub loading: bool,
    pub error: Option<String>,
    pub focused: bool,
    pub indicators: IndicatorConfig,
    /// `(instance id, script name)` of each applied Pine script.
    pub pine_instances: Vec<(u64, String)>,
}

/// What the MCP candle and drawing tools read off one market chart. Borrowed from the workspace,
/// so nothing is cloned until a tool decides what it returns.
pub struct ChartData<'a> {
    pub instrument: InstrumentId,
    pub chart_type: crate::model::ChartType,
    /// The market's own candles, cut at the replay position as the chart shows them.
    pub candles: &'a [Kline],
    /// The replay position, when replay hides the bars after it.
    pub replay_ms: Option<i64>,
    pub drawings: &'a [Drawing],
    /// The id the chart's next new drawing takes.
    pub next_drawing_id: u64,
}

/// The result of [`ChartWorkspace::add_chart_for`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AddChartOutcome {
    pub chart_id: u64,
    /// False when the market was already charted here and that chart was reused.
    pub created: bool,
}

/// What a chart's run of one applied Pine script has produced so far. A script runs only while its
/// chart is drawn, so a chart in a window nobody has looked at yet has none.
#[derive(Clone, Debug)]
pub struct PineRunView {
    /// The last run's failure; the chart keeps showing the previous good output meanwhile.
    pub error: Option<crate::pine::token::Diagnostic>,
    pub bar_count: usize,
    /// What the script lets the user restyle (empty until it has run).
    pub style_rows: Vec<crate::pine::style_overrides::StyleRow>,
    pub is_strategy: bool,
    /// The backtest report, only when it was asked for (it can be large).
    pub strategy: Option<crate::pine::strategy::StrategyReport>,
}

/// One Pine script applied to a chart, with the context it runs in.
#[derive(Clone, Debug)]
pub struct PineInstanceView {
    pub chart_id: u64,
    pub instrument: InstrumentId,
    pub timeframe: Timeframe,
    pub instance: crate::model::pine_instance::PineInstance,
    pub run: Option<PineRunView>,
}

/// What [`ChartWorkspace::apply_indicator`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndicatorOutcome {
    Added,
    /// Already applied; it was shown again or its EMA period changed.
    Updated,
    AlreadyApplied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndicatorError {
    NoSuchChart,
    /// Volume on a line chart (prediction markets, CoinMarketCap).
    UnsupportedOnLineChart,
    /// A period for an indicator that has none (only EMA takes one).
    NoParameters,
    /// An EMA period outside `EMA_PERIODS`.
    InvalidEmaPeriod,
}

/// One chart's runtime state: market data, fetch result, and view.
/// A new price alert to start: the market, and the price a chart click suggests for it.
struct AlertRequest {
    asset: AlertAsset,
    /// The clicked price to prefill, when the request came from a chart point.
    price: Option<f64>,
    /// The chart's last price, to choose the way the alert crosses.
    last: Option<f64>,
}

impl AlertRequest {
    fn market(asset: AlertAsset) -> Self {
        Self {
            asset,
            price: None,
            last: None,
        }
    }
}

struct Chart {
    id: u64,
    symbol: String,
    source: DataSource,
    candles: Vec<Kline>,
    loading: bool,
    error: Option<String>,
    last_fetch: Instant,
    inbox: Option<Receiver<Result<Fetched, String>>>,
    /// What is known of the history behind the oldest candle, and the request for it, if any.
    older: OlderHistory,
    older_inbox: Option<OlderRequest>,
    fetch_generation: Arc<AtomicU64>,
    chart: ChartCard,
    log_scale: bool,
    indicators: IndicatorConfig,
    drawings: Vec<Drawing>,
    next_drawing_id: u64,
    last_paper_quote: Option<(f64, Instant)>,
    last_live_open_time: Option<i64>,
    /// Prediction markets: the choices charted together as lines, when there are several.
    pm_series: Vec<PmSeries>,
    /// Provider artwork carried into watchlists without another metadata search.
    image_url: Option<String>,
    /// A portfolio card instead of a market: `symbol` is the card's key and `source` a
    /// placeholder nothing reads. Cards never fetch, stream, trade or run scripts.
    card: Option<PortfolioCard>,
    /// The card's own price demand and feeds, made the first frame it is drawn with an app that
    /// has portfolios (`ChartWorkspace::set_card_env`). Dropping it releases the demand.
    card_runtime: Option<CardRuntime>,
}

/// What one fetch brought back: the candles the chart reads, and for a multi-choice prediction
/// market each choice's own history.
struct Fetched {
    candles: Vec<Kline>,
    series: Vec<(String, Vec<Kline>)>,
}

impl Fetched {
    /// The candles come from the first choice that has any; the rest only draw their lines.
    fn from_choices(histories: Vec<(String, Vec<Kline>)>) -> Result<Self, String> {
        let candles = histories
            .iter()
            .find(|(_, history)| !history.is_empty())
            .map(|(_, history)| history.clone())
            .ok_or_else(|| "No price history for these choices".to_owned())?;
        Ok(Self {
            candles,
            series: histories,
        })
    }
}

impl Chart {
    fn new(
        id: u64,
        symbol: impl Into<String>,
        source: DataSource,
        timeframe: Timeframe,
        ctx: &egui::Context,
    ) -> Self {
        let mut chart = Self::blank(id, symbol, source);
        chart.fetch(timeframe, ctx);
        chart
    }

    /// A chart that has not fetched yet, for callers that set up its prediction choices first.
    fn blank(id: u64, symbol: impl Into<String>, source: DataSource) -> Self {
        Self {
            id,
            symbol: symbol.into(),
            source,
            candles: Vec::new(),
            loading: false,
            error: None,
            last_fetch: Instant::now(),
            inbox: None,
            older: OlderHistory::default(),
            older_inbox: None,
            fetch_generation: Arc::new(AtomicU64::new(0)),
            chart: ChartCard::default(),
            log_scale: false,
            indicators: IndicatorConfig::default(),
            drawings: Vec::new(),
            next_drawing_id: 1,
            last_paper_quote: None,
            last_live_open_time: None,
            pm_series: Vec::new(),
            image_url: None,
            card: None,
            card_runtime: None,
        }
    }

    /// A portfolio card. It has no market data, so it never fetches.
    fn portfolio_card(id: u64, card: PortfolioCard) -> Self {
        let mut chart = Self::blank(id, card.key(), DataSource::Binance);
        chart.card = Some(card);
        chart
    }

    /// Whether this is a market chart (candles, indicators, alerts, scripts) rather than a
    /// portfolio card. Every loop that reads market data skips the cards.
    fn is_market(&self) -> bool {
        self.card.is_none()
    }

    /// The name shown for the chart: an explicit display title, else its provider symbol.
    fn display_title(&self) -> String {
        if let Some(card) = &self.card {
            return card.kind.title().to_owned();
        }
        self.chart
            .title()
            .map_or_else(|| self.source.display_symbol(&self.symbol), str::to_owned)
    }

    /// Titles a prediction market and sets the choices it draws as lines (none for a single one).
    fn set_prediction(&mut self, title: String, series: Vec<PmSeries>) {
        self.chart.set_title(Some(title));
        self.pm_series = if series.len() > 1 { series } else { Vec::new() };
    }

    /// Applies the display metadata retained by a watchlist row.
    fn apply_watchlist_metadata(&mut self, item: &WatchlistInstrument) {
        let source = item.instrument.source;
        self.chart.set_title(
            (source == DataSource::DexScreener || source.is_prediction_market()).then(|| {
                item.display_name
                    .clone()
                    .unwrap_or_else(|| item.name.clone())
            }),
        );
        self.image_url = item.image_url.clone();
    }

    /// Replaces the market-specific state while preserving this card's presentation settings.
    fn replace_market(&mut self, item: &WatchlistInstrument) {
        self.symbol = item.instrument.symbol.clone();
        self.source = item.instrument.source;
        self.apply_watchlist_metadata(item);
        self.chart.set_series(Vec::new());
        self.pm_series.clear();
        self.last_paper_quote = None;
        self.last_live_open_time = None;
        self.candles.clear();
        self.reset_history();
    }

    fn restored(saved: PersistedChart, timeframe: Timeframe, ctx: &egui::Context) -> Self {
        let instances = saved.instances();
        let mut chart = Self::blank(saved.id, saved.symbol, saved.source);
        chart.card = saved.card;
        chart.chart.set_title(saved.title);
        chart.pm_series = saved.pm_series;
        chart.image_url = saved.image_url;
        chart.fetch(timeframe, ctx);
        chart.log_scale = saved.log_scale;
        chart.chart.set_y_zoom(saved.y_zoom);
        chart.chart.set_appearance(saved.appearance);
        chart.chart.set_pine_instances(instances);
        chart.chart.set_visible_candles(saved.candle_count as usize);
        chart.chart.restore_viewport(saved.viewport);
        chart.indicators = saved.indicators;
        chart.drawings = saved.drawings;
        chart.next_drawing_id = chart
            .drawings
            .iter()
            .map(Drawing::stored_id)
            .max()
            .map_or(1, |max| max + 1);
        chart
    }

    fn fetch(&mut self, timeframe: Timeframe, ctx: &egui::Context) {
        if !self.is_market() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let symbol = self.symbol.clone();
        let source = self.source;
        let interval = timeframe.interval();
        let choices = self.pm_series.clone();
        let repaint = ctx.clone();

        let generation = self.fetch_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let current = Arc::downgrade(&self.fetch_generation);
        service::work_queue::submit_for(source, move || {
            if !current
                .upgrade()
                .is_some_and(|state| state.load(Ordering::Acquire) == generation)
            {
                return;
            }
            let result = if choices.is_empty() {
                service::fetch_klines(source, &symbol, interval, CANDLE_LIMIT).map(|candles| {
                    Fetched {
                        candles,
                        series: Vec::new(),
                    }
                })
            } else {
                Fetched::from_choices(service::fetch_choices(
                    source,
                    &choices,
                    interval,
                    CANDLE_LIMIT,
                ))
            };
            let _ = sender.send(result);
            repaint.request_repaint();
        });

        self.inbox = Some(receiver);
        self.loading = true;
        self.last_fetch = Instant::now();
    }

    /// How long after a fetch began the next one is due.
    fn refresh_after(&self) -> Duration {
        match self.source {
            DataSource::CoinGecko | DataSource::DexScreener if self.error.is_some() => {
                SLOW_RETRY_INTERVAL
            }
            DataSource::CoinGecko | DataSource::DexScreener => SLOW_REFRESH_INTERVAL,
            _ => REFRESH_INTERVAL,
        }
    }

    /// Forgets what was learnt of the history behind the candles: they are another market's or
    /// timeframe's now.
    fn reset_history(&mut self) {
        self.older = OlderHistory::default();
        self.older_inbox = None;
    }

    /// Asks the provider for the page before the oldest candle once the view has come near it.
    fn request_older(&mut self, timeframe: Timeframe, ctx: &egui::Context) {
        if !self.is_market() || !self.pm_series.is_empty() {
            return;
        }
        let Some(end) = history::page_end(&self.candles) else {
            return;
        };
        let wanted = self.older.should_request(
            self.source.supports_older_history(timeframe),
            self.chart.near_oldest_bar() || self.older.untouched(),
            self.candles.len(),
            Instant::now(),
        );
        if !wanted {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let (source, symbol) = (self.source, self.symbol.clone());
        let interval = timeframe.interval();
        let repaint = ctx.clone();
        let asked = symbol.clone();
        service::work_queue::submit_for(source, move || {
            let page = service::fetch_klines_range(source, &asked, interval, 0, end, history::PAGE);
            let _ = sender.send(page);
            repaint.request_repaint();
        });
        self.older.started();
        self.older_inbox = Some(OlderRequest {
            receiver,
            source,
            symbol,
            timeframe,
        });
    }

    fn receive_older(&mut self, timeframe: Timeframe) {
        let Some(request) = &self.older_inbox else {
            return;
        };
        let result = match request.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Older candles stopped loading".to_owned())
            }
        };
        let current = request.source == self.source
            && request.symbol == self.symbol
            && request.timeframe == timeframe;
        self.older_inbox = None;
        if !current {
            self.older.abandoned();
            return;
        }
        match result {
            Ok(page) => {
                let added = history::prepend(&mut self.candles, page);
                self.older.arrived(added);
            }
            Err(_) => self.older.failed(Instant::now()),
        }
    }

    fn receive_fetch(&mut self) {
        let Some(receiver) = &self.inbox else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Chart fetch stopped before returning data".to_owned())
            }
        };

        self.loading = false;
        self.inbox = None;
        match result {
            Ok(fetched) => {
                let fresh =
                    merge_fetched_candles(fetched.candles, &self.candles, self.last_live_open_time);
                self.candles = history::with_older(&self.candles, fresh, self.older.keeps_older());
                if !self.pm_series.is_empty() {
                    self.chart
                        .set_series(SeriesLine::build(&self.pm_series, fetched.series));
                }
                self.error = None;
            }
            Err(message) => self.error = Some(message),
        }
    }
}

/// A request for the candles before the oldest one, with what it was asked for: an answer for a
/// market or timeframe the chart has left is dropped.
struct OlderRequest {
    receiver: Receiver<Result<Vec<Kline>, String>>,
    source: DataSource,
    symbol: String,
    timeframe: Timeframe,
}

/// A websocket update for the current or a later bar wins over an older REST snapshot.
fn merge_fetched_candles(
    mut fetched: Vec<Kline>,
    current: &[Kline],
    last_live_open_time: Option<i64>,
) -> Vec<Kline> {
    if let Some(live_open) = last_live_open_time
        && let Some(last_fetched) = fetched.last()
    {
        let newer = current
            .partition_point(|candle| candle.open_time < live_open.max(last_fetched.open_time));
        if newer < current.len() {
            let first_newer = current[newer].open_time;
            fetched.retain(|candle| candle.open_time < first_newer);
            fetched.extend_from_slice(&current[newer..]);
        }
    }
    if fetched.len() > CANDLE_LIMIT as usize {
        fetched.drain(..fetched.len() - CANDLE_LIMIT as usize);
    }
    fetched
}

/// A deferred mutation the grid collects while drawing, applied after the pass.
enum ChartCommand {
    Focus(u64),
    Settings(u64),
    Remove(u64),
    Indicator(u64, crate::ui::chart_indicator_strip::Action),
    Move {
        chart_id: u64,
        to_column: u64,
        before: Option<u64>,
    },
    NewColumn(u64),
    /// The chart-type selector of one chart card was used.
    SetChartType(u64, crate::model::ChartType),
    /// A row of one chart card's camera menu was picked.
    Snapshot(u64, crate::model::snapshot::SnapshotAction),
}

/// A reversible change to one chart: what it held before, so undo can restore it. The stack is
/// the workspace's, in the order things happened, whatever they changed.
struct UndoEntry {
    chart_id: u64,
    before: Snapshot,
}

/// The part of a chart an undoable change touched.
enum Snapshot {
    /// The chart's drawings before a batch of commands (rulers excluded).
    Drawings(Vec<Drawing>),
    /// The applied built-in indicators and the scripts on the chart.
    Indicators {
        config: IndicatorConfig,
        scripts: Vec<crate::model::pine_instance::PineInstance>,
    },
}

impl Snapshot {
    /// Puts this snapshot on `chart` and returns what the chart held, to undo the undo.
    fn swap_into(self, chart: &mut Chart) -> Snapshot {
        match self {
            Self::Drawings(drawings) => {
                let current = std::mem::replace(&mut chart.drawings, drawings);
                Self::Drawings(current.into_iter().filter(|d| !is_ruler(d)).collect())
            }
            Self::Indicators { config, scripts } => {
                let current = Self::Indicators {
                    config: chart.indicators,
                    scripts: chart.chart.pine_instances().to_vec(),
                };
                chart.indicators = config;
                chart.chart.set_pine_instances(scripts);
                current
            }
        }
    }
}

/// Rulers are transient measurements: never persisted, never in undo history.
fn is_ruler(drawing: &Drawing) -> bool {
    drawing.is_ruler()
}

/// The drawings that are saved: rulers are never part of a layout.
fn plain_drawings(drawings: &[Drawing]) -> Vec<Drawing> {
    drawings.iter().filter(|d| !is_ruler(d)).cloned().collect()
}

/// Three-way merge of one chart's drawings by id. `base` is what this tab last saved or opened,
/// `local` what it shows now and `remote` what the layout holds after another Mac's edit. A change
/// on one side wins over no change on the other; when both changed the same drawing, this tab's
/// version wins and is saved over the other (last writer). Local order is kept, new drawings
/// from the other Mac follow in their order.
///
/// A drawing this version cannot read (a tool from a newer build) is matched by the id it was
/// stored with, since `Drawing::id` is 0 for all of them; one with no readable id is matched by
/// its content.
fn merge_remote_drawings(base: &[Drawing], local: &[Drawing], remote: &[Drawing]) -> Vec<Drawing> {
    let same = |a: &Drawing, b: &Drawing| match (a.stored_id(), b.stored_id()) {
        (0, 0) => a == b,
        (left, right) => left == right,
    };
    let find = |list: &[Drawing], like: &Drawing| list.iter().find(|d| same(d, like)).cloned();
    let mut merged = Vec::new();
    for drawing in local {
        let (old, theirs) = (find(base, drawing), find(remote, drawing));
        let local_unchanged = old.as_ref() == Some(drawing);
        match theirs {
            Some(theirs) if local_unchanged => merged.push(theirs),
            // Removed on the other Mac and untouched here.
            None if local_unchanged => {}
            _ => merged.push(drawing.clone()),
        }
    }
    for drawing in remote {
        if find(local, drawing).is_none() && find(base, drawing).is_none() {
            merged.push(drawing.clone());
        }
    }
    merged
}

/// How long after a restyle another of the same drawing still counts as the same edit.
const RESTYLE_WINDOW: Duration = Duration::from_millis(800);

/// The drawing a batch restyles, when that is all the batch does.
fn restyled_drawing(commands: &[DrawingCommand]) -> Option<u64> {
    match commands {
        [DrawingCommand::Style { id, .. }] => Some(*id),
        _ => None,
    }
}

/// Whether restyling `drawing_id` of `chart_id` at `now` continues `last`.
fn restyle_merges(
    last: Option<(u64, u64, Instant)>,
    chart_id: u64,
    drawing_id: u64,
    now: Instant,
) -> bool {
    last.is_some_and(|(chart, drawing, at)| {
        chart == chart_id
            && drawing == drawing_id
            && now.saturating_duration_since(at) <= RESTYLE_WINDOW
    })
}

/// Drawings the strip's lock and hide controls act on: everything but the transient ruler and
/// rows this version cannot read.
fn is_lockable(drawing: &Drawing) -> bool {
    !is_ruler(drawing) && drawing.tool().is_some()
}

fn displayed_candles(candles: &[Kline], replay_ms: Option<i64>) -> &[Kline] {
    let Some(replay_ms) = replay_ms else {
        return candles;
    };
    let end = candles.partition_point(|candle| candle.open_time <= replay_ms);
    &candles[..end]
}

/// The slow inputs of the empty page: the recents setting, every saved layout and whether
/// Alpaca is configured (a Keychain read). Reloaded when the page first shows and when the
/// window regains focus, as Swift's `onAppear` and `didBecomeKey` do, so a layout saved in
/// another tab appears without a read on every repaint.
#[derive(Default)]
struct EmptyPageCache {
    data: Option<EmptyPageData>,
    was_focused: bool,
    /// When the page last drew. A tab behind another in its window is not drawn, so a long gap
    /// means it was just brought to the front (a frame is requested every 250 ms while it shows).
    last_drawn: Option<std::time::Instant>,
}

/// Longer than the repaint interval, shorter than any deliberate tab switch.
const EMPTY_PAGE_REVEAL_GAP: Duration = Duration::from_millis(900);

struct EmptyPageData {
    recents: Vec<add_chart_sheet::RecentMarket>,
    layouts: Vec<empty_state::LayoutSummary>,
    offers_stocks: bool,
}

pub struct ChartWorkspace {
    id: u64,
    timeframe: Timeframe,
    charts: Vec<Chart>,
    columns: Vec<ChartColumn>,
    /// Which parts of horizontal navigation the charts share. Saved with the layout.
    sync: ChartSyncSettings,
    /// Carries one chart's navigation to the others of this workspace, and no further.
    sync_hub: SyncHub,
    focused: Option<u64>,
    next_id: u64,
    watchlist_visible: bool,
    baseline: Option<LayoutSnapshot>,
    /// Bumped by every change to a chart's drawings; see `drawings_dirty`.
    drawings_version: u64,
    saved_drawings_version: u64,
    /// Each chart's drawings as of the last save or open: the common ancestor when another Mac's
    /// edit to the open layout arrives (`adopt_remote_layout`).
    saved_drawings: HashMap<u64, Vec<Drawing>>,
    add_chart: AddChartSheet,
    add_to_watchlist: Option<EntryId>,
    unseen_alerts: usize,
    /// How portfolio cards reach the app's portfolios; set by the window that draws this tab.
    card_env: Option<PortfolioCardEnv>,
    live_feed: Option<BinanceLiveFeed>,
    settings_chart: Option<u64>,
    /// The Inputs and Style popover of an applied script, up beside its chip or settings row.
    script_popover: Option<PineSettingsPopover>,
    /// The open properties dialog of a drawing.
    drawing_props: Option<DrawingProperties>,
    /// Saved drawing templates, each tool's default style and the strip's remembered options.
    /// Opened on first use.
    drawing_templates: Option<DrawingTemplateStore>,
    /// The strip's options as last saved, to notice a change.
    saved_prefs: DrawingPrefs,
    /// The style last copied from a drawing, for Paste style.
    style_clipboard: Option<(Tool, DrawingStyle)>,
    /// A chart whose drawings the user asked to remove all of; waiting for the confirmation.
    confirm_remove_drawings: Option<u64>,
    /// The last restyle, so a burst of them (dragging a colour picker) is one undo step.
    last_restyle: Option<(u64, u64, Instant)>,
    /// The chart a header ✕ asked to remove, awaiting confirmation.
    removing_chart: Option<u64>,
    settings_state: ChartSettingsState,
    crosshair: Option<CrosshairInfo>,
    active_tool: Tool,
    /// The selected drawings of one chart: `(chart id, drawing ids)`.
    selected_drawing: Option<(u64, Vec<u64>)>,
    /// The tool the strip's grouped brushes, arrows and shapes tile shows: the last one used.
    group_tool: Tool,
    line_tool: Tool,
    /// How drawings snap to the bars' prices (the strip's magnet).
    magnet: MagnetMode,
    /// Stay on a drawing tool after a drawing is placed; off returns to the crosshair.
    keep_drawing: bool,
    /// Every drawing is hidden (view state only: nothing is changed or saved).
    hide_all_drawings: bool,
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
    replay: ReplayEngine,
    replay_last_step: Instant,
    watchlist: Arc<Mutex<WatchlistStore>>,
    watchlist_sidebar: WatchlistSidebarState,
    date_sheet: ReplayDateSheetState,
    quote_book: WatchlistQuoteBook,
    quote_coordinator: WatchlistQuoteCoordinator,
    import_report: Option<ImportReport>,
    watchlist_error: Option<String>,
    watchlist_file_result: Option<Receiver<Result<(), String>>>,
    paper_panel_visible: bool,
    /// A chart's bell (or a watchlist row's menu) asked for a price alert this frame; `ui`
    /// opens the sheet on it.
    new_alert_request: Option<AlertRequest>,
    /// The chart menu's state: the open menu, its messages, and the panels it toggles.
    context: ContextState,
    /// The snapshot being made, and what it waits for.
    snapshot: SnapshotState,
    /// The snapshot settings all windows share (set by the app).
    snapshots: Option<crate::snapshot_host::SnapshotHost>,
    /// The "Create Price Alert" sheet, over this tab's window.
    alert_editor: Option<PriceAlertEditorState>,
    /// USD → currency rates for the sheet's currency menu, pushed in by the app.
    alert_fx_rates: HashMap<AlertCurrency, f64>,
    open_new_tab_request: Option<WatchlistInstrument>,
    /// What the empty page reads from the database and Keychain, loaded once per visit.
    empty_page: EmptyPageCache,
    paper_panel: PaperPanel,
    /// Saved-layout persistence + the tab's relation to the layout library.
    store: SharedStore,
    active_view_id: Option<u64>,
    active_view_name: Option<String>,
    /// The name prompt that is open, if any, and the text in it.
    layout_prompt: Option<NamePrompt>,
    prompt_name: String,
    prompt_focus: bool,
    /// A layout waiting for the "save changes?" answer before it opens.
    pending_open_id: Option<u64>,
    /// Why the last layout operation failed, until the alert is dismissed.
    layout_error: Option<String>,
    autosave: AutosaveState,
    /// The chart drag in progress over the grid: where it would land, and the new-column preview.
    grid_drag: ui::grid_drop::GridDrag,
}

/// How long the layout must sit unchanged before an autosave-enabled layout is written.
const AUTOSAVE_DELAY: Duration = Duration::from_secs(2);

/// What the debounce wants done this frame.
#[derive(Debug, Eq, PartialEq)]
enum AutosaveStep {
    Nothing,
    /// Check again after this long.
    Wait(Duration),
    Save,
}

/// The debounce behind autosave. `armed` is the last layout seen dirty; `due` is when it gets
/// written, or `None` once it has been tried, so an unchanged layout is never retried in a loop.
#[derive(Default)]
struct AutosaveState {
    armed: Option<LayoutSnapshot>,
    due: Option<Instant>,
    /// The last autosave did not reach disk; the layout is still dirty.
    failed: bool,
}

impl AutosaveState {
    /// Advances the debounce: an edit restarts the countdown, and a layout that has sat unchanged
    /// for [`AUTOSAVE_DELAY`] is due. Once tried, a layout is not retried until it changes again.
    fn step(
        &mut self,
        dirty: bool,
        enabled: bool,
        snapshot: impl FnOnce() -> LayoutSnapshot,
        now: Instant,
    ) -> AutosaveStep {
        if !dirty || !enabled {
            self.armed = None;
            self.due = None;
            return AutosaveStep::Nothing;
        }
        let snapshot = snapshot();
        if self.armed.as_ref() != Some(&snapshot) {
            self.armed = Some(snapshot);
            self.due = Some(now + AUTOSAVE_DELAY);
        }
        match self.due {
            None => AutosaveStep::Nothing,
            Some(due) if now < due => AutosaveStep::Wait(due - now),
            Some(_) => {
                self.due = None;
                AutosaveStep::Save
            }
        }
    }
}

/// What the Pine alert runtime needs from one chart: its market, the bars its scripts run over
/// and the scripts. Those are the market's candles, or the bars the chart type constructs from
/// them, so a script's alert sees what the chart shows.
pub struct ChartFeed {
    pub chart_id: u64,
    pub symbol_key: String,
    pub timeframe: String,
    pub bars: Vec<Kline>,
    pub run: crate::pine_subscription_host::ChartRun,
    pub instances: Vec<crate::model::pine_instance::PineInstance>,
}

impl ChartWorkspace {
    #[allow(clippy::too_many_arguments)]
    pub fn new_for_watchlist(
        id: u64,
        item: &WatchlistInstrument,
        watchlist: Arc<Mutex<WatchlistStore>>,
        quote_book: WatchlistQuoteBook,
        quote_coordinator: WatchlistQuoteCoordinator,
        store: SharedStore,
        ctx: &egui::Context,
    ) -> Self {
        if let Some(card) = item.card {
            let mut workspace = Self::empty(
                id,
                Timeframe::Day,
                watchlist,
                quote_book,
                quote_coordinator,
                store,
            );
            workspace.add_card(card);
            workspace.set_baseline();
            return workspace;
        }
        let mut workspace = Self::new_with_source(
            id,
            &item.instrument.symbol,
            item.instrument.source,
            Timeframe::Day,
            watchlist,
            quote_book,
            quote_coordinator,
            store,
            ctx,
        );
        // Prediction-market and DEX symbols are opaque ids; carry the row's name and artwork so
        // the tab title and header match the row.
        if let Some(chart) = workspace.charts.first_mut() {
            chart.apply_watchlist_metadata(item);
        }
        workspace.set_baseline();
        workspace
    }

    /// A tab with no chart yet: it shows the empty page (search, favorites, recents, saved
    /// layouts) until one is added, the way a fresh Swift tab does.
    pub fn empty(
        id: u64,
        timeframe: Timeframe,
        watchlist: Arc<Mutex<WatchlistStore>>,
        quote_book: WatchlistQuoteBook,
        quote_coordinator: WatchlistQuoteCoordinator,
        store: SharedStore,
    ) -> Self {
        let mut workspace = Self::blank(
            id,
            timeframe,
            watchlist,
            quote_book,
            quote_coordinator,
            store,
        );
        workspace.set_baseline();
        workspace
    }

    fn blank(
        id: u64,
        timeframe: Timeframe,
        watchlist: Arc<Mutex<WatchlistStore>>,
        quote_book: WatchlistQuoteBook,
        quote_coordinator: WatchlistQuoteCoordinator,
        store: SharedStore,
    ) -> Self {
        Self {
            id,
            timeframe,
            charts: Vec::new(),
            columns: Vec::new(),
            sync: ChartSyncSettings::default(),
            sync_hub: SyncHub::default(),
            focused: None,
            next_id: 1,
            watchlist_visible: false,
            baseline: None,
            add_chart: AddChartSheet::new(store.clone()),
            add_to_watchlist: None,
            unseen_alerts: 0,
            card_env: None,
            live_feed: None,
            settings_chart: None,
            script_popover: None,
            drawing_props: None,
            drawing_templates: None,
            saved_prefs: DrawingPrefs::default(),
            style_clipboard: None,
            confirm_remove_drawings: None,
            last_restyle: None,
            removing_chart: None,
            settings_state: ChartSettingsState::default(),
            crosshair: None,
            active_tool: Tool::Cursor,
            selected_drawing: None,
            group_tool: Tool::Brush,
            line_tool: Tool::TrendLine,
            magnet: MagnetMode::Off,
            keep_drawing: true,
            hide_all_drawings: false,
            undo: Vec::new(),
            redo: Vec::new(),
            replay: ReplayEngine::default(),
            replay_last_step: Instant::now(),
            watchlist,
            watchlist_sidebar: WatchlistSidebarState::default(),
            date_sheet: ReplayDateSheetState::default(),
            quote_book,
            quote_coordinator,
            import_report: None,
            watchlist_error: None,
            watchlist_file_result: None,
            paper_panel_visible: false,
            new_alert_request: None,
            context: ContextState::default(),
            snapshot: SnapshotState::default(),
            snapshots: None,
            alert_editor: None,
            alert_fx_rates: HashMap::new(),
            open_new_tab_request: None,
            empty_page: EmptyPageCache::default(),
            paper_panel: PaperPanel::default(),
            store,
            active_view_id: None,
            active_view_name: None,
            layout_prompt: None,
            prompt_name: String::new(),
            prompt_focus: false,
            pending_open_id: None,
            layout_error: None,
            autosave: AutosaveState::default(),
            drawings_version: 0,
            saved_drawings_version: 0,
            saved_drawings: HashMap::new(),
            grid_drag: ui::grid_drop::GridDrag::default(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn new_with_source(
        id: u64,
        symbol: impl Into<String>,
        source: DataSource,
        timeframe: Timeframe,
        watchlist: Arc<Mutex<WatchlistStore>>,
        quote_book: WatchlistQuoteBook,
        quote_coordinator: WatchlistQuoteCoordinator,
        store: SharedStore,
        ctx: &egui::Context,
    ) -> Self {
        let mut workspace = Self::blank(
            id,
            timeframe,
            watchlist,
            quote_book,
            quote_coordinator,
            store,
        );
        let chart_id = workspace.fresh_id();
        workspace
            .charts
            .push(Chart::new(chart_id, symbol, source, timeframe, ctx));
        let chart_ids = [chart_id];
        workspace.columns = ChartColumn::resolved(None, &chart_ids, &mut workspace.next_id);
        workspace.focused = Some(chart_id);
        workspace.set_baseline();
        workspace
    }

    pub fn restored(
        saved: PersistedWorkspace,
        watchlist: Arc<Mutex<WatchlistStore>>,
        quote_book: WatchlistQuoteBook,
        quote_coordinator: WatchlistQuoteCoordinator,
        store: SharedStore,
        ctx: &egui::Context,
    ) -> Self {
        let PersistedWorkspace {
            id,
            timeframe,
            watchlist_visible,
            charts,
            columns,
            focused,
            sync,
        } = saved;

        let mut workspace = Self::blank(
            id,
            timeframe,
            watchlist,
            quote_book,
            quote_coordinator,
            store,
        );
        workspace.watchlist_visible = watchlist_visible;
        workspace.sync = sync;
        workspace.focused = focused;
        workspace.charts = charts
            .into_iter()
            .map(|saved| Chart::restored(saved, timeframe, ctx))
            .collect();
        workspace.next_id = workspace
            .charts
            .iter()
            .map(|chart| chart.id)
            .max()
            .map_or(1, |max| max + 1);
        let chart_ids: Vec<u64> = workspace.charts.iter().map(|chart| chart.id).collect();
        workspace.columns =
            ChartColumn::resolved(Some(&columns), &chart_ids, &mut workspace.next_id);
        if workspace
            .focused
            .is_none_or(|id| !workspace.charts.iter().any(|c| c.id == id))
        {
            workspace.focused = workspace.charts.first().map(|chart| chart.id);
        }
        workspace.set_baseline();
        workspace
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    /// How many charts show a market: the ones that can be linked (cards show no time axis).
    fn market_charts(&self) -> usize {
        self.charts.iter().filter(|chart| chart.is_market()).count()
    }

    /// Links or unlinks the charts. Nothing moves when a setting turns on: it takes effect with
    /// the next navigation, and a setting that turns off leaves every chart where it is.
    pub fn set_sync(&mut self, sync: ChartSyncSettings) {
        if self.sync != sync {
            self.sync = sync;
            self.sync_hub.reset();
        }
    }

    pub fn timeframe(&self) -> Timeframe {
        self.timeframe
    }

    pub fn chart_count(&self) -> usize {
        self.charts.len()
    }

    /// Every chart's identity and state, in layout order (column by column). No market data.
    pub fn chart_summaries(&self) -> Vec<ChartSummary> {
        let focused = self.resolved_focused_id();
        let ordered = self
            .columns
            .iter()
            .flat_map(|column| column.chart_ids.iter())
            .filter_map(|id| self.charts.iter().find(|chart| chart.id == *id))
            .filter(|chart| chart.is_market());
        ordered
            .map(|chart| ChartSummary {
                id: chart.id,
                instrument: InstrumentId::new(chart.source, &chart.symbol),
                display_symbol: chart.source.display_symbol(&chart.symbol),
                is_line_chart: chart.source.is_line_chart(),
                chart_type: chart.chart.effective_chart_type(chart.source),
                loading: chart.loading,
                error: chart.error.clone(),
                focused: focused == Some(chart.id),
                indicators: chart.indicators,
                pine_instances: chart
                    .chart
                    .pine_instances()
                    .iter()
                    .map(|instance| (instance.id, instance.name.clone()))
                    .collect(),
            })
            .collect()
    }

    /// A market chart's candles and drawings, for the MCP tools. `None` for an unknown chart or a
    /// portfolio card.
    pub fn chart_data(&self, chart_id: u64) -> Option<ChartData<'_>> {
        let chart = self
            .charts
            .iter()
            .find(|chart| chart.id == chart_id && chart.is_market())?;
        let replay_ms = if self.replay.is_active() {
            self.replay.current_ms()
        } else {
            None
        };
        Some(ChartData {
            instrument: InstrumentId::new(chart.source, &chart.symbol),
            chart_type: chart.chart.effective_chart_type(chart.source),
            candles: displayed_candles(&chart.candles, replay_ms),
            replay_ms,
            drawings: &chart.drawings,
            next_drawing_id: chart.next_drawing_id,
        })
    }

    /// Applies drawing commands an MCP tool planned from [`Self::chart_data`] through the same
    /// path as the pointer, so they are one undo step and are saved like any other edit.
    /// `next_free_id` is the first id the commands left unused; the chart's counter moves past it
    /// (0 when the commands mint none).
    pub fn apply_planned_drawings(
        &mut self,
        chart_id: u64,
        commands: Vec<DrawingCommand>,
        next_free_id: u64,
    ) {
        if let Some(chart) = self
            .charts
            .iter_mut()
            .find(|chart| chart.id == chart_id && chart.is_market())
        {
            chart.next_drawing_id = chart.next_drawing_id.max(next_free_id);
        }
        self.apply_drawing_commands(chart_id, commands);
    }

    /// The Pine scripts applied to a chart (or just `only` instance), with each one's latest run.
    /// `None` when the chart does not exist. Cloning the strategy report is opt-in.
    pub fn pine_instance_views(
        &self,
        chart_id: u64,
        only: Option<u64>,
        with_report: bool,
    ) -> Option<Vec<PineInstanceView>> {
        let chart = self.charts.iter().find(|chart| chart.id == chart_id)?;
        let instrument = InstrumentId::new(chart.source, &chart.symbol);
        Some(
            chart
                .chart
                .pine_instances()
                .iter()
                .filter(|instance| only.is_none_or(|id| instance.id == id))
                .map(|instance| PineInstanceView {
                    chart_id,
                    instrument: instrument.clone(),
                    timeframe: self.timeframe,
                    instance: instance.clone(),
                    run: chart
                        .chart
                        .pine_status(instance.id)
                        .map(|status| PineRunView {
                            error: status.error.cloned(),
                            bar_count: status.output.bar_count,
                            style_rows: chart.chart.pine_style_rows(instance.id),
                            is_strategy: status.output.strategy.is_some(),
                            strategy: if with_report {
                                status.output.strategy.clone()
                            } else {
                                None
                            },
                        }),
                })
                .collect(),
        )
    }

    /// Applies a built-in indicator to a chart, as the chart's indicator menu does: it is drawn
    /// (shown again if its eye was off), and an EMA takes `ema_period`. Each indicator exists
    /// once per chart, so applying one again only updates it. Returns the chart's resulting
    /// configuration.
    pub fn apply_indicator(
        &mut self,
        chart_id: u64,
        kind: BuiltIn,
        ema_period: Option<usize>,
    ) -> Result<(IndicatorOutcome, IndicatorConfig), IndicatorError> {
        let chart = self
            .charts
            .iter_mut()
            .find(|chart| chart.id == chart_id)
            .ok_or(IndicatorError::NoSuchChart)?;
        if !kind.supports_line_charts() && chart.source.is_line_chart() {
            return Err(IndicatorError::UnsupportedOnLineChart);
        }
        if let Some(period) = ema_period {
            if kind != BuiltIn::Ema {
                return Err(IndicatorError::NoParameters);
            }
            if !EMA_PERIODS.contains(&period) {
                return Err(IndicatorError::InvalidEmaPeriod);
            }
        }
        let before = chart.indicators;
        kind.set_applied(&mut chart.indicators, true);
        if let Some(period) = ema_period {
            chart.indicators.ema_period = period;
        }
        let outcome = if !kind.applied(&before) {
            IndicatorOutcome::Added
        } else if chart.indicators != before {
            IndicatorOutcome::Updated
        } else {
            IndicatorOutcome::AlreadyApplied
        };
        Ok((outcome, chart.indicators))
    }

    /// Adds a chart for `instrument` through the same path as the Add Chart sheet: an existing
    /// chart of the same market is reused (and focused) instead of duplicated, otherwise a new
    /// chart is placed in the grid, focused, and starts fetching.
    pub fn add_chart_for(
        &mut self,
        instrument: &InstrumentId,
        ctx: &egui::Context,
    ) -> AddChartOutcome {
        if let Some(existing) = self
            .charts
            .iter()
            .find(|chart| InstrumentId::new(chart.source, &chart.symbol).is_same_market(instrument))
        {
            let chart_id = existing.id;
            self.focus_chart(chart_id);
            return AddChartOutcome {
                chart_id,
                created: false,
            };
        }
        self.add_chart_with_symbol(&instrument.symbol, instrument.source, ctx);
        let chart_id = self.charts.last().map_or(0, |chart| chart.id);
        AddChartOutcome {
            chart_id,
            created: true,
        }
    }

    /// Adds a portfolio card to the grid, or focuses the one already showing the same kind for
    /// the same portfolio. Cards have no market, so nothing is fetched.
    pub fn add_card(&mut self, card: PortfolioCard) -> AddChartOutcome {
        if let Some(existing) = self
            .charts
            .iter()
            .find(|chart| chart.card.is_some_and(|shown| shown.same_card(&card)))
        {
            let chart_id = existing.id;
            self.focus_chart(chart_id);
            return AddChartOutcome {
                chart_id,
                created: false,
            };
        }
        let id = self.fresh_id();
        self.charts.push(Chart::portfolio_card(id, card));
        self.place_in_grid(id);
        self.focused = Some(id);
        AddChartOutcome {
            chart_id: id,
            created: true,
        }
    }

    fn fresh_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn resolved_focused_id(&self) -> Option<u64> {
        if let Some(id) = self.focused
            && self.charts.iter().any(|chart| chart.id == id)
        {
            return Some(id);
        }
        self.charts.first().map(|chart| chart.id)
    }

    pub fn title(&self) -> String {
        let Some(chart) = self
            .resolved_focused_id()
            .and_then(|id| self.charts.iter().find(|chart| chart.id == id))
        else {
            // No chart yet: the layout's name, or "New tab" for a blank page. (The layout menu
            // still calls an unsaved layout "Unnamed", as Swift does.)
            return self
                .active_view_name
                .clone()
                .unwrap_or_else(|| "New tab".to_owned());
        };
        if chart.card.is_some() {
            // A card has no timeframe to name.
            return chart.display_title();
        }
        format!("{} · {}", chart.display_title(), self.timeframe.label())
    }

    pub fn persisted(&self) -> PersistedWorkspace {
        PersistedWorkspace {
            id: self.id,
            timeframe: self.timeframe,
            watchlist_visible: self.watchlist_visible,
            charts: self
                .charts
                .iter()
                .map(|chart| PersistedChart {
                    id: chart.id,
                    symbol: chart.symbol.clone(),
                    source: chart.source,
                    log_scale: chart.log_scale,
                    y_zoom: chart.chart.y_zoom(),
                    indicators: chart.indicators,
                    appearance: chart.chart.appearance(),
                    drawings: chart
                        .drawings
                        .iter()
                        .filter(|drawing| !is_ruler(drawing))
                        .cloned()
                        .collect(),
                    pine_instances: chart.chart.pine_instances().to_vec(),
                    candle_count: chart.chart.visible_candles() as i64,
                    viewport: chart.chart.saved_viewport(),
                    title: chart.chart.title().map(str::to_owned),
                    pm_series: chart.pm_series.clone(),
                    image_url: chart.image_url.clone(),
                    card: chart.card,
                })
                .collect(),
            columns: self.columns.clone(),
            focused: self.focused,
            sync: self.sync,
        }
    }

    pub fn layout_snapshot(&self) -> LayoutSnapshot {
        LayoutSnapshot {
            timeframe: self.timeframe,
            configs: self
                .charts
                .iter()
                .map(|chart| ChartConfig {
                    id: chart.id,
                    symbol: chart.symbol.clone(),
                    source: chart.source,
                    log_scale: chart.log_scale,
                    appearance: chart.chart.appearance(),
                    card: chart.card,
                })
                .collect(),
            columns: self
                .columns
                .iter()
                .map(|column| column.chart_ids.clone())
                .collect(),
            sync: self.sync,
        }
    }

    /// Whether the layout differs from the last saved fingerprint. Zoom, focus,
    /// and watchlist edits do not count.
    pub fn is_dirty(&self) -> bool {
        self.baseline
            .as_ref()
            .is_some_and(|baseline| *baseline != self.layout_snapshot())
    }

    pub fn mark_clean(&mut self) {
        self.set_baseline();
    }

    /// Records the layout as saved: its fingerprint, and the drawings it holds.
    fn set_baseline(&mut self) {
        self.baseline = Some(self.layout_snapshot());
        self.saved_drawings_version = self.drawings_version;
        self.saved_drawings = self
            .charts
            .iter()
            .map(|chart| (chart.id, plain_drawings(&chart.drawings)))
            .collect();
    }

    /// Another Mac changed the layout this tab has open and iCloud Sync stored the merged result.
    /// Brings its drawings into the live charts without losing what was drawn here since the last
    /// save, and without writing anything back: without this the tab keeps showing (and with
    /// Autosave later saves) the old drawings, which would delete the other Mac's from iCloud.
    pub fn adopt_remote_layout(&mut self) {
        let Some(view) = self.active_view() else {
            return;
        };
        let was_dirty = self.drawings_dirty();
        let mut changed = false;
        let mut diverged = false;
        for saved in &view.charts {
            let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == saved.id) else {
                continue;
            };
            let remote = plain_drawings(&saved.drawings);
            let base = self
                .saved_drawings
                .get(&chart.id)
                .cloned()
                .unwrap_or_default();
            if base == remote {
                continue;
            }
            let local = plain_drawings(&chart.drawings);
            let merged = merge_remote_drawings(&base, &local, &remote);
            diverged |= merged != remote;
            if merged != local {
                let rulers: Vec<Drawing> = chart
                    .drawings
                    .iter()
                    .filter(|d| is_ruler(d))
                    .cloned()
                    .collect();
                if let Some(highest) = merged.iter().map(Drawing::stored_id).max() {
                    chart.next_drawing_id = chart.next_drawing_id.max(highest + 1);
                }
                chart.drawings = merged;
                chart.drawings.extend(rulers);
                // An undo step would put back drawings that were since added or removed elsewhere.
                let id = chart.id;
                let drawn = |entry: &UndoEntry| {
                    entry.chart_id == id && matches!(entry.before, Snapshot::Drawings(_))
                };
                self.undo.retain(|entry| !drawn(entry));
                self.redo.retain(|entry| !drawn(entry));
                changed = true;
            }
            self.saved_drawings.insert(chart.id, remote);
        }
        if changed {
            self.drawings_version += 1;
            // Nothing here beyond what is already stored: no save is due.
            if !was_dirty && !diverged {
                self.saved_drawings_version = self.drawings_version;
            }
        }
    }

    /// Drawings changed since the layout was last saved. The layout's fingerprint leaves drawings
    /// out (a drag must not mark a layout as edited), so an Autosave layout watches this too:
    /// without it a drawing would only reach the saved layout, and other Macs, by accident.
    fn drawings_dirty(&self) -> bool {
        self.drawings_version != self.saved_drawings_version
    }

    /// Adds a Pine script to the focused chart, after the ones already there (the Script
    /// Manager's "Add to Chart").
    pub fn add_pine_on_focused(&mut self, script_id: Option<String>, name: &str, source: &str) {
        if let Some(id) = self.resolved_focused_id()
            && let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == id)
        {
            chart.chart.add_pine_instance(script_id, name, source);
        }
    }

    /// Applies a saved script to a market chart for an agent, as the chart's Scripts tab does,
    /// with the inputs it was given. Undo takes it off again. `None` when the chart is not a
    /// market chart in this workspace.
    pub fn add_pine_instance_to(
        &mut self,
        chart_id: u64,
        script_id: String,
        name: &str,
        source: &str,
        inputs: &HashMap<String, crate::pine::program::InputValue>,
        visible: bool,
    ) -> Option<u64> {
        if !self
            .charts
            .iter()
            .any(|chart| chart.id == chart_id && chart.is_market())
        {
            return None;
        }
        self.remember_indicators(chart_id);
        let chart = self.charts.iter_mut().find(|chart| chart.id == chart_id)?;
        let id = chart.chart.add_pine_instance(Some(script_id), name, source);
        chart.chart.set_pine_instance_inputs(id, inputs);
        chart.chart.set_pine_visible(id, visible);
        Some(id)
    }

    /// Changes an applied script's inputs and/or visibility. Returns whether anything differed;
    /// only then does it take an undo step.
    pub fn change_pine_instance(
        &mut self,
        chart_id: u64,
        instance_id: u64,
        inputs: Option<&HashMap<String, crate::pine::program::InputValue>>,
        visible: Option<bool>,
    ) -> Option<bool> {
        let chart = self.charts.iter().find(|chart| chart.id == chart_id)?;
        let instance = chart
            .chart
            .pine_instances()
            .iter()
            .find(|instance| instance.id == instance_id)?;
        let changed = inputs.is_some_and(|inputs| *inputs != instance.input_values())
            || visible.is_some_and(|visible| visible != instance.visible);
        if changed {
            self.remember_indicators(chart_id);
            let chart = self.charts.iter_mut().find(|chart| chart.id == chart_id)?;
            if let Some(inputs) = inputs {
                chart.chart.set_pine_instance_inputs(instance_id, inputs);
            }
            if let Some(visible) = visible {
                chart.chart.set_pine_visible(instance_id, visible);
            }
        }
        Some(changed)
    }

    /// Takes an applied script off a chart (its saved source and alerts are left alone). Undo
    /// brings it back. Returns whether the instance existed.
    pub fn remove_pine_instance_from(&mut self, chart_id: u64, instance_id: u64) -> bool {
        let exists = self.charts.iter().any(|chart| {
            chart.id == chart_id
                && chart
                    .chart
                    .pine_instances()
                    .iter()
                    .any(|instance| instance.id == instance_id)
        });
        if exists {
            self.remember_indicators(chart_id);
            if let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == chart_id) {
                chart.chart.remove_pine_instance(instance_id);
            }
        }
        exists
    }

    /// The focused chart's identity for the Pine integration (alert subscription creation).
    pub fn focused_pine_target(&self) -> Option<PineChartTarget> {
        let id = self.resolved_focused_id()?;
        let chart = self.charts.iter().find(|chart| chart.id == id)?;
        Some(PineChartTarget {
            chart_id: chart.id,
            instrument: InstrumentId::new(chart.source, &chart.symbol),
            timeframe: self.timeframe,
            instances: chart.chart.pine_instances().to_vec(),
        })
    }

    /// Each chart's `(chart_id, symbol_key, timeframe)`, so script alerts can say whether their
    /// chart is open and still on their market.
    pub fn pine_datasets(&self) -> Vec<(u64, String, String)> {
        self.charts
            .iter()
            .filter(|chart| chart.is_market())
            .map(|chart| {
                (
                    chart.id,
                    InstrumentId::new(chart.source, &chart.symbol).key(),
                    self.timeframe.interval().to_owned(),
                )
            })
            .collect()
    }

    /// Every chart's `(symbol_key, candles)`, so the Pine subscription tick loop fires on any open
    /// chart, not just the focused one.
    pub fn charts_candles(&self) -> Vec<ChartFeed> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as i64);
        self.charts
            .iter()
            .filter(|chart| chart.is_market())
            .map(|chart| {
                let (bars, run) = chart.chart.pine_feed(&chart.candles, chart.source, now_ms);
                ChartFeed {
                    chart_id: chart.id,
                    symbol_key: InstrumentId::new(chart.source, &chart.symbol).key(),
                    timeframe: self.timeframe.interval().to_owned(),
                    bars,
                    run,
                    instances: chart.chart.pine_instances().to_vec(),
                }
            })
            .collect()
    }

    // MARK: Saved layouts

    fn load_saved_views(&self) -> Vec<SavedView> {
        self.store
            .lock()
            .map(|store| store.saved_views())
            .unwrap_or_default()
    }

    fn active_view(&self) -> Option<SavedView> {
        let id = self.active_view_id?;
        self.load_saved_views()
            .into_iter()
            .find(|view| view.id == id)
    }

    fn autosave_enabled(&self) -> bool {
        self.active_view()
            .is_some_and(|view| view.autosave == Some(true))
    }

    /// The toolbar menu's "Save": save in place, or ask for a name the first time.
    fn request_save(&mut self, ctx: &egui::Context) {
        if self.active_view_id.is_some() {
            self.save_active(true);
        } else {
            self.open_name_prompt(NamePrompt::Save { then_open: None }, ctx);
        }
    }

    /// The Save shortcut (⌘S / Ctrl+S) and any app-level save.
    pub fn save_layout(&mut self, ctx: &egui::Context) {
        self.request_save(ctx);
    }

    fn open_name_prompt(&mut self, prompt: NamePrompt, ctx: &egui::Context) {
        let base = self.active_view_name.clone();
        self.prompt_name = match prompt {
            NamePrompt::Copy => format!("{} copy", base.as_deref().unwrap_or("My Chart")),
            NamePrompt::Rename => base.unwrap_or_default(),
            NamePrompt::Save { .. } => String::new(),
        };
        self.prompt_focus = true;
        self.layout_prompt = Some(prompt);
        ctx.request_repaint();
    }

    fn close_name_prompt(&mut self) {
        self.layout_prompt = None;
        self.prompt_name.clear();
    }

    /// Writes the live layout over the open one, keeping its autosave choice. A failure leaves
    /// the layout dirty; `report` says whether to interrupt with an alert (autosave only flags it).
    fn save_active(&mut self, report: bool) -> bool {
        let Some(existing) = self.active_view() else {
            return false;
        };
        let view = self.build_view(
            existing.id,
            existing.name.clone(),
            existing.created_at_ms,
            Some(now_ms()),
            existing.autosave,
        );
        match self.upsert_view(view) {
            Ok(()) => {
                self.mark_clean();
                self.autosave = AutosaveState::default();
                true
            }
            Err(error) => {
                if report {
                    self.layout_error = Some(format!("Couldn’t save “{}”: {error}", existing.name));
                }
                false
            }
        }
    }

    /// First save of an unnamed layout (or a copy): mints a new identity.
    fn save_named(&mut self, name: String, autosave: Option<bool>) -> bool {
        let view = self.build_view(new_id(), name, now_ms(), Some(now_ms()), autosave);
        let (id, name) = (view.id, view.name.clone());
        match self.upsert_view(view) {
            Ok(()) => {
                self.active_view_id = Some(id);
                self.active_view_name = Some(name);
                self.mark_clean();
                self.autosave = AutosaveState::default();
                true
            }
            Err(error) => {
                self.layout_error = Some(format!("Couldn’t save the layout: {error}"));
                false
            }
        }
    }

    /// Captures the live layout into a `SavedView` (without persisting it).
    fn build_view(
        &self,
        id: u64,
        name: String,
        created_at_ms: i64,
        last_opened_at_ms: Option<i64>,
        autosave: Option<bool>,
    ) -> SavedView {
        let persisted = self.persisted();
        SavedView {
            id,
            name,
            timeframe: persisted.timeframe,
            created_at_ms,
            charts: persisted.charts,
            columns: Some(persisted.columns),
            candle_count: self
                .resolved_focused_id()
                .and_then(|id| self.charts.iter().find(|chart| chart.id == id))
                .map_or(0, |chart| chart.chart.visible_candles() as i64),
            last_opened_at_ms,
            autosave,
            sync: persisted.sync,
        }
    }

    fn upsert_view(&self, view: SavedView) -> Result<(), String> {
        let store = self.store.lock().map_err(|error| error.to_string())?;
        let mut views = store.saved_views();
        match views.iter_mut().find(|existing| existing.id == view.id) {
            // Replace in place so the layout keeps its position.
            Some(existing) => *existing = view,
            None => views.push(view),
        }
        store
            .replace_saved_views(&views)
            .map_err(|error| error.to_string())
    }

    /// Edits the open layout's stored row.
    fn update_active_view(&mut self, edit: impl FnOnce(&mut SavedView)) -> Result<(), String> {
        let Some(id) = self.active_view_id else {
            return Ok(());
        };
        let store = self.store.lock().map_err(|error| error.to_string())?;
        let mut views = store.saved_views();
        if let Some(view) = views.iter_mut().find(|view| view.id == id) {
            edit(view);
            store
                .replace_saved_views(&views)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn set_autosave(&mut self, enabled: bool) {
        match self.update_active_view(|view| view.autosave = enabled.then_some(true)) {
            Ok(()) => self.autosave = AutosaveState::default(),
            Err(error) => {
                self.layout_error = Some(format!("Couldn’t change Autosave: {error}"));
            }
        }
    }

    /// Runs once a frame: arms the debounce while an autosave layout has unsaved edits, and
    /// writes it once it has sat unchanged for [`AUTOSAVE_DELAY`].
    fn tick_autosave(&mut self, ctx: &egui::Context, dirty: bool, enabled: bool) {
        let mut state = std::mem::take(&mut self.autosave);
        let step = state.step(dirty, enabled, || self.layout_snapshot(), Instant::now());
        self.autosave = state;
        match step {
            AutosaveStep::Nothing => {}
            AutosaveStep::Wait(remaining) => ctx.request_repaint_after(remaining),
            AutosaveStep::Save => self.autosave.failed = !self.save_active(false),
        }
    }

    /// Writes a pending autosave now, before the layout changes or the app quits. False means
    /// the layout is still dirty on disk.
    pub fn flush_autosave(&mut self) -> bool {
        self.autosave.due = None;
        let enabled = self.autosave_enabled();
        if !(self.is_dirty() || (enabled && self.drawings_dirty())) || !enabled {
            return true;
        }
        if self.save_active(true) {
            true
        } else {
            self.autosave.failed = true;
            false
        }
    }

    /// The menu's "open": write an autosave layout first, ask over unsaved changes, or apply.
    fn request_open_saved_view(&mut self, id: u64, ctx: &egui::Context) {
        if self.autosave_enabled() {
            if self.flush_autosave() {
                self.apply_saved_view_by_id(id, ctx);
            }
        } else if self.is_dirty() {
            self.pending_open_id = Some(id);
            ctx.request_repaint();
        } else {
            self.apply_saved_view_by_id(id, ctx);
        }
    }

    fn apply_saved_view_by_id(&mut self, id: u64, ctx: &egui::Context) {
        let Some(view) = self
            .load_saved_views()
            .into_iter()
            .find(|view| view.id == id)
        else {
            return;
        };
        self.apply_saved_view(&view, ctx);
    }

    /// Copies the current layout (unsaved changes included) under a new identity; the original
    /// stays as it was saved.
    fn copy_layout(&mut self, name: String) {
        let autosave = self.active_view().and_then(|view| view.autosave);
        self.save_named(name, autosave);
    }

    fn rename_layout(&mut self, name: String) {
        if self.active_view_id.is_none() {
            self.save_named(name, None);
            return;
        }
        let renamed = name.clone();
        match self.update_active_view(|view| view.name = renamed) {
            Ok(()) => self.active_view_name = Some(name),
            Err(error) => {
                self.layout_error = Some(format!("Couldn’t rename the layout: {error}"));
            }
        }
    }

    fn delete_saved_view(&mut self, id: u64) {
        let result = self
            .store
            .lock()
            .map_err(|error| error.to_string())
            .and_then(|store| {
                let mut views = store.saved_views();
                views.retain(|view| view.id != id);
                store
                    .replace_saved_views(&views)
                    .map_err(|error| error.to_string())
            });
        if let Err(error) = result {
            self.layout_error = Some(format!("Couldn’t delete the layout: {error}"));
            return;
        }
        // The tab keeps its charts and becomes unnamed.
        if self.active_view_id == Some(id) {
            self.active_view_id = None;
            self.active_view_name = None;
            self.autosave = AutosaveState::default();
        }
    }

    /// Rebuilds the workspace from a saved layout; the one place a layout is applied.
    fn apply_saved_view(&mut self, view: &SavedView, ctx: &egui::Context) {
        self.charts.clear();
        for saved in &view.charts {
            self.charts
                .push(Chart::restored(saved.clone(), view.timeframe, ctx));
        }
        self.timeframe = view.timeframe;
        self.next_id = self
            .charts
            .iter()
            .map(|chart| chart.id)
            .max()
            .map_or(1, |max| max + 1);
        let chart_ids: Vec<u64> = self.charts.iter().map(|chart| chart.id).collect();
        self.columns =
            ChartColumn::resolved(view.columns.as_deref(), &chart_ids, &mut self.next_id);
        self.focused = self.charts.first().map(|chart| chart.id);
        self.set_sync(view.sync);
        self.drop_rulers();
        self.undo.clear();
        self.redo.clear();
        self.active_view_id = Some(view.id);
        self.active_view_name = Some(view.name.clone());
        self.set_baseline();
        self.autosave = AutosaveState::default();
        let _ = self.update_active_view(|opened| opened.last_opened_at_ms = Some(now_ms()));
    }

    // MARK: Chart lifecycle

    fn add_chart_with_symbol(&mut self, symbol: &str, source: DataSource, ctx: &egui::Context) {
        let id = self.fresh_id();
        self.charts
            .push(Chart::new(id, symbol, source, self.timeframe, ctx));
        self.place_in_grid(id);
        self.focused = Some(id);
    }

    fn add_from_search(&mut self, result: &SearchResult, ctx: &egui::Context) {
        let instrument = InstrumentId::new(result.source, &result.full_symbol);
        let outcome = self.add_chart_for(&instrument, ctx);
        if result.source == DataSource::DexScreener
            && let Some(chart) = self
                .charts
                .iter_mut()
                .find(|chart| chart.id == outcome.chart_id)
        {
            // DEXScreener fetches by pair address, but that opaque id should never become the
            // card title. Keep the search result's BASE/QUOTE label alongside it.
            chart.chart.set_title(Some(result.symbol.clone()));
        }
        if result.source.is_prediction_market()
            && let Some(chart) = self
                .charts
                .iter_mut()
                .find(|chart| chart.id == outcome.chart_id)
        {
            chart.image_url = result
                .event
                .as_ref()
                .and_then(|event| event.image_url.clone());
        }
        if !outcome.created {
            return;
        }
        if result.source.is_prediction_market() {
            self.title_prediction(result, ctx);
        }
    }

    /// Gives the chart just added for a prediction-market pick its event title and, when several
    /// choices were ticked, their lines; the choices need a second fetch.
    fn title_prediction(&mut self, result: &SearchResult, ctx: &egui::Context) {
        let Some(chart) = self.charts.last_mut() else {
            return;
        };
        let (title, series) = match &result.event {
            Some(event) if !event.title.is_empty() => (event.title.clone(), event.series.clone()),
            Some(event) => (result.symbol.clone(), event.series.clone()),
            None => (result.symbol.clone(), Vec::new()),
        };
        chart.set_prediction(title, series);
        chart.fetch(self.timeframe, ctx);
    }

    // MARK: Search sheet

    /// The chart settings sheet over the focused window; returns what the workspace's host should
    /// do (opening the Script Manager).
    fn show_settings_window(&mut self, ctx: &egui::Context) -> Vec<WorkspaceAction> {
        let Some(chart_id) = self.settings_chart else {
            return Vec::new();
        };
        let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == chart_id) else {
            self.settings_chart = None;
            return Vec::new();
        };
        let title = chart.display_title();
        let outcome = chart_settings::show(
            ctx,
            &mut self.settings_state,
            ChartSettingsContext {
                title: &title,
                symbol: &chart.symbol,
                source: chart.source,
                loaded_candles: chart.candles.len(),
                indicators: &mut chart.indicators,
                card: &mut chart.chart,
                store: &self.store,
            },
        );
        let mut actions = Vec::new();
        match outcome {
            Some(ChartSettingsAction::Done) => {
                self.settings_chart = None;
                self.script_popover = None;
            }
            Some(ChartSettingsAction::RemoveChart) => {
                self.settings_chart = None;
                self.script_popover = None;
                self.remove_chart(chart_id);
            }
            Some(ChartSettingsAction::OpenScriptManager) => {
                self.settings_chart = None;
                self.script_popover = None;
                actions.push(WorkspaceAction::OpenPineScripts);
            }
            Some(ChartSettingsAction::ConfigureScript(id, anchor)) => {
                self.toggle_script_popover(chart_id, id, anchor);
            }
            Some(ChartSettingsAction::ApplyToAll(scope)) => {
                let appearance = self
                    .charts
                    .iter()
                    .find(|chart| chart.id == chart_id)
                    .map(|chart| chart.chart.appearance());
                if let Some(appearance) = appearance {
                    let message = self.apply_appearance_to_all(chart_id, &appearance, scope);
                    self.settings_state.say(message);
                }
            }
            None => {}
        }
        actions
    }

    /// Gives every other chart of this workspace `appearance`. The plan is made first, for every
    /// chart, so none is left half-restyled; each chart then takes it through its own card, so a
    /// new chart type resets that chart's scroll and bars the way a pick in its selector does.
    /// Other workspaces, saved layouts and everything else a chart holds are not touched.
    fn apply_appearance_to_all(
        &mut self,
        source_chart: u64,
        appearance: &crate::model::ChartAppearance,
        scope: crate::model::chart_template::ApplyScope,
    ) -> String {
        let markets: Vec<(u64, DataSource)> = self
            .charts
            .iter()
            .filter(|chart| chart.is_market())
            .map(|chart| (chart.id, chart.source))
            .collect();
        let plan = crate::model::chart_template::plan_apply_to_all(
            &markets,
            appearance,
            scope,
            source_chart,
        );
        for (id, step) in &plan.steps {
            if let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == *id) {
                chart.chart.apply_template(appearance, *step);
            }
        }
        plan.message(appearance.style.chart_type.label())
    }

    /// Opens an applied script's Inputs and Style popover beside `anchor`, or puts it away when it
    /// is already up for that script.
    fn toggle_script_popover(&mut self, chart_id: u64, instance_id: u64, anchor: egui::Rect) {
        if self
            .script_popover
            .as_ref()
            .is_some_and(|popover| popover.is_for(chart_id, instance_id))
        {
            self.script_popover = None;
            return;
        }
        self.script_popover = self
            .charts
            .iter()
            .find(|chart| chart.id == chart_id)
            .and_then(|chart| {
                let instance = chart
                    .chart
                    .pine_instances()
                    .iter()
                    .find(|instance| instance.id == instance_id)?;
                Some(PineSettingsPopover::open(
                    chart_id,
                    instance,
                    chart.chart.pine_style_rows(instance_id),
                    anchor,
                ))
            });
    }

    /// The saved drawing templates and defaults. The first call opens them and takes the strip's
    /// remembered options (magnet, keep drawing, the grouped tile's tool) from them.
    fn drawing_templates(&mut self) -> &mut DrawingTemplateStore {
        if self.drawing_templates.is_none() {
            let templates = DrawingTemplateStore::new(self.store.clone());
            let prefs = templates.prefs();
            self.magnet = prefs.magnet;
            self.keep_drawing = prefs.keep_drawing;
            self.group_tool = prefs.group_tool;
            self.line_tool = prefs.line_tool;
            self.saved_prefs = prefs;
            self.drawing_templates = Some(templates);
        }
        let templates = self.drawing_templates.as_mut().expect("opened just above");
        templates.refresh(Instant::now());
        templates
    }

    /// Saves the strip's options when they changed.
    fn save_drawing_prefs(&mut self) {
        let prefs = DrawingPrefs {
            magnet: self.magnet,
            keep_drawing: self.keep_drawing,
            group_tool: self.group_tool,
            line_tool: self.line_tool,
        };
        if prefs != self.saved_prefs {
            self.saved_prefs = prefs;
            self.drawing_templates().set_prefs(&prefs);
        }
    }

    /// Opens the properties dialog of one drawing (replacing any other that was open).
    fn open_drawing_properties(&mut self, chart_id: u64, drawing_id: u64) {
        // One dialog at a time: a request while one is open (a Return meant for it, say) must
        // not replace the edits in it.
        if self.drawing_props.is_some() {
            return;
        }
        self.drawing_props = self
            .charts
            .iter()
            .find(|chart| chart.id == chart_id)
            .and_then(|chart| chart.drawings.iter().find(|d| d.id() == drawing_id))
            .filter(|drawing| !drawing.locked())
            .and_then(|drawing| DrawingProperties::new(chart_id, drawing));
    }

    /// The alert behind "Remove all drawings".
    fn confirm_drawings_removal(&mut self, ctx: &egui::Context) {
        let Some(chart_id) = self.confirm_remove_drawings else {
            return;
        };
        let count = self
            .charts
            .iter()
            .find(|chart| chart.id == chart_id)
            .map_or(0, |chart| {
                chart.drawings.iter().filter(|d| is_lockable(d)).count()
            });
        if count == 0 {
            self.confirm_remove_drawings = None;
            return;
        }
        let message = format!(
            "{count} drawing{} will be removed from this chart. You can undo it.",
            if count == 1 { "" } else { "s" }
        );
        let result = crate::ui::kit::confirm(
            ctx,
            egui::Id::new("remove-drawings"),
            &crate::ui::kit::Confirm {
                icon: crate::ui::icons::Icon::Trash,
                title: "Remove all drawings?",
                message: &message,
                destructive: "Remove All",
            },
        );
        match result {
            Some(crate::ui::kit::ConfirmOutcome::Confirmed) => {
                self.confirm_remove_drawings = None;
                let commands: Vec<DrawingCommand> = self
                    .charts
                    .iter()
                    .find(|chart| chart.id == chart_id)
                    .map(|chart| {
                        chart
                            .drawings
                            .iter()
                            .filter(|d| is_lockable(d))
                            .map(|d| DrawingCommand::Remove(d.id()))
                            .collect()
                    })
                    .unwrap_or_default();
                self.apply_drawing_commands(chart_id, commands);
                self.selected_drawing = None;
            }
            Some(crate::ui::kit::ConfirmOutcome::Cancelled) => self.confirm_remove_drawings = None,
            None => {}
        }
    }

    /// Draws the drawing's properties dialog, if one is up, and applies what it returns. The
    /// dialog goes away with its drawing or its chart.
    fn show_drawing_properties(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.drawing_props.take() else {
            return;
        };
        let (chart_id, drawing_id) = (dialog.chart_id, dialog.draft().id());
        let exists = self
            .charts
            .iter()
            .find(|chart| chart.id == chart_id)
            .is_some_and(|chart| chart.drawings.iter().any(|d| d.id() == drawing_id));
        if !exists {
            return;
        }
        let templates = self.drawing_templates();
        match dialog.show(ctx, templates) {
            drawing_properties::Outcome::Open => self.drawing_props = Some(dialog),
            drawing_properties::Outcome::Cancel => {}
            drawing_properties::Outcome::Apply(drawing) => {
                self.apply_drawing_commands(
                    chart_id,
                    vec![DrawingCommand::Replace {
                        id: drawing_id,
                        drawing: *drawing,
                    }],
                );
            }
        }
    }

    /// Draws the script popover, if one is up, and applies what it returns.
    fn show_script_popover(&mut self, ctx: &egui::Context) {
        let Some(mut popover) = self.script_popover.take() else {
            ui::chart_indicator_strip::set_open_script_popover(ctx, None);
            return;
        };
        let (chart_id, instance_id) = (popover.chart_id(), popover.instance_id());
        let applied = self
            .charts
            .iter()
            .find(|chart| chart.id == chart_id)
            .is_some_and(|chart| {
                chart
                    .chart
                    .pine_instances()
                    .iter()
                    .any(|instance| instance.id == instance_id)
            });
        if !applied {
            ui::chart_indicator_strip::set_open_script_popover(ctx, None);
            return;
        }
        match popover.show(ctx) {
            pine_settings_popover::Outcome::Open => {
                ui::chart_indicator_strip::set_open_script_popover(
                    ctx,
                    Some((chart_id, instance_id)),
                );
                self.script_popover = Some(popover);
            }
            pine_settings_popover::Outcome::Cancel => {
                ui::chart_indicator_strip::set_open_script_popover(ctx, None);
            }
            pine_settings_popover::Outcome::Apply { inputs, style } => {
                ui::chart_indicator_strip::set_open_script_popover(ctx, None);
                if let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == chart_id) {
                    chart.chart.set_pine_instance_inputs(instance_id, &inputs);
                    chart.chart.set_pine_instance_style(instance_id, style);
                }
            }
        }
    }

    fn place_in_grid(&mut self, chart_id: u64) {
        if self.columns.is_empty() || self.columns.len() < 2 {
            let column_id = self.fresh_id();
            self.columns
                .push(ChartColumn::new(column_id, vec![chart_id]));
        } else if let Some(shortest) = self
            .columns
            .iter()
            .map(|column| column.chart_ids.len())
            .enumerate()
            .min_by_key(|(index, len)| (*len, *index))
            .map(|(index, _)| index)
        {
            self.columns[shortest].chart_ids.push(chart_id);
        }
    }

    /// The macOS-style alert behind a header ✕.
    fn confirm_chart_removal(&mut self, ctx: &egui::Context, chart_id: u64) {
        let Some(chart) = self.charts.iter().find(|chart| chart.id == chart_id) else {
            self.removing_chart = None;
            return;
        };
        let title = format!("Remove {}?", chart.display_title());
        let result = crate::ui::kit::confirm(
            ctx,
            egui::Id::new("remove-chart"),
            &crate::ui::kit::Confirm {
                icon: crate::ui::icons::Icon::Trash,
                title: &title,
                message: "The chart leaves this tab. You can add it again from Add Chart.",
                destructive: "Remove",
            },
        );
        if result == Some(crate::ui::kit::ConfirmOutcome::Confirmed) {
            self.remove_chart(chart_id);
        }
        if result.is_some() {
            self.removing_chart = None;
        }
    }

    fn remove_chart(&mut self, chart_id: u64) {
        self.sync_hub.forget(chart_id);
        self.charts.retain(|chart| chart.id != chart_id);
        if self.focused == Some(chart_id) {
            self.focused = None;
        }
        for column in &mut self.columns {
            column.chart_ids.retain(|id| *id != chart_id);
        }
        self.columns.retain(|column| !column.chart_ids.is_empty());
    }

    fn focus_chart(&mut self, chart_id: u64) {
        if self.focused == Some(chart_id) {
            return;
        }
        if self.charts.iter().any(|chart| chart.id == chart_id) {
            self.focused = Some(chart_id);
        }
    }

    // MARK: Watchlist navigation

    fn switch_chart(&mut self, chart_id: u64, item: &WatchlistInstrument, ctx: &egui::Context) {
        let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == chart_id) else {
            return;
        };
        let instrument = &item.instrument;
        let already_showing = chart.source == instrument.source
            && InstrumentId::new(chart.source, &chart.symbol).is_same_market(instrument);
        if already_showing {
            self.focus_chart(chart_id);
            return;
        }
        chart.replace_market(item);
        chart.chart.reset_view();
        chart.fetch(self.timeframe, ctx);
        self.focus_chart(chart_id);
    }

    // MARK: Live feed

    fn update_live_feed(&mut self, ctx: &egui::Context) {
        let symbols: Vec<String> = self
            .charts
            .iter()
            .filter(|chart| chart.is_market() && chart.source == DataSource::Binance)
            .map(|chart| chart.symbol.to_uppercase())
            .collect();
        let interval = self.timeframe.interval().to_owned();
        // Binance streams no quarterly or yearly klines; those fold monthly candles.
        let monthly_fold = matches!(self.timeframe, Timeframe::Quarter | Timeframe::Year);
        let signature = (symbols, interval);

        let should_connect = !monthly_fold && !signature.0.is_empty();
        let needs_change = match &self.live_feed {
            Some(feed) => feed.signature() != &signature || !should_connect,
            None => should_connect,
        };
        if needs_change {
            self.live_feed = None;
            if should_connect {
                self.live_feed = Some(BinanceLiveFeed::connect(
                    signature.0.clone(),
                    signature.1.clone(),
                    ctx.clone(),
                ));
            }
        }

        let mut updates = Vec::new();
        if let Some(feed) = &self.live_feed {
            while let Ok((symbol, kline)) = feed.receiver.try_recv() {
                updates.push((symbol, kline));
            }
        }
        for (symbol, kline) in updates {
            self.apply_live_kline(&symbol, kline);
        }
    }

    fn apply_live_kline(&mut self, symbol: &str, kline: crate::model::Kline) {
        let Some(chart) = self.charts.iter_mut().find(|chart| {
            chart.source == DataSource::Binance && chart.symbol.eq_ignore_ascii_case(symbol)
        }) else {
            return;
        };
        let replaces_last = chart
            .candles
            .last()
            .is_some_and(|last| last.open_time == kline.open_time);
        if replaces_last {
            if let Some(last) = chart.candles.last_mut() {
                *last = kline;
            }
        } else {
            chart.candles.push(kline);
        }
        chart.last_live_open_time = chart.candles.last().map(|candle| candle.open_time);
    }

    // MARK: Drawings

    fn apply_drawing_commands(&mut self, chart_id: u64, commands: Vec<DrawingCommand>) {
        if commands.is_empty() {
            return;
        }
        let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == chart_id) else {
            return;
        };
        let restyled = restyled_drawing(&commands);
        let now = Instant::now();
        let merge = restyled.is_some_and(|id| restyle_merges(self.last_restyle, chart_id, id, now));
        // A restyle that merges into the last step needs no copy of the drawings to go back to.
        let before: Vec<Drawing> = if merge {
            Vec::new()
        } else {
            chart
                .drawings
                .iter()
                .filter(|drawing| !is_ruler(drawing))
                .cloned()
                .collect()
        };
        // One batch is one undo step, and a batch that changes nothing (or only a ruler) none.
        if crate::model::drawings::apply_commands(&mut chart.drawings, commands) {
            self.drawings_version += 1;
            // A restyle that follows another of the same drawing moments ago is part of it.
            if !merge {
                self.undo.push(UndoEntry {
                    chart_id,
                    before: Snapshot::Drawings(before),
                });
            }
            self.redo.clear();
            self.last_restyle = restyled.map(|id| (chart_id, id, now));
        } else if restyled.is_none() {
            self.last_restyle = None;
        }
    }

    fn undo(&mut self) {
        self.last_restyle = None;
        let Some(entry) = self.undo.pop() else {
            return;
        };
        if let Some(chart) = self
            .charts
            .iter_mut()
            .find(|chart| chart.id == entry.chart_id)
        {
            let after = entry.before.swap_into(chart);
            self.drawings_version += 1;
            self.redo.push(UndoEntry {
                chart_id: entry.chart_id,
                before: after,
            });
        }
    }

    fn redo(&mut self) {
        self.last_restyle = None;
        let Some(entry) = self.redo.pop() else {
            return;
        };
        if let Some(chart) = self
            .charts
            .iter_mut()
            .find(|chart| chart.id == entry.chart_id)
        {
            let after = entry.before.swap_into(chart);
            self.drawings_version += 1;
            self.undo.push(UndoEntry {
                chart_id: entry.chart_id,
                before: after,
            });
        }
    }

    fn drop_rulers(&mut self) {
        for chart in &mut self.charts {
            chart.drawings.retain(|drawing| !is_ruler(drawing));
        }
    }

    fn handle_keyboard(&mut self, ui: &egui::Ui) {
        // Undo in the Add Chart search box edits the text, not the charts behind it.
        if self.add_chart.is_open() {
            return;
        }
        // A text field (the properties dialog's) keeps its own undo.
        let typing = ui.ctx().egui_wants_keyboard_input();
        let (undo, redo) =
            ui.input(|input| (shortcut::UNDO.peek(input), shortcut::REDO.peek(input)));
        if undo && !typing {
            self.undo();
        }
        if redo && !typing {
            self.redo();
        }
        if !typing && ui.input_mut(|input| shortcut::HIDE_DRAWINGS.consume(input)) {
            self.hide_all_drawings = !self.hide_all_drawings;
        }
        // Alt (Option) and a letter arm a line tool, like the strip's tile does. Option+letter
        // types a character on a Mac, so a text field keeps it.
        if !typing {
            for tool in Tool::GROUPED {
                let pressed = shortcut::tool(tool)
                    .is_some_and(|shortcut| ui.input_mut(|input| shortcut.consume(input)));
                if pressed {
                    self.toggle_tool(tool);
                    self.save_drawing_prefs();
                }
            }
        }
        // The snapshot shortcuts act on the focused chart, the one whose frame is lit. Option+S
        // types a letter on a Mac, so they leave a text field alone.
        if !typing && let Some(chart_id) = self.resolved_focused_id() {
            for action in [
                SnapshotAction::Download,
                SnapshotAction::CopyImage,
                SnapshotAction::CopyLink,
            ] {
                let pressed = shortcut::snapshot(action)
                    .is_some_and(|shortcut| ui.input_mut(|input| shortcut.consume(input)));
                if pressed {
                    self.begin_snapshot(ui.ctx(), chart_id, action);
                }
            }
        }
        // macOS owns Save in its menu (`NativeCommand::SaveCurrentLayout`); everywhere else the
        // shortcut is handled here, in the tab that has focus.
        if !shortcut::is_mac()
            && self.layout_prompt.is_none()
            && ui.input_mut(|input| shortcut::SAVE.consume(input))
        {
            self.save_layout(ui.ctx());
        }
    }

    // MARK: Replay

    /// Timeline the replay bar builds from the focused chart's candle opens.
    #[allow(dead_code)] // consumed by the replay bar UI (second agent)
    fn replay_timeline(&self) -> Vec<i64> {
        self.resolved_focused_id()
            .and_then(|id| self.charts.iter().find(|chart| chart.id == id))
            .map(|chart| chart.candles.iter().map(|kline| kline.open_time).collect())
            .unwrap_or_default()
    }

    fn toggle_replay(&mut self) {
        if self.replay.is_bar_visible() {
            self.replay.stop();
        } else {
            self.replay.open();
        }
    }

    fn tick_replay(&mut self) {
        if self.replay.status != ReplayStatus::Playing {
            return;
        }
        let delay = self
            .replay
            .session
            .as_ref()
            .map_or(1_000, |session| session.speed.delay_ms());
        if self.replay_last_step.elapsed() >= Duration::from_millis(delay) {
            self.replay_last_step = Instant::now();
            self.replay.step_forward();
        }
    }

    fn focused_chart_data(&self) -> (String, Vec<i64>) {
        self.resolved_focused_id()
            .and_then(|id| self.charts.iter().find(|chart| chart.id == id))
            .map(|chart| {
                (
                    chart.symbol.clone(),
                    chart.candles.iter().map(|kline| kline.open_time).collect(),
                )
            })
            .unwrap_or_default()
    }

    fn start_replay_first(&mut self) {
        let (symbol, timeline) = self.focused_chart_data();
        if let Some(first) = timeline.first().copied() {
            self.replay.start(first, &symbol, timeline);
        }
    }

    fn start_replay_random(&mut self) {
        let (symbol, timeline) = self.focused_chart_data();
        if timeline.is_empty() {
            return;
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as usize)
            .unwrap_or(0);
        // Leave the final fifth available to play through, as the Swift replay picker does.
        let eligible = ((timeline.len() as f64) * 0.8).ceil().max(1.0) as usize;
        let index = nanos % eligible;
        let picked = timeline[index];
        self.replay.start(picked, &symbol, timeline);
    }

    fn show_replay_bar(&mut self, ui: &mut egui::Ui) {
        let available = [ReplayInterval::Automatic, ReplayInterval::ChartBar];
        let input = ReplayBarInput {
            engine: &self.replay,
            is_preparing: false,
            notice: None,
            resolved_interval: None,
            available_intervals: &available,
        };
        let actions = replay_bar::show(ui, &input);
        self.apply_replay_actions(actions);
    }

    fn apply_replay_actions(&mut self, actions: Vec<ReplayBarAction>) {
        for action in actions {
            match action {
                ReplayBarAction::SelectOnChart => self.replay.begin_selecting(),
                ReplayBarAction::CancelSelection => self.replay.cancel_selection(),
                ReplayBarAction::ReturnToLive => self.replay.stop(),
                ReplayBarAction::ChangeStart => self.replay.begin_selecting(),
                ReplayBarAction::Restart => self.replay.restart(),
                ReplayBarAction::StepBack => {
                    self.replay.step_backward();
                }
                ReplayBarAction::StepForward => {
                    self.replay.step_forward();
                }
                ReplayBarAction::TogglePlayback => {
                    if self.replay.status == ReplayStatus::Playing {
                        self.replay.pause();
                    } else {
                        self.replay.play();
                    }
                }
                ReplayBarAction::Seek(fraction) => self.replay.seek(fraction),
                ReplayBarAction::SetSpeed(speed) => self.replay.set_speed(speed),
                ReplayBarAction::SetInterval(interval) => self.replay.set_interval(interval),
                ReplayBarAction::FirstBar => self.start_replay_first(),
                ReplayBarAction::RandomBar => self.start_replay_random(),
                ReplayBarAction::ChooseDate => {
                    let timeline = self.replay_timeline();
                    self.date_sheet.open(&timeline);
                }
                ReplayBarAction::DismissNotice => {}
            }
        }
    }

    // MARK: Drag reorder

    fn move_chart(&mut self, chart_id: u64, to_column: u64, before: Option<u64>) {
        if before == Some(chart_id) || !self.columns.iter().any(|c| c.id == to_column) {
            return;
        }
        let original = self.columns.clone();

        for column in &mut self.columns {
            column.chart_ids.retain(|id| *id != chart_id);
        }
        self.columns
            .retain(|column| !column.chart_ids.is_empty() || column.id == to_column);

        let Some(column_index) = self
            .columns
            .iter()
            .position(|column| column.id == to_column)
        else {
            self.columns = original;
            return;
        };
        let insertion = before
            .and_then(|target| {
                self.columns[column_index]
                    .chart_ids
                    .iter()
                    .position(|id| *id == target)
            })
            .unwrap_or(self.columns[column_index].chart_ids.len());
        self.columns[column_index]
            .chart_ids
            .insert(insertion, chart_id);
    }

    fn move_to_new_trailing_column(&mut self, chart_id: u64) {
        if !self.charts.iter().any(|chart| chart.id == chart_id) {
            return;
        }
        for column in &mut self.columns {
            column.chart_ids.retain(|id| *id != chart_id);
        }
        self.columns.retain(|column| !column.chart_ids.is_empty());
        let column_id = self.fresh_id();
        self.columns
            .push(ChartColumn::new(column_id, vec![chart_id]));
    }

    // MARK: Panels

    fn show_toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) -> Vec<WorkspaceAction> {
        let mut actions = Vec::new();
        let active_view_id = self.active_view_id;
        let saved_views = self.load_saved_views();
        let mut save_clicked = false;
        let mut copy_clicked = false;
        let mut rename_clicked = false;
        let mut autosave_toggled = false;
        let mut open_id: Option<u64> = None;
        let mut delete_id: Option<u64> = None;
        let layouts: Vec<(u64, String)> = saved_views
            .iter()
            .map(|view| (view.id, view.name.clone()))
            .collect();
        let focused = self.resolved_focused_id();
        let unlinkable: Vec<String> = self
            .charts
            .iter()
            .filter(|chart| chart.is_market() && !chart.chart.takes_part_in_sync(chart.source))
            .map(Chart::display_title)
            .collect();
        let dirty = self.is_dirty();
        let autosave_on = saved_views
            .iter()
            .find(|view| Some(view.id) == active_view_id)
            .is_some_and(|view| view.autosave == Some(true));
        // An Autosave layout also saves drawing-only edits; the toolbar's "unsaved" state does not
        // count them, as before.
        self.tick_autosave(
            ctx,
            dirty || (autosave_on && self.drawings_dirty()),
            autosave_on,
        );
        let input = workspace_toolbar::ToolbarInput {
            timeframe: self.timeframe,
            log_scale: focused
                .and_then(|id| self.charts.iter().find(|chart| chart.id == id))
                .map(|chart| chart.log_scale),
            layout_name: self.active_view_name.as_deref().unwrap_or("Unnamed"),
            // Autosave keeps a layout clean in the user's eyes: "(unsaved)" only shows while
            // nothing is going to save it.
            unsaved: dirty && !(autosave_on && !self.autosave.failed),
            autosave: workspace_toolbar::AutosaveState {
                available: active_view_id.is_some(),
                on: autosave_on,
                failed: self.autosave.failed,
            },
            layouts: &layouts,
            active_layout: active_view_id,
            watchlist_visible: self.watchlist_visible,
            sync: self.sync,
            can_link: self.market_charts() > 1,
            unlinkable: &unlinkable,
        };
        for action in workspace_toolbar::show(ui, &input) {
            match action {
                ToolbarAction::SetTimeframe(timeframe) => {
                    self.timeframe = timeframe;
                    self.drop_rulers();
                    for chart in &mut self.charts {
                        chart.chart.reset_view();
                        chart.candles.clear();
                        chart.reset_history();
                        chart.last_live_open_time = None;
                        chart.fetch(self.timeframe, ctx);
                    }
                }
                ToolbarAction::ToggleLogScale => {
                    if let Some(chart) =
                        focused.and_then(|id| self.charts.iter_mut().find(|chart| chart.id == id))
                    {
                        chart.log_scale = !chart.log_scale;
                    }
                }
                ToolbarAction::SaveLayout => save_clicked = true,
                ToolbarAction::CopyLayout => copy_clicked = true,
                ToolbarAction::RenameLayout => rename_clicked = true,
                ToolbarAction::ToggleAutosave => autosave_toggled = true,
                ToolbarAction::OpenLayout(id) => open_id = Some(id),
                ToolbarAction::DeleteLayout(id) => delete_id = Some(id),
                ToolbarAction::ToggleWatchlist => {
                    self.watchlist_visible = !self.watchlist_visible;
                }
                ToolbarAction::OpenPortfolio => actions.push(WorkspaceAction::OpenPortfolio),
                ToolbarAction::OpenScriptManager => {
                    actions.push(WorkspaceAction::OpenPineScripts);
                }
                ToolbarAction::AddChart => self.add_chart.open(),
                ToolbarAction::SetSync(sync) => self.set_sync(sync),
            }
        }

        if save_clicked {
            self.request_save(ctx);
        }
        if autosave_toggled {
            self.set_autosave(!autosave_on);
        }
        if copy_clicked {
            self.open_name_prompt(NamePrompt::Copy, ctx);
        }
        if rename_clicked {
            self.open_name_prompt(NamePrompt::Rename, ctx);
        }
        if let Some(id) = open_id {
            self.request_open_saved_view(id, ctx);
        }
        if let Some(id) = delete_id {
            self.delete_saved_view(id);
        }
        self.show_layout_sheets(ctx);
        actions
    }

    /// The name prompt, the "save changes?" question and the error alert, one at a time.
    fn show_layout_sheets(&mut self, ctx: &egui::Context) {
        if let Some(message) = self.layout_error.clone() {
            if layout_prompts::show_error(ctx, &message) {
                self.layout_error = None;
            }
            return;
        }
        if let Some(prompt) = self.layout_prompt {
            let mut focus = std::mem::take(&mut self.prompt_focus);
            let mut name = std::mem::take(&mut self.prompt_name);
            let outcome = layout_prompts::show_name(ctx, prompt, &mut name, &mut focus);
            self.prompt_name = name;
            self.prompt_focus = focus;
            match outcome {
                Some(NameOutcome::Confirm(name)) => {
                    self.close_name_prompt();
                    match prompt {
                        NamePrompt::Save { then_open } => {
                            if self.save_named(name, None)
                                && let Some(id) = then_open
                            {
                                self.pending_open_id = None;
                                self.apply_saved_view_by_id(id, ctx);
                            }
                        }
                        NamePrompt::Copy => self.copy_layout(name),
                        NamePrompt::Rename => self.rename_layout(name),
                    }
                }
                Some(NameOutcome::Cancel) => {
                    self.close_name_prompt();
                    self.pending_open_id = None;
                }
                None => {}
            }
            return;
        }
        let Some(id) = self.pending_open_id else {
            return;
        };
        let name = self
            .active_view_name
            .clone()
            .unwrap_or_else(|| "Unnamed".to_owned());
        match layout_prompts::show_unsaved(ctx, &name) {
            Some(UnsavedChoice::Save) => {
                if self.active_view_id.is_none() {
                    // The first save needs a name; opening follows it.
                    self.open_name_prompt(
                        NamePrompt::Save {
                            then_open: Some(id),
                        },
                        ctx,
                    );
                } else if self.save_active(true) {
                    self.pending_open_id = None;
                    self.apply_saved_view_by_id(id, ctx);
                }
            }
            Some(UnsavedChoice::Discard) => {
                self.pending_open_id = None;
                self.apply_saved_view_by_id(id, ctx);
            }
            Some(UnsavedChoice::Cancel) => self.pending_open_id = None,
            None => {}
        }
    }

    fn show_tool_rail(&mut self, ui: &mut egui::Ui) -> Vec<WorkspaceAction> {
        // The strip shows the remembered options, so they are read before it is drawn.
        let _ = self.drawing_templates();
        let (has_drawings, all_locked) = self.drawing_lock_state();
        let input = tool_sidebar::ToolSidebarInput {
            active_tool: self.active_tool,
            group_tool: self.group_tool,
            line_tool: self.line_tool,
            magnet: self.magnet,
            keep_drawing: self.keep_drawing,
            hide_all: self.hide_all_drawings,
            all_locked,
            has_drawings,
            paper_open: self.paper_panel_visible,
            replay_open: self.replay.is_bar_visible(),
            has_market: self
                .charts
                .iter()
                .any(|chart| chart.is_market() && chart.source != DataSource::CoinMarketCap),
            unseen_alerts: self.unseen_alerts,
        };
        let mut actions = Vec::new();
        for action in tool_sidebar::show(ui, &input) {
            match action {
                ToolSidebarAction::Select(tool) => self.toggle_tool(tool),
                ToolSidebarAction::Arm(tool) => {
                    self.active_tool = tool;
                    self.remember_group_tool(tool);
                    self.drop_rulers();
                }
                ToolSidebarAction::SetMagnet(mode) => self.magnet = mode,
                ToolSidebarAction::ToggleKeepDrawing => self.keep_drawing = !self.keep_drawing,
                ToolSidebarAction::ToggleHideAll => {
                    self.hide_all_drawings = !self.hide_all_drawings
                }
                ToolSidebarAction::ToggleLockAll => {
                    let (_, all_locked) = self.drawing_lock_state();
                    self.set_all_locked(!all_locked);
                }
                ToolSidebarAction::TogglePaper => {
                    self.paper_panel_visible = !self.paper_panel_visible;
                }
                ToolSidebarAction::ToggleReplay => self.toggle_replay(),
                ToolSidebarAction::OpenAlerts => actions.push(WorkspaceAction::OpenAlerts),
            }
        }
        self.save_drawing_prefs();
        actions
    }

    /// Arms `tool`, or goes back to the crosshair when it is already armed (or is the crosshair).
    fn toggle_tool(&mut self, tool: Tool) {
        self.active_tool = if self.active_tool == tool || tool == Tool::Cursor {
            Tool::Cursor
        } else {
            tool
        };
        self.remember_group_tool(tool);
        self.drop_rulers();
    }

    /// Remembers `tool` as the one the tile of its menu shows, when it belongs to one.
    fn remember_group_tool(&mut self, tool: Tool) {
        use crate::model::ToolGroup;
        let group = tool.spec().group;
        if ToolGroup::MENU.contains(&group) {
            self.group_tool = tool;
        } else if ToolGroup::LINE_MENU.contains(&group) {
            self.line_tool = tool;
        }
    }

    /// Whether any chart has a drawing to lock, and whether every one of them is locked.
    fn drawing_lock_state(&self) -> (bool, bool) {
        let mut any = false;
        let mut all_locked = true;
        for chart in self.charts.iter().filter(|chart| chart.is_market()) {
            for drawing in chart.drawings.iter().filter(|d| is_lockable(d)) {
                any = true;
                all_locked &= drawing.locked();
            }
        }
        (any, any && all_locked)
    }

    /// Locks or unlocks every drawing on every chart of this tab. One undo step per chart,
    /// since history is kept per chart.
    fn set_all_locked(&mut self, locked: bool) {
        let ids: Vec<u64> = self
            .charts
            .iter()
            .filter(|chart| chart.is_market())
            .map(|chart| chart.id)
            .collect();
        for id in ids {
            let Some(chart) = self.charts.iter().find(|chart| chart.id == id) else {
                continue;
            };
            let commands: Vec<DrawingCommand> = chart
                .drawings
                .iter()
                .filter(|d| is_lockable(d) && d.locked() != locked)
                .map(|drawing| {
                    let mut changed = drawing.clone();
                    changed.set_locked(locked);
                    DrawingCommand::Replace {
                        id: drawing.id(),
                        drawing: changed,
                    }
                })
                .collect();
            self.apply_drawing_commands(id, commands);
        }
    }

    /// How many alerts have fired since the Alerts window was last looked at, for the bell.
    pub fn set_unseen_alerts(&mut self, count: usize) {
        self.unseen_alerts = count;
    }

    /// Gives the cards on this tab their portfolios. Cheap to call every frame.
    pub fn set_card_env(&mut self, env: &PortfolioCardEnv) {
        if self.card_env.is_none() {
            self.card_env = Some(env.clone());
        }
    }

    /// The identity a chart has in a watchlist: its market, or its card's placeholder.
    fn chart_instrument(chart: &Chart) -> InstrumentId {
        chart.card.as_ref().map_or_else(
            || InstrumentId::new(chart.source, &chart.symbol),
            PortfolioCard::instrument_id,
        )
    }

    /// The watchlist row that stands for `chart`.
    fn favorite_item(&self, chart: &Chart) -> WatchlistInstrument {
        if let Some(card) = chart.card {
            let scope = self
                .card_env
                .as_ref()
                .map_or_else(Vec::new, PortfolioCardEnv::portfolios);
            return WatchlistInstrument::for_card(card, &card.scope_name(&scope));
        }
        let mut item =
            WatchlistInstrument::from_market(chart.source, &chart.display_title(), &chart.symbol);
        item.image_url = chart.image_url.clone();
        item
    }

    /// The app's latest USD → currency rates, for the price-alert sheet.
    pub fn set_alert_fx_rates(&mut self, rates: &HashMap<AlertCurrency, f64>) {
        if self.alert_fx_rates != *rates {
            self.alert_fx_rates.clone_from(rates);
        }
    }

    /// The open price-alert sheet is in a currency that needs FX rates.
    pub fn alert_editor_needs_fx_rates(&self) -> bool {
        self.alert_editor
            .as_ref()
            .is_some_and(PriceAlertEditorState::needs_fx_rates)
    }

    /// Keeps the open sheet's market priced: it asks for the quote for as long as it is open.
    fn show_alert_editor(&mut self, ctx: &egui::Context) -> Vec<WorkspaceAction> {
        let owner = alert_editor::quote_owner(self.id);
        if let Some(request) = self.new_alert_request.take() {
            let mut editor = PriceAlertEditorState::new(request.asset, None, None);
            // A chart point prefills the level and the way to cross it; the user still reviews
            // and saves, so nothing is created here.
            if let Some(price) = request.price {
                editor.target = AlertCurrency::Usd.plain_price_text(price);
                editor.direction =
                    match crate::model::chart_context::alert_side(price, request.last) {
                        Some(crate::model::chart_context::AlertSide::Below) => {
                            alert_editor::Direction::Below
                        }
                        _ => alert_editor::Direction::Above,
                    };
            }
            self.alert_editor = Some(editor);
        }
        let Some(state) = self.alert_editor.as_mut() else {
            return Vec::new();
        };
        self.quote_coordinator
            .set_demand(owner, vec![state.asset().instrument.clone()], true);
        state.feed_from_book(&self.quote_book, &self.alert_fx_rates);
        let endpoints = self
            .store
            .lock()
            .map(|store| store.webhook_endpoints())
            .unwrap_or_default();
        let mut actions = Vec::new();
        let mut finished = false;
        match alert_editor::show(ctx, state, &endpoints, now_ms()) {
            Some(PriceAlertEditorOutcome::Save(alert)) => {
                actions.push(WorkspaceAction::SavePriceAlert {
                    alert,
                    baseline: state.current_price(),
                });
                finished = true;
            }
            Some(PriceAlertEditorOutcome::Cancel) => finished = true,
            Some(PriceAlertEditorOutcome::OpenWebhookSettings) => {
                actions.push(WorkspaceAction::OpenWebhookSettings);
            }
            None => {}
        }
        if finished {
            self.alert_editor = None;
            self.quote_coordinator.release(owner);
        }
        actions
    }

    fn show_watchlist(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(receiver) = &self.watchlist_file_result {
            match receiver.try_recv() {
                Ok(Err(error)) => {
                    self.watchlist_error = Some(error);
                    self.watchlist_file_result = None;
                }
                Ok(Ok(())) | Err(mpsc::TryRecvError::Disconnected) => {
                    self.watchlist_file_result = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let (actions, instruments) = {
            let watchlist = Arc::clone(&self.watchlist);
            let store = watchlist.lock().expect("watchlist lock poisoned");
            let lists = store.lists();
            let flags = store.flags();
            let instruments: Vec<InstrumentId> = lists
                .iter()
                .flat_map(|list| list.instruments())
                .map(|item| item.instrument.clone())
                .collect();
            let quote = |id: &InstrumentId| self.quote_book.get(id);
            let focused_market = self
                .resolved_focused_id()
                .and_then(|id| self.charts.iter().find(|chart| chart.id == id))
                .map(|chart| InstrumentId::new(chart.source, &chart.symbol));
            let input = WatchlistSidebarInput {
                lists,
                flags,
                quote: &quote,
                focused_market: focused_market.as_ref(),
                load_failed: store.load_failed(),
                error: self
                    .watchlist_error
                    .as_deref()
                    .or_else(|| store.last_error()),
                import_report: self.import_report.as_ref(),
            };
            let actions = watchlist_sidebar::show(ui, &mut self.watchlist_sidebar, &input);
            (actions, instruments)
        };
        self.quote_coordinator
            .set_demand(self.id, instruments, self.watchlist_visible);
        self.apply_watchlist_actions(actions, ctx);
    }

    fn apply_watchlist_actions(&mut self, actions: Vec<WatchlistAction>, ctx: &egui::Context) {
        let mut opens: Vec<(bool, WatchlistInstrument)> = Vec::new();
        {
            let mut store = self.watchlist.lock().expect("watchlist lock poisoned");
            for action in actions {
                match action {
                    WatchlistAction::Open(item) => opens.push((true, item)),
                    WatchlistAction::AddChart(item) => opens.push((false, item)),
                    WatchlistAction::OpenInNewTab(item) => {
                        // Hand off after the store lock is released.
                        self.open_new_tab_request = Some(item);
                    }
                    WatchlistAction::AddSymbol { list } => {
                        if let Some(list_id) = list
                            && let Some(name) = store
                                .lists()
                                .iter()
                                .find(|candidate| candidate.id == list_id)
                                .map(|candidate| candidate.name.clone())
                        {
                            self.add_to_watchlist = Some(list_id);
                            self.add_chart.open_for_watchlist(&name);
                        }
                    }
                    WatchlistAction::CreateAlert(item) => {
                        self.new_alert_request = Some(AlertRequest::market(AlertAsset::new(
                            item.instrument.clone(),
                            crate::ui::watchlist_rows::icon_base_symbol(&item),
                            item.name.clone(),
                        )));
                    }
                    WatchlistAction::CreateList { name, with } => {
                        store.create_list(&name, with);
                    }
                    WatchlistAction::RenameList { id, name } => store.rename_list(id, &name),
                    WatchlistAction::DuplicateList { id, name } => {
                        store.duplicate_list(id, &name);
                    }
                    WatchlistAction::DeleteList(id) => store.delete_list(id),
                    WatchlistAction::AddToList { list, item } => store.add_to_list(list, item),
                    WatchlistAction::RemoveFromList { list, instrument } => {
                        store.remove_from_list(list, &instrument);
                    }
                    WatchlistAction::RemoveEntry { list, entry } => {
                        store.remove_entry(list, entry);
                    }
                    WatchlistAction::MoveToSection {
                        list,
                        entry,
                        section,
                    } => store.move_instruments(list, &[entry], section),
                    WatchlistAction::Move { list, mv } => {
                        store.move_entries(list, &mv.ids, mv.before);
                    }
                    WatchlistAction::MoveToSectionByDrop { list, mv } => {
                        store.move_instruments(list, &mv.ids, mv.section);
                    }
                    WatchlistAction::AddSection { list, title } => store.add_section(list, &title),
                    WatchlistAction::RenameSection {
                        list,
                        section,
                        title,
                    } => store.rename_section(list, section, &title),
                    WatchlistAction::DeleteSection { list, section } => {
                        store.delete_section(list, section);
                    }
                    WatchlistAction::SetSectionCollapsed {
                        list,
                        section,
                        collapsed,
                    } => store.set_section_collapsed(list, section, collapsed),
                    WatchlistAction::SetSort { list, sort } => store.set_sort(list, sort),
                    WatchlistAction::SetColumn {
                        list,
                        column,
                        visible,
                    } => store.set_column(list, column, visible),
                    WatchlistAction::SetShowsDescription { list, shows } => {
                        store.set_shows_description(list, shows);
                    }
                    WatchlistAction::SetFlag { instrument, flag } => {
                        store.set_flag(&instrument, flag);
                    }
                    WatchlistAction::Import { list, text } => {
                        match store.import_into(list, &text) {
                            Some(report) => {
                                self.import_report = Some(report);
                                self.watchlist_error = None;
                            }
                            None => {
                                self.watchlist_error = Some("Import failed".to_owned());
                            }
                        }
                    }
                    WatchlistAction::BeginImport => self.import_report = None,
                    WatchlistAction::Export { list } => {
                        if let Some(text) = store.export(list) {
                            let name = store
                                .lists()
                                .iter()
                                .find(|candidate| candidate.id == list)
                                .map(|candidate| candidate.name.clone())
                                .unwrap_or_else(|| "Watchlist".to_owned());
                            let (sender, receiver) = mpsc::channel();
                            self.watchlist_file_result = Some(receiver);
                            std::thread::spawn(move || {
                                let result = crate::service::file_dialog::choose_save_path(
                                    "Export watchlist",
                                    &format!("{name}.txt"),
                                )
                                .and_then(|path| {
                                    path.map(|path| {
                                        crate::service::file_dialog::write_text(&path, &text)
                                    })
                                    .transpose()
                                })
                                .map(|_| ());
                                let _ = sender.send(result);
                            });
                        }
                    }
                    WatchlistAction::CopyText(text) => ctx.copy_text(text),
                    WatchlistAction::DismissError => {
                        self.watchlist_error = None;
                        store.clear_error();
                    }
                }
            }
        }
        for (open, item) in opens {
            if open {
                self.open_watchlist_instrument(&item, ctx);
            } else {
                self.add_chart_from_instrument(&item, ctx);
            }
        }
    }

    fn open_watchlist_instrument(&mut self, item: &WatchlistInstrument, ctx: &egui::Context) {
        // A card is not a market: show the one already here, else add it beside the others
        // (never replace the focused chart's market with it).
        if let Some(card) = item.card {
            self.add_card(card);
            return;
        }
        let instrument = &item.instrument;
        if let Some(existing) = self.charts.iter().position(|chart| {
            InstrumentId::new(chart.source, &chart.symbol).is_same_market(instrument)
        }) {
            let chart_id = self.charts[existing].id;
            self.charts[existing].apply_watchlist_metadata(item);
            self.focus_chart(chart_id);
            return;
        }
        if let Some(focused) = self.resolved_focused_id() {
            self.switch_chart(focused, item, ctx);
        } else {
            self.add_chart_from_instrument(item, ctx);
        }
    }

    fn add_chart_from_instrument(&mut self, item: &WatchlistInstrument, ctx: &egui::Context) {
        if let Some(card) = item.card {
            self.add_card(card);
            return;
        }
        let outcome = self.add_chart_for(&item.instrument, ctx);
        if let Some(chart) = self
            .charts
            .iter_mut()
            .find(|chart| chart.id == outcome.chart_id)
        {
            chart.apply_watchlist_metadata(item);
        }
    }

    fn sync_paper_quotes(&mut self, host: &mut PaperHost) {
        if !self.paper_panel_visible {
            return;
        }
        let _ = host.ensure_default_account();
        let now = Instant::now();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis() as i64);
        for chart in &mut self.charts {
            if !chart.is_market() || !Self::supports_paper_trading(chart.source) {
                continue;
            }
            let Some(last) = chart.candles.last().map(|candle| candle.close) else {
                continue;
            };
            if !last.is_finite() || last <= 0.0 {
                continue;
            }
            let should_send = chart.last_paper_quote.is_none_or(|(price, sent_at)| {
                let elapsed = sent_at.elapsed();
                (price != last && elapsed >= PAPER_QUOTE_FLUSH_INTERVAL)
                    || elapsed >= REFRESH_INTERVAL
            });
            if !should_send {
                continue;
            }
            let instrument = Self::paper_instrument(chart);
            let _ = host.process_quote(PaperQuote {
                instrument_key: instrument.key,
                bid: None,
                ask: None,
                last: Some(last),
                timestamp,
                is_market_open: true,
            });
            chart.last_paper_quote = Some((last, now));
        }
    }

    fn configure_paper_cards(&mut self, host: &PaperHost) {
        let account_id = host.snapshot().selected_account_id;
        let account = account_id.and_then(|id| {
            host.snapshot()
                .accounts
                .iter()
                .find(|account| account.id == id)
        });
        let enabled = self.paper_panel_visible
            && self.paper_panel.show_trading_on_charts()
            && account.is_some();
        let currency = account.map_or(PaperCurrency::Usd, |account| account.base_currency);
        for chart in &mut self.charts {
            let key = Self::paper_instrument(chart).key;
            let chart_enabled =
                enabled && chart.is_market() && Self::supports_paper_trading(chart.source);
            chart.chart.paper.trading_enabled = chart_enabled;
            chart.chart.paper.show_markers = chart_enabled;
            chart.chart.paper.currency = currency;
            chart.chart.paper.positions = host
                .snapshot()
                .positions
                .iter()
                .filter(|position| {
                    account.is_some_and(|account| account.settings.show_positions_on_chart)
                        && Some(position.account_id) == account_id
                        && position.instrument.key == key
                })
                .map(|position| (position.clone(), host.unrealized_pnl(position)))
                .collect();
            chart.chart.paper.orders = host
                .snapshot()
                .orders
                .iter()
                .filter(|order| {
                    account.is_some_and(|account| account.settings.show_orders_on_chart)
                        && Some(order.account_id) == account_id
                        && order.instrument.key == key
                        && order.status.is_working()
                })
                .cloned()
                .collect();
        }
    }

    /// The market a chart's bell starts an alert on: its identity, the ticker and a name.
    fn alert_asset(chart: &Chart) -> AlertAsset {
        Self::alert_asset_for(&chart.symbol, chart.source)
    }

    pub fn alert_asset_for(symbol: &str, source: DataSource) -> AlertAsset {
        let display = source.display_symbol(symbol);
        let ticker = display.split('/').next().unwrap_or(&display).to_uppercase();
        AlertAsset::new(
            crate::model::InstrumentId::new(source, symbol),
            ticker.clone(),
            ticker,
        )
    }

    fn paper_instrument(chart: &Chart) -> PaperInstrument {
        PaperInstrument::chart(
            &chart.symbol,
            chart.source.display_symbol(&chart.symbol),
            chart.source,
        )
    }

    fn supports_paper_trading(source: DataSource) -> bool {
        source != DataSource::CoinMarketCap
    }

    fn apply_paper_overlay_actions(host: &mut PaperHost, actions: Vec<PaperOverlayAction>) {
        for action in actions {
            match action {
                PaperOverlayAction::ClosePosition {
                    account_id,
                    instrument_key,
                } => {
                    let _ = host.close_position(account_id, &instrument_key);
                }
                PaperOverlayAction::ModifyOrder { order_id, price } => {
                    let _ = host.move_order(order_id, price);
                }
                PaperOverlayAction::CancelOrder(id) => {
                    let _ = host.cancel(id);
                }
            }
        }
    }

    // MARK: Grid

    /// An empty tab: a search launcher and shortcuts back into favorites, recent markets and
    /// saved layouts.
    fn show_empty_state(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let focused = ui.input(|input| input.viewport().focused.unwrap_or(false));
        let revealed = self
            .empty_page
            .last_drawn
            .is_none_or(|drawn| drawn.elapsed() > EMPTY_PAGE_REVEAL_GAP);
        self.empty_page.last_drawn = Some(std::time::Instant::now());
        let regained_focus = (focused && !self.empty_page.was_focused) || revealed;
        self.empty_page.was_focused = focused;
        let mut page = match self.empty_page.data.take() {
            Some(page) if !regained_focus => page,
            _ => self.load_empty_page(),
        };

        // Favorites are in memory and follow the sidebar live.
        let favorites_list: Vec<WatchlistInstrument> = self
            .watchlist
            .lock()
            .map(|store| {
                let id = store.favorites();
                store
                    .lists()
                    .iter()
                    .find(|list| list.id == id)
                    .map(|list| list.instruments().cloned().collect())
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        let hidden_favorites = favorites_list.len().saturating_sub(empty_state::ITEM_LIMIT);
        let favorites: Vec<WatchlistInstrument> = favorites_list
            .into_iter()
            .take(empty_state::ITEM_LIMIT)
            .collect();
        let kept = empty_state::trim_recents(&page.recents, &favorites);
        let recents: Vec<add_chart_sheet::RecentMarket> = kept
            .iter()
            .map(|index| page.recents[*index].clone())
            .collect();
        let input = empty_state::EmptyStateInput {
            favorites: &favorites,
            hidden_favorites,
            recents: &recents,
            layouts: &page.layouts,
            offers_stocks: page.offers_stocks,
        };
        let actions = empty_state::show(ui, &input);
        for action in actions {
            match action {
                EmptyStateAction::AddChart => self.add_chart.open(),
                EmptyStateAction::OpenFavorite(index) => {
                    self.add_chart_from_instrument(&favorites[index], &ctx);
                }
                EmptyStateAction::OpenRecent(index) => {
                    self.add_from_search(&recents[index].result(), &ctx);
                }
                EmptyStateAction::OpenLayout(id) => self.request_open_saved_view(id, &ctx),
                EmptyStateAction::OpenSuggestion(source, symbol, display) => {
                    self.add_from_search(
                        &SearchResult {
                            symbol: display,
                            full_symbol: symbol,
                            source,
                            price: None,
                            event: None,
                        },
                        &ctx,
                    );
                }
                EmptyStateAction::RemoveRecent(index) => {
                    page.recents.remove(kept[index]);
                    self.store_recents(&page.recents);
                }
                EmptyStateAction::ClearRecents => {
                    page.recents.clear();
                    self.store_recents(&page.recents);
                }
                EmptyStateAction::ShowFavorites => self.watchlist_visible = true,
            }
        }
        self.empty_page.data = Some(page);
    }

    fn load_empty_page(&self) -> EmptyPageData {
        let recents = self
            .store
            .lock()
            .map(|store| add_chart_sheet::load_recents(&store))
            .unwrap_or_default();
        let mut views = self.load_saved_views();
        views.sort_by_key(|view| std::cmp::Reverse(view.created_at_ms));
        let layouts = views
            .iter()
            .map(|view| empty_state::LayoutSummary {
                id: view.id,
                name: view.name.clone(),
                chart_count: view.charts.len(),
                timeframe: view.timeframe.label(),
                previews: view
                    .charts
                    .iter()
                    .filter(|chart| {
                        chart.card.is_none() && chart.source != DataSource::CoinMarketCap
                    })
                    .map(|chart| (chart.source, chart.symbol.clone()))
                    .collect(),
            })
            .collect();
        EmptyPageData {
            recents,
            layouts,
            offers_stocks: crate::service::keychain_api_keys::alpaca_credentials().is_some(),
        }
    }

    fn store_recents(&self, recents: &[add_chart_sheet::RecentMarket]) {
        if let Ok(store) = self.store.lock() {
            add_chart_sheet::store_recents(&store, recents);
        }
    }

    /// While a chart is dragged: finds what is under the pointer, draws its cue (an insert
    /// line, or the new-column strip and its preview) and, on release, queues the move.
    #[allow(clippy::too_many_arguments)]
    fn drive_grid_drag(
        &mut self,
        ctx: &egui::Context,
        dragged: Option<u64>,
        grid_rect: egui::Rect,
        mut zones: ui::grid_drop::Zones,
        can_add_column: bool,
        previewed: Option<u64>,
        commands: &mut Vec<ChartCommand>,
    ) {
        use ui::grid_drop::{self, DropTarget};
        let Some(dragged) = dragged else {
            return;
        };
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            egui::DragAndDrop::clear_payload(ctx);
            self.grid_drag.reset();
            return;
        }
        if can_add_column && zones.new_column.is_none() {
            zones.new_column = Some(egui::Rect::from_min_max(
                egui::pos2(
                    grid_rect.right() - grid_drop::NEW_COLUMN_STRIP_WIDTH,
                    grid_rect.top(),
                ),
                grid_rect.right_bottom(),
            ));
        }
        let target = ctx
            .pointer_latest_pos()
            .and_then(|pos| zones.target_at(pos));
        if let Some(wait) = self.grid_drag.hover(dragged, target, Instant::now()) {
            ctx.request_repaint_after(wait);
        }

        // Cues sit above the cards, on the grid's own layer.
        let painter_ui = egui::Ui::new(
            ctx.clone(),
            egui::Id::new("grid-drop-cues"),
            egui::UiBuilder::new()
                .layer_id(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::new("grid-drop-cues-layer"),
                ))
                .max_rect(grid_rect),
        );
        if can_add_column
            && previewed.is_none()
            && let Some(strip) = zones.new_column
        {
            grid_drop::new_column_strip(&painter_ui, strip, target == Some(DropTarget::NewColumn));
        }
        match target {
            Some(DropTarget::Before { chart, .. }) if chart != dragged => {
                if let Some((_, _, rect)) = zones.cards.iter().find(|(_, id, _)| *id == chart) {
                    grid_drop::insert_line_above(&painter_ui, *rect);
                }
            }
            Some(DropTarget::Append { column }) => {
                if let Some((_, rect)) = zones.columns.iter().find(|(id, _)| *id == column) {
                    grid_drop::insert_line_below(&painter_ui, *rect);
                }
            }
            _ => {}
        }
        if let Some(chart) = self.charts.iter().find(|chart| chart.id == dragged) {
            grid_drop::drag_label(ctx, &chart.display_title());
        }
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);

        if ctx.input(|input| input.pointer.any_released()) {
            match target {
                Some(DropTarget::Before { column, chart }) if chart != dragged => {
                    commands.push(ChartCommand::Move {
                        chart_id: dragged,
                        to_column: column,
                        before: Some(chart),
                    });
                }
                Some(DropTarget::Append { column }) => commands.push(ChartCommand::Move {
                    chart_id: dragged,
                    to_column: column,
                    before: None,
                }),
                Some(DropTarget::NewColumn) => commands.push(ChartCommand::NewColumn(dragged)),
                _ => {}
            }
            self.grid_drag.reset();
        }
    }

    fn render_grid(&mut self, ui: &mut egui::Ui, paper_host: &mut PaperHost) {
        if self.charts.is_empty() {
            self.show_empty_state(ui);
            return;
        }
        // The next visit to the empty page starts from fresh data.
        self.empty_page.data = None;

        let available = ui.available_size();
        let column_count = self.columns.len().max(1);
        let max_rows = self
            .columns
            .iter()
            .map(|column| column.chart_ids.len())
            .max()
            .unwrap_or(0);
        let card_height =
            ChartLayout::grid_card_height(available.y, max_rows, ui.spacing().item_spacing.y);

        let plan = self.columns.clone();
        let index_map: HashMap<u64, usize> = self
            .charts
            .iter()
            .enumerate()
            .map(|(index, chart)| (chart.id, index))
            .collect();
        let focused = self.focused;
        let multi = self.charts.len() > 1;
        let crosshair = self.crosshair;
        let mut crosshair_update: Option<CrosshairInfo> = None;
        let mut hover_update: Option<(u64, crate::model::Anchor)> = None;
        let active_tool = self.active_tool;
        let selected_drawing = self.selected_drawing.clone();
        let (magnet, keep_drawing, hide_all_drawings) =
            (self.magnet, self.keep_drawing, self.hide_all_drawings);
        let replay_ms = if self.replay.is_active() {
            self.replay.current_ms()
        } else {
            None
        };
        let replay_selecting = self.replay.status == ReplayStatus::SelectingStart;
        let timeframe = self.timeframe;
        let hosting_note = match self.snapshot_hosting() {
            crate::snapshot_host::Hosting::Ready(_) => None,
            crate::snapshot_host::Hosting::NotConfigured => {
                Some("Set a snapshot host in Settings ▸ Snapshots to share.")
            }
        };
        let snapshot_shortcut =
            |action| shortcut::snapshot(action).map(|shortcut| shortcut.label());
        let snapshot_menu = ui::snapshot_menu::SnapshotMenu {
            busy: self.snapshot_busy(),
            hosting_unavailable: hosting_note,
            shortcut: &snapshot_shortcut,
        };
        let replay_active = self.replay.is_active();
        let mut replay_picked: Option<(u64, i64)> = None;
        // Linked navigation needs another chart to follow.
        let sync = self.sync;
        let linked = self.market_charts() > 1 && sync.is_linked();
        let mut navigated = false;

        let mut drawing_commands: Vec<(u64, Vec<DrawingCommand>)> = Vec::new();
        let mut drawing_select: Option<(u64, Vec<u64>)> = None;
        let mut drawing_disarm = false;
        let mut drawing_properties_request: Option<(u64, u64)> = None;
        let mut drawing_style_copied: Option<(Tool, DrawingStyle)> = None;
        let style_clipboard = self.style_clipboard;
        let dialog_open = self.drawing_props.is_some()
            || self.confirm_remove_drawings.is_some()
            || self.add_chart.is_open();
        let mut drawing_reveal = false;
        let mut drawing_copy = false;
        let mut drawing_alert: Option<(u64, f64)> = None;
        let drawing_defaults = self.drawing_templates().defaults().clone();
        let drawing_preview: Option<(u64, Drawing)> = self
            .drawing_props
            .as_ref()
            .map(|props| (props.chart_id, props.draft().clone()));
        let mut drawing_escape = false;
        let mut paper_actions = Vec::new();
        let mut quick_trade = None;
        let mut new_alert: Option<AlertRequest> = None;
        let mut context_request: Option<(u64, ui::ContextRequest)> = None;
        let (favorites, watchlist_memberships): (HashSet<InstrumentId>, Vec<_>) = {
            let store = self.watchlist.lock().expect("watchlist lock poisoned");
            let favorites = store
                .lists()
                .iter()
                .find(|list| list.is_favorites)
                .into_iter()
                .flat_map(|list| list.instruments())
                .map(|item| item.instrument.clone())
                .collect();
            let memberships = store
                .lists()
                .iter()
                .map(|list| {
                    (
                        list.id,
                        list.name.clone(),
                        list.instruments()
                            .map(|item| item.instrument.clone())
                            .collect::<HashSet<_>>(),
                    )
                })
                .collect();
            (favorites, memberships)
        };
        let mut favorite_toggles = Vec::new();
        let mut watchlist_changes = Vec::new();
        let card_env = self.card_env.clone();

        let mut commands: Vec<ChartCommand> = Vec::new();
        let can_add_column = ChartLayout::can_add_column(available.x, column_count);
        let ctx = ui.ctx().clone();
        // Nothing about a drag shows until a chart is being dragged.
        let dragged: Option<u64> = egui::DragAndDrop::payload::<u64>(&ctx).map(|payload| *payload);
        if dragged.is_none() {
            self.grid_drag.reset();
        }
        let previewed = if can_add_column {
            self.grid_drag.previewed()
        } else {
            None
        };
        let mut zones = ui::grid_drop::Zones::default();
        let grid_rect = ui.available_rect_before_wrap();

        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(available.x, available.y),
                egui::Layout::left_to_right(egui::Align::Min),
                |ui| {
                    ui.columns(column_count + usize::from(previewed.is_some()), |uis| {
                        for (column_index, column_ui) in uis.iter_mut().enumerate() {
                            let Some(column) = plan.get(column_index) else {
                                // The previewed new column: a dashed placeholder where it would be.
                                let rect = column_ui.max_rect();
                                let card = egui::Rect::from_min_size(
                                    rect.min,
                                    egui::vec2(rect.width(), card_height),
                                );
                                ui::grid_drop::new_column_placeholder(column_ui, card);
                                zones.new_column = Some(egui::Rect::from_min_size(
                                    rect.min,
                                    egui::vec2(rect.width(), available.y),
                                ));
                                continue;
                            };
                            let column_rect = column_ui.max_rect();
                            zones.columns.push((
                                column.id,
                                egui::Rect::from_min_size(
                                    column_rect.min,
                                    egui::vec2(column_rect.width(), available.y),
                                ),
                            ));
                            for &chart_id in &column.chart_ids {
                                let Some(&index) = index_map.get(&chart_id) else {
                                    continue;
                                };
                                let is_focused = multi && focused == Some(chart_id);
                                let card = egui::Frame::new()
                                    .fill(ui::theme::surface())
                                    .corner_radius(10.0)
                                    .inner_margin(egui::Margin::same(6))
                                    .stroke(egui::Stroke::new(
                                        1.0,
                                        if replay_ms.is_some() {
                                            ui::theme::REPLAY.gamma_multiply(0.45)
                                        } else if is_focused {
                                            ui::theme::ACCENT
                                        } else {
                                            ui::theme::border()
                                        },
                                    ));

                                let drawn = column_ui.allocate_ui_with_layout(
                                    egui::vec2(column_ui.available_width(), card_height),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        if previewed == Some(chart_id) {
                                            ui.multiply_opacity(0.35);
                                        }
                                        let chart = &mut self.charts[index];
                                        if let Some(mut shown) = chart.card {
                                            let instrument = shown.instrument_id();
                                            let memberships: Vec<crate::ui::WatchlistMembership> =
                                                watchlist_memberships
                                                    .iter()
                                                    .map(|(id, name, instruments)| {
                                                        crate::ui::WatchlistMembership {
                                                            id: *id,
                                                            name: name.clone(),
                                                            contains_market: instruments
                                                                .contains(&instrument),
                                                        }
                                                    })
                                                    .collect();
                                            return card.show(ui, |ui| {
                                                let mut action = ChartCardAction::default();
                                                let data = card_env.as_ref().map(|env| {
                                                    let runtime = chart
                                                        .card_runtime
                                                        .get_or_insert_with(|| env.runtime());
                                                    env.view(runtime, &shown, ui.ctx())
                                                });
                                                let scope = data.as_ref().map_or_else(
                                                    || shown.scope_name(&[]),
                                                    |data| data.scope_name.clone(),
                                                );
                                                ui::portfolio_cards::header(
                                                    ui,
                                                    &shown,
                                                    &scope,
                                                    chart_id,
                                                    favorites.contains(&instrument),
                                                    &memberships,
                                                    &mut action,
                                                );
                                                if ui::portfolio_cards::body(
                                                    ui,
                                                    &mut shown,
                                                    data.as_ref(),
                                                    chart_id,
                                                ) {
                                                    action.focus_requested = true;
                                                }
                                                chart.card = Some(shown);
                                                action
                                            });
                                        }
                                        chart.chart.set_series_cutoff(replay_ms);
                                        chart.chart.set_timeframe(timeframe);
                                        let candles = displayed_candles(&chart.candles, replay_ms);
                                        if linked {
                                            chart.chart.follow_sync(
                                                &mut self.sync_hub,
                                                chart_id,
                                                sync,
                                                chart.source,
                                                candles,
                                            );
                                        }
                                        let instrument =
                                            InstrumentId::new(chart.source, &chart.symbol);
                                        let memberships: Vec<crate::ui::WatchlistMembership> =
                                            watchlist_memberships
                                                .iter()
                                                .map(|(id, name, instruments)| {
                                                    crate::ui::WatchlistMembership {
                                                        id: *id,
                                                        name: name.clone(),
                                                        contains_market: instruments
                                                            .contains(&instrument),
                                                    }
                                                })
                                                .collect();
                                        card.show(ui, |ui| {
                                            let mut action = ChartCardAction::default();
                                            chart.chart.header(
                                                ui,
                                                &chart.symbol,
                                                chart.source,
                                                &chart.indicators,
                                                candles,
                                                replay_ms.is_some(),
                                                favorites.contains(&instrument),
                                                &memberships,
                                                &mut action,
                                                &snapshot_menu,
                                                chart_id,
                                            );
                                            let selected = selected_drawing
                                                .as_ref()
                                                .filter(|(chart, _)| *chart == chart_id)
                                                .map(|(_, drawings)| drawings.clone())
                                                .unwrap_or_default();
                                            let mut frame = DrawingFrame::new(
                                                &chart.drawings,
                                                active_tool,
                                                None,
                                                chart.next_drawing_id,
                                            )
                                            .with_selection(selected)
                                            .with_view(timeframe, hide_all_drawings)
                                            .with_behavior(magnet, keep_drawing)
                                            .with_style_clipboard(style_clipboard)
                                            .with_keys_blocked(dialog_open)
                                            .with_defaults(&drawing_defaults)
                                            .with_preview(
                                                drawing_preview
                                                    .as_ref()
                                                    .filter(|(preview_chart, _)| {
                                                        *preview_chart == chart_id
                                                    })
                                                    .map(|(_, draft)| draft),
                                            );
                                            let mut replay_frame = ReplayChartFrame {
                                                selecting: replay_selecting,
                                                active: replay_active,
                                                picked_ms: None,
                                            };
                                            let pointer = chart.chart.body(
                                                ui,
                                                &chart.symbol,
                                                candles,
                                                chart.loading,
                                                chart.error.as_deref(),
                                                chart.log_scale,
                                                chart.source,
                                                &chart.indicators,
                                                crosshair.as_ref(),
                                                chart_id,
                                                &mut frame,
                                                &mut replay_frame,
                                            );
                                            if let Some(picked_ms) = replay_frame.picked_ms {
                                                replay_picked = Some((chart_id, picked_ms));
                                            }
                                            if linked
                                                && chart.chart.publish_sync(
                                                    &mut self.sync_hub,
                                                    chart_id,
                                                    sync,
                                                )
                                            {
                                                navigated = true;
                                            }
                                            chart.next_drawing_id = frame.next_id;
                                            if let Some(select) = frame.select {
                                                drawing_select = Some((chart_id, select));
                                            }
                                            drawing_escape |= frame.escape_consumed;
                                            drawing_disarm |= frame.disarm;
                                            drawing_reveal |= frame.reveal_all;
                                            drawing_copy |= frame.copy_requested;
                                            if let Some(price) = frame.alert_price {
                                                drawing_alert = Some((chart_id, price));
                                            }
                                            if frame.style_copied.is_some() {
                                                drawing_style_copied = frame.style_copied;
                                            }
                                            if let Some(drawing_id) = frame.open_properties {
                                                drawing_properties_request =
                                                    Some((chart_id, drawing_id));
                                            }
                                            let frame_commands =
                                                std::mem::take(&mut frame.commands);
                                            if !frame_commands.is_empty() {
                                                drawing_commands.push((chart_id, frame_commands));
                                            }
                                            if pointer.clicked {
                                                action.focus_requested = true;
                                            }
                                            if let Some(anchor) = pointer.hover_anchor {
                                                hover_update = Some((chart_id, anchor));
                                            }
                                            if let Some(request) = pointer.context.clone() {
                                                context_request = Some((chart_id, request));
                                            }
                                            if let (Some(x_fraction), Some(price)) =
                                                (pointer.hover_x_fraction, pointer.hover_price)
                                            {
                                                crosshair_update = Some(CrosshairInfo {
                                                    owner_id: chart_id,
                                                    x_fraction,
                                                    price,
                                                });
                                            }
                                            action
                                        })
                                    },
                                );

                                if drawn.inner.inner.focus_requested {
                                    commands.push(ChartCommand::Focus(chart_id));
                                }
                                if drawn.inner.inner.remove_requested {
                                    commands.push(ChartCommand::Remove(chart_id));
                                }
                                if let Some(chart_type) = drawn.inner.inner.chart_type {
                                    commands.push(ChartCommand::SetChartType(chart_id, chart_type));
                                }
                                if let Some(action) = drawn.inner.inner.snapshot {
                                    commands.push(ChartCommand::Snapshot(chart_id, action));
                                }
                                if drawn.inner.inner.settings_requested {
                                    commands.push(ChartCommand::Settings(chart_id));
                                }
                                if let Some(action) = drawn.inner.inner.indicator_action {
                                    commands.push(ChartCommand::Indicator(chart_id, action));
                                }
                                if drawn.inner.inner.new_alert {
                                    new_alert = Some(AlertRequest::market(Self::alert_asset(
                                        &self.charts[index],
                                    )));
                                }
                                if drawn.inner.inner.toggle_favorite {
                                    favorite_toggles.push(chart_id);
                                }
                                if let Some(change) = drawn.inner.inner.watchlist_change {
                                    watchlist_changes.push((chart_id, change));
                                }
                                if let Some(side) = drawn.inner.inner.quick_trade {
                                    let reference_price =
                                        displayed_candles(&self.charts[index].candles, replay_ms)
                                            .last()
                                            .map(|candle| candle.close);
                                    quick_trade = Some((chart_id, side, reference_price));
                                }
                                paper_actions.append(&mut self.charts[index].chart.paper.actions);
                                zones.cards.push((column.id, chart_id, drawn.response.rect));
                            }
                        }
                    });
                },
            );
        });

        self.sync_hub.end_frame();
        if navigated {
            // Charts drawn before the one being navigated hear of it one frame later: ask for it
            // at once.
            ctx.request_repaint();
        }

        self.drive_grid_drag(
            &ctx,
            dragged,
            grid_rect,
            zones,
            can_add_column,
            previewed,
            &mut commands,
        );

        if !favorite_toggles.is_empty() || !watchlist_changes.is_empty() {
            let mut store = self.watchlist.lock().expect("watchlist lock poisoned");
            let list_id = store.favorites();
            for chart_id in favorite_toggles {
                if let Some(chart) = self.charts.iter().find(|chart| chart.id == chart_id) {
                    let instrument = Self::chart_instrument(chart);
                    if favorites.contains(&instrument) {
                        store.remove_from_list(list_id, &instrument);
                    } else {
                        store.add_to_list(list_id, self.favorite_item(chart));
                    }
                }
            }
            for (chart_id, (list_id, included)) in watchlist_changes {
                if let Some(chart) = self.charts.iter().find(|chart| chart.id == chart_id) {
                    let instrument = Self::chart_instrument(chart);
                    if included {
                        store.add_to_list(list_id, self.favorite_item(chart));
                    } else {
                        store.remove_from_list(list_id, &instrument);
                    }
                }
            }
        }

        for command in commands {
            match command {
                ChartCommand::Focus(chart_id) => self.focus_chart(chart_id),
                ChartCommand::Snapshot(chart_id, action) => {
                    self.begin_snapshot(&ctx, chart_id, action);
                }
                ChartCommand::Remove(chart_id) => self.removing_chart = Some(chart_id),
                ChartCommand::Settings(chart_id) => {
                    self.settings_state = ChartSettingsState::default();
                    self.settings_chart = Some(chart_id);
                }
                ChartCommand::Indicator(chart_id, action) => {
                    use crate::ui::chart_indicator_strip::{Action, Kind};
                    match action {
                        Action::OpenIndicatorSettings => {
                            self.settings_state = ChartSettingsState::default();
                            self.settings_chart = Some(chart_id);
                        }
                        Action::ConfigureScript(id, anchor) => {
                            self.toggle_script_popover(chart_id, id, anchor);
                        }
                        Action::OpenScriptSettings => {
                            self.settings_state = ChartSettingsState::default();
                            self.settings_state.tab = chart_settings::Tab::Scripts;
                            self.settings_chart = Some(chart_id);
                        }
                        _ => {
                            if let Some(chart) = self.charts.iter_mut().find(|c| c.id == chart_id) {
                                match action {
                                    Action::Add(kind) => {
                                        kind.set_applied(&mut chart.indicators, true)
                                    }
                                    Action::Remove(kind) => {
                                        kind.set_applied(&mut chart.indicators, false)
                                    }
                                    Action::EmaPeriod(period) => {
                                        chart.indicators.ema_period = period
                                    }
                                    Action::ToggleVisible(Kind::BuiltIn(kind)) => {
                                        kind.toggle_visible(&mut chart.indicators);
                                    }
                                    Action::ToggleVisible(Kind::Script(id)) => {
                                        if let Some(instance) =
                                            chart.chart.pine_instances().iter().find(|i| i.id == id)
                                        {
                                            chart.chart.set_pine_visible(id, !instance.visible);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                ChartCommand::Move {
                    chart_id,
                    to_column,
                    before,
                } => self.move_chart(chart_id, to_column, before),
                ChartCommand::NewColumn(chart_id) => self.move_to_new_trailing_column(chart_id),
                ChartCommand::SetChartType(chart_id, chart_type) => {
                    if let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == chart_id)
                        && chart_type.supported_by(chart.source)
                    {
                        chart.chart.set_chart_type(chart_type);
                    }
                }
            }
        }

        self.crosshair = crosshair_update;
        if let Some((chart_id, anchor)) = hover_update {
            self.note_hover(chart_id, Some(anchor));
        }
        if let Some((chart_id, request)) = context_request {
            self.open_context_menu(chart_id, request);
        }

        if let Some((chart_id, picked_ms)) = replay_picked {
            let (symbol, timeline) = self
                .charts
                .iter()
                .find(|chart| chart.id == chart_id)
                .map(|chart| {
                    (
                        chart.symbol.clone(),
                        chart.candles.iter().map(|kline| kline.open_time).collect(),
                    )
                })
                .unwrap_or_default();
            self.replay.start(picked_ms, &symbol, timeline);
        }

        for (chart_id, drawing) in drawing_commands {
            self.apply_drawing_commands(chart_id, drawing);
        }
        Self::apply_paper_overlay_actions(paper_host, paper_actions);
        if new_alert.is_some() {
            self.new_alert_request = new_alert;
        }
        if let Some((chart_id, side, reference_price)) = quick_trade
            && let Some(chart) = self.charts.iter().find(|chart| chart.id == chart_id)
        {
            let instrument = Self::paper_instrument(chart);
            self.paper_panel
                .open_ticket(paper_host, instrument, reference_price, side);
        }
        if let Some((chart_id, select)) = drawing_select {
            self.selected_drawing = (!select.is_empty()).then_some((chart_id, select));
        }
        if drawing_disarm {
            self.active_tool = Tool::Cursor;
        }
        if drawing_reveal {
            self.hide_all_drawings = false;
        }
        if drawing_copy {
            self.copy_selection(&ctx);
        }
        if let Some((chart_id, price)) = drawing_alert {
            self.start_alert_at(chart_id, Some(price));
        }
        if drawing_style_copied.is_some() {
            self.style_clipboard = drawing_style_copied;
        }
        if let Some((chart_id, drawing_id)) = drawing_properties_request {
            self.open_drawing_properties(chart_id, drawing_id);
        }
        let esc_pressed = ui.input(|input| input.key_pressed(egui::Key::Escape));
        if esc_pressed && !drawing_escape && !self.date_sheet.is_open() && !self.context_menu_open()
        {
            if self.replay.status == ReplayStatus::SelectingStart {
                self.replay.cancel_selection();
            } else if self.active_tool != Tool::Cursor {
                self.active_tool = Tool::Cursor;
            } else {
                self.selected_drawing = None;
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, paper_host: &mut PaperHost) -> Vec<WorkspaceAction> {
        let ctx = ui.ctx().clone();
        let timeframe = self.timeframe;
        for chart in &mut self.charts {
            chart.receive_fetch();
            chart.receive_older(timeframe);
            chart.request_older(timeframe, &ctx);
            if chart.is_market()
                && chart.inbox.is_none()
                && chart.last_fetch.elapsed() >= chart.refresh_after()
            {
                chart.fetch(timeframe, &ctx);
            }
        }

        self.update_live_feed(&ctx);
        self.sync_paper_quotes(paper_host);
        self.configure_paper_cards(paper_host);
        self.handle_keyboard(ui);
        self.handle_clipboard(ui);
        self.tick_replay();

        self.add_chart.tick(&ctx);

        ctx.request_repaint_after(Duration::from_millis(250));

        let mut actions = egui::Panel::top("workspace_toolbar")
            .exact_size(workspace_toolbar::HEIGHT)
            .frame(egui::Frame::new().fill(ui::theme::chrome()))
            .show(ui, |ui| self.show_toolbar(ui, &ctx))
            .inner;

        if self.replay.is_bar_visible() {
            // The bar paints its own tinted bottom line, so egui's separator would double it.
            egui::Panel::top("replay_bar")
                .show_separator_line(false)
                .exact_size(replay_bar::preferred_height(
                    self.replay.status,
                    ui.available_width(),
                    false,
                ))
                .frame(
                    egui::Frame::new()
                        .fill(ui::theme::chrome())
                        .inner_margin(egui::Margin::ZERO),
                )
                .show(ui, |ui| self.show_replay_bar(ui));
        }

        let rail_actions = egui::Panel::left("tool_rail")
            .exact_size(tool_sidebar::WIDTH)
            .resizable(false)
            .frame(egui::Frame::new().fill(ui::theme::chrome()))
            .show(ui, |ui| self.show_tool_rail(ui))
            .inner;
        actions.extend(rail_actions);

        if self.watchlist_visible {
            let minimum_width = self
                .watchlist
                .lock()
                .ok()
                .map(|store| self.watchlist_sidebar.minimum_width(store.lists()))
                .unwrap_or(286.0);
            egui::Panel::right("watchlist")
                .default_size(minimum_width.max(270.0))
                .min_size(minimum_width)
                .max_size(640.0)
                .frame(
                    egui::Frame::new()
                        .fill(ui::theme::chrome())
                        .inner_margin(egui::Margin::ZERO),
                )
                .show(ui, |ui| self.show_watchlist(ui, &ctx));
        }

        if self.paper_panel_visible {
            let panel_actions = egui::Panel::bottom(egui::Id::new(("paper_panel", self.id)))
                .resizable(true)
                .default_size(340.0)
                .min_size(280.0)
                .frame(egui::Frame::new().fill(ui::kit::window()))
                .show(ui, |ui| self.paper_panel.ui(ui, paper_host))
                .inner;
            for action in panel_actions {
                match action {
                    PaperPanelAction::Close => self.paper_panel_visible = false,
                    // File dialogs/export plumbing is a follow-up; the panel keeps the action
                    // explicit instead of silently exporting to an arbitrary directory.
                    PaperPanelAction::Export => {}
                }
            }
        }

        self.sync_context_panels();
        if self.context.tree_chart.is_some() {
            egui::Panel::right(egui::Id::new(("object_tree", self.id)))
                .resizable(true)
                .default_size(280.0)
                .min_size(220.0)
                .max_size(520.0)
                .frame(
                    egui::Frame::new()
                        .fill(ui::theme::chrome())
                        .inner_margin(egui::Margin::ZERO),
                )
                .show(ui, |ui| self.show_tree_panel(ui));
        }
        if self.context.table_chart.is_some() {
            egui::Panel::bottom(egui::Id::new(("chart_table", self.id)))
                .resizable(true)
                .default_size(260.0)
                .min_size(150.0)
                .frame(egui::Frame::new().fill(ui::kit::window()))
                .show(ui, |ui| self.show_table_panel(ui));
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(ui::theme::background())
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ui, |ui| self.render_grid(ui, paper_host));
        if let Some(item) = self.open_new_tab_request.take() {
            actions.push(WorkspaceAction::OpenWatchlistInNewTab(item));
        }

        if self.add_chart.is_open()
            && let Some(env) = &self.card_env
        {
            self.add_chart.set_portfolios(env.portfolios());
        }
        match self.add_chart.show(&ctx) {
            Some(AddChartAction::AddCard(card)) => {
                if let Some(list_id) = self
                    .add_to_watchlist
                    .filter(|_| self.add_chart.is_watchlist_mode())
                {
                    let scope = self
                        .card_env
                        .as_ref()
                        .map_or_else(Vec::new, PortfolioCardEnv::portfolios);
                    if let Ok(mut store) = self.watchlist.lock() {
                        store.add_to_list(
                            list_id,
                            WatchlistInstrument::for_card(card, &card.scope_name(&scope)),
                        );
                    }
                } else {
                    self.add_card(card);
                }
            }
            Some(AddChartAction::Add(result)) => {
                if let Some(list_id) = self
                    .add_to_watchlist
                    .filter(|_| self.add_chart.is_watchlist_mode())
                {
                    if let Ok(mut store) = self.watchlist.lock() {
                        let mut item = WatchlistInstrument::from_market(
                            result.source,
                            &result.symbol,
                            &result.full_symbol,
                        );
                        item.image_url = result
                            .event
                            .as_ref()
                            .and_then(|event| event.image_url.clone());
                        store.add_to_list(list_id, item);
                    }
                } else {
                    self.add_from_search(&result, &ctx);
                }
            }
            Some(AddChartAction::OpenAlpacaSettings) => {
                actions.push(WorkspaceAction::OpenAlpacaSettings);
            }
            None => {}
        }
        if !self.add_chart.is_open() {
            self.add_to_watchlist = None;
        }
        // Before the sheet, so the popover takes an Esc the sheet would otherwise act on.
        self.show_script_popover(&ctx);
        self.show_drawing_properties(&ctx);
        self.confirm_drawings_removal(&ctx);
        if self.settings_chart.is_some() {
            actions.extend(self.show_settings_window(&ctx));
        }
        if let Some(chart_id) = self.removing_chart {
            self.confirm_chart_removal(&ctx, chart_id);
        }
        actions.extend(self.show_alert_editor(&ctx));
        actions.extend(self.show_context_ui(&ctx, paper_host));
        if self.date_sheet.is_open() {
            let timeline = self.replay_timeline();
            if let Some(replay_date_sheet::ReplayDateSheetResult::Start(ms)) =
                replay_date_sheet::show(&ctx, &mut self.date_sheet, &timeline)
            {
                let (symbol, timeline) = self.focused_chart_data();
                self.replay.start(ms, &symbol, timeline);
            }
        }
        actions
    }
}

impl Drop for ChartWorkspace {
    fn drop(&mut self) {
        self.quote_coordinator.release(self.id);
        self.quote_coordinator
            .release(alert_editor::quote_owner(self.id));
    }
}

#[cfg(test)]
mod restyle_tests {
    use super::*;

    #[test]
    fn restyles_of_one_drawing_in_a_burst_are_one_edit() {
        let now = Instant::now();
        let soon = now + Duration::from_millis(300);
        let late = now + Duration::from_secs(2);
        let last = Some((1, 7, now));
        assert!(restyle_merges(last, 1, 7, soon));
        assert!(
            !restyle_merges(last, 1, 7, late),
            "a pause starts a new edit"
        );
        assert!(!restyle_merges(last, 1, 8, soon), "another drawing");
        assert!(!restyle_merges(last, 2, 7, soon), "another chart");
        assert!(!restyle_merges(None, 1, 7, soon));
    }

    #[test]
    fn only_a_lone_restyle_command_counts_as_one() {
        let drawing = Drawing::Ruler {
            id: 3,
            a: crate::model::Anchor {
                time_ms: 0,
                price: 1.0,
            },
            b: crate::model::Anchor {
                time_ms: 1,
                price: 2.0,
            },
        };
        let style = DrawingCommand::Style {
            id: 3,
            drawing: drawing.clone(),
        };
        assert_eq!(restyled_drawing(std::slice::from_ref(&style)), Some(3));
        assert_eq!(restyled_drawing(&[style.clone(), style]), None);
        assert_eq!(restyled_drawing(&[DrawingCommand::Remove(3)]), None);
        assert_eq!(restyled_drawing(&[]), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_candle_wins_over_an_older_rest_snapshot() {
        let fetched = vec![candle(1, 10.0), candle(2, 20.0)];
        let live = vec![candle(1, 10.0), candle(2, 25.0), candle(3, 30.0)];
        assert_eq!(merge_fetched_candles(fetched.clone(), &live, Some(2)), live);
        assert_eq!(merge_fetched_candles(fetched.clone(), &live, None), fetched);
    }

    /// A chart holding `times`, with a request for older candles outstanding that answers `page`.
    fn chart_awaiting_older(times: &[i64], page: Result<Vec<Kline>, String>) -> Chart {
        let mut chart = Chart::blank(1, "BTCUSDT", DataSource::Binance);
        chart.candles = times.iter().map(|time| candle(*time, 1.0)).collect();
        let (sender, receiver) = mpsc::channel();
        sender.send(page).unwrap();
        chart.older.started();
        chart.older_inbox = Some(OlderRequest {
            receiver,
            source: DataSource::Binance,
            symbol: "BTCUSDT".into(),
            timeframe: Timeframe::Hour,
        });
        chart
    }

    #[test]
    fn rate_limited_sources_refresh_less_often_and_back_off_after_an_error() {
        let mut binance = Chart::blank(1, "BTCUSDT", DataSource::Binance);
        let mut dex = Chart::blank(2, "0xpool", DataSource::DexScreener);
        assert_eq!(binance.refresh_after(), REFRESH_INTERVAL);
        assert_eq!(dex.refresh_after(), SLOW_REFRESH_INTERVAL);
        binance.error = Some("down".into());
        dex.error = Some("429".into());
        assert_eq!(binance.refresh_after(), REFRESH_INTERVAL);
        assert_eq!(dex.refresh_after(), SLOW_RETRY_INTERVAL);
    }

    fn open_times(chart: &Chart) -> Vec<i64> {
        chart.candles.iter().map(|bar| bar.open_time).collect()
    }

    #[test]
    fn an_older_page_goes_in_front_and_a_refresh_does_not_lose_it() {
        let mut chart =
            chart_awaiting_older(&[10, 11, 12], Ok(vec![candle(7, 1.0), candle(8, 1.0)]));
        chart.receive_older(Timeframe::Hour);
        assert_eq!(open_times(&chart), vec![7, 8, 10, 11, 12]);
        assert!(chart.older_inbox.is_none());

        // The next refresh brings only the newest candles.
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Ok(Fetched {
                candles: vec![candle(11, 2.0), candle(12, 2.0), candle(13, 2.0)],
                series: Vec::new(),
            }))
            .unwrap();
        chart.inbox = Some(receiver);
        chart.receive_fetch();
        assert_eq!(open_times(&chart), vec![7, 8, 10, 11, 12, 13]);
    }

    #[test]
    fn a_refresh_replaces_the_candles_while_no_older_page_was_loaded() {
        let mut chart = Chart::blank(1, "BTCUSDT", DataSource::Binance);
        chart.candles = vec![candle(1, 1.0), candle(2, 1.0)];
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Ok(Fetched {
                candles: vec![candle(2, 2.0), candle(3, 2.0)],
                series: Vec::new(),
            }))
            .unwrap();
        chart.inbox = Some(receiver);
        chart.receive_fetch();
        assert_eq!(open_times(&chart), vec![2, 3]);
    }

    #[test]
    fn an_older_page_for_a_timeframe_the_chart_left_is_dropped() {
        let mut chart = chart_awaiting_older(&[10, 11], Ok(vec![candle(7, 1.0)]));
        chart.receive_older(Timeframe::Day);
        assert_eq!(open_times(&chart), vec![10, 11]);
        assert!(chart.older_inbox.is_none());
    }

    #[test]
    fn a_failed_older_page_is_not_retried_at_once() {
        let mut chart = chart_awaiting_older(&[10, 11], Err("offline".into()));
        chart.receive_older(Timeframe::Hour);
        assert_eq!(open_times(&chart), vec![10, 11]);
        assert!(
            !chart.older.should_request(true, true, 2, Instant::now()),
            "it waits before asking again"
        );
    }

    #[test]
    fn an_empty_older_page_is_the_start_of_history() {
        let mut chart = chart_awaiting_older(&[10, 11], Ok(Vec::new()));
        chart.receive_older(Timeframe::Hour);
        assert!(!chart.older.should_request(true, true, 2, Instant::now()));
    }

    #[test]
    fn changing_the_market_forgets_the_history_behind_the_old_one() {
        let mut chart = chart_awaiting_older(&[10, 11], Ok(Vec::new()));
        chart.receive_older(Timeframe::Hour);
        chart.reset_history();
        assert!(chart.older.should_request(true, true, 2, Instant::now()));
    }

    fn candle(open_time: i64, close: f64) -> Kline {
        Kline {
            open_time,
            close_time: open_time + 59_999,
            open: close,
            high: close,
            low: close,
            close,
            volume: 1.0,
        }
    }

    fn empty_workspace(id: u64) -> ChartWorkspace {
        let store: SharedStore = Arc::new(Mutex::new(
            crate::store::Store::open_in_memory().expect("in-memory store"),
        ));
        let book = WatchlistQuoteBook::default();
        ChartWorkspace::empty(
            id,
            Timeframe::Day,
            Arc::new(Mutex::new(WatchlistStore::new(store.clone()))),
            book.clone(),
            WatchlistQuoteCoordinator::new(book, Arc::new(|| {})),
            store,
        )
    }

    #[test]
    fn watchlist_tab_is_titled_by_the_row_not_the_opaque_symbol() {
        let ctx = egui::Context::default();
        let store: SharedStore = Arc::new(Mutex::new(
            crate::store::Store::open_in_memory().expect("in-memory store"),
        ));
        let book = WatchlistQuoteBook::default();
        let mut item = WatchlistInstrument::new(
            InstrumentId::new(
                DataSource::Polymarket,
                "71321045679252212594626385532706912750",
            ),
            "Will it rain tomorrow?",
            "Yes",
        );
        item.display_name = Some("Will it rain tomorrow?".to_owned());
        let workspace = ChartWorkspace::new_for_watchlist(
            3,
            &item,
            Arc::new(Mutex::new(WatchlistStore::new(store.clone()))),
            book.clone(),
            WatchlistQuoteCoordinator::new(book, Arc::new(|| {})),
            store,
            &ctx,
        );
        assert!(workspace.title().starts_with("Will it rain tomorrow?"));
        assert!(!workspace.is_dirty());
    }

    #[test]
    fn agent_script_changes_are_undoable_and_only_real_changes_take_a_step() {
        use crate::pine::program::InputValue;
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(7);
        let btc = InstrumentId::new(DataSource::Binance, "BTCUSDT");
        let chart_id = workspace.add_chart_for(&btc, &ctx).chart_id;
        let base = workspace.undo.len();

        let inputs = HashMap::from([("len".to_owned(), InputValue::Int(50))]);
        assert_eq!(
            workspace.add_pine_instance_to(999, "s1".into(), "SMA", "src", &inputs, true),
            None
        );
        let id = workspace
            .add_pine_instance_to(chart_id, "s1".into(), "SMA", "src", &inputs, false)
            .expect("market chart");
        let instance = |workspace: &ChartWorkspace| {
            workspace
                .pine_instance_views(chart_id, None, false)
                .unwrap()
                .into_iter()
                .map(|view| view.instance)
                .collect::<Vec<_>>()
        };
        let applied = instance(&workspace);
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].script_id.as_deref(), Some("s1"));
        assert!(!applied[0].visible);
        assert_eq!(applied[0].input_values(), inputs);

        // Saying what is already true is not a change, so it is not an undo step.
        assert_eq!(
            workspace.change_pine_instance(chart_id, id, Some(&inputs), Some(false)),
            Some(false)
        );
        assert_eq!(workspace.undo.len(), base + 1);
        assert_eq!(
            workspace.change_pine_instance(chart_id, 99, None, Some(true)),
            None
        );
        assert_eq!(
            workspace.change_pine_instance(chart_id, id, Some(&HashMap::new()), Some(true)),
            Some(true)
        );
        assert!(instance(&workspace)[0].visible);
        assert!(instance(&workspace)[0].inputs.is_empty());

        assert!(workspace.remove_pine_instance_from(chart_id, id));
        assert!(!workspace.remove_pine_instance_from(chart_id, id));
        assert!(instance(&workspace).is_empty());

        workspace.undo();
        assert_eq!(instance(&workspace).len(), 1);
        workspace.undo();
        assert!(!instance(&workspace)[0].visible);
        assert_eq!(instance(&workspace)[0].input_values(), inputs);
        workspace.undo();
        assert!(instance(&workspace).is_empty());
    }

    #[test]
    fn add_chart_for_places_focuses_and_reuses() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(7);
        assert!(workspace.chart_summaries().is_empty());

        let btc = InstrumentId::new(DataSource::Binance, "BTCUSDT");
        let first = workspace.add_chart_for(&btc, &ctx);
        assert!(first.created);
        assert!(workspace.is_dirty());
        assert_eq!(workspace.chart_count(), 1);

        // Adding the same market again (any spelling the provider treats alike) reuses it.
        let again =
            workspace.add_chart_for(&InstrumentId::new(DataSource::Binance, "btcusdt"), &ctx);
        assert_eq!(
            again,
            AddChartOutcome {
                chart_id: first.chart_id,
                created: false
            }
        );
        assert_eq!(workspace.chart_count(), 1);

        let eth =
            workspace.add_chart_for(&InstrumentId::new(DataSource::Coinbase, "ETH-USD"), &ctx);
        assert!(eth.created);
        assert_ne!(eth.chart_id, first.chart_id);

        let summaries = workspace.chart_summaries();
        assert_eq!(summaries.len(), 2);
        let ids: Vec<u64> = summaries.iter().map(|chart| chart.id).collect();
        assert!(ids.contains(&first.chart_id) && ids.contains(&eth.chart_id));
        let focused: Vec<_> = summaries.iter().filter(|chart| chart.focused).collect();
        assert_eq!(focused.len(), 1);
        assert_eq!(focused[0].id, eth.chart_id);
        assert_eq!(focused[0].display_symbol, "ETH/USD");
        assert!(!focused[0].is_line_chart);
    }

    #[test]
    fn a_portfolio_card_is_added_once_and_is_not_a_market() {
        use crate::model::portfolio_card::{CardKind, PortfolioCard};
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(9);
        let btc = workspace.add_chart_for(&InstrumentId::new(DataSource::Binance, "BTCUSDT"), &ctx);
        workspace.mark_clean();

        let allocation = PortfolioCard::new(CardKind::Allocation, None);
        let first = workspace.add_card(allocation);
        assert!(first.created);
        assert!(workspace.is_dirty(), "adding a card changes the layout");
        assert_eq!(workspace.chart_count(), 2);

        // The same kind for the same portfolio is focused, not duplicated; another scope is new.
        let again = workspace.add_card(PortfolioCard::new(CardKind::Allocation, None));
        assert_eq!((again.chart_id, again.created), (first.chart_id, false));
        let scoped = workspace.add_card(PortfolioCard::new(CardKind::Allocation, Some(3)));
        assert!(scoped.created);

        // Cards stay out of everything that reads a market: MCP, scripts, paper, live feeds.
        let summaries = workspace.chart_summaries();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, btc.chart_id);
        assert_eq!(workspace.pine_datasets().len(), 1);
        assert_eq!(workspace.charts_candles().len(), 1);
        let card_chart = workspace
            .charts
            .iter()
            .find(|chart| chart.id == first.chart_id)
            .unwrap();
        assert!(!card_chart.is_market());
        assert!(
            card_chart.inbox.is_none() && !card_chart.loading,
            "a card never fetches"
        );
    }

    #[test]
    fn a_card_survives_a_save_and_changing_its_range_marks_the_layout() {
        use crate::model::portfolio_card::{CardKind, PortfolioCard};
        use crate::model::portfolio_statistics::PortfolioHistoryRange;
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(10);
        workspace.add_card(PortfolioCard::new(CardKind::TopPerformers, Some(2)));
        workspace.mark_clean();

        let saved = workspace.persisted();
        assert_eq!(
            saved.charts[0]
                .card
                .map(|card| (card.kind, card.portfolio_id)),
            Some((CardKind::TopPerformers, Some(2)))
        );

        workspace.charts[0].card.as_mut().unwrap().range = PortfolioHistoryRange::OneYear;
        assert!(workspace.is_dirty(), "a range is part of the layout");

        let store: SharedStore = Arc::new(Mutex::new(
            crate::store::Store::open_in_memory().expect("in-memory store"),
        ));
        let book = WatchlistQuoteBook::default();
        let restored = ChartWorkspace::restored(
            workspace.persisted(),
            Arc::new(Mutex::new(WatchlistStore::new(store.clone()))),
            book.clone(),
            WatchlistQuoteCoordinator::new(book, Arc::new(|| {})),
            store,
            &ctx,
        );
        let chart = &restored.charts[0];
        assert!(
            !chart.is_market() && chart.inbox.is_none(),
            "a restored card does not fetch"
        );
        assert_eq!(chart.card.unwrap().range, PortfolioHistoryRange::OneYear);
        assert_eq!(chart.display_title(), "Top Performers");
    }

    #[test]
    fn a_dex_search_result_keeps_its_pair_address_and_shows_its_ticker() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(8);
        let result = SearchResult {
            symbol: "MCADE/USDC".into(),
            full_symbol: "7pWfX2LongPairAddress".into(),
            source: DataSource::DexScreener,
            price: Some(0.01),
            event: None,
        };

        workspace.add_from_search(&result, &ctx);

        assert_eq!(workspace.charts[0].symbol, "7pWfX2LongPairAddress");
        assert_eq!(workspace.charts[0].display_title(), "MCADE/USDC");
        assert_eq!(
            workspace.persisted().charts[0].title.as_deref(),
            Some("MCADE/USDC")
        );
    }

    #[test]
    fn replacing_a_chart_from_a_watchlist_updates_its_market_metadata() {
        let mut chart = Chart::blank(1, "OldPairAddress", DataSource::DexScreener);
        chart.chart.set_title(Some("OLD/USDC".into()));
        chart.pm_series.push(PmSeries {
            token_id: "old-choice".into(),
            label: "Old choice".into(),
        });

        let mut prediction = WatchlistInstrument::from_market(
            DataSource::Polymarket,
            "Will it rain?",
            "CaseSensitiveToken",
        );
        prediction.image_url = Some("https://img/rain.png".into());
        chart.replace_market(&prediction);

        assert_eq!(chart.symbol, "CaseSensitiveToken");
        assert_eq!(chart.source, DataSource::Polymarket);
        assert_eq!(chart.display_title(), "Will it rain?");
        assert_eq!(chart.image_url, prediction.image_url);
        assert!(chart.pm_series.is_empty());

        let bitcoin = WatchlistInstrument::from_market(DataSource::Binance, "BTCUSDT", "BTCUSDT");
        chart.replace_market(&bitcoin);
        assert_eq!(chart.symbol, "BTCUSDT");
        assert_eq!(chart.display_title(), "BTC/USDT");
        assert_eq!(chart.chart.title(), None);
        assert_eq!(chart.image_url, None);
    }

    #[test]
    fn apply_indicator_adds_updates_and_validates() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(5);
        let btc = workspace
            .add_chart_for(&InstrumentId::new(DataSource::Binance, "BTCUSDT"), &ctx)
            .chart_id;

        let (outcome, config) = workspace.apply_indicator(btc, BuiltIn::Rsi, None).unwrap();
        assert_eq!(outcome, IndicatorOutcome::Added);
        assert!(config.show_rsi);
        assert_eq!(
            workspace
                .apply_indicator(btc, BuiltIn::Rsi, None)
                .unwrap()
                .0,
            IndicatorOutcome::AlreadyApplied
        );

        // EMA: added with a period, then re-configured; an unknown period changes nothing.
        let (outcome, config) = workspace
            .apply_indicator(btc, BuiltIn::Ema, Some(50))
            .unwrap();
        assert_eq!((outcome, config.ema_period), (IndicatorOutcome::Added, 50));
        let (outcome, config) = workspace
            .apply_indicator(btc, BuiltIn::Ema, Some(200))
            .unwrap();
        assert_eq!(
            (outcome, config.ema_period),
            (IndicatorOutcome::Updated, 200)
        );
        assert_eq!(
            workspace.apply_indicator(btc, BuiltIn::Ema, Some(21)),
            Err(IndicatorError::InvalidEmaPeriod)
        );
        assert_eq!(workspace.chart_summaries()[0].indicators.ema_period, 200);

        // Only EMA takes a parameter.
        assert_eq!(
            workspace.apply_indicator(btc, BuiltIn::Bollinger, Some(20)),
            Err(IndicatorError::NoParameters)
        );
        assert!(!workspace.chart_summaries()[0].indicators.show_bollinger);

        // A hidden indicator is shown again by applying it.
        BuiltIn::Rsi.toggle_visible(&mut workspace.charts[0].indicators);
        assert!(workspace.charts[0].indicators.hide_rsi);
        assert_eq!(
            workspace
                .apply_indicator(btc, BuiltIn::Rsi, None)
                .unwrap()
                .0,
            IndicatorOutcome::Updated
        );
        assert!(!workspace.charts[0].indicators.hide_rsi);

        assert_eq!(
            workspace.apply_indicator(999, BuiltIn::Rsi, None),
            Err(IndicatorError::NoSuchChart)
        );
    }

    #[test]
    fn volume_is_refused_on_line_charts_but_others_are_not() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(6);
        let market = workspace
            .add_chart_for(
                &InstrumentId::new(DataSource::Polymarket, "some-market"),
                &ctx,
            )
            .chart_id;
        assert_eq!(
            workspace.apply_indicator(market, BuiltIn::Volume, None),
            Err(IndicatorError::UnsupportedOnLineChart)
        );
        assert!(
            workspace
                .apply_indicator(market, BuiltIn::Ema, None)
                .is_ok()
        );
    }

    #[test]
    fn indicators_stay_on_their_own_chart_and_persist() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(8);
        let a = workspace
            .add_chart_for(&InstrumentId::new(DataSource::Binance, "BTCUSDT"), &ctx)
            .chart_id;
        let b = workspace
            .add_chart_for(&InstrumentId::new(DataSource::Binance, "ETHUSDT"), &ctx)
            .chart_id;
        workspace
            .apply_indicator(a, BuiltIn::Bollinger, None)
            .unwrap();
        let persisted = workspace.persisted();
        let of = |id| {
            persisted
                .charts
                .iter()
                .find(|c| c.id == id)
                .unwrap()
                .indicators
        };
        assert!(of(a).show_bollinger);
        assert!(!of(b).show_bollinger);
    }

    #[test]
    fn added_charts_persist_and_restore() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(3);
        let outcome =
            workspace.add_chart_for(&InstrumentId::new(DataSource::Binance, "SOLUSDT"), &ctx);
        let persisted = workspace.persisted();
        assert_eq!(persisted.charts.len(), 1);
        assert_eq!(persisted.charts[0].id, outcome.chart_id);
        assert_eq!(persisted.charts[0].symbol, "SOLUSDT");
    }

    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn apply_to_all_restyles_the_other_charts_of_this_workspace_only() {
        use crate::model::canvas::BackgroundMode;
        use crate::model::chart_template::ApplyScope;
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(3);
        let mut control = empty_workspace(4);
        let ids: Vec<u64> = ["BTCUSDT", "ETHUSDT", "SOLUSDT"]
            .iter()
            .map(|symbol| {
                workspace
                    .add_chart_for(&InstrumentId::new(DataSource::Binance, *symbol), &ctx)
                    .chart_id
            })
            .collect();
        let other = control
            .add_chart_for(&InstrumentId::new(DataSource::Binance, "XRPUSDT"), &ctx)
            .chart_id;
        let untouched = control.charts[0].chart.appearance();
        // The second chart carries a script and a Kagi type of its own.
        workspace.charts[1].chart.add_pine_instance(
            None,
            "RSI",
            "//@version=6\nindicator(\"x\")\nplot(close)\n",
        );
        workspace.charts[1]
            .chart
            .set_chart_type(crate::model::ChartType::Kagi);
        workspace.mark_clean();

        let mut look = crate::model::ChartAppearance::default();
        look.bullish = [9, 8, 7];
        look.canvas.background.mode = BackgroundMode::Gradient;
        look.canvas.margins.right = 21;
        look.style.chart_type = crate::model::ChartType::Renko;
        let message = workspace.apply_appearance_to_all(ids[0], &look, ApplyScope::ColorsAndCanvas);
        assert_eq!(message, "Applied the appearance to 2 charts.");
        for chart in &workspace.charts[1..] {
            let appearance = chart.chart.appearance();
            assert_eq!(appearance.bullish, [9, 8, 7]);
            assert_eq!(appearance.canvas, look.canvas);
        }
        // Colours and canvas only: each chart keeps the type it had.
        assert_eq!(
            workspace.charts[1].chart.appearance().style.chart_type,
            crate::model::ChartType::Kagi
        );
        assert_eq!(
            workspace.charts[2].chart.appearance().style.chart_type,
            crate::model::ChartType::Candles
        );
        assert_eq!(
            workspace
                .charts
                .iter()
                .map(|c| c.symbol.as_str())
                .collect::<Vec<_>>(),
            ["BTCUSDT", "ETHUSDT", "SOLUSDT"]
        );
        assert_eq!(workspace.charts[1].chart.pine_instances().len(), 1);
        assert!(workspace.is_dirty(), "the layout notices the new look");
        // The source chart and the other workspace are untouched.
        assert_eq!(
            workspace.charts[0].chart.appearance().bullish,
            crate::model::ChartAppearance::default().bullish
        );
        assert_eq!(control.charts[0].chart.appearance(), untouched);
        assert_eq!(control.charts[0].id, other);

        // With the type too, charts whose market can draw it take it.
        workspace.apply_appearance_to_all(ids[0], &look, ApplyScope::AppearanceAndCanvas);
        assert_eq!(
            workspace.charts[2].chart.appearance().style.chart_type,
            crate::model::ChartType::Renko
        );
        let alone = control.apply_appearance_to_all(other, &look, ApplyScope::AppearanceAndCanvas);
        assert_eq!(alone, "There are no other charts in this workspace.");
    }

    #[test]
    fn a_new_tab_has_no_chart_and_is_clean() {
        let workspace = empty_workspace(3);
        assert!(workspace.charts.is_empty());
        assert!(workspace.columns.is_empty());
        assert_eq!(workspace.focused, None);
        assert!(!workspace.is_dirty(), "an empty tab has nothing to save");
        assert_eq!(workspace.title(), "New tab");
    }

    #[test]
    fn an_empty_tab_survives_a_restart_empty() {
        let workspace = empty_workspace(4);
        let encoded = serde_json::to_string(&workspace.persisted()).expect("serialize");
        let decoded: PersistedWorkspace = serde_json::from_str(&encoded).expect("deserialize");
        assert!(decoded.charts.is_empty());
        let restored = ChartWorkspace::restored(
            decoded,
            workspace.watchlist.clone(),
            workspace.quote_book.clone(),
            workspace.quote_coordinator.clone(),
            workspace.store.clone(),
            &egui::Context::default(),
        );
        assert_eq!(restored.id(), 4);
        assert!(restored.charts.is_empty());
        assert_eq!(restored.focused, None);
    }

    #[test]
    fn persisted_workspace_round_trips() {
        let saved = PersistedWorkspace {
            id: 7,
            timeframe: Timeframe::Hour,
            watchlist_visible: false,
            charts: vec![
                PersistedChart {
                    id: 1,
                    symbol: "BTCUSDT".into(),
                    source: DataSource::Binance,
                    log_scale: false,
                    y_zoom: 1.0,
                    indicators: IndicatorConfig::default(),
                    appearance: crate::model::ChartAppearance::default(),
                    drawings: Vec::new(),
                    pine_instances: Vec::new(),
                    candle_count: 0,
                    viewport: Default::default(),
                    title: None,
                    pm_series: Vec::new(),
                    image_url: None,
                    card: None,
                },
                PersistedChart {
                    id: 2,
                    symbol: "ETHUSDT".into(),
                    source: DataSource::Coinbase,
                    log_scale: true,
                    y_zoom: 1.5,
                    indicators: IndicatorConfig {
                        show_rsi: true,
                        ..IndicatorConfig::default()
                    },
                    appearance: crate::model::ChartAppearance {
                        bullish: [12, 34, 56],
                        bearish: [210, 87, 65],
                        axis_decimals: Some(3),
                        axis_small_prices: crate::model::SmallPrices::Decimals,
                        style: crate::model::ChartStyle {
                            chart_type: crate::model::ChartType::Renko,
                            ..crate::model::ChartStyle::default()
                        },
                        canvas: crate::model::ChartCanvas {
                            margins: crate::model::canvas::ChartMargins {
                                top: 20,
                                bottom: 15,
                                right: 7,
                            },
                            ..crate::model::ChartCanvas::default()
                        },
                    },
                    drawings: Vec::new(),
                    pine_instances: vec![crate::model::pine_instance::PineInstance::new(
                        1,
                        Some("s".into()),
                        "RSI",
                        "plot(close)",
                    )],
                    candle_count: 0,
                    viewport: Default::default(),
                    title: None,
                    pm_series: Vec::new(),
                    image_url: None,
                    card: None,
                },
            ],
            columns: vec![ChartColumn::new(10, vec![1]), ChartColumn::new(11, vec![2])],
            focused: Some(2),
            sync: ChartSyncSettings {
                horizontal_position: true,
                horizontal_zoom: false,
            },
        };
        let encoded = serde_json::to_string(&saved).expect("serialize workspace");
        let decoded: PersistedWorkspace =
            serde_json::from_str(&encoded).expect("deserialize workspace");
        assert_eq!(decoded, saved);
    }

    #[test]
    fn linked_charts_are_saved_with_the_layout_and_restored() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(5);
        for symbol in ["BTCUSDT", "ETHUSDT"] {
            workspace.add_chart_for(&InstrumentId::new(DataSource::Binance, symbol), &ctx);
        }
        let both = ChartSyncSettings {
            horizontal_position: true,
            horizontal_zoom: true,
        };
        assert_eq!(workspace.sync, both, "new tabs start linked both ways");
        workspace.mark_clean();
        workspace.set_sync(both);
        assert!(!workspace.is_dirty(), "keeping the default is no change");
        workspace.set_sync(ChartSyncSettings::UNLINKED);
        assert!(workspace.is_dirty(), "unlinking is a change to the layout");

        let decoded: PersistedWorkspace =
            serde_json::from_str(&serde_json::to_string(&workspace.persisted()).expect("encode"))
                .expect("decode");
        let restored = ChartWorkspace::restored(
            decoded,
            workspace.watchlist.clone(),
            workspace.quote_book.clone(),
            workspace.quote_coordinator.clone(),
            workspace.store.clone(),
            &ctx,
        );
        assert_eq!(
            restored.sync,
            ChartSyncSettings::UNLINKED,
            "the choice to unlink survives a restart"
        );

        let one_off = ChartSyncSettings {
            horizontal_zoom: false,
            ..both
        };
        workspace.set_sync(one_off);
        assert_eq!(
            workspace.persisted().sync,
            one_off,
            "a change persists as well"
        );
    }

    #[test]
    fn a_layout_row_from_before_linked_charts_opens_linked() {
        let json = r#"{"id":4,"timeframe":"Hour","watchlist_visible":true,"charts":[],"columns":[],"focused":null}"#;
        let saved: PersistedWorkspace = serde_json::from_str(json).expect("old row");
        assert!(saved.sync.horizontal_position && saved.sync.horizontal_zoom);
        let view: SavedView = serde_json::from_str(
            r#"{"id":1,"name":"BTC","timeframe":"Hour","created_at_ms":1000,"charts":[]}"#,
        )
        .expect("old layout");
        assert_eq!(view.sync, ChartSyncSettings::default());
        assert!(view.sync.is_linked());
    }

    #[test]
    fn toggling_a_link_never_moves_a_chart() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(6);
        for symbol in ["BTCUSDT", "ETHUSDT"] {
            workspace.add_chart_for(&InstrumentId::new(DataSource::Binance, symbol), &ctx);
        }
        for chart in &mut workspace.charts {
            chart
                .chart
                .restore_viewport(crate::model::viewport::SavedViewport {
                    right_offset: 12.0,
                    price_range: None,
                });
        }
        let before = workspace.persisted().charts;
        workspace.set_sync(ChartSyncSettings::UNLINKED);
        workspace.set_sync(ChartSyncSettings {
            horizontal_position: true,
            horizontal_zoom: false,
        });
        workspace.set_sync(ChartSyncSettings::default());
        let after = workspace.persisted().charts;
        for (before, after) in before.iter().zip(&after) {
            assert_eq!(before.viewport, after.viewport);
            assert_eq!(before.candle_count, after.candle_count);
        }
    }

    #[test]
    fn a_multi_choice_fetch_reads_candles_from_the_first_choice_with_history() {
        let bar = |time: i64| Kline {
            open_time: time,
            close_time: time + 1,
            open: 0.5,
            high: 0.5,
            low: 0.5,
            close: 0.5,
            volume: 0.0,
        };
        let fetched = Fetched::from_choices(vec![
            ("a".into(), Vec::new()),
            ("b".into(), vec![bar(1), bar(2)]),
            ("c".into(), vec![bar(3)]),
        ])
        .expect("some history");
        assert_eq!(fetched.candles.len(), 2);
        assert_eq!(fetched.series.len(), 3);
        let none = Fetched::from_choices(vec![("a".into(), Vec::new())]);
        assert!(none.is_err());
    }

    #[test]
    fn a_prediction_chart_keeps_its_title_and_choices_through_a_save() {
        let mut saved = PersistedChart {
            id: 1,
            symbol: "111".into(),
            source: DataSource::Polymarket,
            log_scale: false,
            y_zoom: 1.0,
            indicators: IndicatorConfig::default(),
            appearance: crate::model::ChartAppearance::default(),
            drawings: Vec::new(),
            pine_instances: Vec::new(),
            candle_count: 0,
            viewport: Default::default(),
            title: Some("Fed decision".into()),
            card: None,
            pm_series: vec![
                PmSeries {
                    token_id: "111".into(),
                    label: "Cut".into(),
                },
                PmSeries {
                    token_id: "222".into(),
                    label: "Hold".into(),
                },
            ],
            image_url: Some("https://img/fed.png".into()),
        };
        let encoded = serde_json::to_value(&saved).expect("serialize");
        assert_eq!(encoded["pm_series"][0]["tokenID"], "111");
        let decoded: PersistedChart = serde_json::from_value(encoded.clone()).expect("round trip");
        assert_eq!(decoded.title.as_deref(), Some("Fed decision"));
        assert_eq!(decoded.pm_series, saved.pm_series);
        // A row from before prediction titles decodes with neither.
        let mut old = encoded;
        old.as_object_mut().unwrap().remove("title");
        old.as_object_mut().unwrap().remove("pm_series");
        old.as_object_mut().unwrap().remove("image_url");
        saved.title = None;
        saved.pm_series.clear();
        saved.image_url = None;
        let decoded: PersistedChart = serde_json::from_value(old).expect("old row");
        assert_eq!(decoded.title, None);
        assert!(decoded.pm_series.is_empty());
        assert_eq!(decoded.image_url, None);
    }

    #[test]
    fn coinmarketcap_index_cards_are_not_paper_markets() {
        assert!(!ChartWorkspace::supports_paper_trading(
            DataSource::CoinMarketCap
        ));
        assert!(ChartWorkspace::supports_paper_trading(DataSource::Binance));
        assert!(ChartWorkspace::supports_paper_trading(DataSource::Alpaca));
        assert!(ChartWorkspace::supports_paper_trading(
            DataSource::Polymarket
        ));
    }

    #[test]
    fn paper_ticket_reference_follows_the_replay_slice() {
        let candles = vec![
            candle(1_000, 10.0),
            candle(2_000, 20.0),
            candle(3_000, 30.0),
        ];
        assert_eq!(
            displayed_candles(&candles, None).last().unwrap().close,
            30.0
        );
        assert_eq!(
            displayed_candles(&candles, Some(2_500))
                .last()
                .unwrap()
                .close,
            20.0
        );
        assert!(displayed_candles(&candles, Some(500)).is_empty());
    }

    #[test]
    fn the_bell_starts_an_alert_on_the_charts_market() {
        let btc = ChartWorkspace::alert_asset_for("BTCUSDT", DataSource::Binance);
        assert_eq!(btc.symbol, "BTC");
        assert_eq!(btc.instrument.symbol, "BTCUSDT");
        assert_eq!(btc.key(), "BINANCE:BTCUSDT");
        let coinbase = ChartWorkspace::alert_asset_for("ETH-USD", DataSource::Coinbase);
        assert_eq!(coinbase.symbol, "ETH");
        assert!(ui::supports_alerts(DataSource::Binance));
        assert!(ui::supports_alerts(DataSource::Alpaca));
        assert!(!ui::supports_alerts(DataSource::CoinMarketCap));
        assert!(!ui::supports_alerts(DataSource::Polymarket));
        assert!(!ui::supports_alerts(DataSource::Kalshi));
    }

    fn snapshot(log_scale: bool) -> LayoutSnapshot {
        LayoutSnapshot {
            timeframe: Timeframe::Hour,
            configs: vec![crate::model::ChartConfig {
                id: 1,
                symbol: "BTCUSDT".into(),
                source: DataSource::Binance,
                log_scale,
                appearance: crate::model::ChartAppearance::default(),
                card: None,
            }],
            columns: vec![vec![1]],
            sync: Default::default(),
        }
    }

    #[test]
    fn autosave_waits_for_the_layout_to_sit_still_then_saves_once() {
        let mut state = AutosaveState::default();
        let start = Instant::now();
        assert_eq!(
            state.step(true, true, || snapshot(false), start),
            AutosaveStep::Wait(AUTOSAVE_DELAY)
        );
        // An edit one second in restarts the countdown.
        let edited = start + Duration::from_secs(1);
        assert_eq!(
            state.step(true, true, || snapshot(true), edited),
            AutosaveStep::Wait(AUTOSAVE_DELAY)
        );
        // Unchanged since, the layout is due once the delay has passed.
        let due = edited + AUTOSAVE_DELAY;
        assert_eq!(
            state.step(
                true,
                true,
                || snapshot(true),
                due - Duration::from_millis(1)
            ),
            AutosaveStep::Wait(Duration::from_millis(1))
        );
        assert_eq!(
            state.step(true, true, || snapshot(true), due),
            AutosaveStep::Save
        );
        // Tried once: a failed write is not retried every frame, only after the next edit.
        assert_eq!(
            state.step(true, true, || snapshot(true), due + Duration::from_secs(5)),
            AutosaveStep::Nothing
        );
        assert_eq!(
            state.step(true, true, || snapshot(false), due + Duration::from_secs(6)),
            AutosaveStep::Wait(AUTOSAVE_DELAY)
        );
    }

    #[test]
    fn autosave_stays_idle_for_clean_layouts_and_layouts_with_it_off() {
        let mut state = AutosaveState::default();
        let now = Instant::now();
        assert_eq!(
            state.step(false, true, || snapshot(true), now),
            AutosaveStep::Nothing
        );
        assert_eq!(
            state.step(true, false, || snapshot(true), now),
            AutosaveStep::Nothing
        );
        assert!(state.armed.is_none() && state.due.is_none());
        // Going clean (saved by hand) drops a pending countdown.
        state.step(true, true, || snapshot(true), now);
        assert!(state.due.is_some());
        state.step(false, true, || snapshot(true), now);
        assert!(state.due.is_none());
    }

    fn rectangle(id: u64, left: i64) -> Drawing {
        Drawing::build(
            Tool::Rectangle,
            id,
            &[
                crate::model::Anchor {
                    time_ms: left,
                    price: 1.0,
                },
                crate::model::Anchor {
                    time_ms: left + 10,
                    price: 2.0,
                },
            ],
            Tool::Rectangle.default_style(),
        )
        .expect("rectangle")
    }

    fn chart_with_drawings(workspace: &mut ChartWorkspace, symbol: &str, ids: &[u64]) -> u64 {
        let ctx = egui::Context::default();
        let chart_id = workspace
            .add_chart_for(&InstrumentId::new(DataSource::Binance, symbol), &ctx)
            .chart_id;
        let chart = workspace
            .charts
            .iter_mut()
            .find(|chart| chart.id == chart_id)
            .expect("chart");
        chart.drawings = ids
            .iter()
            .map(|id| rectangle(*id, *id as i64 * 100))
            .collect();
        chart_id
    }

    fn drawing_ids(workspace: &ChartWorkspace, chart_id: u64) -> Vec<u64> {
        workspace
            .charts
            .iter()
            .find(|chart| chart.id == chart_id)
            .expect("chart")
            .drawings
            .iter()
            .map(Drawing::id)
            .collect()
    }

    #[test]
    fn drawings_planned_for_mcp_are_one_undo_step_and_keep_their_style_on_save() {
        use crate::mcp::drawings::{AnchorSpec, McpDrawingTool, NewDrawing, PriceField, PriceSpec};
        use crate::model::chart_type::LineDash;
        let mut workspace = empty_workspace(9);
        assert!(workspace.chart_data(404).is_none());
        let chart_id = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        let minute = 60_000;
        workspace
            .charts
            .iter_mut()
            .find(|chart| chart.id == chart_id)
            .unwrap()
            .candles = (0..20)
            .map(|index| candle(1_000_000 + index * minute, 100.0 + index as f64))
            .collect();
        let data = workspace.chart_data(chart_id).expect("market chart");
        assert_eq!(data.candles.len(), 20);
        // The counter lags the stored drawing (the fixture bypassed it); minting still avoids it.
        let first_id = crate::mcp::chart_data::next_free_id(data.drawings, data.next_drawing_id);
        assert_eq!(first_id, 2);
        let anchor = |index: i64, price: PriceSpec| AnchorSpec {
            time_ms: 1_000_000 + index * minute + 5,
            price,
        };
        let mut line = NewDrawing {
            tool: McpDrawingTool::TrendLine,
            a: anchor(2, PriceSpec::Field(PriceField::Low)),
            b: anchor(15, PriceSpec::Field(PriceField::High)),
            patch: Default::default(),
        };
        line.patch.dash = Some(LineDash::Dotted);
        line.patch.extend_right = Some(true);
        line.patch.color = Some([255, 193, 7, 255]);
        let zone = NewDrawing {
            tool: McpDrawingTool::Rectangle,
            a: anchor(4, PriceSpec::Value(103.0)),
            b: anchor(9, PriceSpec::Value(108.0)),
            patch: Default::default(),
        };
        let planned =
            crate::mcp::chart_data::plan_add(data.drawings, data.candles, first_id, &[line, zone])
                .unwrap();
        let commands: Vec<DrawingCommand> =
            planned.iter().cloned().map(DrawingCommand::Add).collect();
        workspace.apply_planned_drawings(chart_id, commands, first_id + 2);

        assert_eq!(drawing_ids(&workspace, chart_id), [1, 2, 3]);
        assert_eq!(workspace.chart_data(chart_id).unwrap().next_drawing_id, 4);
        // Times snapped to the bars' opens, prices read from the bars.
        let anchors = planned[0].anchors();
        assert_eq!(anchors[0].time_ms, 1_000_000 + 2 * minute);
        assert_eq!(anchors[1].price, 115.0);

        // It is saved with the session, style and all, and reads back the same.
        let saved = workspace.persisted();
        let json = serde_json::to_string(&saved).unwrap();
        let reread: PersistedWorkspace = serde_json::from_str(&json).unwrap();
        let restored = reread.charts[0].drawings.clone();
        assert_eq!(restored.len(), 3);
        assert_eq!(restored[1], planned[0]);
        assert_eq!(restored[1].style().unwrap().stroke.dash, LineDash::Dotted);
        assert!(restored[1].style().unwrap().extend_right);

        // One undo takes both away, one redo brings both back.
        workspace.undo();
        assert_eq!(drawing_ids(&workspace, chart_id), [1]);
        workspace.redo();
        assert_eq!(drawing_ids(&workspace, chart_id), [1, 2, 3]);

        // A chart that is not there changes nothing.
        workspace.apply_planned_drawings(404, vec![DrawingCommand::Remove(1)], 50);
        assert_eq!(drawing_ids(&workspace, chart_id), [1, 2, 3]);
    }

    /// What another Mac's edit looks like once iCloud Sync has stored it: the saved layout's
    /// drawings for one chart replaced.
    fn store_remote_drawings(workspace: &ChartWorkspace, chart: u64, drawings: Vec<Drawing>) {
        let mut views = workspace.load_saved_views();
        for view in &mut views {
            for saved in view.charts.iter_mut().filter(|saved| saved.id == chart) {
                saved.drawings = drawings.clone();
            }
        }
        workspace
            .store
            .lock()
            .unwrap()
            .replace_saved_views(&views)
            .unwrap();
    }

    fn live_ids(workspace: &ChartWorkspace, chart: u64) -> Vec<u64> {
        drawing_ids(workspace, chart)
    }

    #[test]
    fn merging_remote_drawings_keeps_each_sides_changes_and_local_wins_a_tie() {
        let d = |id: u64, at: i64| rectangle(id, at);
        let base = [d(1, 100), d(2, 200), d(3, 300)];
        // Local moved 1 and added 4. Remote deleted 2, moved 3 and added 5.
        let local = [d(1, 111), d(2, 200), d(3, 300), d(4, 400)];
        let remote = [d(1, 100), d(3, 333), d(5, 500)];
        let merged = merge_remote_drawings(&base, &local, &remote);
        assert_eq!(
            merged,
            [d(1, 111), d(3, 333), d(4, 400), d(5, 500)],
            "local move kept, remote delete and move taken, both additions kept"
        );
        // Both changed the same drawing: this tab's version wins.
        let merged = merge_remote_drawings(&[d(1, 100)], &[d(1, 111)], &[d(1, 122)]);
        assert_eq!(merged, [d(1, 111)]);
        // A drawing deleted here stays deleted even if the other Mac moved it.
        let merged = merge_remote_drawings(&[d(1, 100)], &[], &[d(1, 122)]);
        assert!(merged.is_empty());
    }

    /// A drawing this version cannot read, as an older build holds a newer build's tool.
    fn unreadable(id: u64) -> Drawing {
        Drawing::Unreadable(serde_json::json!({
            "Shape": {"id": id, "geometry": {"hologram": {"at": id}}}
        }))
    }

    #[test]
    fn unreadable_drawings_merge_by_their_stored_id_not_as_one_drawing() {
        let d = |id: u64, at: i64| rectangle(id, at);
        // The other Mac drew a tool this build cannot read, next to one already held.
        let merged = merge_remote_drawings(
            &[d(1, 100), unreadable(50)],
            &[d(1, 100), unreadable(50)],
            &[d(1, 100), unreadable(50), unreadable(51)],
        );
        assert_eq!(merged, [d(1, 100), unreadable(50), unreadable(51)]);
        // The other Mac deleted one of two: the other must not be duplicated or lost.
        let merged = merge_remote_drawings(
            &[unreadable(50), unreadable(51)],
            &[unreadable(50), unreadable(51)],
            &[unreadable(51)],
        );
        assert_eq!(merged, [unreadable(51)]);
        // Edited here, untouched there: this tab's version stays.
        let merged = merge_remote_drawings(
            &[d(1, 100), unreadable(50)],
            &[d(1, 111), unreadable(50)],
            &[d(1, 100), unreadable(50)],
        );
        assert_eq!(merged, [d(1, 111), unreadable(50)]);
    }

    #[test]
    fn unreadable_drawings_arriving_in_an_open_layout_survive_the_next_autosave() {
        let mut workspace = empty_workspace(8);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        assert!(workspace.save_named("Nico".to_owned(), Some(true)));
        store_remote_drawings(
            &workspace,
            chart,
            vec![rectangle(1, 100), unreadable(50), unreadable(51)],
        );
        workspace.adopt_remote_layout();
        store_remote_drawings(
            &workspace,
            chart,
            vec![
                rectangle(1, 100),
                unreadable(50),
                unreadable(51),
                unreadable(52),
            ],
        );
        workspace.adopt_remote_layout();

        let stored_ids = |drawings: &[Drawing]| -> Vec<u64> {
            drawings.iter().map(Drawing::stored_id).collect()
        };
        let live = &workspace
            .charts
            .iter()
            .find(|c| c.id == chart)
            .unwrap()
            .drawings;
        assert_eq!(stored_ids(live), [1, 50, 51, 52]);
        // Whatever the next write is, nothing the other Mac drew is dropped.
        workspace.flush_autosave();
        let saved = workspace.load_saved_views();
        assert_eq!(stored_ids(&saved[0].charts[0].drawings), [1, 50, 51, 52]);
        assert!(
            workspace.chart_data(chart).unwrap().next_drawing_id > 52,
            "new drawings never reuse an id a newer build stored"
        );
    }

    #[test]
    fn a_drawing_another_mac_made_appears_in_the_open_autosave_layout_without_a_write_back() {
        let mut workspace = empty_workspace(8);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        assert!(workspace.save_named("Nico".to_owned(), Some(true)));
        assert!(!workspace.drawings_dirty());

        store_remote_drawings(
            &workspace,
            chart,
            vec![rectangle(1, 100), rectangle(7, 700)],
        );
        workspace.adopt_remote_layout();

        assert_eq!(live_ids(&workspace, chart), [1, 7]);
        assert!(
            !workspace.drawings_dirty(),
            "it is already stored: nothing to save"
        );
        // The old behaviour: the next autosave wrote the tab's stale list over the other Mac's.
        assert!(workspace.flush_autosave());
        let saved = workspace.load_saved_views();
        assert_eq!(saved[0].charts[0].drawings.len(), 2);
        // The counter moved past the arrived drawing.
        assert!(workspace.chart_data(chart).unwrap().next_drawing_id > 7);
    }

    #[test]
    fn unsaved_local_drawings_survive_an_incoming_layout_and_still_get_saved() {
        let mut workspace = empty_workspace(8);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1, 2]);
        assert!(workspace.save_named("Nico".to_owned(), Some(true)));
        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Add(rectangle(3, 300))]);
        assert!(workspace.drawings_dirty());

        // Meanwhile the other Mac deleted 2 and drew 7.
        store_remote_drawings(
            &workspace,
            chart,
            vec![rectangle(1, 100), rectangle(7, 700)],
        );
        workspace.adopt_remote_layout();

        assert_eq!(live_ids(&workspace, chart), [1, 3, 7]);
        assert!(
            workspace.drawings_dirty(),
            "the drawing made here is still unsaved"
        );
        assert!(workspace.flush_autosave());
        let ids: Vec<u64> = workspace.load_saved_views()[0].charts[0]
            .drawings
            .iter()
            .map(Drawing::id)
            .collect();
        assert_eq!(ids, [1, 3, 7]);
    }

    #[test]
    fn a_layout_nobody_changed_is_left_alone() {
        let mut workspace = empty_workspace(8);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1, 2]);
        assert!(workspace.save_named("Nico".to_owned(), Some(true)));
        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Remove(1)]);
        workspace.adopt_remote_layout();
        assert_eq!(
            live_ids(&workspace, chart),
            [2],
            "an unsynced local edit stays"
        );
        assert!(workspace.drawings_dirty());
    }

    #[test]
    fn drawing_edits_mark_drawings_dirty_but_not_the_layout_fingerprint() {
        let mut workspace = empty_workspace(8);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        workspace.mark_clean();
        assert!(!workspace.drawings_dirty());

        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Add(rectangle(2, 500))]);
        assert!(workspace.drawings_dirty());
        assert!(
            !workspace.is_dirty(),
            "a drawing never marks the layout as edited"
        );
        workspace.mark_clean();
        assert!(!workspace.drawings_dirty(), "saving takes them in");

        workspace.undo();
        assert!(workspace.drawings_dirty(), "undo changes drawings too");
        workspace.mark_clean();
        workspace.redo();
        assert!(workspace.drawings_dirty());
    }

    #[test]
    fn a_command_that_changes_nothing_leaves_drawings_clean() {
        let mut workspace = empty_workspace(8);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        workspace.mark_clean();
        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Remove(999)]);
        assert!(!workspace.drawings_dirty());
        workspace.apply_drawing_commands(chart, Vec::new());
        assert!(!workspace.drawings_dirty());
    }

    #[test]
    fn an_autosave_layout_writes_a_drawing_only_edit_and_a_manual_one_does_not() {
        let mut workspace = empty_workspace(8);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        assert!(workspace.save_named("Auto".to_owned(), Some(true)));
        assert!(workspace.autosave_enabled());
        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Add(rectangle(1, 100))]);
        assert!(workspace.drawings_dirty());

        assert!(workspace.flush_autosave());
        let saved = workspace.load_saved_views();
        assert_eq!(saved.len(), 1);
        assert_eq!(
            saved[0].charts[0].drawings.len(),
            1,
            "the drawing reached the saved layout"
        );
        assert!(!workspace.drawings_dirty());

        // A layout with Autosave off keeps today's behaviour: a drawing alone writes nothing.
        let mut manual = empty_workspace(9);
        let chart = chart_with_drawings(&mut manual, "ETHUSDT", &[]);
        assert!(manual.save_named("Manual".to_owned(), Some(false)));
        manual.apply_drawing_commands(chart, vec![DrawingCommand::Add(rectangle(1, 100))]);
        assert!(manual.flush_autosave());
        assert!(manual.load_saved_views()[0].charts[0].drawings.is_empty());
    }

    #[test]
    fn one_batch_of_drawing_commands_is_one_undo_step_and_redo_replays_it() {
        let mut workspace = empty_workspace(8);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        // Draw, move (two commands at once), delete: three steps.
        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Add(rectangle(2, 500))]);
        workspace.apply_drawing_commands(
            chart,
            vec![
                DrawingCommand::Replace {
                    id: 1,
                    drawing: rectangle(1, 900),
                },
                DrawingCommand::Replace {
                    id: 2,
                    drawing: rectangle(2, 950),
                },
            ],
        );
        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Remove(1)]);
        assert_eq!(drawing_ids(&workspace, chart), vec![2]);

        workspace.undo();
        assert_eq!(drawing_ids(&workspace, chart), vec![1, 2]);
        assert_eq!(
            workspace.charts[0].drawings[0].anchors()[0].time_ms,
            900,
            "the move came back as one step"
        );
        workspace.undo();
        assert_eq!(workspace.charts[0].drawings[0].anchors()[0].time_ms, 100);
        workspace.undo();
        assert_eq!(drawing_ids(&workspace, chart), vec![1]);
        workspace.undo();
        assert_eq!(
            drawing_ids(&workspace, chart),
            vec![1],
            "nothing earlier to undo"
        );

        for _ in 0..3 {
            workspace.redo();
        }
        assert_eq!(drawing_ids(&workspace, chart), vec![2]);
        // A new edit clears what could be redone.
        workspace.undo();
        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Add(rectangle(3, 0))]);
        workspace.redo();
        assert_eq!(drawing_ids(&workspace, chart), vec![1, 2, 3]);
    }

    #[test]
    fn a_ruler_and_a_batch_that_changes_nothing_leave_no_undo_entry() {
        let mut workspace = empty_workspace(9);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        let ruler = Drawing::Ruler {
            id: 50,
            a: crate::model::Anchor {
                time_ms: 0,
                price: 1.0,
            },
            b: crate::model::Anchor {
                time_ms: 9,
                price: 2.0,
            },
        };
        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Add(ruler)]);
        workspace.apply_drawing_commands(
            chart,
            vec![DrawingCommand::Replace {
                id: 1,
                drawing: rectangle(1, 100),
            }],
        );
        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Remove(404)]);
        assert!(workspace.undo.is_empty(), "no entries");
        assert_eq!(workspace.charts[0].drawings.len(), 2, "the ruler is there");
        // A persisted chart never carries it.
        assert!(
            workspace.persisted().charts[0]
                .drawings
                .iter()
                .all(|d| !d.is_ruler())
        );
    }

    #[test]
    fn a_burst_of_restyles_is_one_undo_step() {
        let mut workspace = empty_workspace(10);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        let restyle = |width: f32| {
            let mut drawing = rectangle(1, 100);
            let mut style = *drawing.style().expect("style");
            style.stroke.width = width;
            drawing.set_style(style);
            DrawingCommand::Style { id: 1, drawing }
        };
        // Dragging a colour picker sends one restyle per frame.
        for width in [3.0, 4.0, 5.0, 6.0] {
            workspace.apply_drawing_commands(chart, vec![restyle(width)]);
        }
        assert_eq!(workspace.undo.len(), 1);
        assert_eq!(
            workspace.charts[0].drawings[0]
                .style()
                .unwrap()
                .stroke
                .width,
            6.0
        );
        workspace.undo();
        assert_eq!(
            workspace.charts[0].drawings[0]
                .style()
                .unwrap()
                .stroke
                .width,
            Tool::Rectangle.default_style().stroke.width,
            "back to how it was before the burst"
        );
        // After an undo the next restyle starts a new step.
        workspace.apply_drawing_commands(chart, vec![restyle(8.0)]);
        workspace.apply_drawing_commands(chart, vec![restyle(9.0)]);
        assert_eq!(workspace.undo.len(), 1);
        // Another edit in between ends the burst.
        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Add(rectangle(2, 300))]);
        workspace.apply_drawing_commands(chart, vec![restyle(10.0)]);
        assert_eq!(workspace.undo.len(), 3);
    }

    #[test]
    fn charts_keep_their_own_drawings() {
        let mut workspace = empty_workspace(11);
        let first = chart_with_drawings(&mut workspace, "BTCUSDT", &[1, 2]);
        let second = chart_with_drawings(&mut workspace, "ETHUSDT", &[7]);
        workspace.apply_drawing_commands(first, vec![DrawingCommand::Remove(1)]);
        assert_eq!(drawing_ids(&workspace, first), vec![2]);
        assert_eq!(drawing_ids(&workspace, second), vec![7]);
        // Undo goes to the chart the edit was made on.
        workspace.apply_drawing_commands(second, vec![DrawingCommand::Add(rectangle(8, 0))]);
        workspace.undo();
        assert_eq!(drawing_ids(&workspace, second), vec![7]);
        assert_eq!(drawing_ids(&workspace, first), vec![2]);
        workspace.undo();
        assert_eq!(drawing_ids(&workspace, first), vec![1, 2]);
    }

    #[test]
    fn lock_all_and_unlock_all_act_on_every_chart_and_report_their_state() {
        let mut workspace = empty_workspace(12);
        let first = chart_with_drawings(&mut workspace, "BTCUSDT", &[1, 2]);
        let second = chart_with_drawings(&mut workspace, "ETHUSDT", &[3]);
        assert_eq!(workspace.drawing_lock_state(), (true, false));
        workspace.set_all_locked(true);
        assert_eq!(workspace.drawing_lock_state(), (true, true));
        assert!(
            workspace
                .charts
                .iter()
                .all(|c| c.drawings.iter().all(Drawing::locked))
        );
        // One undo step per chart, since history is per chart.
        assert_eq!(workspace.undo.len(), 2);
        workspace.undo();
        workspace.undo();
        assert_eq!(workspace.drawing_lock_state(), (true, false));
        workspace.set_all_locked(true);
        workspace.set_all_locked(false);
        assert_eq!(workspace.drawing_lock_state(), (true, false));
        let _ = (first, second);
        // Locking nothing is not an edit.
        let mut empty = empty_workspace(13);
        chart_with_drawings(&mut empty, "BTCUSDT", &[]);
        empty.set_all_locked(true);
        assert!(empty.undo.is_empty());
        assert_eq!(empty.drawing_lock_state(), (false, false));
    }

    #[test]
    fn one_properties_dialog_at_a_time_and_none_for_a_locked_drawing() {
        let mut workspace = empty_workspace(15);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1, 2]);
        let mut locked = rectangle(2, 200);
        locked.set_locked(true);
        workspace.apply_drawing_commands(
            chart,
            vec![DrawingCommand::Replace {
                id: 2,
                drawing: locked,
            }],
        );
        workspace.open_drawing_properties(chart, 2);
        assert!(
            workspace.drawing_props.is_none(),
            "a locked drawing is not edited"
        );
        workspace.open_drawing_properties(chart, 1);
        let opened = workspace
            .drawing_props
            .as_ref()
            .map(|dialog| dialog.draft().id());
        assert_eq!(opened, Some(1));
        // A second request (a Return meant for the dialog) leaves the open one, and its edits.
        workspace
            .drawing_props
            .as_mut()
            .expect("dialog")
            .set_draft_text_for_test("edited");
        workspace.open_drawing_properties(chart, 1);
        assert_eq!(
            workspace
                .drawing_props
                .as_ref()
                .map(|dialog| dialog.draft().text().to_owned()),
            Some("edited".to_owned())
        );
    }

    #[test]
    fn the_session_saves_every_drawing_and_restores_the_same_ones() {
        let mut workspace = empty_workspace(14);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1, 2, 3]);
        let mut locked = rectangle(2, 200);
        locked.set_locked(true);
        workspace.apply_drawing_commands(
            chart,
            vec![DrawingCommand::Replace {
                id: 2,
                drawing: locked,
            }],
        );
        let saved = workspace.persisted();
        let json = serde_json::to_string(&saved).unwrap();
        let reopened: PersistedWorkspace = serde_json::from_str(&json).unwrap();
        assert_eq!(reopened, saved);
        assert_eq!(reopened.charts[0].drawings.len(), 3);
        assert!(reopened.charts[0].drawings[1].locked());
    }
}
