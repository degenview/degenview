use crate::lock::lock;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::market::venue;
use crate::service::net::ws::{Frame, Session};
use eframe::egui;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The most bars the inbox keeps. A feed nobody reads for days (a background tab, a hidden
/// window) still sees one new bar per symbol per interval, so this is only a backstop.
const INBOX_CAPACITY: usize = 4096;

/// What a live feed has received and the workspace has not applied yet: the latest update of each
/// `(bar, symbol)`. A kline stream resends the forming bar every second or two, and applying the
/// last of those is the same as applying all of them, so the queue never grows with time. A plain
/// channel did: it is only drained while the workspace is painted, and a tab left in the
/// background kept every update for as long as the app ran.
#[derive(Default)]
pub struct LiveInbox {
    latest: Mutex<BTreeMap<(i64, String), Kline>>,
}

impl LiveInbox {
    fn push(&self, symbol: String, kline: Kline) {
        let mut latest = lock(&self.latest);
        latest.insert((kline.open_time, symbol), kline);
        while latest.len() > INBOX_CAPACITY {
            latest.pop_first();
        }
    }

    /// Everything received since the last call, oldest bar first.
    pub fn drain(&self) -> Vec<(String, Kline)> {
        let mut latest = lock(&self.latest);
        std::mem::take(&mut *latest)
            .into_iter()
            .map(|((_, symbol), kline)| (symbol, kline))
            .collect()
    }
}

/// How a venue's candle socket is opened and read. The loop, the reconnecting and the inbox are
/// [`LiveFeed`]'s alone.
pub trait CandleProtocol: Sync {
    /// The connections that stream the candles of length `base` for `symbols` (an exchange that
    /// caps the topics of a connection gets several); `None` when the venue has no such candle.
    fn sessions(&self, symbols: &[String], base: Timeframe) -> Option<Vec<Session>>;

    /// What one message says: the market as the venue names it, and its candle. Nothing for a
    /// message that is not one (an acknowledgement, a heartbeat, another topic).
    fn parse(&self, frame: Frame<'_>, base: Timeframe) -> Option<(String, Kline)>;
}

/// A kline feed for one workspace's charts of one source: background threads (one per connection:
/// an exchange that caps the topics of a connection gets several) that reconnect with backoff and
/// put `(symbol, kline)` updates in the inbox.
pub struct LiveFeed {
    pub inbox: Arc<LiveInbox>,
    signature: (Vec<String>, Timeframe),
    shutdown: Arc<AtomicBool>,
}

impl LiveFeed {
    /// Opens the stream of `base` candles for `symbols`; `None` when `source` streams none.
    pub fn connect(
        source: DataSource,
        symbols: Vec<String>,
        base: Timeframe,
        ctx: egui::Context,
    ) -> Option<Self> {
        let protocol = venue::of(source).live()?;
        let sessions = protocol.sessions(&symbols, base)?;
        let inbox = Arc::new(LiveInbox::default());
        let shutdown = Arc::new(AtomicBool::new(false));
        for session in sessions {
            let thread_shutdown = Arc::clone(&shutdown);
            let thread_inbox = Arc::clone(&inbox);
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                session.run_frames(&thread_shutdown, |frame| {
                    if let Some((symbol, kline)) = protocol.parse(frame, base) {
                        thread_inbox.push(symbol, kline);
                        ctx.request_repaint();
                    }
                    true
                });
            });
        }
        Some(Self {
            inbox,
            signature: (symbols, base),
            shutdown,
        })
    }

    /// The symbols and candle length this feed was opened for, so the owner can tell when a
    /// reconnect is needed.
    pub fn signature(&self) -> &(Vec<String>, Timeframe) {
        &self.signature
    }
}

impl Drop for LiveFeed {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kline(open_time: i64, close: f64) -> Kline {
        Kline {
            open_time,
            close_time: open_time + 3_599_999,
            open: 1.0,
            high: 2.0,
            low: 0.5,
            close,
            volume: 1.0,
        }
    }

    #[test]
    fn the_inbox_keeps_only_the_latest_update_of_a_bar() {
        let inbox = LiveInbox::default();
        for tick in 0..10_000 {
            inbox.push("BTCUSDT".into(), kline(1_000, f64::from(tick)));
        }
        let drained = inbox.drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].1.close, 9_999.0);
        assert!(inbox.drain().is_empty());
    }

    #[test]
    fn the_inbox_hands_over_older_bars_first() {
        let inbox = LiveInbox::default();
        inbox.push("ETHUSDT".into(), kline(2_000, 1.0));
        inbox.push("BTCUSDT".into(), kline(2_000, 2.0));
        inbox.push("BTCUSDT".into(), kline(1_000, 3.0));
        let order: Vec<_> = inbox
            .drain()
            .into_iter()
            .map(|(symbol, kline)| (kline.open_time, symbol))
            .collect();
        assert_eq!(
            order,
            [
                (1_000, "BTCUSDT".to_string()),
                (2_000, "BTCUSDT".to_string()),
                (2_000, "ETHUSDT".to_string())
            ]
        );
    }

    #[test]
    fn an_unread_inbox_stops_growing_at_its_capacity() {
        let inbox = LiveInbox::default();
        for bar in 0..(INBOX_CAPACITY as i64 + 500) {
            inbox.push("BTCUSDT".into(), kline(bar * 3_600_000, 1.0));
        }
        let drained = inbox.drain();
        assert_eq!(drained.len(), INBOX_CAPACITY);
        // The newest bars survive.
        assert_eq!(
            drained.last().unwrap().1.open_time,
            (INBOX_CAPACITY as i64 + 499) * 3_600_000
        );
    }

    /// Run with `cargo test --lib live::tests::live_okx -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_okx_streams_the_forming_candle() {
        let feed = LiveFeed::connect(
            DataSource::Okx,
            vec!["BTC-USDT".to_owned()],
            Timeframe::MINUTE_1,
            egui::Context::default(),
        )
        .expect("OKX streams one-minute candles");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            if let Some((symbol, kline)) = feed.inbox.drain().pop() {
                assert_eq!(symbol, "BTC-USDT");
                assert!(kline.close > 0.0 && kline.close_time > kline.open_time);
                return;
            }
            assert!(std::time::Instant::now() < deadline, "no candle in 30 s");
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }

    /// Run with `cargo test --lib live::tests::live_kraken -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_kraken_streams_the_forming_candle() {
        let feed = LiveFeed::connect(
            DataSource::Kraken,
            vec!["BTC/USD".to_owned()],
            Timeframe::MINUTE_1,
            egui::Context::default(),
        )
        .expect("Kraken streams one-minute candles");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            if let Some((symbol, kline)) = feed.inbox.drain().pop() {
                assert_eq!(symbol, "BTC/USD");
                assert!(kline.close > 0.0 && kline.close_time > kline.open_time);
                return;
            }
            assert!(std::time::Instant::now() < deadline, "no candle in 60 s");
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }

    /// Run with `cargo test --lib live::tests::live_bybit -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_bybit_streams_the_forming_candle_of_every_market_asked_for_even_beside_an_unknown_one()
    {
        let feed = LiveFeed::connect(
            DataSource::Bybit,
            vec![
                "NOPEUSDT".to_owned(),
                "BTCUSDT".to_owned(),
                "eth/usdt".to_owned(),
            ],
            Timeframe::MINUTE_1,
            egui::Context::default(),
        )
        .expect("Bybit streams one-minute candles");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let mut seen = std::collections::BTreeSet::new();
        while seen.len() < 2 {
            for (symbol, kline) in feed.inbox.drain() {
                assert!(kline.close > 0.0 && kline.close_time > kline.open_time);
                seen.insert(symbol);
            }
            assert!(
                std::time::Instant::now() < deadline,
                "only {seen:?} in 60 s"
            );
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        assert_eq!(seen.into_iter().collect::<Vec<_>>(), ["BTCUSDT", "ETHUSDT"]);
    }

    /// Run with `cargo test --lib live::tests::live_gate -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_gate_streams_the_forming_candle_of_every_market_asked_for_even_beside_an_unknown_one() {
        let feed = LiveFeed::connect(
            DataSource::Gate,
            vec![
                "NOPE_USDT".to_owned(),
                "BTC_USDT".to_owned(),
                "eth/usdt".to_owned(),
            ],
            Timeframe::MINUTE_1,
            egui::Context::default(),
        )
        .expect("Gate.io streams one-minute candles");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let mut seen = std::collections::BTreeSet::new();
        while seen.len() < 2 {
            for (symbol, kline) in feed.inbox.drain() {
                assert!(kline.close > 0.0 && kline.close_time > kline.open_time);
                seen.insert(symbol);
            }
            assert!(
                std::time::Instant::now() < deadline,
                "only {seen:?} in 60 s"
            );
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        assert_eq!(
            seen.into_iter().collect::<Vec<_>>(),
            ["BTC_USDT", "ETH_USDT"]
        );
    }

    /// Run with `cargo test --lib live::tests::live_mexc -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_mexc_streams_the_forming_candle_of_every_market_asked_for_even_beside_an_unknown_one() {
        let feed = LiveFeed::connect(
            DataSource::Mexc,
            vec![
                "NOPEUSDT".to_owned(),
                "BTCUSDT".to_owned(),
                "eth/usdt".to_owned(),
            ],
            Timeframe::MINUTE_1,
            egui::Context::default(),
        )
        .expect("MEXC streams one-minute candles");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let mut seen = std::collections::BTreeSet::new();
        while seen.len() < 2 {
            for (symbol, kline) in feed.inbox.drain() {
                assert!(kline.close > 0.0 && kline.close_time > kline.open_time);
                seen.insert(symbol);
            }
            assert!(
                std::time::Instant::now() < deadline,
                "only {seen:?} in 60 s"
            );
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        assert_eq!(seen.into_iter().collect::<Vec<_>>(), ["BTCUSDT", "ETHUSDT"]);
    }

    #[test]
    fn a_source_without_a_candle_stream_opens_none() {
        assert!(
            LiveFeed::connect(
                DataSource::Coinbase,
                vec!["BTC-USD".to_owned()],
                Timeframe::MINUTE_1,
                egui::Context::default()
            )
            .is_none()
        );
        // OKX has no 8-hour bar to stream.
        assert!(
            LiveFeed::connect(
                DataSource::Okx,
                vec!["BTC-USDT".to_owned()],
                Timeframe::parse("8h").unwrap(),
                egui::Context::default()
            )
            .is_none()
        );
    }
}
