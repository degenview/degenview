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
static LIGHTER: OnceLock<RateLimiter> = OnceLock::new();
static BITFINEX: OnceLock<RateLimiter> = OnceLock::new();
static CMC_GLOBAL: OnceLock<RateLimiter> = OnceLock::new();

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

/// Lighter lets an IP make about one request a second: a burst of 60 or so, then HTTP 429
/// (`{"code":23000,"message":"Too Many Requests!"}`) until the allowance, which comes back at about
/// one a second, has refilled (measured on 2026-11-21: 80 requests at exactly one a second all got
/// through, a faster pace was refused after the burst). A second a request keeps a client inside it.
pub fn lighter() -> &'static RateLimiter {
    LIGHTER.get_or_init(|| RateLimiter::new(Duration::from_secs(1)))
}

/// Bitfinex answers HTTP 429 to the 34th candle request of a run at one a second, and then nothing
/// for about five minutes (`Retry-After: 284`, measured on 2026-11-21); 80 requests at one every two
/// seconds were all answered. 2.5 seconds a request (24 a minute) keeps a client inside it with
/// room for the market list and a second window of the app.
pub fn bitfinex() -> &'static RateLimiter {
    BITFINEX.get_or_init(|| RateLimiter::new(Duration::from_millis(2_500)))
}

/// CoinMarketCap's website data API (the global-metrics history behind `TOTAL` and `BTC.D`) has no
/// published limit. Twelve requests in a row were all answered (2026-11-21); it does answer "the
/// system is busy" under load. A second a request keeps a chart's few windows from arriving as a
/// burst.
pub fn cmc_global() -> &'static RateLimiter {
    CMC_GLOBAL.get_or_init(|| RateLimiter::new(Duration::from_secs(1)))
}

pub fn geckoterminal() -> &'static RateLimiter {
    GECKOTERMINAL.get_or_init(|| RateLimiter::new(Duration::from_millis(2_200)))
}
