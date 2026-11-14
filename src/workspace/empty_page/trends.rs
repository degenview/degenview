//! The last week of daily closes for the markets on the empty page, read in the background and
//! kept for a while, so each tile can draw its sparkline.

use crate::model::{InstrumentId, Kline, Timeframe};
use crate::service::{self, ProviderError};
use crate::ui::workspace::empty_state::Trend;
use eframe::egui;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Closes a sparkline shows.
const DAYS: usize = 7;
/// How long a week is shown before it is read again.
const FRESH_FOR: Duration = Duration::from_secs(10 * 60);
/// How long a market that gave no week is left alone.
const RETRY_AFTER: Duration = Duration::from_secs(5 * 60);

struct Slot {
    /// What the last read found; `None` until the first one ends.
    trend: Option<Trend>,
    /// When that read ended (or began, before it has).
    at: Instant,
    reading: bool,
}

/// Shared with the reading threads, so cloning it is cheap and every clone sees the same weeks.
#[derive(Clone, Default)]
pub(super) struct TrendLoader {
    slots: Arc<Mutex<HashMap<String, Slot>>>,
}

impl TrendLoader {
    /// What is known so far, by market key. A market still being read for the first time is not in it.
    pub(super) fn snapshot(&self) -> HashMap<String, Trend> {
        let Ok(slots) = self.slots.lock() else {
            return HashMap::new();
        };
        slots
            .iter()
            .filter_map(|(key, slot)| Some((key.clone(), slot.trend.clone()?)))
            .collect()
    }

    /// Starts reading every market of `wanted` that has no week yet or whose week has gone stale.
    pub(super) fn request(&self, wanted: &[InstrumentId], ctx: &egui::Context) {
        for instrument in self.claim(wanted, Instant::now()) {
            let loader = self.clone();
            let ctx = ctx.clone();
            service::work_queue::submit_for(instrument.source, move || {
                let result = week_of_closes(&instrument);
                loader.finish(&instrument.key(), result, Instant::now());
                ctx.request_repaint();
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

    /// Records what a read found. A failed refresh keeps the week already shown.
    fn finish(&self, key: &str, result: Result<Vec<f32>, ProviderError>, now: Instant) {
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
}

fn week_of_closes(instrument: &InstrumentId) -> Result<Vec<f32>, ProviderError> {
    service::fetch_klines(
        instrument.source,
        &instrument.api_symbol(),
        Timeframe::Day.interval(),
        DAYS as u32 + 1,
    )
    .map(|candles| closes(&candles))
}

/// The closes of the last `DAYS` candles, oldest first.
fn closes(candles: &[Kline]) -> Vec<f32> {
    let start = candles.len().saturating_sub(DAYS);
    candles[start..]
        .iter()
        .map(|candle| candle.close as f32)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;

    fn market(symbol: &str) -> InstrumentId {
        InstrumentId::new(DataSource::Binance, symbol)
    }

    fn candle(close: f64) -> Kline {
        Kline {
            open_time: 0,
            close_time: 1,
            open: close,
            high: close,
            low: close,
            close,
            volume: 0.0,
        }
    }

    #[test]
    fn a_market_is_read_once_while_it_is_being_read() {
        let loader = TrendLoader::default();
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
    fn a_week_is_shown_then_read_again_when_stale() {
        let loader = TrendLoader::default();
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
        // The old week stays on screen while the new one is read, and survives a failed read.
        assert!(loader.snapshot().contains_key(&btc.key()));
        loader.finish(&btc.key(), Err(ProviderError::other("offline")), later);
        assert_eq!(
            loader.snapshot().get(&btc.key()),
            Some(&Trend::Ready(vec![1.0, 2.0, 3.0]))
        );
    }

    #[test]
    fn a_market_without_history_is_retried_later() {
        let loader = TrendLoader::default();
        let market = market("NOPEUSDT");
        let now = Instant::now();
        loader.claim(std::slice::from_ref(&market), now);
        loader.finish(
            &market.key(),
            Err(ProviderError::other("unknown market")),
            now,
        );
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
    fn one_close_is_not_a_week() {
        let loader = TrendLoader::default();
        let market = market("BTCUSDT");
        let now = Instant::now();
        loader.claim(std::slice::from_ref(&market), now);
        loader.finish(&market.key(), Ok(vec![5.0]), now);
        assert_eq!(
            loader.snapshot().get(&market.key()),
            Some(&Trend::Unavailable)
        );
    }

    #[test]
    fn only_the_last_week_of_candles_is_kept() {
        let candles: Vec<Kline> = (0..10).map(|day| candle(day as f64)).collect();
        assert_eq!(closes(&candles), vec![3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
        assert_eq!(closes(&candles[..3]), vec![0.0, 1.0, 2.0]);
        assert!(closes(&[]).is_empty());
    }
}
