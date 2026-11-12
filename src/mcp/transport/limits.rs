//! Rate limits for the MCP server: one bucket for failed authentications and one per client.
//!
//! Failures share one bucket, and the bucket only ever slows *bad* tokens down: a valid token is
//! never refused because of failures. Calls from a valid client draw from that client's own
//! bucket, so one runaway agent cannot starve another.
//!
//! A request from another machine has an address, and its failures also draw from a bucket of that
//! address, taken first: one host guessing tokens is cut off by its own bucket without spending the
//! shared one, so it cannot slow down a local client's mistakes. Requests from this machine have no
//! bucket of their own (they were all "127.0.0.1" and still share the one bucket). The table of
//! addresses is bounded; an address that does not fit is judged by the shared bucket alone.
//!
//! The buckets take the current time as an argument, so tests drive them with made-up instants
//! and never sleep.

use crate::model::mcp::Peer;
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

/// Failed authentications let through at once before the rest are answered 429.
pub const FAILED_AUTH_BURST: u32 = 20;
/// Failed authentications let through per second once the burst is spent.
pub const FAILED_AUTH_PER_SEC: f64 = 2.0;
/// Failed authentications one remote address may make at once before it is answered 429.
pub const PEER_FAILED_BURST: u32 = 10;
/// Failed authentications per second one remote address keeps getting once its burst is spent.
pub const PEER_FAILED_PER_SEC: f64 = 1.0;
/// Remote addresses tracked at once. A flood from more addresses than this falls back to the
/// shared bucket for the surplus.
pub const MAX_TRACKED_PEERS: usize = 1024;
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

    /// Whether the bucket has refilled completely, so forgetting it changes nothing.
    fn is_full(&self, now: Instant) -> bool {
        let elapsed = self.last.map_or(f64::INFINITY, |last| {
            now.saturating_duration_since(last).as_secs_f64()
        });
        self.tokens + elapsed * self.per_sec >= self.capacity
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
    /// Failures per remote address.
    peers: HashMap<IpAddr, TokenBucket>,
    clients: HashMap<String, TokenBucket>,
}

impl RateLimiter {
    pub fn new(limits: RateLimits) -> Self {
        Self {
            limits,
            failed: TokenBucket::new(limits.failed_burst, limits.failed_per_sec),
            peers: HashMap::new(),
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

    /// Counts one failed authentication from `peer` (`None`: unknown). `Err` carries how long to
    /// ask the caller to wait. A remote address is judged by its own bucket first, and the shared
    /// bucket is spent only by what that lets through.
    pub fn failed_auth(&mut self, peer: Option<Peer>, now: Instant) -> Result<(), Duration> {
        // This machine (and a peer the server could not name) share the one bucket.
        let remote = peer.filter(|peer| !peer.local).map(|peer| peer.ip);
        if let Some(ip) = remote
            && let Some(bucket) = self.peer_bucket(ip, now)
        {
            bucket.take(now)?;
        }
        self.failed.take(now)
    }

    fn peer_bucket(&mut self, ip: IpAddr, now: Instant) -> Option<&mut TokenBucket> {
        if !self.peers.contains_key(&ip) && self.peers.len() >= MAX_TRACKED_PEERS {
            // Forget the addresses that have been quiet long enough to be back at full strength.
            self.peers.retain(|_, bucket| !bucket.is_full(now));
            if self.peers.len() >= MAX_TRACKED_PEERS {
                return None;
            }
        }
        Some(
            self.peers
                .entry(ip)
                .or_insert_with(|| TokenBucket::new(PEER_FAILED_BURST, PEER_FAILED_PER_SEC)),
        )
    }

    #[cfg(test)]
    pub fn tracked_peers(&self) -> usize {
        self.peers.len()
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
        assert!(limiter.failed_auth(None, start).is_ok());
        assert!(limiter.failed_auth(None, start).is_ok());
        assert!(limiter.failed_auth(None, start).is_err());
        assert!(limiter.client_call("b", start).is_ok());
    }

    fn remote(text: &str) -> Option<Peer> {
        Some(Peer::from_ip(text.parse().unwrap()))
    }

    #[test]
    fn a_remote_address_is_cut_off_by_its_own_bucket_before_the_shared_one() {
        let start = t0();
        let mut limiter = RateLimiter::new(RateLimits::default());
        let guesser = remote("203.0.113.9");
        for _ in 0..PEER_FAILED_BURST {
            assert!(limiter.failed_auth(guesser, start).is_ok());
        }
        // Its own bucket is spent, so it is refused here...
        assert!(limiter.failed_auth(guesser, start).is_err());
        assert!(limiter.failed_auth(guesser, start).is_err());
        // ...and those refusals took nothing from the shared bucket: only the first
        // PEER_FAILED_BURST failures did, so this machine still has the rest of its own burst.
        for _ in 0..(FAILED_AUTH_BURST - PEER_FAILED_BURST) {
            assert!(limiter.failed_auth(None, start).is_ok());
        }
        assert!(limiter.failed_auth(None, start).is_err());
        // Another address starts with a full bucket of its own.
        assert!(
            limiter.failed_auth(remote("198.51.100.4"), start).is_err(),
            "shared is spent"
        );
    }

    #[test]
    fn this_machine_has_no_bucket_of_its_own_and_a_mapped_address_is_the_same_address() {
        let start = t0();
        let mut limiter = RateLimiter::new(RateLimits::default());
        for peer in ["127.0.0.1", "::1", "::ffff:127.0.0.1"] {
            limiter.failed_auth(remote(peer), start).unwrap();
        }
        assert_eq!(limiter.tracked_peers(), 0);
        limiter
            .failed_auth(remote("::ffff:203.0.113.9"), start)
            .unwrap();
        limiter.failed_auth(remote("203.0.113.9"), start).unwrap();
        assert_eq!(limiter.tracked_peers(), 1);
    }

    #[test]
    fn a_peer_on_this_machines_own_address_is_this_machine_here_too() {
        let start = t0();
        let mut limiter = RateLimiter::new(RateLimits::default());
        let own: IpAddr = "192.168.1.20".parse().unwrap();
        // A client on this Mac using the LAN address the server listens on is not a remote host:
        // no bucket of its own, whatever it does.
        for _ in 0..(PEER_FAILED_BURST + 5) {
            let _ = limiter.failed_auth(Some(Peer::of_connection(own, own)), start);
        }
        assert_eq!(limiter.tracked_peers(), 0);
    }

    #[test]
    fn the_table_of_addresses_is_bounded_and_forgets_quiet_ones() {
        let start = t0();
        let mut limiter = RateLimiter::new(RateLimits {
            failed_burst: u32::MAX,
            failed_per_sec: 1e9,
            ..RateLimits::default()
        });
        let address = |n: usize| remote(&format!("10.{}.{}.1", n / 256, n % 256));
        for n in 0..MAX_TRACKED_PEERS {
            limiter.failed_auth(address(n), start).unwrap();
        }
        assert_eq!(limiter.tracked_peers(), MAX_TRACKED_PEERS);
        // Everyone just failed once, so none is back to full: a new address is not tracked...
        limiter
            .failed_auth(address(MAX_TRACKED_PEERS), start)
            .unwrap();
        assert_eq!(limiter.tracked_peers(), MAX_TRACKED_PEERS);
        // ...until time has passed and the quiet ones are forgotten to make room.
        let later = start + Duration::from_secs(3600);
        limiter
            .failed_auth(address(MAX_TRACKED_PEERS), later)
            .unwrap();
        assert_eq!(limiter.tracked_peers(), 1);
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
        assert!(limiter.failed_auth(None, start).is_ok());
        assert!(limiter.failed_auth(None, start).is_err());
        limiter.set_limits(RateLimits {
            failed_burst: 5,
            ..RateLimits::default()
        });
        assert_eq!(limiter.limits().failed_burst, 5);
        assert!(limiter.failed_auth(None, start).is_ok());
    }
}
