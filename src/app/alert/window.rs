//! The Alerts window: the alert centre, the price-alert editor and a script alert's Webhooks…
//! sheet, in a viewport of its own.

use super::fx::SharedFxRates;
use super::registration::AlertRegistration;
use crate::app::AppAction;
use crate::app::now_ms;
use crate::app::window::{raise_viewport, settings_viewport_id};
use crate::host::alert::AlertHost;
use crate::host::pine::subscription::PineSubscriptionHost;
use crate::host::settings::SettingsHost;
use crate::lock::lock;
use crate::model::alert::pine::PineAlertState;
use crate::model::alert::{
    AlertCurrency, AlertRuntimeCommand, AlertRuntimeCommandPayload, AlertState,
};
use crate::model::watchlist::quote::Freshness;
use crate::service::watchlist_quotes::{WatchlistQuoteBook, WatchlistQuoteCoordinator};
use crate::store::SharedStore;
use crate::ui::alert::center::{
    self, AlertCenterAction, AlertCenterInput, AlertCenterState, QuoteSummary,
};
use crate::ui::alert::editor::{self, PriceAlertEditorOutcome, PriceAlertEditorState};
use crate::ui::alert::pine_editor::{self, PineAlertWebhooksOutcome, PineAlertWebhooksState};
use crate::ui::alert::script_list::ScriptAlertRow;
use eframe::egui;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The Alerts window's viewport.
pub(in crate::app) fn alerts_viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("degenview-alerts")
}

/// What the Alerts window keeps across frames.
#[derive(Default)]
struct AlertsWindowState {
    /// The centre's tab, search and dialogs.
    center: AlertCenterState,
    /// The price-alert editor its Edit opened.
    editor: Option<PriceAlertEditorState>,
    /// The Webhooks… sheet open on a script alert, by subscription id.
    script_webhooks: Option<(u64, PineAlertWebhooksState)>,
    /// `chart_id → (symbol_key, timeframe)` of every open chart, refreshed each frame for the
    /// script alerts' status.
    chart_datasets: HashMap<u64, (String, String)>,
}

/// Whether the Alerts window is open, and what it keeps across frames. Cheap to clone; its
/// viewport callback holds one. The app thread and the callback both run on the main thread, one
/// after the other, so the state's one lock is never contended.
#[derive(Clone, Default)]
pub(super) struct AlertsWindow {
    open: Arc<AtomicBool>,
    state: Arc<Mutex<AlertsWindowState>>,
}

impl AlertsWindow {
    /// Opens the window, or raises it when it is already open.
    pub(super) fn open(&self, ctx: &egui::Context) {
        if self.open.swap(true, Ordering::AcqRel) {
            raise_viewport(ctx, alerts_viewport_id());
        }
    }

    pub(super) fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }

    /// The editor open in the window prices its alert in a currency other than the dollar.
    pub(super) fn editor_needs_fx_rates(&self) -> bool {
        self.state.lock().is_ok_and(|state| {
            state
                .editor
                .as_ref()
                .is_some_and(|editor| editor.draft.needs_fx_rates())
        })
    }

    /// What each open chart shows, for the script alerts' status.
    pub(super) fn set_chart_datasets(&self, datasets: HashMap<u64, (String, String)>) {
        if let Ok(mut state) = self.state.lock() {
            state.chart_datasets = datasets;
        }
    }
}

/// What the Alerts window's callback reads and changes; cloned into it each frame.
pub(super) struct AlertsWindowEnv {
    pub(super) window: AlertsWindow,
    pub(super) alert_host: Arc<Mutex<AlertHost>>,
    pub(super) registration: AlertRegistration,
    pub(super) fx: SharedFxRates,
    pub(super) store: SharedStore,
    pub(super) quote_book: WatchlistQuoteBook,
    pub(super) quote_coordinator: WatchlistQuoteCoordinator,
    pub(super) settings_host: Arc<Mutex<SettingsHost>>,
    pub(super) pine_host: Arc<Mutex<PineSubscriptionHost>>,
    pub(super) app_actions: Arc<Mutex<Vec<AppAction>>>,
}

/// Shows the Alerts window (the caller checked it is open).
pub(super) fn sync_viewport(ctx: &egui::Context, env: AlertsWindowEnv) {
    let builder = crate::ui::window::chrome::branded(
        egui::ViewportBuilder::default()
            .with_title("Alerts")
            .with_inner_size([880.0, 600.0])
            .with_min_inner_size([760.0, 480.0]),
    );
    ctx.show_viewport_deferred(alerts_viewport_id(), builder, move |ui, _class| {
        show(ui, &env);
    });
}

/// One frame of the Alerts window.
///
/// Lock order: the store, then the window's state, then the alert host, the Pine host, the
/// Settings host or the registration's locks, one at a time.
fn show(ui: &mut egui::Ui, env: &AlertsWindowEnv) {
    if ui.input(|input| input.viewport().close_requested()) {
        env.window.open.store(false, Ordering::Release);
        if let Ok(store) = env.store.lock()
            && let Ok(mut host) = env.alert_host.lock()
        {
            host.set_alerts_window_active(false, &store, now_ms());
        }
        return;
    }
    let store = lock(&env.store);
    if let Ok(mut host) = env.alert_host.lock() {
        let focused = ui.input(|input| input.viewport().focused.unwrap_or(false));
        host.set_alerts_window_active(focused, &store, now_ms());
    }
    let mut window = lock(&env.window.state);
    let window = &mut *window;
    let script_alerts: Vec<ScriptAlertRow> = store
        .pine_subscriptions_cached()
        .iter()
        .map(|subscription| {
            let dataset = window
                .chart_datasets
                .get(&subscription.chart_id)
                .map(|(key, timeframe)| (key.as_str(), timeframe.as_str()));
            ScriptAlertRow::from_subscription(subscription, dataset)
        })
        .collect();
    let muted = store.muted_alerts_cached();
    let sync_enabled = store.sync_enabled();
    let actions = if let Ok(host) = env.alert_host.lock() {
        let snapshot = host.snapshot();
        let input = AlertCenterInput {
            load_error: host.load_error(),
            alerts: &snapshot.alerts,
            history: &snapshot.history,
            health: &snapshot.health,
            quote: &|key| {
                let quote = env.quote_book.get_by_key(key)?;
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
            utc_offset_secs: crate::service::system::timezone::local_utc_offset_secs(now_ms()),
        };
        center::show(ui, &mut window.center, &input)
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
                if let Ok(host) = env.alert_host.lock() {
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
                if let Ok(mut host) = env.pine_host.lock() {
                    host.reload(&store);
                }
            }
            AlertCenterAction::EditScriptWebhooks(id) => {
                if let Some(subscription) = store
                    .pine_subscriptions()
                    .into_iter()
                    .find(|subscription| subscription.id == id)
                {
                    window.script_webhooks = Some((id, PineAlertWebhooksState::new(&subscription)));
                }
            }
            AlertCenterAction::OpenPineStudio => {
                if let Ok(mut pending) = env.app_actions.lock() {
                    pending.push(AppAction::OpenPine);
                }
            }
            AlertCenterAction::OpenNotificationSettings => {
                if let Ok(mut host) = env.settings_host.lock() {
                    if host.is_open() {
                        raise_viewport(ui.ctx(), settings_viewport_id());
                    }
                    host.set_open(true);
                }
            }
            AlertCenterAction::Edit(id) => {
                if let Ok(host) = env.alert_host.lock()
                    && let Some(alert) = host.snapshot().alerts.iter().find(|alert| alert.id == id)
                {
                    let editor = PriceAlertEditorState::new(
                        alert.asset.clone(),
                        Some(alert.clone()),
                        Some(alert.currency),
                    );
                    window.editor = Some(editor);
                }
            }
            AlertCenterAction::Duplicate(id) => {
                // A copy of the rule, saved at once as a new, active alert.
                let original = env.alert_host.lock().ok().and_then(|host| {
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
                    let baseline = env
                        .quote_book
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
    let slot = &mut window.script_webhooks;
    if let Some((id, sheet)) = slot.as_mut() {
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
                if let Ok(mut host) = env.pine_host.lock() {
                    host.reload(&store);
                }
                *slot = None;
            }
            Some(PineAlertWebhooksOutcome::Cancel) => *slot = None,
            Some(PineAlertWebhooksOutcome::OpenSettings) => {
                if let Ok(mut host) = env.settings_host.lock() {
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
    let slot = &mut window.editor;
    let mut finished = false;
    if let Some(state) = slot.as_mut() {
        let rates = env.fx.with(Clone::clone).unwrap_or_default();
        env.quote_coordinator.set_demand(
            editor::ALERTS_WINDOW_QUOTE_OWNER,
            vec![state.draft.asset().instrument.clone()],
            true,
        );
        state.feed_from_book(&env.quote_book, &rates);
        let endpoints = store.webhook_endpoints_cached();
        let now = now_ms();
        match editor::show(ui.ctx(), state, &endpoints, now) {
            Some(PriceAlertEditorOutcome::Save(alert)) => {
                let baseline = state.draft.current_price();
                env.registration.commit(&store, alert, baseline, now);
                finished = true;
            }
            Some(PriceAlertEditorOutcome::Cancel) => finished = true,
            Some(PriceAlertEditorOutcome::OpenWebhookSettings) => {
                if let Ok(mut host) = env.settings_host.lock() {
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
        env.quote_coordinator
            .release(editor::ALERTS_WINDOW_QUOTE_OWNER);
    }
}
