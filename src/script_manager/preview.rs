//! The preview chart: the market and timeframe the script is tried on, the candle fetch and
//! refresh, the debounced apply and the stale-output handling. Ported from
//! `ScriptPreviewViewModel`, `ScriptPreviewPane` and `ScriptPreviewMarketPicker`.
//!
//! The preview never raises alerts: it only ever calls `ChartCard::set_pine_source`, which runs
//! the script for drawing, never the alert path.

use super::Cmd;
use super::inputs::{self, InputEdit, InputValues, InputsStore};
use super::model::ScriptType;
use super::registry::LibraryRegistry;
use super::workspace::PreviewInput;
use crate::format;
use crate::model::price_scale::PriceScaleMode;
use crate::model::{DataSource, IndicatorConfig, Kline, Timeframe};
use crate::pine::compiler;
use crate::pine::program::{InputSchema, VisualOutput};
use crate::pine::token::{DiagnosticSeverity, Limits};
use crate::service::{self, SearchResult};
use crate::store::SharedStore;
use crate::ui::icons::{self, Icon};
use crate::ui::theme;
use crate::ui::{ChartCard, DrawingFrame, PREVIEW_INSTANCE, ReplayChartFrame};
use crate::ui::{add_chart_sheet, kit, source_logo};
use eframe::egui::{
    self, Align, Align2, Color32, CursorIcon, FontId, Id, Layout, Margin, Rect, RichText, Sense,
    Stroke, Ui, Vec2, pos2, vec2,
};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const MARKET_KEY: &str = "scriptPreview.market";
const RECENTS_KEY: &str = "scriptPreview.recents";
const TIMEFRAME_KEY: &str = "scriptPreview.timeRange";
const CANDLE_KEY: &str = "scriptPreview.candleCount";
const YZOOM_KEY: &str = "scriptPreview.yZoom";
const RECENTS_LIMIT: usize = 5;
/// Quiet time after the last keystroke before the chart takes the new source.
const APPLY_DELAY: Duration = Duration::from_millis(350);
const REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const SEARCH_DELAY: Duration = Duration::from_millis(300);
const CANDLE_LIMIT: u32 = 500;
const HEADER_PAD: (f32, f32) = (10.0, 8.0);
/// The market button's artwork, source badge included.
const HEADER_LOGO: f32 = 26.0;

/// A market the preview can chart: crypto and stock sources only.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreviewMarket {
    /// The symbol as the chart takes it (a search result's `full_symbol`).
    pub ticker: String,
    pub source: DataSource,
}

impl PreviewMarket {
    pub fn default_market() -> Self {
        Self {
            ticker: "BTCUSDT".to_owned(),
            source: DataSource::Binance,
        }
    }

    /// Exchanges, aggregators, DEX pairs and stocks; not prediction markets (no candles a
    /// script runs on) or the CoinMarketCap indices.
    pub fn is_supported(source: DataSource) -> bool {
        matches!(
            source,
            DataSource::Binance
                | DataSource::Coinbase
                | DataSource::CoinGecko
                | DataSource::DexScreener
                | DataSource::Alpaca
        )
    }

    fn title(&self) -> String {
        self.source.display_symbol(&self.ticker)
    }
}

/// The crypto sources the picker searches, in order.
const CRYPTO_SOURCES: [DataSource; 4] = [
    DataSource::Binance,
    DataSource::Coinbase,
    DataSource::CoinGecko,
    DataSource::DexScreener,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AssetType {
    Crypto,
    Stock,
}

struct Picker {
    asset: AssetType,
    crypto_text: String,
    stock_text: String,
    last_edit: Option<Instant>,
    results: Vec<SearchResult>,
    searching: bool,
    inbox: Option<Receiver<(u64, Vec<SearchResult>)>>,
    generation: u64,
    needs_focus: bool,
    /// The keyboard-highlighted row of the results (or of the recents).
    cursor: usize,
}

impl Picker {
    fn new(current: &PreviewMarket) -> Self {
        Self {
            asset: if current.source == DataSource::Alpaca {
                AssetType::Stock
            } else {
                AssetType::Crypto
            },
            crypto_text: String::new(),
            stock_text: String::new(),
            last_edit: None,
            results: Vec::new(),
            searching: false,
            inbox: None,
            generation: 0,
            needs_focus: true,
            cursor: 0,
        }
    }

    fn text(&mut self) -> &mut String {
        match self.asset {
            AssetType::Crypto => &mut self.crypto_text,
            AssetType::Stock => &mut self.stock_text,
        }
    }
}

pub struct PreviewState {
    shared: Option<SharedStore>,
    market: PreviewMarket,
    recents: Vec<PreviewMarket>,
    timeframe: Timeframe,
    candles: Vec<Kline>,
    loading: bool,
    error: Option<String>,
    inbox: Option<Receiver<Result<Vec<Kline>, String>>>,
    last_fetch: Option<Instant>,
    /// What the candles on screen were fetched for.
    fetched_for: Option<(PreviewMarket, Timeframe)>,
    card: ChartCard,
    script_id: Option<String>,
    script_type: Option<ScriptType>,
    /// The editor text last seen, and when it last changed.
    latest_source: Option<String>,
    changed_at: Option<Instant>,
    applied_source: Option<String>,
    stale: bool,
    compile_failed: bool,
    drawn_this_frame: bool,
    picker: Option<Picker>,
    saved_zoom: (usize, f64),
    script_title: Option<String>,
    pub(super) input_schema: InputSchema,
    pub(super) input_values: InputValues,
    inputs_store: InputsStore,
    inputs_dirty_at: Option<Instant>,
    /// The picker asked to open Settings (Stocks need Alpaca keys).
    pending_settings: bool,
}

impl PreviewState {
    pub fn load(shared: &SharedStore) -> Self {
        let read = |f: &dyn Fn(&crate::store::Store) -> Option<PreviewMarket>| {
            shared.lock().ok().and_then(|store| f(&store))
        };
        let market = read(&|s| s.setting::<PreviewMarket>(MARKET_KEY))
            .filter(|m| PreviewMarket::is_supported(m.source))
            .unwrap_or_else(PreviewMarket::default_market);
        let (recents, timeframe, candles, y_zoom) = shared
            .lock()
            .map(|store| {
                (
                    store
                        .setting::<Vec<PreviewMarket>>(RECENTS_KEY)
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|m| PreviewMarket::is_supported(m.source))
                        .collect::<Vec<_>>(),
                    store
                        .setting::<Timeframe>(TIMEFRAME_KEY)
                        .unwrap_or(Timeframe::Day),
                    store.setting::<usize>(CANDLE_KEY),
                    store.setting::<f64>(YZOOM_KEY),
                )
            })
            .unwrap_or_else(|_| (Vec::new(), Timeframe::Day, None, None));
        let mut card = ChartCard::default();
        // A script preview has no price-scale menu: its scale is always plain.
        card.set_scale_menu(false);
        if let Some(count) = candles {
            card.set_visible_candles(count);
        }
        if let Some(zoom) = y_zoom.filter(|z| z.is_finite()) {
            card.set_y_zoom(zoom.clamp(0.25, 20.0));
        }
        let saved_zoom = (card.visible_candles(), card.y_zoom());
        Self {
            shared: Some(shared.clone()),
            market,
            recents,
            timeframe,
            candles: Vec::new(),
            loading: false,
            error: None,
            inbox: None,
            last_fetch: None,
            fetched_for: None,
            card,
            script_id: None,
            script_type: None,
            latest_source: None,
            changed_at: None,
            applied_source: None,
            stale: false,
            compile_failed: false,
            drawn_this_frame: false,
            picker: None,
            saved_zoom,
            script_title: None,
            input_schema: InputSchema::default(),
            input_values: InputValues::new(),
            inputs_store: InputsStore::open(inputs_path()),
            inputs_dirty_at: None,
            pending_settings: false,
        }
    }

    /// Points the preview at another script; its source arrives through `source_changed`.
    pub fn script_changed(&mut self, id: &str) {
        if self.script_id.as_deref() == Some(id) {
            return;
        }
        self.script_id = Some(id.to_owned());
        self.latest_source = None;
        self.changed_at = None;
        self.applied_source = None;
        self.stale = false;
        self.compile_failed = false;
        self.card.set_pine_source(None);
        self.flush_inputs();
        self.input_schema = InputSchema::default();
        self.input_values = self.inputs_store.inputs(id);
        self.card.set_pine_inputs(self.input_values.clone());
    }

    pub fn tick(&mut self, ctx: &egui::Context) {
        self.receive_candles();
        // Persist the scroll zoom and price zoom once they settle.
        let zoom = (self.card.visible_candles(), self.card.y_zoom());
        if zoom != self.saved_zoom {
            self.saved_zoom = zoom;
            self.save(CANDLE_KEY, &zoom.0);
            self.save(YZOOM_KEY, &zoom.1);
        }
        if self.changed_at.is_some() {
            ctx.request_repaint_after(APPLY_DELAY);
        }
        if self
            .inputs_dirty_at
            .is_some_and(|at| at.elapsed() >= Duration::from_millis(500))
        {
            self.flush_inputs();
        } else if self.inputs_dirty_at.is_some() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        self.drawn_this_frame = false;
    }

    fn save<T: Serialize>(&self, key: &str, value: &T) {
        if let Some(shared) = &self.shared
            && let Ok(store) = shared.lock()
        {
            let _ = store.set_setting(key, value);
        }
    }

    pub fn status_text(&self) -> String {
        if self.script_type == Some(ScriptType::Library) {
            return "Libraries export functions — they don't draw on a chart.".to_owned();
        }
        if self.runtime_error().is_some() {
            return if self.card.has_valid_pine_output() {
                "Runtime failed — last valid output remains active".to_owned()
            } else {
                "Runtime failed".to_owned()
            };
        }
        if self.compile_failed {
            return if self.applied_source.is_some() {
                "Compile failed — last valid output remains active".to_owned()
            } else {
                "Compile failed".to_owned()
            };
        }
        let Some(applied) = &self.applied_source else {
            return "No script applied".to_owned();
        };
        if self.loading && self.candles.is_empty() {
            return "Evaluating…".to_owned();
        }
        let _ = applied;
        let live = if self.candles.is_empty() {
            ""
        } else {
            " · live"
        };
        format!(
            "Applied {} · {} bars{live}",
            self.title_hint(),
            self.candles.len()
        )
    }

    fn title_hint(&self) -> String {
        self.script_title.clone().unwrap_or_default()
    }

    // MARK: Source

    fn source_changed(&mut self, input: &PreviewInput) {
        self.script_type = Some(input.script_type);
        self.script_title = Some(input.name.clone());
        if self.latest_source.as_deref() == Some(input.source.as_str()) {
            return;
        }
        let first = self.latest_source.is_none();
        self.latest_source = Some(input.source.clone());
        self.changed_at = Some(if first {
            // The first source after opening a script applies at once.
            Instant::now() - APPLY_DELAY
        } else {
            Instant::now()
        });
    }

    /// Applies the latest source once typing pauses. Source that doesn't compile leaves the last
    /// working output on screen and shows the stale banner.
    fn maybe_apply(&mut self, input: &PreviewInput) {
        let Some(at) = self.changed_at else {
            return;
        };
        // The diagnostics below decide whether the source may run; wait until they describe it.
        if at.elapsed() < APPLY_DELAY || input.compile_pending {
            return;
        }
        self.changed_at = None;
        if input.script_type == ScriptType::Library {
            self.card.set_pine_source(None);
            self.applied_source = None;
            return;
        }
        let has_errors = input
            .diagnostics
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error);
        if has_errors {
            self.compile_failed = true;
            self.stale = self.applied_source.is_some();
            return;
        }
        self.compile_failed = false;
        self.stale = false;
        if self.applied_source.as_deref() != Some(input.source.as_str()) {
            // The script's declared inputs, with the user's overrides kept where they still fit.
            let program = compiler::compile(&input.source, Limits::default(), &LibraryRegistry);
            self.input_values = inputs::prune_values(&self.input_values, &program.input_schema);
            self.input_schema = program.input_schema;
            self.card.set_pine_inputs(self.input_values.clone());
            self.card.set_pine_source(Some(input.source.clone()));
            self.applied_source = Some(input.source.clone());
        }
    }

    /// Applies a card's change: a value equal to the default is no override at all.
    pub(super) fn apply_input_edit(&mut self, edit: InputEdit) {
        inputs::apply_edit(&mut self.input_values, &self.input_schema, edit);
        self.card.set_pine_inputs(self.input_values.clone());
        self.inputs_dirty_at = Some(Instant::now());
    }

    /// Brings the Swift app's saved preview inputs across (script id → values); a script that
    /// already has values here keeps them.
    pub(super) fn import_inputs(&mut self, imported: HashMap<String, InputValues>) {
        self.inputs_store.import_missing(imported);
    }

    fn flush_inputs(&mut self) {
        if self.inputs_dirty_at.take().is_some()
            && let Some(id) = self.script_id.clone()
        {
            self.inputs_store.set_inputs(&id, &self.input_values);
        }
    }

    /// The runtime failure of the script on the chart, if its last run stopped early.
    pub(super) fn runtime_error(&self) -> Option<&crate::pine::token::Diagnostic> {
        self.card.pine_error()
    }

    /// The messages the preview's script wrote with `log.*()` in its current run.
    pub(super) fn logs(&self) -> Option<crate::ui::PineLogs<'_>> {
        self.card.pine_logs(PREVIEW_INSTANCE)
    }

    pub(super) fn clear_logs(&mut self) {
        self.card.clear_pine_logs(PREVIEW_INSTANCE);
    }

    pub(super) fn logging(&self) -> bool {
        self.card.pine_logging(PREVIEW_INSTANCE)
    }

    pub(super) fn set_logging(&mut self, enabled: bool) {
        self.card.set_pine_logging(PREVIEW_INSTANCE, enabled);
    }

    /// Scrolls the preview chart to the bar holding `time_ms`.
    pub(super) fn scroll_to_time(&mut self, time_ms: i64) -> crate::ui::Scrolled {
        self.card.scroll_to_time(time_ms, &self.candles)
    }

    /// The last run's output, for the Report tab.
    pub(super) fn visual(&self) -> Option<&VisualOutput> {
        self.card.pine_visual()
    }

    // MARK: Candles

    fn want_fetch(&self) -> bool {
        let key = (self.market.clone(), self.timeframe);
        self.fetched_for.as_ref() != Some(&key) && self.inbox.is_none()
    }

    fn fetch(&mut self, ctx: &egui::Context) {
        // Unit tests draw the pane headlessly and must not touch the network.
        if cfg!(test) {
            self.fetched_for = Some((self.market.clone(), self.timeframe));
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let market = self.market.clone();
        let interval = self.timeframe.interval();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let result =
                service::fetch_klines(market.source, &market.ticker, interval, CANDLE_LIMIT);
            let _ = sender.send(result);
            repaint.request_repaint();
        });
        self.inbox = Some(receiver);
        self.loading = true;
        self.last_fetch = Some(Instant::now());
        self.fetched_for = Some((self.market.clone(), self.timeframe));
    }

    fn receive_candles(&mut self) {
        let Some(receiver) = &self.inbox else {
            return;
        };
        let Ok(result) = receiver.try_recv() else {
            return;
        };
        self.inbox = None;
        self.loading = false;
        match result {
            Ok(candles) => {
                self.candles = candles;
                self.error = None;
            }
            Err(message) => self.error = Some(message),
        }
    }

    fn select_market(&mut self, market: PreviewMarket) {
        if market != self.market {
            self.candles.clear();
            self.market = market.clone();
            self.fetched_for = None;
            self.error = None;
        }
        self.recents.retain(|m| m != &market);
        self.recents.insert(0, market);
        self.recents.truncate(RECENTS_LIMIT);
        self.save(MARKET_KEY, &self.market);
        self.save(RECENTS_KEY, &self.recents);
    }

    fn select_timeframe(&mut self, timeframe: Timeframe) {
        if timeframe != self.timeframe {
            self.timeframe = timeframe;
            self.candles.clear();
            self.fetched_for = None;
            self.save(TIMEFRAME_KEY, &timeframe);
        }
    }

    /// The last close and the change across the candles on screen. The warm-up candles off the
    /// left edge are not counted: they would report a move nobody can see.
    fn price_line(&self) -> Option<(String, f64)> {
        let last = self.candles.last()?;
        let change = self.card.visible_change(&self.candles)?;
        Some((format::price(last.close), change))
    }
}

fn inputs_path() -> Option<std::path::PathBuf> {
    if cfg!(test) {
        return None;
    }
    crate::store::data_dir().map(|dir| dir.join("script_preview_inputs.json"))
}

/// The header over the chart; returns the y where the chart area starts.
pub fn header(ui: &mut Ui, preview: &mut PreviewState, full: Rect) -> f32 {
    preview.drawn_this_frame = true;
    let height = 52.0;
    let rect = Rect::from_min_size(full.min, vec2(full.width(), height));
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(HEADER_PAD.0, HEADER_PAD.1)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    market_button(&mut inner, preview);
    inner.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.add_space(0.0);
        timeframe_picker(ui, preview);
    });
    full.min.y + height
}

fn market_button(ui: &mut Ui, preview: &mut PreviewState) {
    let title = preview.market.title();
    let price = preview.price_line();
    let title_galley = ui.painter().layout_no_wrap(
        title.clone(),
        FontId::proportional(11.0),
        theme::text_primary(),
    );
    let price_width = price.as_ref().map_or(0.0, |(p, pct)| {
        ui.painter()
            .layout_no_wrap(
                format!("{p}  {}", signed_percent(*pct)),
                FontId::monospace(10.0),
                theme::text_secondary(),
            )
            .size()
            .x
    });
    let source_name = preview.market.source.display_name();
    let width = HEADER_LOGO
        + 8.0
        + title_galley
            .size()
            .x
            .max(price_width)
            .max(if price.is_none() { 60.0 } else { 0.0 })
        + 8.0
        + 10.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width + 8.0, 34.0), Sense::click());
    let response = response.on_hover_text("Choose the market to test on");
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        ui.painter().rect_filled(rect, 6.0, theme::ink(10));
    }
    let logo = Rect::from_center_size(
        pos2(rect.min.x + 4.0 + HEADER_LOGO / 2.0, rect.center().y),
        Vec2::splat(HEADER_LOGO),
    );
    kit::chart_icon(
        ui,
        logo,
        &preview.market.ticker,
        preview.market.source,
        true,
    );
    let painter = ui.painter();
    let text_x = logo.max.x + 8.0;
    painter.galley(
        pos2(text_x, rect.min.y + 2.0),
        title_galley.clone(),
        theme::text_primary(),
    );
    icons::paint(
        painter,
        Rect::from_center_size(
            pos2(text_x + title_galley.size().x + 9.0, rect.min.y + 9.0),
            Vec2::splat(8.0),
        ),
        Icon::ChevronUpDown,
        theme::text_secondary(),
    );
    match &price {
        Some((p, pct)) => {
            let first =
                painter.layout_no_wrap(p.clone(), FontId::monospace(10.0), theme::text_secondary());
            let first_width = first.size().x;
            painter.galley(
                pos2(text_x, rect.max.y - 15.0),
                first,
                theme::text_secondary(),
            );
            painter.text(
                pos2(text_x + first_width + 6.0, rect.max.y - 15.0),
                Align2::LEFT_TOP,
                signed_percent(*pct),
                FontId::monospace(10.0),
                if *pct >= 0.0 {
                    theme::syntax::variable()
                } else {
                    theme::syntax::string()
                },
            );
        }
        None => {
            painter.text(
                pos2(text_x, rect.max.y - 15.0),
                Align2::LEFT_TOP,
                source_name,
                FontId::proportional(10.0),
                theme::text_secondary(),
            );
        }
    }

    // The button toggles; a press anywhere else outside the popup dismisses it (below).
    let toggled_closed = response.clicked() && preview.picker.is_some();
    if toggled_closed {
        preview.picker = None;
        egui::Popup::close_id(ui.ctx(), Id::new("script-preview-picker"));
    } else if response.clicked() {
        preview.picker = Some(Picker::new(&preview.market));
    }
    if preview.picker.is_some() {
        let popup = egui::Popup::from_response(&response)
            .id(Id::new("script-preview-picker"))
            .open_memory(Some(egui::SetOpenCommand::Bool(true)))
            .anchor(egui::PopupAnchor::ParentRect(response.rect))
            .align(egui::RectAlign::BOTTOM_START)
            .gap(6.0)
            .frame(kit::menu_frame().inner_margin(Margin::same(10)))
            .close_behavior(egui::PopupCloseBehavior::IgnoreClicks);
        let mut selected = None;
        let mut close = false;
        let opened = popup.show(|ui| {
            ui.set_width(380.0);
            selected = picker_ui(ui, preview);
        });
        match &opened {
            None => close = true,
            Some(inner) => {
                // A press outside the popup (and not on its button, which toggles) closes it.
                let outside = ui.input(|i| {
                    i.pointer.any_pressed()
                        && i.pointer.interact_pos().is_some_and(|pos| {
                            !inner.response.rect.contains(pos) && !response.rect.contains(pos)
                        })
                });
                if outside {
                    close = true;
                }
            }
        }
        if let Some(market) = selected {
            preview.select_market(market);
            close = true;
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        if close {
            preview.picker = None;
            egui::Popup::close_id(ui.ctx(), Id::new("script-preview-picker"));
        }
    }
}

fn signed_percent(value: f64) -> String {
    let sign = if value >= 0.0 { "+" } else { "−" };
    format!("{sign}{:.2}%", value.abs())
}

fn timeframe_picker(ui: &mut Ui, preview: &mut PreviewState) {
    // A segmented control: one rounded track, the selected segment raised.
    let font = FontId::proportional(11.0);
    let labels: Vec<&str> = Timeframe::ALL.iter().map(|t| t.label()).collect();
    let widths: Vec<f32> = labels
        .iter()
        .map(|l| {
            ui.painter()
                .layout_no_wrap((*l).to_owned(), font.clone(), Color32::WHITE)
                .size()
                .x
                + 20.0
        })
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + 4.0;
    let (rect, _) = ui.allocate_exact_size(vec2(total, 24.0), Sense::hover());
    ui.painter().rect_filled(rect, 6.0, theme::quaternary(1.0));
    let mut x = rect.min.x + 2.0;
    for (index, timeframe) in Timeframe::ALL.iter().enumerate() {
        let segment = Rect::from_min_size(pos2(x, rect.min.y + 2.0), vec2(widths[index], 20.0));
        let response = ui.interact(
            segment,
            Id::new(("preview-timeframe", index)),
            Sense::click(),
        );
        let selected = *timeframe == preview.timeframe;
        if selected {
            ui.painter()
                .rect_filled(segment, 5.0, theme::selected_plate());
        } else if response.hovered() {
            ui.painter().rect_filled(segment, 5.0, theme::ink(10));
        }
        ui.painter().text(
            segment.center(),
            Align2::CENTER_CENTER,
            labels[index],
            font.clone(),
            if selected {
                theme::on_selected_plate()
            } else {
                theme::text_primary()
            },
        );
        if response.clicked() {
            preview.select_timeframe(*timeframe);
        }
        x += widths[index];
    }
}

// MARK: Chart area

pub fn chart(
    ui: &mut Ui,
    preview: &mut PreviewState,
    input: &PreviewInput,
    cmds: &RefCell<Vec<Cmd>>,
) {
    let area = ui.max_rect();
    ui.painter().rect_filled(area, 0.0, theme::background());
    preview.source_changed(input);
    preview.maybe_apply(input);
    if std::mem::take(&mut preview.pending_settings) {
        cmds.borrow_mut().push(Cmd::OpenAlpacaSettings);
    }

    if input.script_type == ScriptType::Library {
        message(
            ui,
            area,
            Icon::Books,
            "Nothing to chart",
            "Libraries export functions — they don't draw on a chart.",
            None,
            cmds,
        );
        return;
    }
    if preview.market.source == DataSource::Alpaca && !alpaca_configured() {
        message(
            ui,
            area,
            Icon::Key,
            "Alpaca isn't set up",
            "Stock candles come from Alpaca. Add your keys in Settings to test on stocks.",
            Some(("Open Settings", Cmd::OpenAlpacaSettings)),
            cmds,
        );
        return;
    }

    let ctx = ui.ctx().clone();
    if preview.want_fetch() {
        preview.fetch(&ctx);
    } else if preview.inbox.is_none()
        && preview
            .last_fetch
            .is_some_and(|at| at.elapsed() >= REFRESH_INTERVAL)
    {
        // Quiet refresh: the candles stay up while the new ones load.
        preview.fetched_for = None;
        preview.fetch(&ctx);
        preview.loading = false;
    }
    ctx.request_repaint_after(REFRESH_INTERVAL);

    let inner = area.shrink(6.0);
    let mut chart_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner));
    chart_ui.set_clip_rect(inner.intersect(ui.clip_rect()));
    let drawings: Vec<crate::model::Drawing> = Vec::new();
    let mut frame = DrawingFrame::new(&drawings, crate::model::Tool::default(), None, 1);
    let mut replay = ReplayChartFrame {
        selecting: false,
        active: false,
        picked_ms: None,
    };
    let candles = std::mem::take(&mut preview.candles);
    let symbol = preview.market.ticker.clone();
    let source = preview.market.source;
    let loading = preview.loading && candles.is_empty();
    let error = preview.error.clone();
    preview.card.body(
        &mut chart_ui,
        &symbol,
        &candles,
        loading,
        if candles.is_empty() {
            error.as_deref()
        } else {
            None
        },
        PriceScaleMode::Regular,
        source,
        &IndicatorConfig::default(),
        None,
        u64::MAX,
        &mut frame,
        &mut replay,
    );
    preview.candles = candles;

    // The stale banner, over the top of the chart.
    if preview.stale {
        let rect = stale_banner(ui, area);
        let response = ui.interact(rect, Id::new("preview-stale"), Sense::click());
        if response.clicked() {
            cmds.borrow_mut()
                .push(Cmd::SetDrawerTab(super::prefs::DrawerTab::Problems));
        }
        response
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Show problems");
    }
    // Refresh failures with candles on screen: a retry banner at the bottom.
    if let Some(message) = error.filter(|_| !preview.candles.is_empty())
        && error_banner(ui, area, &message)
    {
        preview.fetched_for = None;
    }
}

fn alpaca_configured() -> bool {
    crate::service::keychain_api_keys::AlpacaCredentialsStore::shared()
        .credentials()
        .is_configured()
}

fn message(
    ui: &mut Ui,
    area: Rect,
    icon: Icon,
    title: &str,
    detail: &str,
    action: Option<(&str, Cmd)>,
    cmds: &RefCell<Vec<Cmd>>,
) {
    let mut column = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_center_size(
                area.center(),
                vec2(260.0_f32.min(area.width() - 20.0), 200.0),
            ))
            .layout(Layout::top_down(Align::Center)),
    );
    column.spacing_mut().item_spacing.y = 8.0;
    icons::inline(&mut column, icon, 22.0, theme::text_secondary());
    column.label(RichText::new(title).size(15.0).color(theme::text_primary()));
    column.label(
        RichText::new(detail)
            .size(10.0)
            .color(theme::text_secondary()),
    );
    if let Some((label, cmd)) = action
        && column.add(egui::Button::new(label)).clicked()
    {
        cmds.borrow_mut().push(cmd);
    }
}

fn stale_banner(ui: &mut Ui, area: Rect) -> Rect {
    let text = "Fix the errors to update the chart — showing the last working version";
    let font = FontId::proportional(10.0);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, theme::text_primary());
    let size = vec2(galley.size().x + 20.0 + 18.0, 24.0);
    let rect = Rect::from_center_size(pos2(area.center().x, area.min.y + 8.0 + size.y / 2.0), size);
    let painter = ui.painter();
    painter.rect_filled(rect, 12.0, Color32::from_rgba_unmultiplied(40, 48, 52, 235));
    painter.rect_stroke(
        rect,
        12.0,
        Stroke::new(1.0, theme::with_alpha(theme::syntax::string(), 0.5)),
        egui::StrokeKind::Inside,
    );
    icons::paint(
        painter,
        Rect::from_center_size(pos2(rect.min.x + 14.0, rect.center().y), Vec2::splat(11.0)),
        Icon::XmarkOctagonFill,
        theme::syntax::string(),
    );
    painter.galley(
        pos2(rect.min.x + 26.0, rect.center().y - galley.size().y / 2.0),
        galley,
        theme::text_primary(),
    );
    rect
}

/// Returns whether Retry was clicked.
fn error_banner(ui: &mut Ui, area: Rect, message: &str) -> bool {
    let font = FontId::proportional(9.0);
    let galley =
        ui.painter()
            .layout_no_wrap(message.to_owned(), font.clone(), theme::text_secondary());
    let retry = ui
        .painter()
        .layout_no_wrap("Retry".to_owned(), font, theme::SYSTEM_ACCENT);
    let width =
        (galley.size().x + retry.size().x + 8.0 + 8.0 + 16.0 + 14.0).min(area.width() - 16.0);
    let rect = Rect::from_min_size(
        pos2(area.min.x + 8.0, area.max.y - 8.0 - 22.0),
        vec2(width, 22.0),
    );
    let painter = ui.painter();
    painter.rect_filled(rect, 4.0, Color32::from_rgba_unmultiplied(40, 48, 52, 235));
    icons::paint(
        painter,
        Rect::from_center_size(pos2(rect.min.x + 12.0, rect.center().y), Vec2::splat(11.0)),
        Icon::TriangleFill,
        theme::ORANGE,
    );
    painter.galley(
        pos2(rect.min.x + 24.0, rect.center().y - galley.size().y / 2.0),
        galley.clone(),
        theme::text_secondary(),
    );
    let retry_rect = Rect::from_min_size(
        pos2(rect.min.x + 24.0 + galley.size().x + 8.0, rect.min.y),
        vec2(retry.size().x + 6.0, 22.0),
    );
    painter.galley(
        pos2(retry_rect.min.x, rect.center().y - retry.size().y / 2.0),
        retry,
        theme::SYSTEM_ACCENT,
    );
    ui.interact(retry_rect, Id::new("preview-retry"), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand)
        .clicked()
}

// MARK: Market picker

fn picker_ui(ui: &mut Ui, preview: &mut PreviewState) -> Option<PreviewMarket> {
    let mut selected = None;
    let mut picker = preview.picker.take()?;
    let ctx = ui.ctx().clone();
    let width = ui.available_width();
    ui.spacing_mut().item_spacing.y = 10.0;

    // The same icon tab bar the alert centre and chart settings use.
    let tabs = [
        kit::TabItem {
            value: AssetType::Crypto,
            title: "Crypto",
            icon: Icon::BitcoinCircle,
            count: 0,
        },
        kit::TabItem {
            value: AssetType::Stock,
            title: "Stocks",
            icon: Icon::BuildingColumns,
            count: 0,
        },
    ];
    let before = picker.asset;
    kit::tab_bar(
        ui,
        Id::new("script-preview-asset"),
        &tabs,
        &mut picker.asset,
    );
    if picker.asset != before {
        picker.results.clear();
        picker.cursor = 0;
        picker.needs_focus = true;
        picker.last_edit = Some(Instant::now());
    }

    let recents: Vec<PreviewMarket> = preview
        .recents
        .iter()
        .filter(|m| (m.source == DataSource::Alpaca) == (picker.asset == AssetType::Stock))
        .cloned()
        .collect();
    let query_empty = picker.text().trim().is_empty();
    let rows = if query_empty {
        recents.len()
    } else {
        picker.results.len()
    };
    // Arrow keys move the highlight; Return picks it. Consumed before the field sees them.
    let mut moved = false;
    if rows > 0 {
        let (down, up) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
            )
        });
        if down {
            picker.cursor = (picker.cursor + 1).min(rows - 1);
            moved = true;
        }
        if up {
            picker.cursor = picker.cursor.saturating_sub(1);
            moved = true;
        }
    }
    picker.cursor = picker.cursor.min(rows.saturating_sub(1));

    let placeholder = match picker.asset {
        AssetType::Crypto => "Search coins and tokens, e.g. BTC, ETH, PEPE",
        AssetType::Stock => "Search US stocks and ETFs, e.g. AAPL, Tesla",
    };
    let field_id = Id::new("script-preview-search");
    let searching = picker.searching;
    let mut submitted = false;
    let mut text_changed = false;
    let focus = std::mem::take(&mut picker.needs_focus);
    let focused = ui.memory(|m| m.has_focus(field_id));
    kit::field_frame(focused)
        .inner_margin(Margin::symmetric(10, 0))
        .show(ui, |ui| {
            // A fixed-height row: `horizontal_centered` would claim the popup's whole height.
            ui.set_width(ui.available_width());
            let row = vec2(ui.available_width(), 34.0);
            ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                icons::inline(ui, Icon::Magnifier, 13.0, theme::text_secondary());
                let field_width = (ui.available_width() - 26.0).max(40.0);
                let field = kit::plain_field(
                    ui,
                    field_id,
                    picker.text(),
                    placeholder,
                    13.0,
                    kit::REGULAR,
                    field_width,
                );
                if focus {
                    field.request_focus();
                }
                text_changed = field.changed();
                submitted = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if searching {
                    ui.add(egui::Spinner::new().size(13.0));
                } else if !picker.text().is_empty()
                    && icons::button(
                        ui,
                        Icon::XmarkCircleFill,
                        18.0,
                        theme::text_secondary(),
                        "Clear search",
                    )
                    .clicked()
                {
                    picker.text().clear();
                    picker.results.clear();
                    text_changed = true;
                    field.request_focus();
                }
            });
        });
    if text_changed {
        picker.last_edit = Some(Instant::now());
        picker.cursor = 0;
    }
    drive_search(&mut picker, &ctx);

    // The body keeps one height, so the popup does not jump between empty, loading and results.
    ui.allocate_ui_with_layout(
        vec2(width, BODY_HEIGHT),
        Layout::top_down(Align::Min),
        |ui| {
            ui.set_min_height(BODY_HEIGHT);
            ui.spacing_mut().item_spacing.y = 0.0;
            if picker.asset == AssetType::Stock && !alpaca_configured() {
                if kit::notice_card(
                    ui,
                    Icon::TriangleFill,
                    theme::YELLOW,
                    "Stocks need Alpaca market data",
                    Some("Add your Alpaca API keys in Settings."),
                    Some("Open Settings"),
                ) {
                    preview.pending_settings = true;
                }
                return;
            }
            let rows_width = ui.available_width();
            if query_empty && recents.is_empty() {
                body_message(
                    ui,
                    Icon::Magnifier,
                    "Search markets",
                    "Find a symbol to run this script on.",
                );
            } else if query_empty {
                section_header(ui, "Recent", None);
                for (index, market) in recents.iter().enumerate() {
                    let response = recent_row(ui, market, &preview.market, index == picker.cursor);
                    if index == picker.cursor && moved {
                        response.scroll_to_me(None);
                    }
                    if response.clicked() {
                        selected = Some(market.clone());
                    }
                }
                if submitted {
                    selected = recents.get(picker.cursor).cloned();
                }
            } else if !picker.results.is_empty() {
                if submitted {
                    selected = picker.results.get(picker.cursor).map(market_of);
                }
                egui::ScrollArea::vertical()
                    .max_height(BODY_HEIGHT)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_width(rows_width);
                        let mut last_source = None;
                        for (index, result) in picker.results.iter().enumerate() {
                            if last_source != Some(result.source) {
                                last_source = Some(result.source);
                                section_header(
                                    ui,
                                    result.source.display_name(),
                                    Some(result.source),
                                );
                            }
                            let current = market_of(result) == preview.market;
                            // The keyboard highlight sits under the row's own paint.
                            let slot = ui.painter().add(egui::Shape::Noop);
                            let response = add_chart_sheet::result_row(
                                ui,
                                result,
                                current,
                                false,
                                rows_width - 8.0,
                            );
                            if index == picker.cursor && !current {
                                ui.painter().set(
                                    slot,
                                    egui::Shape::rect_filled(response.rect, 8.0, theme::ink(15)),
                                );
                                if moved {
                                    response.scroll_to_me(None);
                                }
                            }
                            if response.clicked() {
                                selected = Some(market_of(result));
                            }
                        }
                    });
            } else if picker.searching || picker.last_edit.is_some() {
                let (rect, _) =
                    ui.allocate_exact_size(vec2(rows_width, BODY_HEIGHT), Sense::hover());
                ui.put(
                    Rect::from_center_size(rect.center(), Vec2::splat(18.0)),
                    egui::Spinner::new().size(18.0),
                );
            } else {
                body_message(
                    ui,
                    Icon::Magnifier,
                    "No markets found",
                    "Try another symbol or name.",
                );
            }
        },
    );

    if rows > 0 {
        ui.label(
            RichText::new("↑↓ Navigate    ↩ Select    esc Close")
                .size(10.0)
                .color(theme::text_tertiary()),
        );
    }
    preview.picker = Some(picker);
    selected
}

/// Height of the picker's results area.
const BODY_HEIGHT: f32 = 300.0;

/// A quiet message that fills the results area.
fn body_message(ui: &mut Ui, icon: Icon, title: &str, detail: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), BODY_HEIGHT), Sense::hover());
    kit::empty_state(ui, rect, icon, title, detail);
}

/// A group caption: the source's logo and name, or just a title.
fn section_header(ui: &mut Ui, title: &str, source: Option<DataSource>) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    let mut x = rect.left() + 10.0;
    if let Some(source) = source {
        let logo = Rect::from_center_size(pos2(x + 6.0, rect.center().y + 2.0), Vec2::splat(12.0));
        source_logo::paint(ui, source, logo);
        x = logo.right() + 6.0;
    }
    let label = kit::galley(
        ui,
        &title.to_uppercase(),
        10.5,
        kit::SEMIBOLD,
        theme::text_secondary(),
        f32::INFINITY,
    );
    ui.painter().galley(
        pos2(x, rect.center().y + 2.0 - label.size().y / 2.0),
        label,
        theme::text_secondary(),
    );
}

fn market_of(result: &SearchResult) -> PreviewMarket {
    PreviewMarket {
        ticker: result.full_symbol.clone(),
        source: result.source,
    }
}

/// A recently used market, drawn like a search hit.
fn recent_row(
    ui: &mut Ui,
    market: &PreviewMarket,
    current: &PreviewMarket,
    highlighted: bool,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width() - 8.0, 38.0), Sense::click());
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    let is_current = market == current;
    let fill = if is_current {
        theme::with_alpha(theme::ACCENT, 0.12)
    } else if response.hovered() || highlighted {
        theme::ink(15)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 8.0, fill);
    if is_current {
        ui.painter().rect_stroke(
            rect,
            8.0,
            Stroke::new(1.0, theme::with_alpha(theme::ACCENT, 0.5)),
            egui::StrokeKind::Inside,
        );
    }
    let content = rect.shrink2(vec2(10.0, 5.0));
    let icon = Rect::from_min_size(
        pos2(content.left(), rect.center().y - 14.0),
        Vec2::splat(28.0),
    );
    kit::chart_icon(ui, icon, &market.ticker, market.source, true);
    let mut right = content.right();
    if is_current {
        let check = Rect::from_center_size(pos2(right - 7.0, rect.center().y), Vec2::splat(14.0));
        icons::paint(ui.painter(), check, Icon::CheckCircleFill, theme::ACCENT);
        right = check.left() - 10.0;
    }
    let source = kit::galley(
        ui,
        market.source.display_name(),
        11.0,
        kit::REGULAR,
        theme::text_secondary(),
        f32::INFINITY,
    );
    right -= source.size().x;
    ui.painter().galley(
        pos2(right, rect.center().y - source.size().y / 2.0),
        source,
        theme::text_secondary(),
    );
    let left = icon.right() + 10.0;
    let title = kit::galley(
        ui,
        &market.title(),
        13.0,
        kit::SEMIBOLD,
        theme::text_primary(),
        (right - left - 10.0).max(20.0),
    );
    ui.painter().galley(
        pos2(left, rect.center().y - title.size().y / 2.0),
        title,
        theme::text_primary(),
    );
    response
}

/// Debounces the query by 300 ms, then searches the right providers off the UI thread.
fn drive_search(picker: &mut Picker, ctx: &egui::Context) {
    if let Some(inbox) = &picker.inbox
        && let Ok((generation, results)) = inbox.try_recv()
    {
        picker.inbox = None;
        picker.searching = false;
        if generation == picker.generation {
            picker.results = results;
            picker.cursor = 0;
        }
    }
    let Some(at) = picker.last_edit else {
        return;
    };
    if at.elapsed() < SEARCH_DELAY {
        ctx.request_repaint_after(SEARCH_DELAY);
        return;
    }
    picker.last_edit = None;
    let query = picker.text().trim().to_owned();
    picker.generation += 1;
    if query.is_empty() {
        picker.results.clear();
        picker.searching = false;
        return;
    }
    let generation = picker.generation;
    let sources: Vec<DataSource> = match picker.asset {
        AssetType::Crypto => CRYPTO_SOURCES.to_vec(),
        AssetType::Stock => vec![DataSource::Alpaca],
    };
    let (sender, receiver) = mpsc::channel();
    let repaint = ctx.clone();
    std::thread::spawn(move || {
        let mut all = Vec::new();
        for source in sources {
            if let Ok(mut found) = service::for_source(source).search_tickers(&query) {
                all.append(&mut found);
            }
        }
        let _ = sender.send((generation, all));
        repaint.request_repaint();
    });
    picker.inbox = Some(receiver);
    picker.searching = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::token::{Diagnostic, DiagnosticCategory, SourcePosition, SourceRange};
    use crate::store::Store;
    use std::sync::{Arc, Mutex};

    fn state() -> PreviewState {
        let shared: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        PreviewState::load(&shared)
    }

    fn input(source: &str, errors: bool) -> PreviewInput {
        PreviewInput {
            name: "Demo".into(),
            script_type: ScriptType::Indicator,
            source: source.into(),
            diagnostics: if errors {
                vec![Diagnostic {
                    code: "PINE1".into(),
                    severity: DiagnosticSeverity::Error,
                    category: DiagnosticCategory::Syntax,
                    message: "bad".into(),
                    range: SourceRange::new(
                        SourcePosition::new(1, 1, 0),
                        SourcePosition::new(1, 2, 1),
                    ),
                }]
            } else {
                Vec::new()
            },
            compile_pending: false,
        }
    }

    #[test]
    fn an_edit_waits_for_its_compile_before_it_applies() {
        let mut preview = state();
        preview.script_changed("s1");
        let good = input("//@version=6\nindicator(\"x\")\nplot(close)\n", false);
        preview.source_changed(&good);
        preview.maybe_apply(&good);

        let mut edited = input("//@version=6\nindicator(\"x\")\nplot(open)\n", false);
        edited.compile_pending = true;
        preview.source_changed(&edited);
        preview.changed_at = Some(Instant::now() - APPLY_DELAY - Duration::from_millis(5));
        preview.maybe_apply(&edited);
        assert_eq!(
            preview.applied_source.as_deref(),
            Some(good.source.as_str()),
            "the diagnostics still describe the old text"
        );
        assert!(preview.changed_at.is_some(), "the apply is kept for later");

        edited.compile_pending = false;
        preview.maybe_apply(&edited);
        assert_eq!(
            preview.applied_source.as_deref(),
            Some(edited.source.as_str())
        );
    }

    #[test]
    fn defaults_and_support() {
        let preview = state();
        assert_eq!(preview.market, PreviewMarket::default_market());
        assert_eq!(preview.timeframe, Timeframe::Day);
        assert!(PreviewMarket::is_supported(DataSource::Alpaca));
        assert!(!PreviewMarket::is_supported(DataSource::Polymarket));
        assert!(!PreviewMarket::is_supported(DataSource::CoinMarketCap));
    }

    #[test]
    fn the_first_source_applies_at_once_then_edits_wait_for_a_pause() {
        let mut preview = state();
        preview.script_changed("s1");
        let good = input("//@version=6\nindicator(\"x\")\nplot(close)\n", false);
        preview.source_changed(&good);
        preview.maybe_apply(&good);
        assert_eq!(
            preview.applied_source.as_deref(),
            Some(good.source.as_str())
        );

        let edited = input("//@version=6\nindicator(\"x\")\nplot(open)\n", false);
        preview.source_changed(&edited);
        preview.maybe_apply(&edited);
        assert_eq!(
            preview.applied_source.as_deref(),
            Some(good.source.as_str()),
            "a keystroke ago: still the old source"
        );
        preview.changed_at = Some(Instant::now() - APPLY_DELAY - Duration::from_millis(5));
        preview.maybe_apply(&edited);
        assert_eq!(
            preview.applied_source.as_deref(),
            Some(edited.source.as_str())
        );
    }

    #[test]
    fn broken_source_keeps_the_last_output_and_goes_stale() {
        let mut preview = state();
        preview.script_changed("s1");
        let good = input("plot(close)", false);
        preview.source_changed(&good);
        preview.maybe_apply(&good);
        let broken = input("plot(", true);
        preview.source_changed(&broken);
        preview.changed_at = Some(Instant::now() - APPLY_DELAY - Duration::from_millis(5));
        preview.maybe_apply(&broken);
        assert!(preview.stale && preview.compile_failed);
        assert_eq!(preview.applied_source.as_deref(), Some("plot(close)"));
        assert_eq!(
            preview.status_text(),
            "Compile failed — last valid output remains active"
        );
        // Back to working text clears the banner.
        let fixed = input("plot(open)", false);
        preview.source_changed(&fixed);
        preview.changed_at = Some(Instant::now() - APPLY_DELAY - Duration::from_millis(5));
        preview.maybe_apply(&fixed);
        assert!(!preview.stale && !preview.compile_failed);
    }

    #[test]
    fn recents_are_newest_first_unique_and_capped() {
        let mut preview = state();
        for i in 0..7 {
            preview.select_market(PreviewMarket {
                ticker: format!("T{i}"),
                source: DataSource::Binance,
            });
        }
        assert_eq!(preview.recents.len(), RECENTS_LIMIT);
        assert_eq!(preview.recents[0].ticker, "T6");
        preview.select_market(PreviewMarket {
            ticker: "T4".into(),
            source: DataSource::Binance,
        });
        assert_eq!(preview.recents[0].ticker, "T4");
        assert_eq!(
            preview.recents.iter().filter(|m| m.ticker == "T4").count(),
            1
        );
    }

    #[test]
    fn the_change_counts_closes_across_the_visible_window_only() {
        let mut preview = state();
        // 40 candles closing 100, 101, … 139; the first 25 are warm-up off the left edge.
        preview.candles = (0..40)
            .map(|i| Kline {
                open_time: i * 60_000,
                close_time: i * 60_000 + 59_999,
                open: 50.0,
                high: 200.0,
                low: 10.0,
                close: 100.0 + i as f64,
                volume: 1.0,
            })
            .collect();
        preview.card.set_visible_candles(15);
        let (price, change) = preview.price_line().unwrap();
        assert_eq!(price, format::price(139.0));
        // Close 125 → close 139, not the first candle's open of 50.
        assert!((change - (139.0 - 125.0) / 125.0 * 100.0).abs() < 1e-9);
    }

    #[test]
    fn percent_uses_a_real_minus() {
        assert_eq!(signed_percent(1.5), "+1.50%");
        assert_eq!(signed_percent(-0.25), "−0.25%");
    }
}
