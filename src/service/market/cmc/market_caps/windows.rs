//! The readings of each window, kept on disk (`cache::lookup`, so Settings ▸ Storage clears them).
//!
//! A window that is over is kept for good; the one still filling is asked again after a while; and
//! when a request fails the kept readings are used however old, so an outage or a "system is busy"
//! costs a chart nothing it already had.

use crate::model::market::cmc::global::GlobalPoint;
use crate::model::market::cmc::interval::{Interval, Window};
use crate::service::ProviderError;
use crate::service::cache::lookup::LookupCache;
use std::time::Duration;

/// The readings of every window, oldest first. `fetch` asks the network for one window.
pub(super) fn readings(
    cache: &LookupCache,
    interval: Interval,
    windows: &[Window],
    now_ms: i64,
    fetch: impl Fn(Interval, &Window) -> Result<Vec<GlobalPoint>, ProviderError>,
) -> Result<Vec<GlobalPoint>, ProviderError> {
    let mut points = Vec::new();
    for window in windows {
        points.extend(window_readings(cache, interval, window, now_ms, &fetch)?);
    }
    Ok(points)
}

fn window_readings(
    cache: &LookupCache,
    interval: Interval,
    window: &Window,
    now_ms: i64,
    fetch: &impl Fn(Interval, &Window) -> Result<Vec<GlobalPoint>, ProviderError>,
) -> Result<Vec<GlobalPoint>, ProviderError> {
    let namespace = namespace(interval);
    let key = window.index.to_string();
    let kept = cache.get::<Vec<GlobalPoint>>(namespace, &key, Duration::ZERO);
    if let Some(hit) = &kept
        && is_current(window, interval, hit.age, now_ms)
    {
        return Ok(hit.value.clone());
    }
    match fetch(interval, window) {
        Ok(points) => {
            cache.put(namespace, &key, &points);
            Ok(points)
        }
        Err(error) => kept.map(|hit| hit.value).ok_or(error),
    }
}

/// Whether readings fetched `age` ago still stand: they were fetched once the window had settled,
/// or (for the window still filling) not long ago.
fn is_current(window: &Window, interval: Interval, age: Duration, now_ms: i64) -> bool {
    let fetched_at = now_ms.saturating_sub(i64::try_from(age.as_millis()).unwrap_or(i64::MAX));
    window.is_settled(fetched_at) || age < open_window_ttl(interval)
}

/// How long the readings of the window still filling are used: a fraction of the spacing, so a
/// chart that refreshes every few seconds asks the network about once per new reading.
fn open_window_ttl(interval: Interval) -> Duration {
    match interval {
        Interval::Minutes5 => Duration::from_secs(60),
        Interval::Hour1 => Duration::from_secs(300),
        Interval::Day1 => Duration::from_secs(1_800),
    }
}

/// How far back the newest readings reach: more than the few a request can be late by.
const NEWEST_SPAN_MS: i64 = 30 * 60_000;

/// How long the newest readings are used before the network is asked again: one reading's time at
/// the finest spacing is five minutes, so a chart that refreshes every few seconds asks about once a
/// minute.
const NEWEST_TTL: Duration = Duration::from_secs(60);

const NEWEST_NAMESPACE: &str = "cmc_global_newest";

/// The readings of the last half hour at the finest spacing, kept for a minute under one key
/// (rewritten, never piling up). A failed request is no readings: they only add to a series that
/// stands without them, and an old copy would put a stale reading beside a newer one.
pub(super) fn newest(
    cache: &LookupCache,
    now_ms: i64,
    fetch: impl Fn(Interval, &Window) -> Result<Vec<GlobalPoint>, ProviderError>,
) -> Vec<GlobalPoint> {
    let kept = cache.get::<Vec<GlobalPoint>>(NEWEST_NAMESPACE, "5m", Duration::ZERO);
    if let Some(hit) = kept
        && hit.age < NEWEST_TTL
    {
        return hit.value;
    }
    let window = Window {
        index: 0,
        start_ms: now_ms - NEWEST_SPAN_MS,
        end_ms: now_ms,
    };
    match fetch(Interval::Minutes5, &window) {
        Ok(points) => {
            cache.put(NEWEST_NAMESPACE, "5m", &points);
            points
        }
        Err(_) => Vec::new(),
    }
}

/// One file per spacing, so keeping a window rewrites only the readings of its own spacing.
fn namespace(interval: Interval) -> &'static str {
    match interval {
        Interval::Minutes5 => "cmc_global_5m",
        Interval::Hour1 => "cmc_global_1h",
        Interval::Day1 => "cmc_global_1d",
    }
}
