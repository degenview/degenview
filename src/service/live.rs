use crate::model::Kline;
use eframe::egui;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

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
        let mut latest = self.latest.lock().unwrap_or_else(PoisonError::into_inner);
        latest.insert((kline.open_time, symbol), kline);
        while latest.len() > INBOX_CAPACITY {
            latest.pop_first();
        }
    }

    /// Everything received since the last call, oldest bar first.
    pub fn drain(&self) -> Vec<(String, Kline)> {
        let mut latest = self.latest.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut *latest)
            .into_iter()
            .map(|((_, symbol), kline)| (symbol, kline))
            .collect()
    }
}

/// A Binance combined-stream kline feed for one workspace's Binance charts. Runs
/// a background thread that reconnects with backoff and puts `(symbol, kline)`
/// updates in the inbox.
pub struct BinanceLiveFeed {
    pub inbox: Arc<LiveInbox>,
    signature: (Vec<String>, String),
    shutdown: Arc<AtomicBool>,
}

impl BinanceLiveFeed {
    pub fn connect(symbols: Vec<String>, interval: String, ctx: egui::Context) -> Self {
        let inbox = Arc::new(LiveInbox::default());
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = Arc::clone(&shutdown);
        let thread_inbox = Arc::clone(&inbox);
        let signature = (symbols.clone(), interval.clone());
        std::thread::spawn(move || {
            run(symbols, interval, &thread_inbox, thread_shutdown, ctx);
        });
        Self {
            inbox,
            signature,
            shutdown,
        }
    }

    /// The symbols and interval this feed was opened for, so the owner can tell
    /// when a reconnect is needed.
    pub fn signature(&self) -> &(Vec<String>, String) {
        &self.signature
    }
}

impl Drop for BinanceLiveFeed {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
    }
}

fn run(
    symbols: Vec<String>,
    interval: String,
    inbox: &LiveInbox,
    shutdown: Arc<AtomicBool>,
    ctx: egui::Context,
) {
    let streams = symbols
        .iter()
        .map(|symbol| format!("{}@kline_{interval}", symbol.to_lowercase()))
        .collect::<Vec<_>>()
        .join("/");
    let url = format!("wss://stream.binance.com/stream?streams={streams}");
    let mut backoff = crate::service::ws::Backoff::new();

    loop {
        if shutdown.load(Ordering::Acquire) {
            return;
        }
        if let Ok(mut socket) = crate::service::ws::connect(&url, "stream.binance.com", 443) {
            let mut health = crate::service::ws::Health::new(Duration::from_secs(60));
            while !shutdown.load(Ordering::Acquire) {
                match health.read(&mut socket) {
                    Ok(Some(tungstenite::Message::Text(text))) => {
                        backoff.reset();
                        if let Some((symbol, kline)) = parse_kline(&text) {
                            inbox.push(symbol, kline);
                            ctx.request_repaint();
                        }
                    }
                    Ok(Some(tungstenite::Message::Close(_))) => break,
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        }
        if shutdown.load(Ordering::Acquire) {
            return;
        }
        backoff.wait(&shutdown);
    }
}

/// `{"stream":"btcusdt@kline_1h","data":{"k":{"t","o","h","l","c","v",…}}}`.
fn parse_kline(text: &str) -> Option<(String, Kline)> {
    let value: Value = serde_json::from_str(text).ok()?;
    let stream = value.get("stream")?.as_str()?;
    let symbol = stream.split('@').next()?.to_uppercase();
    let k = value.pointer("/data/k")?;
    let t = k.get("t").and_then(Value::as_i64)?;
    // `T` is the bar's last millisecond, as in the REST rows. Without it the forming bar would read
    // as already closed (`close_time <= now`) and Pine would rebuild on every tick.
    let close_time = k.get("T").and_then(Value::as_i64).unwrap_or(t);
    let open = k.get("o").and_then(Value::as_str)?.parse().ok()?;
    let high = k.get("h").and_then(Value::as_str)?.parse().ok()?;
    let low = k.get("l").and_then(Value::as_str)?.parse().ok()?;
    let close = k.get("c").and_then(Value::as_str)?.parse().ok()?;
    let volume = k.get("v").and_then(Value::as_str)?.parse().ok()?;
    Some((
        symbol,
        Kline {
            open_time: t,
            close_time,
            open,
            high,
            low,
            close,
            volume,
        },
    ))
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

    #[test]
    fn parses_kline_stream_message() {
        let text = r#"{"stream":"btcusdt@kline_1h","data":{"k":{"t":1700000000000,"T":1700003599999,"o":"10","h":"12","l":"9","c":"11","v":"42.5","q":"0","x":false}}}"#;
        let (symbol, kline) = parse_kline(text).expect("parse");
        assert_eq!(symbol, "BTCUSDT");
        assert_eq!(kline.open_time, 1_700_000_000_000);
        assert_eq!(kline.close_time, 1_700_003_599_999);
        assert_eq!(kline.high, 12.0);
        assert_eq!(kline.low, 9.0);
        assert_eq!(kline.close, 11.0);
        assert_eq!(kline.volume, 42.5);
    }

    #[test]
    fn a_message_without_a_close_time_falls_back_to_the_open_time() {
        let text = r#"{"stream":"btcusdt@kline_1h","data":{"k":{"t":1700000000000,"o":"10","h":"12","l":"9","c":"11","v":"1"}}}"#;
        let (_, kline) = parse_kline(text).expect("parse");
        assert_eq!(kline.close_time, 1_700_000_000_000);
    }
}
