use super::*;
use crate::model::DataSource;
use crate::service::market::ports::fakes::Rig;
use serde::Deserialize;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct Pair(String);

fn pairs(names: &[&str]) -> Vec<Pair> {
    names.iter().map(|name| Pair((*name).to_owned())).collect()
}

struct Fixture {
    rig: Rig,
    downloads: AtomicUsize,
}

impl Fixture {
    fn new() -> Self {
        Self {
            rig: Rig::new(),
            downloads: AtomicUsize::new(0),
        }
    }

    /// A `Catalog` with empty memory over the lookup cache the fixture keeps: a launch.
    fn launch(&self) -> Catalog<Pair> {
        Catalog::new(
            Listing::spot(DataSource::Bybit),
            "catalog_test",
            self.rig.cache.clone(),
        )
    }

    fn pass(&self, time: Duration) {
        self.rig.clock.advance(time);
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
    let catalog = fixture.launch();
    for _ in 0..5 {
        let items = catalog
            .get_for(
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
    let catalog = fixture.launch();
    catalog
        .get_for(EVERYONE, fixture.download(Ok(pairs(&["BTCUSDT"]))))
        .unwrap();

    let catalog = fixture.launch();
    let items = catalog
        .get_for(
            EVERYONE,
            fixture.download(Err(ProviderError::refused(
                crate::model::DataSource::Bybit,
                "must not be asked",
            ))),
        )
        .unwrap();
    assert_eq!(*items, pairs(&["BTCUSDT"]));
    assert_eq!(fixture.downloads(), 1);
}

#[test]
fn a_list_past_its_ttl_is_downloaded_again() {
    let fixture = Fixture::new();
    let catalog = fixture.launch();
    catalog
        .get_for(EVERYONE, fixture.download(Ok(pairs(&["OLD"]))))
        .unwrap();
    fixture.pass(Duration::from_secs(25 * 3_600));

    let catalog = fixture.launch();
    let items = catalog
        .get_for(EVERYONE, fixture.download(Ok(pairs(&["NEW"]))))
        .unwrap();
    assert_eq!(*items, pairs(&["NEW"]));
    assert_eq!(fixture.downloads(), 2);
}

#[test]
fn a_failed_refresh_serves_the_stale_list_and_backs_off() {
    let fixture = Fixture::new();
    let catalog = fixture.launch();
    catalog
        .get_for(EVERYONE, fixture.download(Ok(pairs(&["OLD"]))))
        .unwrap();
    fixture.pass(Duration::from_secs(25 * 3_600));

    let catalog = fixture.launch();
    for _ in 0..4 {
        let items = catalog
            .get_for(
                EVERYONE,
                fixture.download(Err(ProviderError::refused(
                    crate::model::DataSource::Bybit,
                    "offline",
                ))),
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
    let catalog = fixture.launch();
    let error = catalog
        .get_for(
            EVERYONE,
            fixture.download(Err(ProviderError::refused(
                crate::model::DataSource::Bybit,
                "offline",
            ))),
        )
        .unwrap_err();
    assert_eq!(
        error,
        ProviderError::refused(crate::model::DataSource::Bybit, "offline")
    );
    let items = catalog
        .get_for(EVERYONE, fixture.download(Ok(pairs(&["BTCUSDT"]))))
        .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(fixture.downloads(), 2);
}

#[test]
fn an_empty_list_is_neither_trusted_nor_persisted() {
    let fixture = Fixture::new();
    let catalog = fixture.launch();
    assert!(
        catalog
            .get_for(EVERYONE, fixture.download(Ok(Vec::new())))
            .is_err()
    );
    let catalog = fixture.launch();
    let items = catalog
        .get_for(EVERYONE, fixture.download(Ok(pairs(&["A"]))))
        .unwrap();
    assert_eq!(items.len(), 1);
}

#[test]
fn clearing_memory_makes_the_next_search_reload() {
    let fixture = Fixture::new();
    let catalog = fixture.launch();
    catalog
        .get_for(EVERYONE, fixture.download(Ok(pairs(&["A"]))))
        .unwrap();
    catalog.clear_memory();
    // Still in the lookup cache, so no download.
    catalog
        .get_for(
            EVERYONE,
            fixture.download(Err(ProviderError::refused(
                crate::model::DataSource::Bybit,
                "not asked",
            ))),
        )
        .unwrap();
    assert_eq!(fixture.downloads(), 1);
}

#[test]
fn a_market_the_list_lacks_is_looked_for_again_but_not_at_every_ask() {
    let fixture = Fixture::new();
    let catalog = fixture.launch();
    let new = |pair: &Pair| pair.0 == "NEW";
    let look = |names: &[&str]| {
        catalog
            .get_listing(new, || {
                fixture.downloads.fetch_add(1, Ordering::SeqCst);
                Ok(pairs(names))
            })
            .expect("a list")
    };
    // Just downloaded: what it lacks does not exist yet.
    for _ in 0..3 {
        assert_eq!(*look(&["OLD"]), pairs(&["OLD"]));
    }
    assert_eq!(fixture.downloads(), 1);

    // Old enough to have missed a listing.
    catalog
        .memory
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .recheck_after = Instant::now();
    assert_eq!(*look(&["OLD", "NEW"]), pairs(&["OLD", "NEW"]));
    assert_eq!(fixture.downloads(), 2);
    // Found: held as any list is.
    look(&["NOT ASKED"]);
    assert_eq!(fixture.downloads(), 2);
}

#[test]
fn a_failed_look_for_a_market_is_an_error_and_leaves_the_list_its_day() {
    let fixture = Fixture::new();
    let catalog = fixture.launch();
    catalog
        .get_for(EVERYONE, fixture.download(Ok(pairs(&["OLD"]))))
        .unwrap();
    let offline = || {
        fixture.downloads.fetch_add(1, Ordering::SeqCst);
        Err(ProviderError::refused(
            crate::model::DataSource::Bybit,
            "offline",
        ))
    };
    let ask = |wanted: &str| catalog.get_listing(|pair: &Pair| pair.0 == wanted, offline);
    catalog
        .memory
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .recheck_after = Instant::now();
    // The same answer until it may be asked again, from one download.
    for _ in 0..3 {
        assert_eq!(
            ask("NEW").unwrap_err(),
            ProviderError::refused(crate::model::DataSource::Bybit, "offline")
        );
    }
    assert_eq!(fixture.downloads(), 2);
    // What the list does hold is still answered, and it is as fresh as before.
    assert_eq!(*ask("OLD").unwrap(), pairs(&["OLD"]));
    let (_, fresh_until, _) = catalog.held(EVERYONE).unwrap();
    assert!(fresh_until > Instant::now() + TTL - FAILURE_BACKOFF);
}

#[test]
fn a_stale_list_that_cannot_be_refreshed_does_not_say_a_market_is_unlisted() {
    let fixture = Fixture::new();
    let catalog = fixture.launch();
    catalog
        .get_for(EVERYONE, fixture.download(Ok(pairs(&["OLD"]))))
        .unwrap();
    fixture.pass(Duration::from_secs(25 * 3_600));

    let catalog = fixture.launch();
    let answer = catalog.get_listing(
        |pair: &Pair| pair.0 == "NEW",
        || {
            Err(ProviderError::refused(
                crate::model::DataSource::Bybit,
                "offline",
            ))
        },
    );
    assert_eq!(
        answer.unwrap_err(),
        ProviderError::refused(crate::model::DataSource::Bybit, "offline")
    );
}

/// The lists `owner` is answered with, asking the provider for `names` if it has to.
fn ask(catalog: &Catalog<Pair>, owner: &str, names: &[&str]) -> Arc<Vec<Pair>> {
    catalog.get_for(owner, || Ok(pairs(names))).expect("a list")
}

#[test]
fn each_owner_has_its_own_list_across_relaunches() {
    let fixture = Fixture::new();
    let catalog = fixture.launch();
    assert_eq!(*ask(&catalog, "a", &["EUR_USD"]), pairs(&["EUR_USD"]));
    assert_eq!(*ask(&catalog, "b", &["USD_JPY"]), pairs(&["USD_JPY"]));

    let catalog = fixture.launch();
    assert_eq!(*ask(&catalog, "a", &["asked"]), pairs(&["EUR_USD"]));
    assert_eq!(*ask(&catalog, "b", &["asked"]), pairs(&["USD_JPY"]));
}

#[test]
fn memory_releases_the_previous_owners_list() {
    let fixture = Fixture::new();
    let catalog = fixture.launch();
    let first = Arc::downgrade(&ask(&catalog, "a", &["EUR_USD"]));
    assert!(
        first.upgrade().is_some(),
        "held while it is the last asked for"
    );
    ask(&catalog, "b", &["USD_JPY"]);
    assert!(first.upgrade().is_none());
}

#[test]
fn a_failed_download_never_answers_with_another_owners_list() {
    let fixture = Fixture::new();
    let catalog = fixture.launch();
    ask(&catalog, "a", &["EUR_USD"]);
    let error = catalog
        .get_for("b", || {
            Err(ProviderError::refused(
                crate::model::DataSource::Bybit,
                "offline",
            ))
        })
        .unwrap_err();
    assert_eq!(
        error,
        ProviderError::refused(crate::model::DataSource::Bybit, "offline")
    );
}

#[test]
fn a_panic_while_the_list_is_locked_does_not_make_search_unavailable() {
    let fixture = Fixture::new();
    let catalog = fixture.launch();
    ask(&catalog, "a", &["EUR_USD"]);
    std::thread::scope(|scope| {
        let _ = scope
            .spawn(|| {
                let _held = (
                    catalog.memory.lock().unwrap(),
                    catalog.refresh.lock().unwrap(),
                );
                panic!("poisons both locks");
            })
            .join();
    });
    assert!(catalog.memory.is_poisoned() && catalog.refresh.is_poisoned());

    // What is held is served, another owner's list is downloaded, and memory can be cleared.
    assert_eq!(*ask(&catalog, "a", &["asked"]), pairs(&["EUR_USD"]));
    assert_eq!(*ask(&catalog, "b", &["USD_JPY"]), pairs(&["USD_JPY"]));
    catalog.clear_memory();
    assert!(catalog.held("b").is_none());
}

#[test]
fn a_download_that_panics_counts_as_a_failed_attempt() {
    let fixture = Fixture::new();
    let catalog = fixture.launch();
    catalog
        .get_for(EVERYONE, fixture.download(Ok(pairs(&["OLD"]))))
        .unwrap();
    fixture.pass(Duration::from_secs(25 * 3_600));

    let catalog = fixture.launch();
    let panicked = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let _ = catalog.get_for(EVERYONE, || -> Result<Vec<Pair>, _> {
            fixture.downloads.fetch_add(1, Ordering::SeqCst);
            panic!("the provider's answer was not what the parser expected");
        });
    }));
    assert!(panicked.is_err(), "the panic is not swallowed");

    // Not asked again at every search, and what is held is still served.
    for _ in 0..4 {
        let items = catalog
            .get_for(
                EVERYONE,
                fixture.download(Err(ProviderError::refused(
                    crate::model::DataSource::Bybit,
                    "must not be asked",
                ))),
            )
            .unwrap();
        assert_eq!(*items, pairs(&["OLD"]));
    }
    assert_eq!(fixture.downloads(), 1 + 1, "the first list, then the panic");
}
