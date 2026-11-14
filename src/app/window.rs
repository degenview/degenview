use super::AppAction;
use super::DegenViewApp;
use super::alert::commit_price_alert;
use super::mcp::show_mcp_confirmation;
use super::now_ms;
use super::open_system_settings;
use super::tabs::SharedTabs;
use super::tabs::TabContent;
use super::tabs::show_tab;
use crate::host::alert::AlertHost;
use crate::host::mcp::confirmation::ConfirmationHost;
use crate::host::paper::PaperHost;
use crate::host::portfolio::PortfolioHost;
use crate::host::portfolio::card::PortfolioCardEnv;
use crate::host::settings::{SettingsHost, SettingsHostAction};
use crate::host::snapshot::SnapshotHost;
use crate::host::tab::window::TabWindowHost;
use crate::host::update::{Phase, UpdateHost, UpdateView};
use crate::lock::lock;
use crate::model::alert::pine::PineAlertState;
use crate::model::alert::{
    AlertCurrency, AlertRuntimeCommand, AlertRuntimeCommandPayload, AlertState,
};
use crate::model::tab::windows::WindowId;
use crate::model::watchlist::quote::Freshness;
use crate::native_bridge;
use crate::service::alert::background_service::BackgroundAlertService;
use crate::service::watchlist_quotes::WatchlistQuoteBook;
use crate::ui::alert::center::{self, AlertCenterAction, AlertCenterInput, QuoteSummary};
use crate::ui::alert::editor::{self, PriceAlertEditorOutcome, PriceAlertEditorState};
use crate::ui::alert::pine_editor::{self, PineAlertWebhooksOutcome, PineAlertWebhooksState};
use crate::ui::alert::script_list::ScriptAlertRow;
use crate::ui::icons::Icon;
use crate::ui::kit;
use crate::ui::settings::mcp::McpSettingsInput;
use crate::ui::settings::window;
use crate::ui::shortcut;
use crate::ui::window::tab_bar::{self, TabBar, TabView, WindowChoice};
use eframe::egui;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

/// Opens a System Settings pane by its extension id, as `x-apple.systempreferences:` links do.
/// The Settings window's viewport (the same id `sync_settings_viewport` opens it under).
pub(super) fn settings_viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("degenview-settings")
}

/// The Alerts window's viewport (the same id `sync_alerts_viewport` opens it under).
pub(super) fn alerts_viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("degenview-alerts")
}

/// Brings an already-open Settings or Alerts window in front of the tab windows. Asking to open
/// a window that exists but sits behind the main one would otherwise do nothing visible.
pub(super) fn raise_viewport(ctx: &egui::Context, viewport: egui::ViewportId) {
    ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Focus);
    ctx.request_repaint_of(viewport);
}

/// Titles the window, but only when the title changed. Every viewport command asks egui for
/// another frame, so sending the same title each frame kept the app repainting at the display's
/// rate for as long as a window was open.
pub(super) fn set_window_title(ctx: &egui::Context, title: String) {
    let remembered = egui::Id::new(("window_title", ctx.viewport_id()));
    let unchanged = ctx.data(|data| data.get_temp::<String>(remembered).as_ref() == Some(&title));
    if !unchanged {
        ctx.data_mut(|data| data.insert_temp(remembered, title.clone()));
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
    }
}

/// What every window callback needs; cloned into each deferred viewport.
#[derive(Clone)]
pub(super) struct WindowEnv {
    pub(super) tabs: SharedTabs,
    pub(super) portfolio_host: Arc<Mutex<PortfolioHost>>,
    pub(super) card_env: PortfolioCardEnv,
    pub(super) paper_host: Arc<Mutex<PaperHost>>,
    pub(super) trading_hub: Arc<crate::host::trading::TradingHub>,
    pub(super) alert_host: Arc<Mutex<AlertHost>>,
    pub(super) alert_fx_shared: Arc<Mutex<std::collections::HashMap<AlertCurrency, f64>>>,
    pub(super) app_actions: Arc<Mutex<Vec<AppAction>>>,
    pub(super) session_save_error: Arc<Mutex<Option<String>>>,
    pub(super) updates: UpdateHost,
    pub(super) snapshots: SnapshotHost,
    pub(super) mcp_confirmations: Arc<Mutex<ConfirmationHost>>,
    /// The shared quotes the tab strip reads its prices from.
    pub(super) quote_book: WatchlistQuoteBook,
}

/// One window: the tab strip, then the tab showing.
fn show_window(
    ui: &mut egui::Ui,
    env: &WindowEnv,
    window_id: WindowId,
    chart_info: Option<crate::pine_studio::ChartInfo>,
) {
    let ctx = ui.ctx().clone();
    if ui.input(|input| input.viewport().close_requested()) {
        // The app closes the window's tabs (a script with edits asks first) and drops the
        // window itself; until then it stays up.
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        env.tabs.windows().request_close_window(window_id);
        ctx.request_repaint_of(egui::ViewportId::ROOT);
        return;
    }
    let Some(window) = env.tabs.windows().window(window_id) else {
        return;
    };
    crate::ui::window::chrome::handle_resize_edges(&ctx);
    crate::ui::window::chrome::paint_rounded_corners(&ctx);
    native_bridge::center_traffic_lights(tab_bar::HEIGHT);
    let focused = ui.input(|input| input.viewport().focused == Some(true));
    if focused {
        env.tabs.windows().note_focused(window_id);
        // A tab change reaches the root's frame, which publishes the Edit menu's state.
        if env.tabs.note_focused_tab(window.active) {
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        }
    }

    // A tab being dragged is driven from the window that received the press.
    env.tabs.windows().drive_drag(&ctx, window_id);
    let host_drag = env.tabs.windows().drag_view(window_id);
    let drag = tab_bar::DragView {
        dragging: host_drag.dragging,
        incoming: host_drag.incoming.and_then(|(tab, slot, left)| {
            Some(tab_bar::Incoming {
                tab: env.tabs.view(tab)?,
                slot,
                left,
            })
        }),
    };
    let views: Vec<TabView> = window
        .tabs
        .iter()
        .filter_map(|id| env.tabs.view(*id))
        .collect();
    let others: Vec<WindowChoice> = env
        .tabs
        .windows()
        .snapshot()
        .into_iter()
        .filter(|(other, _)| other.id != window_id)
        .filter_map(|(other, _)| {
            Some(WindowChoice {
                id: other.id,
                title: env.tabs.view(other.active)?.title,
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
                quotes: &env.quote_book,
                style: env.tabs.titles().style(),
                update: Some(&update),
            }
            .show(ui)
        })
        .inner;
    let (outer, inner, transient) = ui.input(|input| {
        let viewport = input.viewport();
        let transient =
            [viewport.minimized, viewport.maximized, viewport.fullscreen].contains(&Some(true));
        (viewport.outer_rect, viewport.inner_rect, transient)
    });
    env.tabs.windows().publish_geometry(
        window_id,
        outer,
        inner,
        output.strip,
        output.scroll,
        transient,
    );

    let mut events = output.events;
    // Titles belong to the tabs and the style to the app, not to the window host.
    events.retain(|event| match event {
        tab_bar::TabBarEvent::SetCustomTitle { tab, title } => {
            env.tabs.set_custom_title(*tab, title.clone());
            false
        }
        tab_bar::TabBarEvent::SetLink { tab, color } => {
            env.tabs.set_link(*tab, *color);
            false
        }
        tab_bar::TabBarEvent::SetLinkToggles { color, toggles } => {
            // A failed write leaves the choice in force until the app quits.
            let _ = env.tabs.links().set_toggles(None, *color, *toggles);
            false
        }
        tab_bar::TabBarEvent::SetTitleStyle(style) => {
            // A failed write leaves the choice in force until the app quits.
            let _ = env.tabs.titles().set(None, *style);
            false
        }
        _ => true,
    });
    if events.contains(&tab_bar::TabBarEvent::UpdateClicked) {
        env.updates.activate(&ctx);
    }
    if focused {
        show_restart_prompt(&ctx, env, &update);
        show_mcp_confirmation(&ctx, &env.mcp_confirmations);
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
            env.tabs.windows().cycle_window(window_id, forward);
        }
    }
    if !events.is_empty() {
        env.tabs.windows().apply(window_id, events);
        // New and Close are the app's to carry out.
        ctx.request_repaint_of(egui::ViewportId::ROOT);
    }

    let Some(window) = env.tabs.windows().window(window_id) else {
        return;
    };
    let Some(content) = env.tabs.content(window.active) else {
        return;
    };
    if let Some(view) = env.tabs.view(window.active) {
        set_window_title(&ctx, view.title);
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
            cautious: false,
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

impl DegenViewApp {
    pub(super) fn sync_viewports(&self, ctx: &egui::Context) {
        // What a script alert would watch: the focused chart's market.
        let chart_info = self
            .focused_pine_target()
            .map(|target| crate::pine_studio::ChartInfo {
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
            tabs: self.tabs.shared().clone(),
            portfolio_host: Arc::clone(&self.portfolio),
            card_env: self.card_env.clone(),
            paper_host: Arc::clone(&self.paper),
            trading_hub: Arc::clone(&self.trading_hub),
            alert_host: Arc::clone(&self.alert_host),
            alert_fx_shared: Arc::clone(&self.alert_fx_shared),
            app_actions: Arc::clone(&self.actions),
            session_save_error: Arc::clone(&self.session_save_error),
            updates: self.updates.clone(),
            snapshots: self.snapshots.clone(),
            mcp_confirmations: Arc::clone(&self.mcp_confirmations),
            quote_book: self.quote_book.clone(),
        };
        for (window, placement) in self.tabs.windows().snapshot() {
            let (title, is_scripts) = env
                .tabs
                .content(window.active)
                .and_then(|content| {
                    let content = content.lock().ok()?;
                    Some((content.title(), matches!(&*content, TabContent::Pine(_))))
                })
                .unwrap_or_default();
            // The Pine Studio shrinks to what its panes' minimums allow.
            let min = if is_scripts {
                [620.0, 420.0]
            } else {
                [960.0, 620.0]
            };
            let mut builder = crate::ui::window::chrome::apply(
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
            ctx.show_viewport_deferred(
                TabWindowHost::viewport_id(window_id),
                builder,
                move |ui, _class| {
                    show_window(ui, &env, window_id, chart_info.clone());
                },
            );
        }
        // With a Pine Studio open, ⌘Q is answered by the app (see `request_quit`).
        native_bridge::intercept_quit(self.tabs.slots().iter().any(|tab| {
            tab.content
                .lock()
                .is_ok_and(|content| matches!(&*content, TabContent::Pine(_)))
        }));
    }

    pub(super) fn sync_settings_viewport(&self, ctx: &egui::Context) {
        if !self
            .settings_host
            .lock()
            .map(|host| host.is_open())
            .unwrap_or(false)
        {
            return;
        }
        let host = Arc::clone(&self.settings_host);
        let mcp_host = Arc::clone(&self.mcp_host);
        let alert_host = Arc::clone(&self.alert_host);
        let pine_host = Arc::clone(&self.pine_host);
        let store = Arc::clone(&self.store);
        let background_service = Arc::clone(&self.background_service);
        let runtime_control = Arc::clone(&self.alert_runtime_control);
        let app_actions = Arc::clone(&self.actions);
        let updates = self.updates.clone();
        let sync = self.sync.clone();
        let mcp_confirmations = Arc::clone(&self.mcp_confirmations);
        let trading_hub = Arc::clone(&self.trading_hub);
        let builder = crate::ui::window::chrome::branded(
            egui::ViewportBuilder::default()
                .with_title("Settings")
                .with_inner_size(window::WINDOW_SIZE)
                .with_min_inner_size([520.0, 400.0]),
        );
        ctx.show_viewport_deferred(settings_viewport_id(), builder, move |ui, _class| {
            if ui.input(|input| input.viewport().close_requested()) {
                if let Ok(mut host) = host.lock() {
                    host.set_open(false);
                }
                return;
            }
            if let Ok(mut host) = host.lock() {
                let store = lock(&store);
                let (notifications, health, alerts) = {
                    let alert = lock(&alert_host);
                    (
                        alert.settings().clone(),
                        alert.health().clone(),
                        alert.snapshot().alerts.clone(),
                    )
                };
                let subscriptions = store.pine_subscriptions_cached();
                // How many price and script alerts post to an endpoint, for the delete
                // confirmation.
                let reference_count = |id| {
                    alerts
                        .iter()
                        .filter(|alert| alert.webhook_endpoint_ids.contains(&id))
                        .count()
                        + subscriptions
                            .iter()
                            .filter(|subscription| subscription.webhook_endpoint_ids.contains(&id))
                            .count()
                };
                let now = now_ms();
                // The MCP host is only read here (no store access), so holding the store
                // lock above cannot deadlock; its changes are queued and applied per frame.
                let mcp = {
                    let mcp = lock(&mcp_host);
                    (
                        mcp.status(),
                        mcp.settings().clone(),
                        mcp.clients(),
                        mcp.global_permissions(),
                        mcp.fresh_token().cloned(),
                        mcp.load_error().map(str::to_owned),
                        mcp.notice().map(str::to_owned),
                        mcp.can_reset(),
                        mcp.activity(),
                        mcp.activity_unreadable(),
                        mcp.tls_fingerprint().map(str::to_owned),
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
                    can_reset: mcp.7,
                    notice: mcp.6.as_deref(),
                    activity: &mcp.8,
                    activity_unreadable: mcp.9,
                    tls_fingerprint: mcp.10.as_deref(),
                    now_ms: now,
                    utc_offset_secs: crate::service::system::timezone::local_utc_offset_secs(now),
                };
                let ctx = ui.ctx().clone();
                let actions = host.show(
                    &ctx,
                    |state, input| window::show(ui, state, input),
                    &store,
                    &notifications,
                    &health,
                    mcp_input,
                    &updates,
                    &sync,
                    &trading_hub,
                    now,
                    &reference_count,
                );
                // A question from an MCP client is answered where the user is looking.
                if ui.input(|input| input.viewport().focused == Some(true)) {
                    show_mcp_confirmation(ui.ctx(), &mcp_confirmations);
                }
                for action in actions {
                    match action {
                        SettingsHostAction::Mcp(action) => {
                            if let Ok(mut mcp) = mcp_host.lock() {
                                mcp.queue(action);
                            }
                            ui.ctx().request_repaint();
                        }
                        SettingsHostAction::UpdateNotifications(settings) => {
                            let _ = store.enqueue_alert_command(&AlertRuntimeCommand::new(
                                AlertRuntimeCommandPayload::UpdateSettings(settings),
                                now,
                            ));
                        }
                        SettingsHostAction::QuitForUpdate => {
                            // Through the same question as ⌘Q: edited scripts are asked
                            // about before the app goes.
                            if let Ok(mut pending) = app_actions.lock() {
                                pending.push(AppAction::Quit);
                            }
                            ui.ctx().request_repaint();
                        }
                        SettingsHostAction::OpenLoginItems => {
                            BackgroundAlertService::open_login_items();
                        }
                        SettingsHostAction::OpenNotificationSettings => {
                            open_system_settings("com.apple.Notifications-Settings.extension");
                        }
                        SettingsHostAction::RetryRegistration => {
                            if let Ok(mut control) = runtime_control.lock() {
                                control.lease = None;
                                if let Ok(mut service) = background_service.lock() {
                                    let state = service.retry_registration();
                                    control.retrying(state, now);
                                }
                            }
                        }
                        SettingsHostAction::EndpointDeleted(id) => {
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
                            if subscriptions
                                .iter()
                                .any(|subscription| subscription.webhook_endpoint_ids.contains(&id))
                            {
                                let mut next = subscriptions.to_vec();
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
        });
    }

    pub(super) fn sync_alerts_viewport(&self, ctx: &egui::Context) {
        if !self.alerts_open.load(Ordering::Acquire) {
            return;
        }
        let alert_host = Arc::clone(&self.alert_host);
        let store = Arc::clone(&self.store);
        let quote_book = self.quote_book.clone();
        let settings_host = Arc::clone(&self.settings_host);
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
        let builder = crate::ui::window::chrome::branded(
            egui::ViewportBuilder::default()
                .with_title("Alerts")
                .with_inner_size([880.0, 600.0])
                .with_min_inner_size([760.0, 480.0]),
        );
        ctx.show_viewport_deferred(alerts_viewport_id(), builder, move |ui, _class| {
            if ui.input(|input| input.viewport().close_requested()) {
                alerts_open.store(false, Ordering::Release);
                if let Ok(store) = store.lock()
                    && let Ok(mut host) = alert_host.lock()
                {
                    host.set_alerts_window_active(false, &store, now_ms());
                }
                return;
            }
            let store = lock(&store);
            if let Ok(mut host) = alert_host.lock() {
                let focused = ui.input(|input| input.viewport().focused.unwrap_or(false));
                host.set_alerts_window_active(focused, &store, now_ms());
            }
            let mut state = lock(&alert_center_state);
            let script_alerts: Vec<ScriptAlertRow> = {
                let datasets = chart_datasets
                    .lock()
                    .map(|datasets| datasets.clone())
                    .unwrap_or_default();
                store
                    .pine_subscriptions_cached()
                    .iter()
                    .map(|subscription| {
                        let dataset = datasets
                            .get(&subscription.chart_id)
                            .map(|(key, timeframe)| (key.as_str(), timeframe.as_str()));
                        ScriptAlertRow::from_subscription(subscription, dataset)
                    })
                    .collect()
            };
            let muted = store.muted_alerts_cached();
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
                    webhook_summary: &|event_id| store.webhook_delivery_summary(event_id),
                    script_alerts: &script_alerts,
                    muted: &muted,
                    sync_enabled,
                    now_ms: now_ms(),
                    utc_offset_secs: crate::service::system::timezone::local_utc_offset_secs(
                        now_ms(),
                    ),
                };
                center::show(ui, &mut state, &input)
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
                    AlertCenterAction::OpenPineStudio => {
                        if let Ok(mut pending) = app_actions.lock() {
                            pending.push(AppAction::OpenPine);
                        }
                    }
                    AlertCenterAction::OpenNotificationSettings => {
                        if let Ok(mut host) = settings_host.lock() {
                            if host.is_open() {
                                raise_viewport(ui.ctx(), settings_viewport_id());
                            }
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
                let subscriptions = store.pine_subscriptions_cached();
                let outcome = subscriptions
                    .iter()
                    .find(|subscription| subscription.id == *id)
                    .and_then(|subscription| {
                        pine_editor::show_webhooks(
                            ui.ctx(),
                            sheet,
                            subscription,
                            &store.webhook_endpoints_cached(),
                        )
                    });
                let id = *id;
                match outcome {
                    Some(PineAlertWebhooksOutcome::Save(ids)) => {
                        let mut subscriptions = subscriptions.to_vec();
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
                    Some(PineAlertWebhooksOutcome::OpenSettings) => {
                        if let Ok(mut host) = settings_host.lock() {
                            if host.is_open() {
                                raise_viewport(ui.ctx(), settings_viewport_id());
                            }
                            host.open_webhooks();
                        }
                    }
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
                        editor::ALERTS_WINDOW_QUOTE_OWNER,
                        vec![state.draft.asset().instrument.clone()],
                        true,
                    );
                    state.feed_from_book(&quote_book, &rates);
                    let endpoints = store.webhook_endpoints_cached();
                    let now = now_ms();
                    match editor::show(ui.ctx(), state, &endpoints, now) {
                        Some(PriceAlertEditorOutcome::Save(alert)) => {
                            let baseline = state.draft.current_price();
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
                            if let Ok(mut host) = settings_host.lock() {
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
                    quote_coordinator.release(editor::ALERTS_WINDOW_QUOTE_OWNER);
                }
            }
        });
    }

    /// Shows the Settings window: from the macOS menu, or the strip's button and Ctrl+, elsewhere.
    pub(super) fn open_settings(&self, ctx: &egui::Context) {
        self.open_settings_page(ctx, |host| host.set_open(true));
    }

    /// Opens Settings through `open` (which may pick a page). A window that is already open is
    /// raised, since it may be hidden behind the tab windows.
    pub(super) fn open_settings_page(
        &self,
        ctx: &egui::Context,
        open: impl FnOnce(&mut SettingsHost),
    ) {
        if let Ok(mut host) = self.settings_host.lock() {
            let was_open = host.is_open();
            open(&mut host);
            if was_open {
                raise_viewport(ctx, settings_viewport_id());
            }
        }
    }
}
