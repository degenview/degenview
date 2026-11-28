use super::*;
use crate::service::market::ports::cache::CacheStore;
use std::time::Duration;

#[test]
fn the_venues_are_built_once_and_shared() {
    assert!(std::ptr::eq(venues(), venues()));
}

#[test]
fn the_apps_lookup_cache_keeps_what_a_venue_puts_in_it() {
    let cache: Arc<dyn CacheStore> = context().cache;
    assert!(
        cache
            .get::<Vec<String>>("wiring_test", "list", Duration::from_secs(60))
            .is_none()
    );
    cache.put("wiring_test", "list", &vec!["BTCUSDT".to_owned()]);
    let hit = cache
        .get::<Vec<String>>("wiring_test", "list", Duration::from_secs(60))
        .expect("what was put");
    assert_eq!(hit.value, ["BTCUSDT"]);
    assert!(hit.fresh);
    // Another shape is no answer.
    assert!(
        cache
            .get::<u32>("wiring_test", "list", Duration::from_secs(60))
            .is_none()
    );
}
