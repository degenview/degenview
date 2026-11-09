use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Enforces a minimum gap between calls to one rate-limited host. Callers reserve
/// their slot before sleeping, so concurrent callers don't all read the same free
/// slot and fire together.
pub struct RateLimiter {
    next_slot: Mutex<Option<Instant>>,
    gap: Duration,
}

impl RateLimiter {
    fn new(gap: Duration) -> Self {
        Self {
            next_slot: Mutex::new(None),
            gap,
        }
    }

    pub fn wait_for_slot(&self) {
        let mut guard = self.next_slot.lock().expect("rate limiter poisoned");
        let now = Instant::now();
        let next = match *guard {
            Some(slot) if slot > now => slot,
            _ => now,
        };
        *guard = Some(next + self.gap);
        let wait = next.saturating_duration_since(now);
        drop(guard);
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
    }
}

static COINGECKO: OnceLock<RateLimiter> = OnceLock::new();
static GECKOTERMINAL: OnceLock<RateLimiter> = OnceLock::new();

pub fn coingecko() -> &'static RateLimiter {
    COINGECKO.get_or_init(|| RateLimiter::new(Duration::from_secs(4)))
}

pub fn geckoterminal() -> &'static RateLimiter {
    GECKOTERMINAL.get_or_init(|| RateLimiter::new(Duration::from_millis(2_200)))
}

static BINANCE_TRADES: OnceLock<RateLimiter> = OnceLock::new();

/// Binance's `aggTrades` weighs 2 against a shared 6000-a-minute budget (klines, quotes and every
/// other request of this IP included). One request every 250 ms is at most 480 a minute, leaving
/// the rest to the charts.
pub fn binance_trades() -> &'static RateLimiter {
    BINANCE_TRADES.get_or_init(|| RateLimiter::new(Duration::from_millis(250)))
}
