mod alert;
mod background;
mod commands;
mod mcp;
mod pine;
mod session;
mod tabs;
mod window;

use crate::app::alert::AlertRuntimeControl;
use crate::app::mcp::McpState;
use crate::app::session::PersistedSpecialTab;
use crate::app::session::PersistedSpecialTabKind;
use crate::app::session::PersistedTab;
use crate::app::session::SessionState;
use crate::app::tabs::TabContent;
use crate::app::tabs::TabSlot;
use crate::app::tabs::TabState;
use crate::app::window::settings_viewport_id;
use crate::host::alert::AlertHost;
use crate::host::alert::runtime::AlertRuntimeRunner;
use crate::host::editor_theme::EditorThemeHost;
use crate::host::paper::PaperHost;
use crate::host::pine::subscription::PineSubscriptionHost;
use crate::host::portfolio::PortfolioHost;
use crate::host::portfolio::card::PortfolioCardEnv;
use crate::host::settings::SettingsHost;
use crate::host::snapshot::SnapshotHost;
use crate::host::tab::link::TabLinkHost;
use crate::host::tab::title::TabTitleHost;
use crate::host::tab::window::Placement;
use crate::host::update::UpdateHost;
use crate::lock::lock;
use crate::model::Timeframe;
use crate::model::alert::{AlertCurrency, AlertRuntimeOwner, AlertState};
use crate::model::watchlist::WatchlistInstrument;
use crate::native_bridge;
use crate::pine_studio::PineStudio;
use crate::service::alert::background_service::BackgroundAlertService;
use crate::service::alert::macos_notifications::{self, Notifier};
use crate::service::market::fx_rates::FxRateService;
use crate::service::portfolio::quotes::PortfolioQuoteFeed;
use crate::service::watchlist_quotes::{WatchlistQuoteBook, WatchlistQuoteCoordinator};
use crate::store::SharedStore;
use crate::store::watchlist::WatchlistStore;
use crate::ui::alert::banner::{self, AlertBannerState};
use crate::ui::alert::center::AlertCenterState;
use crate::ui::alert::editor::PriceAlertEditorState;
use crate::ui::alert::pine_editor::PineAlertWebhooksState;
use crate::ui::portfolio::tab::PortfolioTab;
use crate::workspace::ChartWorkspace;
use eframe::egui;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

#[derive(Clone, Debug, PartialEq)]
enum AppAction {
    /// Quit, after asking about unsaved scripts.
    Quit,
    /// A Pine Studio answered the "save changes?" question a quit raised.
    QuitDecided {
        proceed: bool,
    },
    OpenPortfolio,
    OpenPine,
    /// Open the Pine Studio on the source of a Pine logs message.
    OpenPineSource(crate::model::pine::logs_view::SourceTarget),
    /// Open Settings on the Alpaca page.
    OpenAlpacaSettings,
    OpenBrokerSettings,
    /// Open Settings on the AI Assistant page.
    OpenAiSettings,
    /// Open Settings on the Snapshots page.
    OpenSnapshotSettings,
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
    /// Add a Pine script to the focused chart (the Pine Studio's "Add to Chart").
    AddPineScript {
        script_id: Option<String>,
        name: String,
        source: String,
    },
    /// Create a Pine alert subscription on the focused chart (the Pine Studio's "Create Alert").
    CreatePineAlert {
        chart_id: u64,
        instance_id: u64,
        script_id: Option<String>,
        script_name: String,
        source: String,
        inputs: std::collections::HashMap<String, crate::model::pine::instance::StoredInput>,
        symbol_key: String,
        timeframe: String,
        note: String,
        webhook_endpoint_ids: Vec<u64>,
    },
}

pub struct DegenViewApp {
    tabs: TabState,
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
    /// Live broker connections, shared by every window (never reachable from MCP or scripts).
    broker_host: Arc<crate::host::broker::BrokerHost>,
    /// The venues the user trades on (paper and each connected broker), shared by every window and
    /// Settings: the one owner of a venue's lifecycle.
    trading_hub: Arc<crate::host::trading::TradingHub>,
    settings_host: Arc<Mutex<SettingsHost>>,
    /// What the user chose about chart snapshots, shared by every window and Settings.
    snapshots: SnapshotHost,
    /// The editor's themes and typography, shared by every window and Settings.
    editor_themes: EditorThemeHost,
    /// The MCP server, the questions its clients raise, and what the app remembers between
    /// their requests.
    mcp: McpState,
    alert_host: Arc<Mutex<AlertHost>>,
    alert_runner: AlertRuntimeRunner,
    alert_runtime_control: Arc<Mutex<AlertRuntimeControl>>,
    background_service: Arc<Mutex<BackgroundAlertService>>,
    pine_host: Arc<Mutex<PineSubscriptionHost>>,
    alerts_open: Arc<AtomicBool>,
    /// Looks for a newer release on a schedule; the tab bar and Settings ▸ About draw it.
    updates: UpdateHost,
    /// iCloud Sync: applies other Macs' changes and carries out the Settings page's requests.
    sync: crate::host::sync::SyncHost,
    /// The Pine Studio's library as last seen by sync, to notice a save.
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
    session: SessionState,
    /// The root window is a hidden 1×1 surface; it is asked to hide once, because every viewport
    /// command asks egui for another frame.
    root_hidden: bool,
}

impl DegenViewApp {
    pub fn new(
        creation: &eframe::CreationContext<'_>,
        store: SharedStore,
        sync_factory: Option<Arc<dyn crate::sync::transport::TransportFactory>>,
    ) -> Self {
        let app_theme = store
            .lock()
            .ok()
            .and_then(|store| store.setting(crate::ui::theme::THEME_KEY))
            .unwrap_or_default();
        crate::ui::theme::install(&creation.egui_ctx, app_theme);
        egui_extras::install_image_loaders(&creation.egui_ctx);
        crate::service::annotation::asset_host::install(
            &creation.egui_ctx,
            crate::service::annotation::asset_host::AssetHost::new(store.clone()),
        );
        crate::service::cache::control::launch(&creation.egui_ctx);
        native_bridge::install(&creation.egui_ctx);
        if crate::ui::window::chrome::rounded_corners()
            && let Some(render_state) = creation.wgpu_render_state.as_ref()
        {
            crate::ui::window::corners::install(render_state);
        }

        let watchlist = Arc::new(Mutex::new(WatchlistStore::new(store.clone())));
        let quote_book = WatchlistQuoteBook::default();
        let repaint_ctx = creation.egui_ctx.clone();
        let quote_coordinator = WatchlistQuoteCoordinator::new(
            quote_book.clone(),
            Arc::new(move || repaint_ctx.request_repaint()),
        );
        let fx_rates = Arc::new(FxRateService::live(
            crate::service::cache::disk::global().directory(crate::service::cache::disk::Kind::Fx),
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
        // Live trading: shared by every window, outside the paper lock. A broker with saved
        // credentials connects now (on its own thread), so an account is live when it is wanted.
        let broker_host = crate::host::broker::BrokerHost::production(store.clone());
        broker_host.attach(&creation.egui_ctx);
        // The venues the user chose: connects every saved key and each chosen broker that needs none.
        let trading_hub = crate::host::trading::TradingHub::new(
            broker_host.clone(),
            paper.clone(),
            Some(store.clone()),
        );
        trading_hub.connect_enabled();
        let snapshots = SnapshotHost::load(store.clone());
        {
            // The secure storage is asked off the window's thread; a window that showed "checking"
            // is drawn again when it answers.
            let repaint = creation.egui_ctx.clone();
            snapshots
                .secrets()
                .set_on_ready(Arc::new(move || repaint.request_repaint()));
        }
        let tab_title_host = TabTitleHost::load(store.clone());
        let tab_link_host = TabLinkHost::load(store.clone());
        let editor_themes = {
            let loaded = lock(&store).load_editor_themes();
            let repaint = creation.egui_ctx.clone();
            let catalog = crate::service::editor_theme::fonts::FontCatalog::start(
                crate::service::editor_theme::fonts::system_font_dirs(),
                move || repaint.request_repaint(),
            );
            let host = EditorThemeHost::new(loaded, catalog);
            host.attach(&creation.egui_ctx);
            host
        };
        let updates = UpdateHost::load(&store);
        let settings_host = Arc::new(Mutex::new(SettingsHost::new(
            snapshots.clone(),
            editor_themes.clone(),
            tab_title_host.clone(),
        )));
        let mcp = {
            let ctx = creation.egui_ctx.clone();
            McpState::load(
                store.clone(),
                Arc::new(move || ctx.request_repaint()),
                crate::mcp::state::QuoteAccess {
                    book: quote_book.clone(),
                    coordinator: quote_coordinator.clone(),
                },
            )
        };
        let alert_host = Arc::new(Mutex::new({
            let store = lock(&store);
            AlertHost::load(&store, now_ms())
        }));
        let pine_host = Arc::new(Mutex::new({
            let store = lock(&store);
            PineSubscriptionHost::load(&store)
        }));
        let sync = {
            let ctx = creation.egui_ctx.clone();
            let backups = crate::store::data_dir()
                .map(|dir| dir.join("SyncBackups"))
                .unwrap_or_else(|| std::env::temp_dir().join("DegenViewSyncBackups"));
            crate::host::sync::SyncHost::new(
                store.clone(),
                sync_factory,
                crate::store::scripts::ScriptStore::default_location(),
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
            let store = lock(&store);
            (
                store.documents::<PersistedTab>("tab"),
                store.setting::<u64>("session.next_id").unwrap_or(2),
            )
        };

        let (slots, next_id) = if saved_tabs.is_empty() {
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
                        Some(TabSlot::new(TabContent::Pine(Box::new(PineStudio::new(
                            id,
                            store.clone(),
                        )))))
                    }
                    PersistedTab::Special(_) => None,
                })
                .collect();
            (tabs, saved_next_id.max(2))
        };

        let first_window = Placement::for_work_area(native_bridge::main_work_area().map(
            |[x, y, width, height]| {
                egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(width, height))
            },
        ));
        let tabs = TabState::restore(slots, next_id, first_window, tab_title_host, tab_link_host);
        Self {
            tabs,
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
            broker_host,
            trading_hub,
            settings_host,
            snapshots,
            editor_themes,
            mcp,
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
            session: SessionState::new(),
            root_hidden: false,
        }
    }
}

impl eframe::App for DegenViewApp {
    /// Clear where windows round their corners (`ui::window::corners`); otherwise eframe's default.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if crate::ui::window::chrome::rounded_corners() {
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
        self.tabs.tick_quotes(
            ctx,
            &self.quote_book,
            &self.quote_coordinator,
            self.broker_host.revision(),
        );
        self.tick_broker_notices();
        self.broker_host.tick_quotes();
        self.tabs.tick_links(ctx);
        self.tabs.windows().restore_frames(
            ctx,
            native_bridge::screen_layout(),
            now_ms().max(0) as u64,
        );
        if self.session.is_due() {
            self.save_session();
        }
        let now = now_ms();
        self.banner.expire(now);
        banner::show(ctx, &mut self.banner, now);
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
        banner::show(&ctx, &mut self.banner, now);
        if !std::mem::replace(&mut self.root_hidden, true) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
    }

    /// The app is going: start the new version if "Restart Now" was chosen for an update and the quit
    /// was not called off. Only here is exit certain; starting it earlier left a second instance
    /// beside this one whenever the quit was cancelled or slow.
    fn on_exit(&mut self) {
        // Stops the MCP server and writes the activity log's last rows (they are otherwise
        // written about once a second, so a quit would lose the final moments).
        self.mcp.shutdown();
        if let Err(error) = self.updates.relaunch_if_armed() {
            eprintln!("{error}");
        }
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        if let Ok(store) = self.store.lock() {
            self.editor_themes.flush(&store);
        }
        // A layout with Autosave on is written before the session goes, even mid-debounce.
        self.tabs.each_chart(|workspace| {
            workspace.flush_autosave();
        });
        self.save_session();
    }
}

#[cfg(test)]
mod tests;
