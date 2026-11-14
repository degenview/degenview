//! The app's alert coordinator: the alert host, the runtime this window runs when the login-item
//! agent does not, the FX rates sheets and alerts are priced with, the Alerts window and the
//! banner that shows what fired.

mod fx;
mod registration;
#[cfg(test)]
mod tests;
mod window;

pub(super) use fx::SharedFxRates;

use super::DegenViewApp;
use super::now_ms;
use super::tabs::TabContent;
use crate::host::alert::AlertHost;
use crate::host::alert::runtime::{AlertRuntimeRunner, GuiRuntimeDecision};
use crate::lock::lock;
use crate::model::alert::pine::PineAlertSubscription;
use crate::model::alert::{AlertCurrency, AlertRuntimeOwner, AlertServiceState, AlertState};
use crate::pine::runtime::state::AlertEvent;
use crate::service::alert::background_service::BackgroundAlertService;
use crate::service::alert::macos_notifications::Notifier;
use crate::service::market::fx_rates::FxRateService;
use crate::service::watchlist_quotes::{WatchlistQuoteBook, WatchlistQuoteCoordinator};
use crate::store::{SharedStore, Store};
use crate::ui::alert::banner::{self, AlertBannerState};
use eframe::egui;
use fx::AlertFxFeed;
use registration::{AlertRegistration, AlertRuntimeControl};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use window::{AlertsWindow, AlertsWindowEnv};

/// Price and script alerts at the app level. The alert host and the registration are shared with
/// the windows; the rest is the app thread's own.
pub(super) struct AlertCoordinator {
    host: Arc<Mutex<AlertHost>>,
    runner: AlertRuntimeRunner,
    registration: AlertRegistration,
    fx: AlertFxFeed,
    window: AlertsWindow,
    banner: AlertBannerState,
}

impl AlertCoordinator {
    /// Registers the background service now if any price alert exists (else only asks how it
    /// stands), so the login-item agent can run alerts while the app is closed.
    pub(super) fn new(
        host: AlertHost,
        notifier: Arc<dyn Notifier>,
        fx_rates: Arc<FxRateService>,
    ) -> Self {
        let runtime_started_at = now_ms();
        let has_price_alerts = !host.snapshot().alerts.is_empty();
        let mut service = BackgroundAlertService::default();
        let service_state = if has_price_alerts {
            service.ensure_registered()
        } else {
            service.status()
        };
        let control = AlertRuntimeControl::new(service_state, runtime_started_at);
        Self {
            host: Arc::new(Mutex::new(host)),
            runner: AlertRuntimeRunner::new(AlertRuntimeOwner::App),
            registration: AlertRegistration::new(notifier, service, control),
            fx: AlertFxFeed::new(fx_rates),
            window: AlertsWindow::default(),
            banner: AlertBannerState::default(),
        }
    }

    pub(super) fn host(&self) -> &Arc<Mutex<AlertHost>> {
        &self.host
    }

    pub(super) fn registration(&self) -> &AlertRegistration {
        &self.registration
    }

    /// The FX rates as the app thread last took them.
    pub(super) fn fx_rates(&self) -> &HashMap<AlertCurrency, f64> {
        self.fx.rates()
    }

    /// The FX rates for the windows' price-alert sheets.
    pub(super) fn fx_shared(&self) -> &SharedFxRates {
        self.fx.shared()
    }

    pub(super) fn open_window(&self, ctx: &egui::Context) {
        self.window.open(ctx);
    }

    /// What each open chart shows, for the Alerts window's script alerts.
    pub(super) fn set_chart_datasets(&self, datasets: HashMap<u64, (String, String)>) {
        self.window.set_chart_datasets(datasets);
    }

    /// Delivers the script alerts that fired (webhooks and a notification).
    pub(super) fn dispatch_pine(
        &self,
        fired: &[(PineAlertSubscription, AlertEvent)],
        store: &Store,
        handle: &SharedStore,
        now: i64,
    ) {
        if let Ok(host) = self.host.lock() {
            host.dispatch_pine(fired, store, handle, &**self.registration.notifier(), now);
        }
    }

    /// Takes the banner's expired messages away and draws the rest.
    pub(super) fn show_banner(&mut self, ctx: &egui::Context, now: i64) {
        self.banner.expire(now);
        banner::show(ctx, &mut self.banner, now);
    }

    /// An active alert is priced in a currency other than the dollar.
    fn alerts_need_fx_rates(&self) -> bool {
        self.host.lock().is_ok_and(|host| {
            host.snapshot().alerts.iter().any(|alert| {
                alert.state == AlertState::Active && alert.currency != AlertCurrency::Usd
            })
        })
    }

    /// One turn of the alerts: new FX rates, a fetch when an alert or a sheet (`charts_need_fx`
    /// for the chart windows') needs them, then the runtime when this window runs it, or a reload
    /// of what the agent did. What fired shows in the banner.
    pub(super) fn tick(
        &mut self,
        store: &SharedStore,
        quote_book: &WatchlistQuoteBook,
        quote_coordinator: &WatchlistQuoteCoordinator,
        charts_need_fx: impl FnOnce() -> bool,
    ) {
        if self.fx.poll() {
            self.runner.invalidate_quotes();
        }
        if self.alerts_need_fx_rates() || charts_need_fx() || self.window.editor_needs_fx_rates() {
            self.fx.refresh_if_stale();
        }
        let now = now_ms();
        let health = self
            .host
            .lock()
            .map(|host| host.health().clone())
            .unwrap_or_default();
        let turn = self.registration.decide(&health, now);

        let events = if turn.decision == GuiRuntimeDecision::Foreground && turn.owns_lease {
            let mut host = lock(&self.host);
            let runtime_state = if turn.service_state == AlertServiceState::Enabled {
                AlertServiceState::Disabled
            } else {
                turn.service_state
            };
            self.runner.tick(
                &mut host,
                store,
                quote_book,
                quote_coordinator,
                self.registration.notifier().as_ref(),
                self.fx.rates(),
                runtime_state,
                now,
            )
        } else {
            self.runner.suspend_quotes(quote_coordinator);
            let store = lock(store);
            lock(&self.host).reload_from_store(&store, now)
        };
        if !events.is_empty()
            && let Ok(host) = self.host.lock()
        {
            for event in &events {
                self.banner.show_price(event, host.settings(), now);
            }
        }
    }
}

impl DegenViewApp {
    pub(super) fn tick_alerts(&mut self) {
        let tabs = &self.tabs;
        self.alerts.tick(
            &self.store,
            &self.quote_book,
            &self.quote_coordinator,
            || {
                tabs.slots().iter().any(|tab| {
                    tab.content.lock().is_ok_and(|content| {
                        matches!(&*content, TabContent::Chart(workspace) if workspace.alert_editor_needs_fx_rates())
                    })
                })
            },
        );
    }

    /// Shows the Alerts window while it is open.
    pub(super) fn sync_alerts_viewport(&self, ctx: &egui::Context) {
        let alerts = &self.alerts;
        if !alerts.window.is_open() {
            return;
        }
        window::sync_viewport(
            ctx,
            AlertsWindowEnv {
                window: alerts.window.clone(),
                alert_host: Arc::clone(&alerts.host),
                registration: alerts.registration.clone(),
                fx: alerts.fx.shared().clone(),
                store: Arc::clone(&self.store),
                quote_book: self.quote_book.clone(),
                quote_coordinator: self.quote_coordinator.clone(),
                settings_host: Arc::clone(&self.settings_host),
                pine_host: Arc::clone(&self.pine_host),
                app_actions: Arc::clone(&self.actions),
            },
        );
    }
}
