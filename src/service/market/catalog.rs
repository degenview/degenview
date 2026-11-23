//! A provider's list of tradable symbols, downloaded once and searched locally.
//!
//! Binance, Coinbase and Alpaca have no search endpoint: they hand over the whole list (megabytes
//! for Binance and Alpaca) and the app filters it. Without a cache that download repeated for
//! every keystroke. A `Catalog` keeps the list in memory, mirrors it to the lookup cache so a
//! relaunch starts warm, and refreshes it after `TTL`. When a refresh fails the previous list is
//! served and the provider is left alone for `FAILURE_BACKOFF`, so an outage costs a search
//! nothing and does not turn into a retry per keystroke.

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
/// The least time between two downloads made because a market was not in the list.
const MISS_REFRESH: Duration = Duration::from_secs(600);
const KEY: &str = "all";

struct Loaded<T> {
    items: Arc<Vec<T>>,
    /// The list is used as it is until then.
    fresh_until: Instant,
}

pub struct Catalog<T> {
    namespace: &'static str,
    memory: Mutex<Option<Loaded<T>>>,
    /// Held while downloading, so concurrent searches share one request.
    refresh: Mutex<()>,
    /// When the list was last downloaded again because a market was not in it.
    last_miss: Mutex<Option<Instant>>,
}

impl<T: Serialize + DeserializeOwned> Catalog<T> {
    pub const fn new(namespace: &'static str) -> Self {
        Self {
            namespace,
            memory: Mutex::new(None),
            refresh: Mutex::new(()),
            last_miss: Mutex::new(None),
        }
    }

    /// The list, from memory, the lookup cache or — only when neither is fresh — `download`.
    pub fn get(
        &self,
        download: impl FnOnce() -> Result<Vec<T>, ProviderError>,
    ) -> Result<Arc<Vec<T>>, ProviderError> {
        self.get_from(lookup::shared(), download)
    }

    /// [`Self::get`], downloaded again when `found` finds no market in it: a market listed since
    /// the last download is not "not listed" for the day the list is held. A miss downloads at most
    /// once every ten minutes, so a mistyped name is one request and not one per keystroke; a
    /// download that fails or is empty leaves the list held as it is.
    pub fn get_or_refresh_on_miss(
        &self,
        download: impl Fn() -> Result<Vec<T>, ProviderError>,
        found: impl Fn(&[T]) -> bool,
    ) -> Result<Arc<Vec<T>>, ProviderError> {
        self.get_or_refresh_on_miss_from(lookup::shared(), Instant::now(), download, found)
    }

    fn get_or_refresh_on_miss_from(
        &self,
        cache: &LookupCache,
        now: Instant,
        download: impl Fn() -> Result<Vec<T>, ProviderError>,
        found: impl Fn(&[T]) -> bool,
    ) -> Result<Arc<Vec<T>>, ProviderError> {
        let items = self.get_from(cache, &download)?;
        if found(&items) {
            return Ok(items);
        }
        {
            let mut last = self
                .last_miss
                .lock()
                .map_err(|_| ProviderError::other("catalog unavailable"))?;
            if last.is_some_and(|at| now.saturating_duration_since(at) < MISS_REFRESH) {
                return Ok(items);
            }
            *last = Some(now);
        }
        let _refreshing = self
            .refresh
            .lock()
            .map_err(|_| ProviderError::other("catalog unavailable"))?;
        match download() {
            Ok(fresh) if !fresh.is_empty() => {
                cache.put(self.namespace, KEY, &fresh);
                let fresh = Arc::new(fresh);
                self.keep(&fresh, TTL);
                Ok(fresh)
            }
            _ => Ok(items),
        }
    }

    fn get_from(
        &self,
        cache: &LookupCache,
        download: impl FnOnce() -> Result<Vec<T>, ProviderError>,
    ) -> Result<Arc<Vec<T>>, ProviderError> {
        if let Some(items) = self.fresh() {
            return Ok(items);
        }
        let _refreshing = self
            .refresh
            .lock()
            .map_err(|_| ProviderError::other("catalog unavailable"))?;
        // Another search may have finished the download while this one waited.
        if let Some(items) = self.fresh() {
            return Ok(items);
        }
        let held = self
            .memory
            .lock()
            .ok()
            .and_then(|memory| memory.as_ref().map(|loaded| Arc::clone(&loaded.items)));
        let stale = match held {
            Some(items) => Some(items),
            None => match cache.get::<Vec<T>>(self.namespace, KEY, TTL) {
                Some(hit) if hit.fresh => {
                    let items = Arc::new(hit.value);
                    self.keep(&items, TTL.saturating_sub(hit.age));
                    return Ok(items);
                }
                Some(hit) => Some(Arc::new(hit.value)),
                None => None,
            },
        };
        match download() {
            Ok(items) if !items.is_empty() => {
                cache.put(self.namespace, KEY, &items);
                let items = Arc::new(items);
                self.keep(&items, TTL);
                Ok(items)
            }
            // An empty list is a provider hiccup, not an answer worth a day.
            Ok(_) => self.fall_back(
                stale,
                ProviderError::other("the provider returned no symbols"),
            ),
            Err(error) => self.fall_back(stale, error),
        }
    }

    fn fall_back(
        &self,
        stale: Option<Arc<Vec<T>>>,
        error: ProviderError,
    ) -> Result<Arc<Vec<T>>, ProviderError> {
        let items = stale.ok_or(error)?;
        self.keep(&items, FAILURE_BACKOFF);
        Ok(items)
    }

    fn fresh(&self) -> Option<Arc<Vec<T>>> {
        let memory = self.memory.lock().ok()?;
        let loaded = memory.as_ref()?;
        (Instant::now() < loaded.fresh_until).then(|| Arc::clone(&loaded.items))
    }

    fn keep(&self, items: &Arc<Vec<T>>, fresh_for: Duration) {
        if let Ok(mut memory) = self.memory.lock() {
            *memory = Some(Loaded {
                items: Arc::clone(items),
                fresh_until: Instant::now() + fresh_for,
            });
        }
    }

    /// The list as it is in memory, however old; nothing before the first download. For a caller
    /// that must not wait on the network (one that runs on the UI thread).
    pub fn held(&self) -> Option<Arc<Vec<T>>> {
        let memory = self.memory.lock().ok()?;
        memory.as_ref().map(|loaded| Arc::clone(&loaded.items))
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
    fn held_is_what_memory_has_and_never_downloads() {
        let fixture = Fixture::new();
        let (cache, catalog) = fixture.launch();
        assert!(catalog.held().is_none());
        catalog
            .get_from(&cache, fixture.download(Ok(pairs(&["BTCUSDT"]))))
            .unwrap();
        assert_eq!(catalog.held().map(|items| items.len()), Some(1));
        catalog.clear_memory();
        assert!(catalog.held().is_none());
        assert_eq!(fixture.downloads(), 1);
    }

    /// Downloads that answer `lists[n]` the n-th time, the last one after that.
    fn listing<'a>(
        fixture: &'a Fixture,
        lists: &'a [&'a [&'a str]],
    ) -> impl Fn() -> Result<Vec<Pair>, ProviderError> + 'a {
        move || {
            let made = fixture.downloads.fetch_add(1, Ordering::SeqCst);
            Ok(pairs(lists[made.min(lists.len() - 1)]))
        }
    }

    fn has(name: &'static str) -> impl Fn(&[Pair]) -> bool {
        move |items| items.iter().any(|pair| pair.0 == name)
    }

    #[test]
    fn a_market_the_list_lacks_is_looked_for_again_once_in_ten_minutes() {
        let fixture = Fixture::new();
        let (cache, catalog) = fixture.launch();
        let start = Instant::now();
        let download = listing(&fixture, &[&["BTC"], &["BTC", "NEW"]]);
        // A listed market costs the one download of the day.
        let items = catalog
            .get_or_refresh_on_miss_from(&cache, start, &download, has("BTC"))
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(fixture.downloads(), 1);

        // A market listed since is found by downloading again...
        let items = catalog
            .get_or_refresh_on_miss_from(&cache, start, &download, has("NEW"))
            .unwrap();
        assert_eq!(*items, pairs(&["BTC", "NEW"]));
        assert_eq!(fixture.downloads(), 2);

        // ...and a name the list never holds asks only once in the throttle.
        for minutes in [1, 5, 9] {
            let later = start + Duration::from_secs(minutes * 60);
            let items = catalog
                .get_or_refresh_on_miss_from(&cache, later, &download, has("NOPE"))
                .unwrap();
            assert_eq!(items.len(), 2);
        }
        assert_eq!(fixture.downloads(), 2, "the miss at the start was the last");
        let later = start + MISS_REFRESH + Duration::from_secs(1);
        catalog
            .get_or_refresh_on_miss_from(&cache, later, &download, has("NOPE"))
            .unwrap();
        assert_eq!(fixture.downloads(), 3);
    }

    #[test]
    fn a_failed_or_empty_download_on_a_miss_keeps_the_list() {
        let fixture = Fixture::new();
        let (cache, catalog) = fixture.launch();
        let start = Instant::now();
        catalog
            .get_from(&cache, fixture.download(Ok(pairs(&["BTC"]))))
            .unwrap();
        let failing =
            || -> Result<Vec<Pair>, ProviderError> { Err(ProviderError::other("offline")) };
        let items = catalog
            .get_or_refresh_on_miss_from(&cache, start, failing, has("NEW"))
            .unwrap();
        assert_eq!(*items, pairs(&["BTC"]));
        let later = start + MISS_REFRESH + Duration::from_secs(1);
        let empty = || -> Result<Vec<Pair>, ProviderError> { Ok(Vec::new()) };
        let items = catalog
            .get_or_refresh_on_miss_from(&cache, later, empty, has("NEW"))
            .unwrap();
        assert_eq!(*items, pairs(&["BTC"]));
        assert_eq!(catalog.held().map(|held| held.len()), Some(1));
    }

    #[test]
    fn the_list_is_downloaded_once_however_often_it_is_searched() {
        let fixture = Fixture::new();
        let (cache, catalog) = fixture.launch();
        for _ in 0..5 {
            let items = catalog
                .get_from(&cache, fixture.download(Ok(pairs(&["BTCUSDT", "ETHUSDT"]))))
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
            .get_from(&cache, fixture.download(Ok(pairs(&["BTCUSDT"]))))
            .unwrap();
        cache.flush("catalog_test");

        let (cache, catalog) = fixture.launch();
        let items = catalog
            .get_from(
                &cache,
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
            .get_from(&cache, fixture.download(Ok(pairs(&["OLD"]))))
            .unwrap();
        cache.flush("catalog_test");
        fixture.clock.fetch_add(25 * 3_600_000, Ordering::Relaxed);

        let (cache, catalog) = fixture.launch();
        let items = catalog
            .get_from(&cache, fixture.download(Ok(pairs(&["NEW"]))))
            .unwrap();
        assert_eq!(*items, pairs(&["NEW"]));
        assert_eq!(fixture.downloads(), 2);
    }

    #[test]
    fn a_failed_refresh_serves_the_stale_list_and_backs_off() {
        let fixture = Fixture::new();
        let (cache, catalog) = fixture.launch();
        catalog
            .get_from(&cache, fixture.download(Ok(pairs(&["OLD"]))))
            .unwrap();
        cache.flush("catalog_test");
        fixture.clock.fetch_add(25 * 3_600_000, Ordering::Relaxed);

        let (cache, catalog) = fixture.launch();
        for _ in 0..4 {
            let items = catalog
                .get_from(
                    &cache,
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
                fixture.download(Err(ProviderError::other("offline"))),
            )
            .unwrap_err();
        assert_eq!(error, ProviderError::other("offline"));
        let items = catalog
            .get_from(&cache, fixture.download(Ok(pairs(&["BTCUSDT"]))))
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
                .get_from(&cache, fixture.download(Ok(Vec::new())))
                .is_err()
        );
        cache.flush("catalog_test");
        let (cache, catalog) = fixture.launch();
        let items = catalog
            .get_from(&cache, fixture.download(Ok(pairs(&["A"]))))
            .unwrap();
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn clearing_memory_makes_the_next_search_reload() {
        let fixture = Fixture::new();
        let (cache, catalog) = fixture.launch();
        catalog
            .get_from(&cache, fixture.download(Ok(pairs(&["A"]))))
            .unwrap();
        catalog.clear_memory();
        // Still in the lookup cache, so no download.
        catalog
            .get_from(
                &cache,
                fixture.download(Err(ProviderError::other("not asked"))),
            )
            .unwrap();
        assert_eq!(fixture.downloads(), 1);
    }
}
