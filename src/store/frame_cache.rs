//! Reads the UI makes every frame, kept until the database changes.
//!
//! A frame must not parse JSON or run more than a cheap change check against SQLite. A read the
//! UI repeats every frame is built once and kept with the database state it saw (a
//! [`ChangeStamp`]); the next frame compares the stamp, two integer reads, and reuses the value
//! while nothing was written. Any write counts, through this connection (the app, sync applying
//! remote records, a restore) or another (the background alert service, a second process): no
//! writer can be forgotten, at the cost of one rebuild per write.

use super::Store;
use crate::model::alert::pine::PineAlertSubscription;
use crate::model::webhook::WebhookEndpoint;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// The database state a cached read was built from: `(changes made through this connection,
/// PRAGMA data_version)`. The second moves when another connection commits.
pub(crate) type ChangeStamp = (u64, i64);

/// One value derived from the database, rebuilt only when the stamp moved.
pub(crate) struct ChangeCached<T> {
    slot: RefCell<Option<(ChangeStamp, Arc<T>)>>,
}

impl<T> Default for ChangeCached<T> {
    fn default() -> Self {
        Self {
            slot: RefCell::new(None),
        }
    }
}

impl<T> ChangeCached<T> {
    /// The value for `stamp`, building it when the database changed since the last build. With no
    /// stamp (an open transaction, which may yet roll back) the value is built and not kept.
    pub(crate) fn get(&self, stamp: Option<ChangeStamp>, build: impl FnOnce() -> T) -> Arc<T> {
        let Some(stamp) = stamp else {
            return Arc::new(build());
        };
        if let Some((seen, value)) = self.slot.borrow().as_ref()
            && *seen == stamp
        {
            return Arc::clone(value);
        }
        // Built outside the borrow, so a build that reads another cached value cannot panic.
        let value = Arc::new(build());
        *self.slot.borrow_mut() = Some((stamp, Arc::clone(&value)));
        value
    }
}

/// Per-key answers derived from the database, all dropped when the stamp moves.
pub(crate) struct ChangeCachedMap<K, V> {
    slot: RefCell<(Option<ChangeStamp>, HashMap<K, V>)>,
}

impl<K, V> Default for ChangeCachedMap<K, V> {
    fn default() -> Self {
        Self {
            slot: RefCell::new((None, HashMap::new())),
        }
    }
}

impl<K: std::hash::Hash + Eq, V: Clone> ChangeCachedMap<K, V> {
    pub(crate) fn get(&self, stamp: Option<ChangeStamp>, key: K, build: impl FnOnce(&K) -> V) -> V {
        let Some(stamp) = stamp else {
            return build(&key);
        };
        {
            let slot = self.slot.borrow();
            if slot.0 == Some(stamp)
                && let Some(value) = slot.1.get(&key)
            {
                return value.clone();
            }
        }
        let value = build(&key);
        let mut slot = self.slot.borrow_mut();
        if slot.0 != Some(stamp) {
            slot.0 = Some(stamp);
            slot.1.clear();
        }
        slot.1.insert(key, value.clone());
        value
    }
}

/// Every cached frame read the store keeps.
#[derive(Default)]
pub(crate) struct FrameCaches {
    /// The saved-layout index the workspace toolbar shows (`Store::layout_index`).
    pub(super) layout_index: ChangeCached<super::chart::saved_view::LayoutIndex>,
    /// The Settings and Alerts windows list the script alerts every frame.
    pine_subscriptions: ChangeCached<Vec<PineAlertSubscription>>,
    /// The Settings window's Webhooks page and a script alert's Webhooks sheet.
    webhook_endpoints: ChangeCached<Vec<WebhookEndpoint>>,
    /// The Alerts window's "not evaluated here" marks.
    muted_alerts: ChangeCached<HashSet<u64>>,
    /// `(delivered, attempted)` per alert event, for the Alerts window's history chips.
    webhook_summaries: ChangeCachedMap<u64, Option<(usize, usize)>>,
}

impl Store {
    /// The database state now, for a cached read to compare against; `None` inside an open
    /// transaction, whose writes may still be rolled back without moving either counter.
    pub(crate) fn change_stamp(&self) -> Option<ChangeStamp> {
        if !self.connection.is_autocommit() {
            return None;
        }
        let version = self
            .connection
            .query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0))
            .ok()?;
        Some((self.connection.total_changes(), version))
    }

    /// `pine_subscriptions`, read again only after a write. For drawing; an edit that is written
    /// back reads `pine_subscriptions` itself.
    pub fn pine_subscriptions_cached(&self) -> Arc<Vec<PineAlertSubscription>> {
        self.frame_caches
            .pine_subscriptions
            .get(self.change_stamp(), || self.pine_subscriptions())
    }

    /// `webhook_endpoints`, read again only after a write.
    pub fn webhook_endpoints_cached(&self) -> Arc<Vec<WebhookEndpoint>> {
        self.frame_caches
            .webhook_endpoints
            .get(self.change_stamp(), || self.webhook_endpoints())
    }

    /// `muted_alerts`, read again only after a write.
    pub fn muted_alerts_cached(&self) -> Arc<HashSet<u64>> {
        self.frame_caches
            .muted_alerts
            .get(self.change_stamp(), || self.muted_alerts())
    }

    /// How many of `event_id`'s webhook deliveries arrived, of how many were attempted; `None`
    /// when it posted to none. Kept per event until the next write.
    pub fn webhook_delivery_summary(&self, event_id: u64) -> Option<(usize, usize)> {
        self.frame_caches
            .webhook_summaries
            .get(self.change_stamp(), event_id, |&event_id| {
                let rows = self.webhook_deliveries(&[event_id]);
                let rows = rows.get(&event_id)?;
                let delivered = rows
                    .iter()
                    .filter(|row| {
                        row.state == crate::model::webhook::WebhookDeliveryState::Delivered
                    })
                    .count();
                Some((delivered, rows.len()))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::pine::tests::subscription;

    #[test]
    fn a_cached_read_is_reused_until_a_write() {
        let store = Store::open_in_memory().expect("open");
        store
            .replace_pine_subscriptions(&[subscription(1)])
            .expect("write");
        let first = store.pine_subscriptions_cached();
        assert_eq!(first.len(), 1);
        assert!(Arc::ptr_eq(&first, &store.pine_subscriptions_cached()));

        store
            .replace_pine_subscriptions(&[subscription(1), subscription(2)])
            .expect("write");
        let after = store.pine_subscriptions_cached();
        assert!(!Arc::ptr_eq(&first, &after));
        assert_eq!(
            after.iter().map(|s| s.id).collect::<Vec<_>>(),
            vec![1, 2],
            "the write is seen"
        );
    }

    /// A write that bypasses `replace_pine_subscriptions` (raw SQL, a restore) is seen too.
    #[test]
    fn a_raw_sql_write_is_seen() {
        let store = Store::open_in_memory().expect("open");
        assert!(store.pine_subscriptions_cached().is_empty());
        let payload = serde_json::to_string(&subscription(5)).unwrap();
        store
            .execute_raw(&format!(
                "INSERT INTO pine_alert_subscription (id, position, payload) VALUES ('5', 0, '{payload}')"
            ))
            .expect("raw write");
        assert_eq!(store.pine_subscriptions_cached()[0].id, 5);
    }

    /// Script alerts and endpoints stay on this Mac; for a synced table, sync applies a remote
    /// change through `write_raw_rows`, and the cached read sees it.
    #[test]
    fn a_remote_change_applied_by_sync_is_seen() {
        use crate::model::{SavedView, Timeframe};
        let store = Store::open_in_memory().expect("open");
        assert!(store.layout_index().layouts.is_empty());
        let view = SavedView {
            id: 8,
            name: "From another Mac".into(),
            timeframe: Timeframe::Hour,
            created_at_ms: 1_000,
            charts: Vec::new(),
            columns: None,
            candle_count: 0,
            last_opened_at_ms: None,
            autosave: None,
            sync: Default::default(),
        };
        store
            .write_raw_rows("saved_view", &[serde_json::to_value(view).unwrap()])
            .expect("sync write");
        assert_eq!(
            store.layout_index().layouts,
            vec![(8, "From another Mac".into())]
        );
    }

    #[test]
    fn a_write_from_another_connection_is_seen() {
        let path = std::env::temp_dir().join(format!(
            "degenview-frame-cache-{}.sqlite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let reader = Store::open(&path).expect("reader");
        let writer = Store::open(&path).expect("writer");
        assert!(reader.pine_subscriptions_cached().is_empty());
        writer
            .replace_pine_subscriptions(&[subscription(3)])
            .expect("write");
        assert_eq!(reader.pine_subscriptions_cached()[0].id, 3);

        drop((reader, writer));
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    /// A write inside a transaction that is rolled back moves no counter back, so nothing read
    /// inside one is kept.
    #[test]
    fn nothing_read_inside_a_transaction_is_kept() {
        let store = Store::open_in_memory().expect("open");
        store.execute_raw("BEGIN").expect("begin");
        store
            .replace_pine_subscriptions(&[subscription(4)])
            .expect("write");
        assert_eq!(store.pine_subscriptions_cached().len(), 1);
        store.execute_raw("ROLLBACK").expect("rollback");
        assert!(store.pine_subscriptions_cached().is_empty());
    }

    #[test]
    fn webhook_summaries_follow_new_deliveries() {
        use crate::model::webhook::WebhookDeliverySource;
        let store = Store::open_in_memory().expect("open");
        assert_eq!(store.webhook_delivery_summary(9), None);
        store
            .claim_webhook_delivery(1, WebhookDeliverySource::Price, 9, 1_000)
            .expect("claim");
        assert_eq!(store.webhook_delivery_summary(9), Some((0, 1)));
    }
}
