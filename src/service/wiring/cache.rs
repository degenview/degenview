use crate::service::cache::lookup;
use crate::service::market::ports::cache::{CacheStore, Cached};
use serde_json::Value;
use std::time::Duration;

/// The app's lookup cache, which icons and the coin index share.
pub struct SharedLookups;

impl CacheStore for SharedLookups {
    fn get_value(&self, namespace: &str, key: &str, ttl: Duration) -> Option<Cached<Value>> {
        let hit = lookup::shared().get_value(namespace, key, ttl)?;
        Some(Cached {
            value: hit.value,
            age: hit.age,
            fresh: hit.fresh,
        })
    }

    fn put_value(&self, namespace: &str, key: &str, value: Value) {
        lookup::shared().put_value(namespace, key, value);
    }
}
