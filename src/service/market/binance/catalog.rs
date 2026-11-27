//! Binance's lists of tradable markets, one per market kind, and the search over them.

use super::endpoints::{Endpoints, endpoints};
use crate::model::DataSource;
use crate::model::InstrumentId;
use crate::model::exchange::Market;
use crate::model::market::binance::join_market;
use crate::service::market::search_rank::normalize;
use crate::service::{ProviderError, SearchResult, rank_results};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The most hits one market kind contributes.
const HITS_PER_MARKET: usize = 20;

/// The hits of both markets, ranked together. A list that cannot be had does not cost the hits of
/// the other; only both failing is an error.
pub(super) fn search(query: &str, needle: &str) -> Result<Vec<SearchResult>, ProviderError> {
    merge(
        query,
        [Market::Spot, Market::Perpetual].map(|market| {
            let endpoints = endpoints(market);
            let pairs = endpoints.catalog.get(|| download_pairs(endpoints))?;
            let hits = matching_pairs(&pairs, needle, market);
            Ok(rank_results(hits, query)
                .into_iter()
                .take(HITS_PER_MARKET)
                .collect())
        }),
    )
}

fn merge(
    query: &str,
    answers: [Result<Vec<SearchResult>, ProviderError>; 2],
) -> Result<Vec<SearchResult>, ProviderError> {
    let mut hits = Vec::new();
    let mut errors = Vec::new();
    for answer in answers {
        match answer {
            Ok(found) => hits.extend(found),
            Err(error) => errors.push(error),
        }
    }
    match errors.into_iter().next() {
        Some(error) if hits.is_empty() => Err(error),
        _ => Ok(rank_results(hits, query)),
    }
}

/// The listed markets whose name contains `needle` (letters and digits, upper case). A
/// perpetual's name ends in its suffix, so `btcusdt.p` finds it.
fn matching_pairs(pairs: &[Pair], needle: &str, market: Market) -> Vec<SearchResult> {
    pairs
        .iter()
        .filter_map(|pair| {
            let symbol = join_market(&pair.symbol, market);
            normalize(&symbol).contains(needle).then(|| {
                SearchResult::new(DataSource::Binance, symbol.clone(), symbol)
                    .pair(pair.base.clone(), pair.quote.clone())
            })
        })
        .collect()
}

/// One tradable market from `exchangeInfo`: all the search needs of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(in crate::service) struct Pair {
    symbol: String,
    base: String,
    quote: String,
}

/// Forgets the lists held in memory (after the disk cache was cleared).
pub(in crate::service) fn clear_catalog() {
    for market in [Market::Spot, Market::Perpetual] {
        endpoints(market).catalog.clear_memory();
    }
}

/// What this one download may read: six times the spot list of today, so listings can grow
/// without breaking search. The shared 8 MiB limit stays for every other call.
const EXCHANGE_INFO_MAX_BYTES: u64 = 16 * 1024 * 1024;

fn download_pairs(endpoints: &Endpoints) -> Result<Vec<Pair>, ProviderError> {
    download_pairs_with(endpoints, |url, max_bytes| {
        crate::service::net::http::get_text_with_limit(
            url,
            &[],
            crate::service::net::http::Retry::Transient,
            max_bytes,
        )
    })
}

/// `download_pairs` with the transport passed in, so a test sees the request without a network.
fn download_pairs_with(
    endpoints: &Endpoints,
    get: impl FnOnce(&str, u64) -> Result<String, crate::service::net::http::Error>,
) -> Result<Vec<Pair>, ProviderError> {
    let body = get(endpoints.exchange_info_url, EXCHANGE_INFO_MAX_BYTES)?;
    parse_pairs(&body, endpoints.lists)
}

pub(super) fn is_trading(entry: &Value) -> bool {
    entry.get("status").and_then(Value::as_str) == Some("TRADING")
}

/// A trading perpetual, of a coin or of a traditional asset, not a dated future. One whose name
/// the app would read as something else (a quote it does not know) is left out
/// rather than offered and then asked for under another name.
pub(super) fn is_trading_perpetual(entry: &Value) -> bool {
    let perpetual = matches!(
        entry.get("contractType").and_then(Value::as_str),
        Some("PERPETUAL" | "TRADIFI_PERPETUAL")
    );
    let named = entry
        .get("symbol")
        .and_then(Value::as_str)
        .is_some_and(|pair| {
            let symbol = join_market(pair, Market::Perpetual);
            InstrumentId::new(DataSource::Binance, symbol.as_str()).canonical_symbol() == symbol
        });
    perpetual && named && is_trading(entry)
}

fn parse_pairs(body: &str, lists: fn(&Value) -> bool) -> Result<Vec<Pair>, ProviderError> {
    let value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    Ok(value
        .get("symbols")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|symbol| lists(symbol))
        .filter_map(|symbol| {
            Some(Pair {
                symbol: symbol.get("symbol")?.as_str()?.to_owned(),
                base: symbol.get("baseAsset")?.as_str()?.to_owned(),
                quote: symbol.get("quoteAsset")?.as_str()?.to_owned(),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pair_is_found_however_it_is_separated() {
        let pairs = vec![
            Pair {
                symbol: "BTCUSDT".into(),
                base: "BTC".into(),
                quote: "USDT".into(),
            },
            Pair {
                symbol: "ETHBTC".into(),
                base: "ETH".into(),
                quote: "BTC".into(),
            },
        ];
        for query in ["btcusdt", "BTC-USDT", "btc/usdt", "BTC USDT"] {
            let found = matching_pairs(&pairs, &normalize(query), Market::Spot);
            assert_eq!(found.len(), 1, "{query}");
            assert_eq!(found[0].symbol, "BTCUSDT");
        }
        assert_eq!(matching_pairs(&pairs, "BTC", Market::Spot).len(), 2);
    }

    #[test]
    fn the_market_list_asks_for_trading_pairs_only_under_its_own_limit() {
        use crate::service::net::http::Error;
        let body = r#"{"symbols":[
            {"symbol":"BTCUSDT","status":"TRADING","baseAsset":"BTC","quoteAsset":"USDT"}
        ]}"#;
        let mut asked = None;
        let pairs = download_pairs_with(endpoints(Market::Spot), |url, max_bytes| {
            asked = Some((url.to_owned(), max_bytes));
            Ok(body.to_owned())
        })
        .expect("pairs");
        assert_eq!(pairs.len(), 1);

        let (url, max_bytes) = asked.expect("one request");
        assert!(url.starts_with("https://api.binance.com/api/v3/exchangeInfo?"));
        assert!(url.contains("symbolStatus=TRADING"));
        assert!(url.contains("showPermissionSets=false"));
        // Room for the list to grow past today's 2.6 MB, but not the shared limit's job.
        assert!(max_bytes > 8 * 1024 * 1024);
        assert!(max_bytes <= 32 * 1024 * 1024);

        // A too-large answer is still an error and not a partial list.
        let too_big = download_pairs_with(endpoints(Market::Spot), |_, max_bytes| {
            Err(Error::BodyTooLarge(max_bytes))
        });
        assert_eq!(
            too_big.expect_err("fails"),
            ProviderError::from(Error::BodyTooLarge(EXCHANGE_INFO_MAX_BYTES))
        );
    }

    #[test]
    fn the_catalog_keeps_only_trading_markets_with_all_their_fields() {
        let body = r#"{"symbols":[
            {"symbol":"BTCUSDT","status":"TRADING","baseAsset":"BTC","quoteAsset":"USDT"},
            {"symbol":"LUNAUSDT","status":"BREAK","baseAsset":"LUNA","quoteAsset":"USDT"},
            {"symbol":"NOQUOTE","status":"TRADING","baseAsset":"X"}
        ]}"#;
        let pairs = parse_pairs(body, is_trading).expect("pairs");
        assert_eq!(pairs.len(), 1);
        assert_eq!(
            (
                pairs[0].symbol.as_str(),
                pairs[0].base.as_str(),
                pairs[0].quote.as_str()
            ),
            ("BTCUSDT", "BTC", "USDT")
        );
        assert!(parse_pairs("not json", is_trading).is_err());
        assert!(
            parse_pairs("{}", is_trading)
                .expect("no symbols")
                .is_empty()
        );
    }

    #[test]
    fn a_perpetual_is_found_and_named_with_its_suffix() {
        let pairs = vec![Pair {
            symbol: "BTCUSDT".into(),
            base: "BTC".into(),
            quote: "USDT".into(),
        }];
        for query in ["btc", "btcusdt", "BTCUSDT.P", "btc/usdt"] {
            let found = matching_pairs(&pairs, &normalize(query), Market::Perpetual);
            assert_eq!(found.len(), 1, "{query}");
            assert_eq!(found[0].full_symbol, "BTCUSDT.P");
            assert_eq!(found[0].market(), Market::Perpetual);
            assert_eq!(found[0].base_asset(), "BTC");
        }
        assert!(matching_pairs(&pairs, "BTCUSDTP", Market::Spot).is_empty());
    }

    #[test]
    fn the_futures_list_keeps_perpetuals_the_app_can_name() {
        let body = r#"{"symbols":[
            {"symbol":"BTCUSDT","status":"TRADING","contractType":"PERPETUAL","baseAsset":"BTC","quoteAsset":"USDT"},
            {"symbol":"TSLAUSDT","status":"TRADING","contractType":"TRADIFI_PERPETUAL","baseAsset":"TSLA","quoteAsset":"USDT"},
            {"symbol":"SUSDT","status":"TRADING","contractType":"PERPETUAL","baseAsset":"S","quoteAsset":"USDT"},
            {"symbol":"BTCUSDT_261225","status":"TRADING","contractType":"CURRENT_QUARTER","baseAsset":"BTC","quoteAsset":"USDT"},
            {"symbol":"LUNAUSDT","status":"SETTLING","contractType":"PERPETUAL","baseAsset":"LUNA","quoteAsset":"USDT"},
            {"symbol":"BTCXYZ","status":"TRADING","contractType":"PERPETUAL","baseAsset":"BTC","quoteAsset":"XYZ"}
        ]}"#;
        let pairs = parse_pairs(body, is_trading_perpetual).expect("pairs");
        let symbols: Vec<&str> = pairs.iter().map(|pair| pair.symbol.as_str()).collect();
        assert_eq!(symbols, ["BTCUSDT", "TSLAUSDT", "SUSDT"]);
    }

    #[test]
    fn one_list_failing_keeps_the_other_markets_hits() {
        let hit = SearchResult::new(DataSource::Binance, "BTCUSDT", "BTCUSDT").pair("BTC", "USDT");
        let down = || Err(ProviderError::other("down"));
        assert_eq!(
            merge("btc", [Ok(vec![hit.clone()]), down()]).expect("hits"),
            [hit]
        );
        assert!(merge("btc", [down(), down()]).is_err());
        assert!(merge("btc", [Ok(Vec::new()), down()]).is_err());
    }
}
