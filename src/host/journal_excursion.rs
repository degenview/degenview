//! The MFE/MAE job for the Trade Journal: pick a candle interval for a trade, fetch the candles,
//! run the pure estimate, and cache what is deterministic.
//!
//! Runs on a worker (never while painting). The candle source is a trait so tests need no
//! network. What is cached is the answer for these fills at this interval: a figure, or "no
//! complete bar fits between the fills". A failed or empty fetch is never cached, so a later try
//! can succeed.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};

use serde::{Deserialize, Serialize};

use crate::model::DataSource;
use crate::model::Kline;
use crate::model::journal::excursion::{Excursion, excursion};
use crate::model::journal::metric::{Metric, Reason};
use crate::model::journal::model::Trade;
use crate::service::work_queue;
use crate::store::SharedStore;

/// Candle intervals tried from the finest, as the providers name them.
const INTERVALS: [(&str, i64); 7] = [
    ("1m", 60_000),
    ("5m", 300_000),
    ("15m", 900_000),
    ("30m", 1_800_000),
    ("1h", 3_600_000),
    ("4h", 14_400_000),
    ("1d", 86_400_000),
];

/// Most candles one estimate asks for.
const MAX_BARS: i64 = 1_000;

pub trait CandleFetcher: Send + Sync {
    fn candles(
        &self,
        source: DataSource,
        symbol: &str,
        interval: &str,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, String>;
}

/// The providers, through the service layer.
#[cfg_attr(test, allow(dead_code))]
pub struct Providers;

impl CandleFetcher for Providers {
    fn candles(
        &self,
        source: DataSource,
        symbol: &str,
        interval: &str,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, String> {
        crate::service::fetch_klines_range(source, symbol, interval, start_ms, end_ms, max)
    }
}

/// A fetcher that never reaches the network, for tests of the tab.
#[cfg(test)]
pub struct Offline;

#[cfg(test)]
impl CandleFetcher for Offline {
    fn candles(
        &self,
        _: DataSource,
        _: &str,
        _: &str,
        _: i64,
        _: i64,
        _: usize,
    ) -> Result<Vec<Kline>, String> {
        Err("offline".into())
    }
}

/// The fetcher a tab uses.
pub fn default_fetcher() -> std::sync::Arc<dyn CandleFetcher> {
    #[cfg(test)]
    return std::sync::Arc::new(Offline);
    #[cfg(not(test))]
    std::sync::Arc::new(Providers)
}

/// What is kept: only answers that do not depend on the network's mood.
#[derive(Debug, Deserialize, PartialEq, Serialize)]
enum Cached {
    Value(Excursion),
    NoCompleteBar,
}

/// The finest interval whose candles over the hold fit in one request.
pub fn choose_interval(hold_ms: i64) -> Option<(&'static str, i64)> {
    INTERVALS
        .into_iter()
        .find(|(_, ms)| hold_ms / ms <= MAX_BARS)
}

/// Identifies the inputs of an estimate: the fills and the interval.
pub fn fingerprint(trade: &Trade, interval: &str) -> String {
    let mut text = format!("v1|{interval}|{}", trade.multiplier);
    for leg in &trade.legs {
        text.push_str(&format!("|{}:{}:{}", leg.ts_ms, leg.quantity, leg.price));
    }
    text
}

fn session_key(trade: &Trade) -> String {
    format!("{}|{}", trade.key, fingerprint(trade, ""))
}

/// Estimates the excursion of a closed trade. `Err` is a message about why no answer could be
/// had (no provider, the fetch failed); `Ok(Unavailable(..))` is an answer.
pub fn estimate(
    trade: &Trade,
    store: &SharedStore,
    fetcher: &dyn CandleFetcher,
) -> Result<Metric<Excursion>, String> {
    let Some(close) = trade.close_ts_ms.filter(|_| !trade.is_open()) else {
        return Ok(Metric::Unavailable(Reason::OpenTrade));
    };
    let hold = close - trade.open_ts_ms;
    let Some((interval, interval_ms)) = choose_interval(hold) else {
        return Err("The trade was held too long to estimate from candles.".into());
    };
    let fingerprint = fingerprint(trade, interval);
    if let Some(payload) = store
        .lock()
        .ok()
        .and_then(|store| store.journal_stat(&trade.key, &fingerprint).ok())
        .flatten()
        && let Ok(cached) = serde_json::from_str::<Cached>(&payload)
    {
        return Ok(match cached {
            Cached::Value(value) => Metric::Value(value),
            Cached::NoCompleteBar => Metric::Unavailable(Reason::NoCompleteBar),
        });
    }
    let source = trade
        .instrument
        .provider
        .as_deref()
        .and_then(DataSource::from_display_name)
        .ok_or_else(|| "This trade's market has no candle provider.".to_owned())?;
    let candles = fetcher
        .candles(
            source,
            &trade.instrument.symbol,
            interval,
            trade.open_ts_ms - interval_ms,
            close + interval_ms,
            (MAX_BARS + 4) as usize,
        )
        .map_err(|error| format!("Candles could not be fetched: {error}"))?;
    let answer = excursion(trade, &candles, interval, interval_ms);
    let cache = match &answer {
        Metric::Value(value) => Some(Cached::Value(value.clone())),
        Metric::Unavailable(Reason::NoCompleteBar) if !candles.is_empty() => {
            Some(Cached::NoCompleteBar)
        }
        _ => None,
    };
    if let Some(cache) = cache
        && let Ok(payload) = serde_json::to_string(&cache)
        && let Ok(store) = store.lock()
    {
        let _ = store.journal_save_stat(&trade.key, &trade.account_uid, &fingerprint, &payload);
    }
    Ok(answer)
}

/// How a trade's estimate stands.
#[derive(Clone, Debug)]
pub enum State {
    Loading,
    Done(Result<Metric<Excursion>, String>),
}

/// Runs estimates one trade at a time on a worker and keeps the answers for the session.
pub struct Excursions {
    /// Keyed by the trade's fills, not just its key, so a trade whose executions changed is
    /// estimated again.
    states: HashMap<String, State>,
    sender: Sender<(String, Result<Metric<Excursion>, String>)>,
    receiver: Receiver<(String, Result<Metric<Excursion>, String>)>,
}

impl Default for Excursions {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            states: HashMap::new(),
            sender,
            receiver,
        }
    }
}

impl Excursions {
    /// Picks up finished answers.
    pub fn poll(&mut self) {
        while let Ok((key, answer)) = self.receiver.try_recv() {
            self.states.insert(key, State::Done(answer));
        }
    }

    /// Puts an answer in directly, for pictures of the panel.
    #[cfg(test)]
    pub fn put(&mut self, trade: &Trade, answer: Result<Metric<Excursion>, String>) {
        self.states.insert(session_key(trade), State::Done(answer));
    }

    pub fn state(&self, trade: &Trade) -> Option<&State> {
        self.states.get(&session_key(trade))
    }

    /// Starts the estimate for `trade` unless one is running or done. `retry` forgets a failure.
    pub fn request(
        &mut self,
        trade: &Trade,
        store: &SharedStore,
        fetcher: std::sync::Arc<dyn CandleFetcher>,
        repaint: impl Fn() + Send + 'static,
        retry: bool,
    ) {
        let key = session_key(trade);
        match self.states.get(&key) {
            Some(State::Loading) => return,
            Some(State::Done(Err(_))) if retry => {}
            Some(_) => return,
            None => {}
        }
        self.states.insert(key.clone(), State::Loading);
        let (trade, store, sender) = (trade.clone(), store.clone(), self.sender.clone());
        work_queue::ASSETS.submit(move || {
            let answer = estimate(&trade, &store, fetcher.as_ref());
            let _ = sender.send((key, answer));
            repaint();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::journal::model::{Direction, Side, TradeLeg, TradeStatus};
    use crate::model::journal::testing::{d, instrument};
    use crate::store::Store;
    use rust_decimal::Decimal;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    const MIN: i64 = 60_000;

    struct Canned {
        candles: Vec<Kline>,
        calls: AtomicUsize,
        fail: bool,
    }

    impl CandleFetcher for Canned {
        fn candles(
            &self,
            _: DataSource,
            _: &str,
            _: &str,
            _: i64,
            _: i64,
            _: usize,
        ) -> Result<Vec<Kline>, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                Err("offline".into())
            } else {
                Ok(self.candles.clone())
            }
        }
    }

    fn bar(minute: i64, high: f64, low: f64) -> Kline {
        Kline {
            open_time: minute * MIN,
            close_time: minute * MIN + MIN - 1,
            open: low,
            high,
            low,
            close: high,
            volume: 1.0,
        }
    }

    fn leg(ts: i64, side: Side, price: &str, entry: bool, after: &str) -> TradeLeg {
        TradeLeg {
            exec_uid: format!("e{ts}"),
            ts_ms: ts,
            side,
            quantity: d("1"),
            price: d(price),
            fee: Some(Decimal::ZERO),
            is_entry: entry,
            realized_gross: Decimal::ZERO,
            position_after: d(after),
        }
    }

    fn trade() -> Trade {
        let mut instrument = instrument("BTCUSDT");
        instrument.provider = Some("Binance".into());
        Trade {
            key: "t1".into(),
            account_uid: "a".into(),
            instrument,
            direction: Direction::Long,
            status: TradeStatus::Closed,
            open_ts_ms: 0,
            close_ts_ms: Some(10 * MIN),
            legs: vec![
                leg(0, Side::Buy, "100", true, "1"),
                leg(10 * MIN, Side::Sell, "104", false, "0"),
            ],
            entry_quantity: d("1"),
            exit_quantity: d("1"),
            open_quantity: Decimal::ZERO,
            max_quantity: d("1"),
            avg_entry: d("100"),
            avg_exit: Some(d("104")),
            multiplier: Decimal::ONE,
            gross_pnl: d("4"),
            fees: Decimal::ZERO,
            fees_known: true,
            open_fees: Decimal::ZERO,
            currency: Some("USD".into()),
            initial_stop: None,
        }
    }

    fn store() -> SharedStore {
        Arc::new(Mutex::new(Store::open_in_memory().unwrap()))
    }

    #[test]
    fn the_finest_interval_that_fits_one_request_is_chosen() {
        assert_eq!(choose_interval(10 * MIN), Some(("1m", 60_000)));
        assert_eq!(choose_interval(1_000 * MIN), Some(("1m", 60_000)));
        assert_eq!(choose_interval(1_001 * MIN), Some(("5m", 300_000)));
        assert_eq!(choose_interval(3 * 86_400_000), Some(("5m", 300_000)));
        assert_eq!(choose_interval(2_000 * 86_400_000), None);
    }

    #[test]
    fn an_estimate_is_computed_once_and_then_read_from_the_cache() {
        let store = store();
        let fetcher = Canned {
            candles: vec![bar(1, 103.0, 98.0), bar(2, 108.0, 102.0)],
            calls: AtomicUsize::new(0),
            fail: false,
        };
        let first = estimate(&trade(), &store, &fetcher).unwrap();
        let Metric::Value(value) = &first else {
            panic!("available");
        };
        assert_eq!(value.mfe, d("8"));
        let second = estimate(&trade(), &store, &fetcher).unwrap();
        assert_eq!(first, second);
        assert_eq!(fetcher.calls.load(Ordering::SeqCst), 1, "cached");
        // Different fills are a different question.
        let mut changed = trade();
        changed.legs[1].price = d("105");
        estimate(&changed, &store, &fetcher).unwrap();
        assert_eq!(fetcher.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_failed_or_empty_fetch_is_not_cached() {
        let store = store();
        let down = Canned {
            candles: vec![],
            calls: AtomicUsize::new(0),
            fail: true,
        };
        assert!(
            estimate(&trade(), &store, &down)
                .unwrap_err()
                .contains("offline")
        );
        let empty = Canned {
            candles: vec![],
            calls: AtomicUsize::new(0),
            fail: false,
        };
        assert_eq!(
            estimate(&trade(), &store, &empty).unwrap(),
            Metric::Unavailable(Reason::NoCandles)
        );
        let up = Canned {
            candles: vec![bar(1, 103.0, 98.0)],
            calls: AtomicUsize::new(0),
            fail: false,
        };
        assert!(estimate(&trade(), &store, &up).unwrap().is_available());
        assert_eq!(empty.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_market_without_a_provider_and_an_open_trade_say_so() {
        let store = store();
        let fetcher = Canned {
            candles: vec![],
            calls: AtomicUsize::new(0),
            fail: false,
        };
        let mut imported = trade();
        imported.instrument.provider = None;
        assert!(
            estimate(&imported, &store, &fetcher)
                .unwrap_err()
                .contains("provider")
        );
        let mut open = trade();
        open.status = TradeStatus::Open;
        open.close_ts_ms = None;
        assert_eq!(
            estimate(&open, &store, &fetcher).unwrap(),
            Metric::Unavailable(Reason::OpenTrade)
        );
        assert_eq!(fetcher.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn the_worker_delivers_the_answer_and_a_failure_can_be_retried() {
        let store = store();
        let mut jobs = Excursions::default();
        let down: Arc<dyn CandleFetcher> = Arc::new(Canned {
            candles: vec![],
            calls: AtomicUsize::new(0),
            fail: true,
        });
        jobs.request(&trade(), &store, down.clone(), || {}, false);
        assert!(matches!(jobs.state(&trade()), Some(State::Loading)));
        let wait = |jobs: &mut Excursions| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while matches!(jobs.state(&trade()), Some(State::Loading)) {
                jobs.poll();
                assert!(std::time::Instant::now() < deadline, "no answer");
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        };
        wait(&mut jobs);
        assert!(matches!(jobs.state(&trade()), Some(State::Done(Err(_)))));
        // Asking again without retry does nothing; with retry it runs.
        jobs.request(&trade(), &store, down, || {}, false);
        assert!(matches!(jobs.state(&trade()), Some(State::Done(Err(_)))));
        let up: Arc<dyn CandleFetcher> = Arc::new(Canned {
            candles: vec![bar(1, 103.0, 98.0)],
            calls: AtomicUsize::new(0),
            fail: false,
        });
        jobs.request(&trade(), &store, up, || {}, true);
        wait(&mut jobs);
        assert!(matches!(
            jobs.state(&trade()),
            Some(State::Done(Ok(Metric::Value(_))))
        ));
    }
}
