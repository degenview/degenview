use crate::format;
use crate::host::pine_host::KlineSecurityDataProvider;
use crate::model::canvas::{self, PaneSpec, Window};
use crate::model::chart_look::Palette;
use crate::model::chart_type::{ChartStyle, ChartType, DataClass};
use crate::model::paper::PaperOrderSide;
use crate::model::pine_instance::{PineInstance, StoredInput, VisualOrder};
use crate::model::price_scale::{self, PriceScaleMode};
use crate::model::synthetic::{self, BarShape, Construction, SyntheticBar, SyntheticSeries};
use crate::model::viewport::{ChartViewport, PriceRange, SavedViewport};
use crate::model::{
    Anchor, BollingerSeries, ChartAppearance, DataSource, IndicatorConfig, IndicatorSeries, Kline,
    PmSeries, PriceFormat, Timeframe,
};
use crate::pine::execution::ExecutionController;
use crate::pine::logs::{LogBatch, LogHistory};
use crate::pine::program::ChartTheme;
use crate::pine::program::{InputValue, SymbolInfo, VisualOutput};
use crate::pine::style_overrides::{StyleOverrides, StyleRow, style_rows};
use crate::pine::token::{Diagnostic, Limits};
use crate::script_manager::registry::LibraryRegistry;
use crate::ui::alert_lines;
use crate::ui::canvas_render::{self, CanvasStyle};
use crate::ui::chart_buttons::{self, PaneAction};
use crate::ui::chart_indicator_strip;
use crate::ui::drawing_layer::{DrawingFrame, DrawingLayer, Projection, TextMeasure};
use crate::ui::drawing_render::TimeIndex;
use crate::ui::icons::{self, Icon};
use crate::ui::paper_chart_trading::{self, PaperCardState};
use crate::ui::pine_chart_render::{self, CandleStyle, PineLayer};
use crate::ui::pine_geometry::{OverlayExtent, overlay_extent};
use crate::ui::price_text;
use crate::ui::replay_overlay::{self, ReplayChartFrame};
use crate::ui::scale_menu;
use crate::ui::series_render::{self, SeriesFrame};
use crate::ui::snapshot_menu::SnapshotMenu;
use crate::ui::theme;
use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Vec2,
};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, Sender},
};

/// The least the price axis is wide, for a chart whose labels are short.
const PRICE_AXIS_WIDTH: f32 = 44.0;
const HEADER_HEIGHT: f32 = 45.0;
/// A plot shorter than this is not drawn: only the time axis stays.
const MIN_PLOT_HEIGHT: f32 = 16.0;
/// How many screens from the oldest bar held the view asks for an older page.
const NEAR_OLDEST_SCREENS: usize = 2;

fn current_time_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as i64)
}

/// Whether a script asks where the chart is scrolled (`chart.left_visible_bar_time` and its
/// right-hand twin), the only thing about the viewport a run can depend on.
fn reads_visible_range(source: &str) -> bool {
    source.contains("visible_bar_time")
}

fn run_revision(candles: &[Kline], visible_range: Option<(i64, i64)>, now_ms: i64) -> RunRevision {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    candles.len().hash(&mut hasher);
    for candle in candles {
        candle.open_time.hash(&mut hasher);
        candle.close_time.hash(&mut hasher);
        candle.open.to_bits().hash(&mut hasher);
        candle.high.to_bits().hash(&mut hasher);
        candle.low.to_bits().hash(&mut hasher);
        candle.close.to_bits().hash(&mut hasher);
        candle.volume.to_bits().hash(&mut hasher);
    }
    RunRevision {
        candles: hasher.finish(),
        visible_range,
        last_is_closed: candles.last().is_some_and(|bar| bar.close_time <= now_ms),
    }
}

/// The header change uses the same candle window as the plot, including pan and zoom.
fn visible_price_change(
    candles: &[Kline],
    visible_count: usize,
    right_offset: f64,
    reserved: usize,
) -> Option<f64> {
    let window = canvas::visible_window(candles.len(), visible_count, right_offset, reserved);
    let visible = candles.get(window.start..window.end)?;
    let (Some(first), Some(last)) = (visible.first(), visible.last()) else {
        return None;
    };
    if visible.len() < 2 || first.close == 0.0 {
        return None;
    }
    let change = (last.close - first.close) / first.close * 100.0;
    change.is_finite().then_some(change)
}

/// Half the width of a candle body, matching the real candles the Pine overlay recolours.
fn candle_body_half_width(slot_width: f32) -> f32 {
    (slot_width * 0.34).clamp(1.0, 8.0)
}

/// Signals a chart card emits while it is being drawn. The owning workspace
/// applies them after the frame, so rendering never mutates layout mid-pass.
#[derive(Clone, Copy, Debug, Default)]
pub struct ChartCardAction {
    pub focus_requested: bool,
    pub settings_requested: bool,
    /// CoinMarketCap charts have no settings sheet, so removal lives in the header.
    pub remove_requested: bool,
    pub indicator_action: Option<chart_indicator_strip::Action>,
    /// A header SELL / BUY pill was clicked: open the order ticket on that side.
    pub quick_trade: Option<PaperOrderSide>,
    /// The header bell was clicked: start a new price alert for this chart's market.
    pub new_alert: bool,
    pub toggle_favorite: bool,
    pub watchlist_change: Option<(u64, bool)>,
    /// A chart type was picked from the header's selector.
    pub chart_type: Option<ChartType>,
    /// A row of the header's camera menu was picked: take that snapshot of this chart.
    pub snapshot: Option<crate::model::snapshot::SnapshotAction>,
}

pub struct WatchlistMembership {
    pub id: u64,
    pub name: String,
    pub contains_market: bool,
}

/// The shared crosshair a tab draws across all its charts. Stored as a fraction
/// of plot width so the same column lines up on every card.
#[derive(Clone, Copy, Debug)]
pub struct CrosshairInfo {
    pub owner_id: u64,
    pub x_fraction: f64,
    pub price: f64,
}

use body::{BodyArgs, OverlayResults};
pub use context::ContextRequest;
use live_overlays::LiveOverlays;
#[cfg(test)]
use snapshot::SnapshotError;
pub use snapshot::SnapshotInput;

/// What a chart reports back while drawing, plus whether its plot was clicked.
#[derive(Clone, Debug, Default)]
pub struct ChartPointer {
    /// A right-click the chart's menu should answer (a drawing's own menu answers its clicks).
    pub context: Option<ContextRequest>,
    pub clicked: bool,
    pub hover_x_fraction: Option<f64>,
    pub hover_price: Option<f64>,
    /// The time and price under the pointer, through the chart's own projection.
    pub hover_anchor: Option<Anchor>,
    /// The chart's plot area, once allocated.
    pub rect: Option<Rect>,
}

/// The "Chart settings" hover hint, except where a chart has no settings to open.
trait SettingsTip {
    fn settings_tip(self, source: DataSource) -> Self;
}

impl SettingsTip for egui::Response {
    fn settings_tip(self, source: DataSource) -> Self {
        if source == DataSource::CoinMarketCap {
            self
        } else {
            self.on_hover_text("Chart settings")
        }
    }
}

/// `label` cut to `max` characters, with an ellipsis when it was longer.
fn truncated(label: &str, max: usize) -> String {
    if label.chars().count() <= max {
        label.to_owned()
    } else {
        let cut: String = label.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

/// What the warning pill says: rate limits get their own wording, `detailed` appends the error.
fn refresh_warning_text(message: &str, source: DataSource, detailed: bool) -> String {
    if message.contains("429") {
        format!(
            "{} rate limit reached · showing the last result",
            source.display_name()
        )
    } else if detailed {
        format!("Couldn't refresh · showing the last result ({message})")
    } else {
        "Couldn't refresh · showing the last result".to_owned()
    }
}

/// A pill along the bottom of the chart saying the latest refresh failed.
fn refresh_warning(ui: &egui::Ui, rect: Rect, message: &str, source: DataSource) {
    // The detailed wording first, then the short one if the card is too narrow for it.
    let layout = |text: String| {
        ui.painter()
            .layout_no_wrap(text, FontId::proportional(10.5), theme::text_primary())
    };
    let mut galley = layout(refresh_warning_text(message, source, true));
    if galley.size().x + 26.0 > rect.width() - 16.0 {
        galley = layout(refresh_warning_text(message, source, false));
    }
    let size = galley.size() + Vec2::new(26.0, 8.0);
    if size.x > rect.width() - 16.0 {
        return;
    }
    let pill = Rect::from_center_size(
        Pos2::new(rect.center().x, rect.max.y - size.y / 2.0 - 8.0),
        size,
    );
    let painter = ui.painter();
    painter.rect_filled(pill, size.y / 2.0, theme::chrome());
    painter.rect_stroke(
        pill,
        size.y / 2.0,
        Stroke::new(1.0, theme::DOWN.gamma_multiply(0.6)),
        egui::StrokeKind::Inside,
    );
    painter.circle_filled(pill.left_center() + Vec2::new(11.0, 0.0), 3.0, theme::DOWN);
    painter.galley(
        pill.left_center() + Vec2::new(20.0, -galley.size().y / 2.0),
        galley,
        theme::text_primary(),
    );
}

/// Line colors for the choices of a multi-choice prediction market, in the order they were ticked.
const SERIES_PALETTE: [Color32; 7] = [
    Color32::from_rgb(10, 132, 255),
    Color32::from_rgb(255, 159, 10),
    Color32::from_rgb(191, 90, 242),
    Color32::from_rgb(48, 209, 88),
    Color32::from_rgb(255, 69, 58),
    Color32::from_rgb(100, 210, 255),
    Color32::from_rgb(255, 55, 95),
];
/// Room between the labels at the ends of the lines, and the longest one drawn.
const SERIES_LABEL_GAP: f32 = 13.0;
const SERIES_LABEL_CHARS: usize = 24;

/// One choice of a prediction market, drawn as a line over the chart.
pub struct SeriesLine {
    pub label: String,
    pub color: Color32,
    pub candles: Vec<Kline>,
}

impl SeriesLine {
    /// The lines for `choices`, colored by their place among them; a choice with no history is
    /// left out.
    pub fn build(choices: &[PmSeries], histories: Vec<(String, Vec<Kline>)>) -> Vec<Self> {
        let mut histories: HashMap<String, Vec<Kline>> = histories.into_iter().collect();
        choices
            .iter()
            .enumerate()
            .filter_map(|(index, choice)| {
                let candles = histories
                    .remove(&choice.token_id)
                    .filter(|candles| !candles.is_empty())?;
                Some(Self {
                    label: choice.label.clone(),
                    color: SERIES_PALETTE[index % SERIES_PALETTE.len()],
                    candles,
                })
            })
            .collect()
    }
}

pub struct ChartCard {
    appearance: ChartAppearance,
    /// How big the plot area was drawn at the last frame, in points: what a snapshot is the size of.
    body_size: Vec2,
    /// A human-readable title shown instead of an opaque provider id.
    title: Option<String>,
    /// The choices of a multi-choice prediction market; empty for every other chart.
    series: Vec<SeriesLine>,
    /// During replay, the last time the lines may reach.
    series_cutoff: Option<i64>,
    /// What part of the chart is on screen: the one place every pan, zoom and scale gesture
    /// writes and every painter reads.
    viewport: ChartViewport,
    /// The drag in progress, if one began on this chart.
    gesture: Option<navigation::ActiveGesture>,
    /// Whether the view reached the oldest bar the chart holds, so older ones are wanted.
    near_oldest: bool,
    /// Open time of the newest bar at the last frame: how many bars arrived since is how far a
    /// scrolled-back view has to move to stay on the same bars.
    newest_bar: Option<i64>,
    /// The horizontal viewport as this frame's input found it, to tell what the user changed.
    sync_baseline: Option<crate::model::viewport::HorizontalViewport>,
    /// What the user's navigation changed this frame, for the workspace to hand to linked charts.
    sync_change: Option<crate::model::chart_sync::HorizontalChange>,
    /// What the last frame drew, for the tests that drive the chart by pointer.
    #[cfg(test)]
    probe: Option<Probe>,
    /// What the overlay scripts need beyond the candles: slots past the last bar and prices the
    /// scale must reach. Set each frame before the view is built.
    script_extent: OverlayExtent,
    layer: DrawingLayer,
    /// Paper-trading inputs and outputs for this card; the workspace fills and drains it.
    pub paper: PaperCardState,
    /// The Pine scripts applied to this chart, in order. Overlay scripts draw over the candles;
    /// the others share a pane under them.
    instances: Vec<PineInstance>,
    /// The preview's input overrides, kept here so they reach an instance made after they were set
    /// (the Script Manager sets a script's saved inputs before its source arrives).
    preview_inputs: HashMap<String, InputValue>,
    /// What each instance's last run produced, by instance id.
    runs: HashMap<u64, Run>,
    /// Counts the runs this card started, so each gets an epoch of its own: an instance id is
    /// reused once its script is removed, a run is never.
    run_epochs: u64,
    /// The bars a price-based chart type constructed from the candles, kept until the candles or
    /// the construction change. Restyling never rebuilds it.
    constructed: Option<Arc<ConstructedSeries>>,
    /// The type drawn at the last frame (the selected one, or candles where the source cannot show
    /// it): what the shared up/down colours belong to.
    drawn_type: ChartType,
    /// The workspace's timeframe, for the watermark.
    timeframe: Option<Timeframe>,
    /// The script pane that fills the chart, if one does. View state, not saved.
    maximized_pane: Option<u64>,
    /// The scripts that had a pane at the last frame, top to bottom.
    pane_ids: Vec<u64>,
    /// The time a vertical cursor line is locked at (context menu). View state, not saved: it is
    /// not a drawing, and it follows neither templates nor linked charts.
    time_lock: Option<i64>,
    /// Table view is open on this chart: build its feed each frame something changes.
    table_wanted: bool,
    table_feed: Option<Arc<crate::model::chart_table::TableFeed>>,
    /// Fingerprint of what `table_feed` was built from.
    table_stamp: u64,
    /// The price alerts to draw as lines (the workspace fills them each frame), and the tag
    /// right-clicked since the last request was read.
    alert_lines: Vec<crate::model::alert_line::AlertLine>,
    alert_menu: Option<alert_lines::LineMenu>,
    /// The market the alert tags are dragged on, and the tags dropped on a new level since the
    /// workspace last took them.
    alert_market: alert_lines::Market,
    alert_moves: Vec<alert_lines::AlertMove>,
    /// Whether the corner carries the price-scale menu (a script preview has none), and the mode
    /// picked from it since the workspace last took it.
    scale_menu: bool,
    scale_request: Option<PriceScaleMode>,
}

/// A saved bullish/bearish colour as an opaque `Rgba`.
fn opaque_rgba([r, g, b]: [u8; 3]) -> crate::model::chart_type::Rgba {
    [r, g, b, 255]
}

/// The bars a price-based type built, with what they were built from.
struct ConstructedSeries {
    construction: Construction,
    revision: RunRevision,
    series: SyntheticSeries,
    /// `series` as the candles scripts, indicators and the price scale read.
    klines: Vec<Kline>,
}

/// The instance the Script Manager's single-script preview chart runs.
const PREVIEW_INSTANCE: u64 = 0;

/// One instance's last run, cached so the card does not recompile every frame.
struct Run {
    source: String,
    inputs: HashMap<String, StoredInput>,
    symbol: SymbolInfo,
    bar_seconds: Option<u64>,
    /// The chart colours the script ran against; a change of app theme runs it again.
    theme: ChartTheme,
    worker: PineWorker,
    requested: Option<RunRevision>,
    generation: u64,
    /// Which run of this card this is; a restart (new source, inputs, symbol) is a new epoch.
    #[allow(dead_code)] // Read by the Pine logs pane.
    epoch: u64,
    /// The `log.*()` messages the run wrote. They belong to the run, not to an output generation:
    /// a result whose output went stale still carries messages nothing else will deliver.
    logs: LogHistory,
    /// Whether `output.overlay` came from a compiled script (or from a previous valid run). Until
    /// then the default output must not claim a pane: `VisualOutput::default()` is non-overlay.
    placement_known: bool,
    output: Arc<VisualOutput>,
    /// The runtime failure that stopped this run early.
    error: Option<Diagnostic>,
    /// The last run that finished without a runtime failure: what stays on screen while a newer
    /// source fails to run.
    last_valid: Option<Arc<VisualOutput>>,
    /// `output` with the instance's style choices applied, and the choices it was made with
    /// (`None` until the first styling).
    styled: Arc<VisualOutput>,
    styled_with: Option<HashMap<String, String>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RunRevision {
    candles: u64,
    visible_range: Option<(i64, i64)>,
    last_is_closed: bool,
}

struct WorkerRequest {
    generation: u64,
    bars: Vec<Kline>,
    visible_range: Option<(i64, i64)>,
    now_ms: i64,
}

struct WorkerResult {
    generation: u64,
    output: VisualOutput,
    error: Option<Diagnostic>,
    logs: LogBatch,
    /// The controller compiled the declaration, so `output.overlay` is authoritative even when
    /// execution stopped with a runtime error.
    placement_known: bool,
}

struct WorkerConfig {
    theme: ChartTheme,
    source: String,
    inputs: HashMap<String, InputValue>,
    symbol: SymbolInfo,
    /// The length of the chart's bars, when their times cannot say (price-based charts).
    bar_seconds: Option<f64>,
}

enum WorkerEngine {
    Uninitialized(WorkerConfig),
    Ready(Box<ExecutionController>),
    Failed(Diagnostic),
}

struct WorkerState {
    engine: Option<WorkerEngine>,
    pending: Option<WorkerRequest>,
    running: bool,
    cancelled: bool,
}

/// Latest-wins background runner. Only one work-queue job touches a controller at a time; updates
/// arriving while it runs replace the pending snapshot rather than building an unbounded queue.
struct PineWorker {
    state: Arc<Mutex<WorkerState>>,
    results: Receiver<WorkerResult>,
    result_sender: Sender<WorkerResult>,
    repaint: egui::Context,
    cancellation: Arc<AtomicBool>,
}

impl PineWorker {
    fn new(
        source: String,
        inputs: HashMap<String, InputValue>,
        symbol: SymbolInfo,
        theme: ChartTheme,
        bar_seconds: Option<f64>,
        repaint: egui::Context,
    ) -> Self {
        let (result_sender, results) = mpsc::channel();
        Self {
            state: Arc::new(Mutex::new(WorkerState {
                engine: Some(WorkerEngine::Uninitialized(WorkerConfig {
                    theme,
                    source,
                    inputs,
                    symbol,
                    bar_seconds,
                })),
                pending: None,
                running: false,
                cancelled: false,
            })),
            results,
            result_sender,
            repaint,
            cancellation: Arc::new(AtomicBool::new(false)),
        }
    }

    fn request(&self, request: WorkerRequest) {
        let should_start = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            if state.cancelled {
                return;
            }
            state.pending = Some(request);
            if state.running {
                false
            } else {
                state.running = true;
                true
            }
        };
        if !should_start {
            return;
        }
        let state = Arc::clone(&self.state);
        let sender = self.result_sender.clone();
        let repaint = self.repaint.clone();
        let cancellation = Arc::clone(&self.cancellation);
        crate::service::work_queue::submit(move || {
            loop {
                let (request, engine) = {
                    let Ok(mut state) = state.lock() else {
                        return;
                    };
                    if state.cancelled {
                        state.running = false;
                        return;
                    }
                    let Some(request) = state.pending.take() else {
                        state.running = false;
                        return;
                    };
                    let engine = state.engine.take().expect("worker engine");
                    (request, engine)
                };
                let mut engine = match engine {
                    WorkerEngine::Uninitialized(config) => match ExecutionController::new(
                        &config.source,
                        config.inputs,
                        config.symbol,
                        request.visible_range,
                        Limits::default(),
                        &LibraryRegistry,
                        Some(Box::new(KlineSecurityDataProvider)),
                        Some(Arc::clone(&cancellation)),
                    ) {
                        Ok(controller) => {
                            let controller = controller.with_theme(config.theme).with_logs();
                            WorkerEngine::Ready(Box::new(match config.bar_seconds {
                                Some(seconds) => controller.with_bar_seconds(seconds),
                                None => controller,
                            }))
                        }
                        Err(error) => WorkerEngine::Failed(error),
                    },
                    engine => engine,
                };
                let result = match &mut engine {
                    WorkerEngine::Ready(controller) => {
                        let update =
                            controller.update(&request.bars, request.visible_range, request.now_ms);
                        WorkerResult {
                            generation: request.generation,
                            output: update.output,
                            error: update.error,
                            logs: update.logs,
                            placement_known: true,
                        }
                    }
                    WorkerEngine::Failed(error) => WorkerResult {
                        generation: request.generation,
                        output: VisualOutput::default(),
                        error: Some(error.clone()),
                        logs: LogBatch::default(),
                        placement_known: false,
                    },
                    WorkerEngine::Uninitialized(_) => unreachable!(),
                };
                {
                    let Ok(mut state) = state.lock() else {
                        return;
                    };
                    state.engine = Some(engine);
                    if state.cancelled {
                        state.running = false;
                        return;
                    }
                }
                let _ = sender.send(result);
                repaint.request_repaint();
            }
        });
    }
}

impl Drop for PineWorker {
    fn drop(&mut self) {
        self.cancellation.store(true, Ordering::Release);
        if let Ok(mut state) = self.state.lock() {
            state.cancelled = true;
            state.pending = None;
        }
    }
}

/// One run's `log.*()` messages. The epoch tells a restarted run from the one before it, which
/// the instance id cannot.
#[allow(dead_code)] // Read by the Pine logs pane.
pub struct PineLogs<'a> {
    pub epoch: u64,
    pub history: &'a LogHistory,
}

/// How one instance is doing, for the chart settings' Scripts tab.
pub struct InstanceStatus<'a> {
    pub output: &'a VisualOutput,
    pub error: Option<&'a Diagnostic>,
}

impl Default for ChartCard {
    fn default() -> Self {
        Self {
            appearance: ChartAppearance::default(),
            body_size: Vec2::ZERO,
            title: None,
            series: Vec::new(),
            series_cutoff: None,
            viewport: ChartViewport::default(),
            gesture: None,
            newest_bar: None,
            near_oldest: false,
            sync_baseline: None,
            sync_change: None,
            #[cfg(test)]
            probe: None,
            script_extent: OverlayExtent::default(),
            layer: DrawingLayer::default(),
            paper: PaperCardState::default(),
            instances: Vec::new(),
            preview_inputs: HashMap::new(),
            runs: HashMap::new(),
            run_epochs: 0,
            constructed: None,
            drawn_type: ChartType::Candles,
            timeframe: None,
            maximized_pane: None,
            pane_ids: Vec::new(),
            time_lock: None,
            table_wanted: false,
            table_feed: None,
            table_stamp: 0,
            alert_lines: Vec::new(),
            alert_menu: None,
            alert_market: alert_lines::Market::default(),
            alert_moves: Vec::new(),
            scale_menu: true,
            scale_request: None,
        }
    }
}

impl ChartCard {
    pub fn appearance(&self) -> ChartAppearance {
        self.appearance
    }

    pub fn appearance_mut(&mut self) -> &mut ChartAppearance {
        &mut self.appearance
    }

    pub fn set_appearance(&mut self, appearance: ChartAppearance) {
        // A saved row may hold values the builders cannot use: heal them rather than draw nothing.
        self.appearance = ChartAppearance {
            style: appearance.style.sanitized(),
            canvas: appearance.canvas.sanitized(),
            ..appearance
        };
    }

    /// Replaces the chart's look with a template's, keeping what the template does not own: the
    /// market, the bars' construction (unless `scope` takes the type too), indicators, scripts,
    /// drawings and alerts are all elsewhere. A new chart type goes through
    /// [`Self::set_chart_type`], so the scroll position and the constructed bars follow it as they
    /// do for a pick in the selector.
    pub fn apply_template(
        &mut self,
        template: &ChartAppearance,
        scope: crate::model::chart_template::ApplyScope,
    ) {
        let merged = scope.merge(&self.appearance, template);
        let chart_type = merged.style.chart_type;
        let current = self.appearance.style.chart_type;
        self.set_appearance(ChartAppearance {
            style: ChartStyle {
                chart_type: current,
                ..merged.style
            },
            ..merged
        });
        self.set_chart_type(chart_type);
    }

    /// How many evaluated scripts draw in panes under the price chart. A pending script claims no
    /// pane until its compiled declaration says it needs one.
    pub fn pane_count(&self) -> usize {
        self.pane_ids.len()
    }

    pub fn set_timeframe(&mut self, timeframe: Timeframe) {
        self.timeframe = Some(timeframe);
    }

    /// The change across the candles on screen, by the same window the plot draws.
    pub fn visible_change(&self, candles: &[Kline]) -> Option<f64> {
        visible_price_change(
            candles,
            self.viewport.horizontal.visible,
            self.viewport.horizontal.right_offset,
            self.reserved_slots(),
        )
    }

    /// The slots past the newest bar the plot keeps free: the right margin, or more where an
    /// overlay script draws further.
    fn reserved_slots(&self) -> usize {
        usize::from(self.appearance.canvas.margins.sanitized().right)
            .max(self.script_extent.future_bars)
    }

    pub fn chart_style_mut(&mut self) -> &mut ChartStyle {
        &mut self.appearance.style
    }

    /// Switches the chart type; each type keeps the settings it had.
    pub fn set_chart_type(&mut self, chart_type: ChartType) {
        let before = self.appearance.style.chart_type;
        self.appearance.style.chart_type = chart_type;
        // A price-based type has its own number of bars: the scroll position of the old series
        // means nothing on it.
        if before != chart_type && (before.is_synthetic() || chart_type.is_synthetic()) {
            self.viewport.horizontal.right_offset = 0.0;
            self.gesture = None;
        }
    }

    /// The chart type drawn for `source`: the selected one, else candles when the source cannot
    /// show it.
    pub fn effective_chart_type(&self, source: DataSource) -> ChartType {
        self.appearance.style.effective_type(source)
    }

    /// What a script alert on this chart runs over: the bars of a price-based type, the market's
    /// candles otherwise, and how the bars were made.
    pub fn pine_feed(
        &self,
        candles: &[Kline],
        source: DataSource,
        now_ms: i64,
    ) -> (Vec<Kline>, crate::host::pine_subscription_host::ChartRun) {
        let mut style = self.appearance.style;
        style.chart_type = self.effective_chart_type(source);
        match Construction::of(&style) {
            Some(construction) => {
                let bars = synthetic::build_with(construction, candles, now_ms).klines();
                let bar_seconds = candles
                    .last()
                    .map(|last| ((last.close_time - last.open_time + 1) as f64 / 1000.0).round())
                    .filter(|seconds| *seconds > 0.0);
                (
                    bars,
                    crate::host::pine_subscription_host::ChartRun {
                        construction: Some(construction),
                        bar_seconds,
                    },
                )
            }
            None => (
                candles.to_vec(),
                crate::host::pine_subscription_host::ChartRun::default(),
            ),
        }
    }

    /// The bars `chart_type` constructs from `candles`, or `None` for a type that draws the
    /// provider's candles as they are. Rebuilt only when the candles or the construction change.
    fn constructed_series(
        &mut self,
        candles: &[Kline],
        chart_type: ChartType,
    ) -> Option<Arc<ConstructedSeries>> {
        let built = self.constructed_for(candles, chart_type);
        self.constructed.clone_from(&built);
        built
    }

    /// What [`Self::constructed_series`] answers, without keeping it.
    fn constructed_for(
        &self,
        candles: &[Kline],
        chart_type: ChartType,
    ) -> Option<Arc<ConstructedSeries>> {
        let mut style = self.appearance.style;
        style.chart_type = chart_type;
        let construction = Construction::of(&style)?;
        let now_ms = current_time_ms();
        let revision = run_revision(candles, None, now_ms);
        if let Some(built) = &self.constructed
            && built.construction == construction
            && built.revision == revision
        {
            return Some(Arc::clone(built));
        }
        let series = synthetic::build_with(construction, candles, now_ms);
        let klines = series.klines();
        Some(Arc::new(ConstructedSeries {
            construction,
            revision,
            series,
            klines,
        }))
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn set_title(&mut self, title: Option<String>) {
        self.title = title;
    }

    pub fn set_series(&mut self, series: Vec<SeriesLine>) {
        self.series = series;
    }

    pub fn set_series_cutoff(&mut self, cutoff: Option<i64>) {
        self.series_cutoff = cutoff;
    }

    /// The name in the header: the explicit title, else the market's own symbol.
    fn display_title(&self, symbol: &str, source: DataSource) -> String {
        self.title
            .clone()
            .unwrap_or_else(|| source.display_symbol(symbol))
    }

    /// A line's points up to the replay cutoff.
    fn series_points<'a>(&self, line: &'a SeriesLine) -> impl Iterator<Item = &'a Kline> + use<'a> {
        let cutoff = self.series_cutoff;
        line.candles
            .iter()
            .take_while(move |candle| cutoff.is_none_or(|cutoff| candle.open_time <= cutoff))
    }

    /// The choice with the highest probability right now, for the header's readout.
    fn leading_choice(&self) -> Option<(&str, f64)> {
        self.series
            .iter()
            .filter_map(|line| {
                self.series_points(line)
                    .last()
                    .map(|candle| (line.label.as_str(), candle.close))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// The up and down colours of what the chart is drawing: the selected type's own where it has
    /// them, else the saved bullish/bearish palette. Volume bars, the last-price tag and the
    /// trend flips use them, so recolouring the bars recolours those too.
    fn up_down_colors(&self) -> (Color32, Color32) {
        let base = (
            opaque_rgba(self.appearance.bullish),
            opaque_rgba(self.appearance.bearish),
        );
        let mut style = self.appearance.style;
        style.chart_type = self.drawn_type;
        let (up, down) = style.up_down(base);
        (series_render::color(up), series_render::color(down))
    }

    fn bullish_color(&self) -> Color32 {
        self.up_down_colors().0
    }

    fn bearish_color(&self) -> Color32 {
        self.up_down_colors().1
    }

    pub fn set_y_zoom(&mut self, y_zoom: f64) {
        self.viewport.vertical.zoom = y_zoom;
    }

    pub fn y_zoom(&self) -> f64 {
        self.viewport.vertical.zoom
    }

    /// The scroll-zoom candle count (visible candles).
    pub fn visible_candles(&self) -> usize {
        self.viewport.horizontal.visible
    }

    /// Restores the scroll-zoom candle count. `0` means "keep the default".
    pub fn set_visible_candles(&mut self, count: usize) {
        if count > 0 {
            self.viewport.horizontal.visible =
                count.clamp(5, crate::model::viewport::MAX_RESTORED_VISIBLE);
        }
    }

    /// Whether the view is within two screens of the oldest bar the chart holds, early enough for
    /// the next page to arrive before the user scrolls onto it.
    pub fn near_oldest_bar(&self) -> bool {
        self.near_oldest
    }

    /// The scroll position and hand-placed price range a layout keeps.
    pub fn saved_viewport(&self) -> SavedViewport {
        self.viewport.saved()
    }

    /// Restores what [`Self::saved_viewport`] kept.
    pub fn restore_viewport(&mut self, saved: SavedViewport) {
        self.viewport.restore(saved);
    }

    // MARK: Pine instances

    pub fn pine_instances(&self) -> &[PineInstance] {
        &self.instances
    }

    /// Replaces every instance (restoring a saved chart).
    pub fn set_pine_instances(&mut self, instances: Vec<PineInstance>) {
        self.instances = instances;
        self.runs.clear();
    }

    /// Applies a script to the chart, after the ones already there. Returns the new instance's id.
    pub fn add_pine_instance(
        &mut self,
        script_id: Option<String>,
        name: &str,
        source: &str,
    ) -> u64 {
        let id = self
            .instances
            .iter()
            .map(|i| i.id)
            .max()
            .map_or(1, |max| max + 1);
        self.instances
            .push(PineInstance::new(id, script_id, name, source));
        id
    }

    pub fn remove_pine_instance(&mut self, id: u64) {
        self.instances.retain(|instance| instance.id != id);
        self.runs.remove(&id);
    }

    /// Moves an instance to `index` (clamped); draw order follows list order.
    pub fn move_pine_instance(&mut self, id: u64, index: usize) {
        if let Some(from) = self.instances.iter().position(|i| i.id == id) {
            let instance = self.instances.remove(from);
            let to = index.min(self.instances.len());
            self.instances.insert(to, instance);
        }
    }

    /// Sends an instance's visuals in front of or behind the chart's own candles; a restyle, so it
    /// never reruns the script.
    pub fn set_pine_visual_order(&mut self, id: u64, order: VisualOrder) {
        if let Some(instance) = self.instances.iter_mut().find(|i| i.id == id) {
            instance.visual_order = order;
        }
    }

    pub fn set_pine_visible(&mut self, id: u64, visible: bool) {
        if let Some(instance) = self.instances.iter_mut().find(|i| i.id == id) {
            instance.visible = visible;
        }
    }

    /// Sets an instance's style overrides; the next frame draws with them.
    pub fn set_pine_instance_style(&mut self, id: u64, overrides: HashMap<String, String>) {
        if let Some(instance) = self.instances.iter_mut().find(|i| i.id == id) {
            instance.style_overrides = overrides;
        }
    }

    /// What the user can restyle in an instance's output, as the script itself draws it. Empty
    /// until the script has run.
    pub fn pine_style_rows(&self, id: u64) -> Vec<StyleRow> {
        self.runs
            .get(&id)
            .filter(|run| run.placement_known)
            .map(|run| style_rows(&run.output))
            .unwrap_or_default()
    }

    /// Sets an instance's input overrides; its next run uses them.
    pub fn set_pine_instance_inputs(&mut self, id: u64, inputs: &HashMap<String, InputValue>) {
        if let Some(instance) = self.instances.iter_mut().find(|i| i.id == id) {
            instance.set_input_values(inputs);
        }
    }

    /// The messages an instance's script wrote with `log.*()` in its current run.
    #[allow(dead_code)] // Read by the Pine logs pane.
    pub fn pine_logs(&self, id: u64) -> Option<PineLogs<'_>> {
        self.runs.get(&id).map(|run| PineLogs {
            epoch: run.epoch,
            history: &run.logs,
        })
    }

    pub fn pine_status(&self, id: u64) -> Option<InstanceStatus<'_>> {
        self.runs
            .get(&id)
            .filter(|run| run.placement_known || run.error.is_some())
            .map(|run| InstanceStatus {
                output: &run.output,
                error: run.error.as_ref(),
            })
    }

    // The single-script preview (Script Manager): one instance, id 0, that keeps its last good
    // output while a newer source fails to run.

    /// Sets (or clears) the one script the preview chart runs.
    pub fn set_pine_source(&mut self, source: Option<String>) {
        match source {
            Some(source) => match self.instances.first_mut() {
                Some(instance) if instance.id == PREVIEW_INSTANCE => instance.source = source,
                _ => {
                    let mut instance =
                        PineInstance::new(PREVIEW_INSTANCE, None, "Preview", &source);
                    instance.set_input_values(&self.preview_inputs);
                    self.instances = vec![instance];
                    self.runs.clear();
                }
            },
            None => {
                self.instances.clear();
                self.runs.clear();
            }
        }
    }

    /// Sets the input overrides the preview script runs with.
    pub fn set_pine_inputs(&mut self, inputs: HashMap<String, InputValue>) {
        self.set_pine_instance_inputs(PREVIEW_INSTANCE, &inputs);
        self.preview_inputs = inputs;
    }

    /// The preview run's runtime failure, if it stopped early.
    pub fn pine_error(&self) -> Option<&Diagnostic> {
        self.runs.get(&PREVIEW_INSTANCE)?.error.as_ref()
    }

    /// Whether a preview run has finished cleanly, so a later failure leaves real output up.
    pub fn has_valid_pine_output(&self) -> bool {
        self.runs
            .get(&PREVIEW_INSTANCE)
            .is_some_and(|run| run.last_valid.is_some())
    }

    /// The preview run's output (strategy report, alerts), for the Report tab.
    pub fn pine_visual(&self) -> Option<&VisualOutput> {
        self.runs
            .get(&PREVIEW_INSTANCE)
            .filter(|run| run.placement_known)
            .map(|run| &*run.output)
    }

    /// The visible instances' outputs in order. Execution is persistent and backgrounded: an
    /// intrabar OHLCV change is a realtime tick, a new opening time closes the prior bar, and
    /// stale worker generations cannot overwrite newer chart state.
    fn pine_outputs(
        &mut self,
        ctx: &egui::Context,
        symbol_info: &SymbolInfo,
        candles: &[Kline],
        bar_seconds: Option<f64>,
    ) -> Vec<(Arc<VisualOutput>, VisualOrder, u64)> {
        if self.instances.iter().all(|instance| !instance.visible) {
            self.runs.clear();
            return Vec::new();
        }
        let window = self.window(candles.len());
        let visible_range = candles
            .get(window.start..window.end)
            .and_then(|visible| Some((visible.first()?.open_time, visible.last()?.open_time)));
        let now_ms = current_time_ms();
        // Hashing every candle is the one pass over the whole series a frame makes; a chart with
        // no script has no use for it.
        let revision = run_revision(candles, visible_range, now_ms);
        // What `chart.bg_color` and `chart.fg_color` read: the colours the chart is drawn on.
        let chart_theme = if theme::is_light() {
            ChartTheme::LIGHT
        } else {
            ChartTheme::DARK
        };
        let visible_ids: HashSet<u64> = self
            .instances
            .iter()
            .filter(|instance| instance.visible)
            .map(|instance| instance.id)
            .collect();
        self.runs.retain(|id, _| visible_ids.contains(id));
        let mut outputs = Vec::new();
        for instance in self.instances.iter().filter(|i| i.visible) {
            let configured = self.runs.get(&instance.id).is_some_and(|run| {
                run.source == instance.source
                    && run.inputs == instance.inputs
                    && run.symbol == *symbol_info
                    && run.bar_seconds == bar_seconds.map(f64::to_bits)
                    && run.theme == chart_theme
            });
            if !configured {
                self.run_epochs += 1;
                let epoch = self.run_epochs;
                let previous_valid = self
                    .runs
                    .remove(&instance.id)
                    .and_then(|run| run.last_valid.clone());
                let placement_known = previous_valid.is_some();
                let shown = previous_valid
                    .clone()
                    .unwrap_or_else(|| Arc::new(VisualOutput::default()));
                self.runs.insert(
                    instance.id,
                    Run {
                        source: instance.source.clone(),
                        inputs: instance.inputs.clone(),
                        symbol: symbol_info.clone(),
                        bar_seconds: bar_seconds.map(f64::to_bits),
                        theme: chart_theme,
                        worker: PineWorker::new(
                            instance.source.clone(),
                            instance.input_values(),
                            symbol_info.clone(),
                            chart_theme,
                            bar_seconds,
                            ctx.clone(),
                        ),
                        requested: None,
                        generation: 0,
                        epoch,
                        logs: LogHistory::default(),
                        placement_known,
                        styled: Arc::clone(&shown),
                        styled_with: None,
                        output: shown,
                        error: None,
                        last_valid: previous_valid,
                    },
                );
            }
            if let Some(run) = self.runs.get_mut(&instance.id) {
                while let Ok(result) = run.worker.results.try_recv() {
                    run.logs.apply(result.logs);
                    if result.generation != run.generation {
                        continue;
                    }
                    let output = Arc::new(result.output);
                    let shown = match (&result.error, run.last_valid.clone()) {
                        (None, _) => {
                            run.last_valid = Some(Arc::clone(&output));
                            Arc::clone(&output)
                        }
                        (Some(_), Some(valid)) => valid,
                        (Some(_), None) => Arc::clone(&output),
                    };
                    run.output = shown;
                    run.error = result.error;
                    run.placement_known = result.placement_known || run.last_valid.is_some();
                    run.styled_with = None;
                }
                // Only a script that reads the visible range depends on where the chart is
                // scrolled: for every other one, panning is not a reason to run it again.
                let (revision, visible_range) = if reads_visible_range(&instance.source) {
                    (revision, visible_range)
                } else {
                    (
                        RunRevision {
                            visible_range: None,
                            ..revision
                        },
                        None,
                    )
                };
                if run.requested != Some(revision) {
                    run.generation = run.generation.wrapping_add(1);
                    run.requested = Some(revision);
                    run.worker.request(WorkerRequest {
                        generation: run.generation,
                        bars: candles.to_vec(),
                        visible_range,
                        now_ms,
                    });
                }
                if run.styled_with.as_ref() != Some(&instance.style_overrides) {
                    run.styled = Arc::new(
                        StyleOverrides::new(instance.style_overrides.clone()).apply(&run.output),
                    );
                    run.styled_with = Some(instance.style_overrides.clone());
                }
                if run.placement_known {
                    outputs.push((Arc::clone(&run.styled), instance.visual_order, instance.id));
                }
            }
        }
        outputs
    }
}

/// The value labels a script pane writes on its axis, with the value each stands at. The painter
/// and the axis width both read this, so the axis is as wide as the labels it draws.
fn pane_labels(
    pine: &VisualOutput,
    shown: usize,
    plot_height: f32,
    style: &CanvasStyle,
) -> Vec<(f64, String)> {
    let (low, high) = crate::ui::pine_geometry::pane_value_range(pine, shown, 0.1);
    // A pane is short: its labels sit closer than the price chart's, and it keeps at least two.
    let budget = canvas::tick_budget(plot_height, style.pane_tick_spacing()).max(2);
    let ticks = canvas::price_ticks(low, high, false, budget);
    let step = tick_step(&ticks);
    ticks
        .into_iter()
        .map(|value| {
            // A tick that is zero up to rounding reads "0", not eight decimals of nothing.
            let label = if value.abs() < (high - low).abs() * 1e-9 {
                "0".to_owned()
            } else {
                PriceFormat::AUTO.format_tick(value, step)
            };
            (value, label)
        })
        .collect()
}

/// The own pane under the candles for `overlay=false` scripts, ported from `PineScriptPaneView`:
/// a value axis fitted to the script's outputs, sharing the price chart's horizontal layout so
/// bar slots line up with the candles above (the time axis stays up there). The pane takes the
/// canvas' grid, scale text and separator like the price chart does, each with its own scale.
#[allow(clippy::too_many_arguments)]
fn draw_script_pane(
    painter: &egui::Painter,
    pane: Rect,
    price_plot: Rect,
    pine: &VisualOutput,
    shown: &[Kline],
    tail: usize,
    slot_width: f32,
    x_for_index: &dyn Fn(i64) -> f32,
    style: &CanvasStyle,
    tick_xs: &[f32],
) {
    // Top and bottom leave just enough room for the outermost axis labels.
    let plot = Rect::from_min_max(
        Pos2::new(price_plot.left(), pane.top() + 8.0),
        Pos2::new(price_plot.right(), pane.bottom() - 8.0),
    );
    if plot.height() <= 1.0 {
        return;
    }
    let (low, high) = crate::ui::pine_geometry::pane_value_range(pine, shown.len(), 0.1);
    let span = (high - low).max(f64::EPSILON);
    let y_for = |value: f64| plot.bottom() - ((value - low) / span) as f32 * plot.height();
    let labels = pane_labels(pine, shown.len(), plot.height(), style);
    if let Some(grid) = style.vertical_grid {
        for x in tick_xs {
            if *x >= plot.left() && *x <= plot.right() {
                canvas_render::vertical(painter, *x, (plot.top(), plot.bottom()), grid);
            }
        }
    }
    for (value, label) in labels {
        let y = y_for(value);
        if !y.is_finite() || y < plot.top() - 0.5 || y > plot.bottom() + 0.5 {
            continue;
        }
        if let Some(grid) = style.horizontal_grid {
            canvas_render::horizontal(painter, (plot.left(), plot.right()), y, grid);
        }
        price_text::paint(
            painter,
            Pos2::new(pane.right() - 7.0, y),
            Align2::RIGHT_CENTER,
            &label,
            style.price_font(),
            style.text_color,
        );
    }
    let layer = PineLayer {
        pine,
        candles: shown,
        tail,
        in_pane: true,
        plot,
        slot_width,
        x_for_index,
        y_for: &y_for,
        candle_style: CandleStyle {
            body_width: candle_body_half_width(slot_width) * 2.0,
            wick_width: 1.0,
            min_body_height: 1.0,
        },
    };
    let clipped = painter.with_clip_rect(plot.expand2(Vec2::new(0.0, 0.0)));
    layer.draw_background(&clipped);
    layer.draw_foreground(&clipped);
    // Tables pin to the plot's corners and are never price-anchored, so they sit outside the
    // series clip.
    layer.draw_tables(painter);
}

/// Whether a chart on `source` can raise price alerts: markets with a price quote feed.
pub fn supports_alerts(source: DataSource) -> bool {
    source != DataSource::CoinMarketCap && !source.is_prediction_market()
}

/// The one map between a price and a height, and what the axis says at a height.
///
/// The map is linear, or logarithmic where `logarithmic` is set. `mode` and `reference` only
/// change what the axis, its tags and the crosshair say: Percent and Indexed are linear in the
/// price, so they move no pixel and every price a chart stores or sends stays a real price.
pub(super) struct PriceScale {
    pub(super) minimum: f64,
    pub(super) maximum: f64,
    pub(super) top: f32,
    pub(super) height: f32,
    /// Whether this frame is drawn on a logarithmic map: the mode asks for it and the view has
    /// no price a logarithm cannot take ([`price_scale::can_draw_logarithmic`]).
    pub(super) logarithmic: bool,
    pub(super) mode: PriceScaleMode,
    /// What Percent and Indexed measure from: the close of the first bar shown.
    pub(super) reference: Option<f64>,
}

/// The labelled prices of an axis, and the distance between two of them as the axis reads it.
pub(super) struct AxisTicks {
    pub(super) prices: Vec<f64>,
    pub(super) step: f64,
}

impl PriceScale {
    /// A linear scale that reads prices.
    pub(super) fn linear(minimum: f64, maximum: f64, top: f32, height: f32) -> Self {
        Self {
            minimum,
            maximum,
            top,
            height,
            logarithmic: false,
            mode: PriceScaleMode::Regular,
            reference: None,
        }
    }

    fn project(&self, price: f64) -> f64 {
        if self.logarithmic {
            // Only a price below the range gets here with no logarithm, and it is off the plot.
            price.max(f64::MIN_POSITIVE).ln()
        } else {
            price
        }
    }

    /// The mode the axis reads in: a relative one only while it has a reference to measure from.
    fn reading(&self) -> Option<(PriceScaleMode, f64)> {
        let reference = self.reference?;
        self.mode.is_relative().then_some((self.mode, reference))
    }

    /// The prices the axis labels, where the grid is drawn too. A relative mode picks its ticks
    /// in the change it shows, so they are round there (`-10%`, `0%`, `+10%`) and `0%` sits on
    /// the reference, and maps them back to prices.
    pub(super) fn axis_ticks(&self, budget: usize) -> AxisTicks {
        let Some((mode, reference)) = self.reading() else {
            let prices = canvas::price_ticks(self.minimum, self.maximum, self.logarithmic, budget);
            let step = tick_step(&prices);
            return AxisTicks { prices, step };
        };
        let shown = |price| price_scale::display(mode, Some(reference), price);
        let (Some(low), Some(high)) = (shown(self.minimum), shown(self.maximum)) else {
            return AxisTicks {
                prices: Vec::new(),
                step: 0.0,
            };
        };
        let values = canvas::price_ticks(low, high, false, budget);
        let step = tick_step(&values);
        let prices = values
            .into_iter()
            .filter_map(|value| price_scale::from_display(mode, Some(reference), value))
            .collect();
        AxisTicks { prices, step }
    }

    /// An axis label for `price`; `step` is the distance between ticks as [`Self::axis_ticks`]
    /// gives it.
    pub(super) fn tick_label(&self, format: PriceFormat, price: f64, step: f64) -> String {
        match self.reading() {
            Some((mode, reference)) => price_scale::display(mode, Some(reference), price)
                .map_or_else(
                    || "—".to_owned(),
                    |value| price_scale::format_relative(mode, value, Some(step)),
                ),
            None => format.format_tick(price, step),
        }
    }

    /// A tag on the axis (the last price, the crosshair, a line's level) as the axis reads.
    pub(super) fn tag_label(&self, format: PriceFormat, price: f64) -> String {
        match self.reading() {
            Some((mode, reference)) => price_scale::display(mode, Some(reference), price)
                .map_or_else(
                    || "—".to_owned(),
                    |value| price_scale::format_relative(mode, value, None),
                ),
            None => format.format(price),
        }
    }

    pub(super) fn y(&self, price: f64) -> f32 {
        let minimum = self.project(self.minimum);
        let maximum = self.project(self.maximum);
        let fraction =
            ((self.project(price) - minimum) / (maximum - minimum).max(f64::EPSILON)) as f32;
        self.top + self.height * (1.0 - fraction)
    }

    pub(super) fn price(&self, y: f32) -> f64 {
        let minimum = self.project(self.minimum);
        let maximum = self.project(self.maximum);
        let fraction = 1.0 - f64::from((y - self.top) / self.height);
        let projected = minimum + fraction * (maximum - minimum);
        if self.logarithmic {
            projected.exp()
        } else {
            projected
        }
    }
}

/// The geometry of the last frame, as a test needs it to aim a pointer.
#[cfg(test)]
#[derive(Clone, Copy)]
struct Probe {
    plot: Rect,
    start: usize,
    lead: f32,
    slot_width: f32,
    minimum: f64,
    maximum: f64,
}

/// The visible slice of the data and the scales that map it onto the plot.
struct View {
    start: usize,
    end: usize,
    scale: PriceScale,
    slot_width: f32,
    lead: f32,
}

impl View {
    fn projection<'a>(
        &'a self,
        plot: Rect,
        candles: &'a [Kline],
        format: PriceFormat,
        ctx: &egui::Context,
    ) -> Projection<'a> {
        Projection {
            plot,
            pixels_per_point: ctx.pixels_per_point(),
            measure: TextMeasure::new(ctx),
            format,
            scale: &self.scale,
            times: TimeIndex::new(candles),
            start: self.start,
            slot_width: self.slot_width,
            lead: self.lead,
        }
    }
}

/// Whether a table's cell was selected on some chart in the last frame, and forgets it: while one
/// is, the keyboard's Copy, Cut and Paste are the cell's and the workspace leaves them alone.
pub fn take_table_cell_selected(ctx: &egui::Context) -> bool {
    ctx.data_mut(|data| {
        data.remove_temp::<bool>(egui::Id::new(crate::ui::drawing_layer::TABLE_CELL_FLAG))
            .unwrap_or(false)
    })
}

impl ChartCard {
    /// The drawings whose note is open (a pin's), so a snapshot can show them as they stand.
    pub fn open_notes(&self) -> Vec<u64> {
        self.layer.open_notes()
    }

    /// Visible range and price scale for the current pan/zoom, if anything is shown.
    fn view(
        &self,
        candles: &[Kline],
        plot: Rect,
        mode: PriceScaleMode,
        chart_type: ChartType,
    ) -> Option<View> {
        let window = self.window(candles.len());
        let (start, end) = (window.start, window.end);
        let shown = &candles[start..end];
        if shown.is_empty() {
            return None;
        }
        let (mut low, mut high) = self.value_range(shown, chart_type);
        if low > high {
            (low, high) = (0.0, 1.0);
        }
        // Every line has to fit, not only the first choice the candles come from.
        let (from, to) = (shown[0].open_time, shown[shown.len() - 1].open_time);
        for line in &self.series {
            for candle in self
                .series_points(line)
                .filter(|candle| (from..=to).contains(&candle.open_time))
            {
                low = low.min(candle.low);
                high = high.max(candle.high);
            }
        }
        // Whatever an overlay script draws has to fit too.
        for value in &self.script_extent.values {
            low = low.min(*value);
            high = high.max(*value);
        }
        // A logarithm takes no price at or below zero. Such a view is drawn linear, the mode
        // staying chosen so the log scale comes back with the data.
        let logarithmic = mode.is_logarithmic() && price_scale::can_draw_logarithmic(low);
        let (minimum, maximum) =
            canvas::price_range(low, high, self.appearance.canvas.margins, logarithmic);
        let PriceRange { minimum, maximum } = self
            .viewport
            .vertical
            .resolve(PriceRange::new(minimum, maximum), logarithmic);
        Some(View {
            start,
            end,
            scale: PriceScale {
                minimum,
                maximum,
                top: plot.top(),
                height: plot.height(),
                logarithmic,
                mode,
                // Percent and Indexed measure from the first bar shown, so a pan moves the
                // reference and the axis follows; the map itself does not change.
                reference: price_scale::reference_from(shown[0].close),
            },
            slot_width: self.slot_width(plot),
            lead: window.lead,
        })
    }

    /// The bars the plot shows out of `len`, and where they sit in its slots.
    fn window(&self, len: usize) -> Window {
        canvas::visible_window(
            len,
            self.viewport.horizontal.visible,
            self.viewport.horizontal.right_offset,
            self.reserved_slots(),
        )
    }

    /// The prices the main series spans over `shown`, which the scale has to reach. Only what is
    /// drawn counts: a line chart follows its one value, not the bars' wicks.
    fn value_range(&self, shown: &[Kline], chart_type: ChartType) -> (f64, f64) {
        let style = &self.appearance.style;
        let reach = |value: fn(&Kline) -> f64| {
            shown
                .iter()
                .map(value)
                .filter(|value| value.is_finite())
                .fold((f64::MAX, f64::MIN), |(low, high), value| {
                    (low.min(value), high.max(value))
                })
        };
        match chart_type.data_class() {
            DataClass::StandardLine => match chart_type {
                ChartType::HlcArea => {
                    let (low, _) = reach(|bar| bar.low);
                    let (_, high) = reach(|bar| bar.high);
                    (low, high)
                }
                _ => {
                    let source = style.value_source();
                    shown
                        .iter()
                        .map(|bar| source.value(bar))
                        .filter(|value| value.is_finite())
                        .fold((f64::MAX, f64::MIN), |(low, high), value| {
                            (low.min(value), high.max(value))
                        })
                }
            },
            _ => {
                let (low, _) = reach(|bar| bar.low);
                let (_, high) = reach(|bar| bar.high);
                (low, high)
            }
        }
    }

    /// The width of one candle slot: the plot shared by the visible candles and the free slots an
    /// overlay script reserves past the last one.
    fn slot_width(&self, plot: Rect) -> f32 {
        plot.width() / (self.viewport.horizontal.visible + self.reserved_slots()) as f32
    }

    /// Applies a pane button. Order, collapse and removal act on the script's own instance, so
    /// there is one list of what is on the chart and in what order.
    fn apply_pane_action(&mut self, id: u64, action: PaneAction) {
        match action {
            PaneAction::MoveUp | PaneAction::MoveDown => {
                let Some(position) = self.pane_ids.iter().position(|pane| *pane == id) else {
                    return;
                };
                let neighbour = if action == PaneAction::MoveUp {
                    position.checked_sub(1)
                } else {
                    Some(position + 1)
                }
                .and_then(|index| self.pane_ids.get(index).copied());
                // Panes are drawn in the order of their scripts, so a pane trades places with its
                // neighbour by taking the neighbour's place in the instance list.
                if let Some(other) = neighbour
                    && let Some(index) = self.instances.iter().position(|i| i.id == other)
                {
                    self.move_pine_instance(id, index);
                    // The next click, before the next frame, sees the new order.
                    if let Some(at) = self.pane_ids.iter().position(|pane| *pane == other) {
                        self.pane_ids.swap(position, at);
                    }
                }
            }
            PaneAction::ToggleCollapse => {
                if let Some(instance) = self.instances.iter_mut().find(|i| i.id == id) {
                    instance.pane_collapsed = !instance.pane_collapsed;
                }
                if self.maximized_pane == Some(id) {
                    self.maximized_pane = None;
                }
            }
            PaneAction::ToggleMaximize => {
                if self.maximized_pane == Some(id) {
                    self.maximized_pane = None;
                } else {
                    // A maximized pane is expanded: that is the state it returns to.
                    if let Some(instance) = self.instances.iter_mut().find(|i| i.id == id) {
                        instance.pane_collapsed = false;
                    }
                    self.maximized_pane = Some(id);
                }
            }
            PaneAction::Remove => {
                self.remove_pine_instance(id);
                if self.maximized_pane == Some(id) {
                    self.maximized_pane = None;
                }
            }
        }
    }

    /// Back to the newest bars at the natural scale, autoscale on.
    pub fn reset_view(&mut self) {
        self.viewport.reset();
        self.gesture = None;
        self.newest_bar = None;
    }

    /// Price scale back to autoscale at the natural zoom, leaving the time window where it is.
    pub fn reset_price_scale(&mut self) {
        self.viewport.vertical.reset();
    }

    /// Header row. Reordering has its own handle so clicking the market title
    /// never moves its icon and text into egui's drag preview layer.
    #[allow(clippy::too_many_arguments)]
    pub fn header(
        &self,
        ui: &mut egui::Ui,
        symbol: &str,
        source: DataSource,
        indicators: &IndicatorConfig,
        candles: &[Kline],
        replaying: bool,
        is_favorite: bool,
        watchlists: &[WatchlistMembership],
        action: &mut ChartCardAction,
        snapshot: &SnapshotMenu,
        drag_id: u64,
    ) {
        let header_width = ui.available_width();
        let price_change = self.visible_change(candles);
        let has_chart_types = !source.is_line_chart();
        let reserved = 164.0
            + if self.paper.trading_enabled {
                125.0
            } else {
                0.0
            }
            + if has_chart_types { 31.0 } else { 0.0 }
            // The camera and the gap after it.
            + 31.0;
        let title_block_width = (header_width - reserved).max(96.0);
        let title_content_width = title_block_width - 18.0;
        let indicator_space = if chart_indicator_strip::has_items(indicators, self, source) {
            60.0
        } else {
            25.0
        };
        let show_replay_badge = replaying && header_width >= 430.0;
        let header_rect =
            Rect::from_min_size(ui.cursor().min, Vec2::new(header_width, HEADER_HEIGHT));
        let header_hovered = ui
            .ctx()
            .pointer_hover_pos()
            .is_some_and(|pos| header_rect.contains(pos));
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), HEADER_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.dnd_drag_source(egui::Id::new(("chart-drag", drag_id)), drag_id, |ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(10.0, 20.0), Sense::hover());
                    let color = theme::text_secondary().gamma_multiply(0.7);
                    for x in [-2.0, 2.0] {
                        for y in [-4.0, 0.0, 4.0] {
                            ui.painter()
                                .circle_filled(rect.center() + Vec2::new(x, y), 1.0, color);
                        }
                    }
                })
                .response
                .on_hover_text("Drag to reorder charts");
                ui.vertical(|ui| {
                    ui.set_width(title_content_width);
                    ui.spacing_mut().item_spacing.y = -3.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let (icon_rect, icon_response) =
                            ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
                        crate::ui::kit::chart_icon_with_label(
                            ui,
                            icon_rect,
                            symbol,
                            self.title.as_deref().unwrap_or(symbol),
                            source,
                            true,
                        );
                        if icon_response.settings_tip(source).clicked() {
                            action.settings_requested = source != DataSource::CoinMarketCap;
                        }
                        let title_max_width = (title_content_width
                            - 26.0
                            - indicator_space
                            - if show_replay_badge { 72.0 } else { 0.0 })
                        .max(20.0);
                        if ui
                            .scope(|ui| {
                                ui.set_max_width(title_max_width);
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(self.display_title(symbol, source))
                                            .size(13.0)
                                            .strong(),
                                    )
                                    .truncate()
                                    .sense(Sense::click()),
                                )
                            })
                            .inner
                            .settings_tip(source)
                            .clicked()
                        {
                            action.settings_requested = source != DataSource::CoinMarketCap;
                        }
                        if show_replay_badge {
                            egui::Frame::new()
                                .fill(theme::REPLAY.gamma_multiply(0.14))
                                .corner_radius(999.0)
                                .inner_margin(egui::Margin::symmetric(7, 2))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 4.0;
                                        let (icon, _) = ui
                                            .allocate_exact_size(Vec2::splat(11.0), Sense::hover());
                                        crate::ui::icons::paint(
                                            ui.painter(),
                                            icon,
                                            crate::ui::icons::Icon::ClockArrow,
                                            theme::REPLAY,
                                        );
                                        ui.label(
                                            egui::RichText::new("Replay")
                                                .size(10.0)
                                                .strong()
                                                .color(theme::REPLAY),
                                        );
                                    });
                                });
                        }
                        // CoinMarketCap's indices aren't price series: nothing to overlay.
                        if source != DataSource::CoinMarketCap {
                            // Salted per chart: the strip keys its hover state by ui id, and
                            // identical ids across charts made one chart's chips light up another's.
                            action.indicator_action = ui
                                .push_id(("indicator-strip", drag_id), |ui| {
                                    chart_indicator_strip::show(
                                        ui,
                                        self,
                                        indicators,
                                        source,
                                        drag_id,
                                        header_hovered,
                                    )
                                })
                                .inner;
                        }
                    });
                    if let Some(last) = candles.last()
                        && ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(if source == DataSource::CoinMarketCap {
                                        // An index on a 0–100 scale, not a price.
                                        format!("{:.0} / 100", last.close)
                                    } else if let Some((label, price)) = self.leading_choice() {
                                        format!("{label} · {}", format::headline(price, true))
                                    } else {
                                        format::headline(last.close, source.is_prediction_market())
                                    })
                                    .size(11.0)
                                    .color(theme::text_secondary()),
                                )
                                .frame(false),
                            )
                            .settings_tip(source)
                            .clicked()
                    {
                        action.settings_requested = source != DataSource::CoinMarketCap;
                    }
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(change) = price_change {
                        let color = if change >= 0.0 {
                            theme::UP
                        } else {
                            theme::DOWN
                        };
                        ui.label(
                            egui::RichText::new(format!(
                                "{} {:.2}%",
                                if change >= 0.0 { "↗" } else { "↘" },
                                change.abs()
                            ))
                            .color(color)
                            .size(11.0)
                            .strong(),
                        );
                    }
                    if self.paper.trading_enabled
                        && let Some(last) = candles.last()
                    {
                        let price_text = self.appearance.price_format().format(last.close);
                        if let Some(side) =
                            paper_chart_trading::quick_trade_buttons(ui, &price_text)
                        {
                            action.quick_trade = Some(side);
                        }
                    }
                    if source == DataSource::CoinMarketCap
                        && icons::button(
                            ui,
                            Icon::Xmark,
                            27.0,
                            theme::text_secondary(),
                            "Remove Chart",
                        )
                        .clicked()
                    {
                        action.remove_requested = true;
                    }
                    if supports_alerts(source)
                        && icons::button(
                            ui,
                            Icon::BellBadge,
                            27.0,
                            theme::text_secondary(),
                            "Create Price Alert",
                        )
                        .clicked()
                    {
                        action.new_alert = true;
                    }
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if let Some(picked) = crate::ui::snapshot_menu::button(ui, snapshot) {
                        action.snapshot = Some(picked);
                    }
                    let favorite_icon = if is_favorite {
                        Icon::StarFill
                    } else {
                        Icon::Star
                    };
                    let favorite_color = if is_favorite {
                        theme::text_primary()
                    } else {
                        theme::text_secondary()
                    };
                    let favorite_tip = if is_favorite {
                        "Remove from Favorites"
                    } else {
                        "Add to Favorites"
                    };
                    // Tighten the gap so the favorite sits close to the alert bell.
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let favorite =
                        icons::button(ui, favorite_icon, 27.0, favorite_color, favorite_tip);
                    favorite.context_menu(|ui| {
                        ui.label(egui::RichText::new("Add to Watchlist").small().weak());
                        for list in watchlists {
                            let mut included = list.contains_market;
                            if ui.checkbox(&mut included, &list.name).changed() {
                                action.watchlist_change = Some((list.id, included));
                                ui.close();
                            }
                        }
                    });
                    if favorite.clicked() {
                        action.toggle_favorite = true;
                    }
                    if has_chart_types {
                        action.chart_type = crate::ui::chart_type_menu::button(
                            ui,
                            self.effective_chart_type(source),
                            source,
                            drag_id,
                        );
                    }
                });
            },
        );
    }

    /// Plot body. Reports the pointer (click + hover) back to the workspace.
    #[allow(clippy::too_many_arguments)]
    /// The chart; when a refresh failed but there is data to show, the last result stays up with
    /// a small warning along the bottom.
    pub fn body(
        &mut self,
        ui: &mut egui::Ui,
        symbol: &str,
        candles: &[Kline],
        loading: bool,
        error: Option<&str>,
        scale_mode: PriceScaleMode,
        source: DataSource,
        indicators: &IndicatorConfig,
        crosshair: Option<&CrosshairInfo>,
        chart_id: u64,
        drawing: &mut DrawingFrame,
        replay: &mut ReplayChartFrame,
    ) -> ChartPointer {
        self.sync_baseline = None;
        let args = BodyArgs {
            symbol,
            candles,
            error,
            scale_mode,
            source,
            indicators,
            crosshair,
            chart_id,
        };
        let _ = loading;
        let pointer = self.body_inner(ui, &args, drawing, replay);
        self.end_sync_frame(source, candles);
        if let (Some(message), false, Some(rect)) = (error, candles.is_empty(), pointer.rect) {
            refresh_warning(ui, rect, message, source);
        }
        pointer
    }

    fn body_inner(
        &mut self,
        ui: &mut egui::Ui,
        args: &BodyArgs,
        drawing: &mut DrawingFrame,
        replay: &mut ReplayChartFrame,
    ) -> ChartPointer {
        let BodyArgs {
            symbol,
            candles,
            scale_mode,
            source,
            indicators,
            crosshair,
            chart_id,
            ..
        } = *args;
        // An open scale menu owns the keyboard: Escape closes it and nothing else, and no draft
        // takes the Enter or Delete meant for one of its rows.
        if scale_menu::is_open(ui.ctx(), chart_id) {
            drawing.keys_blocked = true;
            scale_menu::close_on_escape(ui.ctx(), chart_id);
        }
        let line = source.is_line_chart();
        let mut pointer = ChartPointer::default();
        let available = ui.available_size();
        let size = Vec2::new(available.x.max(1.0), available.y.max(50.0));
        let (response, painter) = ui.allocate_painter(size, Sense::click_and_drag());
        let response = crate::ui::accessibility::label_as(
            response,
            egui::WidgetType::Panel,
            format!(
                "{} {} chart",
                self.display_title(symbol, source),
                source.display_name()
            ),
        );
        let rect = response.rect;
        pointer.rect = Some(rect);
        self.body_size = rect.size();
        let style = CanvasStyle::resolve(&self.appearance.canvas);
        let full_rect = rect;
        canvas_render::paint_background(&painter, rect, style.fill);

        if self.paint_placeholder(ui, &response, &painter, args) {
            return pointer;
        }

        // A script that isn't an overlay (RSI, MACD …) gets a pane under the candles: 30% of the
        // chart's height shared between them, the candles keep the rest and the time axis. On a
        // line chart every script draws over the line instead.
        // A price-based chart type draws bars it constructed from the candles, and the scripts
        // and indicators on it read those same bars; every other type draws the candles as they
        // are. Either way `bars` is what the chart is, and `candles` stays the market.
        let chart_type = if line {
            ChartType::Candles
        } else {
            self.appearance.style.effective_type(source)
        };
        self.drawn_type = chart_type;
        let constructed = self.constructed_series(candles, chart_type);
        let bars: &[Kline] = constructed
            .as_ref()
            .map_or(candles, |built| built.klines.as_slice());
        self.keep_view_on_its_bars(bars);
        self.begin_sync_frame();
        let symbol_info = body::symbol_info(symbol, source, chart_type);
        let source_bar_seconds = body::source_bar_seconds(candles, constructed.is_some());
        let placed = self.pine_outputs(ui.ctx(), &symbol_info, bars, source_bar_seconds);
        let outputs = frame::split_forced_overlays(placed, line);
        self.feed_table(
            bars,
            chart_type,
            line,
            source.provides_volume(),
            indicators,
            &outputs,
        );
        self.refresh_script_extent(&outputs, line, bars.len());
        let data = frame::FrameData {
            symbol,
            source,
            line,
            chart_type,
            scale_mode,
            candles,
            bars,
            constructed: constructed.as_deref(),
            outputs: &outputs,
            indicators,
        };
        let layout = self.frame_layout(&data, full_rect, &painter, &style);
        self.keep_panes_in_step(&layout);
        let plot = layout.plot;
        let pane_rects = &layout.pane_rects;
        let chart_hovered = ui.rect_contains_pointer(full_rect);
        let regions = self.handle_frame_input(ui, &response, &data, &layout, drawing, replay);

        let Some(view) = self.view(bars, plot, scale_mode, chart_type) else {
            return pointer;
        };
        self.near_oldest = view.start <= self.viewport.horizontal.visible * NEAR_OLDEST_SCREENS;
        #[cfg(test)]
        {
            self.probe = Some(Probe {
                plot,
                start: view.start,
                lead: view.lead,
                slot_width: view.slot_width,
                minimum: view.scale.minimum,
                maximum: view.scale.maximum,
            });
        }
        let projection = view.projection(plot, bars, self.appearance.price_format(), ui.ctx());
        pointer.context = self.context_request(
            &response,
            &regions,
            pane_rects,
            &projection,
            &view.scale,
            drawing,
        );
        let mut layer = std::mem::take(&mut self.layer);
        let mut overlays = LiveOverlays {
            ui,
            response: &response,
            card: self,
            replay,
            chart_id,
            chart_hovered,
            pane_actions: Vec::new(),
            alert_menu: None,
            alert_moves: Vec::new(),
            paper: (Vec::new(), None),
            navigation: None,
            scale_mode: None,
        };
        self.paint_frame(
            &painter,
            &data,
            &layout,
            &view,
            &mut layer,
            drawing,
            &frame::Decor {
                chart_id,
                replay_active: overlays.replay.active,
                time_lock: true,
                crosshair,
            },
            &mut overlays,
        );
        let LiveOverlays {
            pane_actions,
            alert_menu,
            alert_moves,
            paper,
            navigation,
            scale_mode,
            ..
        } = overlays;
        self.layer = layer;
        self.apply_overlay_results(
            OverlayResults {
                pane_actions,
                alert_menu,
                alert_moves,
                paper,
                navigation,
                scale_mode,
            },
            bars.len(),
        );
        body::report_hover(&mut pointer, &response, plot, &view, &projection);

        pointer.clicked = response.clicked();
        pointer
    }

    /// "Click a candle" mode: marker at the candle under the pointer, click picks it.
    #[allow(clippy::too_many_arguments)]
    fn handle_start_pick(
        &self,
        ui: &egui::Ui,
        response: &Response,
        painter: &egui::Painter,
        plot: Rect,
        time_of: &dyn Fn(usize) -> i64,
        view: &View,
        replay: &mut ReplayChartFrame,
    ) {
        let Some(position) = response.hover_pos().filter(|point| plot.contains(*point)) else {
            return;
        };
        let Some(index) = replay_overlay::candle_at(
            position.x,
            plot.left(),
            view.slot_width,
            view.lead,
            view.start,
            view.end,
        ) else {
            return;
        };
        let open_time = time_of(index);
        let x = plot.left()
            + view.slot_width * ((index - view.start) as f32 + 0.5)
            + view.slot_width * view.lead;
        replay_overlay::paint_selection(painter, plot, x, open_time);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        if response.clicked() {
            replay.picked_ms = Some(open_time);
        }
    }

    /// The price labels and the grid lines behind the series. One tick list serves both, so a
    /// line is always on its label; the vertical lines stand on the time axis' own ticks. The
    /// grid is drawn from the scale and never feeds back into it.
    fn draw_grid(
        &self,
        painter: &egui::Painter,
        plot: Rect,
        rect: Rect,
        scale: &PriceScale,
        style: &CanvasStyle,
        tick_xs: &[f32],
    ) {
        let budget = canvas::tick_budget(plot.height(), style.price_tick_spacing());
        let AxisTicks { prices, step } = scale.axis_ticks(budget);
        let half = style.tag_half_height();
        for price in prices {
            let y = scale.y(price);
            if !y.is_finite() || y < plot.top() || y > plot.bottom() {
                continue;
            }
            if let Some(grid) = style.horizontal_grid {
                canvas_render::horizontal(painter, (plot.left(), plot.right()), y, grid);
            }
            // A label that would hang over the plot's edge waits for the scale to move.
            if y - half >= plot.top() - half && y + half <= plot.bottom() + half {
                price_text::paint(
                    painter,
                    Pos2::new(rect.right() - 7.0, y),
                    Align2::RIGHT_CENTER,
                    &scale.tick_label(self.appearance.price_format(), price, step),
                    style.price_font(),
                    style.text_color,
                );
            }
        }
        if let Some(grid) = style.vertical_grid {
            for x in tick_xs {
                if *x >= plot.left() && *x <= plot.right() {
                    canvas_render::vertical(painter, *x, (plot.top(), plot.bottom()), grid);
                }
            }
        }
    }

    fn draw_line(
        &self,
        painter: &egui::Painter,
        candles: &[Kline],
        x_for: &dyn Fn(usize) -> f32,
        scale: &PriceScale,
    ) {
        if candles.len() < 2 {
            if let Some(candle) = candles.first() {
                painter.circle_filled(
                    Pos2::new(x_for(0), scale.y(candle.close)),
                    2.0,
                    self.bullish_color(),
                );
            }
            return;
        }
        let points: Vec<Pos2> = candles
            .iter()
            .enumerate()
            .map(|(index, candle)| Pos2::new(x_for(index), scale.y(candle.close)))
            .collect();
        for pair in points.windows(2) {
            let color = if pair[1].y <= pair[0].y {
                self.bullish_color()
            } else {
                self.bearish_color()
            };
            painter.line_segment([pair[0], pair[1]], Stroke::new(1.5, color));
        }
    }

    /// Each choice as a plain line, labelled at its right end.
    fn draw_series(
        &self,
        painter: &egui::Painter,
        plot: Rect,
        shown: &[Kline],
        x_for: &dyn Fn(usize) -> f32,
        scale: &PriceScale,
    ) {
        let (Some(first), Some(last)) = (shown.first(), shown.last()) else {
            return;
        };
        let mut ends: Vec<(f32, &SeriesLine)> = Vec::new();
        for line in &self.series {
            let mut previous: Option<Pos2> = None;
            for candle in self
                .series_points(line)
                .filter(|candle| (first.open_time..=last.open_time).contains(&candle.open_time))
            {
                let index = shown
                    .partition_point(|shown| shown.open_time < candle.open_time)
                    .min(shown.len() - 1);
                let point = Pos2::new(x_for(index), scale.y(candle.close));
                if let Some(previous) = previous {
                    painter.line_segment([previous, point], Stroke::new(1.5, line.color));
                }
                previous = Some(point);
            }
            if let Some(end) = previous {
                ends.push((end.y, line));
            }
        }
        // Top to bottom, pushed apart so neighbouring labels do not stack, and kept in the plot.
        ends.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut top = f32::MIN;
        for (y, line) in ends {
            let y = y
                .max(top + SERIES_LABEL_GAP)
                .clamp(plot.top() + 6.0, plot.bottom() - 6.0);
            top = y;
            let label = painter.layout_no_wrap(
                truncated(&line.label, SERIES_LABEL_CHARS),
                FontId::proportional(9.0),
                theme::text_primary(),
            );
            let text = Rect::from_min_size(
                Pos2::new(
                    plot.right() - 4.0 - label.size().x,
                    y - label.size().y / 2.0,
                ),
                label.size(),
            );
            painter.rect_filled(
                text.expand2(Vec2::new(11.0, 2.0))
                    .translate(Vec2::new(-4.0, 0.0)),
                4.0,
                theme::with_alpha(theme::surface(), 0.85),
            );
            painter.circle_filled(Pos2::new(text.left() - 6.0, y), 3.0, line.color);
            painter.galley(text.min, label, theme::text_primary());
        }
    }

    fn draw_volume(
        &self,
        painter: &egui::Painter,
        plot: Rect,
        x_for: &dyn Fn(usize) -> f32,
        slot_width: f32,
        candles: &[Kline],
    ) {
        let peak = candles
            .iter()
            .map(|candle| candle.volume)
            .fold(0.0, f64::max);
        if peak <= 0.0 {
            return;
        }
        let pane_height = plot.height() * 0.22;
        let pane_bottom = plot.bottom();
        let bar_width = (slot_width * 0.6).clamp(1.0, 8.0);
        for (index, candle) in candles.iter().enumerate() {
            if candle.volume <= 0.0 {
                continue;
            }
            let height = (pane_height * (candle.volume / peak) as f32).max(1.0);
            let x = x_for(index);
            let color = if candle.is_up() {
                self.bullish_color()
            } else {
                self.bearish_color()
            };
            let bar = Rect::from_min_max(
                Pos2::new(x - bar_width / 2.0, pane_bottom - height),
                Pos2::new(x + bar_width / 2.0, pane_bottom),
            );
            painter.rect_filled(bar, 0.0, translucent(color, 89));
        }
    }

    fn draw_ema(
        &self,
        painter: &egui::Painter,
        x_for: &dyn Fn(usize) -> f32,
        scale: &PriceScale,
        values: &[Option<f64>],
    ) {
        let stroke = Stroke::new(1.5, theme::EMA);
        let mut previous: Option<Pos2> = None;
        for (index, value) in values.iter().enumerate() {
            let Some(value) = value else { continue };
            let point = Pos2::new(x_for(index), scale.y(*value));
            if let Some(previous) = previous {
                painter.line_segment([previous, point], stroke);
            }
            previous = Some(point);
        }
    }

    fn draw_bollinger(
        &self,
        painter: &egui::Painter,
        x_for: &dyn Fn(usize) -> f32,
        scale: &PriceScale,
        bands: &BollingerSeries,
    ) {
        let mut upper_points = Vec::new();
        let mut lower_points = Vec::new();
        for index in 0..bands.upper.len() {
            if let (Some(upper), Some(lower)) = (bands.upper[index], bands.lower[index]) {
                let x = x_for(index);
                upper_points.push(Pos2::new(x, scale.y(upper)));
                lower_points.push(Pos2::new(x, scale.y(lower)));
            }
        }
        if upper_points.len() < 2 {
            return;
        }

        // Tint the channel as a strip of quads. One `PathShape` around the whole band is a
        // concave polygon, which egui fills as a fan from its first vertex, so it paints
        // spokes from the far-left point to every other vertex.
        let tint = translucent(theme::BOLLINGER, 26);
        let mut channel = egui::epaint::Mesh::default();
        for (upper, lower) in upper_points.iter().zip(&lower_points) {
            channel.colored_vertex(*upper, tint);
            channel.colored_vertex(*lower, tint);
        }
        for column in 0..(upper_points.len() as u32 - 1) {
            let top = column * 2;
            channel.add_triangle(top, top + 1, top + 2);
            channel.add_triangle(top + 1, top + 3, top + 2);
        }
        painter.add(egui::Shape::mesh(channel));

        let rail = Stroke::new(1.0, translucent(theme::BOLLINGER, 180));
        for pair in upper_points.windows(2) {
            painter.line_segment([pair[0], pair[1]], rail);
        }
        for pair in lower_points.windows(2) {
            painter.line_segment([pair[0], pair[1]], rail);
        }

        let mut previous: Option<Pos2> = None;
        for (index, value) in bands.middle.iter().enumerate() {
            let Some(value) = value else { continue };
            let point = Pos2::new(x_for(index), scale.y(*value));
            if let Some(previous) = previous {
                painter.add(egui::Shape::dashed_line(
                    &[previous, point],
                    Stroke::new(1.0, translucent(theme::BOLLINGER, 115)),
                    4.0,
                    3.0,
                ));
            }
            previous = Some(point);
        }
    }

    fn draw_rsi(
        &self,
        painter: &egui::Painter,
        plot: Rect,
        x_for: &dyn Fn(usize) -> f32,
        values: &[Option<f64>],
    ) {
        if !values.iter().any(|value| value.is_some()) {
            return;
        }
        let pane = Rect::from_min_max(
            Pos2::new(plot.left(), plot.bottom() - plot.height() * 0.28),
            plot.right_bottom(),
        );
        let y_for = |reading: f64| pane.bottom() - (reading as f32 / 100.0) * pane.height();

        for level in [crate::model::RSI_OVERBOUGHT, crate::model::RSI_OVERSOLD] {
            let y = y_for(level);
            painter.add(egui::Shape::dashed_line(
                &[Pos2::new(pane.left(), y), Pos2::new(pane.right(), y)],
                Stroke::new(1.0, theme::grid()),
                4.0,
                3.0,
            ));
        }

        let stroke = Stroke::new(1.0, theme::RSI);
        let mut previous: Option<Pos2> = None;
        for (index, value) in values.iter().enumerate() {
            let Some(value) = value else { continue };
            let point = Pos2::new(x_for(index), y_for(*value));
            if let Some(previous) = previous {
                painter.line_segment([previous, point], stroke);
            }
            previous = Some(point);
        }
    }

    /// Labelled up/down triangles at confirmed Supertrend regime changes. Each marker (triangle
    /// plus "Bullish"/"Bearish" pill) is clamped inside the plot so a flip at a price extreme
    /// stays readable.
    fn draw_trend_flips(
        &self,
        painter: &egui::Painter,
        plot: Rect,
        x_for: &dyn Fn(usize) -> f32,
        scale: &PriceScale,
        candles: &[Kline],
        flips: &[Option<bool>],
    ) {
        let half = TREND_FLIP_MARKER_SIZE / 2.0;
        for (index, flip) in flips.iter().enumerate() {
            let Some(bullish) = *flip else { continue };
            let Some(candle) = candles.get(index) else {
                continue;
            };
            let x = x_for(index);
            let color = if bullish {
                self.bullish_color()
            } else {
                self.bearish_color()
            };
            let galley = painter.layout_no_wrap(
                if bullish { "Bullish" } else { "Bearish" }.to_owned(),
                FontId::proportional(TREND_FLIP_LABEL_FONT_SIZE),
                Color32::WHITE,
            );
            let label_size = galley.size() + TREND_FLIP_LABEL_PAD * 2.0;
            let group_height = TREND_FLIP_MARKER_SIZE + TREND_FLIP_LABEL_GAP + label_size.y;
            let price_y = scale.y(if bullish { candle.low } else { candle.high });
            let raw_center = if bullish {
                price_y + TREND_FLIP_MARKER_GAP + group_height / 2.0
            } else {
                price_y - TREND_FLIP_MARKER_GAP - group_height / 2.0
            };
            let (Some(group_center), Some(label_center_x)) = (
                contained_center(raw_center, plot.top(), plot.bottom(), group_height),
                contained_center(x, plot.left(), plot.right(), label_size.x),
            ) else {
                continue;
            };
            // The triangle sits on the candle's side of the group, the label beyond it.
            let side = if bullish { 1.0 } else { -1.0 };
            let triangle_y = group_center - side * (group_height - TREND_FLIP_MARKER_SIZE) / 2.0;
            let label_y = group_center + side * (group_height - label_size.y) / 2.0;
            let points = if bullish {
                vec![
                    Pos2::new(x, triangle_y - half),
                    Pos2::new(x - half, triangle_y + half),
                    Pos2::new(x + half, triangle_y + half),
                ]
            } else {
                vec![
                    Pos2::new(x, triangle_y + half),
                    Pos2::new(x - half, triangle_y - half),
                    Pos2::new(x + half, triangle_y - half),
                ]
            };
            painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));

            let pill = Rect::from_center_size(Pos2::new(label_center_x, label_y), label_size);
            painter.rect_filled(pill, label_size.y / 2.0, color);
            painter.galley(pill.min + TREND_FLIP_LABEL_PAD, galley, Color32::WHITE);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_last_price(
        &self,
        painter: &egui::Painter,
        plot: Rect,
        rect: Rect,
        scale: &PriceScale,
        candles: &[Kline],
        constructed_last: Option<&Kline>,
        style: &CanvasStyle,
    ) {
        let half = style.tag_half_height();
        // A price-based chart's own last close is not what the market last traded at: it gets its
        // own dashed line and an outlined tag, and the solid tag stays the market's.
        if let Some(bar) = constructed_last {
            let y = scale.y(bar.close);
            if plot.y_range().contains(y) {
                let tint = theme::text_secondary();
                painter.extend(Shape::dashed_line(
                    &[Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
                    Stroke::new(1.0, tint.gamma_multiply(0.7)),
                    3.0,
                    3.0,
                ));
                let tag = Rect::from_min_max(
                    Pos2::new(plot.right() + 3.0, y - half),
                    Pos2::new(rect.right() - 3.0, y + half),
                );
                painter.rect_stroke(tag, 3.0, Stroke::new(1.0, tint), StrokeKind::Inside);
                price_text::paint(
                    painter,
                    tag.center(),
                    Align2::CENTER_CENTER,
                    &scale.tag_label(self.appearance.price_format(), bar.close),
                    style.price_font(),
                    tint,
                );
            }
        }
        let Some(last) = candles.last() else {
            return;
        };
        let y = scale.y(last.close);
        if !plot.y_range().contains(y) {
            return;
        }
        let color = if last.is_up() {
            self.bullish_color()
        } else {
            self.bearish_color()
        };
        painter.line_segment(
            [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
            Stroke::new(1.0, color.gamma_multiply(0.65)),
        );
        let tag = Rect::from_min_max(
            Pos2::new(plot.right() + 3.0, y - half),
            Pos2::new(rect.right() - 3.0, y + half),
        );
        painter.rect_filled(tag, 3.0, color);
        price_text::paint(
            painter,
            tag.center(),
            Align2::CENTER_CENTER,
            &scale.tag_label(self.appearance.price_format(), last.close),
            style.price_font(),
            Color32::WHITE,
        );
    }

    /// The time labels, on the bars the vertical grid stands on.
    #[allow(clippy::too_many_arguments)]
    fn draw_time_axis(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        candles: &[Kline],
        ticks: &[usize],
        x_for: &dyn Fn(usize) -> f32,
        plot_right: f32,
        style: &CanvasStyle,
    ) {
        let y = rect.bottom() - style.time_axis_height() / 2.0;
        for index in ticks {
            let Some(candle) = candles.get(*index) else {
                continue;
            };
            let label = format::timestamp(candle.open_time);
            let x = x_for(*index);
            // A label that would hang over the plot's right edge waits for the chart to move, as a
            // price label does at the plot's top and bottom: it would run into the price axis and
            // the scale button in the corner.
            let galley =
                painter.layout_no_wrap(label[5..].to_owned(), style.time_font(), style.text_color);
            if x + galley.size().x / 2.0 > plot_right {
                continue;
            }
            let at = Pos2::new(x, y) - galley.size() / 2.0;
            painter.galley(at, galley, style.text_color);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_crosshair(
        &self,
        painter: &egui::Painter,
        plot: Rect,
        rect: Rect,
        pane_rects: &[Rect],
        crosshair: &CrosshairInfo,
        chart_id: u64,
        scale: &PriceScale,
        candles: &[Kline],
        slot_width: f32,
        lead: f32,
        show_volume: bool,
        style: &CanvasStyle,
        main_visible: bool,
        info: &CrosshairBars,
    ) {
        let line = style.crosshair;
        let x = plot.left() + plot.width() * crosshair.x_fraction as f32;

        // Vertical line — every chart draws it at the shared column, through the price chart and
        // on through each script pane under it.
        if main_visible {
            canvas_render::vertical(painter, x, (plot.top(), plot.bottom()), line);
        }
        for pane in pane_rects {
            if pane.height() > crate::model::canvas::COLLAPSED_PANE_HEIGHT {
                canvas_render::vertical(painter, x, (pane.top(), pane.bottom()), line);
            }
        }

        // Horizontal line + price pill — only the chart the pointer is over.
        if main_visible && crosshair.owner_id == chart_id {
            let y = scale.y(crosshair.price);
            if plot.y_range().contains(y) {
                canvas_render::horizontal(painter, (plot.left(), plot.right()), y, line);
                let half = style.tag_half_height();
                let price_tag = Rect::from_min_max(
                    Pos2::new(plot.right() + 3.0, y - half),
                    Pos2::new(rect.right() - 3.0, y + half),
                );
                painter.rect_filled(price_tag, 3.0, Color32::from_gray(79));
                price_text::paint(
                    painter,
                    price_tag.center(),
                    Align2::CENTER_CENTER,
                    &scale.tag_label(self.appearance.price_format(), crosshair.price),
                    style.price_font(),
                    Color32::WHITE,
                );
            }
        }

        let raw_index = ((x - plot.left()) / slot_width - lead).floor() as isize;
        let Some((index, candle)) = usize::try_from(raw_index)
            .ok()
            .and_then(|index| candles.get(index).map(|candle| (index, candle)))
        else {
            return;
        };

        let date = format::timestamp(candle.open_time);
        let time_tag_width = 116.0 * style.time_size / 9.0;
        let time_tag = Rect::from_center_size(
            Pos2::new(x, rect.bottom() - style.time_axis_height() / 2.0),
            Vec2::new(time_tag_width, style.time_axis_height() - 6.0),
        )
        .intersect(Rect::from_min_max(
            Pos2::new(plot.left(), plot.bottom()),
            Pos2::new(plot.right(), rect.bottom()),
        ));
        painter.rect_filled(time_tag, 3.0, Color32::from_gray(79));
        painter.text(
            time_tag.center(),
            Align2::CENTER_CENTER,
            date,
            style.time_font(),
            Color32::WHITE,
        );

        if style.ohlc_readout {
            self.draw_readout(painter, plot, info, index, candle, show_volume);
        }

        // Line, area and baseline charts follow one value: ring it on the bar under the pointer.
        if info.chart_type.data_class() == DataClass::StandardLine
            && info.chart_type != ChartType::Columns
        {
            let value = if info.chart_type == ChartType::HlcArea {
                candle.close
            } else {
                self.appearance.style.value_source().value(candle)
            };
            let centre = Pos2::new(
                plot.left() + slot_width * (index as f32 + 0.5) + slot_width * lead,
                scale.y(value),
            );
            if plot.contains(centre) {
                painter.circle(
                    centre,
                    4.0,
                    theme::surface(),
                    Stroke::new(2.0, theme::text_primary()),
                );
            }
        }
    }

    /// The box of values for the bar under the crosshair, in the plot's top-left corner.
    fn draw_readout(
        &self,
        painter: &egui::Painter,
        plot: Rect,
        info: &CrosshairBars,
        index: usize,
        candle: &Kline,
        show_volume: bool,
    ) {
        let color = if candle.is_up() {
            self.bullish_color()
        } else {
            self.bearish_color()
        };
        let constructed = info.synthetic.and_then(|bars| bars.get(index));
        let price_format = self.appearance.price_format();
        let mut items = readout_items(info, constructed, candle, price_format);
        if show_volume {
            items.push((
                "Vol".to_owned(),
                crate::model::watchlist_quote::compact_number(candle.volume),
            ));
        }
        // The market's own price, apart from the constructed bar's.
        if constructed.is_some()
            && let Some(last) = info.source_candles.last()
        {
            items.push(("Mkt".to_owned(), price_format.format(last.close)));
        }
        // The script's own candles and bars under the pointer, apart from the chart's.
        let hovered_from_end = info.shown_len + info.tail;
        for pine in &info.pine {
            for output in pine
                .candles
                .iter()
                .filter(|output| output.display & crate::pine::outputs::DISPLAY_PANE != 0)
            {
                let Some(bar) = crate::ui::pine_geometry::series_index(
                    output.bars.len(),
                    hovered_from_end,
                    index,
                )
                .and_then(|slot| output.bars[slot].as_ref()) else {
                    continue;
                };
                let name = output
                    .title
                    .clone()
                    .filter(|title| !title.is_empty())
                    .unwrap_or_else(|| {
                        if output.bar_style { "Bars" } else { "Candles" }.to_owned()
                    });
                items.push((
                    name,
                    format!(
                        "{} {} {} {}",
                        price_format.format(bar.open),
                        price_format.format(bar.high),
                        price_format.format(bar.low),
                        price_format.format(bar.close)
                    ),
                ));
            }
        }
        paint_ohlc_readout(painter, plot, &items, color);
    }
}

/// What the crosshair needs to name the bar under it: how the chart is drawn and, for a
/// price-based type, the bars it constructed with the candles they came from.
struct CrosshairBars<'a> {
    chart_type: ChartType,
    synthetic: Option<&'a [SyntheticBar]>,
    /// The market's own candles, whole.
    source_candles: &'a [Kline],
    /// The scripts drawing on the chart: their `plotcandle()` and `plotbar()` values show too.
    pine: Vec<&'a VisualOutput>,
    /// Bars in view, and how many lie after them.
    shown_len: usize,
    tail: usize,
}

/// The smallest distance between two neighbouring grid ticks, `0.0` when there are fewer than
/// two: how fine a label must be to tell its neighbours apart.
fn tick_step(ticks: &[f64]) -> f64 {
    let step = ticks
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .filter(|gap| *gap > 0.0)
        .fold(f64::INFINITY, f64::min);
    if step.is_finite() { step } else { 0.0 }
}

/// The crosshair's values for one bar. A candle chart reads open, high, low and close; a
/// constructed bar says what it is first, then what is meaningful for its shape (a Kagi segment
/// has a start and an end, not a body), then which candles made it and whether it can still
/// change.
fn readout_items(
    info: &CrosshairBars,
    constructed: Option<&SyntheticBar>,
    bar: &Kline,
    price_format: PriceFormat,
) -> Vec<(String, String)> {
    let ohlc = || {
        vec![
            ("O".to_owned(), price_format.format(bar.open)),
            ("H".to_owned(), price_format.format(bar.high)),
            ("L".to_owned(), price_format.format(bar.low)),
            ("C".to_owned(), price_format.format(bar.close)),
        ]
    };
    let Some(made) = constructed else {
        return ohlc();
    };
    let mut items = vec![(
        match info.chart_type {
            ChartType::HeikinAshi => "Heikin Ashi",
            ChartType::Renko => "Brick",
            ChartType::LineBreak => "Line",
            ChartType::Range => "Range bar",
            ChartType::Kagi => "Kagi",
            _ => "Point & Figure",
        }
        .to_owned(),
        String::new(),
    )];
    match made.shape {
        BarShape::Ohlc => items.extend(ohlc()),
        BarShape::Kagi { thick, .. } => {
            items.push(("From".to_owned(), price_format.format(made.open)));
            items.push(("To".to_owned(), price_format.format(made.close)));
            items.push((
                "Line".to_owned(),
                if thick { "thick" } else { "thin" }.to_owned(),
            ));
        }
        BarShape::Column { rising, boxes } => {
            items.push(("From".to_owned(), price_format.format(made.open)));
            items.push(("To".to_owned(), price_format.format(made.close)));
            items.push((if rising { "X" } else { "O" }.to_owned(), boxes.to_string()));
        }
    }
    if let (Some(first), Some(last)) = (
        info.source_candles.get(made.first_source),
        info.source_candles.get(made.last_source),
    ) {
        let (first, last) = (
            format::timestamp(first.open_time),
            format::timestamp(last.open_time),
        );
        items.push(("From candle".to_owned(), first[5..].to_owned()));
        if made.last_source != made.first_source {
            items.push(("to".to_owned(), last[5..].to_owned()));
        }
    }
    if !made.confirmed {
        items.push(("Live".to_owned(), String::new()));
    }
    items
}

const TREND_FLIP_MARKER_SIZE: f32 = 7.0;
const TREND_FLIP_MARKER_GAP: f32 = 4.0;
const TREND_FLIP_LABEL_FONT_SIZE: f32 = 10.5;
const TREND_FLIP_LABEL_PAD: Vec2 = Vec2::new(5.0, 2.0);
const TREND_FLIP_LABEL_GAP: f32 = 2.0;

/// `center` moved so an item `length` long stays within `lower..=upper`, or `None` when it does
/// not fit at all.
fn contained_center(center: f32, lower: f32, upper: f32, length: f32) -> Option<f32> {
    let (low, high) = (lower + length / 2.0, upper - length / 2.0);
    (length >= 0.0 && low <= high).then(|| center.clamp(low, high))
}

const READOUT_MARGIN: f32 = 8.0;
const READOUT_PAD: Vec2 = Vec2::new(8.0, 5.0);
const READOUT_GAP: f32 = 12.0;
const READOUT_ROW_GAP: f32 = 2.0;

/// Flows items of the given widths left to right, starting a new row whenever the next one would
/// pass `max_width`. Returns each item's `(row, x offset)` within its row. An item wider than a
/// whole row sits alone on its own row.
fn flow_readout(widths: &[f32], gap: f32, max_width: f32) -> Vec<(usize, f32)> {
    let mut placed = Vec::with_capacity(widths.len());
    let (mut row, mut x) = (0, 0.0_f32);
    for &width in widths {
        if x > 0.0 && x + width > max_width {
            row += 1;
            x = 0.0;
        }
        placed.push((row, x));
        x += width + gap;
    }
    placed
}

/// The crosshair's O/H/L/C(/Vol) box in the plot's top-left corner. Each item is a `(label,
/// value)` pair: the label in the theme's ink (white on dark, black on light), the value in
/// `color`. The box is measured from the text, so it hugs it at any price magnitude, and the
/// items wrap onto more rows when the plot is too narrow for one line — it never spills past
/// the plot.
fn paint_ohlc_readout(
    painter: &egui::Painter,
    plot: Rect,
    items: &[(String, String)],
    color: Color32,
) {
    let font = FontId::monospace(10.0);
    let label_color = theme::pick(Color32::WHITE, Color32::BLACK);
    let galleys: Vec<_> = items
        .iter()
        .map(|(label, value)| {
            let mut job = egui::text::LayoutJob::default();
            job.append(
                &format!("{label} "),
                0.0,
                egui::TextFormat::simple(font.clone(), label_color),
            );
            job.append(value, 0.0, egui::TextFormat::simple(font.clone(), color));
            painter.layout_job(job)
        })
        .collect();
    let widths: Vec<f32> = galleys.iter().map(|galley| galley.size().x).collect();
    let max_width = (plot.width() - 2.0 * (READOUT_MARGIN + READOUT_PAD.x)).max(0.0);
    let placed = flow_readout(&widths, READOUT_GAP, max_width);

    let rows = placed.last().map_or(0, |(row, _)| row + 1);
    let row_height = galleys.iter().map(|g| g.size().y).fold(0.0, f32::max);
    let content_width = placed
        .iter()
        .zip(&widths)
        .map(|((_, x), width)| x + width)
        .fold(0.0, f32::max);
    let content = Vec2::new(
        content_width,
        rows as f32 * row_height + rows.saturating_sub(1) as f32 * READOUT_ROW_GAP,
    );

    let origin = plot.min + Vec2::splat(READOUT_MARGIN) - Vec2::new(0.0, 1.0);
    painter.rect(
        Rect::from_min_size(origin, content + READOUT_PAD * 2.0),
        4.0,
        theme::chrome().gamma_multiply(0.94),
        Stroke::new(1.0, theme::border()),
        StrokeKind::Inside,
    );
    for (galley, (row, x)) in galleys.into_iter().zip(placed) {
        let at = origin + READOUT_PAD + Vec2::new(x, row as f32 * (row_height + READOUT_ROW_GAP));
        painter.galley(at, galley, color);
    }
}

/// A color at the given alpha, premultiplied for egui's blending.
fn translucent(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

mod body;
mod context;
mod frame;
mod live_overlays;
mod navigation;
mod snapshot;
mod sync;
mod table;
mod time_lock;

#[cfg(test)]
mod canvas_tests;

#[cfg(test)]
mod snapshot_tests;

#[cfg(test)]
mod navigation_tests;

#[cfg(test)]
mod sync_tests;

#[cfg(test)]
mod drawing_tests;

#[cfg(test)]
mod price_scale_tests;

#[cfg(test)]
mod tests {
    #[test]
    fn contained_center_keeps_an_item_inside_the_bounds() {
        use super::contained_center;
        assert_eq!(contained_center(50.0, 0.0, 100.0, 20.0), Some(50.0));
        assert_eq!(contained_center(-5.0, 0.0, 100.0, 20.0), Some(10.0));
        assert_eq!(contained_center(500.0, 0.0, 100.0, 20.0), Some(90.0));
        assert_eq!(contained_center(50.0, 0.0, 10.0, 20.0), None);
    }

    #[test]
    fn the_refresh_warning_names_rate_limits() {
        use super::{DataSource, refresh_warning_text};
        let source = DataSource::CoinMarketCap;
        assert_eq!(
            refresh_warning_text("HTTP 429", source, true),
            "CoinMarketCap rate limit reached · showing the last result"
        );
        assert!(refresh_warning_text("timeout", source, true).ends_with("(timeout)"));
        assert!(!refresh_warning_text("timeout", source, false).contains("timeout"));
    }

    use super::*;

    #[test]
    fn preview_inputs_set_before_the_source_still_apply() {
        let mut card = ChartCard::default();
        let mut inputs = HashMap::new();
        inputs.insert("len".to_owned(), InputValue::Int(30));
        card.set_pine_inputs(inputs.clone());
        card.set_pine_source(Some("indicator(\"x\")".into()));
        assert_eq!(card.instances[0].input_values(), inputs);
        // Clearing the script keeps the overrides for the next source.
        card.set_pine_source(None);
        card.set_pine_source(Some("indicator(\"x\")".into()));
        assert_eq!(card.instances[0].input_values(), inputs);
    }

    #[test]
    fn readout_flows_onto_rows_without_exceeding_the_width() {
        let widths = [40.0, 40.0, 40.0, 40.0, 50.0];
        // One row when it all fits.
        assert!(
            flow_readout(&widths, 10.0, 1000.0)
                .iter()
                .all(|(row, _)| *row == 0)
        );
        // Narrow: breaks between items, every row within the limit.
        let placed = flow_readout(&widths, 10.0, 100.0);
        assert_eq!(placed[0], (0, 0.0));
        assert_eq!(placed[1], (0, 50.0));
        assert_eq!(placed[2], (1, 0.0));
        for ((row, x), width) in placed.iter().zip(widths) {
            assert!(*row <= 2 && x + width <= 100.0);
        }
        // An item wider than the limit gets its own row rather than looping.
        assert_eq!(
            flow_readout(&[200.0, 20.0], 10.0, 100.0),
            [(0, 0.0), (1, 0.0)]
        );
        assert!(flow_readout(&[], 10.0, 100.0).is_empty());
    }

    fn candle(open: f64, close: f64) -> Kline {
        Kline {
            open_time: 0,
            close_time: 1,
            open,
            high: open.max(close),
            low: open.min(close),
            close,
            volume: 1.0,
        }
    }

    fn point(open_time: i64, close: f64) -> Kline {
        Kline {
            open_time,
            close_time: open_time + 1,
            open: close,
            high: close,
            low: close,
            close,
            volume: 0.0,
        }
    }

    fn choice(token_id: &str, label: &str) -> PmSeries {
        PmSeries {
            token_id: token_id.into(),
            label: label.into(),
        }
    }

    fn two_lines() -> Vec<SeriesLine> {
        SeriesLine::build(
            &[choice("a", "Cut"), choice("b", "Hold")],
            vec![
                ("b".into(), vec![point(0, 0.5), point(1, 0.9)]),
                ("a".into(), vec![point(0, 0.1), point(1, 0.2)]),
            ],
        )
    }

    #[test]
    fn lines_follow_the_order_of_the_choices_and_skip_empty_histories() {
        let lines = SeriesLine::build(
            &[choice("a", "A"), choice("gone", "Gone"), choice("c", "C")],
            vec![
                ("c".into(), vec![point(0, 0.3)]),
                ("a".into(), vec![point(0, 0.1)]),
                ("gone".into(), Vec::new()),
            ],
        );
        let labels: Vec<&str> = lines.iter().map(|line| line.label.as_str()).collect();
        assert_eq!(labels, ["A", "C"]);
        // The color is the choice's place among those ticked, even when an earlier one is empty.
        assert_eq!(lines[0].color, SERIES_PALETTE[0]);
        assert_eq!(lines[1].color, SERIES_PALETTE[2]);
    }

    #[test]
    fn slots_an_overlay_script_reserves_past_the_last_bar_narrow_every_slot_and_extend_the_scale() {
        let mut card = ChartCard::default();
        card.set_visible_candles(100);
        card.appearance_mut().canvas.margins.right = 0;
        let candles: Vec<Kline> = (0..100).map(|i| point(i, 1.0)).collect();
        let plot = Rect::from_min_size(Pos2::ZERO, Vec2::new(550.0, 200.0));
        let alone = card
            .view(&candles, plot, PriceScaleMode::Regular, ChartType::Candles)
            .expect("view");
        assert_eq!(alone.slot_width, 5.5);
        card.script_extent = OverlayExtent {
            future_bars: 10,
            values: vec![9.0],
        };
        let with_script = card
            .view(&candles, plot, PriceScaleMode::Regular, ChartType::Candles)
            .expect("view");
        assert_eq!(with_script.slot_width, 5.0, "550 px over 100 + 10 slots");
        assert!(
            with_script.scale.maximum > 9.0,
            "a drawn price fits the scale"
        );
    }

    #[test]
    fn the_price_range_covers_every_line_not_only_the_candles() {
        let mut card = ChartCard::default();
        let candles = [point(0, 0.2), point(1, 0.3)];
        let plot = Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 200.0));
        let alone = card
            .view(&candles, plot, PriceScaleMode::Regular, ChartType::Candles)
            .expect("view")
            .scale
            .maximum;
        card.set_series(two_lines());
        let with_lines = card
            .view(&candles, plot, PriceScaleMode::Regular, ChartType::Candles)
            .expect("view")
            .scale
            .maximum;
        assert!(with_lines > 0.9 && with_lines > alone);
    }

    #[test]
    fn replay_cuts_lines_at_the_cutoff_and_the_leader_follows() {
        let mut card = ChartCard::default();
        card.set_series(two_lines());
        assert_eq!(card.leading_choice(), Some(("Hold", 0.9)));
        card.set_series_cutoff(Some(0));
        assert_eq!(card.leading_choice(), Some(("Hold", 0.5)));
        assert_eq!(card.series_points(&card.series[0]).count(), 1);
    }

    #[test]
    fn long_labels_are_cut_with_an_ellipsis() {
        assert_eq!(truncated("Yes", 5), "Yes");
        assert_eq!(truncated("abcdefgh", 5), "abcd…");
    }

    #[test]
    fn a_title_replaces_the_market_id_in_the_header() {
        let mut card = ChartCard::default();
        assert_eq!(card.display_title("12345", DataSource::Polymarket), "12345");
        card.set_title(Some("Fed decision".into()));
        assert_eq!(
            card.display_title("12345", DataSource::Polymarket),
            "Fed decision"
        );
    }

    #[test]
    fn header_change_uses_visible_closes_after_pan_and_zoom() {
        let candles = [
            candle(80.0, 100.0),
            candle(105.0, 110.0),
            candle(95.0, 90.0),
            candle(115.0, 120.0),
        ];
        assert_eq!(
            visible_price_change(&candles, 2, 0.0, 0),
            Some(30.0 / 90.0 * 100.0)
        );
        assert_eq!(
            visible_price_change(&candles, 2, 1.0, 0),
            Some(-20.0 / 110.0 * 100.0)
        );
        assert_eq!(visible_price_change(&candles, 1, 0.0, 0), None);
        assert_eq!(visible_price_change(&[], 2, 0.0, 0), None);
    }

    #[test]
    fn pine_revision_changes_for_intrabar_values_viewports_and_close_transition() {
        let first = candle(10.0, 11.0);
        let mut updated = first.clone();
        updated.close = 12.0;
        let base = run_revision(std::slice::from_ref(&first), Some((0, 0)), 0);
        assert_ne!(base, run_revision(&[updated], Some((0, 0)), 0));
        assert_ne!(base, run_revision(std::slice::from_ref(&first), None, 0));
        assert_ne!(base, run_revision(&[first], Some((0, 0)), 1));
    }

    fn hourly(closes: &[f64]) -> Vec<Kline> {
        closes
            .iter()
            .enumerate()
            .map(|(index, close)| Kline {
                open_time: index as i64 * 3_600_000,
                close_time: (index as i64 + 1) * 3_600_000 - 1,
                open: *close,
                high: *close,
                low: *close,
                close: *close,
                volume: 1.0,
            })
            .collect()
    }

    fn renko_card(amount: f64) -> ChartCard {
        let mut card = ChartCard::default();
        let style = card.chart_style_mut();
        style.chart_type = ChartType::Renko;
        style.renko.size = crate::model::chart_type::SizeRule {
            method: crate::model::chart_type::SizeMethod::Traditional,
            amount,
            ..Default::default()
        };
        card
    }

    #[test]
    fn constructed_bars_are_kept_until_the_candles_or_the_construction_change() {
        let candles = hourly(&[10.0, 12.5, 11.0, 9.0]);
        let mut card = renko_card(1.0);
        let first = card
            .constructed_series(&candles, ChartType::Renko)
            .expect("renko builds bars");
        // Source candles and bricks are different series.
        assert_eq!(first.klines.len(), 4);
        let again = card.constructed_series(&candles, ChartType::Renko).unwrap();
        assert!(Arc::ptr_eq(&first, &again), "a repaint does not rebuild");

        // Colours and visibility are the chart's looks: the bars stay.
        card.chart_style_mut().renko.candle.up_body = Some([1, 2, 3, 255]);
        card.chart_style_mut().renko.candle.wick = false;
        let restyled = card.constructed_series(&candles, ChartType::Renko).unwrap();
        assert!(
            Arc::ptr_eq(&first, &restyled),
            "appearance-only changes keep the bars"
        );

        card.chart_style_mut().renko.size.amount = 2.0;
        let resized = card.constructed_series(&candles, ChartType::Renko).unwrap();
        assert!(!Arc::ptr_eq(&first, &resized));
        assert_ne!(resized.klines.len(), first.klines.len());

        let mut more = candles.clone();
        more.extend(hourly(&[4.0]).into_iter().map(|mut bar| {
            bar.open_time = 4 * 3_600_000;
            bar.close_time = 5 * 3_600_000 - 1;
            bar
        }));
        let grown = card.constructed_series(&more, ChartType::Renko).unwrap();
        assert!(!Arc::ptr_eq(&resized, &grown), "new candles rebuild");

        // Standard types draw the candles as they are and drop the constructed bars.
        assert!(card.constructed_series(&candles, ChartType::Line).is_none());
        assert!(card.constructed.is_none());
    }

    #[test]
    fn a_replay_prefix_builds_the_prefix_of_the_bars_without_the_future() {
        let candles = hourly(&[10.0, 12.5, 11.0, 9.0, 14.0, 6.0]);
        let mut card = renko_card(1.0);
        let full = card.constructed_series(&candles, ChartType::Renko).unwrap();
        let replayed = card
            .constructed_series(&candles[..4], ChartType::Renko)
            .unwrap();
        assert!(replayed.klines.len() < full.klines.len());
        assert_eq!(full.klines[..replayed.klines.len()], replayed.klines[..]);
        // The prefix's bars know nothing past it.
        assert!(replayed.series.bars.iter().all(|bar| bar.last_source < 4));
    }

    #[test]
    fn switching_between_standard_and_price_based_types_resets_the_scroll() {
        let mut card = ChartCard::default();
        card.viewport.horizontal.right_offset = 30.0;
        card.set_chart_type(ChartType::Line);
        let offset = |card: &ChartCard| card.viewport.horizontal.right_offset;
        assert_eq!(offset(&card), 30.0, "a line follows the same candles");
        card.set_chart_type(ChartType::Renko);
        assert_eq!(offset(&card), 0.0);
        card.viewport.horizontal.right_offset = 5.0;
        card.set_chart_type(ChartType::Kagi);
        assert_eq!(offset(&card), 0.0);
    }

    #[test]
    fn a_source_that_cannot_show_a_type_falls_back_to_candles() {
        let mut card = ChartCard::default();
        card.set_chart_type(ChartType::VolumeCandles);
        assert_eq!(
            card.effective_chart_type(DataSource::Binance),
            ChartType::VolumeCandles
        );
        assert_eq!(
            card.effective_chart_type(DataSource::CoinGecko),
            ChartType::Candles
        );
        assert_eq!(
            card.appearance().style.chart_type,
            ChartType::VolumeCandles,
            "the choice is kept for when the market allows it"
        );
    }

    #[test]
    fn shared_colours_follow_the_drawn_types_colours_or_the_saved_palette() {
        let mut card = ChartCard::default();
        let saved = (card.bullish_color(), card.bearish_color());
        assert_eq!(saved.0, Color32::from_rgb(48, 209, 88));
        card.set_chart_type(ChartType::Kagi);
        card.chart_style_mut().kagi.up = Some([10, 20, 30, 255]);
        // Not drawn as Kagi yet: the colours follow what was last drawn.
        assert_eq!(card.bullish_color(), saved.0);
        card.drawn_type = ChartType::Kagi;
        assert_eq!(card.bullish_color(), Color32::from_rgb(10, 20, 30));
        assert_eq!(
            card.bearish_color(),
            saved.1,
            "an unset side keeps the saved palette"
        );
        card.appearance_mut().bearish = [1, 2, 3];
        assert_eq!(card.bearish_color(), Color32::from_rgb(1, 2, 3));
    }
}
