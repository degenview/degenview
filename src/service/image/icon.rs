//! Coin and company artwork, resolved the way the Swift app's `IconResolver` does: a chain of
//! sources, widest coverage last, with a positive and a negative cache.
//!
//! 1. the cached answer for `(source, ticker)`, positive or negative (a miss is remembered for
//!    a day, so a card that has none does not re-walk the chain every frame);
//! 2. a snapshot of the top coins by market cap, indexed by symbol *and* by coin id;
//! 3. a source-specific lookup (CoinGecko by coin id, DEXScreener by pair address);
//! 4. CoinGecko `/search`, which reaches every coin it lists;
//! 5. a static icon set, for long-established symbols the APIs could not place.
//!
//! Answers are persisted (`cache::lookup`, namespace `icon_url`): a relaunch knows each market's
//! artwork — and each clean miss — without walking the chain again, so `get_url` answers on the
//! first frame. A stored artwork older than `URL_TTL` is used as it is and re-resolved in the
//! background; a *failed* lookup is never stored. The image bytes behind a URL are cached
//! separately (`image::loader`).
//!
//! Equities (Alpaca) are kept out of every coin-oriented step: an absent logo must become a
//! monogram, never a plausible but wrong crypto match. Polymarket artwork is looked up by
//! token; the other prediction markets have none. Callers draw a monogram until `get_url` has an answer.

use crate::lock::lock;
use crate::model::DataSource;
use crate::service::cache::lookup::{self, Hit, LookupCache};
use crate::service::market::coin_index;
use crate::service::market::coingecko::BASE_URL;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const STATIC_CDN: &str =
    "https://raw.githubusercontent.com/spothq/cryptocurrency-icons/master/128/color";
const STOCK_CDN: &str = "https://raw.githubusercontent.com/nvstly/icons/main/ticker_icons";
const DEXSCREENER: &str = "https://api.dexscreener.com/latest/dex";

/// Persisted answers: `(source, ticker) → artwork URL`.
const URL_NAMESPACE: &str = "icon_url";
/// The lookup namespaces this module reads on its first frame; `cache::control::launch` loads
/// them before then.
pub const NAMESPACES: [&str; 2] = [URL_NAMESPACE, coin_index::NAMESPACE];
/// How long a found artwork is trusted before it is re-resolved in the background.
const URL_TTL: Duration = Duration::from_secs(30 * 86_400);
/// How long an exhausted lookup is remembered before the chain runs again.
const NEGATIVE_TTL: Duration = Duration::from_secs(86_400);
/// How soon a lookup that *failed* (offline, rate-limited) is tried again. A clean miss waits
/// out `NEGATIVE_TTL`; a failure says nothing about whether the coin has artwork.
const FAILURE_RETRY: Duration = Duration::from_secs(180);
/// Longest string still plausibly a ticker; anything longer is a contract address.
const MAX_SYMBOL_LENGTH: usize = 12;

#[derive(Default)]
struct Entry {
    url: Option<String>,
    resolving: bool,
    /// Not asked again before this: a day after a clean miss, a few minutes after a failure.
    retry_after: Option<Instant>,
    /// The persisted answer has been looked at.
    seeded: bool,
    /// `url` came from storage past `URL_TTL`: show it, but resolve again.
    revalidate: bool,
}

fn cache() -> &'static Mutex<HashMap<String, Entry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn key(source: DataSource, ticker: &str) -> String {
    format!("{}:{}", source.watchlist_slug(), ticker.to_lowercase())
}

/// Reads the persisted answer into a fresh entry, once.
fn seed(entry: &mut Entry, key: &str, store: &LookupCache) {
    entry.seeded = true;
    let stored = store.get::<Option<String>>(URL_NAMESPACE, key, URL_TTL);
    match stored {
        Some(Hit {
            value: Some(url),
            fresh,
            ..
        }) => {
            entry.url = Some(url);
            entry.revalidate = !fresh;
        }
        Some(Hit {
            value: None, age, ..
        }) if age < NEGATIVE_TTL => {
            entry.retry_after = Some(Instant::now() + (NEGATIVE_TTL - age));
        }
        _ => {}
    }
}

/// The cached icon URL for a market, or `None` if unresolved (callers draw a monogram).
pub fn get_url(source: DataSource, ticker: &str) -> Option<String> {
    let cache_key = key(source, ticker);
    let mut guard = cache().lock().ok()?;
    let entry = guard.entry(cache_key.clone()).or_default();
    if !entry.seeded {
        seed(entry, &cache_key, lookup::shared());
    }
    entry.url.clone()
}

/// Ensure an icon is resolving in the background; the URL appears via `get_url`. The base
/// asset is read off the ticker (`SEIUSDT` → `sei`).
pub fn request(source: DataSource, ticker: &str) {
    request_with_base(source, ticker, &base_symbol(source, ticker));
}

/// Like `request`, for callers that know the market's base asset already (a portfolio asset's
/// ticker, while its `ticker` is the provider's own id: a CoinGecko coin id, a DEX pair address).
pub fn request_with_base(source: DataSource, ticker: &str, base: &str) {
    let cache_key = key(source, ticker);
    {
        let mut guard = lock(cache());
        let entry = guard.entry(cache_key.clone()).or_default();
        if entry.resolving {
            return;
        }
        if !entry.seeded {
            seed(entry, &cache_key, lookup::shared());
        }
        if entry.url.is_some() && !entry.revalidate {
            return;
        }
        if entry
            .retry_after
            .is_some_and(|after| Instant::now() < after)
        {
            return;
        }
        entry.resolving = true;
    }

    let ticker = ticker.to_owned();
    let base = base.to_lowercase();
    std::thread::spawn(move || {
        let (url, failed) = resolve(source, &ticker, &base);
        if let Ok(mut guard) = cache().lock()
            && let Some(entry) = guard.get_mut(&cache_key)
        {
            finish(entry, &cache_key, url, failed, lookup::shared());
        }
    });
}

/// Records what the chain found. An entry that is gone — the cache was cleared while the lookup
/// ran — is not brought back by the answer.
fn finish(entry: &mut Entry, key: &str, url: Option<String>, failed: bool, store: &LookupCache) {
    entry.resolving = false;
    entry.revalidate = false;
    match url {
        Some(url) => {
            store.put(URL_NAMESPACE, key, &Some(&url));
            entry.url = Some(url);
        }
        // Re-resolving a stored artwork found nothing now: keep showing it.
        None if entry.url.is_some() => {}
        None => {
            entry.retry_after =
                Some(Instant::now() + if failed { FAILURE_RETRY } else { NEGATIVE_TTL });
            // Only a clean miss is an answer; a failure says nothing about the coin.
            if !failed {
                store.put(URL_NAMESPACE, key, &None::<String>);
            }
        }
    }
}

/// Forgets everything held in memory (after the disk cache was cleared). Markets on screen
/// resolve again on their next frame.
pub fn clear_memory() {
    if let Ok(mut guard) = cache().lock() {
        guard.clear();
    }
    if let Ok(mut lookups) = symbol_lookups().lock() {
        lookups.clear();
    }
    coin_index::clear();
}

/// The symbol reduced to its base asset, used by the symbol-keyed steps of the chain.
fn base_symbol(source: DataSource, symbol: &str) -> String {
    match source {
        DataSource::Binance
        | DataSource::Coinbase
        | DataSource::Okx
        | DataSource::Kraken
        | DataSource::Bybit
        | DataSource::Gate
        | DataSource::Mexc => source.base_asset(symbol).to_lowercase(),
        _ => symbol.to_lowercase(),
    }
}

// MARK: Chain

/// The artwork for a market and whether a step *failed* on the way (so the miss is not final).
fn resolve(source: DataSource, ticker: &str, base: &str) -> (Option<String>, bool) {
    // A Polymarket token names its own artwork; Kalshi has none.
    if source == DataSource::Polymarket {
        return match crate::service::wiring::venues()
            .polymarket
            .artwork_for_token(ticker)
        {
            Ok(url) => (url, false),
            Err(()) => (None, true),
        };
    }
    // The other prediction markets have no coin behind them, and a CoinMarketCap card is an index.
    if source.is_prediction_market() || source == DataSource::CoinMarketCap {
        return (None, false);
    }
    if source == DataSource::Alpaca {
        return (stock_icon(ticker), false);
    }
    let mut symbol = base.to_owned();
    let mut failed = false;

    // 1. Market snapshot: Binance/Coinbase base symbols and top-ranked coin ids.
    let (coins, coins_failed) = coin_index::load();
    failed |= coins_failed;
    let by_source = match source {
        DataSource::CoinGecko => coins.by_id(ticker).or_else(|| coins.by_symbol(&symbol)),
        _ => coins.by_symbol(&symbol),
    };
    if let Some(coin) = by_source {
        return (Some(coin.image.clone()), false);
    }

    // 2. Source-specific lookups.
    match source {
        DataSource::CoinGecko => match coingecko_icon_by_id(ticker) {
            Ok(Some(url)) => return (Some(url), false),
            Ok(None) => {}
            Err(()) => failed = true,
        },
        DataSource::DexScreener => match dex_pair_icon(ticker) {
            Ok(Some(pair)) => {
                if let Some(url) = pair.image {
                    return (Some(url), false);
                }
                // The pair handed back a real ticker, so the symbol-keyed steps below have
                // something usable instead of a contract address.
                if let Some(resolved) = pair.base_symbol
                    && !resolved.is_empty()
                {
                    symbol = resolved.to_lowercase();
                    if let Some(coin) = coins.by_symbol(&symbol) {
                        return (Some(coin.image.clone()), false);
                    }
                }
            }
            Ok(None) => {}
            Err(()) => failed = true,
        },
        _ => {}
    }

    if !is_plausible_symbol(&symbol) {
        return (None, failed);
    }
    // 3 and 4 are shared by every market of the same base asset.
    let (url, symbol_failed) = resolve_symbol(&symbol);
    (url, failed || symbol_failed)
}

/// Symbol-shaped enough to be worth spending a search on.
fn is_plausible_symbol(symbol: &str) -> bool {
    !symbol.is_empty()
        && symbol.chars().count() <= MAX_SYMBOL_LENGTH
        && symbol.chars().all(char::is_alphanumeric)
}

// MARK: Symbol lookups (search, static set)

enum Lookup {
    Pending,
    /// The answer, and when to ask again if it was none.
    Done(Option<String>, Instant),
}

fn symbol_lookups() -> &'static Mutex<HashMap<String, Lookup>> {
    static LOOKUPS: OnceLock<Mutex<HashMap<String, Lookup>>> = OnceLock::new();
    LOOKUPS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// CoinGecko search, then the static set, once per base symbol: a hundred markets of one coin
/// share a single search, the others wait for it. Also reports whether a step failed.
fn resolve_symbol(symbol: &str) -> (Option<String>, bool) {
    loop {
        {
            let Ok(mut lookups) = symbol_lookups().lock() else {
                return (None, true);
            };
            match lookups.get(symbol) {
                Some(Lookup::Done(url, _)) if url.is_some() => {
                    return (url.clone(), false);
                }
                Some(Lookup::Done(None, retry_after)) if Instant::now() < *retry_after => {
                    return (None, false);
                }
                Some(Lookup::Pending) => {}
                _ => {
                    lookups.insert(symbol.to_owned(), Lookup::Pending);
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    let searched = coingecko_search_icon(symbol);
    let found = match &searched {
        Ok(Some(url)) => Some(url.clone()),
        _ => static_icon(symbol),
    };
    // A failed search leaves the answer open: retried soon, not after a day.
    let failed = found.is_none() && searched.is_err();
    if let Ok(mut lookups) = symbol_lookups().lock() {
        let retry_after = Instant::now() + if failed { FAILURE_RETRY } else { NEGATIVE_TTL };
        lookups.insert(symbol.to_owned(), Lookup::Done(found.clone(), retry_after));
    }
    (found, failed)
}

/// CoinGecko `/search` reaches every listed coin. Prefers an exact symbol match and, among
/// those, the best-ranked coin, so "btc" resolves to bitcoin rather than a same-ticker copycat.
fn coingecko_search_icon(base: &str) -> Result<Option<String>, ()> {
    crate::service::net::rate_limit::coingecko().wait_for_slot();
    let url = format!("{BASE_URL}/search?query={base}");
    let body =
        crate::service::net::http::get_text(&url, &[], crate::service::net::http::Retry::Transient)
            .map_err(|_| ())?;
    let value: Value = serde_json::from_str(&body).map_err(|_| ())?;
    Ok(best_search_icon(&value, base))
}

fn best_search_icon(value: &Value, base: &str) -> Option<String> {
    let coins = value.get("coins")?.as_array()?;
    let exact: Vec<&Value> = coins
        .iter()
        .filter(|coin| {
            coin.get("symbol")
                .and_then(Value::as_str)
                .is_some_and(|symbol| symbol.eq_ignore_ascii_case(base))
        })
        .collect();
    let candidates: Vec<&Value> = if exact.is_empty() {
        coins.iter().collect()
    } else {
        exact
    };
    let best = candidates.into_iter().min_by_key(|coin| {
        coin.get("market_cap_rank")
            .and_then(Value::as_i64)
            .unwrap_or(i64::MAX)
    })?;
    best.get("large")
        .or_else(|| best.get("thumb"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// A plain file in a public repo: no API, no rate limit, 404 on miss. The 404 is checked here
/// so an unknown symbol never caches a broken image reference.
fn static_icon(symbol: &str) -> Option<String> {
    verified(format!("{STATIC_CDN}/{}.png", symbol.to_lowercase()))
}

/// Company artwork for Alpaca equities, keyed by uppercase exchange ticker.
fn stock_icon(ticker: &str) -> Option<String> {
    let ticker = ticker.trim().to_uppercase();
    if !is_plausible_symbol(&ticker) {
        return None;
    }
    verified(format!("{STOCK_CDN}/{ticker}.png"))
}

fn verified(url: String) -> Option<String> {
    (crate::service::net::http::head_status(&url).ok()? == 200).then_some(url)
}

// MARK: Source-specific

/// A coin's artwork by CoinGecko id, for ids outside the snapshot.
fn coingecko_icon_by_id(id: &str) -> Result<Option<String>, ()> {
    crate::service::net::rate_limit::coingecko().wait_for_slot();
    let url = format!(
        "{BASE_URL}/coins/markets?vs_currency=usd&ids={}&per_page=1&sparkline=false",
        id.to_lowercase()
    );
    let body =
        crate::service::net::http::get_text(&url, &[], crate::service::net::http::Retry::Transient)
            .map_err(|_| ())?;
    let coins: Vec<Value> = serde_json::from_str(&body).map_err(|_| ())?;
    Ok(coins
        .first()
        .and_then(|coin| coin.get("image"))
        .and_then(Value::as_str)
        .map(str::to_owned))
}

struct DexPairIcon {
    image: Option<String>,
    base_symbol: Option<String>,
}

/// DEXScreener's own artwork for a pair, and the real ticker behind its address.
fn dex_pair_icon(address: &str) -> Result<Option<DexPairIcon>, ()> {
    let query = address.trim();
    if query.is_empty() {
        return Ok(None);
    }
    let url = format!("{DEXSCREENER}/search?q={query}");
    let body =
        crate::service::net::http::get_text(&url, &[], crate::service::net::http::Retry::Transient)
            .map_err(|_| ())?;
    Ok(parse_dex_pair(&body))
}

fn parse_dex_pair(body: &str) -> Option<DexPairIcon> {
    let value: Value = serde_json::from_str(body).ok()?;
    let pairs = value.get("pairs")?.as_array()?;
    let first = pairs.first()?;
    Some(DexPairIcon {
        image: pairs
            .iter()
            .find_map(|pair| pair.pointer("/info/imageUrl").and_then(Value::as_str))
            .map(str::to_owned),
        base_symbol: first
            .pointer("/baseToken/symbol")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::service::cache::disk::DiskCache;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};

    const DAY_MS: i64 = 86_400_000;

    /// A lookup cache in memory with a clock the test moves.
    fn store() -> (LookupCache, Arc<AtomicI64>) {
        let disk: &'static DiskCache = Box::leak(Box::new(DiskCache::disabled()));
        let clock = Arc::new(AtomicI64::new(1_000 * DAY_MS));
        let reader = Arc::clone(&clock);
        let store = LookupCache::for_tests(disk, Box::new(move || reader.load(Ordering::Relaxed)));
        (store, clock)
    }

    fn seeded(store: &LookupCache) -> Entry {
        let mut entry = Entry::default();
        seed(&mut entry, "binance:btcusdt", store);
        entry
    }

    #[test]
    fn a_stored_artwork_is_known_on_the_first_look() {
        let (store, clock) = store();
        store.put(URL_NAMESPACE, "binance:btcusdt", &Some("https://x/btc.png"));
        let entry = seeded(&store);
        assert_eq!(entry.url.as_deref(), Some("https://x/btc.png"));
        assert!(!entry.revalidate && entry.seeded);

        // Past `URL_TTL` it still shows, and is looked up again behind it.
        clock.fetch_add(31 * DAY_MS, Ordering::Relaxed);
        let entry = seeded(&store);
        assert_eq!(entry.url.as_deref(), Some("https://x/btc.png"));
        assert!(entry.revalidate);
    }

    #[test]
    fn a_stored_miss_holds_for_a_day_then_the_chain_runs_again() {
        let (store, clock) = store();
        store.put(URL_NAMESPACE, "binance:btcusdt", &None::<String>);
        clock.fetch_add(DAY_MS / 2, Ordering::Relaxed);
        let entry = seeded(&store);
        assert!(entry.url.is_none());
        assert!(
            entry
                .retry_after
                .is_some_and(|after| after > Instant::now())
        );

        clock.fetch_add(DAY_MS, Ordering::Relaxed);
        let entry = seeded(&store);
        assert!(entry.url.is_none() && entry.retry_after.is_none());
    }

    #[test]
    fn nothing_stored_leaves_the_entry_to_be_resolved() {
        let (store, _) = store();
        let entry = seeded(&store);
        assert!(entry.url.is_none() && entry.retry_after.is_none() && !entry.revalidate);
    }

    #[test]
    fn a_found_artwork_is_stored_and_a_clean_miss_too_but_a_failure_is_not() {
        let (store, _) = store();
        let mut found = Entry::default();
        finish(
            &mut found,
            "k:found",
            Some("https://x/a.png".into()),
            false,
            &store,
        );
        assert_eq!(found.url.as_deref(), Some("https://x/a.png"));
        let hit = store.get::<Option<String>>(URL_NAMESPACE, "k:found", URL_TTL);
        assert_eq!(hit.unwrap().value.as_deref(), Some("https://x/a.png"));

        let mut missed = Entry::default();
        finish(&mut missed, "k:missed", None, false, &store);
        assert!(missed.retry_after.is_some());
        let hit = store.get::<Option<String>>(URL_NAMESPACE, "k:missed", URL_TTL);
        assert_eq!(hit.unwrap().value, None);

        let mut failed = Entry::default();
        finish(&mut failed, "k:failed", None, true, &store);
        assert!(failed.retry_after.is_some(), "retried soon");
        assert!(
            store
                .get::<Option<String>>(URL_NAMESPACE, "k:failed", URL_TTL)
                .is_none()
        );
    }

    #[test]
    fn re_resolving_a_stored_artwork_that_now_finds_nothing_keeps_showing_it() {
        let (store, _) = store();
        let mut entry = Entry {
            url: Some("https://x/old.png".into()),
            revalidate: true,
            resolving: true,
            ..Entry::default()
        };
        finish(&mut entry, "k:old", None, false, &store);
        assert_eq!(entry.url.as_deref(), Some("https://x/old.png"));
        assert!(!entry.revalidate && !entry.resolving);
        assert!(
            store
                .get::<Option<String>>(URL_NAMESPACE, "k:old", URL_TTL)
                .is_none(),
            "the stored artwork is not overwritten by a miss"
        );

        finish(
            &mut entry,
            "k:old",
            Some("https://x/new.png".into()),
            false,
            &store,
        );
        assert_eq!(entry.url.as_deref(), Some("https://x/new.png"));
    }

    #[test]
    fn the_first_look_does_not_resolve_what_is_already_stored() {
        // The global store is memory-only under test: this is the public path end to end.
        let cache_key = key(DataSource::Binance, "PERSISTEDTESTUSDT");
        lookup::shared().put(URL_NAMESPACE, &cache_key, &Some("https://x/p.png"));
        assert_eq!(
            get_url(DataSource::Binance, "PERSISTEDTESTUSDT").as_deref(),
            Some("https://x/p.png")
        );
        request(DataSource::Binance, "PERSISTEDTESTUSDT");
        let guard = cache().lock().unwrap();
        // (Another test may have cleared the map meanwhile; either way nothing is resolving.)
        assert!(
            !guard.get(&cache_key).is_some_and(|entry| entry.resolving),
            "no lookup was started"
        );
    }

    #[test]
    fn clearing_memory_forgets_what_was_resolved() {
        let cache_key = key(DataSource::Binance, "CLEARTESTUSDT");
        cache().lock().unwrap().insert(
            cache_key.clone(),
            Entry {
                url: Some("https://x/c.png".into()),
                seeded: true,
                ..Entry::default()
            },
        );
        clear_memory();
        assert!(!cache().lock().unwrap().contains_key(&cache_key));
    }

    #[test]
    fn binance_base_drops_any_known_quote() {
        for (symbol, base) in [
            ("BTCUSDT", "btc"),
            ("XRPBTC", "xrp"),
            ("SOLBNB", "sol"),
            ("ETHFDUSD", "eth"),
            ("SEIUSDT", "sei"),
        ] {
            assert_eq!(base_symbol(DataSource::Binance, symbol), base);
        }
        assert_eq!(base_symbol(DataSource::Coinbase, "SEI-USD"), "sei");
        assert_eq!(base_symbol(DataSource::Okx, "SEI-USDT"), "sei");
        assert_eq!(base_symbol(DataSource::Kraken, "SEI/USD"), "sei");
        assert_eq!(base_symbol(DataSource::Bybit, "SEIUSDT"), "sei");
        assert_eq!(base_symbol(DataSource::Gate, "SEI_USDT"), "sei");
        assert_eq!(base_symbol(DataSource::Mexc, "SEIUSDT"), "sei");
    }

    #[test]
    fn search_prefers_an_exact_symbol_and_the_best_rank() {
        let value: Value = serde_json::from_str(
            r#"{"coins":[
                {"symbol":"SEIX","large":"https://x/seix.png","market_cap_rank":3},
                {"symbol":"SEI","large":"https://x/copy.png","market_cap_rank":900},
                {"symbol":"SEI","large":"https://x/sei.png","market_cap_rank":120},
                {"symbol":"SEI","thumb":"https://x/none.png"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(
            best_search_icon(&value, "sei").as_deref(),
            Some("https://x/sei.png")
        );
        // With no exact symbol the best-ranked coin of any symbol answers.
        let loose: Value = serde_json::from_str(
            r#"{"coins":[{"symbol":"AAA","large":"https://x/a.png","market_cap_rank":50}]}"#,
        )
        .unwrap();
        assert_eq!(
            best_search_icon(&loose, "zzz").as_deref(),
            Some("https://x/a.png")
        );
        assert!(best_search_icon(&serde_json::json!({"coins": []}), "sei").is_none());
    }

    #[test]
    fn a_dex_pair_gives_its_artwork_and_the_real_ticker() {
        let body = r#"{"pairs":[
            {"baseToken":{"symbol":"MCADE"},"info":null},
            {"baseToken":{"symbol":"MCADE"},"info":{"imageUrl":"https://x/mcade.png"}}
        ]}"#;
        let pair = parse_dex_pair(body).expect("pair");
        assert_eq!(pair.image.as_deref(), Some("https://x/mcade.png"));
        assert_eq!(pair.base_symbol.as_deref(), Some("MCADE"));
        assert!(parse_dex_pair(r#"{"pairs":[]}"#).is_none());
        assert!(parse_dex_pair(r#"{"pairs":null}"#).is_none());
    }

    #[test]
    fn only_ticker_shaped_strings_are_searched() {
        assert!(is_plausible_symbol("sei"));
        assert!(is_plausible_symbol("mcade"));
        assert!(!is_plausible_symbol(""));
        assert!(!is_plausible_symbol("5zpyutju9ee6jfymdgok"));
        assert!(!is_plausible_symbol("so-l"));
    }
}
