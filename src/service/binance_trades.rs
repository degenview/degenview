//! Binance trades with the exchange's aggressor side: `aggTrades` history over REST and the live
//! `aggTrade` WebSocket stream.
//!
//! This is the only trade data in the app, and it is what makes a footprint exact (a candle gives
//! volume, never who hit whom at which price). It is deliberately bounded:
//!
//! - History is fetched **newest first** by id (ids are consecutive per market, so a page is a plain
//!   id range) and stops at the window's start, a trade budget, a cancel, or a rate-limit refusal,
//!   so the bars on screen fill first and a slow answer still leaves the recent part complete.
//! - Requests share one limiter (`rate_limit::binance_trades`), and a 429 or 418 backs off instead of
//!   retrying into a ban. Nothing here retries forever.
//! - The live stream reconnects like the kline feed and counts its reconnects; the owner backfills
//!   the ids it missed (the book detects the gap by id), so a reconnect never double counts or
//!   silently drops trades.
//!
//! [`TradeSource`] is the REST boundary: the real one is [`BinanceRest`], tests use a fake.

use crate::model::market_profile::trades::Trade;
use eframe::egui;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

const REST_URL: &str = "https://api.binance.com/api/v3/aggTrades";

/// The most trades one request returns.
pub const PAGE: usize = 1_000;

/// Trades the live inbox keeps between two reads; older ones are dropped and show up as an id gap
/// that the owner backfills.
const INBOX_CAPACITY: usize = 200_000;

/// Rate-limit refusals in a row before a history fetch gives up.
const MAX_REFUSALS: u32 = 3;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TradesError {
    /// 429: too many requests.
    RateLimited,
    /// 418: the address has been banned for ignoring 429.
    Banned,
    Network(String),
    Data(String),
}

impl std::fmt::Display for TradesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RateLimited => f.write_str("Binance is rate limiting trade requests"),
            Self::Banned => f.write_str("Binance has temporarily blocked trade requests"),
            Self::Network(message) => write!(f, "Network error: {message}"),
            Self::Data(message) => write!(f, "Data error: {message}"),
        }
    }
}

/// How trades are asked for. Implemented over HTTP by [`BinanceRest`].
pub trait TradeSource {
    /// The newest trade of a market (empty for a market that has none).
    fn latest(&self, symbol: &str) -> Result<Vec<Trade>, TradesError>;
    /// Up to `limit` (at most [`PAGE`]) trades with ids from `from_id` upward.
    fn page_from(
        &self,
        symbol: &str,
        from_id: u64,
        limit: usize,
    ) -> Result<Vec<Trade>, TradesError>;
}

pub struct BinanceRest;

impl TradeSource for BinanceRest {
    fn latest(&self, symbol: &str) -> Result<Vec<Trade>, TradesError> {
        get(&format!(
            "{REST_URL}?symbol={}&limit=1",
            symbol.to_uppercase()
        ))
    }

    fn page_from(
        &self,
        symbol: &str,
        from_id: u64,
        limit: usize,
    ) -> Result<Vec<Trade>, TradesError> {
        get(&format!(
            "{REST_URL}?symbol={}&fromId={from_id}&limit={}",
            symbol.to_uppercase(),
            limit.clamp(1, PAGE)
        ))
    }
}

fn get(url: &str) -> Result<Vec<Trade>, TradesError> {
    super::rate_limit::binance_trades().wait_for_slot();
    let body =
        super::http::get_text(url, &[], super::http::Retry::Never).map_err(
            |error| match error {
                super::http::Error::Status(429) => TradesError::RateLimited,
                super::http::Error::Status(418) => TradesError::Banned,
                other => TradesError::Network(other.to_string()),
            },
        )?;
    parse_rest(&body)
}

/// `[{"a":26129,"p":"0.01633102","q":"4.70443515","f":27781,"l":27781,"T":1498793709153,"m":true,"M":true}]`.
pub fn parse_rest(body: &str) -> Result<Vec<Trade>, TradesError> {
    let value: Value =
        serde_json::from_str(body).map_err(|error| TradesError::Data(error.to_string()))?;
    let rows = value
        .as_array()
        .ok_or_else(|| TradesError::Data("expected a trade array".to_owned()))?;
    rows.iter()
        .map(|row| parse_trade(row).ok_or_else(|| TradesError::Data("invalid trade".to_owned())))
        .collect()
}

fn parse_trade(row: &Value) -> Option<Trade> {
    Some(Trade {
        id: row.get("a")?.as_u64()?,
        time_ms: row.get("T")?.as_i64()?,
        price: row.get("p")?.as_str()?.parse().ok()?,
        quantity: row.get("q")?.as_str()?.parse().ok()?,
        buyer_is_maker: row.get("m")?.as_bool()?,
    })
}

/// `{"stream":"btcusdt@aggTrade","data":{"e":"aggTrade","s":"BTCUSDT","a":1,"p":"..","q":"..","T":..,"m":true}}`.
pub fn parse_stream(text: &str) -> Option<(String, Trade)> {
    let value: Value = serde_json::from_str(text).ok()?;
    let data = value.get("data")?;
    if data.get("e").and_then(Value::as_str) != Some("aggTrade") {
        return None;
    }
    let symbol = data.get("s")?.as_str()?.to_uppercase();
    Some((symbol, parse_trade(data)?))
}

// MARK: History

/// How a history fetch ended.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HistoryEnd {
    /// The window's start was reached.
    WindowStart,
    /// The market has no older trades.
    Exhausted,
    /// The trade budget was spent (the window's start was not reached).
    Budget,
    Cancelled,
    /// A request failed; what was fetched before it was still delivered.
    Failed(TradesError),
}

/// Fetches the trades of `symbol` from `window_start_ms` to now, **newest first**, handing each page
/// to `sink` (oldest trade of the page first). Stops at the window's start, after `budget` trades,
/// when `cancel` is set, or on an error; whatever was delivered stays valid, and because pages are
/// contiguous id ranges the delivered part is gap free.
///
/// `sleep` is how a refusal waits (injected so tests do not).
pub fn fetch_history(
    source: &dyn TradeSource,
    symbol: &str,
    window_start_ms: i64,
    budget: usize,
    cancel: &AtomicBool,
    sleep: &dyn Fn(Duration),
    sink: &mut dyn FnMut(Vec<Trade>),
) -> HistoryEnd {
    let ask =
        |action: &dyn Fn() -> Result<Vec<Trade>, TradesError>| -> Result<Vec<Trade>, TradesError> {
            let mut refusals = 0;
            loop {
                match action() {
                    Err(TradesError::RateLimited) if refusals < MAX_REFUSALS => {
                        refusals += 1;
                        sleep(Duration::from_secs(5 * (1 << refusals)));
                        if cancel.load(Ordering::Acquire) {
                            return Err(TradesError::RateLimited);
                        }
                    }
                    other => return other,
                }
            }
        };
    if cancel.load(Ordering::Acquire) {
        return HistoryEnd::Cancelled;
    }
    let newest = match ask(&|| source.latest(symbol)) {
        Ok(trades) => match trades.into_iter().max_by_key(|trade| trade.id) {
            Some(trade) => trade,
            None => return HistoryEnd::Exhausted,
        },
        Err(error) => return HistoryEnd::Failed(error),
    };
    // The newest trade is part of the answer; then walk back by whole id pages.
    let mut fetched = 1;
    let mut end = newest.id; // exclusive: the next page ends just below it
    let reached = newest.time_ms <= window_start_ms;
    sink(vec![newest]);
    if reached {
        return HistoryEnd::WindowStart;
    }
    while end > 0 {
        if cancel.load(Ordering::Acquire) {
            return HistoryEnd::Cancelled;
        }
        if fetched >= budget {
            return HistoryEnd::Budget;
        }
        let start = end.saturating_sub(PAGE as u64);
        let count = (end - start) as usize;
        let page = match ask(&|| source.page_from(symbol, start, count)) {
            Ok(page) => page,
            Err(error) => return HistoryEnd::Failed(error),
        };
        if page.is_empty() {
            return HistoryEnd::Exhausted;
        }
        // A short page is sound (the exchange's ids may skip nothing, but may start later).
        fetched += page.len();
        let oldest_time = page
            .iter()
            .map(|trade| trade.time_ms)
            .min()
            .unwrap_or(i64::MAX);
        let lowest_id = page.iter().map(|trade| trade.id).min().unwrap_or(start);
        sink(page);
        if oldest_time <= window_start_ms {
            return HistoryEnd::WindowStart;
        }
        end = start.min(lowest_id);
    }
    HistoryEnd::Exhausted
}

/// Fetches the trades from id `from_id` up to (not including) `to_id`, oldest first, in pages: the
/// ids a reconnect missed. Delivered through `sink`; `Failed` carries the error and what came
/// before it was delivered.
pub fn fetch_range(
    source: &dyn TradeSource,
    symbol: &str,
    from_id: u64,
    to_id: u64,
    cancel: &AtomicBool,
    sleep: &dyn Fn(Duration),
    sink: &mut dyn FnMut(Vec<Trade>),
) -> HistoryEnd {
    let mut next = from_id;
    let mut refusals = 0;
    while next < to_id {
        if cancel.load(Ordering::Acquire) {
            return HistoryEnd::Cancelled;
        }
        let count = ((to_id - next) as usize).min(PAGE);
        match source.page_from(symbol, next, count) {
            Ok(page) => {
                refusals = 0;
                let Some(last) = page.iter().map(|trade| trade.id).max() else {
                    return HistoryEnd::Exhausted;
                };
                sink(page);
                next = last + 1;
            }
            Err(TradesError::RateLimited) if refusals < MAX_REFUSALS => {
                refusals += 1;
                sleep(Duration::from_secs(5 * (1 << refusals)));
            }
            Err(error) => return HistoryEnd::Failed(error),
        }
    }
    HistoryEnd::WindowStart
}

// MARK: Live

/// What the stream has delivered and the owner has not read yet.
#[derive(Default)]
pub struct TradeInbox {
    trades: Mutex<Vec<Trade>>,
    /// Counts connections after the first: a change tells the owner ids may have been missed.
    reconnects: AtomicU64,
}

impl TradeInbox {
    /// Called by the stream thread (and tests).
    pub fn push(&self, trade: Trade) {
        let mut trades = self.trades.lock().unwrap_or_else(PoisonError::into_inner);
        trades.push(trade);
        if trades.len() > INBOX_CAPACITY {
            let excess = trades.len() - INBOX_CAPACITY;
            trades.drain(..excess);
        }
    }

    /// Everything received since the last call, in arrival order.
    pub fn drain(&self) -> Vec<Trade> {
        std::mem::take(&mut *self.trades.lock().unwrap_or_else(PoisonError::into_inner))
    }

    pub fn reconnects(&self) -> u64 {
        self.reconnects.load(Ordering::Acquire)
    }
}

/// An `aggTrade` stream for one market. A thread reconnects with backoff and fills the inbox.
pub struct BinanceTradeFeed {
    pub inbox: Arc<TradeInbox>,
    shutdown: Arc<AtomicBool>,
}

impl BinanceTradeFeed {
    pub fn connect(symbol: &str, ctx: egui::Context) -> Self {
        let inbox = Arc::new(TradeInbox::default());
        let shutdown = Arc::new(AtomicBool::new(false));
        let (thread_inbox, thread_shutdown) = (Arc::clone(&inbox), Arc::clone(&shutdown));
        let name = symbol.to_uppercase();
        std::thread::spawn(move || run(&name, &thread_inbox, thread_shutdown, ctx));
        Self { inbox, shutdown }
    }
}

impl Drop for BinanceTradeFeed {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
    }
}

fn run(symbol: &str, inbox: &TradeInbox, shutdown: Arc<AtomicBool>, ctx: egui::Context) {
    let url = format!(
        "wss://stream.binance.com/stream?streams={}@aggTrade",
        symbol.to_lowercase()
    );
    let mut backoff = super::ws::Backoff::new();
    let mut connections = 0u64;
    loop {
        if shutdown.load(Ordering::Acquire) {
            return;
        }
        if let Ok(mut socket) = super::ws::connect(&url, "stream.binance.com", 443) {
            connections += 1;
            if connections > 1 {
                inbox.reconnects.fetch_add(1, Ordering::AcqRel);
            }
            let mut health = super::ws::Health::new(Duration::from_secs(60));
            while !shutdown.load(Ordering::Acquire) {
                match health.read(&mut socket) {
                    Ok(Some(tungstenite::Message::Text(text))) => {
                        backoff.reset();
                        if let Some((_, trade)) = parse_stream(&text) {
                            inbox.push(trade);
                            ctx.request_repaint();
                        }
                    }
                    Ok(Some(tungstenite::Message::Close(_))) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        }
        if shutdown.load(Ordering::Acquire) {
            return;
        }
        backoff.wait(&shutdown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    /// A market with trades `1..=count`, one every 10 ms from t=1000, a failure script, and a log.
    struct Fake {
        count: u64,
        /// Errors handed out first, one per request, before any answer.
        errors: RefCell<Vec<TradesError>>,
        requests: Cell<usize>,
    }

    impl Fake {
        fn new(count: u64) -> Self {
            Self {
                count,
                errors: RefCell::new(Vec::new()),
                requests: Cell::new(0),
            }
        }

        fn trade(id: u64) -> Trade {
            Trade {
                id,
                time_ms: 1_000 + id as i64 * 10,
                price: 100.0,
                quantity: 1.0,
                buyer_is_maker: id.is_multiple_of(2),
            }
        }

        fn gate(&self) -> Result<(), TradesError> {
            self.requests.set(self.requests.get() + 1);
            let mut errors = self.errors.borrow_mut();
            if errors.is_empty() {
                Ok(())
            } else {
                Err(errors.remove(0))
            }
        }
    }

    impl TradeSource for Fake {
        fn latest(&self, _: &str) -> Result<Vec<Trade>, TradesError> {
            self.gate()?;
            Ok(if self.count == 0 {
                vec![]
            } else {
                vec![Self::trade(self.count)]
            })
        }

        fn page_from(
            &self,
            _: &str,
            from_id: u64,
            limit: usize,
        ) -> Result<Vec<Trade>, TradesError> {
            self.gate()?;
            Ok((from_id.max(1)..=self.count)
                .take(limit.min(PAGE))
                .map(Self::trade)
                .collect())
        }
    }

    /// The ids delivered, ascending, each once (the page that reaches id 0 may repeat one).
    fn unique_ids(trades: &[Trade]) -> Vec<u64> {
        let mut ids: Vec<u64> = trades.iter().map(|t| t.id).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    fn history(
        fake: &Fake,
        window_start: i64,
        budget: usize,
    ) -> (HistoryEnd, Vec<Trade>, Vec<Duration>) {
        let cancel = AtomicBool::new(false);
        let sleeps = RefCell::new(Vec::new());
        let mut all = Vec::new();
        let end = fetch_history(
            fake,
            "BTCUSDT",
            window_start,
            budget,
            &cancel,
            &|d| sleeps.borrow_mut().push(d),
            &mut |page| all.extend(page),
        );
        (end, all, sleeps.into_inner())
    }

    #[test]
    fn a_rest_row_and_a_stream_message_parse_to_the_same_trade() {
        let rest = r#"[{"a":26129,"p":"0.01633102","q":"4.70443515","f":27781,"l":27781,"T":1498793709153,"m":true,"M":true}]"#;
        let trades = parse_rest(rest).expect("rest");
        assert_eq!(
            trades[0],
            Trade {
                id: 26_129,
                time_ms: 1_498_793_709_153,
                price: 0.016_331_02,
                quantity: 4.704_435_15,
                buyer_is_maker: true,
            }
        );
        let stream = r#"{"stream":"btcusdt@aggTrade","data":{"e":"aggTrade","E":1,"s":"BTCUSDT","a":26129,"p":"0.01633102","q":"4.70443515","f":1,"l":1,"T":1498793709153,"m":true,"M":true}}"#;
        let (symbol, from_stream) = parse_stream(stream).expect("stream");
        assert_eq!(symbol, "BTCUSDT");
        assert_eq!(from_stream, trades[0]);
    }

    #[test]
    fn bad_rows_and_other_messages_are_refused() {
        assert!(parse_rest("not json").is_err());
        assert!(parse_rest(r#"{"code":-1121}"#).is_err());
        assert!(parse_rest(r#"[{"a":1}]"#).is_err());
        assert!(parse_stream(r#"{"stream":"x","data":{"e":"kline"}}"#).is_none());
        assert!(parse_stream("nope").is_none());
    }

    #[test]
    fn history_comes_newest_first_and_is_contiguous() {
        let fake = Fake::new(3_500);
        // Window starts at trade 1000's time: needs ids 1000..=3500 (2501 trades).
        let (end, trades, _) = history(&fake, 1_000 + 1_000 * 10, 1_000_000);
        assert_eq!(end, HistoryEnd::WindowStart);
        let mut ids: Vec<u64> = trades.iter().map(|t| t.id).collect();
        ids.sort_unstable();
        assert!(*ids.first().unwrap() <= 1_000);
        assert_eq!(*ids.last().unwrap(), 3_500);
        assert!(
            ids.windows(2).all(|w| w[1] == w[0] + 1),
            "no gap, no overlap"
        );
        // The first thing delivered is the newest.
        assert_eq!(trades[0].id, 3_500);
    }

    #[test]
    fn history_stops_at_the_budget_and_says_so() {
        let fake = Fake::new(10_000);
        let (end, trades, _) = history(&fake, 0, 2_500);
        assert_eq!(end, HistoryEnd::Budget);
        assert!(trades.len() >= 2_500 && trades.len() < 2_500 + PAGE);
    }

    #[test]
    fn history_stops_at_the_start_of_the_market() {
        let fake = Fake::new(2_500);
        let (end, trades, _) = history(&fake, 0, 1_000_000);
        assert_eq!(end, HistoryEnd::Exhausted);
        let ids = unique_ids(&trades);
        assert_eq!(ids.first(), Some(&1));
        assert_eq!(ids.len(), 2_500);
    }

    #[test]
    fn an_empty_market_is_exhausted_at_once() {
        let fake = Fake::new(0);
        assert_eq!(history(&fake, 0, 100).0, HistoryEnd::Exhausted);
    }

    #[test]
    fn a_cancel_stops_before_any_request() {
        let fake = Fake::new(5_000);
        let cancel = AtomicBool::new(true);
        let end = fetch_history(&fake, "BTCUSDT", 0, 100, &cancel, &|_| {}, &mut |_| {});
        assert_eq!(end, HistoryEnd::Cancelled);
        assert_eq!(fake.requests.get(), 0);
    }

    #[test]
    fn a_refusal_backs_off_then_continues() {
        let fake = Fake::new(1_500);
        fake.errors
            .borrow_mut()
            .extend([TradesError::RateLimited, TradesError::RateLimited]);
        let (end, trades, sleeps) = history(&fake, 0, 1_000_000);
        assert_eq!(end, HistoryEnd::Exhausted);
        assert_eq!(unique_ids(&trades).len(), 1_500);
        assert_eq!(sleeps, [Duration::from_secs(10), Duration::from_secs(20)]);
    }

    #[test]
    fn repeated_refusals_give_up_after_a_few_waits() {
        let fake = Fake::new(5_000);
        fake.errors
            .borrow_mut()
            .extend(std::iter::repeat_n(TradesError::RateLimited, 4));
        let (end, trades, sleeps) = history(&fake, 0, 1_000_000);
        assert_eq!(end, HistoryEnd::Failed(TradesError::RateLimited));
        assert!(trades.is_empty());
        assert_eq!(sleeps.len(), MAX_REFUSALS as usize);
    }

    #[test]
    fn a_failure_after_some_pages_keeps_what_was_delivered() {
        let fake = Fake::new(5_000);
        // latest() and the first page succeed; the second page is banned.
        let cancel = AtomicBool::new(false);
        let calls = Cell::new(0);
        struct Flaky<'a>(&'a Fake, &'a Cell<usize>);
        impl TradeSource for Flaky<'_> {
            fn latest(&self, s: &str) -> Result<Vec<Trade>, TradesError> {
                self.0.latest(s)
            }
            fn page_from(&self, s: &str, id: u64, n: usize) -> Result<Vec<Trade>, TradesError> {
                self.1.set(self.1.get() + 1);
                if self.1.get() >= 2 {
                    Err(TradesError::Banned)
                } else {
                    self.0.page_from(s, id, n)
                }
            }
        }
        let mut all = Vec::new();
        let end = fetch_history(
            &Flaky(&fake, &calls),
            "BTCUSDT",
            0,
            1_000_000,
            &cancel,
            &|_| {},
            &mut |p| {
                all.extend(p);
            },
        );
        assert_eq!(end, HistoryEnd::Failed(TradesError::Banned));
        let mut ids: Vec<u64> = all.iter().map(|t| t.id).collect();
        ids.sort_unstable();
        assert!(ids.len() > 1_000);
        assert!(
            ids.windows(2).all(|w| w[1] == w[0] + 1),
            "what arrived is gap free"
        );
    }

    #[test]
    fn a_ban_is_not_retried() {
        let fake = Fake::new(100);
        fake.errors.borrow_mut().push(TradesError::Banned);
        let (end, trades, sleeps) = history(&fake, 0, 100);
        assert_eq!(end, HistoryEnd::Failed(TradesError::Banned));
        assert!(trades.is_empty() && sleeps.is_empty());
        assert_eq!(fake.requests.get(), 1);
    }

    #[test]
    fn a_range_fetch_fills_exactly_the_missed_ids() {
        let fake = Fake::new(10_000);
        let cancel = AtomicBool::new(false);
        let mut all = Vec::new();
        let end = fetch_range(&fake, "BTCUSDT", 4_000, 6_500, &cancel, &|_| {}, &mut |p| {
            all.extend(p);
        });
        assert_eq!(end, HistoryEnd::WindowStart);
        let ids: Vec<u64> = all.iter().map(|t| t.id).collect();
        assert_eq!(ids.first(), Some(&4_000));
        assert_eq!(ids.last(), Some(&6_499));
        assert_eq!(ids.len(), 2_500);
        assert!(ids.windows(2).all(|w| w[1] == w[0] + 1));
    }

    #[test]
    fn the_inbox_hands_over_in_order_and_is_bounded() {
        let inbox = TradeInbox::default();
        for id in 0..(INBOX_CAPACITY as u64 + 10) {
            inbox.push(Fake::trade(id));
        }
        let trades = inbox.drain();
        assert_eq!(trades.len(), INBOX_CAPACITY);
        assert_eq!(trades.last().unwrap().id, INBOX_CAPACITY as u64 + 9);
        assert!(inbox.drain().is_empty());
    }

    /// Live check against Binance (public, read-only): `cargo test live_binance -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_binance_history_is_contiguous_and_newest_first() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let cancel = AtomicBool::new(false);
        let mut pages: Vec<Vec<Trade>> = Vec::new();
        let started = std::time::Instant::now();
        let end = fetch_history(
            &BinanceRest,
            "BTCUSDT",
            now - 60_000,
            20_000,
            &cancel,
            &|d| std::thread::sleep(d),
            &mut |page| pages.push(page),
        );
        let mut trades: Vec<Trade> = pages.into_iter().flatten().collect();
        trades.sort_by_key(|t| t.id);
        trades.dedup_by_key(|t| t.id);
        println!(
            "{end:?}: {} trades in {:?}, ids {}..={}, buys {}",
            trades.len(),
            started.elapsed(),
            trades.first().map_or(0, |t| t.id),
            trades.last().map_or(0, |t| t.id),
            trades.iter().filter(|t| t.is_buy()).count()
        );
        assert!(!trades.is_empty());
        assert!(
            trades.windows(2).all(|w| w[1].id == w[0].id + 1),
            "ids are consecutive"
        );
        assert!(trades.iter().all(Trade::is_valid));
    }
}
