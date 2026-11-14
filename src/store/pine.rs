//! Pine alert-subscription persistence. Each row is one applied `PineAlertSubscription` document,
//! keyed by its id, stored under `pine_alert_subscription` like the other ordered document tables.

use crate::model::alert::pine::PineAlertSubscription;
use crate::model::watchlist::new_id;
use crate::store::{Document, Store};
use rusqlite::params;

const PINE_ALERT_EVENT_HISTORY_LIMIT: i64 = 5_000;

impl Document for PineAlertSubscription {
    fn document_id(&self) -> String {
        self.id.to_string()
    }
}

impl Store {
    pub fn pine_subscriptions(&self) -> Vec<PineAlertSubscription> {
        self.documents("pine_alert_subscription")
    }

    pub fn replace_pine_subscriptions(
        &self,
        subscriptions: &[PineAlertSubscription],
    ) -> rusqlite::Result<()> {
        self.replace_documents("pine_alert_subscription", subscriptions)
    }

    /// Claims one Pine notification before any external delivery. A stable `dedupe_key` admits
    /// only the first process/frame that sees a once-per-bar event; `None` deliberately admits
    /// every `alert.freq_all` execution. The returned id is globally unique and is also the
    /// webhook event id, so subscriptions can never collide in `webhook_delivery`.
    pub fn claim_pine_alert_event(
        &self,
        subscription_id: u64,
        dedupe_key: Option<&str>,
        timestamp_ms: i64,
    ) -> Option<u64> {
        let id = new_id();
        let inserted = self.connection.execute(
            "INSERT OR IGNORE INTO pine_alert_event (id, subscription_id, dedupe_key, timestamp)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                id.to_string(),
                subscription_id.to_string(),
                dedupe_key,
                timestamp_ms
            ],
        );
        match inserted {
            Ok(changes) if changes > 0 => {
                let _ = self.connection.execute(
                    "DELETE FROM pine_alert_event WHERE id NOT IN (
                        SELECT id FROM pine_alert_event
                        ORDER BY timestamp DESC, rowid DESC LIMIT ?1)",
                    params![PINE_ALERT_EVENT_HISTORY_LIMIT],
                );
                Some(id)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::model::alert::pine::PineAlertState;

    pub(in crate::store) fn subscription(id: u64) -> PineAlertSubscription {
        PineAlertSubscription {
            id,
            chart_id: 1,
            instance_id: None,
            script_id: None,
            script_name: "My Script".into(),
            source: String::new(),
            inputs: Default::default(),
            symbol_key: "binance:BTCUSDT".into(),
            timeframe: "1h".into(),
            source_hash: "abc".into(),
            note: String::new(),
            state: PineAlertState::Active,
            created_at_ms: 1_000,
            webhook_endpoint_ids: Vec::new(),
        }
    }

    #[test]
    fn subscriptions_round_trip_in_order() {
        let store = Store::open_in_memory().expect("open");
        store
            .replace_pine_subscriptions(&[subscription(7), subscription(9)])
            .expect("write");
        let loaded = store.pine_subscriptions();
        assert_eq!(loaded.len(), 2);
        assert_eq!(
            (loaded[0].id, loaded[0].script_name.as_str()),
            (7, "My Script")
        );
        assert_eq!(loaded[1].id, 9);
    }

    #[test]
    fn pine_event_claims_dedupe_stable_keys_but_not_frequency_all() {
        let store = Store::open_in_memory().expect("open");
        let first = store
            .claim_pine_alert_event(7, Some("7|3|1000|once"), 2_000)
            .expect("first claim");
        assert!(
            store
                .claim_pine_alert_event(7, Some("7|3|1000|once"), 2_001)
                .is_none()
        );
        let all_a = store
            .claim_pine_alert_event(7, None, 2_002)
            .expect("first freq_all event");
        let all_b = store
            .claim_pine_alert_event(7, None, 2_003)
            .expect("second freq_all event");
        assert_ne!(first, all_a);
        assert_ne!(all_a, all_b);
    }
}
