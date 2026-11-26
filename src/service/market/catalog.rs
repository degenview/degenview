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

use crate::service::ProviderError;
use crate::service::cache::lookup::{self, LookupCache};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a downloaded list is used without asking again.
pub const TTL: Duration = Duration::from_secs(24 * 3_600);
/// How long to leave a provider alone after a failed (or empty) download.
const FAILURE_BACKOFF: Duration = Duration::from_secs(300);
/// The owner of a list that is the same for everybody.
const EVERYONE: &str = "all";

struct Loaded<T> {
    owner: String,
    items: Arc<Vec<T>>,
    /// The list is used as it is until then.
    fresh_until: Instant,
}

pub struct Catalog<T> {
    namespace: &'static str,
    memory: Mutex<Option<Loaded<T>>>,
    /// Held while downloading, so concurrent searches share one request.
    refresh: Mutex<()>,
}

impl<T: Serialize + DeserializeOwned> Catalog<T> {
    pub const fn new(namespace: &'static str) -> Self {
        Self {
            namespace,
            memory: Mutex::new(None),
            refresh: Mutex::new(()),
        }
    }

    /// The provider's public list: `get_for` for everybody.
    pub fn get(
        &self,
        download: impl FnOnce() -> Result<Vec<T>, ProviderError>,
    ) -> Result<Arc<Vec<T>>, ProviderError> {
        self.get_for(EVERYONE, download)
    }

    /// `owner`'s list, from memory, the lookup cache or — only when neither is fresh —
    /// `download`.
    pub fn get_for(
        &self,
        owner: &str,
        download: impl FnOnce() -> Result<Vec<T>, ProviderError>,
    ) -> Result<Arc<Vec<T>>, ProviderError> {
        self.get_from(lookup::shared(), owner, download)
    }

    fn get_from(
        &self,
        cache: &LookupCache,
        owner: &str,
        download: impl FnOnce() -> Result<Vec<T>, ProviderError>,
    ) -> Result<Arc<Vec<T>>, ProviderError> {
        if let Some(items) = self.fresh(owner) {
            return Ok(items);
        }
        let _refreshing = self
            .refresh
            .lock()
            .map_err(|_| ProviderError::other("catalog unavailable"))?;
        // Another search may have finished the download while this one waited.
        if let Some(items) = self.fresh(owner) {
            return Ok(items);
        }
        let stale = match self.held(owner) {
            Some((items, _)) => Some(items),
            None => match cache.get::<Vec<T>>(self.namespace, owner, TTL) {
                Some(hit) if hit.fresh => {
                    let items = Arc::new(hit.value);
                    self.keep(owner, &items, TTL.saturating_sub(hit.age));
                    return Ok(items);
                }
                Some(hit) => Some(Arc::new(hit.value)),
                None => None,
            },
        };
        match download() {
            Ok(items) if !items.is_empty() => {
                cache.put(self.namespace, owner, &items);
                let items = Arc::new(items);
                self.keep(owner, &items, TTL);
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
        &self,
        owner: &str,
        stale: Option<Arc<Vec<T>>>,
        error: ProviderError,
    ) -> Result<Arc<Vec<T>>, ProviderError> {
        let items = stale.ok_or(error)?;
        self.keep(owner, &items, FAILURE_BACKOFF);
        Ok(items)
    }

    /// `owner`'s list if memory holds it, and until when it is used as it is.
    fn held(&self, owner: &str) -> Option<(Arc<Vec<T>>, Instant)> {
        let memory = self.memory.lock().ok()?;
        let loaded = memory.as_ref().filter(|loaded| loaded.owner == owner)?;
        Some((Arc::clone(&loaded.items), loaded.fresh_until))
    }

    fn fresh(&self, owner: &str) -> Option<Arc<Vec<T>>> {
        self.held(owner)
            .filter(|(_, fresh_until)| Instant::now() < *fresh_until)
            .map(|(items, _)| items)
    }

    fn keep(&self, owner: &str, items: &Arc<Vec<T>>, fresh_for: Duration) {
        if let Ok(mut memory) = self.memory.lock() {
            *memory = Some(Loaded {
                owner: owner.to_owned(),
                items: Arc::clone(items),
                fresh_until: Instant::now() + fresh_for,
            });
        }
    }

    pub fn clear_memory(&self) {
        if let Ok(mut memory) = self.memory.lock() {
            *memory = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::cache::disk::{DiskCache, testing::TempCache};
    use serde::Deserialize;
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    struct Pair(String);

    fn pairs(names: &[&str]) -> Vec<Pair> {
        names.iter().map(|name| Pair((*name).to_owned())).collect()
    }

    struct Fixture {
        temp: TempCache,
        clock: Arc<AtomicI64>,
        downloads: AtomicUsize,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                temp: TempCache::new(),
                clock: Arc::new(AtomicI64::new(1_000 * 86_400_000)),
                downloads: AtomicUsize::new(0),
            }
        }

        /// A lookup cache over the shared directory, and a `Catalog` with empty memory: a launch.
        fn launch(&self) -> (LookupCache, Catalog<Pair>) {
            let disk: &'static DiskCache = Box::leak(Box::new(DiskCache::at(self.temp.path())));
            let clock = Arc::clone(&self.clock);
            let cache =
                LookupCache::for_tests(disk, Box::new(move || clock.load(Ordering::Relaxed)));
            (cache, Catalog::new("catalog_test"))
        }

        fn download(
            &self,
            result: Result<Vec<Pair>, ProviderError>,
        ) -> impl FnOnce() -> Result<Vec<Pair>, ProviderError> + '_ {
            move || {
                self.downloads.fetch_add(1, Ordering::SeqCst);
                result
            }
        }

        fn downloads(&self) -> usize {
            self.downloads.load(Ordering::SeqCst)
        }
    }

    #[test]
    fn the_list_is_downloaded_once_however_often_it_is_searched() {
        let fixture = Fixture::new();
        let (cache, catalog) = fixture.launch();
        for _ in 0..5 {
            let items = catalog
                .get_from(
                    &cache,
                    EVERYONE,
                    fixture.download(Ok(pairs(&["BTCUSDT", "ETHUSDT"]))),
                )
                .unwrap();
            assert_eq!(items.len(), 2);
        }
        assert_eq!(fixture.downloads(), 1);
    }

    #[test]
    fn a_relaunch_starts_from_the_persisted_list() {
        let fixture = Fixture::new();
        let (cache, catalog) = fixture.launch();
        catalog
            .get_from(&cache, EVERYONE, fixture.download(Ok(pairs(&["BTCUSDT"]))))
            .unwrap();
        cache.flush("catalog_test");

        let (cache, catalog) = fixture.launch();
        let items = catalog
            .get_from(
                &cache,
                EVERYONE,
                fixture.download(Err(ProviderError::other("must not be asked"))),
            )
            .unwrap();
        assert_eq!(*items, pairs(&["BTCUSDT"]));
        assert_eq!(fixture.downloads(), 1);
    }

    #[test]
    fn a_list_past_its_ttl_is_downloaded_again() {
        let fixture = Fixture::new();
        let (cache, catalog) = fixture.launch();
        catalog
            .get_from(&cache, EVERYONE, fixture.download(Ok(pairs(&["OLD"]))))
            .unwrap();
        cache.flush("catalog_test");
        fixture.clock.fetch_add(25 * 3_600_000, Ordering::Relaxed);

        let (cache, catalog) = fixture.launch();
        let items = catalog
            .get_from(&cache, EVERYONE, fixture.download(Ok(pairs(&["NEW"]))))
            .unwrap();
        assert_eq!(*items, pairs(&["NEW"]));
        assert_eq!(fixture.downloads(), 2);
    }

    #[test]
    fn a_failed_refresh_serves_the_stale_list_and_backs_off() {
        let fixture = Fixture::new();
        let (cache, catalog) = fixture.launch();
        catalog
            .get_from(&cache, EVERYONE, fixture.download(Ok(pairs(&["OLD"]))))
            .unwrap();
        cache.flush("catalog_test");
        fixture.clock.fetch_add(25 * 3_600_000, Ordering::Relaxed);

        let (cache, catalog) = fixture.launch();
        for _ in 0..4 {
            let items = catalog
                .get_from(
                    &cache,
                    EVERYONE,
                    fixture.download(Err(ProviderError::other("offline"))),
                )
                .unwrap();
            assert_eq!(*items, pairs(&["OLD"]));
        }
        assert_eq!(
            fixture.downloads(),
            2,
            "one failed attempt, then the back-off"
        );
    }

    #[test]
    fn with_nothing_cached_a_failure_is_an_error_and_is_retried() {
        let fixture = Fixture::new();
        let (cache, catalog) = fixture.launch();
        let error = catalog
            .get_from(
                &cache,
                EVERYONE,
                fixture.download(Err(ProviderError::other("offline"))),
            )
            .unwrap_err();
        assert_eq!(error, ProviderError::other("offline"));
        let items = catalog
            .get_from(&cache, EVERYONE, fixture.download(Ok(pairs(&["BTCUSDT"]))))
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(fixture.downloads(), 2);
    }

    #[test]
    fn an_empty_list_is_neither_trusted_nor_persisted() {
        let fixture = Fixture::new();
        let (cache, catalog) = fixture.launch();
        assert!(
            catalog
                .get_from(&cache, EVERYONE, fixture.download(Ok(Vec::new())))
                .is_err()
        );
        cache.flush("catalog_test");
        let (cache, catalog) = fixture.launch();
        let items = catalog
            .get_from(&cache, EVERYONE, fixture.download(Ok(pairs(&["A"]))))
            .unwrap();
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn clearing_memory_makes_the_next_search_reload() {
        let fixture = Fixture::new();
        let (cache, catalog) = fixture.launch();
        catalog
            .get_from(&cache, EVERYONE, fixture.download(Ok(pairs(&["A"]))))
            .unwrap();
        catalog.clear_memory();
        // Still in the lookup cache, so no download.
        catalog
            .get_from(
                &cache,
                EVERYONE,
                fixture.download(Err(ProviderError::other("not asked"))),
            )
            .unwrap();
        assert_eq!(fixture.downloads(), 1);
    }

    /// The lists `owner` is answered with, asking the provider for `names` if it has to.
    fn ask(
        catalog: &Catalog<Pair>,
        cache: &LookupCache,
        owner: &str,
        names: &[&str],
    ) -> Arc<Vec<Pair>> {
        catalog
            .get_from(cache, owner, || Ok(pairs(names)))
            .expect("a list")
    }

    #[test]
    fn each_owner_has_its_own_list_across_relaunches() {
        let fixture = Fixture::new();
        let (cache, catalog) = fixture.launch();
        assert_eq!(
            *ask(&catalog, &cache, "a", &["EUR_USD"]),
            pairs(&["EUR_USD"])
        );
        assert_eq!(
            *ask(&catalog, &cache, "b", &["USD_JPY"]),
            pairs(&["USD_JPY"])
        );
        cache.flush("catalog_test");

        let (cache, catalog) = fixture.launch();
        assert_eq!(*ask(&catalog, &cache, "a", &["asked"]), pairs(&["EUR_USD"]));
        assert_eq!(*ask(&catalog, &cache, "b", &["asked"]), pairs(&["USD_JPY"]));
    }

    #[test]
    fn memory_releases_the_previous_owners_list() {
        let fixture = Fixture::new();
        let (cache, catalog) = fixture.launch();
        let first = Arc::downgrade(&ask(&catalog, &cache, "a", &["EUR_USD"]));
        assert!(
            first.upgrade().is_some(),
            "held while it is the last asked for"
        );
        ask(&catalog, &cache, "b", &["USD_JPY"]);
        assert!(first.upgrade().is_none());
    }

    #[test]
    fn a_failed_download_never_answers_with_another_owners_list() {
        let fixture = Fixture::new();
        let (cache, catalog) = fixture.launch();
        ask(&catalog, &cache, "a", &["EUR_USD"]);
        let error = catalog
            .get_from(&cache, "b", || Err(ProviderError::other("offline")))
            .unwrap_err();
        assert_eq!(error, ProviderError::other("offline"));
    }
}
