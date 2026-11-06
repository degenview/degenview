//! Rate limits for the MCP server: one bucket for failed authentications and one per client.
//!
//! Every connection arrives from 127.0.0.1, so there is no address to key a limit on. Failures
//! share one bucket instead, and the bucket only ever slows *bad* tokens down: a valid token is
//! never refused because of failures. Calls from a valid client draw from that client's own
//! bucket, so one runaway agent cannot starve another.
//!
//! The buckets take the current time as an argument, so tests drive them with made-up instants
//! and never sleep.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Failed authentications let through at once before the rest are answered 429.
pub const FAILED_AUTH_BURST: u32 = 20;
/// Failed authentications let through per second once the burst is spent.
pub const FAILED_AUTH_PER_SEC: f64 = 2.0;
/// Calls one client may make at once before it is slowed down. Agents batch calls, so this is
/// generous: it exists to stop a runaway loop, not to meter ordinary use.
pub const CLIENT_BURST: u32 = 60;
/// Calls per second one client keeps getting once its burst is spent.
pub const CLIENT_PER_SEC: f64 = 20.0;

/// How many calls a bucket lets through and how fast it refills.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RateLimits {
    pub failed_burst: u32,
    pub failed_per_sec: f64,
    pub client_burst: u32,
    pub client_per_sec: f64,
}

impl Default for RateLimits {
    fn default() -> Self {
        Self {
            failed_burst: FAILED_AUTH_BURST,
            failed_per_sec: FAILED_AUTH_PER_SEC,
            client_burst: CLIENT_BURST,
            client_per_sec: CLIENT_PER_SEC,
        }
    }
}

/// A token bucket: starts full, spends one token per call, refills continuously up to its
/// capacity.
#[derive(Clone, Debug)]
pub struct TokenBucket {
    capacity: f64,
    per_sec: f64,
    tokens: f64,
    last: Option<Instant>,
}

impl TokenBucket {
    pub fn new(capacity: u32, per_sec: f64) -> Self {
        Self {
            capacity: f64::from(capacity.max(1)),
            per_sec: per_sec.max(f64::MIN_POSITIVE),
            tokens: f64::from(capacity.max(1)),
            last: None,
        }
    }

    /// Spends one token, or says how long until one is available.
    pub fn take(&mut self, now: Instant) -> Result<(), Duration> {
        if let Some(last) = self.last {
            let elapsed = now.saturating_duration_since(last).as_secs_f64();
            self.tokens = (self.tokens + elapsed * self.per_sec).min(self.capacity);
        }
        self.last = Some(now);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Ok(())
        } else {
            let wait = (1.0 - self.tokens) / self.per_sec;
            Err(Duration::from_secs_f64(wait.min(3600.0)))
        }
    }
}

/// `Retry-After` for a refusal: whole seconds, rounded up, at least one.
pub fn retry_after_secs(wait: Duration) -> u64 {
    wait.as_secs_f64().ceil().max(1.0) as u64
}

/// The failed-authentication bucket and one bucket per known client.
pub struct RateLimiter {
    limits: RateLimits,
    failed: TokenBucket,
    clients: HashMap<String, TokenBucket>,
}

impl RateLimiter {
    pub fn new(limits: RateLimits) -> Self {
        Self {
            limits,
            failed: TokenBucket::new(limits.failed_burst, limits.failed_per_sec),
            clients: HashMap::new(),
        }
    }

    #[cfg(test)]
    pub fn limits(&self) -> RateLimits {
        self.limits
    }

    /// Replaces the limits and starts every bucket full again.
    #[cfg(test)]
    pub fn set_limits(&mut self, limits: RateLimits) {
        *self = Self::new(limits);
    }

    /// Counts one failed authentication. `Err` carries how long to ask the caller to wait.
    pub fn failed_auth(&mut self, now: Instant) -> Result<(), Duration> {
        self.failed.take(now)
    }

    /// Counts one call from `client_id`.
    pub fn client_call(&mut self, client_id: &str, now: Instant) -> Result<(), Duration> {
        let limits = self.limits;
        self.clients
            .entry(client_id.to_owned())
            .or_insert_with(|| TokenBucket::new(limits.client_burst, limits.client_per_sec))
            .take(now)
    }

    /// Forgets the buckets of clients that no longer exist, so the map cannot outgrow the list
    /// and a re-created client id starts fresh.
    pub fn retain_clients<'a>(&mut self, ids: impl IntoIterator<Item = &'a str>) {
        let live: std::collections::HashSet<&str> = ids.into_iter().collect();
        self.clients.retain(|id, _| live.contains(id.as_str()));
    }

    #[cfg(test)]
    pub fn tracked_clients(&self) -> usize {
        self.clients.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn a_bucket_lets_its_burst_through_then_refuses() {
        let start = t0();
        let mut bucket = TokenBucket::new(3, 1.0);
        for _ in 0..3 {
            assert!(bucket.take(start).is_ok());
        }
        let wait = bucket.take(start).unwrap_err();
        assert!(wait > Duration::from_millis(900) && wait <= Duration::from_secs(1));
    }

    #[test]
    fn a_bucket_refills_with_time_and_never_past_its_capacity() {
        let start = t0();
        let mut bucket = TokenBucket::new(2, 2.0);
        assert!(bucket.take(start).is_ok());
        assert!(bucket.take(start).is_ok());
        assert!(bucket.take(start).is_err());
        // Half a second at 2/s is one token.
        let later = start + Duration::from_millis(500);
        assert!(bucket.take(later).is_ok());
        assert!(bucket.take(later).is_err());
        // A long idle spell refills to the capacity and no further.
        let much_later = start + Duration::from_secs(3600);
        assert!(bucket.take(much_later).is_ok());
        assert!(bucket.take(much_later).is_ok());
        assert!(bucket.take(much_later).is_err());
    }

    #[test]
    fn a_clock_that_goes_backwards_does_not_refill_or_panic() {
        let start = t0();
        let mut bucket = TokenBucket::new(1, 1.0);
        assert!(bucket.take(start + Duration::from_secs(10)).is_ok());
        assert!(bucket.take(start).is_err());
    }

    #[test]
    fn retry_after_rounds_up_to_whole_seconds_and_is_never_zero() {
        assert_eq!(retry_after_secs(Duration::from_millis(1)), 1);
        assert_eq!(retry_after_secs(Duration::ZERO), 1);
        assert_eq!(retry_after_secs(Duration::from_millis(1001)), 2);
        assert_eq!(retry_after_secs(Duration::from_secs(3)), 3);
    }

    #[test]
    fn clients_have_their_own_buckets_and_failures_have_another() {
        let start = t0();
        let mut limiter = RateLimiter::new(RateLimits {
            failed_burst: 2,
            failed_per_sec: 1.0,
            client_burst: 2,
            client_per_sec: 1.0,
        });
        assert!(limiter.client_call("a", start).is_ok());
        assert!(limiter.client_call("a", start).is_ok());
        assert!(limiter.client_call("a", start).is_err());
        // Another client, and the failure bucket, are untouched.
        assert!(limiter.client_call("b", start).is_ok());
        assert!(limiter.failed_auth(start).is_ok());
        assert!(limiter.failed_auth(start).is_ok());
        assert!(limiter.failed_auth(start).is_err());
        assert!(limiter.client_call("b", start).is_ok());
    }

    #[test]
    fn removed_clients_lose_their_bucket() {
        let start = t0();
        let mut limiter = RateLimiter::new(RateLimits::default());
        limiter.client_call("a", start).unwrap();
        limiter.client_call("b", start).unwrap();
        assert_eq!(limiter.tracked_clients(), 2);
        limiter.retain_clients(["b"]);
        assert_eq!(limiter.tracked_clients(), 1);
        limiter.retain_clients(std::iter::empty());
        assert_eq!(limiter.tracked_clients(), 0);
    }

    #[test]
    fn setting_limits_starts_every_bucket_full() {
        let start = t0();
        let mut limiter = RateLimiter::new(RateLimits {
            failed_burst: 1,
            failed_per_sec: 0.001,
            client_burst: 1,
            client_per_sec: 0.001,
        });
        assert!(limiter.failed_auth(start).is_ok());
        assert!(limiter.failed_auth(start).is_err());
        limiter.set_limits(RateLimits {
            failed_burst: 5,
            ..RateLimits::default()
        });
        assert_eq!(limiter.limits().failed_burst, 5);
        assert!(limiter.failed_auth(start).is_ok());
    }
}
