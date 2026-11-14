//! Candle fetching behind the Top Performers card: how each holding's market moved since a
//! starting time. Holdings only carry today's and yesterday's price, so a longer range needs the
//! market's own candles. The ranking is `model::portfolio::card::performance`; this is the
//! blocking provider boundary, a small cache, and the worker bookkeeping around them.
//!
//! Fetches run on `service::work_queue` threads and land in the cache, which the UI reads each
//! frame; a result never arrives over a channel the card has to drain.

use crate::lock::lock;
use crate::model::Kline;
use crate::model::portfolio::PortfolioAsset;
use crate::model::portfolio::card::AssetMove;
use crate::model::portfolio::history::DAY_MS;
use crate::service::portfolio::history::{daily_bars, fetch_all, market_symbol};
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// A move this old is shown while it is fetched again.
const FRESH_MS: i64 = 5 * 60 * 1_000;
/// A failed fetch waits this long before the next attempt.
const RETRY_MS: i64 = 60 * 1_000;
/// Up to this long a range is compared on hourly candles, beyond it on daily ones.
const HOURLY_SPAN_MS: i64 = 36 * 60 * 60 * 1_000;
const HOUR_MS: i64 = 60 * 60 * 1_000;

/// One asset and the time to compare it from.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveRequest {
    pub asset: PortfolioAsset,
    /// Epoch milliseconds: the start of the range, or when the asset was first bought if later.
    pub since_ms: i64,
}

/// What the feed knows right now.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Moves {
    pub moves: HashMap<String, AssetMove>,
    /// Some asset has no answer yet (still being fetched).
    pub loading: bool,
}

type Fetcher = dyn Fn(&PortfolioAsset, i64, i64) -> Option<AssetMove> + Send + Sync;

/// An asset and the bucketed start time its answer was computed for.
type Key = (String, i64);

struct Entry {
    fetched_ms: i64,
    movement: Option<AssetMove>,
}

#[derive(Default)]
struct State {
    cache: HashMap<Key, Entry>,
    in_flight: HashSet<Key>,
}

/// Cheap shared handle; every Top Performers card reads through the same cache, so two cards on
/// one portfolio fetch once.
#[derive(Clone)]
pub struct PerformerFeed {
    state: Arc<Mutex<State>>,
    fetcher: Arc<Fetcher>,
}

impl Default for PerformerFeed {
    fn default() -> Self {
        Self::with_fetcher(Arc::new(candle_move))
    }
}

impl PerformerFeed {
    fn with_fetcher(fetcher: Arc<Fetcher>) -> Self {
        Self {
            state: Arc::default(),
            fetcher,
        }
    }

    /// The moves known for `requests`, starting a fetch for any that are missing or stale. Call
    /// every frame the card shows; it repaints itself when a fetch lands.
    pub fn moves(&self, ctx: &egui::Context, requests: &[MoveRequest], now_ms: i64) -> Moves {
        let mut result = Moves::default();
        let mut wanted: Vec<(Key, MoveRequest)> = Vec::new();
        {
            let mut state = lock(&self.state);
            for request in requests {
                let key = key_for(request, now_ms);
                match state.cache.get(&key) {
                    Some(entry) => {
                        if let Some(movement) = entry.movement {
                            result.moves.insert(request.asset.key.clone(), movement);
                        }
                        let ttl = if entry.movement.is_some() {
                            FRESH_MS
                        } else {
                            RETRY_MS
                        };
                        if now_ms - entry.fetched_ms >= ttl && !state.in_flight.contains(&key) {
                            wanted.push((key, request.clone()));
                        }
                    }
                    None => {
                        result.loading |= true;
                        if !state.in_flight.contains(&key) {
                            wanted.push((key, request.clone()));
                        }
                    }
                }
            }
            for (key, _) in &wanted {
                state.in_flight.insert(key.clone());
            }
        }
        if !wanted.is_empty() {
            let shared = Arc::clone(&self.state);
            let fetcher = Arc::clone(&self.fetcher);
            let ctx = ctx.clone();
            crate::service::work_queue::submit(move || {
                refresh(&shared, fetcher.as_ref(), &wanted, now_ms);
                ctx.request_repaint();
            });
        }
        result
    }
}

fn refresh(shared: &Mutex<State>, fetcher: &Fetcher, wanted: &[(Key, MoveRequest)], now_ms: i64) {
    let assets: Vec<PortfolioAsset> = wanted
        .iter()
        .map(|(_, request)| request.asset.clone())
        .collect();
    let since: HashMap<&str, i64> = wanted
        .iter()
        .map(|(_, request)| (request.asset.key.as_str(), request.since_ms))
        .collect();
    let fetched = fetch_all(&assets, |asset| {
        let since_ms = since.get(asset.key.as_str()).copied()?;
        fetcher(asset, since_ms, now_ms)
    });
    let mut state = lock(shared);
    for (key, request) in wanted {
        state.in_flight.remove(key);
        state.cache.insert(
            key.clone(),
            Entry {
                fetched_ms: now_ms,
                movement: fetched.get(&request.asset.key).copied(),
            },
        );
    }
}

/// An hour-long range changes by the half hour, a longer one by the day, so a card doesn't ask
/// for new candles every frame while its start time creeps forward.
fn key_for(request: &MoveRequest, now_ms: i64) -> Key {
    let bucket = if now_ms - request.since_ms <= HOURLY_SPAN_MS {
        HOUR_MS / 2
    } else {
        DAY_MS
    };
    (
        request.asset.key.clone(),
        request.since_ms.div_euclid(bucket) * bucket,
    )
}

/// The provider fetch: hourly candles for a day or so, daily ones for anything longer.
fn candle_move(asset: &PortfolioAsset, since_ms: i64, now_ms: i64) -> Option<AssetMove> {
    let bars = if now_ms - since_ms <= HOURLY_SPAN_MS {
        let hours = ((now_ms - since_ms).max(0) / HOUR_MS) as u32 + 4;
        let mut bars =
            crate::service::fetch_klines(asset.source, &market_symbol(asset), "1h", hours)
                .unwrap_or_default();
        bars.sort_by_key(|bar| bar.open_time);
        bars
    } else {
        daily_bars(asset, since_ms, now_ms)
    };
    movement(&bars, since_ms)
}

/// Where the market stood at `since_ms` (the close of the last candle that ended by then, else
/// the open of the first one) and where it stands now. `bars` ascend by time.
pub fn movement(bars: &[Kline], since_ms: i64) -> Option<AssetMove> {
    let latest = bars.last()?.close;
    let start = bars
        .iter()
        .rev()
        .find(|bar| bar.close_time <= since_ms)
        .map(|bar| bar.close)
        .or_else(|| bars.first().map(|bar| bar.open))?;
    (start > 0.0 && start.is_finite() && latest.is_finite()).then_some(AssetMove { start, latest })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::PortfolioCurrency;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    fn asset(symbol: &str) -> PortfolioAsset {
        PortfolioAsset {
            key: format!("binance:{symbol}USDT"),
            symbol: format!("{symbol}/USDT"),
            name: symbol.to_owned(),
            source: DataSource::Binance,
            quote_currency: PortfolioCurrency::Usd,
            metadata: HashMap::new(),
        }
    }

    fn bar(open_time: i64, close_time: i64, open: f64, close: f64) -> Kline {
        Kline {
            open_time,
            close_time,
            open,
            high: open.max(close),
            low: open.min(close),
            close,
            volume: 1.0,
        }
    }

    fn request(symbol: &str, since_ms: i64) -> MoveRequest {
        MoveRequest {
            asset: asset(symbol),
            since_ms,
        }
    }

    /// Calls until nothing is loading, so a test sees the worker's answer.
    fn settle(feed: &PerformerFeed, requests: &[MoveRequest], now_ms: i64) -> Moves {
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let moves = feed.moves(&ctx, requests, now_ms);
            if !moves.loading || Instant::now() > deadline {
                return moves;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn the_start_is_the_close_of_the_last_candle_that_ended_by_then() {
        let bars = [
            bar(0, 99, 10.0, 11.0),
            bar(100, 199, 11.0, 12.0),
            bar(200, 299, 12.0, 15.0),
        ];
        let result = movement(&bars, 200).unwrap();
        assert_eq!((result.start, result.latest), (12.0, 15.0));
    }

    #[test]
    fn an_asset_younger_than_the_range_starts_at_its_first_open() {
        let bars = [bar(100, 199, 11.0, 12.0), bar(200, 299, 12.0, 15.0)];
        assert_eq!(movement(&bars, 0).unwrap().start, 11.0);
        assert_eq!(movement(&[], 0), None);
        assert_eq!(movement(&[bar(0, 99, 0.0, 1.0)], 0), None);
    }

    #[test]
    fn a_range_is_bucketed_so_it_does_not_refetch_as_time_creeps() {
        let now = 40 * DAY_MS;
        let a = key_for(&request("BTC", now - 7 * DAY_MS + 1_000), now);
        let b = key_for(&request("BTC", now - 7 * DAY_MS + 60_000), now);
        assert_eq!(a, b);
        let day = key_for(&request("BTC", now - DAY_MS + 1_000), now);
        let later = key_for(&request("BTC", now - DAY_MS + 2 * 60_000), now);
        assert_eq!(day, later);
        assert_ne!(
            key_for(&request("BTC", now - 7 * DAY_MS), now).1,
            key_for(&request("BTC", now - 30 * DAY_MS), now).1
        );
    }

    #[test]
    fn answers_are_cached_per_asset_and_start() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let feed = PerformerFeed::with_fetcher(Arc::new(move |_, _, _| {
            counter.fetch_add(1, Ordering::SeqCst);
            Some(AssetMove {
                start: 1.0,
                latest: 2.0,
            })
        }));
        let now = 100 * DAY_MS;
        let requests = [
            request("BTC", now - 7 * DAY_MS),
            request("ETH", now - 7 * DAY_MS),
        ];
        let first = settle(&feed, &requests, now);
        assert_eq!(first.moves.len(), 2);
        let again = settle(&feed, &requests, now + 1_000);
        assert_eq!(again.moves.len(), 2);
        assert_eq!(calls.load(Ordering::SeqCst), 2, "fresh answers are reused");
        settle(&feed, &[request("BTC", now - 30 * DAY_MS)], now);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            3,
            "another range is another answer"
        );
    }

    #[test]
    fn a_failed_fetch_is_not_an_answer_and_waits_before_retrying() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let feed = PerformerFeed::with_fetcher(Arc::new(move |_, _, _| {
            counter.fetch_add(1, Ordering::SeqCst);
            None
        }));
        let now = 100 * DAY_MS;
        let requests = [request("BTC", now - 7 * DAY_MS)];
        let first = settle(&feed, &requests, now);
        assert!(first.moves.is_empty() && !first.loading);
        settle(&feed, &requests, now + 1_000);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        settle(&feed, &requests, now + RETRY_MS + 1);
        // The retry runs on a worker; wait for it to land.
        let deadline = Instant::now() + Duration::from_secs(5);
        while calls.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
