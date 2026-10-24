//! Shared alert runtime host: loads the persisted snapshot, drains the GUI -> runtime command
//! queue, and exposes the evaluated snapshot. The exclusive runtime owner may be either the GUI
//! fallback or the embedded login-item agent.

use crate::model::alert::{
    AlertCurrency, AlertNotificationSettings, AlertPersistenceSnapshot, AlertRuntimeHealth,
    AlertTriggerEvent, MarketQuote,
};
use crate::model::alert_engine::AlertEngine;
use crate::model::alert_feed::UnseenAlerts;
use crate::model::alert_message::render_for_event;
use crate::model::alert_notification::{
    begin_attempt, delivered, failed, notifications_enabled, pending_deliveries,
    price_alert_content, script_alert_content, suppressed,
};
use crate::model::pine_alert::PineAlertSubscription;
use crate::model::watchlist_quote::maximum_age_secs;
use crate::model::webhook::WebhookDeliverySource;
use crate::model::webhook_policy::price_event_is_sendable;
use crate::pine::state::AlertEvent;
use crate::service::keychain_webhook_secrets::KeychainWebhookSecretStore;
use crate::service::macos_notifications::Notifier;
use crate::service::watchlist_quotes::WatchlistQuoteBook;
use crate::service::webhook_delivery::{self, Mode, UreqTransport};
use crate::store::{SharedStore, Store};
use std::collections::HashSet;
use std::sync::Arc;

pub struct AlertHost {
    engine: AlertEngine,
    unseen: UnseenAlerts,
    dirty: bool,
    pending_ack_ids: Vec<u64>,
    load_error: Option<String>,
    transport: Arc<UreqTransport>,
    secrets: Arc<KeychainWebhookSecretStore>,
}

impl AlertHost {
    /// Loads the snapshot (or starts from defaults when nothing was ever saved) and seeds the
    /// unseen-count from the stored last-seen timestamp.
    pub fn load(store: &Store, now_ms: i64) -> Self {
        let loaded = store.load_alert_snapshot();
        let load_error = loaded.as_ref().err().map(ToString::to_string);
        let engine = AlertEngine::new(loaded.ok().flatten());
        let (unseen, to_save) = UnseenAlerts::new(store.last_seen_at(), now_ms);
        if let Some(ms) = to_save {
            let _ = store.set_last_seen_at(ms);
        }
        Self {
            engine,
            unseen,
            dirty: false,
            pending_ack_ids: Vec::new(),
            load_error,
            transport: Arc::new(UreqTransport::default()),
            secrets: Arc::new(KeychainWebhookSecretStore::default()),
        }
    }

    pub fn snapshot(&self) -> &AlertPersistenceSnapshot {
        self.engine.snapshot()
    }

    /// An alert of the same market, condition, currency and frequency, if one exists.
    pub fn find_identical(
        &self,
        candidate: &crate::model::alert::PriceAlert,
    ) -> Option<&crate::model::alert::PriceAlert> {
        self.engine.find_identical(candidate)
    }

    pub fn settings(&self) -> &AlertNotificationSettings {
        &self.engine.snapshot().settings
    }

    pub fn health(&self) -> &AlertRuntimeHealth {
        &self.engine.snapshot().health
    }

    pub fn unseen(&self) -> &UnseenAlerts {
        &self.unseen
    }

    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    pub fn active_instruments(&self) -> Vec<crate::model::InstrumentId> {
        self.engine
            .active_assets()
            .into_iter()
            .map(|asset| asset.instrument)
            .collect()
    }

    pub fn update_health(&mut self, health: AlertRuntimeHealth) {
        if self.engine.snapshot().health != health {
            self.engine.replace_health(health);
            self.dirty = true;
        }
    }

    /// Refreshes the UI's read model after the other process writes a newer snapshot. New trigger
    /// events are returned for banners and fed into the unseen counter exactly once.
    pub fn reload_from_store(&mut self, store: &Store, now_ms: i64) -> Vec<AlertTriggerEvent> {
        let loaded = match store.load_alert_snapshot() {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => return Vec::new(),
            Err(error) => {
                self.load_error = Some(error.to_string());
                return Vec::new();
            }
        };
        if loaded.revision == self.engine.snapshot().revision {
            return Vec::new();
        }
        let known: HashSet<u64> = self
            .engine
            .snapshot()
            .history
            .iter()
            .map(|event| event.id)
            .collect();
        let events: Vec<_> = loaded
            .history
            .iter()
            .filter(|event| !known.contains(&event.id))
            .cloned()
            .collect();
        self.engine = AlertEngine::new(Some(loaded));
        self.engine.set_muted(store.muted_alerts());
        self.load_error = None;
        self.receive_events(&events, store, now_ms);
        events
    }

    pub fn receive_events(&mut self, events: &[AlertTriggerEvent], store: &Store, _now_ms: i64) {
        for event in events {
            if let Some(last_seen) =
                self.unseen
                    .receive(crate::model::alert::AlertDomainEvent::PriceAlertTriggered {
                        event_id: event.id,
                        at_ms: event.timestamp_ms,
                    })
            {
                let _ = store.set_last_seen_at(last_seen);
            }
        }
    }

    pub fn set_alerts_window_active(&mut self, active: bool, store: &Store, now_ms: i64) {
        if active == self.unseen.window_is_active() {
            return;
        }
        if active {
            let last_seen = self.unseen.window_did_become_active(now_ms);
            let _ = store.set_last_seen_at(last_seen);
        } else {
            self.unseen.window_did_resign();
        }
    }

    /// Applies every queued command exactly once (idempotent by command id) and marks the
    /// snapshot dirty.
    pub fn drain_commands(&mut self, store: &Store, now_ms: i64) {
        if self.load_error.is_some() {
            return;
        }
        // Synced alerts that this Mac has not switched on are defined here but never evaluated.
        if self.engine.set_muted(store.muted_alerts()) {
            self.dirty = true;
        }
        for command in store.pending_alert_commands() {
            if self.pending_ack_ids.contains(&command.id) {
                continue;
            }
            self.engine.apply(&command, now_ms);
            self.pending_ack_ids.push(command.id);
            self.dirty = true;
        }
    }

    /// Delivers each trigger to the alert's webhook endpoints: claim → deliver → finish. One
    /// trigger reaches one endpoint exactly once (the claim dedupes). The sendable gate and the
    /// disabled-endpoint skip mirror `PriceAlertWebhookDispatcher.swift`. Sends run on workers.
    pub fn dispatch_webhooks(
        &self,
        events: &[AlertTriggerEvent],
        store: &Store,
        handle: &SharedStore,
        now_ms: i64,
    ) {
        if events.is_empty() {
            return;
        }
        let endpoints = store.webhook_endpoints();
        if endpoints.is_empty() {
            return;
        }
        let delivery_enabled = self.engine.snapshot().settings.delivery_enabled;
        for event in events {
            if !price_event_is_sendable(event, delivery_enabled, now_ms) {
                continue;
            }
            let Some(alert) = self
                .engine
                .snapshot()
                .alerts
                .iter()
                .find(|alert| alert.id == event.alert_id)
            else {
                continue;
            };
            for endpoint_id in &alert.webhook_endpoint_ids {
                let Some(endpoint) = endpoints
                    .iter()
                    .find(|endpoint| endpoint.id == *endpoint_id)
                else {
                    continue;
                };
                if !endpoint.is_enabled {
                    continue;
                }
                let Some(delivery_id) = store.claim_webhook_delivery(
                    endpoint.id,
                    WebhookDeliverySource::Price,
                    event.id,
                    now_ms,
                ) else {
                    continue;
                };
                let message = render_for_event(alert.webhook_message.as_deref(), event, now_ms);
                // The claim is durable; the network attempt runs off the UI thread so a slow
                // endpoint never freezes the app.
                let transport = Arc::clone(&self.transport);
                let secrets = Arc::clone(&self.secrets);
                let handle = Arc::clone(handle);
                let endpoint = endpoint.clone();
                std::thread::spawn(move || {
                    let result = webhook_delivery::deliver(
                        transport.as_ref(),
                        secrets.as_ref(),
                        &message,
                        &endpoint,
                        Mode::Production,
                    );
                    if let Ok(store) = handle.lock() {
                        store.finish_webhook_delivery(delivery_id, &result);
                    }
                });
            }
        }
    }

    /// Builds a `MarketQuote` per active alert asset (and per alert currency) from the shared quote
    /// book and runs them through the engine. Source quotes are USD; other currencies are converted
    /// with the FX service. A currency that cannot be converted is skipped, never zero-filled.
    /// Uses an already fetched FX snapshot so the UI frame never waits on a rate request.
    pub fn evaluate_from_book_with_rates(
        &mut self,
        book: &WatchlistQuoteBook,
        mut rate: impl FnMut(AlertCurrency) -> Option<f64>,
        now_ms: i64,
    ) -> Vec<AlertTriggerEvent> {
        let mut events = Vec::new();
        for asset in self.engine.active_assets() {
            let source = asset.source();
            let key = asset.key();
            let Some(quote) = book.get_by_key(&key) else {
                continue;
            };
            let (Some(usd_price), Some(source_timestamp_ms)) = (quote.last, quote.timestamp_ms)
            else {
                continue;
            };
            let currencies: Vec<AlertCurrency> = self
                .engine
                .snapshot()
                .alerts
                .iter()
                .filter(|alert| alert.asset.key() == key && self.engine.evaluates(alert))
                .map(|alert| alert.currency)
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            for currency in currencies {
                let price = if currency == AlertCurrency::Usd {
                    usd_price
                } else {
                    match rate(currency) {
                        Some(rate) => usd_price * rate,
                        None => continue,
                    }
                };
                let market = MarketQuote {
                    asset: asset.clone(),
                    price,
                    currency,
                    source_timestamp_ms,
                    received_at_ms: now_ms,
                    maximum_age_secs: maximum_age_secs(source) as f64,
                    fingerprint: source_timestamp_ms.to_string(),
                    candle: None,
                };
                events.extend(self.engine.process(&market, None, Some(currency)));
            }
        }
        if !events.is_empty() {
            self.dirty = true;
        }
        events
    }

    /// Delivers the in-app/macOS notification for every trigger that is owed one, then records the
    /// attempt (delivered / failed / retry) on the event. Synchronous so the delivery result is
    /// durably recorded before this runtime tick commits its snapshot.
    pub fn dispatch_notifications(&mut self, notifier: &dyn Notifier, now_ms: i64) {
        if !notifications_enabled(self.settings()) {
            let updates: Vec<_> = pending_deliveries(&self.engine.snapshot().history, now_ms)
                .into_iter()
                .map(|event| (event.id, suppressed(&event.delivery)))
                .collect();
            for (event_id, record) in updates {
                self.engine.mark_delivery(event_id, record);
                self.dirty = true;
            }
            return;
        }
        let updates = {
            let pending = pending_deliveries(&self.engine.snapshot().history, now_ms);
            let mut updates = Vec::new();
            for event in pending {
                let content = price_alert_content(event, self.settings());
                let attempted = begin_attempt(&event.delivery, now_ms);
                let record = match notifier.deliver(&content) {
                    Ok(()) => delivered(&attempted),
                    Err(error) => failed(&attempted, &error, now_ms),
                };
                updates.push((event.id, record));
            }
            updates
        };
        for (event_id, record) in updates {
            self.engine.mark_delivery(event_id, record);
            self.dirty = true;
        }
    }

    /// Delivers fired Pine alerts: webhooks (source = Pine) to each subscription's endpoints, then
    /// the macOS notification. Webhook delivery runs on a worker thread, like the price path.
    pub fn dispatch_pine(
        &self,
        fired: &[(PineAlertSubscription, AlertEvent)],
        store: &Store,
        handle: &SharedStore,
        notifier: &dyn Notifier,
        now_ms: i64,
    ) {
        if fired.is_empty() {
            return;
        }
        let endpoints = store.webhook_endpoints();
        let settings = self.engine.snapshot().settings.clone();
        for (subscription, alert) in fired {
            if !alert.is_realtime {
                continue;
            }
            let dedupe_key = pine_alert_dedupe_key(subscription, alert);
            let Some(event_id) =
                store.claim_pine_alert_event(subscription.id, dedupe_key.as_deref(), now_ms)
            else {
                continue;
            };
            for endpoint_id in &subscription.webhook_endpoint_ids {
                let Some(endpoint) = endpoints
                    .iter()
                    .find(|endpoint| endpoint.id == *endpoint_id)
                else {
                    continue;
                };
                if !endpoint.is_enabled {
                    continue;
                }
                let Some(delivery_id) = store.claim_webhook_delivery(
                    endpoint.id,
                    WebhookDeliverySource::Pine,
                    event_id,
                    now_ms,
                ) else {
                    continue;
                };
                let message = alert.message.clone();
                let transport = Arc::clone(&self.transport);
                let secrets = Arc::clone(&self.secrets);
                let handle = Arc::clone(handle);
                let endpoint = endpoint.clone();
                std::thread::spawn(move || {
                    let result = webhook_delivery::deliver(
                        transport.as_ref(),
                        secrets.as_ref(),
                        &message,
                        &endpoint,
                        Mode::Production,
                    );
                    if let Ok(store) = handle.lock() {
                        store.finish_webhook_delivery(delivery_id, &result);
                    }
                });
            }
            if notifications_enabled(&settings) {
                let content = script_alert_content(
                    event_id,
                    subscription.id,
                    &subscription.script_name,
                    &alert.message,
                    &settings,
                );
                let _ = notifier.deliver(&content);
            }
        }
    }

    pub fn save_if_dirty(&mut self, store: &Store) {
        if self.load_error.is_none()
            && self.dirty
            && store
                .save_alert_snapshot_and_acknowledge(self.engine.snapshot(), &self.pending_ack_ids)
                .is_ok()
        {
            self.pending_ack_ids.clear();
            self.dirty = false;
        }
    }
}

fn pine_alert_dedupe_key(
    subscription: &PineAlertSubscription,
    alert: &AlertEvent,
) -> Option<String> {
    (alert.frequency != crate::pine::alert_frequency::AlertFrequency::All).then(|| {
        format!(
            "{}|{}|{}|{}",
            subscription.id,
            alert.site,
            alert.time,
            alert.frequency.pine_name()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::alert::{AlertRuntimeCommand, AlertRuntimeCommandPayload};
    use crate::model::watchlist::new_id;

    #[test]
    fn a_fresh_host_starts_empty_and_seeds_last_seen() {
        let store = Store::open_in_memory().expect("open");
        let now_ms = 1_000;
        let mut host = AlertHost::load(&store, now_ms);
        assert!(host.snapshot().alerts.is_empty());
        assert_eq!(store.last_seen_at(), Some(now_ms));
        host.save_if_dirty(&store);
        assert!(store.load_alert_snapshot().expect("load").is_none());
    }

    #[test]
    fn commands_drain_once_and_persist() {
        let store = Store::open_in_memory().expect("open");
        let command = AlertRuntimeCommand {
            id: new_id(),
            created_at_ms: 1_000,
            expected_revision: None,
            payload: AlertRuntimeCommandPayload::ClearHistory,
        };
        store.enqueue_alert_command(&command).expect("enqueue");

        let mut host = AlertHost::load(&store, 1_000);
        host.drain_commands(&store, 1_000);
        assert_eq!(store.pending_alert_commands().len(), 1);
        host.save_if_dirty(&store);
        assert!(store.pending_alert_commands().is_empty());
        // A revision is now written, so a reload finds a snapshot.
        assert!(store.load_alert_snapshot().expect("load").is_some());
    }

    #[test]
    fn a_failed_snapshot_save_retries_without_losing_the_command() {
        let path = std::env::temp_dir().join(format!("degenview-alert-save-{}.sqlite", new_id()));
        let store = Store::open(&path).expect("open");
        let command = AlertRuntimeCommand::new(AlertRuntimeCommandPayload::ClearHistory, 1_000);
        store.enqueue_alert_command(&command).expect("enqueue");
        let connection = rusqlite::Connection::open(&path).expect("second connection");
        connection
            .execute_batch(
                "CREATE TRIGGER reject_alert_revision BEFORE INSERT ON setting
             WHEN NEW.key = 'alert.revision' BEGIN SELECT RAISE(ABORT, 'save failed'); END;",
            )
            .expect("trigger");
        let mut host = AlertHost::load(&store, 1_000);
        host.drain_commands(&store, 1_000);
        host.save_if_dirty(&store);
        assert_eq!(store.pending_alert_commands().len(), 1);
        assert!(store.load_alert_snapshot().expect("load").is_none());
        connection
            .execute_batch("DROP TRIGGER reject_alert_revision")
            .expect("drop trigger");
        host.drain_commands(&store, 2_000);
        host.save_if_dirty(&store);
        assert!(store.pending_alert_commands().is_empty());
        assert!(store.load_alert_snapshot().expect("load").is_some());
        drop(store);
        drop(connection);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn unreadable_history_blocks_commands_and_saves() {
        let path = std::env::temp_dir().join(format!("degenview-alert-load-{}.sqlite", new_id()));
        let store = Store::open(&path).expect("open");
        store
            .save_alert_snapshot_and_acknowledge(&AlertPersistenceSnapshot::default(), &[])
            .expect("seed snapshot");
        let connection = rusqlite::Connection::open(&path).expect("second connection");
        connection
            .execute(
                "INSERT INTO alert_event (alert_id, timestamp, payload) VALUES ('7', 1, 'bad')",
                [],
            )
            .expect("seed event");
        let command = AlertRuntimeCommand::new(AlertRuntimeCommandPayload::ClearHistory, 1_000);
        store.enqueue_alert_command(&command).expect("enqueue");
        let mut host = AlertHost::load(&store, 1_000);
        assert!(host.load_error().is_some());
        host.drain_commands(&store, 1_000);
        host.save_if_dirty(&store);
        assert_eq!(store.pending_alert_commands().len(), 1);
        assert!(store.load_alert_snapshot().is_err());
        drop(store);
        drop(connection);
        let _ = std::fs::remove_file(path);
    }
}
