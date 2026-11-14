//! Candles held in memory by `(source, symbol, interval)`, in front of the slow providers.
//!
//! A refresh is not a download: a held series is brought up to date by asking for its last few
//! candles only (`TAIL_LIMIT`), which turns Coinbase's weekly fold (a dozen paged requests) and
//! GeckoTerminal's rate-limited pool lookups into one call. Concurrent requests for the same series
//! share one network call (single flight), so two charts of one pool, or a chart and a Pine
//! `request.security`, do not spend the limiter twice.

pub mod control;
pub mod disk;
pub mod lookup;

use crate::model::{DataSource, Kline};
use crate::service::ProviderError;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Candles a refresh asks for when it only needs the end of a held series. Four is at least two
/// whole buckets of a folded interval, so the oldest (cut by the request) can be dropped.
pub const TAIL_LIMIT: u32 = 4;
/// Series kept at once; the least recently fetched goes first.
const MAX_ENTRIES: usize = 64;
/// A series older than this is fetched whole again rather than patched.
const MAX_PATCH_AGE: Duration = Duration::from_secs(600);

/// How one source's candles are cached.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Policy {
    /// Candles younger than this are served without a request.
    pub fresh: Duration,
}

/// The sources whose candles are worth holding, and for how long they are served as they are.
/// Binance answers in one cheap request and has a live feed, and the line charts are small.
pub fn policy(source: DataSource) -> Option<Policy> {
    let fresh = match source {
        DataSource::CoinGecko => 120,
        DataSource::DexScreener => 30,
        DataSource::Coinbase | DataSource::Alpaca => 0,
        _ => return None,
    };
    Some(Policy {
        fresh: Duration::from_secs(fresh),
    })
}

struct Entry {
    fetched: Instant,
    /// The `limit` the series was fetched for: a request for more cannot be answered from it.
    requested: u32,
    candles: Vec<Kline>,
}

#[derive(Default)]
struct Held {
    entries: HashMap<String, Entry>,
    flights: HashMap<String, Arc<Mutex<()>>>,
}

fn held() -> &'static Mutex<Held> {
    static HELD: OnceLock<Mutex<Held>> = OnceLock::new();
    HELD.get_or_init(Mutex::default)
}

fn key(source: DataSource, symbol: &str, interval: &str) -> String {
    format!("{source:?}:{}:{interval}", symbol.to_lowercase())
}

/// The newest `limit` candles of `candles`.
fn newest(candles: &[Kline], limit: u32) -> Vec<Kline> {
    let start = candles.len().saturating_sub(limit as usize);
    candles[start..].to_vec()
}

/// `candles` brought up to date with `tail`, a fetch of its end. `None` when the tail does not
/// reach back to the series (the gap between them is unknown), so it must be fetched whole.
fn patched(candles: &[Kline], tail: &[Kline]) -> Option<Vec<Kline>> {
    let (last, first) = (candles.last()?, tail.first()?);
    // The tail must start inside the series and not end before it does.
    if first.open_time > last.open_time || tail.last()?.open_time < last.open_time {
        return None;
    }
    let keep = candles.partition_point(|candle| candle.open_time < first.open_time);
    let mut merged = candles[..keep].to_vec();
    merged.extend_from_slice(tail);
    Some(merged)
}

/// Candles for `source`/`symbol`/`interval` through `fetch`, which asks the provider for a `limit`.
///
/// Fresh candles are served as they are; older ones are patched with `fetch(TAIL_LIMIT)`; anything
/// else is fetched whole. Calls for one series wait for each other, and the later ones find what
/// the first brought.
pub fn fetch(
    source: DataSource,
    symbol: &str,
    interval: &str,
    limit: u32,
    policy: Policy,
    fetch: impl Fn(u32) -> Result<Vec<Kline>, ProviderError>,
) -> Result<Vec<Kline>, ProviderError> {
    let key = key(source, symbol, interval);
    let flight = {
        let mut held = held()
            .lock()
            .map_err(|_| ProviderError::other("The candle cache stopped"))?;
        // Series nobody waits on are forgotten, so the table follows the entries.
        if held.flights.len() > MAX_ENTRIES * 4 {
            held.flights
                .retain(|_, flight| Arc::strong_count(flight) > 1);
        }
        held.flights.entry(key.clone()).or_default().clone()
    };
    let _turn = flight
        .lock()
        .map_err(|_| ProviderError::other("The candle cache stopped"))?;

    let current = {
        let held = held()
            .lock()
            .map_err(|_| ProviderError::other("The candle cache stopped"))?;
        held.entries
            .get(&key)
            .filter(|entry| entry.requested >= limit)
            .map(|entry| {
                (
                    entry.fetched.elapsed(),
                    entry.requested,
                    entry.candles.clone(),
                )
            })
    };
    if let Some((age, requested, candles)) = current {
        if age < policy.fresh {
            return Ok(newest(&candles, limit));
        }
        if age < MAX_PATCH_AGE
            && let Some(merged) = patched(&candles, &fetch(TAIL_LIMIT)?)
        {
            return Ok(store(&key, requested, merged, limit));
        }
    }
    let candles = fetch(limit)?;
    Ok(store(&key, limit, candles, limit))
}

/// Keeps `candles` and returns the newest `limit` of them.
fn store(key: &str, requested: u32, mut candles: Vec<Kline>, limit: u32) -> Vec<Kline> {
    // A series stays as long as it was asked for, however many buckets arrived since.
    if candles.len() > requested as usize {
        candles.drain(..candles.len() - requested as usize);
    }
    let shown = newest(&candles, limit);
    if let Ok(mut held) = held().lock() {
        if held.entries.len() >= MAX_ENTRIES
            && !held.entries.contains_key(key)
            && let Some(oldest) = held
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.fetched)
                .map(|(key, _)| key.clone())
        {
            held.entries.remove(&oldest);
        }
        held.entries.insert(
            key.to_owned(),
            Entry {
                fetched: Instant::now(),
                requested,
                candles,
            },
        );
    }
    shown
}

/// Forgets every series (the user cleared the cache).
pub fn clear() {
    forget_where(|_| true);
}

fn forget_where(forget: impl Fn(&str) -> bool) {
    if let Ok(mut held) = held().lock() {
        held.entries.retain(|key, _| !forget(key));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::net::http;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn candle(open_time: i64, close: f64) -> Kline {
        Kline {
            open_time,
            close_time: open_time + 9,
            open: close,
            high: close,
            low: close,
            close,
            volume: 1.0,
        }
    }

    fn series(times: std::ops::Range<i64>) -> Vec<Kline> {
        times.map(|time| candle(time * 10, time as f64)).collect()
    }

    const STALE: Policy = Policy {
        fresh: Duration::ZERO,
    };
    const LONG: Policy = Policy {
        fresh: Duration::from_secs(3_600),
    };

    #[test]
    fn a_tail_that_reaches_the_series_replaces_its_end() {
        let held = series(0..10);
        let mut tail = series(8..12);
        tail[0].close = 99.0;
        let merged = patched(&held, &tail).expect("overlaps");
        let times: Vec<i64> = merged.iter().map(|c| c.open_time / 10).collect();
        assert_eq!(times, (0..12).collect::<Vec<_>>());
        assert_eq!(merged[8].close, 99.0, "the tail's copy of a candle wins");
        // One candle past the end still joins.
        assert!(patched(&held, &series(10..12)).is_none());
        assert!(patched(&held, &series(9..12)).is_some());
        assert!(
            patched(&held, &series(2..5)).is_none(),
            "a tail older than the series"
        );
        assert!(patched(&[], &tail).is_none());
        assert!(patched(&held, &[]).is_none());
    }

    #[test]
    fn fresh_candles_are_not_fetched_again_and_stale_ones_are_patched() {
        let calls = AtomicUsize::new(0);
        let asked = Mutex::new(Vec::new());
        let provider = |limit: u32| {
            calls.fetch_add(1, Ordering::SeqCst);
            asked.lock().unwrap().push(limit);
            Ok(if limit == TAIL_LIMIT {
                series(8..12)
            } else {
                series(0..10)
            })
        };
        let source = DataSource::Coinbase;
        let first = fetch(source, "patch-test", "1h", 10, LONG, provider).unwrap();
        assert_eq!(first.len(), 10);
        // Served from memory.
        fetch(source, "patch-test", "1h", 10, LONG, provider).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // No longer fresh: only the tail is asked for, and the series keeps its length.
        let patched = fetch(source, "patch-test", "1h", 10, STALE, provider).unwrap();
        assert_eq!(*asked.lock().unwrap(), [10, TAIL_LIMIT]);
        assert_eq!(patched.len(), 10);
        assert_eq!(patched.last().unwrap().open_time, 110);
        // More than was held needs a whole fetch.
        fetch(source, "patch-test", "1h", 50, LONG, provider).unwrap();
        assert_eq!(*asked.lock().unwrap(), [10, TAIL_LIMIT, 50]);
    }

    #[test]
    fn a_tail_that_leaves_a_gap_is_followed_by_a_whole_fetch() {
        let asked = Mutex::new(Vec::new());
        let provider = |limit: u32| {
            asked.lock().unwrap().push(limit);
            Ok(if limit == TAIL_LIMIT {
                series(50..54)
            } else {
                series(0..10)
            })
        };
        fetch(DataSource::Alpaca, "gap-test", "1d", 10, STALE, provider).unwrap();
        fetch(DataSource::Alpaca, "gap-test", "1d", 10, STALE, provider).unwrap();
        assert_eq!(*asked.lock().unwrap(), [10, TAIL_LIMIT, 10]);
    }

    #[test]
    fn a_failed_refresh_is_an_error_not_a_stale_answer() {
        let ok = |_: u32| Ok(series(0..10));
        fetch(DataSource::Coinbase, "error-test", "1h", 10, STALE, ok).unwrap();
        let broken = |_: u32| Err(ProviderError::from(http::Error::Network("down".to_owned())));
        assert!(fetch(DataSource::Coinbase, "error-test", "1h", 10, STALE, broken).is_err());
    }

    #[test]
    fn concurrent_requests_for_one_series_make_one_call() {
        let calls = Arc::new(AtomicUsize::new(0));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let calls = calls.clone();
                std::thread::spawn(move || {
                    fetch(
                        DataSource::DexScreener,
                        "flight-test",
                        "1h",
                        10,
                        LONG,
                        |_| {
                            calls.fetch_add(1, Ordering::SeqCst);
                            std::thread::sleep(Duration::from_millis(60));
                            Ok(series(0..10))
                        },
                    )
                    .unwrap()
                    .len()
                })
            })
            .collect();
        for handle in handles {
            assert_eq!(handle.join().unwrap(), 10);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn clearing_forgets_every_series() {
        let calls = AtomicUsize::new(0);
        let provider = |_: u32| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(series(0..10))
        };
        fetch(
            DataSource::CoinGecko,
            "clear-test",
            "1d",
            10,
            LONG,
            provider,
        )
        .unwrap();
        // Not `clear()`: the other tests share the table and run alongside this one.
        forget_where(|key| key.contains("clear-test"));
        fetch(
            DataSource::CoinGecko,
            "clear-test",
            "1d",
            10,
            LONG,
            provider,
        )
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn only_the_slow_providers_are_cached() {
        assert!(policy(DataSource::Binance).is_none());
        assert!(policy(DataSource::Polymarket).is_none());
        assert!(policy(DataSource::DexScreener).is_some());
    }
}
