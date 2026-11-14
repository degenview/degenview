use crate::host::paper::PaperHost;
use crate::host::portfolio::card::PortfolioCardEnv;
use crate::lock::lock;
use crate::model::alert::{AlertCurrency, PriceAlert};
use crate::model::chart::sync::SyncHub;
use crate::model::tab::link::LinkColor;
use crate::model::tab::title::{MAX_CUSTOM_TITLE_CHARS, TabChart};
use crate::model::watchlist::text::ImportReport;
use crate::model::watchlist::{EntryId, WatchlistInstrument, now_ms};
use crate::model::{
    BuiltIn, ChartColumn, ChartSyncSettings, DataSource, Drawing, DrawingCommand, DrawingStyle,
    InstrumentId, LayoutSnapshot, MagnetMode, ReplayEngine, ReplayStatus, Timeframe, Tool,
};
use crate::service::market::live::BinanceLiveFeed;
use crate::service::watchlist_quotes::{WatchlistQuoteBook, WatchlistQuoteCoordinator};
use crate::store::SharedStore;
use crate::store::drawing_template::{DrawingPrefs, DrawingTemplateStore};
use crate::store::watchlist::WatchlistStore;
use crate::ui::alert::editor::{self, PriceAlertEditorState};
use crate::ui::chart::indicator_picker::{self, IndicatorPicker};
use crate::ui::chart::settings::overlay_popover::OverlaySettingsPopover;
use crate::ui::chart::settings::{self as chart_settings, ChartSettingsState};
use crate::ui::chart::symbol_picker::SymbolPicker;
use crate::ui::drawing::properties::DrawingProperties;
use crate::ui::paper::chart_trading::PaperOverlayAction;
use crate::ui::paper::panel::PaperPanel;
use crate::ui::pine::settings_popover::PineSettingsPopover;
use crate::ui::replay::date_sheet::{self, ReplayDateSheetState};
use crate::ui::watchlist::sidebar::WatchlistSidebarState;
use crate::ui::workspace::layout_prompts::NamePrompt;
use crate::ui::{self, CrosshairInfo, replay::bar, workspace::tool_sidebar};
use alerts::AlertRequest;
use chart::context::ContextState;
use chart::snapshot::SnapshotState;
use chart::{Chart, displayed_candles};
use drawing::is_lockable;
use eframe::egui;
use empty_page::EmptyPageCache;
use layout::autosave::AutosaveState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use undo::{Snapshot, UndoEntry};

mod alerts;
mod chart;
mod drawing;
mod empty_page;
mod grid;
mod input;
mod layout;
mod overlay;
mod pine;
mod replay;
mod tab_link;
#[cfg(test)]
mod test_support;
mod toolbar;
mod trade;
mod undo;
mod watchlist;

pub use chart::collection::{ChartSummary, ChartTypeError};
pub use chart::snapshot::CaptureError;
pub use pine::{
    IndicatorError, IndicatorOutcome, PineChartTarget, PineInstanceView, RemoveOutcome,
};
// Named only by the MCP host's tests; it is the type of `PineInstanceView::run`.
#[cfg(test)]
pub use pine::PineRunView;

#[derive(Clone, Debug, PartialEq)]
pub enum WorkspaceAction {
    OpenPortfolio,
    /// Open the Pine Studio tab.
    OpenPineScripts,
    /// Open the Pine Studio on the source of a Pine logs message.
    OpenPineSource(crate::model::pine::logs_view::SourceTarget),
    OpenAlerts,
    /// The price-alert sheet's Save: queue the alert for the runtime. `baseline` is the price
    /// the sheet last showed.
    SavePriceAlert {
        alert: Box<PriceAlert>,
        baseline: Option<f64>,
    },
    /// The price-alert sheet's "Add in Settings".
    OpenWebhookSettings,
    /// Open Settings on the Alpaca page (the symbol picker's "Stocks need an Alpaca key" notice).
    OpenAlpacaSettings,
    /// Open Settings on the Brokers page (the live trading panel).
    OpenBrokerSettings,
    /// Open Settings on the Snapshots page (the camera menu's "Configure Snapshots…").
    OpenSnapshotSettings,
    /// Pause or resume a price alert (its line on a chart).
    SetAlertState {
        id: crate::model::alert::AlertId,
        state: crate::model::alert::AlertState,
    },
    /// Delete a price alert (its line on a chart).
    DeleteAlert(crate::model::alert::AlertId),
    /// An alert's line was dragged to a new level: queue the alert as changed. `baseline` is the
    /// market's last price in the alert's currency, which arms it for the side it now watches.
    /// Unlike a save from the sheet it asks for nothing: the alert already exists.
    MoveAlert {
        alert: Box<PriceAlert>,
        baseline: Option<f64>,
    },
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
    /// The saved layout this tab has open, so a restart shows its name again instead of
    /// "Unnamed". Rows from before this field, and tabs on no layout, have none.
    #[serde(default)]
    pub active_view: Option<u64>,
    /// A title the user gave this tab, shown in place of the symbol and price. Device-local:
    /// the `tab` table never syncs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_title: Option<String>,
    /// The Tab Linking colour this tab is tagged with. Device-local like the title: the `tab`
    /// table never syncs. A colour this build does not know reads as not linked.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "LinkColor::lenient"
    )]
    pub link: Option<LinkColor>,
}

impl crate::store::Document for PersistedWorkspace {
    fn document_id(&self) -> String {
        self.id.to_string()
    }
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
    indicator_picker: IndicatorPicker,
    symbol_picker: SymbolPicker,
    add_to_watchlist: Option<EntryId>,
    unseen_alerts: usize,
    /// How portfolio cards reach the app's portfolios; set by the window that draws this tab.
    card_env: Option<PortfolioCardEnv>,
    /// The venues this tab can trade on, handed in by the window each frame.
    trading_hub: Option<Arc<crate::host::trading::TradingHub>>,
    live_feed: Option<BinanceLiveFeed>,
    /// The candles of every overlay market on this workspace's charts, fetched once per market.
    overlay_hub: overlay::OverlayHub,
    settings_chart: Option<u64>,
    /// The Inputs and Style popover of an applied script, up beside its chip or settings row.
    script_popover: Option<PineSettingsPopover>,
    /// The settings of a compared market, up beside its row on the chart's overlay card.
    overlay_popover: Option<OverlaySettingsPopover>,
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
    /// The overlay colour edit the undo stack last took: chart, overlay, the stack's length right
    /// after, and when. A drag of the colour picker continues it instead of stacking steps.
    last_overlay_style: Option<(u64, crate::model::overlay::OverlayId, usize, Instant)>,
    /// The chart a header ✕ asked to remove, awaiting confirmation.
    removing_chart: Option<u64>,
    settings_state: ChartSettingsState,
    crosshair: Option<CrosshairInfo>,
    active_tool: Tool,
    /// The Zoom In mode of the tool rail. A mode of its own, never a drawing tool: a drag on a
    /// plot selects a rectangle to zoom to. Exclusive with `active_tool`; not saved.
    zoom_armed: bool,
    /// The selected drawings of one chart: `(chart id, drawing ids)`.
    selected_drawing: Option<(u64, Vec<u64>)>,
    /// The tool the strip's grouped brushes, arrows and shapes tile shows: the last one used.
    group_tool: Tool,
    line_tool: Tool,
    note_tool: Tool,
    /// The tool the chart patterns, Elliott waves and cycles tile shows.
    pattern_tool: Tool,
    /// The tool the forecasting, volume and measurement tile shows.
    forecast_tool: Tool,
    /// The tool the Fibonacci and Gann tile shows.
    fib_tool: Tool,
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
    /// The timeframe changed under a replay: its bars are being reloaded, and the replay moves
    /// onto them once they are in.
    replay_rebase: bool,
    watchlist: Arc<Mutex<WatchlistStore>>,
    watchlist_sidebar: WatchlistSidebarState,
    date_sheet: ReplayDateSheetState,
    quote_book: WatchlistQuoteBook,
    quote_coordinator: WatchlistQuoteCoordinator,
    import_report: Option<ImportReport>,
    watchlist_error: Option<crate::model::watchlist::WatchlistNotice>,
    watchlist_file_result: Option<crate::service::work_queue::Pending<Result<(), String>>>,
    paper_panel_visible: bool,
    /// The order sheet beside the chart. Open and closed with the trading panel, and by its own ✕.
    trade_sheet_visible: bool,
    trade_sheet: crate::ui::trade::order_sheet::SheetState,
    /// Where this tab's trading happens: paper, or a connected broker with real money. Not
    /// saved: a tab always starts on paper.
    trading_venue: crate::model::broker::venue::TradingVenue,
    broker_panel: crate::ui::broker::panel::BrokerPanelState,
    /// A chart's bell (or a watchlist row's menu) asked for a price alert this frame; `ui`
    /// opens the sheet on it.
    new_alert_request: Option<AlertRequest>,
    /// The chart menu's state: the open menu, its messages, and the panels it toggles.
    context: ContextState,
    /// The snapshot being made, and what it waits for.
    snapshot: SnapshotState,
    /// The snapshot settings all windows share (set by the app).
    snapshots: Option<crate::host::snapshot::SnapshotHost>,
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
    /// What the user typed as this tab's title; `None` shows the symbol, price and layout.
    custom_title: Option<String>,
    /// The Tab Linking group this tab is in, if any. Membership is the tab's own; the switches
    /// belong to the colour and live in `TabLinkHost`.
    link: Option<LinkColor>,
    link_state: tab_link::LinkState,
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
    grid_drag: ui::workspace::grid_drop::GridDrag,
    /// Choosing what a content tool places: the picture that waits, the sheet, the worker.
    content: drawing::content::ContentFlow,
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
            indicator_picker: IndicatorPicker::new(Some(store.clone())),
            symbol_picker: SymbolPicker::new(store.clone()),
            add_to_watchlist: None,
            unseen_alerts: 0,
            card_env: None,
            trading_hub: None,
            live_feed: None,
            overlay_hub: overlay::OverlayHub::new(),
            settings_chart: None,
            script_popover: None,
            overlay_popover: None,
            drawing_props: None,
            drawing_templates: None,
            saved_prefs: DrawingPrefs::default(),
            style_clipboard: None,
            confirm_remove_drawings: None,
            last_restyle: None,
            last_overlay_style: None,
            removing_chart: None,
            settings_state: ChartSettingsState::default(),
            crosshair: None,
            active_tool: Tool::Cursor,
            zoom_armed: false,
            selected_drawing: None,
            group_tool: Tool::Brush,
            line_tool: Tool::TrendLine,
            note_tool: Tool::Text,
            pattern_tool: Tool::XabcdPattern,
            forecast_tool: Tool::LongPosition,
            fib_tool: Tool::FibRetracement,
            magnet: MagnetMode::Off,
            keep_drawing: true,
            hide_all_drawings: false,
            undo: Vec::new(),
            redo: Vec::new(),
            replay: ReplayEngine::default(),
            replay_last_step: Instant::now(),
            replay_rebase: false,
            watchlist,
            watchlist_sidebar: WatchlistSidebarState::default(),
            date_sheet: ReplayDateSheetState::default(),
            quote_book,
            quote_coordinator,
            import_report: None,
            watchlist_error: None,
            watchlist_file_result: None,
            paper_panel_visible: false,
            trade_sheet_visible: false,
            trade_sheet: crate::ui::trade::order_sheet::SheetState::default(),
            trading_venue: crate::model::broker::venue::TradingVenue::default(),
            broker_panel: crate::ui::broker::panel::BrokerPanelState::default(),
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
            custom_title: None,
            link: None,
            link_state: Default::default(),
            layout_prompt: None,
            prompt_name: String::new(),
            prompt_focus: false,
            pending_open_id: None,
            layout_error: None,
            autosave: AutosaveState::default(),
            drawings_version: 0,
            saved_drawings_version: 0,
            saved_drawings: HashMap::new(),
            grid_drag: ui::workspace::grid_drop::GridDrag::default(),
            content: drawing::content::ContentFlow::default(),
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
            active_view,
            custom_title,
            link,
        } = saved;

        // The layout may have been deleted (here or on another Mac) since the tab was saved.
        let active_view = active_view.and_then(|id| {
            store
                .lock()
                .ok()?
                .saved_views()
                .into_iter()
                .find(|view| view.id == id)
        });

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
        workspace.custom_title = custom_title.filter(|title| !title.trim().is_empty());
        workspace.link = link;
        if let Some(view) = active_view {
            workspace.active_view_id = Some(view.id);
            workspace.active_view_name = Some(view.name);
        }
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

    /// The focused chart's market, when it is one (a portfolio card has none). The strip's
    /// logo, price and change follow it, so a layout with several charts is represented by the
    /// one the user last clicked, or the first.
    pub fn focused_instrument(&self) -> Option<InstrumentId> {
        let chart = self.focused_market_chart()?;
        Some(InstrumentId::new(chart.source, &chart.symbol))
    }

    fn focused_market_chart(&self) -> Option<&Chart> {
        let id = self.resolved_focused_id()?;
        self.charts
            .iter()
            .find(|chart| chart.id == id && chart.is_market())
    }

    #[cfg(test)]
    pub fn custom_title(&self) -> Option<&str> {
        self.custom_title.as_deref()
    }

    /// The Tab Linking colour this tab is tagged with.
    pub fn link(&self) -> Option<LinkColor> {
        self.link
    }

    /// Tags the tab with a colour, or unlinks it. Only the tag changes: the chart, its symbol,
    /// timeframe and viewport are left exactly as they are, and the tab's tag is not part of any
    /// layout. Returns whether it changed.
    pub fn set_link(&mut self, link: Option<LinkColor>) -> bool {
        if self.link == link {
            return false;
        }
        self.link = link;
        if link.is_none() {
            // Unlinking stops what the tab would have announced, at once.
            self.link_state.outbox.take();
            self.link_state.probation = None;
        }
        true
    }

    /// Sets or clears the tab's own title (blank clears). Not part of the layout: it belongs
    /// to the tab and is saved with the session.
    pub fn set_custom_title(&mut self, title: Option<String>) {
        self.custom_title = title
            .map(|title| title.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|title| !title.is_empty())
            .map(|title| title.chars().take(MAX_CUSTOM_TITLE_CHARS).collect());
    }

    /// What the strip needs to describe this tab: the focused chart's market and the layout.
    /// `None` for a blank tab or when a card has the focus.
    pub fn tab_chart(&self) -> Option<TabChart> {
        let chart = self.focused_market_chart()?;
        Some(TabChart {
            source: chart.source,
            symbol: chart.symbol.clone(),
            title: chart.display_title(),
            layout: self.active_view_name.clone(),
            custom_title: self.custom_title.clone(),
        })
    }

    // MARK: Saved layouts

    // MARK: Chart lifecycle

    // MARK: Search sheet

    // MARK: Watchlist navigation

    // MARK: Live feed

    // MARK: Drawings

    // MARK: Replay

    // MARK: Drag reorder

    // MARK: Panels

    // MARK: Grid

    /// Draws the workspace. The paper host is passed as its shared handle and locked only around
    /// the parts that edit it: the order sheet and the venue pill read it through the trading hub,
    /// which locks the same mutex, so a guard held across the whole frame would deadlock them.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        paper: &std::sync::Arc<std::sync::Mutex<PaperHost>>,
    ) -> Vec<WorkspaceAction> {
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

        self.tick_link_probation(&ctx);
        self.tick_overlays(&ctx);
        self.update_live_feed(&ctx);
        {
            let mut host = lock(paper);
            self.sync_paper_quotes(&mut host);
            self.configure_paper_cards(&host);
        }
        self.configure_broker_cards();
        self.handle_keyboard(ui);
        self.handle_clipboard(ui);
        self.tick_content(&ctx);
        self.tick_replay();

        ctx.request_repaint_after(Duration::from_millis(250));

        let mut actions = egui::Panel::top("workspace_toolbar")
            .exact_size(ui::kit::bar::HEIGHT)
            .frame(egui::Frame::new().fill(ui::theme::chrome()))
            .show(ui, |ui| self.show_toolbar(ui, &ctx))
            .inner;

        if self.replay.is_bar_visible() {
            egui::Panel::top("replay_bar")
                .exact_size(bar::preferred_height(
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
        } else {
            self.suspend_watchlist_demand();
        }

        if self.trade_sheet_visible {
            let sheet_actions = egui::Panel::right(egui::Id::new(("trade_sheet", self.id)))
                .resizable(true)
                .default_size(340.0)
                .min_size(300.0)
                .max_size(460.0)
                .frame(
                    egui::Frame::new()
                        .fill(ui::kit::window())
                        .inner_margin(egui::Margin::ZERO),
                )
                .show(ui, |ui| self.show_trade_sheet(ui))
                .inner;
            actions.extend(sheet_actions);
        }

        if self.paper_panel_visible {
            let mut panel_actions: Vec<WorkspaceAction> = Vec::new();
            let events = egui::Panel::bottom(egui::Id::new(("paper_panel", self.id)))
                .resizable(true)
                .default_size(340.0)
                .min_size(280.0)
                .frame(egui::Frame::new().fill(ui::kit::window()))
                .show(ui, |ui| {
                    let Some((venue, bar)) = self.panel_venue(ui, &mut panel_actions) else {
                        return Vec::new();
                    };
                    let hub = self.trading_hub.clone();
                    let context = self.broker_panel_context();
                    match (venue.broker(), hub, context) {
                        (Some(_), Some(hub), Some(context)) => {
                            let Some(backend) = hub.backend(venue) else {
                                return Vec::new();
                            };
                            let stats = backend.stats();
                            crate::ui::broker::panel::ui(
                                ui,
                                &mut self.broker_panel,
                                &*backend,
                                context,
                                &bar,
                                stats.as_ref(),
                            )
                        }
                        _ => self.paper_panel.ui(ui, &mut lock(paper), &bar),
                    }
                })
                .inner;
            actions.extend(panel_actions);
            for event in events {
                self.apply_panel_event(event, &mut actions);
            }
        }

        self.show_broker_dialogs(&ctx);
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
        if self.context.stats_chart.is_some() {
            egui::Panel::right(egui::Id::new(("session_stats", self.id)))
                .resizable(true)
                .default_size(340.0)
                .min_size(280.0)
                .max_size(560.0)
                .frame(
                    egui::Frame::new()
                        .fill(ui::theme::chrome())
                        .inner_margin(egui::Margin::ZERO),
                )
                .show(ui, |ui| self.show_stats_panel(ui));
        }
        if self.context.logs.chart.is_some() {
            egui::Panel::right(egui::Id::new(("pine_logs", self.id)))
                .resizable(true)
                .default_size(320.0)
                .min_size(240.0)
                .max_size(640.0)
                .frame(
                    egui::Frame::new()
                        .fill(ui::theme::chrome())
                        .inner_margin(egui::Margin::ZERO),
                )
                .show(ui, |ui| self.show_logs_panel(ui));
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
            .show(ui, |ui| self.render_grid(ui, &mut lock(paper)));
        if let Some(item) = self.open_new_tab_request.take() {
            actions.push(WorkspaceAction::OpenWatchlistInNewTab(item));
        }
        if std::mem::take(&mut self.snapshot.settings_requested) {
            actions.push(WorkspaceAction::OpenSnapshotSettings);
        }

        actions.extend(self.show_symbol_picker(&ctx));
        if self.indicator_picker.is_open() {
            let target = self.indicator_target();
            match self.indicator_picker.show(&ctx, target.as_ref()) {
                Some(indicator_picker::Output::Apply { action, .. }) => {
                    if let Some(chart_id) = self.resolved_focused_id() {
                        self.apply_indicator_action(chart_id, &action);
                    }
                }
                Some(indicator_picker::Output::OpenPineStudio) => {
                    actions.push(WorkspaceAction::OpenPineScripts);
                }
                None => {}
            }
        }
        // Before the sheet, so the popover takes an Esc the sheet would otherwise act on.
        self.show_script_popover(&ctx);
        self.show_overlay_popover(&ctx);
        self.show_drawing_properties(&ctx);
        self.confirm_drawings_removal(&ctx);
        if self.settings_chart.is_some() {
            actions.extend(self.show_settings_window(&ctx));
        }
        if let Some(chart_id) = self.removing_chart {
            self.confirm_chart_removal(&ctx, chart_id);
        }
        actions.extend(self.show_alert_editor(&ctx));
        actions.extend(self.show_context_ui(&ctx, &mut lock(paper)));
        actions.extend(self.take_logs_actions());
        if self.date_sheet.is_open() {
            let timeline = self.replay_timeline();
            if let Some(date_sheet::ReplayDateSheetResult::Start(ms)) =
                date_sheet::show(&ctx, &mut self.date_sheet, &timeline)
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
        self.quote_coordinator.release(editor::quote_owner(self.id));
        self.quote_coordinator
            .release(crate::ui::chart::symbol_picker::quote_owner(self.id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{IndicatorConfig, SavedView};
    use crate::workspace::test_support::empty_workspace;

    #[test]
    fn closing_the_sidebar_stops_its_quote_demand_at_once() {
        let workspace = empty_workspace(1);
        let btc = crate::model::InstrumentId::new(crate::model::DataSource::Binance, "BTCUSDT");
        workspace
            .quote_coordinator
            .set_sidebar_demand(workspace.id, vec![btc], true);
        assert!(workspace.quote_coordinator.has_subscriptions());
        workspace.suspend_watchlist_demand();
        assert!(!workspace.quote_coordinator.has_subscriptions());
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
                    scale_mode: None,
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
                    overlays: Default::default(),
                },
                PersistedChart {
                    id: 2,
                    symbol: "ETHUSDT".into(),
                    source: DataSource::Coinbase,
                    log_scale: true,
                    scale_mode: None,
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
                            margins: crate::model::chart::canvas::ChartMargins {
                                top: 20,
                                bottom: 15,
                                right: 7,
                            },
                            ..crate::model::ChartCanvas::default()
                        },
                    },
                    drawings: Vec::new(),
                    pine_instances: vec![crate::model::pine::instance::PineInstance::new(
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
                    overlays: Default::default(),
                },
            ],
            columns: vec![ChartColumn::new(10, vec![1]), ChartColumn::new(11, vec![2])],
            focused: Some(2),
            sync: ChartSyncSettings {
                horizontal_position: true,
                horizontal_zoom: false,
            },
            active_view: Some(42),
            custom_title: Some("Earnings".into()),
            link: Some(LinkColor::Orange),
        };
        let encoded = serde_json::to_string(&saved).expect("serialize workspace");
        let decoded: PersistedWorkspace =
            serde_json::from_str(&encoded).expect("deserialize workspace");
        assert_eq!(decoded, saved);

        // A row from before custom titles has none and decodes; no title writes no key.
        let mut json: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        json.as_object_mut().unwrap().remove("custom_title");
        let old: PersistedWorkspace = serde_json::from_value(json).expect("old row decodes");
        assert_eq!(old.custom_title, None);
        assert!(
            !serde_json::to_string(&old)
                .unwrap()
                .contains("custom_title")
        );
    }

    #[test]
    fn a_tab_link_survives_a_save_and_old_or_unreadable_values_mean_not_linked() {
        let mut workspace = empty_workspace(8);
        assert_eq!(workspace.persisted().link, None);
        assert!(workspace.set_link(Some(LinkColor::Violet)));
        assert!(!workspace.set_link(Some(LinkColor::Violet)), "no change");
        let saved = workspace.persisted();
        assert_eq!(saved.link, Some(LinkColor::Violet));
        let mut json = serde_json::to_value(&saved).unwrap();
        assert_eq!(json["link"], "violet");
        let decoded: PersistedWorkspace = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(decoded.link, Some(LinkColor::Violet));

        // A row from before Tab Linking has no key, and an unlinked tab writes none.
        json.as_object_mut().unwrap().remove("link");
        let old: PersistedWorkspace = serde_json::from_value(json.clone()).expect("old row");
        assert_eq!(old.link, None);
        assert!(!serde_json::to_string(&old).unwrap().contains("\"link\""));

        // A colour from a newer build, a wrong type or a null must not make the row undecodable:
        // a tab table with a bad row is never overwritten.
        for bad in [
            serde_json::json!("magenta"),
            serde_json::json!(7),
            serde_json::json!({"x": 1}),
            serde_json::Value::Null,
        ] {
            json["link"] = bad;
            let row: PersistedWorkspace = serde_json::from_value(json.clone()).expect("decodes");
            assert_eq!(row.link, None);
        }
    }

    #[test]
    fn the_tab_describes_its_focused_market_and_layout() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(6);
        assert_eq!(workspace.tab_chart(), None, "a blank tab has no market");
        assert_eq!(workspace.focused_instrument(), None);

        let btc = InstrumentId::new(DataSource::Binance, "BTCUSDT");
        let eth = InstrumentId::new(DataSource::Binance, "ETHUSDT");
        workspace.add_chart_for(&btc, &ctx);
        workspace.add_chart_for(&eth, &ctx);
        let eth_id = workspace.charts[1].id;
        workspace.focused = Some(eth_id);
        assert_eq!(workspace.focused_instrument(), Some(eth.clone()));

        workspace.active_view_name = Some("Layout 1".into());
        let chart = workspace.tab_chart().expect("market tab");
        assert_eq!(chart.layout.as_deref(), Some("Layout 1"));
        assert_eq!(chart.symbol, "ETHUSDT");
        assert_eq!(chart.source, DataSource::Binance);

        // A renamed layout shows at once; the symbol is not part of the layout's identity.
        workspace.active_view_name = Some("Swing".into());
        workspace.focused = Some(workspace.charts[0].id);
        let chart = workspace.tab_chart().expect("market tab");
        assert_eq!(chart.layout.as_deref(), Some("Swing"));
        assert_eq!(chart.symbol, "BTCUSDT");

        // A focus that points nowhere falls back to the first chart.
        workspace.focused = Some(9999);
        assert_eq!(workspace.focused_instrument(), Some(btc));
    }

    #[test]
    fn a_custom_title_is_tidied_saved_and_independent_of_the_symbol() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(8);
        workspace.add_chart_for(&InstrumentId::new(DataSource::Binance, "BTCUSDT"), &ctx);
        workspace.set_custom_title(Some("  Swing   setups \n".into()));
        assert_eq!(workspace.custom_title(), Some("Swing setups"));
        assert_eq!(
            workspace.tab_chart().unwrap().custom_title.as_deref(),
            Some("Swing setups")
        );

        let restored = ChartWorkspace::restored(
            workspace.persisted(),
            workspace.watchlist.clone(),
            workspace.quote_book.clone(),
            workspace.quote_coordinator.clone(),
            workspace.store.clone(),
            &ctx,
        );
        assert_eq!(restored.custom_title(), Some("Swing setups"));

        // Another symbol in the same tab keeps the title.
        workspace.charts[0].symbol = "ETHUSDT".into();
        assert_eq!(workspace.custom_title(), Some("Swing setups"));

        workspace.set_custom_title(Some("   ".into()));
        assert_eq!(workspace.custom_title(), None);
        workspace.set_custom_title(Some("x".repeat(500)));
        assert_eq!(
            workspace.custom_title().map(|title| title.chars().count()),
            Some(MAX_CUSTOM_TITLE_CHARS)
        );
        workspace.set_custom_title(None);
        assert_eq!(workspace.custom_title(), None);
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
    fn a_restart_keeps_the_open_layout_and_its_name() {
        let ctx = egui::Context::default();
        let workspace = empty_workspace(8);
        let view: SavedView = serde_json::from_str(
            r#"{"id":77,"name":"Swing setups","timeframe":"Hour","created_at_ms":1000,"charts":[]}"#,
        )
        .expect("layout");
        workspace
            .store
            .lock()
            .expect("store")
            .replace_saved_views(&[view])
            .expect("save layout");

        let restore = |active_view: Option<u64>| {
            let mut saved = workspace.persisted();
            saved.active_view = active_view;
            ChartWorkspace::restored(
                saved,
                workspace.watchlist.clone(),
                workspace.quote_book.clone(),
                workspace.quote_coordinator.clone(),
                workspace.store.clone(),
                &ctx,
            )
        };

        let restored = restore(Some(77));
        assert_eq!(restored.active_view_id, Some(77));
        assert_eq!(restored.active_view_name.as_deref(), Some("Swing setups"));
        assert_eq!(restored.persisted().active_view, Some(77));

        let deleted = restore(Some(78));
        assert_eq!(
            deleted.active_view_id, None,
            "a deleted layout is not reopened"
        );
        assert_eq!(deleted.active_view_name, None);
        assert_eq!(restore(None).active_view_id, None);
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
                .restore_viewport(crate::model::chart::viewport::SavedViewport {
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
}
