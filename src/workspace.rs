use crate::model::alert::{AlertCurrency, PriceAlert};
use crate::model::chart_sync::SyncHub;
use crate::model::watchlist::{EntryId, WatchlistInstrument, now_ms};
use crate::model::watchlist_text::ImportReport;
use crate::model::{
    BuiltIn, ChartColumn, ChartSyncSettings, DataSource, Drawing, DrawingCommand, DrawingStyle,
    InstrumentId, LayoutSnapshot, MagnetMode, ReplayEngine, ReplayStatus, Timeframe, Tool,
};
use crate::paper_host::{PaperHost, PaperPanel, PaperPanelAction};
use crate::portfolio_card_host::PortfolioCardEnv;
use crate::service::live::BinanceLiveFeed;
use crate::service::watchlist_quotes::{WatchlistQuoteBook, WatchlistQuoteCoordinator};
use crate::store::SharedStore;
use crate::store::drawing_template::{DrawingPrefs, DrawingTemplateStore};
use crate::store::watchlist::WatchlistStore;
use crate::ui::add_chart_sheet::{AddChartAction, AddChartSheet};
use crate::ui::alert_editor::{self, PriceAlertEditorState};
use crate::ui::chart_settings::{self, ChartSettingsState};
use crate::ui::drawing_properties::DrawingProperties;
use crate::ui::layout_prompts::NamePrompt;
use crate::ui::paper_chart_trading::PaperOverlayAction;
use crate::ui::pine_settings_popover::PineSettingsPopover;
use crate::ui::replay_date_sheet::{self, ReplayDateSheetState};
use crate::ui::watchlist_sidebar::WatchlistSidebarState;
use crate::ui::{self, CrosshairInfo, replay_bar, tool_sidebar, workspace_toolbar};
use alerts::AlertRequest;
use autosave::AutosaveState;
use chart::{Chart, displayed_candles};
use context::ContextState;
use drawings::is_lockable;
use eframe::egui;
use empty_page::EmptyPageCache;
use serde::{Deserialize, Serialize};
use snapshot::SnapshotState;
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use undo::{Snapshot, UndoEntry};

mod alerts;
mod autosave;
mod chart;
mod charts;
mod content;
mod context;
mod drawings;
mod empty_page;
mod grid;
mod input;
mod layouts;
mod links;
mod live;
mod paper;
mod pine;
mod replay;
mod settings;
mod snapshot;
#[cfg(test)]
mod test_support;
mod tool_rail;
mod toolbar;
mod undo;
mod watchlist;

pub use charts::ChartSummary;
pub use pine::{IndicatorError, IndicatorOutcome, PineChartTarget, PineInstanceView};
// Named only by the MCP host's tests; it is the type of `PineInstanceView::run`.
#[cfg(test)]
pub use pine::PineRunView;

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
    note_tool: Tool,
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
    /// Choosing what a content tool places: the picture that waits, the sheet, the worker.
    content: content::ContentFlow,
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
            note_tool: Tool::Text,
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
            content: content::ContentFlow::default(),
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
        self.tick_content(&ctx);
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
mod tests {
    use super::*;
    use crate::model::{IndicatorConfig, SavedView};
    use crate::workspace::test_support::empty_workspace;

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
}
