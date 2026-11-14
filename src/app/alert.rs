use super::DegenViewApp;
use super::now_ms;
use super::tabs::TabContent;
use crate::host::alert::runtime::{AlertRuntimeLease, GuiRuntimeDecision, gui_runtime_decision};
use crate::lock::lock;
use crate::model::alert::{
    AlertCurrency, AlertRuntimeCommand, AlertRuntimeCommandPayload, AlertServiceState, AlertState,
};
use crate::service::alert::background_service::BackgroundAlertService;
use crate::service::alert::macos_notifications::Notifier;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub(super) struct AlertRuntimeControl {
    pub(super) lease: Option<AlertRuntimeLease>,
    registration_attempted_at_ms: i64,
    pub(super) service_state: AlertServiceState,
    last_status_check_ms: i64,
    /// A status query running off the UI thread; its answer replaces `service_state`.
    pub(super) status_probe: Option<mpsc::Receiver<AlertServiceState>>,
}

impl AlertRuntimeControl {
    pub(super) fn new(service_state: AlertServiceState, now_ms: i64) -> Self {
        Self {
            lease: None,
            registration_attempted_at_ms: now_ms,
            service_state,
            last_status_check_ms: i64::MIN / 2,
            status_probe: None,
        }
    }

    pub(super) fn retrying(&mut self, service_state: AlertServiceState, now_ms: i64) {
        self.lease = None;
        self.registration_attempted_at_ms = now_ms;
        self.service_state = service_state;
        self.last_status_check_ms = now_ms;
        // A query started before this registration would report the state from before it.
        self.status_probe = None;
    }

    /// Takes the answer of a finished status query, if there is one.
    pub(super) fn poll_service_status(&mut self) {
        let Some(receiver) = &self.status_probe else {
            return;
        };
        match receiver.try_recv() {
            Ok(state) => {
                self.service_state = state;
                self.status_probe = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => self.status_probe = None,
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }
}

/// Queues a saved price alert for the runtime and makes sure notifications and the background
/// service are ready for it. Shared by the chart windows' sheet and the Alerts window's Edit.
pub(super) fn commit_price_alert(
    store: &crate::store::Store,
    notifier: &Arc<dyn Notifier>,
    background_service: &Arc<Mutex<BackgroundAlertService>>,
    runtime_control: &Arc<Mutex<AlertRuntimeControl>>,
    alert: Box<crate::model::alert::PriceAlert>,
    baseline: Option<f64>,
    now: i64,
) {
    // The sheets have always ignored a failed save; the side effects below are skipped though.
    let _ = commit_price_alert_checked(
        store,
        notifier,
        background_service,
        runtime_control,
        alert,
        baseline,
        None,
        now,
    );
}

/// [`commit_price_alert`] for a caller that must know whether the alert reached the queue: the
/// MCP server. Returns the id of the queued save command. A save that could not be queued is an
/// error and has no side effects: no notification authorization is requested and no service is
/// registered for an alert that does not exist. `precondition` is checked by the runtime.
#[allow(clippy::too_many_arguments)]
pub(super) fn commit_price_alert_checked(
    store: &crate::store::Store,
    notifier: &Arc<dyn Notifier>,
    background_service: &Arc<Mutex<BackgroundAlertService>>,
    runtime_control: &Arc<Mutex<AlertRuntimeControl>>,
    alert: Box<crate::model::alert::PriceAlert>,
    baseline: Option<f64>,
    precondition: Option<crate::model::alert::AlertPrecondition>,
    now: i64,
) -> rusqlite::Result<u64> {
    let mut save =
        AlertRuntimeCommand::new(AlertRuntimeCommandPayload::Save { alert, baseline }, now);
    save.precondition = precondition;
    store.enqueue_alert_command(&save)?;
    // Best effort from here: the alert itself is safely queued.
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
    Ok(save.id)
}

impl DegenViewApp {
    pub(super) fn tick_alerts(&mut self) {
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
            .is_ok_and(|slot| {
                slot.as_ref()
                    .is_some_and(|editor| editor.draft.needs_fx_rates())
            });
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
            let mut control = lock(&self.alert_runtime_control);
            control.poll_service_status();
            if control.status_probe.is_none()
                && now.saturating_sub(control.last_status_check_ms) >= 1_000
            {
                match self.background_service.lock() {
                    Ok(service) => {
                        let probe = service.status_probe();
                        let (sender, receiver) = mpsc::channel();
                        // Not the shared work queue: a slow system service must not hold up
                        // provider requests, nor they it.
                        std::thread::spawn(move || {
                            let _ = sender.send(probe());
                        });
                        control.status_probe = Some(receiver);
                    }
                    Err(_) => control.service_state = AlertServiceState::Failed,
                }
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
            let mut host = lock(&self.alert_host);
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
            let store = lock(&self.store);
            lock(&self.alert_host).reload_from_store(&store, now)
        };
        if !events.is_empty()
            && let Ok(host) = self.alert_host.lock()
        {
            for event in &events {
                self.banner.show_price(event, host.settings(), now);
            }
        }
    }
}
