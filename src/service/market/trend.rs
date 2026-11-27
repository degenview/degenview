//! Recent closes of markets, read in the background and kept for a while, so a tile, a watchlist
//! row or the symbol picker can draw them. One loader reads one span (a week, a month, the last
//! day).

use crate::model::market::trend::{self, Trend};
use crate::model::session_stats::MarketHours;
use crate::model::{DataSource, InstrumentId, Kline, Timeframe};
use crate::service::{self, ProviderError};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

/// How long a span of days is shown before it is read again.
const FRESH_FOR: Duration = Duration::from_secs(10 * 60);
/// How long the last day is shown before it is read again.
const DAY_FRESH_FOR: Duration = Duration::from_secs(2 * 60);
/// The candles of the last day: 24 hours of them.
const DAY_CANDLE: Timeframe = Timeframe::MINUTE_15;
const DAY_CANDLES: u32 = 96;
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

/// Where a loader's candles come from: the providers, or canned ones under test.
pub type Fetch = fn(DataSource, &str, Timeframe, u32) -> Result<Vec<Kline>, ProviderError>;

/// What a loader reads of each market.
#[derive(Clone, Copy, Debug)]
enum Span {
    /// Daily closes of this many calendar days.
    Days(u32),
    /// The last day's 15-minute closes; a stock's last session.
    LastDay,
}

/// Shared with the reading threads, so cloning it is cheap and every clone sees the same closes.
#[derive(Clone)]
pub struct TrendLoader {
    span: Span,
    fetch: Fetch,
    slots: Arc<Mutex<HashMap<String, Slot>>>,
}

impl TrendLoader {
    /// A loader of `days` calendar days of daily closes.
    pub fn new(days: u32) -> Self {
        Self::reading(Span::Days(days.max(2)), service::fetch_klines)
    }

    /// A loader of the last day, for a chart beside a price and its change.
    pub fn last_day(fetch: Fetch) -> Self {
        Self::reading(Span::LastDay, fetch)
    }

    fn reading(span: Span, fetch: Fetch) -> Self {
        Self {
            span,
            fetch,
            slots: Arc::default(),
        }
    }

    /// What is known of one market.
    pub fn get(&self, key: &str) -> Option<Trend> {
        self.slots.lock().ok()?.get(key)?.trend.clone()
    }

    /// Like [`Self::request`] for one market whose read is only worth making while `wanted` says
    /// so when its turn comes: a market scrolled past is not fetched.
    pub fn request_if(
        &self,
        instrument: &InstrumentId,
        wanted: impl Fn() -> bool + Send + 'static,
        arrived: Arc<dyn Fn() + Send + Sync>,
    ) {
        let claimed = self.claim(std::slice::from_ref(instrument), Instant::now());
        if let Some(instrument) = claimed.into_iter().next() {
            self.read_in_background(instrument, wanted, arrived);
        }
    }

    /// Reads a claimed market on its provider's workers, unless it is no longer wanted by then.
    fn read_in_background(
        &self,
        instrument: InstrumentId,
        wanted: impl Fn() -> bool + Send + 'static,
        arrived: Arc<dyn Fn() + Send + Sync>,
    ) {
        let loader = self.clone();
        service::work_queue::submit_for(instrument.source, move || {
            if !wanted() {
                loader.release(&instrument.key());
                return;
            }
            let result = loader.read(&instrument);
            loader.finish(&instrument.key(), result, Instant::now());
            arrived();
        });
    }

    /// Gives up a read that was never made, so the next request makes it.
    fn release(&self, key: &str) {
        let Ok(mut slots) = self.slots.lock() else {
            return;
        };
        let Some(slot) = slots.get_mut(key) else {
            return;
        };
        if slot.trend.is_none() {
            slots.remove(key);
        } else {
            slot.reading = false;
        }
    }

    /// Asks again for the markets that gave nothing, without waiting out their pause: what was
    /// missing (a login, a network) may be there now.
    pub fn forget_unavailable(&self) {
        if let Ok(mut slots) = self.slots.lock() {
            slots.retain(|_, slot| slot.reading || slot.trend != Some(Trend::Unavailable));
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
            self.read_in_background(instrument, || true, Arc::clone(&arrived));
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
                Some(Trend::Ready(_)) => now.duration_since(slot.at) >= self.fresh_for(),
                Some(Trend::Unavailable) => now.duration_since(slot.at) >= RETRY_AFTER,
            };
            if due && !slot.reading {
                slot.reading = true;
                claimed.push(instrument.clone());
            }
        }
        claimed
    }

    fn fresh_for(&self) -> Duration {
        match self.span {
            Span::Days(_) => FRESH_FOR,
            Span::LastDay => DAY_FRESH_FOR,
        }
    }

    /// Records what a read found. A failed refresh keeps the closes already shown.
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

    fn read(&self, instrument: &InstrumentId) -> Result<Vec<f32>, ProviderError> {
        let symbol = instrument.api_symbol();
        let fetch = |timeframe, limit| (self.fetch)(instrument.source, &symbol, timeframe, limit);
        match self.span {
            // One more candle than the span has days, so a market that trades every day fills it.
            Span::Days(days) => {
                fetch(Timeframe::DAY_1, days + 1).map(|bars| trend::closes_within(&bars, days))
            }
            Span::LastDay => fetch(DAY_CANDLE, DAY_CANDLES).map(|bars| {
                if MarketHours::for_source(instrument.source) == MarketHours::UsStock {
                    trend::last_session_closes(&bars)
                } else {
                    trend::last_day_closes(&bars)
                }
            }),
        }
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
        loader.finish(&btc.key(), Err(ProviderError::other("offline")), later);
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
    fn a_read_given_up_is_made_again_and_a_failed_one_forgotten_on_request() {
        let loader = TrendLoader::new(7);
        let btc = market("BTCUSDT");
        let now = Instant::now();
        loader.claim(std::slice::from_ref(&btc), now);
        loader.release(&btc.key());
        assert_eq!(loader.claim(std::slice::from_ref(&btc), now).len(), 1);
        loader.finish(&btc.key(), Err(ProviderError::other("no key")), now);
        assert_eq!(loader.get(&btc.key()), Some(Trend::Unavailable));
        loader.forget_unavailable();
        assert_eq!(loader.get(&btc.key()), None);
        assert_eq!(loader.claim(std::slice::from_ref(&btc), now).len(), 1);
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
