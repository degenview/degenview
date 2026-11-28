//! The persisted lookups a venue keeps: its symbol catalog, the chain a pool sits on.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::time::Duration;

/// A stored answer, how old it is, and whether it is still inside the TTL the caller asked about.
#[derive(Clone, Debug, PartialEq)]
pub struct Cached<T> {
    pub value: T,
    pub age: Duration,
    pub fresh: bool,
}

/// Answers kept between launches, by namespace and key, as JSON.
pub trait CacheStore: Send + Sync {
    /// The stored answer for `key`, however old, and whether it is younger than `ttl`.
    fn get_value(&self, namespace: &str, key: &str, ttl: Duration) -> Option<Cached<Value>>;

    fn put_value(&self, namespace: &str, key: &str, value: Value);
}

impl dyn CacheStore + '_ {
    /// [`CacheStore::get_value`] read as a `T`: an answer of another shape is no answer.
    pub fn get<T: DeserializeOwned>(
        &self,
        namespace: &str,
        key: &str,
        ttl: Duration,
    ) -> Option<Cached<T>> {
        let hit = self.get_value(namespace, key, ttl)?;
        Some(Cached {
            value: serde_json::from_value(hit.value).ok()?,
            age: hit.age,
            fresh: hit.fresh,
        })
    }

    pub fn put<T: Serialize>(&self, namespace: &str, key: &str, value: &T) {
        if let Ok(value) = serde_json::to_value(value) {
            self.put_value(namespace, key, value);
        }
    }
}
