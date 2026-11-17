use crate::lock::lock;
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
        let mut guard = lock(&self.next_slot);
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
static KRAKEN: OnceLock<RateLimiter> = OnceLock::new();

pub fn coingecko() -> &'static RateLimiter {
    COINGECKO.get_or_init(|| RateLimiter::new(Duration::from_secs(4)))
}

/// Kraken's public REST endpoints let a client through in a burst of about nine requests and then
/// answer `EGeneral:Too many requests` to anything faster than one every few seconds (measured: at
/// a request a second two of five got through, at one every three seconds every second one was
/// refused after the burst). Six seconds keeps a client inside it.
pub fn kraken() -> &'static RateLimiter {
    KRAKEN.get_or_init(|| RateLimiter::new(Duration::from_secs(6)))
}

pub fn geckoterminal() -> &'static RateLimiter {
    GECKOTERMINAL.get_or_init(|| RateLimiter::new(Duration::from_millis(2_200)))
}
