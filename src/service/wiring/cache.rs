use crate::service::cache::lookup::{self, Hit};
use crate::service::market::ports::cache::CacheStore;
use serde_json::Value;
use std::time::Duration;

/// The app's lookup cache, which icons and the coin index share.
pub struct SharedLookups;

impl CacheStore for SharedLookups {
    fn get_value(&self, namespace: &str, key: &str, ttl: Duration) -> Option<Hit<Value>> {
        lookup::shared().get_value(namespace, key, ttl)
    }

    fn put_value(&self, namespace: &str, key: &str, value: Value) {
        lookup::shared().put_value(namespace, key, value);
    }
}
