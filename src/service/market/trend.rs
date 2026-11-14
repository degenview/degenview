//! Recent daily closes of markets, read in the background and kept for a while, so a tile or a
//! watchlist row can draw its sparkline. One loader reads one span (a week, a month).

use crate::model::market::trend::{self, Trend};
use crate::model::{InstrumentId, Timeframe};
use crate::service;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

/// How long a span is shown before it is read again.
const FRESH_FOR: Duration = Duration::from_secs(10 * 60);
/// How long a market that gave no closes is left alone.
const RETRY_AFTER: Duration = Duration::from_secs(5 * 60);
/// Calendar days of the watchlist's month.
const MONTH_DAYS: u32 = 30;

/// The month every watchlist sidebar draws. One for the app, so tabs and windows showing the
/// same markets share their reads.
pub fn month() -> &'static TrendLoader {
    static MONTH: LazyLock<TrendLoader> = LazyLock::new(|| TrendLoader::new(MONTH_DAYS));
    &MONTH
}

struct Slot {
    /// What the last read found; `None` until the first one ends.
    trend: Option<Trend>,
    /// When that read ended (or began, before it has).
    at: Instant,
    reading: bool,
}

/// Shared with the reading threads, so cloning it is cheap and every clone sees the same closes.
#[derive(Clone)]
pub struct TrendLoader {
    /// Calendar days of closes each market shows.
    days: u32,
    slots: Arc<Mutex<HashMap<String, Slot>>>,
}

impl TrendLoader {
    pub fn new(days: u32) -> Self {
        Self {
            days: days.max(2),
            slots: Arc::default(),
        }
    }

    /// What is known so far, by market key. A market still being read for the first time is not in it.
    pub fn snapshot(&self) -> HashMap<String, Trend> {
        let Ok(slots) = self.slots.lock() else {
            return HashMap::new();
        };
        slots
            .iter()
            .filter_map(|(key, slot)| Some((key.clone(), slot.trend.clone()?)))
            .collect()
    }

    /// Starts reading every market of `wanted` that has no closes yet or whose closes have gone
    /// stale; `arrived` runs after each read (a repaint request).
    pub fn request(&self, wanted: &[InstrumentId], arrived: Arc<dyn Fn() + Send + Sync>) {
        for instrument in self.claim(wanted, Instant::now()) {
            let loader = self.clone();
            let arrived = Arc::clone(&arrived);
            service::work_queue::submit_for(instrument.source, move || {
                let result = loader.read(&instrument);
                loader.finish(&instrument.key(), result, Instant::now());
                arrived();
            });
        }
    }

    /// Marks the markets of `wanted` that need a read as being read, and returns them.
    fn claim(&self, wanted: &[InstrumentId], now: Instant) -> Vec<InstrumentId> {
        let Ok(mut slots) = self.slots.lock() else {
            return Vec::new();
        };
        let mut claimed = Vec::new();
        for instrument in wanted {
            let slot = slots.entry(instrument.key()).or_insert_with(|| Slot {
                trend: None,
                at: now,
                reading: false,
            });
            let due = match &slot.trend {
                None => true,
                Some(Trend::Ready(_)) => now.duration_since(slot.at) >= FRESH_FOR,
                Some(Trend::Unavailable) => now.duration_since(slot.at) >= RETRY_AFTER,
            };
            if due && !slot.reading {
                slot.reading = true;
                claimed.push(instrument.clone());
            }
        }
        claimed
    }

    /// Records what a read found. A failed refresh keeps the closes already shown.
    fn finish(&self, key: &str, result: Result<Vec<f32>, String>, now: Instant) {
        let Ok(mut slots) = self.slots.lock() else {
            return;
        };
        let Some(slot) = slots.get_mut(key) else {
            return;
        };
        slot.reading = false;
        slot.at = now;
        match result {
            Ok(closes) if closes.len() >= 2 => slot.trend = Some(Trend::Ready(closes)),
            _ if matches!(slot.trend, Some(Trend::Ready(_))) => {}
            _ => slot.trend = Some(Trend::Unavailable),
        }
    }

    /// One more candle than the span has days, so a market that trades every day fills it.
    fn read(&self, instrument: &InstrumentId) -> Result<Vec<f32>, String> {
        service::fetch_klines(
            instrument.source,
            &instrument.api_symbol(),
            Timeframe::Day.interval(),
            self.days + 1,
        )
        .map(|candles| trend::closes_within(&candles, self.days))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;

    fn market(symbol: &str) -> InstrumentId {
        InstrumentId::new(DataSource::Binance, symbol)
    }

    #[test]
    fn a_market_is_read_once_while_it_is_being_read() {
        let loader = TrendLoader::new(7);
        let now = Instant::now();
        let wanted = [market("BTCUSDT"), market("btcusdt"), market("ETHUSDT")];
        let claimed = loader.claim(&wanted, now);
        assert_eq!(claimed.len(), 2, "two spellings of one market are one read");
        assert!(loader.claim(&wanted, now).is_empty());
        assert!(
            loader.snapshot().is_empty(),
            "nothing is shown before it arrives"
        );
    }

    #[test]
    fn closes_are_shown_then_read_again_when_stale() {
        let loader = TrendLoader::new(7);
        let btc = market("BTCUSDT");
        let now = Instant::now();
        loader.claim(std::slice::from_ref(&btc), now);
        loader.finish(&btc.key(), Ok(vec![1.0, 2.0, 3.0]), now);
        assert_eq!(
            loader.snapshot().get(&btc.key()),
            Some(&Trend::Ready(vec![1.0, 2.0, 3.0]))
        );
        assert!(loader.claim(std::slice::from_ref(&btc), now).is_empty());
        let later = now + FRESH_FOR;
        assert_eq!(loader.claim(std::slice::from_ref(&btc), later).len(), 1);
        // The old closes stay on screen while the new ones are read, and survive a failed read.
        assert!(loader.snapshot().contains_key(&btc.key()));
        loader.finish(&btc.key(), Err("offline".into()), later);
        assert_eq!(
            loader.snapshot().get(&btc.key()),
            Some(&Trend::Ready(vec![1.0, 2.0, 3.0]))
        );
    }

    #[test]
    fn a_market_without_history_is_retried_later() {
        let loader = TrendLoader::new(7);
        let market = market("NOPEUSDT");
        let now = Instant::now();
        loader.claim(std::slice::from_ref(&market), now);
        loader.finish(&market.key(), Err("unknown market".into()), now);
        assert_eq!(
            loader.snapshot().get(&market.key()),
            Some(&Trend::Unavailable)
        );
        assert!(loader.claim(std::slice::from_ref(&market), now).is_empty());
        assert_eq!(
            loader
                .claim(std::slice::from_ref(&market), now + RETRY_AFTER)
                .len(),
            1
        );
    }

    #[test]
    fn one_close_is_not_a_trend() {
        let loader = TrendLoader::new(7);
        let market = market("BTCUSDT");
        let now = Instant::now();
        loader.claim(std::slice::from_ref(&market), now);
        loader.finish(&market.key(), Ok(vec![5.0]), now);
        assert_eq!(
            loader.snapshot().get(&market.key()),
            Some(&Trend::Unavailable)
        );
    }
}
