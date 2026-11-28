//! Stand-ins for the ports, so a venue's tests run on recorded answers: no network, no Keychain,
//! no disk, and a clock that stands still.

use super::cache::{CacheStore, Cached};
use super::clock::Clock;
use super::context::MarketContext;
use super::credentials::Credentials;
use super::http::{Error, Http, Pace, Request, Retry};
use crate::lock::lock;
use crate::model::market::oanda::OandaCredentials;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A request as the venue made it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sent {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub retry: Retry,
    pub max_body_bytes: Option<u64>,
    pub pace: Option<Pace>,
}

struct Route {
    fragment: String,
    replies: VecDeque<Result<String, Error>>,
}

/// Answers by what the address contains. A route is tried in the order it was added, and each
/// reply is given once: a request nothing is left for fails like a dropped connection, and shows
/// in [`Self::urls`].
#[derive(Default)]
pub struct FakeHttp {
    routes: Mutex<Vec<Route>>,
    sent: Mutex<Vec<Sent>>,
}

impl FakeHttp {
    /// The next request whose address contains `fragment` is answered `body`.
    pub fn on(&self, fragment: &str, body: &str) -> &Self {
        self.reply(fragment, Ok(body.to_owned()))
    }

    /// The next request whose address contains `fragment` fails with `error`.
    pub fn fail(&self, fragment: &str, error: Error) -> &Self {
        self.reply(fragment, Err(error))
    }

    fn reply(&self, fragment: &str, reply: Result<String, Error>) -> &Self {
        let mut routes = lock(&self.routes);
        match routes.iter_mut().find(|route| route.fragment == fragment) {
            Some(route) => route.replies.push_back(reply),
            None => routes.push(Route {
                fragment: fragment.to_owned(),
                replies: VecDeque::from([reply]),
            }),
        }
        self
    }

    pub fn sent(&self) -> Vec<Sent> {
        lock(&self.sent).clone()
    }

    pub fn urls(&self) -> Vec<String> {
        self.sent().into_iter().map(|sent| sent.url).collect()
    }
}

impl Http for FakeHttp {
    fn get(&self, request: &Request<'_>) -> Result<String, Error> {
        lock(&self.sent).push(Sent {
            url: request.url.to_owned(),
            headers: request
                .headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            retry: request.retry,
            max_body_bytes: request.max_body_bytes,
            pace: request.pace,
        });
        lock(&self.routes)
            .iter_mut()
            .filter(|route| request.url.contains(&route.fragment))
            .find_map(|route| route.replies.pop_front())
            .unwrap_or_else(|| {
                Err(Error::Network(format!(
                    "no reply recorded for {}",
                    request.url
                )))
            })
    }
}

/// A clock that stands where it was put.
pub struct FixedClock(AtomicI64);

impl FixedClock {
    pub fn at(now_ms: i64) -> Self {
        Self(AtomicI64::new(now_ms))
    }

    pub fn advance(&self, by: Duration) {
        self.0.fetch_add(by.as_millis() as i64, Ordering::SeqCst);
    }
}

impl Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// The keys the user has saved, and how often a venue asked for each.
#[derive(Default)]
pub struct FakeCredentials {
    alpaca: Mutex<Option<(String, String)>>,
    oanda: Mutex<Option<OandaCredentials>>,
    cmc: Mutex<Option<String>>,
    alpaca_lookups: AtomicUsize,
    oanda_lookups: AtomicUsize,
    cmc_lookups: AtomicUsize,
}

impl FakeCredentials {
    pub fn save_alpaca(&self, key_id: &str, secret_key: &str) {
        *lock(&self.alpaca) = Some((key_id.to_owned(), secret_key.to_owned()));
    }

    pub fn save_oanda(&self, login: OandaCredentials) {
        *lock(&self.oanda) = Some(login);
    }

    pub fn save_cmc(&self, key: &str) {
        *lock(&self.cmc) = Some(key.to_owned());
    }

    pub fn alpaca_lookups(&self) -> usize {
        self.alpaca_lookups.load(Ordering::SeqCst)
    }

    pub fn oanda_lookups(&self) -> usize {
        self.oanda_lookups.load(Ordering::SeqCst)
    }

    pub fn cmc_lookups(&self) -> usize {
        self.cmc_lookups.load(Ordering::SeqCst)
    }
}

impl Credentials for FakeCredentials {
    fn alpaca(&self) -> Option<(String, String)> {
        self.alpaca_lookups.fetch_add(1, Ordering::SeqCst);
        lock(&self.alpaca).clone()
    }

    fn oanda(&self) -> Option<OandaCredentials> {
        self.oanda_lookups.fetch_add(1, Ordering::SeqCst);
        lock(&self.oanda).clone()
    }

    fn cmc(&self) -> Option<String> {
        self.cmc_lookups.fetch_add(1, Ordering::SeqCst);
        lock(&self.cmc).clone()
    }
}

/// Lookups kept in memory, aged by the clock they are built with. Two catalogs over one of these
/// are a launch and a relaunch.
pub struct MemoryCache {
    clock: Arc<FixedClock>,
    entries: Mutex<HashMap<(String, String), (i64, Value)>>,
}

impl MemoryCache {
    pub fn new(clock: Arc<FixedClock>) -> Self {
        Self {
            clock,
            entries: Mutex::default(),
        }
    }
}

impl CacheStore for MemoryCache {
    fn get_value(&self, namespace: &str, key: &str, ttl: Duration) -> Option<Cached<Value>> {
        let (at_ms, value) = lock(&self.entries)
            .get(&(namespace.to_owned(), key.to_owned()))?
            .clone();
        let age = Duration::from_millis(
            self.clock
                .now_ms()
                .saturating_sub(at_ms)
                .max(0)
                .unsigned_abs(),
        );
        Some(Cached {
            value,
            age,
            fresh: age < ttl,
        })
    }

    fn put_value(&self, namespace: &str, key: &str, value: Value) {
        lock(&self.entries).insert(
            (namespace.to_owned(), key.to_owned()),
            (self.clock.now_ms(), value),
        );
    }
}

/// The four ports as fakes, and the context a venue is built from.
pub struct Rig {
    pub http: Arc<FakeHttp>,
    pub clock: Arc<FixedClock>,
    pub credentials: Arc<FakeCredentials>,
    pub cache: Arc<MemoryCache>,
}

impl Rig {
    /// Noon UTC on 2026-11-16.
    pub const NOON: i64 = 1_794_830_400_000;

    pub fn new() -> Self {
        let clock = Arc::new(FixedClock::at(Self::NOON));
        Self {
            http: Arc::default(),
            cache: Arc::new(MemoryCache::new(Arc::clone(&clock))),
            clock,
            credentials: Arc::default(),
        }
    }

    pub fn context(&self) -> MarketContext {
        MarketContext {
            http: self.http.clone(),
            clock: self.clock.clone(),
            credentials: self.credentials.clone(),
            cache: self.cache.clone(),
        }
    }
}
