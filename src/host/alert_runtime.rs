//! Exclusive ownership and the shared price-alert runtime loop used by both the GUI fallback and
//! the embedded login item. Pine alerts deliberately stay in the GUI.

use crate::host::alert_host::AlertHost;
use crate::model::alert::{
    AlertCurrency, AlertRuntimeCommandPayload, AlertRuntimeHealth, AlertRuntimeOwner,
    AlertServiceState, AlertTriggerEvent,
};
use crate::service::fx_rates::FxRateService;
use crate::service::macos_notifications::{Notifier, default_notifier};
use crate::service::watchlist_quotes::{WatchlistQuoteBook, WatchlistQuoteCoordinator};
use crate::store::{self, SharedStore};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

pub const HEARTBEAT_FRESH_MS: i64 = 5_000;
pub const AGENT_STARTUP_GRACE_MS: i64 = 3_000;
const ALERT_QUOTE_CONSUMER: u64 = u64::MAX - 1;

pub struct AlertRuntimeLease {
    file: File,
}

impl AlertRuntimeLease {
    pub fn try_acquire(directory: &Path) -> io::Result<Option<Self>> {
        std::fs::create_dir_all(directory)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join("alert_runtime.lock"))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { file })),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(error),
        }
    }
}

impl Drop for AlertRuntimeLease {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuiRuntimeDecision {
    Agent,
    StartupGrace,
    Foreground,
}

pub fn agent_heartbeat_is_fresh(health: &AlertRuntimeHealth, now_ms: i64) -> bool {
    health.owner == AlertRuntimeOwner::Agent
        && health
            .heartbeat_ms
            .is_some_and(|heartbeat| now_ms.saturating_sub(heartbeat) <= HEARTBEAT_FRESH_MS)
}

pub fn gui_runtime_decision(
    service_state: AlertServiceState,
    health: &AlertRuntimeHealth,
    now_ms: i64,
    registration_attempted_at_ms: i64,
) -> GuiRuntimeDecision {
    if service_state == AlertServiceState::Enabled && agent_heartbeat_is_fresh(health, now_ms) {
        GuiRuntimeDecision::Agent
    } else if service_state == AlertServiceState::Enabled
        && now_ms.saturating_sub(registration_attempted_at_ms) < AGENT_STARTUP_GRACE_MS
    {
        GuiRuntimeDecision::StartupGrace
    } else {
        GuiRuntimeDecision::Foreground
    }
}

pub struct AlertRuntimeRunner {
    owner: AlertRuntimeOwner,
    last_quote_revision: u64,
    last_tick_ms: i64,
}

impl AlertRuntimeRunner {
    pub fn new(owner: AlertRuntimeOwner) -> Self {
        Self {
            owner,
            last_quote_revision: u64::MAX,
            last_tick_ms: i64::MIN / 2,
        }
    }

    pub fn suspend_quotes(&self, quote_coordinator: &WatchlistQuoteCoordinator) {
        quote_coordinator.set_demand(ALERT_QUOTE_CONSUMER, Vec::new(), false);
    }

    pub fn invalidate_quotes(&mut self) {
        self.last_quote_revision = u64::MAX;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn tick(
        &mut self,
        host: &mut AlertHost,
        store_handle: &SharedStore,
        quote_book: &WatchlistQuoteBook,
        quote_coordinator: &WatchlistQuoteCoordinator,
        notifier: &dyn Notifier,
        rates: &HashMap<AlertCurrency, f64>,
        service_state: AlertServiceState,
        now_ms: i64,
    ) -> Vec<AlertTriggerEvent> {
        if now_ms.saturating_sub(self.last_tick_ms) < 1_000 {
            return Vec::new();
        }
        self.last_tick_ms = now_ms;

        let requests_authorization = {
            let store = store_handle.lock().expect("store lock poisoned");
            let requests_authorization = store.pending_alert_commands().iter().any(|command| {
                matches!(
                    &command.payload,
                    AlertRuntimeCommandPayload::RequestNotificationAuthorization
                )
            });
            host.drain_commands(&store, now_ms);
            requests_authorization
        };
        if requests_authorization {
            let _ = notifier.request_authorization();
        }
        quote_coordinator.set_demand(ALERT_QUOTE_CONSUMER, host.active_instruments(), true);

        let revision = quote_book.revision();
        let events = if revision != self.last_quote_revision {
            self.last_quote_revision = revision;
            host.evaluate_from_book_with_rates(
                quote_book,
                |currency| rates.get(&currency).copied(),
                now_ms,
            )
        } else {
            Vec::new()
        };
        if !events.is_empty() {
            let store = store_handle.lock().expect("store lock poisoned");
            host.dispatch_webhooks(&events, &store, store_handle, now_ms);
            host.receive_events(&events, &store, now_ms);
        }
        host.dispatch_notifications(notifier, now_ms);
        host.update_health(AlertRuntimeHealth {
            owner: self.owner,
            service_state,
            heartbeat_ms: Some(now_ms),
            notification_permission: notifier.permission(),
            providers: quote_coordinator.provider_health(),
            unreconciled_gaps: Vec::new(),
        });
        let store = store_handle.lock().expect("store lock poisoned");
        host.save_if_dirty(&store);
        events
    }
}

pub fn run_alert_agent() -> Result<(), Box<dyn std::error::Error>> {
    let directory = store::data_dir().ok_or("cannot locate the application data directory")?;
    let _lease = loop {
        if let Some(lease) = AlertRuntimeLease::try_acquire(&directory)? {
            break lease;
        }
        std::thread::sleep(Duration::from_millis(250));
    };
    let store = store::open_default()?;
    let quote_book = WatchlistQuoteBook::default();
    let quote_coordinator = WatchlistQuoteCoordinator::new(quote_book.clone(), Arc::new(|| {}));
    let notifier = default_notifier();
    let fx = FxRateService::live(
        crate::service::disk_cache::global().directory(crate::service::disk_cache::Kind::Fx),
    );
    let mut rates = HashMap::new();
    let mut rates_refreshed_at = i64::MIN / 2;
    let mut host = {
        let store = store.lock().expect("store lock poisoned");
        AlertHost::load(&store, now_ms())
    };
    let mut runner = AlertRuntimeRunner::new(AlertRuntimeOwner::Agent);

    loop {
        let now = now_ms();
        if now.saturating_sub(rates_refreshed_at) >= 3_600_000 {
            rates = refresh_rates(&fx);
            rates_refreshed_at = now;
        }
        runner.tick(
            &mut host,
            &store,
            &quote_book,
            &quote_coordinator,
            notifier.as_ref(),
            &rates,
            AlertServiceState::Enabled,
            now,
        );
        wait_for_callbacks();
    }
}

#[cfg(target_os = "macos")]
fn wait_for_callbacks() {
    let limit = objc2_foundation::NSDate::dateWithTimeIntervalSinceNow(0.2);
    objc2_foundation::NSRunLoop::currentRunLoop().runUntilDate(&limit);
}

#[cfg(not(target_os = "macos"))]
fn wait_for_callbacks() {
    std::thread::sleep(Duration::from_millis(200));
}

fn refresh_rates(fx: &FxRateService) -> HashMap<AlertCurrency, f64> {
    [
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
    .collect()
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::alert::{
        AlertAsset, AlertCondition, AlertNotificationPermission, AlertRuntimeCommand,
        AlertRuntimeCommandPayload, AlertRuntimeOwner, PriceAlert,
    };
    use crate::model::watchlist::new_id;
    use crate::model::watchlist_quote::{Freshness, WatchlistQuote};
    use crate::model::{DataSource, InstrumentId};
    use crate::service::macos_notifications::Notifier;
    use crate::service::watchlist_quotes::{Config, QuoteFailure, QuoteProvider, QuoteRequest};
    use crate::store::Store;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    struct NoNetwork;

    impl QuoteProvider for NoNetwork {
        fn quotes(
            &self,
            _: DataSource,
            _: &[QuoteRequest],
        ) -> Result<HashMap<String, crate::model::watchlist_quote::SourceQuote>, QuoteFailure>
        {
            Ok(HashMap::new())
        }

        fn resolve_chain(&self, _: &str) -> Option<String> {
            None
        }
    }

    #[derive(Default)]
    struct RecordingNotifier(AtomicUsize);

    impl Notifier for RecordingNotifier {
        fn deliver(
            &self,
            _: &crate::model::alert_notification::NotificationContent,
        ) -> Result<(), String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn permission(&self) -> AlertNotificationPermission {
            AlertNotificationPermission::Authorized
        }
    }

    #[test]
    fn a_lease_is_exclusive_and_released_on_drop() {
        let directory = std::env::temp_dir().join(format!("degenview-lease-{}", new_id()));
        let first = AlertRuntimeLease::try_acquire(&directory)
            .unwrap()
            .expect("first owner");
        assert!(
            AlertRuntimeLease::try_acquire(&directory)
                .unwrap()
                .is_none()
        );
        drop(first);
        assert!(
            AlertRuntimeLease::try_acquire(&directory)
                .unwrap()
                .is_some()
        );
        let _ = std::fs::remove_file(directory.join("alert_runtime.lock"));
        let _ = std::fs::remove_dir(directory);
    }

    #[test]
    fn a_fresh_agent_wins_then_stale_or_unavailable_falls_back() {
        let now = 20_000;
        let mut health = AlertRuntimeHealth {
            owner: AlertRuntimeOwner::Agent,
            heartbeat_ms: Some(now - 1_000),
            ..Default::default()
        };
        assert_eq!(
            gui_runtime_decision(AlertServiceState::Enabled, &health, now, now - 10_000),
            GuiRuntimeDecision::Agent
        );
        health.heartbeat_ms = Some(now - HEARTBEAT_FRESH_MS - 1);
        assert_eq!(
            gui_runtime_decision(AlertServiceState::Enabled, &health, now, now - 1_000),
            GuiRuntimeDecision::StartupGrace
        );
        assert_eq!(
            gui_runtime_decision(AlertServiceState::Enabled, &health, now, now - 10_000),
            GuiRuntimeDecision::Foreground
        );
        for state in [
            AlertServiceState::RequiresApproval,
            AlertServiceState::NotFound,
            AlertServiceState::Failed,
            AlertServiceState::Disabled,
            AlertServiceState::Unavailable,
        ] {
            assert_eq!(
                gui_runtime_decision(state, &health, now, now),
                GuiRuntimeDecision::Foreground
            );
        }
    }

    #[test]
    fn agent_only_crossing_persists_and_delivers_once() {
        const T0: i64 = 1_700_000_000_000;
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let instrument = InstrumentId::new(DataSource::Binance, "BTCUSDT");
        let alert = PriceAlert::new(
            AlertAsset::new(instrument.clone(), "BTC", "Bitcoin"),
            AlertCondition::CrossesAbove { target: 100.0 },
            T0,
        );
        store
            .lock()
            .unwrap()
            .enqueue_alert_command(&AlertRuntimeCommand::new(
                AlertRuntimeCommandPayload::Save {
                    alert: Box::new(alert),
                    baseline: Some(90.0),
                },
                T0,
            ))
            .unwrap();
        let book = WatchlistQuoteBook::default();
        book.apply(HashMap::from([(
            instrument.key(),
            WatchlistQuote {
                last: Some(110.0),
                timestamp_ms: Some(T0),
                freshness: Freshness::Live,
                ..Default::default()
            },
        )]));
        let coordinator = WatchlistQuoteCoordinator::with_provider(
            book.clone(),
            Arc::new(|| {}),
            Arc::new(NoNetwork),
            Config::default(),
        );
        let mut host = AlertHost::load(&store.lock().unwrap(), T0);
        let notifier = RecordingNotifier::default();
        let mut runner = AlertRuntimeRunner::new(AlertRuntimeOwner::Agent);
        let first = runner.tick(
            &mut host,
            &store,
            &book,
            &coordinator,
            &notifier,
            &HashMap::new(),
            AlertServiceState::Enabled,
            T0,
        );
        assert_eq!(first.len(), 1);
        runner.tick(
            &mut host,
            &store,
            &book,
            &coordinator,
            &notifier,
            &HashMap::new(),
            AlertServiceState::Enabled,
            T0 + 1_000,
        );
        assert_eq!(notifier.0.load(Ordering::SeqCst), 1);
        let stored = store
            .lock()
            .unwrap()
            .load_alert_snapshot()
            .unwrap()
            .unwrap();
        assert_eq!(stored.history.len(), 1);
        assert_eq!(stored.health.owner, AlertRuntimeOwner::Agent);
        assert!(store.lock().unwrap().pending_alert_commands().is_empty());
    }
}
