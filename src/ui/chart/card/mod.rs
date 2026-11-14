//! A chart card: one chart's state and how it is drawn. The card's state and the small shared
//! helpers live here; the children split the rest by concern, among them the header (`header`),
//! the plot's frame (`body`, `frame`), the layers under the crosshair (`paint`), the crosshair and
//! its readout (`crosshair`), the price scale (`price_scale`), what a frame shows (`view`) and the
//! Pine scripts on the chart (`scripts`).

use crate::format;
use crate::host::pine::chart::{ChartFrame, ChartRuns, InstanceStatus, PineLogs, RunRevision};
use crate::model::chart;
use crate::model::chart::canvas::{PaneSpec, Window};
use crate::model::chart::kind::{ChartStyle, ChartType, DataClass};
use crate::model::chart::look::Palette;
use crate::model::chart::price_scale::PriceScaleMode;
use crate::model::chart::synthetic::{BarShape, Construction, SyntheticBar, SyntheticSeries};
use crate::model::chart::viewport::{ChartViewport, PriceRange, SavedViewport};
use crate::model::paper::PaperOrderSide;
use crate::model::pine::instance::{PineInstance, VisualOrder};
use crate::model::{
    Anchor, BollingerSeries, ChartAppearance, DataSource, IndicatorConfig, IndicatorSeries, Kline,
    PmSeries, PriceFormat, Timeframe,
};
use crate::pine::compiler::program::ChartTheme;
use crate::pine::compiler::program::{InputValue, SymbolInfo, VisualOutput};
use crate::pine::outputs::style_overrides::StyleRow;
use crate::pine::syntax::token::Diagnostic;
use crate::service::ProviderError;
use crate::ui::alert::lines;
use crate::ui::chart::buttons::{self, PaneAction};
use crate::ui::chart::canvas::{self, CanvasStyle};
use crate::ui::chart::indicator_strip;
use crate::ui::chart::price_text;
use crate::ui::chart::scale_menu;
use crate::ui::chart::series::{self, SeriesFrame};
use crate::ui::drawing::layer::{DrawingFrame, DrawingLayer, Projection, TextMeasure};
use crate::ui::drawing::render::TimeIndex;
use crate::ui::icons::{self, Icon};
use crate::ui::paper::chart_trading::{self, PaperCardState};
use crate::ui::pine::chart_render::{self, CandleStyle, HoverTarget, PineLayer};
use crate::ui::pine::geometry::{OverlayExtent, overlay_extent};
use crate::ui::replay::overlay::{self, ReplayChartFrame};
use crate::ui::theme;
use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Vec2,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

/// The least the price axis is wide, for a chart whose labels are short.
const PRICE_AXIS_WIDTH: f32 = 44.0;
/// The header row's height. The header is drawn over the top of the plot (`set_top_inset`), which
/// reaches the card's top edge.
pub const HEADER_HEIGHT: f32 = 45.0;
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

/// The header change uses the same candle window as the plot, including pan and zoom: the price
/// change (last close less first) and the percent.
fn visible_price_move(
    candles: &[Kline],
    visible_count: usize,
    right_offset: f64,
    reserved: usize,
) -> Option<(f64, f64)> {
    let window =
        chart::canvas::visible_window(candles.len(), visible_count, right_offset, reserved);
    let visible = candles.get(window.start..window.end)?;
    let (Some(first), Some(last)) = (visible.first(), visible.last()) else {
        return None;
    };
    if visible.len() < 2 || first.close == 0.0 {
        return None;
    }
    let amount = last.close - first.close;
    let percent = amount / first.close * 100.0;
    (amount.is_finite() && percent.is_finite()).then_some((amount, percent))
}

/// Half the width of a candle body, matching the real candles the Pine overlay recolours.
fn candle_body_half_width(slot_width: f32) -> f32 {
    (slot_width * 0.34).clamp(1.0, 8.0)
}

/// Signals a chart card emits while it is being drawn. The owning workspace
/// applies them after the frame, so rendering never mutates layout mid-pass.
#[derive(Clone, Debug, Default)]
pub struct ChartCardAction {
    pub focus_requested: bool,
    pub settings_requested: bool,
    /// CoinMarketCap charts have no settings sheet, so removal lives in the header.
    pub remove_requested: bool,
    pub indicator_action: Option<indicator_strip::Action>,
    /// A header SELL / BUY pill was clicked: open the order ticket on that side.
    pub quick_trade: Option<PaperOrderSide>,
    /// A live SELL / BUY pill was pressed: open the live ticket on that side.
    pub live_trade: Option<degenview_brokers::Side>,
    pub toggle_favorite: bool,
    pub watchlist_change: Option<(u64, bool)>,
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
    /// The pointer is on the overlay card, not the plot: the bar stays where it was and only the
    /// readouts are drawn (no lines, no tags).
    pub frozen: bool,
    pub owner_id: u64,
    pub x_fraction: f64,
    pub price: f64,
}

pub use context::ContextRequest;
use crosshair::{CrosshairBars, READOUT_MARGIN, READOUT_PAD, READOUT_ROW_GAP};
use live_overlays::LiveOverlays;
use price_scale::tick_step;
pub(in crate::ui) use price_scale::{AxisTicks, PriceScale};
use scripts::{draw_script_pane, pane_labels};
pub use snapshot::{SnapshotError, SnapshotInput};
#[cfg(test)]
use view::Probe;
use view::View;

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
    /// What a control on the overlay card asked for.
    pub overlay_action: Option<indicator_strip::Action>,
    /// The pointer is on the overlay card, which takes it from the plot.
    pub on_overlay_card: bool,
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
fn refresh_warning_text(error: &ProviderError, source: DataSource, detailed: bool) -> String {
    if error.is_rate_limited() {
        format!(
            "{} rate limit reached · showing the last result",
            source.display_name()
        )
    } else if detailed {
        format!("Couldn't refresh · showing the last result ({error})")
    } else {
        "Couldn't refresh · showing the last result".to_owned()
    }
}

/// A pill along the bottom of the chart saying the latest refresh failed.
fn refresh_warning(ui: &egui::Ui, rect: Rect, error: &ProviderError, source: DataSource) {
    // The detailed wording first, then the short one if the card is too narrow for it.
    let layout = |text: String| {
        ui.painter()
            .layout_no_wrap(text, FontId::proportional(10.5), theme::text_primary())
    };
    let mut galley = layout(refresh_warning_text(error, source, true));
    if galley.size().x + 26.0 > rect.width() - 16.0 {
        galley = layout(refresh_warning_text(error, source, false));
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
    /// How much of the plot's top the header covers: the price scale starts below it, and what
    /// anchors to the plot's top (readouts, tags) sits under it. A snapshot has no header.
    top_inset: f32,
    /// A human-readable title shown instead of an opaque provider id.
    title: Option<String>,
    /// The choices of a multi-choice prediction market; empty for every other chart.
    series: Vec<SeriesLine>,
    /// During replay, the last time the lines may reach.
    series_cutoff: Option<i64>,
    /// The markets compared on this chart: their configuration and candles, handed in by the
    /// workspace. Drawn under the main series on the chart's own time axis.
    overlay: overlays::OverlayState,
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
    sync_baseline: Option<crate::model::chart::viewport::HorizontalViewport>,
    /// What the user's navigation changed this frame, for the workspace to hand to linked charts.
    sync_change: Option<crate::model::chart::sync::HorizontalChange>,
    /// What the last frame drew, for the tests that drive the chart by pointer.
    #[cfg(test)]
    probe: Option<Probe>,
    /// What the overlay scripts need beyond the candles: slots past the last bar and prices the
    /// scale must reach. Set each frame before the view is built.
    script_extent: OverlayExtent,
    layer: DrawingLayer,
    /// Paper-trading inputs and outputs for this card; the workspace fills and drains it.
    pub paper: PaperCardState,
    /// Live trading at a broker: the header pills and the lines of working orders.
    pub live: crate::ui::broker::chart_trading::BrokerCardState,
    /// The Pine scripts applied to this chart, in order. Overlay scripts draw over the candles;
    /// the others share a pane under them.
    instances: Vec<PineInstance>,
    /// The preview's input overrides, kept here so they reach an instance made after they were set
    /// (the Pine Studio sets a script's saved inputs before its source arrives).
    preview_inputs: HashMap<String, InputValue>,
    /// The instances' runs: the latest output, failure and logs of each, run in the background.
    pine: ChartRuns,
    /// What a click on a candle is for while the chart is in "click a candle" mode.
    pick_kind: overlay::PickKind,
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
    /// The bars the logs pane lists, shown as a band. View state, not saved.
    logs_range: Option<(i64, i64)>,
    /// The bar "Scroll to bar" went to and when: a marker shown for a few seconds. View state.
    bar_marker: Option<(i64, std::time::Instant)>,
    /// Where the view was before a scroll the workspace made outside a frame, until the next frame
    /// measures the change for linked charts.
    scroll_from: Option<crate::model::chart::viewport::HorizontalViewport>,
    /// The Zoom In tool: armed state, the rectangle being dragged, the views to zoom out to.
    /// View state, never saved.
    zoom: zoom_rect::ZoomTool,
    /// Bars that arrived at the newest end since this card began, whatever the view showed: what a
    /// stored view is translated by (see [`zoom_rect::ViewStep`]).
    arrivals: u64,
    /// Which series the viewport's offset counts in; moves when that changes meaning.
    series_epoch: u64,
    /// The construction the card last drew, to notice one being edited. View state.
    drawn_series: Option<Construction>,
    /// Table view is open on this chart: build its feed each frame something changes.
    table_wanted: bool,
    table_feed: Option<Arc<crate::model::chart::table::TableFeed>>,
    /// Fingerprint of what `table_feed` was built from.
    table_stamp: u64,
    /// The price alerts to draw as lines (the workspace fills them each frame), and the tag
    /// right-clicked since the last request was read.
    alert_lines: Vec<crate::model::alert::line::AlertLine>,
    alert_menu: Option<lines::LineMenu>,
    /// The market the alert tags are dragged on, and the tags dropped on a new level since the
    /// workspace last took them.
    alert_market: lines::Market,
    alert_moves: Vec<lines::AlertMove>,
    /// The alerts whose pill is open (the rest show only their arrow), and the ones deleted from
    /// their pill since the workspace last took them. Never saved.
    alert_open: std::collections::HashSet<crate::model::alert::AlertId>,
    alert_deletes: Vec<crate::model::alert::AlertId>,
    /// Whether the corner carries the price-scale menu (a script preview has none), and the mode
    /// picked from it since the workspace last took it.
    scale_menu: bool,
    scale_request: Option<PriceScaleMode>,
}

/// A saved bullish/bearish colour as an opaque `Rgba`.
fn opaque_rgba([r, g, b]: [u8; 3]) -> crate::model::chart::kind::Rgba {
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

/// The instance the Pine Studio's single-script preview chart runs.
pub const PREVIEW_INSTANCE: u64 = 0;

impl Default for ChartCard {
    fn default() -> Self {
        Self {
            appearance: ChartAppearance::default(),
            body_size: Vec2::ZERO,
            top_inset: 0.0,
            title: None,
            series: Vec::new(),
            series_cutoff: None,
            overlay: overlays::OverlayState::default(),
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
            live: Default::default(),
            instances: Vec::new(),
            preview_inputs: HashMap::new(),
            pine: ChartRuns::default(),
            pick_kind: overlay::PickKind::default(),
            constructed: None,
            drawn_type: ChartType::Candles,
            timeframe: None,
            maximized_pane: None,
            pane_ids: Vec::new(),
            time_lock: None,
            bar_marker: None,
            logs_range: None,
            scroll_from: None,
            zoom: zoom_rect::ZoomTool::default(),
            arrivals: 0,
            series_epoch: 0,
            drawn_series: None,
            table_wanted: false,
            table_feed: None,
            table_stamp: 0,
            alert_lines: Vec::new(),
            alert_menu: None,
            alert_market: lines::Market::default(),
            alert_moves: Vec::new(),
            alert_open: std::collections::HashSet::new(),
            alert_deletes: Vec::new(),
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
        scope: crate::model::chart::template::ApplyScope,
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
        self.visible_move(candles).map(|(_, percent)| percent)
    }

    /// The change across the candles on screen as (price, percent).
    pub fn visible_move(&self, candles: &[Kline]) -> Option<(f64, f64)> {
        visible_price_move(
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
            self.new_series();
        }
    }

    /// The chart type drawn for `source`: the selected one, else candles when the source cannot
    /// show it.
    pub fn effective_chart_type(&self, source: DataSource) -> ChartType {
        self.appearance.style.effective_type(source)
    }

    /// Says what the next click on a candle is for, while the chart is picking one.
    pub fn set_pick_kind(&mut self, kind: overlay::PickKind) {
        self.pick_kind = kind;
    }

    /// The `syminfo` a script reads on a chart of plain candles for `symbol`.
    pub(crate) fn pine_symbol_info(
        symbol: &str,
        source: DataSource,
    ) -> crate::pine::compiler::program::SymbolInfo {
        body::symbol_info(symbol, source, ChartType::Candles)
    }

    /// How the chart builds its bars for `source`: the construction of a price-based type, `None`
    /// for one that draws the market's candles. What a script alert on this chart runs over.
    pub fn construction(&self, source: DataSource) -> Option<Construction> {
        let mut style = self.appearance.style;
        style.chart_type = self.effective_chart_type(source);
        Construction::of(&style)
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

    /// The bars the chart draws and its scripts and indicators read when its type builds its own
    /// (Heikin Ashi, Renko, Kagi, Line Break, Point & Figure, Range) out of `candles`; `None`
    /// when it draws the candles themselves. Built by the code the renderer uses.
    pub fn constructed_bars(&self, candles: &[Kline], source: DataSource) -> Option<Vec<Kline>> {
        if source.is_line_chart() {
            return None;
        }
        let chart_type = self.appearance.style.effective_type(source);
        self.constructed_for(candles, chart_type)
            .map(|built| built.klines.clone())
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
        let revision = RunRevision::of(candles, None, now_ms);
        if let Some(built) = &self.constructed
            && built.construction == construction
            && built.revision == revision
        {
            return Some(Arc::clone(built));
        }
        let series = chart::synthetic::build_with(construction, candles, now_ms);
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
        (series::color(up), series::color(down))
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
                count.clamp(5, crate::model::chart::viewport::MAX_RESTORED_VISIBLE);
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
}

/// Whether a chart on `source` can raise price alerts: markets with a price quote feed.
pub fn supports_alerts(source: DataSource) -> bool {
    source != DataSource::CoinMarketCap && !source.is_prediction_market()
}

/// Whether a table's cell was selected on some chart in the last frame, and forgets it: while one
/// is, the keyboard's Copy, Cut and Paste are the cell's and the workspace leaves them alone.
pub fn take_table_cell_selected(ctx: &egui::Context) -> bool {
    ctx.data_mut(|data| {
        data.remove_temp::<bool>(egui::Id::new(crate::ui::drawing::layer::TABLE_CELL_FLAG))
            .unwrap_or(false)
    })
}

impl ChartCard {
    /// The drawings whose note is open (a pin's), so a snapshot can show them as they stand.
    pub fn open_notes(&self) -> Vec<u64> {
        self.layer.open_notes()
    }
}

mod bar_marker;
mod body;
mod context;
mod crosshair;
mod frame;
mod header;
mod live_overlays;
mod navigation;
mod overlay_card;
mod overlays;
mod paint;
mod price_scale;
mod scripts;
mod snapshot;
mod sync;
mod table;
mod view;
mod zoom_rect;
pub use bar_marker::Scrolled;
pub use zoom_rect::{ViewMove, ViewStep};
mod logs_range;
mod time_lock;

#[cfg(test)]
mod canvas_tests;
#[cfg(test)]
mod overlay_tests;

#[cfg(test)]
mod snapshot_tests;

#[cfg(test)]
mod navigation_tests;

#[cfg(test)]
mod sync_tests;
#[cfg(test)]
mod tab_link_tests;

#[cfg(test)]
mod drawing_tests;

#[cfg(test)]
mod price_scale_tests;

#[cfg(test)]
mod market_switch_tests;

#[cfg(test)]
mod tests;
