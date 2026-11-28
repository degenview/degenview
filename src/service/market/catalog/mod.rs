//! A provider's list of tradable symbols, downloaded once and searched locally.
//!
//! Binance, Coinbase and Alpaca have no search endpoint: they hand over the whole list (megabytes
//! for Binance and Alpaca) and the app filters it. Without a cache that download repeated for
//! every keystroke. A `Catalog` keeps the list in memory, mirrors it to the lookup cache so a
//! relaunch starts warm, and refreshes it after `TTL`. When a refresh fails the previous list is
//! served and the provider is left alone for `FAILURE_BACKOFF`, so an outage costs a search
//! nothing and does not turn into a retry per keystroke.
//!
//! A list has an owner. Most providers publish one list for everybody; a list that depends on who
//! asks (an account's instruments) is asked for by that owner, and no owner is ever answered with
//! another's. Memory holds the list of the last owner asked for, the lookup cache each owner's.
//!
//! [`search`] is how a venue answers a query from its list, and [`registry`] forgets every list held
//! in memory when the cache is cleared: a catalog registers itself when it first keeps one, so a
//! catalog is a `static`.

pub(in crate::service) mod registry;
pub(in crate::service) mod search;

use crate::lock::lock;
use crate::service::ProviderError;
use crate::service::cache::lookup::{self, LookupCache};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

/// How long a downloaded list is used without asking again.
pub const TTL: Duration = Duration::from_secs(24 * 3_600);
/// How long to leave a provider alone after a failed (or empty) download.
const FAILURE_BACKOFF: Duration = Duration::from_secs(300);
/// How old a list must be before it is downloaded again for lacking a market asked for.
const RECHECK_AFTER: Duration = Duration::from_secs(300);
/// The owner of a list that is the same for everybody.
const EVERYONE: &str = "all";

struct Loaded<T> {
    owner: String,
    items: Arc<Vec<T>>,
    /// The list is used as it is until then.
    fresh_until: Instant,
    /// A market it lacks is not looked for in a new one before then.
    recheck_after: Instant,
    /// Why the last download failed, when the list is held in spite of it.
    failed: Option<ProviderError>,
}

pub struct Catalog<T> {
    namespace: &'static str,
    memory: Mutex<Option<Loaded<T>>>,
    /// Held while downloading, so concurrent searches share one request.
    refresh: Mutex<()>,
    /// Adds the catalog to the `registry` with the first list it keeps.
    registered: Once,
}

impl<T: Serialize + DeserializeOwned + Send + Sync> Catalog<T> {
    pub const fn new(namespace: &'static str) -> Self {
        Self {
            namespace,
            memory: Mutex::new(None),
            refresh: Mutex::new(()),
            registered: Once::new(),
        }
    }

    /// The provider's public list: `get_for` for everybody.
    pub fn get(
        &'static self,
        download: impl FnOnce() -> Result<Vec<T>, ProviderError>,
    ) -> Result<Arc<Vec<T>>, ProviderError> {
        self.get_for(EVERYONE, download)
    }

    /// The public list for a market `listed` accepts. A list without one may be older than the
    /// market, so it is downloaded again, though not within `RECHECK_AFTER` of the last time:
    /// a market that does not exist must not cost a download at every retry. A list that could
    /// not be brought up to date says nothing of what it lacks: that is the download's error.
    pub fn get_listing(
        &'static self,
        listed: impl Fn(&T) -> bool,
        download: impl Fn() -> Result<Vec<T>, ProviderError>,
    ) -> Result<Arc<Vec<T>>, ProviderError> {
        self.get_listing_from(lookup::shared(), listed, download)
    }

    fn get_listing_from(
        &'static self,
        cache: &LookupCache,
        listed: impl Fn(&T) -> bool,
        download: impl Fn() -> Result<Vec<T>, ProviderError>,
    ) -> Result<Arc<Vec<T>>, ProviderError> {
        let items = self.get_from(cache, EVERYONE, &download)?;
        if items.iter().any(&listed) {
            return Ok(items);
        }
        let _refreshing = lock(&self.refresh);
        let current = match self.held(EVERYONE) {
            // Too new to ask again, or another caller has just done so.
            Some((held, _, recheck_after)) if Instant::now() < recheck_after => held,
            _ => self.refresh_from(cache, EVERYONE, Some(items), download)?,
        };
        match self.failure(EVERYONE) {
            Some(error) if !current.iter().any(&listed) => Err(error),
            _ => Ok(current),
        }
    }

    /// `owner`'s list, from memory, the lookup cache or — only when neither is fresh —
    /// `download`.
    pub fn get_for(
        &'static self,
        owner: &str,
        download: impl FnOnce() -> Result<Vec<T>, ProviderError>,
    ) -> Result<Arc<Vec<T>>, ProviderError> {
        self.get_from(lookup::shared(), owner, download)
    }

    fn get_from(
        &'static self,
        cache: &LookupCache,
        owner: &str,
        download: impl FnOnce() -> Result<Vec<T>, ProviderError>,
    ) -> Result<Arc<Vec<T>>, ProviderError> {
        if let Some(items) = self.fresh(owner) {
            return Ok(items);
        }
        let _refreshing = lock(&self.refresh);
        // Another search may have finished the download while this one waited.
        if let Some(items) = self.fresh(owner) {
            return Ok(items);
        }
        let stale = match self.held(owner) {
            Some((items, ..)) => Some(items),
            None => match cache.get::<Vec<T>>(self.namespace, owner, TTL) {
                Some(hit) if hit.fresh => {
                    let items = Arc::new(hit.value);
                    self.keep(
                        owner,
                        &items,
                        TTL.saturating_sub(hit.age),
                        RECHECK_AFTER.saturating_sub(hit.age),
                        None,
                    );
                    return Ok(items);
                }
                Some(hit) => Some(Arc::new(hit.value)),
                None => None,
            },
        };
        self.refresh_from(cache, owner, stale, download)
    }

    /// Downloads `owner`'s list and keeps it, or `stale` when that fails. The caller holds
    /// `refresh`.
    fn refresh_from(
        &'static self,
        cache: &LookupCache,
        owner: &str,
        stale: Option<Arc<Vec<T>>>,
        download: impl FnOnce() -> Result<Vec<T>, ProviderError>,
    ) -> Result<Arc<Vec<T>>, ProviderError> {
        // A download that panics has failed like one that errs: asked again at every search it
        // would never stop (Alpaca's list is tens of megabytes).
        let downloaded = match catch_unwind(AssertUnwindSafe(download)) {
            Ok(downloaded) => downloaded,
            Err(panic) => {
                if let Some(items) = &stale {
                    self.keep_stale(owner, items, ProviderError::other("the download panicked"));
                }
                resume_unwind(panic);
            }
        };
        match downloaded {
            Ok(items) if !items.is_empty() => {
                cache.put(self.namespace, owner, &items);
                let items = Arc::new(items);
                self.keep(owner, &items, TTL, RECHECK_AFTER, None);
                Ok(items)
            }
            // An empty list is a provider hiccup, not an answer worth a day.
            Ok(_) => self.fall_back(
                owner,
                stale,
                ProviderError::other("the provider returned no symbols"),
            ),
            Err(error) => self.fall_back(owner, stale, error),
        }
    }

    fn fall_back(
        &'static self,
        owner: &str,
        stale: Option<Arc<Vec<T>>>,
        error: ProviderError,
    ) -> Result<Arc<Vec<T>>, ProviderError> {
        let Some(items) = stale else {
            return Err(error);
        };
        self.keep_stale(owner, &items, error);
        Ok(items)
    }

    /// Keeps `items` after a failed download and leaves the provider alone for `FAILURE_BACKOFF`.
    fn keep_stale(&'static self, owner: &str, items: &Arc<Vec<T>>, error: ProviderError) {
        // A list still within its day stays so: one failed look does not age it.
        let still_fresh = self.held(owner).map_or(Duration::ZERO, |(_, until, _)| {
            until.saturating_duration_since(Instant::now())
        });
        self.keep(
            owner,
            items,
            still_fresh.max(FAILURE_BACKOFF),
            FAILURE_BACKOFF,
            Some(error),
        );
    }

    /// `owner`'s list if memory holds it, until when it is used as it is, and from when it may
    /// be rechecked.
    fn held(&self, owner: &str) -> Option<(Arc<Vec<T>>, Instant, Instant)> {
        let memory = lock(&self.memory);
        let loaded = memory.as_ref().filter(|loaded| loaded.owner == owner)?;
        Some((
            Arc::clone(&loaded.items),
            loaded.fresh_until,
            loaded.recheck_after,
        ))
    }

    /// Why `owner`'s list in memory could not be brought up to date, if the last try failed.
    fn failure(&self, owner: &str) -> Option<ProviderError> {
        let memory = lock(&self.memory);
        memory
            .as_ref()
            .filter(|loaded| loaded.owner == owner)?
            .failed
            .clone()
    }

    fn fresh(&self, owner: &str) -> Option<Arc<Vec<T>>> {
        self.held(owner)
            .filter(|(_, fresh_until, _)| Instant::now() < *fresh_until)
            .map(|(items, ..)| items)
    }

    fn keep(
        &'static self,
        owner: &str,
        items: &Arc<Vec<T>>,
        fresh_for: Duration,
        recheck_in: Duration,
        failed: Option<ProviderError>,
    ) {
        self.registered.call_once(|| registry::register(self));
        *lock(&self.memory) = Some(Loaded {
            owner: owner.to_owned(),
            items: Arc::clone(items),
            fresh_until: Instant::now() + fresh_for,
            recheck_after: Instant::now() + recheck_in,
            failed,
        });
    }
}

impl<T> Catalog<T> {
    fn clear_memory(&self) {
        *lock(&self.memory) = None;
    }
}

#[cfg(test)]
mod tests;
