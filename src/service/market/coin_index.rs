//! The top coins by market cap, read from CoinGecko once a day: artwork for the icon resolver and
//! names for the markets of exchanges that list none.

use crate::lock::lock;
use crate::model::market::asset_name::sanitize;
use crate::service::cache::lookup;
use crate::service::market::coingecko;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The lookup namespace the stored ranking lives in.
pub const NAMESPACE: &str = "coin_index";
const SNAPSHOT_KEY: &str = "top";
/// How long the persisted ranking is used without being fetched again.
const SNAPSHOT_TTL: Duration = Duration::from_secs(86_400);
/// Coins pulled into the ranking.
const MAX_COINS: usize = 250;
/// Minimum spacing between attempts: a failed refresh must not re-fire for every card.
const REFRESH_RETRY: Duration = Duration::from_secs(300);

/// One coin of the market-cap ranking, as stored.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Coin {
    id: String,
    symbol: String,
    name: String,
    image: String,
}

/// What one coin of the ranking is known by.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CoinInfo {
    /// Made fit to show; `None` when the ranking gave nothing that is.
    pub name: Option<String>,
    pub image: String,
}

/// The ranking, indexed by ticker symbol and by CoinGecko id, both lower-cased.
#[derive(Clone, Default)]
pub struct CoinIndex {
    by_symbol: HashMap<String, Arc<CoinInfo>>,
    by_id: HashMap<String, Arc<CoinInfo>>,
}

impl CoinIndex {
    /// The coins are in market-cap order, so the first coin of a symbol keeps it: "btc" is
    /// bitcoin, not batcat.
    fn from_coins(coins: &[Coin]) -> Self {
        let mut index = Self::default();
        for coin in coins {
            let info = Arc::new(CoinInfo {
                name: sanitize(&coin.name),
                image: coin.image.clone(),
            });
            index
                .by_symbol
                .entry(coin.symbol.to_lowercase())
                .or_insert_with(|| Arc::clone(&info));
            index.by_id.insert(coin.id.to_lowercase(), info);
        }
        index
    }

    /// The biggest coin trading under `symbol`, whatever its case.
    pub fn by_symbol(&self, symbol: &str) -> Option<&CoinInfo> {
        self.by_symbol.get(&symbol.to_lowercase()).map(Arc::as_ref)
    }

    pub fn by_id(&self, id: &str) -> Option<&CoinInfo> {
        self.by_id.get(&id.to_lowercase()).map(Arc::as_ref)
    }

    /// Whether nothing was ever read: a failed load.
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// An index over `(id, symbol, name)` coins in market-cap order, for tests.
    #[cfg(test)]
    pub fn of(coins: &[(&str, &str, &str)]) -> Self {
        let coins: Vec<Coin> = coins
            .iter()
            .map(|(id, symbol, name)| Coin {
                id: (*id).to_owned(),
                symbol: (*symbol).to_owned(),
                name: (*name).to_owned(),
                image: String::new(),
            })
            .collect();
        Self::from_coins(&coins)
    }
}

#[derive(Default)]
struct State {
    index: Arc<CoinIndex>,
    loaded: bool,
    last_attempt: Option<Instant>,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(Mutex::default)
}

/// Serialises refreshes: the first caller fetches, the rest wait on the lock and find the
/// ranking loaded.
fn refresh() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(Mutex::default)
}

/// Forgets the ranking held in memory (after the disk cache was cleared).
pub(in crate::service) fn clear() {
    *lock(state()) = State::default();
}

/// The ranking, and whether the fetch failed. Used from storage while it is under a day old,
/// fetched otherwise (once per launch, a failure retried no sooner than `REFRESH_RETRY` later),
/// and kept in storage for the next launch. A stored ranking of any age still stands in when the
/// fetch fails. Blocks: not for the frame loop.
pub fn load() -> (Arc<CoinIndex>, bool) {
    {
        let state = lock(state());
        if state.loaded {
            return (Arc::clone(&state.index), false);
        }
    }
    let _refreshing = lock(refresh());
    let stored = lookup::shared().get::<Vec<Coin>>(NAMESPACE, SNAPSHOT_KEY, SNAPSHOT_TTL);
    {
        let mut state = lock(state());
        if state.loaded {
            return (Arc::clone(&state.index), false);
        }
        if let Some(hit) = stored.as_ref().filter(|hit| hit.fresh) {
            state.index = Arc::new(CoinIndex::from_coins(&hit.value));
            state.loaded = true;
            return (Arc::clone(&state.index), false);
        }
        if state
            .last_attempt
            .is_some_and(|at| at.elapsed() < REFRESH_RETRY)
        {
            return (Arc::clone(&state.index), true);
        }
        state.last_attempt = Some(Instant::now());
    }
    let fetched = fetch_coins();
    let mut state = lock(state());
    let coins = match (fetched, stored) {
        (Some(coins), _) => {
            lookup::shared().put(NAMESPACE, SNAPSHOT_KEY, &coins);
            coins
        }
        (None, Some(stale)) => stale.value,
        (None, None) => return (Arc::clone(&state.index), true),
    };
    state.index = Arc::new(CoinIndex::from_coins(&coins));
    state.loaded = true;
    (Arc::clone(&state.index), false)
}

fn fetch_coins() -> Option<Vec<Coin>> {
    crate::service::net::rate_limit::coingecko().wait_for_slot();
    let url = format!(
        "{}/coins/markets?vs_currency=usd&order=market_cap_desc&per_page={MAX_COINS}&sparkline=false",
        coingecko::BASE_URL
    );
    let body =
        crate::service::net::http::get_text(&url, &[], crate::service::net::http::Retry::Transient)
            .ok()?;
    parse_coins(&body)
}

/// The coins of a `/coins/markets` response that have an id, a symbol and artwork.
fn parse_coins(body: &str) -> Option<Vec<Coin>> {
    let coins: Vec<Value> = serde_json::from_str(body).ok()?;
    let coins: Vec<Coin> = coins
        .iter()
        .filter_map(|coin| {
            let text = |field: &str| coin.get(field).and_then(Value::as_str);
            Some(Coin {
                id: text("id")?.to_owned(),
                symbol: text("symbol")?.to_owned(),
                name: text("name").unwrap_or_default().to_owned(),
                image: text("image")?.to_owned(),
            })
        })
        .collect();
    (!coins.is_empty()).then_some(coins)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ranking_is_indexed_by_symbol_and_id_and_the_biggest_coin_owns_a_symbol() {
        let body = r#"[
            {"id":"bitcoin","symbol":"btc","name":"Bitcoin","image":"https://x/bitcoin.png"},
            {"id":"sei-network","symbol":"sei","name":"Sei","image":"https://x/sei.png"},
            {"id":"batcat","symbol":"btc","name":"batcat","image":"https://x/batcat.png"},
            {"id":"broken","symbol":"zzz","name":"No art"}
        ]"#;
        let index = CoinIndex::from_coins(&parse_coins(body).expect("coins"));
        let btc = index.by_symbol("BTC").expect("btc");
        assert_eq!(
            (btc.name.as_deref(), btc.image.as_str()),
            (Some("Bitcoin"), "https://x/bitcoin.png")
        );
        let name = |id| index.by_id(id).and_then(|c| c.name.as_deref());
        assert_eq!(name("sei-network"), Some("Sei"));
        assert_eq!(name("batcat"), Some("batcat"));
        assert!(index.by_id("broken").is_none());
        assert!(parse_coins("[]").is_none());
        assert!(parse_coins("not json").is_none());
    }
}
