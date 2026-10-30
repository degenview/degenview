use crate::host::alert_host::AlertHost;
use crate::host::alert_runtime::{
    AlertRuntimeLease, AlertRuntimeRunner, GuiRuntimeDecision, gui_runtime_decision,
};
use crate::host::editor_theme_host::EditorThemeHost;
use crate::host::mcp_host::{self, McpHost};
use crate::host::paper_host::PaperHost;
use crate::host::pine_subscription_host::PineSubscriptionHost;
use crate::host::portfolio_card_host::PortfolioCardEnv;
use crate::host::portfolio_host::{PortfolioHost, PortfolioTab};
use crate::host::snapshot_host::SnapshotHost;
use crate::host::tab_window_host::{TabRequest, TabWindowHost};
use crate::host::update_host::{Phase, UpdateHost, UpdateView};
use crate::host::webhook_host::{WebhookHost, WebhookHostAction};
use crate::mcp::bridge::{McpCommand, McpError, McpReply};
use crate::mcp::types::{AlertChangeResult, CreateAlertResult, UpdateAlertResult};
use crate::model::Timeframe;
use crate::model::alert::{
    AlertCurrency, AlertRuntimeCommand, AlertRuntimeCommandPayload, AlertRuntimeOwner,
    AlertServiceState, AlertState,
};
use crate::model::pine_alert::{PineAlertState, PineAlertSubscription};
use crate::model::tab_windows::WindowId;
use crate::model::watchlist::{WatchlistInstrument, new_id};
use crate::model::watchlist_quote::Freshness;
use crate::model::webhook::WebhookDeliveryState;
use crate::native_bridge::{self, NativeCommand};
use crate::script_manager::editor::EditAction;
use crate::script_manager::store::hash_source;
use crate::script_manager::{ExitReason, ScriptManager, ScriptManagerAction};
use crate::service::background_service::BackgroundAlertService;
use crate::service::fx_rates::FxRateService;
use crate::service::macos_notifications::{self, Notifier};
use crate::service::portfolio_quotes::PortfolioQuoteFeed;
use crate::service::watchlist_quotes::{WatchlistQuoteBook, WatchlistQuoteCoordinator};
use crate::store::SharedStore;
use crate::store::watchlist::WatchlistStore;
use crate::ui::alert_banner::{self, AlertBannerState};
use crate::ui::alert_center::{
    self, AlertCenterAction, AlertCenterInput, AlertCenterState, QuoteSummary,
};
use crate::ui::alert_editor::{self, PriceAlertEditorOutcome, PriceAlertEditorState};
use crate::ui::icons::Icon;
use crate::ui::kit;
use crate::ui::pine_alert_editor::{self, PineAlertWebhooksOutcome, PineAlertWebhooksState};
use crate::ui::script_alert_list::ScriptAlertRow;
use crate::ui::settings_mcp::McpSettingsInput;
use crate::ui::settings_window;
use crate::ui::shortcut;
use crate::ui::tab_bar::{self, TabBar, TabView, WindowChoice};
use crate::workspace::{ChartWorkspace, PersistedWorkspace, WorkspaceAction};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Opens a System Settings pane by its extension id, as `x-apple.systempreferences:` links do.
/// The Settings window's viewport (the same id `sync_settings_viewport` opens it under).
fn settings_viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("degenview-settings")
}

fn open_system_settings(pane: &str) {
    let _ = std::process::Command::new("/usr/bin/open")
        .arg(format!("x-apple.systempreferences:{pane}"))
        .spawn();
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

struct AlertRuntimeControl {
    lease: Option<AlertRuntimeLease>,
    registration_attempted_at_ms: i64,
    service_state: AlertServiceState,
    last_status_check_ms: i64,
}

impl AlertRuntimeControl {
    fn new(service_state: AlertServiceState, now_ms: i64) -> Self {
        Self {
            lease: None,
            registration_attempted_at_ms: now_ms,
            service_state,
            last_status_check_ms: i64::MIN / 2,
        }
    }

    fn retrying(&mut self, service_state: AlertServiceState, now_ms: i64) {
        self.lease = None;
        self.registration_attempted_at_ms = now_ms;
        self.service_state = service_state;
        self.last_status_check_ms = now_ms;
    }
}

enum TabContent {
    Chart(Box<ChartWorkspace>),
    Portfolio(Box<PortfolioTab>),
    Pine(Box<ScriptManager>),
}

impl TabContent {
    fn id(&self) -> u64 {
        match self {
            Self::Chart(workspace) => workspace.id(),
            Self::Portfolio(portfolio) => portfolio.id(),
            Self::Pine(pine) => pine.id(),
        }
    }

    fn title(&self) -> String {
        match self {
            Self::Chart(workspace) => workspace.title(),
            Self::Portfolio(portfolio) => portfolio.title().to_owned(),
            Self::Pine(pine) => pine.title(),
        }
    }

    fn icon(&self) -> Icon {
        match self {
            Self::Chart(_) => Icon::ChartLine,
            Self::Portfolio(_) => Icon::Briefcase,
            Self::Pine(_) => Icon::Curlybraces,
        }
    }
}

struct TabSlot {
    content: Arc<Mutex<TabContent>>,
}

impl TabSlot {
    fn new(content: TabContent) -> Self {
        Self {
            content: Arc::new(Mutex::new(content)),
        }
    }

    fn id(&self) -> u64 {
        self.content.lock().map_or(0, |content| content.id())
    }
}

/// Every open tab's content by id. The window callbacks read it live (they outlive any one
/// frame's snapshot) because a dragged tab changes windows between frames.
type TabRegistry = Arc<Mutex<std::collections::HashMap<u64, Arc<Mutex<TabContent>>>>>;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum PersistedSpecialTabKind {
    Portfolio,
    Pine,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct PersistedSpecialTab {
    id: u64,
    kind: PersistedSpecialTabKind,
}

/// Untagged keeps every existing chart-tab row byte-compatible. Special tabs have a required
/// `kind`, so they cannot be mistaken for the older `PersistedWorkspace` shape.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
enum PersistedTab {
    Chart(PersistedWorkspace),
    Special(PersistedSpecialTab),
}

impl PersistedTab {
    fn id(&self) -> u64 {
        match self {
            Self::Chart(workspace) => workspace.id,
            Self::Special(tab) => tab.id,
        }
    }
}

impl crate::store::Document for PersistedTab {
    fn document_id(&self) -> String {
        self.id().to_string()
    }
}

#[derive(Clone, Debug, PartialEq)]
enum AppAction {
    /// Quit, after asking about unsaved scripts.
    Quit,
    /// A Script Manager answered the "save changes?" question a quit raised.
    QuitDecided {
        proceed: bool,
    },
    OpenPortfolio,
    OpenPine,
    /// Open the Script Manager on the source of a Pine logs message.
    OpenPineSource(crate::model::pine_logs_view::SourceTarget),
    /// Open Settings on the Alpaca page.
    OpenAlpacaSettings,
    /// Open Settings on the AI Assistant page.
    OpenAiSettings,
    OpenAlerts,
    /// The price-alert sheet in a chart window saved an alert.
    SavePriceAlert {
        alert: Box<crate::model::alert::PriceAlert>,
        baseline: Option<f64>,
    },
    /// The price-alert sheet's "Add in Settings".
    OpenWebhookSettings,
    /// A chart's alert line was paused or resumed.
    SetAlertState {
        id: crate::model::alert::AlertId,
        state: AlertState,
    },
    /// A chart's alert line was deleted.
    DeleteAlert(crate::model::alert::AlertId),
    /// A chart's alert line was dragged to a new level.
    MoveAlert {
        alert: Box<crate::model::alert::PriceAlert>,
        baseline: Option<f64>,
    },
    OpenWatchlistInNewTab(WatchlistInstrument),
    /// Add a Pine script to the focused chart (the Script Manager's "Add to Chart").
    AddPineScript {
        script_id: Option<String>,
        name: String,
        source: String,
    },
    /// Create a Pine alert subscription on the focused chart (the Script Manager's "Create Alert").
    CreatePineAlert {
        chart_id: u64,
        instance_id: u64,
        script_id: Option<String>,
        script_name: String,
        source: String,
        inputs: std::collections::HashMap<String, crate::model::pine_instance::StoredInput>,
        symbol_key: String,
        timeframe: String,
        note: String,
        webhook_endpoint_ids: Vec<u64>,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct PersistedSession {
    next_id: u64,
    tabs: Vec<PersistedTab>,
}

pub struct DegenViewApp {
    tabs: Vec<TabSlot>,
    registry: TabRegistry,
    /// Which tabs share a window; each window shows one tab strip over its active tab.
    tab_host: TabWindowHost,
    /// The id of the tab whose window last had keyboard focus, for Save and other app-level commands.
    focused_tab: Arc<AtomicU64>,
    next_id: u64,
    store: SharedStore,
    watchlist: Arc<Mutex<WatchlistStore>>,
    quote_book: WatchlistQuoteBook,
    quote_coordinator: WatchlistQuoteCoordinator,
    portfolio_quotes: PortfolioQuoteFeed,
    /// How portfolio cards on chart grids reach the portfolios.
    card_env: PortfolioCardEnv,
    fx_rates: Arc<FxRateService>,
    alert_fx_rates: std::collections::HashMap<AlertCurrency, f64>,
    /// `alert_fx_rates` for the chart windows' and Alerts window's price-alert sheets.
    alert_fx_shared: Arc<Mutex<std::collections::HashMap<AlertCurrency, f64>>>,
    alert_fx_receiver: Option<Receiver<std::collections::HashMap<AlertCurrency, f64>>>,
    alert_fx_refreshed_at: Option<Instant>,
    portfolio: Arc<Mutex<PortfolioHost>>,
    paper: Arc<Mutex<PaperHost>>,
    webhook_host: Arc<Mutex<WebhookHost>>,
    /// What the user chose about chart snapshots, shared by every window and Settings.
    snapshots: SnapshotHost,
    /// The editor's themes and typography, shared by every window and Settings.
    editor_themes: EditorThemeHost,
    /// The MCP server's lifecycle, credentials and request queue.
    mcp_host: Arc<Mutex<McpHost>>,
    alert_host: Arc<Mutex<AlertHost>>,
    alert_runner: AlertRuntimeRunner,
    alert_runtime_control: Arc<Mutex<AlertRuntimeControl>>,
    background_service: Arc<Mutex<BackgroundAlertService>>,
    pine_host: Arc<Mutex<PineSubscriptionHost>>,
    alerts_open: Arc<AtomicBool>,
    /// Looks for a newer release on a schedule; the tab bar and Settings ▸ About draw it.
    updates: UpdateHost,
    /// iCloud Sync: applies other Macs' changes and carries out the Settings page's requests.
    sync: crate::host::sync_host::SyncHost,
    /// The Script Manager's library as last seen by sync, to notice a save.
    script_signature: u64,
    alert_editor: Arc<Mutex<Option<PriceAlertEditorState>>>,
    /// The Alerts window's tab, search and dialogs, kept across frames.
    alert_center_state: Arc<Mutex<AlertCenterState>>,
    /// `chart_id → (symbol_key, timeframe)` of every open chart, refreshed each frame for the
    /// Alerts window's script-alert status.
    chart_datasets: Arc<Mutex<std::collections::HashMap<u64, (String, String)>>>,
    /// The Webhooks… sheet open on a script alert, by subscription id.
    script_webhooks: Arc<Mutex<Option<(u64, PineAlertWebhooksState)>>>,
    banner: AlertBannerState,
    notifier: Arc<dyn Notifier>,
    actions: Arc<Mutex<Vec<AppAction>>>,
    session_save_error: Arc<Mutex<Option<String>>>,
    session_last_saved_at: Instant,
    last_saved_session: Option<PersistedSession>,
}

/// Queues a saved price alert for the runtime and makes sure notifications and the background
/// service are ready for it. Shared by the chart windows' sheet and the Alerts window's Edit.
fn commit_price_alert(
    store: &crate::store::Store,
    notifier: &Arc<dyn Notifier>,
    background_service: &Arc<Mutex<BackgroundAlertService>>,
    runtime_control: &Arc<Mutex<AlertRuntimeControl>>,
    alert: Box<crate::model::alert::PriceAlert>,
    baseline: Option<f64>,
    now: i64,
) {
    let _ = store.enqueue_alert_command(&AlertRuntimeCommand::new(
        AlertRuntimeCommandPayload::Save { alert, baseline },
        now,
    ));
    let _ = store.enqueue_alert_command(&AlertRuntimeCommand::new(
        AlertRuntimeCommandPayload::RequestNotificationAuthorization,
        now,
    ));
    let notifier = Arc::clone(notifier);
    std::thread::spawn(move || {
        let _ = notifier.request_authorization();
    });
    let service_state = background_service
        .lock()
        .map(|mut service| service.ensure_registered())
        .unwrap_or(AlertServiceState::Failed);
    if let Ok(mut control) = runtime_control.lock() {
        control.retrying(service_state, now);
    }
}

fn alerts_unavailable() -> McpError {
    McpError::AppUnavailable("Price alerts are unavailable right now".into())
}

/// What every window callback needs; cloned into each deferred viewport.
#[derive(Clone)]
struct WindowEnv {
    host: TabWindowHost,
    registry: TabRegistry,
    focused_tab: Arc<AtomicU64>,
    portfolio_host: Arc<Mutex<PortfolioHost>>,
    card_env: PortfolioCardEnv,
    paper_host: Arc<Mutex<PaperHost>>,
    alert_host: Arc<Mutex<AlertHost>>,
    alert_fx_shared: Arc<Mutex<std::collections::HashMap<AlertCurrency, f64>>>,
    app_actions: Arc<Mutex<Vec<AppAction>>>,
    session_save_error: Arc<Mutex<Option<String>>>,
    updates: UpdateHost,
    snapshots: SnapshotHost,
}

impl WindowEnv {
    fn content(&self, tab: u64) -> Option<Arc<Mutex<TabContent>>> {
        self.registry.lock().ok()?.get(&tab).cloned()
    }

    fn view(&self, tab: u64) -> Option<TabView> {
        let content = self.content(tab)?;
        let content = content.lock().ok()?;
        Some(TabView {
            id: tab,
            title: content.title(),
            icon: content.icon(),
        })
    }
}

/// One window: the tab strip, then the tab showing.
fn show_window(
    ui: &mut egui::Ui,
    env: &WindowEnv,
    window_id: WindowId,
    chart_info: Option<crate::script_manager::ChartInfo>,
) {
    let ctx = ui.ctx().clone();
    if ui.input(|input| input.viewport().close_requested()) {
        // The app closes the window's tabs (a script with edits asks first) and drops the
        // window itself; until then it stays up.
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        env.host.request_close_window(window_id);
        ctx.request_repaint_of(egui::ViewportId::ROOT);
        return;
    }
    let Some(window) = env.host.window(window_id) else {
        return;
    };
    crate::ui::window_chrome::handle_resize_edges(&ctx);
    crate::ui::window_chrome::paint_rounded_corners(&ctx);
    native_bridge::center_traffic_lights(tab_bar::HEIGHT);
    let focused = ui.input(|input| input.viewport().focused == Some(true));
    if focused {
        env.host.note_focused(window_id);
        // A tab change reaches the root's frame, which publishes the Edit menu's state.
        if env.focused_tab.swap(window.active, Ordering::AcqRel) != window.active {
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        }
    }

    // A tab being dragged is driven from the window that received the press.
    env.host.drive_drag(&ctx, window_id);
    let host_drag = env.host.drag_view(window_id);
    let drag = tab_bar::DragView {
        dragging: host_drag.dragging,
        incoming: host_drag.incoming.and_then(|(tab, slot, left)| {
            Some(tab_bar::Incoming {
                tab: env.view(tab)?,
                slot,
                left,
            })
        }),
    };
    let views: Vec<TabView> = window.tabs.iter().filter_map(|id| env.view(*id)).collect();
    let others: Vec<WindowChoice> = env
        .host
        .snapshot()
        .into_iter()
        .filter(|(other, _)| other.id != window_id)
        .filter_map(|(other, _)| {
            Some(WindowChoice {
                id: other.id,
                title: env.view(other.active)?.title,
            })
        })
        .collect();
    let update = env.updates.view_for(&ctx);
    let output = egui::Panel::top("window_tabs")
        .exact_size(tab_bar::HEIGHT)
        .frame(egui::Frame::new())
        .show(ui, |ui| {
            TabBar {
                window: window_id,
                tabs: &views,
                active: window.active,
                drag,
                other_windows: &others,
                update: Some(&update),
            }
            .show(ui)
        })
        .inner;
    let (outer, inner) = ui.input(|input| {
        let viewport = input.viewport();
        (viewport.outer_rect, viewport.inner_rect)
    });
    env.host
        .publish_geometry(window_id, outer, inner, output.strip);

    let mut events = output.events;
    if events.contains(&tab_bar::TabBarEvent::UpdateClicked) {
        env.updates.activate(&ctx);
    }
    if focused {
        show_restart_prompt(&ctx, env, &update);
        let cycle = ui.input_mut(|input| {
            // macOS reaches these two through File ▸ New Tab / Close Tab.
            if !shortcut::is_mac() && shortcut::NEW_TAB.consume(input) {
                events.push(tab_bar::TabBarEvent::New);
            }
            if !shortcut::is_mac() && shortcut::CLOSE_TAB.consume(input) {
                events.push(tab_bar::TabBarEvent::Close(window.active));
            }
            // And Settings through the app menu's Settings…
            if !shortcut::is_mac() && shortcut::OPEN_SETTINGS.consume(input) {
                events.push(tab_bar::TabBarEvent::OpenSettings);
            }
            if shortcut::NEXT_TAB.consume(input) {
                Some(true)
            } else if shortcut::PREVIOUS_TAB.consume(input) {
                Some(false)
            } else {
                None
            }
        });
        if let Some(forward) = cycle {
            env.host.cycle_window(window_id, forward);
        }
    }
    if !events.is_empty() {
        env.host.apply(window_id, events);
        // New and Close are the app's to carry out.
        ctx.request_repaint_of(egui::ViewportId::ROOT);
    }

    let Some(window) = env.host.window(window_id) else {
        return;
    };
    let Some(content) = env.content(window.active) else {
        return;
    };
    if let Some(view) = env.view(window.active) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(view.title));
    }
    ui.push_id(window.active, |ui| {
        show_tab(ui, env, window.active, &content, chart_info);
    });

    if let Ok(error) = env.session_save_error.lock()
        && let Some(message) = error.as_ref()
    {
        egui::Window::new("Could not save session")
            .id(egui::Id::new(("session-save-error", window_id)))
            .collapsible(false)
            .show(&ctx, |ui| {
                ui.label(message);
                ui.label("The app will retry automatically.");
            });
    }
}

/// Asks to restart once an update is installed and ready. Only the focused window asks, so one
/// question shows however many windows are open; "Later" leaves the tab bar's Restart mark.
fn show_restart_prompt(ctx: &egui::Context, env: &WindowEnv, update: &UpdateView) {
    let Phase::Ready { version } = &update.phase else {
        return;
    };
    if !env.updates.restart_prompt_pending() {
        return;
    }
    let message = format!("DegenView {version} is ready. DegenView will close and reopen.");
    let outcome = kit::alert(
        ctx,
        egui::Id::new("update-restart-prompt"),
        &kit::Alert {
            icon: Icon::Cycle,
            tint: kit::ORANGE,
            title: "Restart to finish updating?",
            message: &message,
            accept: "Restart Now",
            dismiss: "Later",
        },
    );
    match outcome {
        // A failed swap turns the tab bar's mark red with the reason; nothing quits.
        Some(kit::ConfirmOutcome::Confirmed) if env.updates.restart().is_ok() => {
            // Through the same question as ⌘Q: edited scripts are asked about before the app goes.
            if let Ok(mut pending) = env.app_actions.lock() {
                pending.push(AppAction::Quit);
            }
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        }
        Some(kit::ConfirmOutcome::Confirmed) => {}
        Some(kit::ConfirmOutcome::Cancelled) => env.updates.dismiss_restart_prompt(),
        None => {}
    }
}

/// The tab showing in a window: its content, and the app actions that content asks for.
fn show_tab(
    ui: &mut egui::Ui,
    env: &WindowEnv,
    id: u64,
    content: &Arc<Mutex<TabContent>>,
    chart_info: Option<crate::script_manager::ChartInfo>,
) {
    let Ok(mut content) = content.lock() else {
        return;
    };
    match &mut *content {
        TabContent::Chart(workspace) => {
            if let Ok(host) = env.alert_host.lock() {
                workspace.set_unseen_alerts(host.unseen().count());
                workspace.set_price_alerts(&host.snapshot().alerts);
            }
            if let Ok(rates) = env.alert_fx_shared.lock() {
                workspace.set_alert_fx_rates(&rates);
            }
            workspace.set_card_env(&env.card_env);
            workspace.set_snapshots(&env.snapshots);
            let actions = env
                .paper_host
                .lock()
                .map(|mut host| workspace.ui(ui, &mut host))
                .unwrap_or_default();
            if !actions.is_empty()
                && let Ok(mut pending) = env.app_actions.lock()
            {
                pending.extend(actions.into_iter().map(|action| match action {
                    WorkspaceAction::OpenPortfolio => AppAction::OpenPortfolio,
                    WorkspaceAction::OpenPineScripts => AppAction::OpenPine,
                    WorkspaceAction::OpenPineSource(target) => AppAction::OpenPineSource(target),
                    WorkspaceAction::OpenAlerts => AppAction::OpenAlerts,
                    WorkspaceAction::SavePriceAlert { alert, baseline } => {
                        AppAction::SavePriceAlert { alert, baseline }
                    }
                    WorkspaceAction::OpenWebhookSettings => AppAction::OpenWebhookSettings,
                    WorkspaceAction::OpenWatchlistInNewTab(item) => {
                        AppAction::OpenWatchlistInNewTab(item)
                    }
                    WorkspaceAction::OpenAlpacaSettings => AppAction::OpenAlpacaSettings,
                    WorkspaceAction::SetAlertState { id, state } => {
                        AppAction::SetAlertState { id, state }
                    }
                    WorkspaceAction::DeleteAlert(id) => AppAction::DeleteAlert(id),
                    WorkspaceAction::MoveAlert { alert, baseline } => {
                        AppAction::MoveAlert { alert, baseline }
                    }
                }));
            }
        }
        TabContent::Portfolio(portfolio) => {
            if let Ok(mut host) = env.portfolio_host.lock() {
                portfolio.ui(ui, &mut host);
            }
        }
        TabContent::Pine(pine) => {
            let pine_actions = pine.ui(ui, chart_info);
            if let Some(decision) = pine.take_exit_decision() {
                match decision.reason {
                    ExitReason::CloseTab if decision.proceed => {
                        env.host.close_tab(id);
                    }
                    ExitReason::CloseTab => {}
                    ExitReason::Quit => {
                        if let Ok(mut pending) = env.app_actions.lock() {
                            pending.push(AppAction::QuitDecided {
                                proceed: decision.proceed,
                            });
                        }
                    }
                }
                ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
            }
            if !pine_actions.is_empty()
                && let Ok(mut pending) = env.app_actions.lock()
            {
                pending.extend(pine_actions.into_iter().map(|action| match action {
                    ScriptManagerAction::AddToChart {
                        script_id,
                        name,
                        source,
                    } => AppAction::AddPineScript {
                        script_id: Some(script_id),
                        name,
                        source,
                    },
                    ScriptManagerAction::CreateAlert {
                        chart_id,
                        instance_id,
                        script_id,
                        script_name,
                        source,
                        inputs,
                        symbol_key,
                        timeframe,
                        note,
                        webhook_endpoint_ids,
                    } => AppAction::CreatePineAlert {
                        chart_id,
                        instance_id,
                        script_id,
                        script_name,
                        source,
                        inputs,
                        symbol_key,
                        timeframe,
                        note,
                        webhook_endpoint_ids,
                    },
                    ScriptManagerAction::OpenAlpacaSettings => AppAction::OpenAlpacaSettings,
                    ScriptManagerAction::OpenAiSettings => AppAction::OpenAiSettings,
                }));
            }
        }
    }
}

impl DegenViewApp {
    pub fn new(
        creation: &eframe::CreationContext<'_>,
        store: SharedStore,
        sync_factory: Option<Arc<dyn crate::host::sync_host::TransportFactory>>,
    ) -> Self {
        let app_theme = store
            .lock()
            .ok()
            .and_then(|store| store.setting(crate::ui::theme::THEME_KEY))
            .unwrap_or_default();
        crate::ui::theme::install(&creation.egui_ctx, app_theme);
        egui_extras::install_image_loaders(&creation.egui_ctx);
        crate::service::asset_host::install(
            &creation.egui_ctx,
            crate::service::asset_host::AssetHost::new(store.clone()),
        );
        crate::service::cache_control::launch(&creation.egui_ctx);
        native_bridge::install(&creation.egui_ctx);
        if crate::ui::window_chrome::rounded_corners()
            && let Some(render_state) = creation.wgpu_render_state.as_ref()
        {
            crate::ui::window_corners::install(render_state);
        }

        let watchlist = Arc::new(Mutex::new(WatchlistStore::new(store.clone())));
        let quote_book = WatchlistQuoteBook::default();
        let repaint_ctx = creation.egui_ctx.clone();
        let quote_coordinator = WatchlistQuoteCoordinator::new(
            quote_book.clone(),
            Arc::new(move || repaint_ctx.request_repaint()),
        );
        let fx_rates = Arc::new(FxRateService::live(
            crate::service::disk_cache::global().directory(crate::service::disk_cache::Kind::Fx),
        ));
        let portfolio_repaint = creation.egui_ctx.clone();
        let portfolio_quotes = PortfolioQuoteFeed::new(
            quote_book.clone(),
            quote_coordinator.clone(),
            fx_rates.clone(),
            Arc::new(move || portfolio_repaint.request_repaint()),
        );

        let portfolio = Arc::new(Mutex::new(PortfolioHost::load(store.clone())));
        let paper = Arc::new(Mutex::new(PaperHost::load(store.clone())));
        let snapshots = SnapshotHost::load(store.clone());
        let editor_themes = {
            let loaded = store
                .lock()
                .expect("store lock poisoned")
                .load_editor_themes();
            let repaint = creation.egui_ctx.clone();
            let catalog = crate::service::font_catalog::FontCatalog::start(
                crate::service::font_catalog::system_font_dirs(),
                move || repaint.request_repaint(),
            );
            let host = EditorThemeHost::new(loaded, catalog);
            host.attach(&creation.egui_ctx);
            host
        };
        let updates = UpdateHost::load(&store);
        let webhook_host = Arc::new(Mutex::new(WebhookHost::new(
            snapshots.clone(),
            editor_themes.clone(),
        )));
        let mcp_host = {
            let ctx = creation.egui_ctx.clone();
            Arc::new(Mutex::new(McpHost::load(
                store.clone(),
                Arc::new(move || ctx.request_repaint()),
            )))
        };
        let alert_host = Arc::new(Mutex::new({
            let store = store.lock().expect("store lock poisoned");
            AlertHost::load(&store, now_ms())
        }));
        let pine_host = Arc::new(Mutex::new({
            let store = store.lock().expect("store lock poisoned");
            PineSubscriptionHost::load(&store)
        }));
        let sync = {
            let ctx = creation.egui_ctx.clone();
            let backups = crate::store::data_dir()
                .map(|dir| dir.join("SyncBackups"))
                .unwrap_or_else(|| std::env::temp_dir().join("DegenViewSyncBackups"));
            crate::host::sync_host::SyncHost::new(
                store.clone(),
                sync_factory,
                crate::script_manager::store::ScriptStore::default_location(),
                backups,
                // The sync page lives in the Settings window, a viewport of its own: waking only the
                // root one leaves that page showing stale state until the mouse moves.
                Arc::new(move || {
                    ctx.request_repaint();
                    ctx.request_repaint_of(settings_viewport_id());
                }),
            )
        };
        let actions = Arc::new(Mutex::new(Vec::new()));
        let alerts_open = Arc::new(AtomicBool::new(false));
        let alert_editor = Arc::new(Mutex::new(None));
        let banner = AlertBannerState::default();
        let notifier = macos_notifications::default_notifier();
        let runtime_started_at = now_ms();
        let has_price_alerts = alert_host
            .lock()
            .is_ok_and(|host| !host.snapshot().alerts.is_empty());
        let mut service = BackgroundAlertService::default();
        let service_state = if has_price_alerts {
            service.ensure_registered()
        } else {
            service.status()
        };
        let background_service = Arc::new(Mutex::new(service));
        let alert_runtime_control = Arc::new(Mutex::new(AlertRuntimeControl::new(
            service_state,
            runtime_started_at,
        )));

        let (saved_tabs, saved_next_id) = {
            let store = store.lock().expect("store lock poisoned");
            (
                store.documents::<PersistedTab>("tab"),
                store.setting::<u64>("session.next_id").unwrap_or(2),
            )
        };

        let (tabs, next_id) = if saved_tabs.is_empty() {
            (
                vec![TabSlot::new(TabContent::Chart(Box::new(
                    ChartWorkspace::empty(
                        1,
                        Timeframe::Day,
                        Arc::clone(&watchlist),
                        quote_book.clone(),
                        quote_coordinator.clone(),
                        store.clone(),
                    ),
                )))],
                2,
            )
        } else {
            let mut has_portfolio = false;
            let mut has_pine = false;
            let tabs = saved_tabs
                .into_iter()
                .filter_map(|saved| match saved {
                    PersistedTab::Chart(workspace) => Some(TabSlot::new(TabContent::Chart(
                        Box::new(ChartWorkspace::restored(
                            workspace,
                            Arc::clone(&watchlist),
                            quote_book.clone(),
                            quote_coordinator.clone(),
                            store.clone(),
                            &creation.egui_ctx,
                        )),
                    ))),
                    PersistedTab::Special(PersistedSpecialTab {
                        id,
                        kind: PersistedSpecialTabKind::Portfolio,
                    }) if !has_portfolio => {
                        has_portfolio = true;
                        Some(TabSlot::new(TabContent::Portfolio(Box::new(
                            PortfolioTab::new(id, portfolio_quotes.clone()),
                        ))))
                    }
                    PersistedTab::Special(PersistedSpecialTab {
                        id,
                        kind: PersistedSpecialTabKind::Pine,
                    }) if !has_pine => {
                        has_pine = true;
                        Some(TabSlot::new(TabContent::Pine(Box::new(
                            ScriptManager::new(id, store.clone()),
                        ))))
                    }
                    PersistedTab::Special(_) => None,
                })
                .collect();
            (tabs, saved_next_id.max(2))
        };

        let tab_host = TabWindowHost::restore(tabs.iter().map(TabSlot::id));
        let registry: TabRegistry = Arc::new(Mutex::new(
            tabs.iter()
                .map(|tab| (tab.id(), Arc::clone(&tab.content)))
                .collect(),
        ));
        Self {
            tabs,
            registry,
            tab_host,
            focused_tab: Arc::new(AtomicU64::new(0)),
            next_id,
            store,
            watchlist,
            quote_book,
            quote_coordinator,
            card_env: PortfolioCardEnv::new(Arc::clone(&portfolio), portfolio_quotes.clone()),
            portfolio_quotes,
            fx_rates,
            alert_fx_rates: std::collections::HashMap::new(),
            alert_fx_shared: Arc::new(Mutex::new(std::collections::HashMap::new())),
            alert_fx_receiver: None,
            alert_fx_refreshed_at: None,
            portfolio,
            paper,
            webhook_host,
            snapshots,
            editor_themes,
            mcp_host,
            alert_host,
            alert_runner: AlertRuntimeRunner::new(AlertRuntimeOwner::App),
            alert_runtime_control,
            background_service,
            pine_host,
            alert_center_state: Arc::new(Mutex::new(AlertCenterState::default())),
            chart_datasets: Arc::new(Mutex::new(std::collections::HashMap::new())),
            script_webhooks: Arc::new(Mutex::new(None)),
            alerts_open,
            updates,
            sync,
            script_signature: 0,
            alert_editor,
            banner,
            notifier,
            actions,
            session_save_error: Arc::new(Mutex::new(None)),
            session_last_saved_at: Instant::now(),
            last_saved_session: None,
        }
    }

    /// Opens `slot` in `window`, or in the window in front, and shows it.
    fn register(&mut self, slot: TabSlot, window: Option<WindowId>) {
        let id = slot.id();
        if let Ok(mut registry) = self.registry.lock() {
            registry.insert(id, Arc::clone(&slot.content));
        }
        match window {
            Some(window) => self.tab_host.add_tab_to(window, id),
            None => {
                self.tab_host.add_tab(id);
            }
        }
        self.tabs.push(slot);
    }

    /// A tab with no chart: it opens on the empty page, as a new Swift tab does.
    fn add_tab(&mut self, window: Option<WindowId>) {
        let id = self.next_id;
        self.next_id += 1;
        let slot = TabSlot::new(TabContent::Chart(Box::new(ChartWorkspace::empty(
            id,
            Timeframe::Day,
            Arc::clone(&self.watchlist),
            self.quote_book.clone(),
            self.quote_coordinator.clone(),
            self.store.clone(),
        ))));
        self.register(slot, window);
    }

    fn add_watchlist_tab(&mut self, item: WatchlistInstrument, ctx: &egui::Context) {
        let id = self.next_id;
        self.next_id += 1;
        let slot = TabSlot::new(TabContent::Chart(Box::new(
            ChartWorkspace::new_for_watchlist(
                id,
                &item,
                Arc::clone(&self.watchlist),
                self.quote_book.clone(),
                self.quote_coordinator.clone(),
                self.store.clone(),
                ctx,
            ),
        )));
        self.register(slot, None);
    }

    /// The focused tab's Script Manager, if that is what has the keyboard, for the Edit menu.
    fn with_focused_script_manager<R>(&self, f: impl FnOnce(&mut ScriptManager) -> R) -> Option<R> {
        let focused = self.focused_tab.load(Ordering::Acquire);
        for tab in &self.tabs {
            let Ok(mut content) = tab.content.lock() else {
                continue;
            };
            if let TabContent::Pine(pine) = &mut *content
                && pine.id() == focused
            {
                return Some(f(pine));
            }
        }
        None
    }

    fn process_commands(&mut self, ctx: &egui::Context) {
        native_bridge::set_edit_menu(
            self.with_focused_script_manager(|pine| pine.edit_menu())
                .unwrap_or_default(),
        );
        native_bridge::set_new_script_enabled(
            self.with_focused_script_manager(|pine| pine.can_create_script())
                .unwrap_or(false),
        );
        for command in native_bridge::take_commands() {
            match command {
                NativeCommand::NewScript => {
                    self.with_focused_script_manager(|pine| pine.new_script());
                    ctx.request_repaint();
                }
                NativeCommand::EditCut
                | NativeCommand::EditCopy
                | NativeCommand::EditPaste
                | NativeCommand::GoToDefinition
                | NativeCommand::RenameSymbol => {
                    let action = match command {
                        NativeCommand::EditCut => EditAction::Cut,
                        NativeCommand::EditCopy => EditAction::Copy,
                        NativeCommand::EditPaste => EditAction::Paste,
                        NativeCommand::GoToDefinition => EditAction::GoToDefinition,
                        _ => EditAction::Rename,
                    };
                    self.with_focused_script_manager(|pine| pine.edit_command(action));
                    ctx.request_repaint();
                }
                NativeCommand::NewTab => {
                    let window = self.tab_host.focused_window();
                    self.add_tab(window);
                }
                NativeCommand::CloseTab => {
                    if let Some(tab) = self.tab_host.focused_tab() {
                        self.close_tab_request(ctx, tab);
                    }
                }
                NativeCommand::MoveTabToNewWindow => self.tab_host.detach_active(),
                NativeCommand::MergeAllWindows => self.tab_host.merge_all(),
                NativeCommand::SelectNextTab => {
                    self.tab_host.cycle(true);
                }
                NativeCommand::SelectPreviousTab => {
                    self.tab_host.cycle(false);
                }
                NativeCommand::OpenSettings => self.open_settings(),
                NativeCommand::Quit => self.request_quit(ctx),
                NativeCommand::SaveCurrentLayout => {
                    let focused = self.focused_tab.load(Ordering::Acquire);
                    // File ▸ Save (⌘S) belongs to the focused tab: a chart saves its layout,
                    // the Script Manager saves the script open in its editor.
                    for tab in &self.tabs {
                        let Ok(mut content) = tab.content.lock() else {
                            continue;
                        };
                        match &mut *content {
                            TabContent::Chart(workspace) if workspace.id() == focused => {
                                workspace.save_layout(ctx);
                                break;
                            }
                            TabContent::Pine(pine) if pine.id() == focused => {
                                pine.request_save();
                                ctx.request_repaint();
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        let actions = self
            .actions
            .lock()
            .map(|mut actions| std::mem::take(&mut *actions))
            .unwrap_or_default();
        for action in actions {
            match action {
                AppAction::Quit => self.request_quit(ctx),
                AppAction::QuitDecided { proceed } => {
                    if proceed {
                        // The next script window with edits asks in turn; none left, and it quits.
                        self.request_quit(ctx);
                    } else {
                        self.call_off_quit();
                    }
                }
                AppAction::OpenPortfolio => self.open_portfolio(ctx),
                AppAction::OpenPine => self.open_pine(ctx),
                AppAction::OpenPineSource(target) => self.open_pine_source(ctx, &target),
                AppAction::OpenAlpacaSettings => {
                    if let Ok(mut host) = self.webhook_host.lock() {
                        host.open_alpaca();
                    }
                }
                AppAction::OpenAiSettings => {
                    if let Ok(mut host) = self.webhook_host.lock() {
                        host.open_ai();
                    }
                }
                AppAction::OpenAlerts => self.alerts_open.store(true, Ordering::Release),
                AppAction::SavePriceAlert { alert, baseline } => {
                    if let Ok(store) = self.store.lock() {
                        commit_price_alert(
                            &store,
                            &self.notifier,
                            &self.background_service,
                            &self.alert_runtime_control,
                            alert,
                            baseline,
                            now_ms(),
                        );
                    }
                }
                AppAction::SetAlertState { id, state } => {
                    if let Ok(store) = self.store.lock() {
                        let _ = store.enqueue_alert_command(&AlertRuntimeCommand::new(
                            AlertRuntimeCommandPayload::SetState {
                                id,
                                state,
                                baseline: None,
                            },
                            now_ms(),
                        ));
                    }
                }
                AppAction::MoveAlert { alert, baseline } => {
                    if let Ok(store) = self.store.lock() {
                        let _ = store.enqueue_alert_command(&AlertRuntimeCommand::new(
                            AlertRuntimeCommandPayload::Save { alert, baseline },
                            now_ms(),
                        ));
                    }
                }
                AppAction::DeleteAlert(id) => {
                    if let Ok(store) = self.store.lock() {
                        let _ = store.enqueue_alert_command(&AlertRuntimeCommand::new(
                            AlertRuntimeCommandPayload::Delete(id),
                            now_ms(),
                        ));
                    }
                }
                AppAction::OpenWebhookSettings => {
                    if let Ok(mut host) = self.webhook_host.lock() {
                        host.open_webhooks();
                    }
                }
                AppAction::OpenWatchlistInNewTab(item) => self.add_watchlist_tab(item, ctx),
                AppAction::AddPineScript {
                    script_id,
                    name,
                    source,
                } => {
                    for tab in &self.tabs {
                        if let Ok(mut content) = tab.content.lock()
                            && let TabContent::Chart(workspace) = &mut *content
                        {
                            workspace.add_pine_on_focused(script_id.clone(), &name, &source);
                            break;
                        }
                    }
                }
                AppAction::CreatePineAlert {
                    chart_id,
                    instance_id,
                    script_id,
                    script_name,
                    source,
                    inputs,
                    symbol_key,
                    timeframe,
                    note,
                    webhook_endpoint_ids,
                } => {
                    let source_hash = hash_source(&source);
                    let subscription = PineAlertSubscription {
                        id: new_id(),
                        chart_id,
                        instance_id: Some(instance_id),
                        script_id,
                        script_name,
                        source,
                        inputs,
                        symbol_key,
                        timeframe,
                        source_hash,
                        note,
                        state: PineAlertState::Active,
                        created_at_ms: now_ms(),
                        webhook_endpoint_ids,
                    };
                    if let Ok(store) = self.store.lock() {
                        let mut subscriptions = store.pine_subscriptions();
                        subscriptions.push(subscription);
                        let _ = store.replace_pine_subscriptions(&subscriptions);
                        // The runtime host reads its own copy; without this a new alert
                        // would stay silent until the next launch.
                        if let Ok(mut host) = self.pine_host.lock() {
                            host.reload(&store);
                        }
                    }
                }
            }
        }
        for request in self.tab_host.take_requests() {
            match request {
                TabRequest::New(window) => self.add_tab(Some(window)),
                TabRequest::Close(tab) => self.close_tab_request(ctx, tab),
                TabRequest::OpenSettings => self.open_settings(),
                TabRequest::CloseWindow(window) => {
                    for tab in self.tab_host.tabs_of(window) {
                        self.close_tab_request(ctx, tab);
                    }
                }
            }
        }
        self.drop_closed_tabs(ctx);
    }

    /// Shows the Settings window: from the macOS menu, or the strip's button and Ctrl+, elsewhere.
    fn open_settings(&self) {
        if let Ok(mut host) = self.webhook_host.lock() {
            host.set_open(true);
        }
    }

    /// Closes `tab`, unless it is a Script Manager with unsaved edits: that one comes to the front
    /// and asks first (its answer closes it through `TabWindowHost::close_tab`).
    fn close_tab_request(&mut self, ctx: &egui::Context, tab: u64) {
        let asking = self.tabs.iter().any(|slot| {
            slot.content
                .lock()
                .is_ok_and(|mut content| match &mut *content {
                    TabContent::Pine(pine) => {
                        pine.id() == tab && pine.begin_exit(ExitReason::CloseTab)
                    }
                    _ => false,
                })
        });
        if asking {
            self.show_tab(ctx, tab);
        } else {
            self.tab_host.close_tab(tab);
        }
    }

    /// Forgets the contents of tabs the window host closed. Without a window left there is
    /// nothing to show, so Windows and Linux quit; macOS stays in the Dock as it always did.
    fn drop_closed_tabs(&mut self, ctx: &egui::Context) {
        let closed = self.tab_host.take_closed();
        if closed.is_empty() {
            return;
        }
        self.tabs.retain(|slot| !closed.contains(&slot.id()));
        if let Ok(mut registry) = self.registry.lock() {
            for id in &closed {
                registry.remove(id);
            }
        }
        if self.tab_host.is_empty() && !cfg!(target_os = "macos") {
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
        }
    }

    /// Brings `tab` to the front of its window and focuses the window.
    fn show_tab(&self, ctx: &egui::Context, tab: u64) {
        if let Some(window) = self.tab_host.select_tab(tab) {
            let viewport = Self::viewport_id(window);
            ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Focus);
            ctx.request_repaint_of(viewport);
        }
    }

    fn open_portfolio(&mut self, ctx: &egui::Context) {
        let existing = self.tabs.iter().find_map(|tab| {
            let content = tab.content.lock().ok()?;
            match &*content {
                TabContent::Portfolio(portfolio) => Some(portfolio.id()),
                TabContent::Chart(_) | TabContent::Pine(_) => None,
            }
        });
        if let Some(id) = existing {
            self.show_tab(ctx, id);
            return;
        }

        let id = self.next_id;
        self.next_id += 1;
        let slot = TabSlot::new(TabContent::Portfolio(Box::new(PortfolioTab::new(
            id,
            self.portfolio_quotes.clone(),
        ))));
        self.register(slot, None);
    }

    /// Quits, but first lets each Script Manager window ask about its unsaved scripts, one window
    /// at a time. Closing the hidden root window ends the app through the same exit path ⌘Q
    /// always took, so the session is saved first.
    /// Saves the editor's themes a moment after they last changed, and picks up a font that has
    /// been read. The store is held only for the write.
    fn tick_editor_themes(&self) {
        if let Ok(store) = self.store.lock() {
            self.editor_themes.tick(&store, Instant::now());
        }
    }

    fn request_quit(&mut self, ctx: &egui::Context) {
        let asking = self
            .tabs
            .iter()
            .find_map(|tab| match &mut *tab.content.lock().ok()? {
                TabContent::Pine(pine) => pine.begin_exit(ExitReason::Quit).then(|| pine.id()),
                _ => None,
            });
        match asking {
            Some(id) => self.show_tab(ctx, id),
            None => ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close),
        }
    }

    /// Cancel or a failed save: the quit stays off and no window keeps asking.
    fn call_off_quit(&mut self) {
        for tab in &self.tabs {
            if let Ok(mut content) = tab.content.lock()
                && let TabContent::Pine(pine) = &mut *content
            {
                pine.cancel_exit();
            }
        }
    }

    fn open_pine(&mut self, ctx: &egui::Context) {
        let existing = self.tabs.iter().find_map(|tab| {
            let content = tab.content.lock().ok()?;
            match &*content {
                TabContent::Pine(pine) => Some(pine.id()),
                TabContent::Chart(_) | TabContent::Portfolio(_) => None,
            }
        });
        if let Some(id) = existing {
            self.show_tab(ctx, id);
            return;
        }

        let id = self.next_id;
        self.next_id += 1;
        let slot = TabSlot::new(TabContent::Pine(Box::new(ScriptManager::new(
            id,
            self.store.clone(),
        ))));
        self.register(slot, None);
    }

    /// Opens the Script Manager (the tab there already is, or a new one) on a log message's call.
    fn open_pine_source(
        &mut self,
        ctx: &egui::Context,
        target: &crate::model::pine_logs_view::SourceTarget,
    ) {
        self.open_pine(ctx);
        for tab in &self.tabs {
            if let Ok(mut content) = tab.content.lock()
                && let TabContent::Pine(pine) = &mut *content
            {
                pine.reveal_source(target);
                break;
            }
        }
    }

    fn viewport_id(window: WindowId) -> egui::ViewportId {
        TabWindowHost::viewport_id(window)
    }

    /// The first chart tab's focused chart identity, for the Pine integration.
    fn focused_pine_target(&self) -> Option<crate::workspace::PineChartTarget> {
        for tab in &self.tabs {
            if let Ok(content) = tab.content.lock()
                && let TabContent::Chart(workspace) = &*content
            {
                return workspace.focused_pine_target();
            }
        }
        None
    }

    /// Applies queued MCP Settings changes and runs the tool requests the server queued. Every
    /// request is checked again just before it runs (see `mcp::bridge::Request::run`).
    fn tick_mcp(&mut self, ctx: &egui::Context) {
        /// Requests run per frame; the rest wait for the next one.
        const REQUESTS_PER_FRAME: usize = 8;
        let (requests, shared) = {
            let Ok(mut host) = self.mcp_host.lock() else {
                return;
            };
            host.tick();
            (host.take_requests(REQUESTS_PER_FRAME), host.shared())
        };
        if requests.len() == REQUESTS_PER_FRAME {
            ctx.request_repaint();
        }
        for request in requests {
            request.run(&shared, |command| self.run_mcp_command(ctx, command));
        }
    }

    fn run_mcp_command(
        &self,
        ctx: &egui::Context,
        command: McpCommand,
    ) -> Result<McpReply, McpError> {
        match command {
            McpCommand::ListWorkspaces => {
                let active = self.focused_tab.load(Ordering::Relaxed);
                let mut workspaces = Vec::new();
                for tab in &self.tabs {
                    let Ok(content) = tab.content.lock() else {
                        continue;
                    };
                    if let TabContent::Chart(workspace) = &*content {
                        workspaces.push(mcp_host::workspace_info(
                            workspace,
                            workspace.id() == active,
                        ));
                    }
                }
                Ok(McpReply::Workspaces(workspaces))
            }
            McpCommand::ListCharts { workspace_id } => self
                .with_mcp_workspace(workspace_id, |workspace| {
                    Ok(McpReply::Charts(mcp_host::charts_result(workspace)))
                }),
            McpCommand::ListAlerts { only } => {
                let host = self.alert_host.lock().map_err(|_| alerts_unavailable())?;
                Ok(McpReply::Alerts(mcp_host::list_alerts(&host, only)))
            }
            McpCommand::CreateAlert(request) => {
                let now = now_ms();
                let plan = {
                    let host = self.alert_host.lock().map_err(|_| alerts_unavailable())?;
                    mcp_host::plan_create_alert(
                        &host,
                        &self.quote_book,
                        &self.alert_fx_rates,
                        &request,
                        now,
                    )?
                };
                match plan {
                    mcp_host::CreatePlan::Existing(alert) => {
                        Ok(McpReply::AlertCreated(CreateAlertResult {
                            status: "already_exists".into(),
                            alert: *alert,
                        }))
                    }
                    mcp_host::CreatePlan::New { alert, baseline } => {
                        let info = mcp_host::alert_info(&alert, None);
                        let store = self.store.lock().map_err(|_| alerts_unavailable())?;
                        // The same commit the price-alert sheet's Save does.
                        commit_price_alert(
                            &store,
                            &self.notifier,
                            &self.background_service,
                            &self.alert_runtime_control,
                            alert,
                            baseline,
                            now,
                        );
                        ctx.request_repaint();
                        Ok(McpReply::AlertCreated(CreateAlertResult {
                            status: "queued".into(),
                            alert: info,
                        }))
                    }
                }
            }
            McpCommand::UpdateAlert(request) => {
                let now = now_ms();
                let plan = {
                    let host = self.alert_host.lock().map_err(|_| alerts_unavailable())?;
                    mcp_host::plan_update_alert(
                        &host,
                        &self.quote_book,
                        &self.alert_fx_rates,
                        &request,
                        now,
                    )?
                };
                match plan {
                    mcp_host::UpdatePlan::Unchanged(alert) => {
                        Ok(McpReply::AlertUpdated(UpdateAlertResult {
                            status: "unchanged".into(),
                            alert: *alert,
                        }))
                    }
                    mcp_host::UpdatePlan::Change { alert, baseline } => {
                        let info = mcp_host::alert_info(&alert, None);
                        self.queue_alert_command(
                            ctx,
                            AlertRuntimeCommandPayload::Save { alert, baseline },
                        )?;
                        Ok(McpReply::AlertUpdated(UpdateAlertResult {
                            status: "queued".into(),
                            alert: info,
                        }))
                    }
                }
            }
            McpCommand::SetAlertPaused { alert_id, paused } => {
                let plan = {
                    let host = self.alert_host.lock().map_err(|_| alerts_unavailable())?;
                    mcp_host::plan_pause_alert(&host, alert_id, paused)?
                };
                let status = match plan {
                    mcp_host::StatePlan::Already if paused => "already_paused",
                    mcp_host::StatePlan::Already => "already_active",
                    mcp_host::StatePlan::Change { id, state } => {
                        self.queue_alert_command(
                            ctx,
                            AlertRuntimeCommandPayload::SetState {
                                id,
                                state,
                                baseline: None,
                            },
                        )?;
                        "queued"
                    }
                };
                Ok(McpReply::AlertChanged(AlertChangeResult {
                    alert_id: alert_id.to_string(),
                    status: status.into(),
                }))
            }
            McpCommand::DeleteAlert { alert_id } => {
                let id = {
                    let host = self.alert_host.lock().map_err(|_| alerts_unavailable())?;
                    mcp_host::plan_delete_alert(&host, alert_id)?
                };
                self.queue_alert_command(ctx, AlertRuntimeCommandPayload::Delete(id))?;
                Ok(McpReply::AlertChanged(AlertChangeResult {
                    alert_id: id.to_string(),
                    status: "queued".into(),
                }))
            }
            McpCommand::ListChartPine {
                workspace_id,
                chart_id,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp_host::list_chart_pine(workspace, chart_id).map(McpReply::ChartPine)
            }),
            McpCommand::GetChartPine {
                workspace_id,
                chart_id,
                instance_id,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp_host::chart_pine_detail(workspace, chart_id, instance_id)
                    .map(McpReply::ChartPineDetail)
            }),
            McpCommand::GetPineStyles {
                workspace_id,
                chart_id,
                instance_id,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp_host::pine_styles(workspace, chart_id, instance_id).map(McpReply::PineStyles)
            }),
            McpCommand::GetPineReport {
                workspace_id,
                chart_id,
                instance_id,
                offset,
                limit,
                include_equity,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp_host::pine_report(
                    workspace,
                    chart_id,
                    instance_id,
                    offset,
                    limit,
                    include_equity,
                )
                .map(|report| McpReply::PineReport(Box::new(report)))
            }),
            McpCommand::EditPineScript {
                script_id,
                expected_revision,
                source,
            } => self
                .edit_pine_script(&script_id, &expected_revision, source.as_deref())
                .map(|result| McpReply::PineEdit(Box::new(result))),
            McpCommand::ListPineAlerts => {
                let store = self.store.lock().map_err(|_| alerts_unavailable())?;
                Ok(McpReply::PineAlerts(mcp_host::pine_alerts(
                    &store.pine_subscriptions(),
                )))
            }
            McpCommand::GetCandles {
                workspace_id,
                chart_id,
                limit,
                before,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp_host::chart_candles(workspace, chart_id, limit, before)
                    .map(|result| McpReply::Candles(Box::new(result)))
            }),
            McpCommand::ListDrawings {
                workspace_id,
                chart_id,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp_host::chart_drawings(workspace, chart_id)
                    .map(|result| McpReply::Drawings(Box::new(result)))
            }),
            McpCommand::AddDrawings {
                workspace_id,
                chart_id,
                drawings,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp_host::add_drawings(workspace, chart_id, &drawings)
                    .map(|result| McpReply::DrawingsChanged(Box::new(result)))
            }),
            McpCommand::EditDrawing {
                workspace_id,
                chart_id,
                edit,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp_host::edit_drawing(workspace, chart_id, &edit)
                    .map(|result| McpReply::DrawingsChanged(Box::new(result)))
            }),
            McpCommand::RemoveDrawings {
                workspace_id,
                chart_id,
                drawing_ids,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp_host::remove_drawings(workspace, chart_id, &drawing_ids)
                    .map(|result| McpReply::DrawingsChanged(Box::new(result)))
            }),
            McpCommand::AddPineInstance {
                workspace_id,
                chart_id,
                script,
                visible,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp_host::add_pine_instance(workspace, chart_id, &script, visible)
                    .map(|result| McpReply::PineInstanceChanged(Box::new(result)))
            }),
            McpCommand::UpdatePineInstance {
                workspace_id,
                chart_id,
                instance_id,
                inputs,
                visible,
            } => {
                let alerts = self.pine_subscriptions_for_mcp()?;
                self.with_mcp_workspace(workspace_id, |workspace| {
                    mcp_host::update_pine_instance(
                        workspace,
                        chart_id,
                        instance_id,
                        inputs.as_ref(),
                        visible,
                        &alerts,
                    )
                    .map(|result| McpReply::PineInstanceChanged(Box::new(result)))
                })
            }
            McpCommand::RemovePineInstance {
                workspace_id,
                chart_id,
                instance_id,
            } => {
                let alerts = self.pine_subscriptions_for_mcp()?;
                self.with_mcp_workspace(workspace_id, |workspace| {
                    mcp_host::remove_pine_instance(workspace, chart_id, instance_id, &alerts)
                        .map(|result| McpReply::PineInstanceChanged(Box::new(result)))
                })
            }
            McpCommand::ApplyIndicator {
                workspace_id,
                chart_id,
                indicator,
                ema_period,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp_host::apply_indicator(workspace, chart_id, indicator, ema_period)
                    .map(McpReply::IndicatorApplied)
            }),
            McpCommand::AddChart {
                workspace_id,
                instrument,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                Ok(McpReply::ChartAdded(mcp_host::add_chart(
                    workspace,
                    &instrument,
                    ctx,
                )))
            }),
        }
    }

    /// The stored script alert subscriptions, for checks that must not disturb a live alert.
    fn pine_subscriptions_for_mcp(
        &self,
    ) -> Result<Vec<crate::model::pine_alert::PineAlertSubscription>, McpError> {
        let store = self.store.lock().map_err(|_| alerts_unavailable())?;
        Ok(store.pine_subscriptions())
    }

    /// The text of `id` in a Script Manager editor that has it open, if any: what the person sees.
    fn open_script_text(&self, id: &str) -> Option<String> {
        self.tabs.iter().find_map(|tab| {
            let content = tab.content.lock().ok()?;
            match &*content {
                TabContent::Pine(manager) => manager.open_text(id),
                _ => None,
            }
        })
    }

    /// Shows an edit in the Script Manager editor that has `id` open. Returns whether one did.
    fn show_in_open_editor(&self, id: &str, text: &str, saved: bool) -> bool {
        self.tabs.iter().any(|tab| {
            tab.content
                .lock()
                .is_ok_and(|mut content| match &mut *content {
                    TabContent::Pine(manager) => manager.adopt_external_text(id, text, saved),
                    _ => false,
                })
        })
    }

    /// Writes a script edit through `ScriptStore`, checked against the text the Script Manager
    /// shows (its open editor if there is one), and updates that editor. `source: Some` is a
    /// draft edit; `None` saves the current draft. Runs on the app thread, so no frame can
    /// type into the editor between the check and the write.
    fn edit_pine_script(
        &self,
        id: &str,
        expected_revision: &str,
        source: Option<&str>,
    ) -> Result<crate::mcp::types::PineEditResult, McpError> {
        let shared = self
            .mcp_host
            .lock()
            .map_err(|_| McpError::AppUnavailable("The MCP host is unavailable".into()))?
            .shared();
        let mut store = shared
            .script_store()
            .ok_or_else(|| McpError::AppUnavailable("The script library is unavailable".into()))?;
        let live = self.open_script_text(id);
        let mut result = match source {
            Some(source) => crate::mcp::pine::update_draft(
                &mut store,
                id,
                expected_revision,
                source,
                live.as_deref(),
            )?,
            None => {
                crate::mcp::pine::save_script(&mut store, id, expected_revision, live.as_deref())?
            }
        };
        if result.status != "unchanged" {
            let (text, saved) = match source {
                Some(text) => (text.to_owned(), false),
                None => (
                    store
                        .script(id)
                        .ok()
                        .flatten()
                        .map(|script| script.source)
                        .unwrap_or_default(),
                    true,
                ),
            };
            result.applied_to_open_editor = self.show_in_open_editor(id, &text, saved);
        } else {
            result.applied_to_open_editor = live.is_some();
        }
        Ok(result)
    }

    /// Hands a change to the alert runtime through the command queue, as the Alerts window does.
    fn queue_alert_command(
        &self,
        ctx: &egui::Context,
        payload: AlertRuntimeCommandPayload,
    ) -> Result<(), McpError> {
        let store = self.store.lock().map_err(|_| alerts_unavailable())?;
        store
            .enqueue_alert_command(&AlertRuntimeCommand::new(payload, now_ms()))
            .map_err(|_| McpError::AppUnavailable("Could not queue the alert change".into()))?;
        ctx.request_repaint();
        Ok(())
    }

    /// Runs `action` on the chart workspace with this id. Portfolio and Script Manager tabs share
    /// the id space but are not chart workspaces.
    fn with_mcp_workspace<R>(
        &self,
        workspace_id: u64,
        action: impl FnOnce(&mut ChartWorkspace) -> Result<R, McpError>,
    ) -> Result<R, McpError> {
        let not_found = || McpError::NotFound(format!("No workspace {workspace_id}"));
        for tab in &self.tabs {
            let mut content = tab
                .content
                .lock()
                .map_err(|_| McpError::AppUnavailable("The workspace is unavailable".into()))?;
            match &mut *content {
                TabContent::Chart(workspace) if workspace.id() == workspace_id => {
                    return action(workspace);
                }
                TabContent::Portfolio(tab) if tab.id() == workspace_id => {
                    return Err(McpError::InvalidArgument(format!(
                        "{workspace_id} is a portfolio tab, not a chart workspace"
                    )));
                }
                TabContent::Pine(manager) if manager.id() == workspace_id => {
                    return Err(McpError::InvalidArgument(format!(
                        "{workspace_id} is the Script Manager, not a chart workspace"
                    )));
                }
                _ => {}
            }
        }
        Err(not_found())
    }

    fn tick_alerts(&mut self) {
        if let Some(receiver) = &self.alert_fx_receiver {
            match receiver.try_recv() {
                Ok(rates) => {
                    if let Ok(mut shared) = self.alert_fx_shared.lock() {
                        shared.clone_from(&rates);
                    }
                    self.alert_fx_rates = rates;
                    self.alert_fx_receiver = None;
                    self.alert_runner.invalidate_quotes();
                }
                Err(mpsc::TryRecvError::Disconnected) => self.alert_fx_receiver = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let needs_fx = self.alert_host.lock().is_ok_and(|host| {
            host.snapshot().alerts.iter().any(|alert| {
                alert.state == AlertState::Active && alert.currency != AlertCurrency::Usd
            })
        }) || self.tabs.iter().any(|tab| {
            tab.content.lock().is_ok_and(|content| {
                matches!(&*content, TabContent::Chart(workspace) if workspace.alert_editor_needs_fx_rates())
            })
        }) || self
            .alert_editor
            .lock()
            .is_ok_and(|slot| slot.as_ref().is_some_and(PriceAlertEditorState::needs_fx_rates));
        if needs_fx
            && self.alert_fx_receiver.is_none()
            && self
                .alert_fx_refreshed_at
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(3600))
        {
            let fx = Arc::clone(&self.fx_rates);
            let (sender, receiver) = mpsc::channel();
            crate::service::work_queue::submit(move || {
                let rates = [
                    AlertCurrency::Eur,
                    AlertCurrency::Gbp,
                    AlertCurrency::Jpy,
                    AlertCurrency::Chf,
                ]
                .into_iter()
                .filter_map(|currency| {
                    fx.rate(
                        crate::model::portfolio::PortfolioCurrency::Usd,
                        match currency {
                            AlertCurrency::Eur => crate::model::portfolio::PortfolioCurrency::Eur,
                            AlertCurrency::Gbp => crate::model::portfolio::PortfolioCurrency::Gbp,
                            AlertCurrency::Jpy => crate::model::portfolio::PortfolioCurrency::Jpy,
                            AlertCurrency::Chf => crate::model::portfolio::PortfolioCurrency::Chf,
                            AlertCurrency::Usd => crate::model::portfolio::PortfolioCurrency::Usd,
                        },
                    )
                    .map(|rate| (currency, rate))
                })
                .collect();
                let _ = sender.send(rates);
            });
            self.alert_fx_receiver = Some(receiver);
            self.alert_fx_refreshed_at = Some(Instant::now());
        }
        let now = now_ms();
        let health = self
            .alert_host
            .lock()
            .map(|host| host.health().clone())
            .unwrap_or_default();
        let (decision, owns_lease, service_state) = {
            let mut control = self
                .alert_runtime_control
                .lock()
                .expect("alert runtime control lock poisoned");
            if now.saturating_sub(control.last_status_check_ms) >= 1_000 {
                control.service_state = self
                    .background_service
                    .lock()
                    .map(|service| service.status())
                    .unwrap_or(AlertServiceState::Failed);
                control.last_status_check_ms = now;
            }
            let decision = gui_runtime_decision(
                control.service_state,
                &health,
                now,
                control.registration_attempted_at_ms,
            );
            if decision != GuiRuntimeDecision::Foreground {
                control.lease = None;
            } else if control.lease.is_none()
                && let Some(directory) = crate::store::data_dir()
            {
                control.lease = AlertRuntimeLease::try_acquire(&directory).ok().flatten();
            }
            (decision, control.lease.is_some(), control.service_state)
        };

        let events = if decision == GuiRuntimeDecision::Foreground && owns_lease {
            let mut host = self.alert_host.lock().expect("alert host lock poisoned");
            let runtime_state = if service_state == AlertServiceState::Enabled {
                AlertServiceState::Disabled
            } else {
                service_state
            };
            self.alert_runner.tick(
                &mut host,
                &self.store,
                &self.quote_book,
                &self.quote_coordinator,
                self.notifier.as_ref(),
                &self.alert_fx_rates,
                runtime_state,
                now,
            )
        } else {
            self.alert_runner.suspend_quotes(&self.quote_coordinator);
            let store = self.store.lock().expect("store lock poisoned");
            self.alert_host
                .lock()
                .expect("alert host lock poisoned")
                .reload_from_store(&store, now)
        };
        if !events.is_empty()
            && let Ok(host) = self.alert_host.lock()
        {
            for event in &events {
                self.banner.show_price(event, host.settings(), now);
            }
        }
    }

    /// Runs active Pine alert subscriptions over every open chart's candles and dispatches any that
    /// fire (webhooks + macOS notification via `dispatch_pine`). Each subscription keeps a
    /// persistent execution controller, so alerts follow the real tick/close lifecycle.
    /// Applies what iCloud Sync fetched and reloads the stores this app holds in memory.
    fn tick_sync(&mut self, ctx: &egui::Context) {
        // Script files change outside the database: tell sync when the library or the text being
        // typed changes.
        let mut unsaved = std::collections::HashSet::new();
        let mut signature = 0u64;
        for slot in &self.tabs {
            if let Ok(content) = slot.content.try_lock()
                && let TabContent::Pine(manager) = &*content
            {
                signature ^= manager.library_signature();
                unsaved.extend(manager.open_unsaved_id());
            }
        }
        if signature != self.script_signature {
            self.script_signature = signature;
            self.sync.note_scripts_changed();
        }
        self.sync.set_unsaved_scripts(unsaved);
        for report in self.sync.tick() {
            if (report.tables.contains("watchlist") || report.settings.contains("watchlist.flags"))
                && let Ok(mut watchlist) = self.watchlist.lock()
            {
                watchlist.reload();
            }
            if report.tables.contains("saved_view") {
                for slot in &self.tabs {
                    if let Ok(mut content) = slot.content.lock()
                        && let TabContent::Chart(workspace) = &mut *content
                    {
                        workspace.adopt_remote_layout();
                    }
                }
            }
            if report.tables.contains("paper_account")
                && let Ok(mut paper) = self.paper.lock()
            {
                paper.reload();
            }
            if (report.tables.contains("portfolio")
                || report.tables.contains("portfolio_transaction"))
                && let Ok(mut portfolio) = self.portfolio.lock()
            {
                portfolio.reload();
            }
            if (report.tables.contains("editor_theme")
                || report
                    .settings
                    .contains(crate::model::editor_theme::prefs::KEY))
                && let Ok(store) = self.store.lock()
            {
                self.editor_themes.reload(&store);
            }
            if report.settings.contains(crate::ui::theme::THEME_KEY) {
                let theme = self
                    .store
                    .lock()
                    .ok()
                    .and_then(|store| store.setting(crate::ui::theme::THEME_KEY))
                    .unwrap_or_default();
                crate::ui::theme::apply(ctx, theme);
            }
            ctx.request_repaint();
        }
    }

    fn tick_pine(&mut self) {
        let store = self.store.lock().expect("store lock poisoned");
        let now = now_ms();
        let mut fired = Vec::new();
        let mut datasets = std::collections::HashMap::new();
        for tab in &self.tabs {
            if let Ok(content) = tab.content.lock()
                && let TabContent::Chart(workspace) = &*content
            {
                for (chart_id, key, timeframe) in workspace.pine_datasets() {
                    datasets.insert(chart_id, (key, timeframe));
                }
            }
        }
        if let Ok(mut shared) = self.chart_datasets.lock() {
            *shared = datasets;
        }
        if let Ok(mut host) = self.pine_host.lock() {
            for tab in &self.tabs {
                if let Ok(content) = tab.content.lock()
                    && let TabContent::Chart(workspace) = &*content
                {
                    for feed in workspace.charts_candles() {
                        host.reconcile_chart(feed.chart_id, &feed.instances, &store);
                        fired.extend(host.run_active_on(
                            feed.chart_id,
                            &feed.symbol_key,
                            &feed.timeframe,
                            &feed.bars,
                            &feed.run,
                            &feed.instances,
                            now,
                        ));
                    }
                }
            }
        }
        if !fired.is_empty()
            && let Ok(host) = self.alert_host.lock()
        {
            host.dispatch_pine(&fired, &store, &self.store, &*self.notifier, now);
        }
    }

    fn sync_viewports(&self, ctx: &egui::Context) {
        // What a script alert would watch: the focused chart's market.
        let chart_info =
            self.focused_pine_target()
                .map(|target| crate::script_manager::ChartInfo {
                    chart_id: target.chart_id,
                    title: target
                        .instrument
                        .source
                        .display_symbol(&target.instrument.symbol),
                    source_name: target.instrument.source.display_name().to_owned(),
                    timeframe: target.timeframe.interval().to_owned(),
                    symbol_key: target.instrument.key(),
                    instances: target.instances,
                });
        let env = WindowEnv {
            host: self.tab_host.clone(),
            registry: Arc::clone(&self.registry),
            focused_tab: Arc::clone(&self.focused_tab),
            portfolio_host: Arc::clone(&self.portfolio),
            card_env: self.card_env.clone(),
            paper_host: Arc::clone(&self.paper),
            alert_host: Arc::clone(&self.alert_host),
            alert_fx_shared: Arc::clone(&self.alert_fx_shared),
            app_actions: Arc::clone(&self.actions),
            session_save_error: Arc::clone(&self.session_save_error),
            updates: self.updates.clone(),
            snapshots: self.snapshots.clone(),
        };
        for (window, placement) in self.tab_host.snapshot() {
            let active = self
                .registry
                .lock()
                .ok()
                .and_then(|registry| registry.get(&window.active).cloned());
            let (title, is_scripts) = active
                .and_then(|content| {
                    let content = content.lock().ok()?;
                    Some((content.title(), matches!(&*content, TabContent::Pine(_))))
                })
                .unwrap_or_default();
            // The Script Manager shrinks to what its panes' minimums allow.
            let min = if is_scripts {
                [620.0, 420.0]
            } else {
                [960.0, 620.0]
            };
            let mut builder = crate::ui::window_chrome::apply(
                egui::ViewportBuilder::default()
                    .with_title(title)
                    .with_inner_size(placement.size)
                    .with_min_inner_size(min),
            );
            if let Some(position) = placement.position {
                builder = builder.with_position(position);
            }
            let env = env.clone();
            let chart_info = chart_info.clone();
            let window_id = window.id;
            ctx.show_viewport_deferred(Self::viewport_id(window_id), builder, move |ui, _class| {
                show_window(ui, &env, window_id, chart_info.clone());
            });
        }
        // With a Script Manager open, ⌘Q is answered by the app (see `request_quit`).
        native_bridge::intercept_quit(self.tabs.iter().any(|tab| {
            tab.content
                .lock()
                .is_ok_and(|content| matches!(&*content, TabContent::Pine(_)))
        }));
    }

    fn sync_settings_viewport(&self, ctx: &egui::Context) {
        if !self
            .webhook_host
            .lock()
            .map(|host| host.is_open())
            .unwrap_or(false)
        {
            return;
        }
        let host = Arc::clone(&self.webhook_host);
        let mcp_host = Arc::clone(&self.mcp_host);
        let alert_host = Arc::clone(&self.alert_host);
        let pine_host = Arc::clone(&self.pine_host);
        let store = Arc::clone(&self.store);
        let background_service = Arc::clone(&self.background_service);
        let runtime_control = Arc::clone(&self.alert_runtime_control);
        let app_actions = Arc::clone(&self.actions);
        let updates = self.updates.clone();
        let sync = self.sync.clone();
        let builder = crate::ui::window_chrome::branded(
            egui::ViewportBuilder::default()
                .with_title("Settings")
                .with_inner_size(settings_window::WINDOW_SIZE)
                .with_min_inner_size([520.0, 400.0]),
        );
        ctx.show_viewport_deferred(
            egui::ViewportId::from_hash_of("degenview-settings"),
            builder,
            move |ui, _class| {
                if ui.input(|input| input.viewport().close_requested()) {
                    if let Ok(mut host) = host.lock() {
                        host.set_open(false);
                    }
                    return;
                }
                if let Ok(mut host) = host.lock() {
                    let store = store.lock().expect("store lock poisoned");
                    let (notifications, health, alerts) = {
                        let alert = alert_host.lock().expect("alert host lock poisoned");
                        (
                            alert.settings().clone(),
                            alert.health().clone(),
                            alert.snapshot().alerts.clone(),
                        )
                    };
                    let subscriptions = store.pine_subscriptions();
                    // How many price and script alerts post to an endpoint, for the delete
                    // confirmation.
                    let reference_count = |id| {
                        alerts
                            .iter()
                            .filter(|alert| alert.webhook_endpoint_ids.contains(&id))
                            .count()
                            + subscriptions
                                .iter()
                                .filter(|subscription| {
                                    subscription.webhook_endpoint_ids.contains(&id)
                                })
                                .count()
                    };
                    let now = now_ms();
                    // The MCP host is only read here (no store access), so holding the store
                    // lock above cannot deadlock; its changes are queued and applied per frame.
                    let mcp = {
                        let mcp = mcp_host.lock().expect("mcp host lock poisoned");
                        (
                            mcp.status(),
                            mcp.settings().clone(),
                            mcp.clients(),
                            mcp.global_permissions(),
                            mcp.fresh_token().cloned(),
                            mcp.load_error().map(str::to_owned),
                            mcp.notice().map(str::to_owned),
                        )
                    };
                    let mcp_input = McpSettingsInput {
                        status: &mcp.0,
                        settings: &mcp.1,
                        clients: &mcp.2,
                        global: &mcp.3,
                        fresh_token: mcp
                            .4
                            .as_ref()
                            .map(|fresh| (fresh.client_name.as_str(), fresh.token.as_str())),
                        load_error: mcp.5.as_deref(),
                        notice: mcp.6.as_deref(),
                        now_ms: now,
                    };
                    let actions = host.show(
                        ui,
                        &store,
                        &notifications,
                        &health,
                        mcp_input,
                        &updates,
                        &sync,
                        now,
                        &reference_count,
                    );
                    for action in actions {
                        match action {
                            WebhookHostAction::Mcp(action) => {
                                if let Ok(mut mcp) = mcp_host.lock() {
                                    mcp.queue(action);
                                }
                                ui.ctx().request_repaint();
                            }
                            WebhookHostAction::UpdateNotifications(settings) => {
                                let _ = store.enqueue_alert_command(&AlertRuntimeCommand::new(
                                    AlertRuntimeCommandPayload::UpdateSettings(settings),
                                    now,
                                ));
                            }
                            WebhookHostAction::QuitForUpdate => {
                                // Through the same question as ⌘Q: edited scripts are asked
                                // about before the app goes.
                                if let Ok(mut pending) = app_actions.lock() {
                                    pending.push(AppAction::Quit);
                                }
                                ui.ctx().request_repaint();
                            }
                            WebhookHostAction::OpenLoginItems => {
                                BackgroundAlertService::open_login_items();
                            }
                            WebhookHostAction::OpenNotificationSettings => {
                                open_system_settings("com.apple.Notifications-Settings.extension");
                            }
                            WebhookHostAction::RetryRegistration => {
                                if let Ok(mut control) = runtime_control.lock() {
                                    control.lease = None;
                                    if let Ok(mut service) = background_service.lock() {
                                        let state = service.retry_registration();
                                        control.retrying(state, now);
                                    }
                                }
                            }
                            WebhookHostAction::EndpointDeleted(id) => {
                                // Alerts stop pointing at a webhook that no longer exists.
                                for alert in alerts
                                    .iter()
                                    .filter(|alert| alert.webhook_endpoint_ids.contains(&id))
                                {
                                    let mut updated = alert.clone();
                                    updated
                                        .webhook_endpoint_ids
                                        .retain(|endpoint| *endpoint != id);
                                    let _ = store.enqueue_alert_command(&AlertRuntimeCommand::new(
                                        AlertRuntimeCommandPayload::Save {
                                            alert: Box::new(updated),
                                            baseline: None,
                                        },
                                        now,
                                    ));
                                }
                                if subscriptions.iter().any(|subscription| {
                                    subscription.webhook_endpoint_ids.contains(&id)
                                }) {
                                    let mut next = subscriptions.clone();
                                    for subscription in &mut next {
                                        subscription
                                            .webhook_endpoint_ids
                                            .retain(|endpoint| *endpoint != id);
                                    }
                                    let _ = store.replace_pine_subscriptions(&next);
                                    if let Ok(mut pine) = pine_host.lock() {
                                        pine.reload(&store);
                                    }
                                }
                            }
                        }
                    }
                }
            },
        );
    }

    fn sync_alerts_viewport(&self, ctx: &egui::Context) {
        if !self.alerts_open.load(Ordering::Acquire) {
            return;
        }
        let alert_host = Arc::clone(&self.alert_host);
        let store = Arc::clone(&self.store);
        let quote_book = self.quote_book.clone();
        let webhook_host = Arc::clone(&self.webhook_host);
        let alerts_open = Arc::clone(&self.alerts_open);
        let alert_editor = Arc::clone(&self.alert_editor);
        let quote_coordinator = self.quote_coordinator.clone();
        let alert_fx_shared = Arc::clone(&self.alert_fx_shared);
        let alert_center_state = Arc::clone(&self.alert_center_state);
        let pine_host = Arc::clone(&self.pine_host);
        let chart_datasets = Arc::clone(&self.chart_datasets);
        let script_webhooks = Arc::clone(&self.script_webhooks);
        let app_actions = Arc::clone(&self.actions);
        let notifier = Arc::clone(&self.notifier);
        let background_service = Arc::clone(&self.background_service);
        let runtime_control = Arc::clone(&self.alert_runtime_control);
        let builder = crate::ui::window_chrome::branded(
            egui::ViewportBuilder::default()
                .with_title("Alerts")
                .with_inner_size([880.0, 600.0])
                .with_min_inner_size([760.0, 480.0]),
        );
        ctx.show_viewport_deferred(
            egui::ViewportId::from_hash_of("degenview-alerts"),
            builder,
            move |ui, _class| {
                if ui.input(|input| input.viewport().close_requested()) {
                    alerts_open.store(false, Ordering::Release);
                    if let Ok(store) = store.lock()
                        && let Ok(mut host) = alert_host.lock()
                    {
                        host.set_alerts_window_active(false, &store, now_ms());
                    }
                    return;
                }
                let store = store.lock().expect("store lock poisoned");
                if let Ok(mut host) = alert_host.lock() {
                    let focused = ui.input(|input| input.viewport().focused.unwrap_or(false));
                    host.set_alerts_window_active(focused, &store, now_ms());
                }
                let mut state = alert_center_state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let script_alerts: Vec<ScriptAlertRow> = {
                    let datasets = chart_datasets
                        .lock()
                        .map(|datasets| datasets.clone())
                        .unwrap_or_default();
                    store
                        .pine_subscriptions()
                        .iter()
                        .map(|subscription| {
                            let dataset = datasets
                                .get(&subscription.chart_id)
                                .map(|(key, timeframe)| (key.as_str(), timeframe.as_str()));
                            ScriptAlertRow::from_subscription(subscription, dataset)
                        })
                        .collect()
                };
                let muted = store.muted_alerts();
                let sync_enabled = store.sync_enabled();
                let actions = if let Ok(host) = alert_host.lock() {
                    let snapshot = host.snapshot();
                    let input = AlertCenterInput {
                        load_error: host.load_error(),
                        alerts: &snapshot.alerts,
                        history: &snapshot.history,
                        health: &snapshot.health,
                        quote: &|key| {
                            let quote = quote_book.get_by_key(key)?;
                            Some(QuoteSummary {
                                price: quote.last?,
                                currency: AlertCurrency::Usd,
                                fresh: quote.freshness == Freshness::Live,
                            })
                        },
                        webhook_summary: &|event_id| {
                            let rows = store.webhook_deliveries(&[event_id]);
                            let rows = rows.get(&event_id)?;
                            let delivered = rows
                                .iter()
                                .filter(|row| row.state == WebhookDeliveryState::Delivered)
                                .count();
                            Some((delivered, rows.len()))
                        },
                        script_alerts: &script_alerts,
                        muted: &muted,
                        sync_enabled,
                        now_ms: now_ms(),
                        utc_offset_secs: crate::service::timezone::local_utc_offset_secs(now_ms()),
                    };
                    alert_center::show(ui, &mut state, &input)
                } else {
                    Vec::new()
                };
                for action in actions {
                    match action {
                        AlertCenterAction::Delete(id) => {
                            let _ = store.enqueue_alert_command(&AlertRuntimeCommand::new(
                                AlertRuntimeCommandPayload::Delete(id),
                                now_ms(),
                            ));
                        }
                        AlertCenterAction::ClearHistory => {
                            let _ = store.enqueue_alert_command(&AlertRuntimeCommand::new(
                                AlertRuntimeCommandPayload::ClearHistory,
                                now_ms(),
                            ));
                        }
                        AlertCenterAction::SetEvaluateHere(id, watch) => {
                            let key = id.to_string();
                            let origin = store
                                .alert_local(&key)
                                .ok()
                                .flatten()
                                .map_or_else(|| "local".to_owned(), |(_, origin)| origin);
                            let _ = store.set_alert_local(&key, watch, &origin);
                        }
                        AlertCenterAction::Toggle(id) => {
                            if let Ok(host) = alert_host.lock() {
                                let state = host
                                    .snapshot()
                                    .alerts
                                    .iter()
                                    .find(|alert| alert.id == id)
                                    .map(|alert| match alert.state {
                                        AlertState::Active => AlertState::Paused,
                                        _ => AlertState::Active,
                                    });
                                if let Some(state) = state {
                                    let _ = store.enqueue_alert_command(&AlertRuntimeCommand::new(
                                        AlertRuntimeCommandPayload::SetState {
                                            id,
                                            state,
                                            baseline: None,
                                        },
                                        now_ms(),
                                    ));
                                }
                            }
                        }
                        AlertCenterAction::PauseScript(id)
                        | AlertCenterAction::RearmScript(id)
                        | AlertCenterAction::DeleteScript(id) => {
                            let mut subscriptions = store.pine_subscriptions();
                            match action {
                                AlertCenterAction::DeleteScript(_) => {
                                    subscriptions.retain(|s| s.id != id);
                                }
                                AlertCenterAction::PauseScript(_) => {
                                    for s in subscriptions.iter_mut().filter(|s| s.id == id) {
                                        s.state = PineAlertState::Paused;
                                    }
                                }
                                _ => {
                                    for s in subscriptions.iter_mut().filter(|s| s.id == id) {
                                        s.state = PineAlertState::Active;
                                    }
                                }
                            }
                            let _ = store.replace_pine_subscriptions(&subscriptions);
                            if let Ok(mut host) = pine_host.lock() {
                                host.reload(&store);
                            }
                        }
                        AlertCenterAction::EditScriptWebhooks(id) => {
                            if let Some(subscription) = store
                                .pine_subscriptions()
                                .into_iter()
                                .find(|subscription| subscription.id == id)
                                && let Ok(mut slot) = script_webhooks.lock()
                            {
                                *slot = Some((id, PineAlertWebhooksState::new(&subscription)));
                            }
                        }
                        AlertCenterAction::OpenScriptManager => {
                            if let Ok(mut pending) = app_actions.lock() {
                                pending.push(AppAction::OpenPine);
                            }
                        }
                        AlertCenterAction::OpenNotificationSettings => {
                            if let Ok(mut host) = webhook_host.lock() {
                                host.set_open(true);
                            }
                        }
                        AlertCenterAction::Edit(id) => {
                            if let Ok(host) = alert_host.lock()
                                && let Some(alert) =
                                    host.snapshot().alerts.iter().find(|alert| alert.id == id)
                            {
                                let editor = PriceAlertEditorState::new(
                                    alert.asset.clone(),
                                    Some(alert.clone()),
                                    Some(alert.currency),
                                );
                                if let Ok(mut slot) = alert_editor.lock() {
                                    *slot = Some(editor);
                                }
                            }
                        }
                        AlertCenterAction::Duplicate(id) => {
                            // A copy of the rule, saved at once as a new, active alert.
                            let original = alert_host.lock().ok().and_then(|host| {
                                host.snapshot()
                                    .alerts
                                    .iter()
                                    .find(|alert| alert.id == id)
                                    .cloned()
                            });
                            if let Some(original) = original {
                                let now = now_ms();
                                let mut copy = crate::model::alert::PriceAlert::new(
                                    original.asset.clone(),
                                    original.condition.clone(),
                                    now,
                                );
                                copy.currency = original.currency;
                                copy.frequency = original.frequency;
                                copy.note = original.note.clone();
                                copy.webhook_endpoint_ids = original.webhook_endpoint_ids.clone();
                                copy.webhook_message = original.webhook_message.clone();
                                let baseline = quote_book
                                    .get_by_key(&original.asset.key())
                                    .and_then(|quote| quote.last);
                                let _ = store.enqueue_alert_command(&AlertRuntimeCommand::new(
                                    AlertRuntimeCommandPayload::Save {
                                        alert: Box::new(copy),
                                        baseline,
                                    },
                                    now,
                                ));
                            }
                        }
                    }
                }
                // Webhooks… sheet of a script alert.
                if let Ok(mut slot) = script_webhooks.lock()
                    && let Some((id, sheet)) = slot.as_mut()
                {
                    let mut subscriptions = store.pine_subscriptions();
                    let outcome = subscriptions
                        .iter()
                        .find(|subscription| subscription.id == *id)
                        .and_then(|subscription| {
                            pine_alert_editor::show_webhooks(
                                ui.ctx(),
                                sheet,
                                subscription,
                                &store.webhook_endpoints(),
                            )
                        });
                    let id = *id;
                    match outcome {
                        Some(PineAlertWebhooksOutcome::Save(ids)) => {
                            for subscription in subscriptions.iter_mut().filter(|s| s.id == id) {
                                subscription.webhook_endpoint_ids = ids.clone();
                            }
                            let _ = store.replace_pine_subscriptions(&subscriptions);
                            if let Ok(mut host) = pine_host.lock() {
                                host.reload(&store);
                            }
                            *slot = None;
                        }
                        Some(PineAlertWebhooksOutcome::Cancel) => *slot = None,
                        None => {}
                    }
                }
                // Price-alert editor window.
                if let Ok(mut slot) = alert_editor.lock() {
                    let mut finished = false;
                    if let Some(state) = slot.as_mut() {
                        let rates = alert_fx_shared
                            .lock()
                            .map(|rates| rates.clone())
                            .unwrap_or_default();
                        quote_coordinator.set_demand(
                            alert_editor::ALERTS_WINDOW_QUOTE_OWNER,
                            vec![state.asset().instrument.clone()],
                            true,
                        );
                        state.feed_from_book(&quote_book, &rates);
                        let endpoints = store.webhook_endpoints();
                        let now = now_ms();
                        match alert_editor::show(ui.ctx(), state, &endpoints, now) {
                            Some(PriceAlertEditorOutcome::Save(alert)) => {
                                let baseline = state.current_price();
                                commit_price_alert(
                                    &store,
                                    &notifier,
                                    &background_service,
                                    &runtime_control,
                                    alert,
                                    baseline,
                                    now,
                                );
                                finished = true;
                            }
                            Some(PriceAlertEditorOutcome::Cancel) => finished = true,
                            Some(PriceAlertEditorOutcome::OpenWebhookSettings) => {
                                if let Ok(mut host) = webhook_host.lock() {
                                    host.open_webhooks();
                                }
                            }
                            None => {}
                        }
                    }
                    if finished {
                        *slot = None;
                    }
                    if slot.is_none() {
                        quote_coordinator.release(alert_editor::ALERTS_WINDOW_QUOTE_OWNER);
                    }
                }
            },
        );
    }

    fn persisted_session(&self) -> Result<PersistedSession, String> {
        // Window order, then strip order: a restore lays the tabs out in the order they were in.
        let ordered = self.tab_host.ordered_tabs();
        let position = |tab: &TabSlot| {
            ordered
                .iter()
                .position(|id| *id == tab.id())
                .unwrap_or(usize::MAX)
        };
        let mut slots: Vec<&TabSlot> = self.tabs.iter().collect();
        slots.sort_by_key(|tab| position(tab));
        let tabs = slots
            .into_iter()
            .map(|tab| {
                let content = tab.content.lock().map_err(|error| error.to_string())?;
                Ok(match &*content {
                    TabContent::Chart(workspace) => PersistedTab::Chart(workspace.persisted()),
                    TabContent::Portfolio(portfolio) => {
                        PersistedTab::Special(PersistedSpecialTab {
                            id: portfolio.id(),
                            kind: PersistedSpecialTabKind::Portfolio,
                        })
                    }
                    TabContent::Pine(pine) => PersistedTab::Special(PersistedSpecialTab {
                        id: pine.id(),
                        kind: PersistedSpecialTabKind::Pine,
                    }),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(PersistedSession {
            next_id: self.next_id,
            tabs,
        })
    }

    fn save_session(&mut self) {
        let result = self.persisted_session().and_then(|session| {
            if self.last_saved_session.as_ref() == Some(&session) {
                return Ok(session);
            }
            self.store
                .lock()
                .map_err(|error| error.to_string())
                .and_then(|store| {
                    store
                        .replace_documents("tab", &session.tabs)
                        .map_err(|error| error.to_string())?;
                    store
                        .set_setting("session.next_id", &session.next_id)
                        .map_err(|error| error.to_string())
                })?;
            Ok(session)
        });
        if let Ok(mut error) = self.session_save_error.lock() {
            *error = result.as_ref().err().cloned();
        }
        if let Ok(session) = result {
            self.last_saved_session = Some(session);
        }
        self.session_last_saved_at = Instant::now();
    }
}

impl eframe::App for DegenViewApp {
    /// Clear where windows round their corners (`ui::window_corners`); otherwise eframe's default.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if crate::ui::window_chrome::rounded_corners() {
            [0.0; 4]
        } else {
            egui::Color32::from_rgba_unmultiplied(12, 12, 12, 180).to_normalized_gamma_f32()
        }
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        crate::ui::theme::sync(ctx);
        self.process_commands(ctx);
        self.sync_viewports(ctx);
        self.sync_settings_viewport(ctx);
        self.sync_alerts_viewport(ctx);
        self.tick_alerts();
        // Runs once per frame (not in `ui`, which also fires for the root viewport).
        self.tick_pine();
        self.tick_mcp(ctx);
        self.updates.tick(ctx);
        self.tick_sync(ctx);
        self.tick_editor_themes();
        if self.session_last_saved_at.elapsed() >= Duration::from_secs(10) {
            self.save_session();
        }
        let now = now_ms();
        self.banner.expire(now);
        alert_banner::show(ctx, &mut self.banner, now);
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.process_commands(&ctx);
        self.sync_viewports(&ctx);
        self.sync_settings_viewport(&ctx);
        self.sync_alerts_viewport(&ctx);
        self.tick_alerts();
        let now = now_ms();
        self.banner.expire(now);
        alert_banner::show(&ctx, &mut self.banner, now);
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        if let Ok(store) = self.store.lock() {
            self.editor_themes.flush(&store);
        }
        // A layout with Autosave on is written before the session goes, even mid-debounce.
        for tab in &self.tabs {
            if let Ok(mut content) = tab.content.lock()
                && let TabContent::Chart(workspace) = &mut *content
            {
                workspace.flush_autosave();
            }
        }
        self.save_session();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ChartColumn, DataSource};
    use crate::workspace::PersistedChart;

    #[test]
    fn persisted_session_round_trips() {
        let session = PersistedSession {
            next_id: 3,
            tabs: vec![PersistedTab::Chart(PersistedWorkspace {
                id: 2,
                timeframe: Timeframe::Hour,
                watchlist_visible: true,
                charts: vec![PersistedChart {
                    id: 1,
                    symbol: "ETHUSDT".into(),
                    source: DataSource::Binance,
                    log_scale: false,
                    scale_mode: None,
                    y_zoom: 1.0,
                    indicators: crate::model::IndicatorConfig::default(),
                    appearance: crate::model::ChartAppearance::default(),
                    drawings: Vec::new(),
                    pine_instances: Vec::new(),
                    candle_count: 0,
                    viewport: Default::default(),
                    title: None,
                    pm_series: Vec::new(),
                    image_url: None,
                    card: None,
                }],
                columns: vec![ChartColumn::new(10, vec![1])],
                focused: Some(1),
                sync: Default::default(),
            })],
        };
        let encoded = serde_json::to_string(&session).expect("serialize session");
        let decoded: PersistedSession =
            serde_json::from_str(&encoded).expect("deserialize session");
        assert_eq!(decoded, session);
    }

    #[test]
    fn old_chart_tab_shape_and_portfolio_tab_both_decode() {
        let old_chart = serde_json::json!({
            "id": 2,
            "timeframe": "Hour",
            "watchlist_visible": true,
            "charts": [],
            "columns": [],
            "focused": null
        });
        let portfolio = serde_json::json!({ "id": 9, "kind": "portfolio" });

        let chart: PersistedTab = serde_json::from_value(old_chart).expect("old chart tab");
        let special: PersistedTab = serde_json::from_value(portfolio).expect("portfolio tab");
        assert!(matches!(chart, PersistedTab::Chart(_)));
        assert_eq!(
            special,
            PersistedTab::Special(PersistedSpecialTab {
                id: 9,
                kind: PersistedSpecialTabKind::Portfolio,
            })
        );
    }
}
