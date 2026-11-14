//! The alert runtime's standing with the system: who runs it (this window or the login-item
//! agent), the background service that runs it while the app is closed, and committing a saved
//! price alert to its queue.

use crate::host::alert::runtime::{AlertRuntimeLease, GuiRuntimeDecision, gui_runtime_decision};
use crate::lock::lock;
use crate::model::alert::{
    AlertPrecondition, AlertRuntimeCommand, AlertRuntimeCommandPayload, AlertRuntimeHealth,
    AlertServiceState, PriceAlert,
};
use crate::service::alert::background_service::BackgroundAlertService;
use crate::service::alert::macos_notifications::Notifier;
use crate::store::Store;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

pub(super) struct AlertRuntimeControl {
    lease: Option<AlertRuntimeLease>,
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

/// Who runs the alert runtime this frame, as [`AlertRegistration::decide`] found it.
pub(super) struct RuntimeTurn {
    pub(super) decision: GuiRuntimeDecision,
    /// This app holds the runtime lease.
    pub(super) owns_lease: bool,
    pub(super) service_state: AlertServiceState,
}

/// The notifier, the background alert service and the runtime's control state, which a saved
/// price alert and Settings ▸ Alerts' Retry both reach. Cheap to clone; the Settings and Alerts
/// windows each hold one.
///
/// Lock order: the runtime control before the background service when both are held (`decide`,
/// `retry`); `commit` takes them one after the other, never together.
#[derive(Clone)]
pub(in crate::app) struct AlertRegistration {
    notifier: Arc<dyn Notifier>,
    background_service: Arc<Mutex<BackgroundAlertService>>,
    runtime_control: Arc<Mutex<AlertRuntimeControl>>,
}

impl AlertRegistration {
    pub(super) fn new(
        notifier: Arc<dyn Notifier>,
        background_service: BackgroundAlertService,
        runtime_control: AlertRuntimeControl,
    ) -> Self {
        Self {
            notifier,
            background_service: Arc::new(Mutex::new(background_service)),
            runtime_control: Arc::new(Mutex::new(runtime_control)),
        }
    }

    pub(super) fn notifier(&self) -> &Arc<dyn Notifier> {
        &self.notifier
    }

    /// Queues a saved price alert for the runtime and makes sure notifications and the background
    /// service are ready for it. Shared by the chart windows' sheet and the Alerts window's Edit.
    pub(in crate::app) fn commit(
        &self,
        store: &Store,
        alert: Box<PriceAlert>,
        baseline: Option<f64>,
        now: i64,
    ) {
        // The sheets have always ignored a failed save; the side effects below are skipped though.
        let _ = self.commit_checked(store, alert, baseline, None, now);
    }

    /// [`Self::commit`] for a caller that must know whether the alert reached the queue: the MCP
    /// server. Returns the id of the queued save command. A save that could not be queued is an
    /// error and has no side effects: no notification authorization is requested and no service
    /// is registered for an alert that does not exist. `precondition` is checked by the runtime.
    pub(in crate::app) fn commit_checked(
        &self,
        store: &Store,
        alert: Box<PriceAlert>,
        baseline: Option<f64>,
        precondition: Option<AlertPrecondition>,
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
        let notifier = Arc::clone(&self.notifier);
        std::thread::spawn(move || {
            let _ = notifier.request_authorization();
        });
        let service_state = self
            .background_service
            .lock()
            .map(|mut service| service.ensure_registered())
            .unwrap_or(AlertServiceState::Failed);
        if let Ok(mut control) = self.runtime_control.lock() {
            control.retrying(service_state, now);
        }
        Ok(save.id)
    }

    /// Settings ▸ Alerts' Retry: registers the background service again.
    pub(in crate::app) fn retry(&self, now: i64) {
        if let Ok(mut control) = self.runtime_control.lock() {
            control.lease = None;
            if let Ok(mut service) = self.background_service.lock() {
                let state = service.retry_registration();
                control.retrying(state, now);
            }
        }
    }

    /// Who runs the alert runtime this frame: takes a finished service status query's answer,
    /// starts the next query once a second, and takes or gives up the runtime lease.
    pub(super) fn decide(&self, health: &AlertRuntimeHealth, now: i64) -> RuntimeTurn {
        let mut control = lock(&self.runtime_control);
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
            health,
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
        RuntimeTurn {
            decision,
            owns_lease: control.lease.is_some(),
            service_state: control.service_state,
        }
    }

    #[cfg(test)]
    pub(super) fn service_state(&self) -> AlertServiceState {
        lock(&self.runtime_control).service_state
    }
}
