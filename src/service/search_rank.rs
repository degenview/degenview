//! Which search hit comes first. One rule serves a single provider's list and the merged list of
//! every provider the symbol picker asks, so a market sorts the same way in both.
//!
//! A query names an asset ("btc"), a market ("BTCUSD", "BTC/USD", "BTC-USD" are one query) or a
//! company ("apple"). Hits are placed in tiers, best first:
//!
//! 0. the market itself, or the asset in a dollar quote (every `BTC` market priced in USD, USDT,
//!    USDC...), on every exchange that lists it;
//! 1. a market whose name begins with the query where the query is its asset plus the start of a
//!    dollar quote (`BTCUSD` finds `BTCUSDT`);
//! 2. the asset in any other quote (`BTCTRY`, `BTCEUR`, `BTCETH`);
//! 3. an asset or company that begins with the query (`BTCB`, `Apple Inc.`);
//! 4. anything else that contains it;
//! 5. the rest.
//!
//! Inside a tier the order is the order the providers were asked in, then the quote
//! (the exchange's main dollar quote, then USD, USDT, USDC), then the provider's own order.

use super::SearchResult;
use crate::model::DataSource;

/// Quotes that read as US dollars, in the order they sort.
const DOLLAR_QUOTES: [&str; 9] = [
    "USD", "USDT", "USDC", "USDS", "FDUSD", "BUSD", "TUSD", "USDP", "DAI",
];

/// Whether `quote` is a dollar or a dollar-pegged coin.
pub fn is_dollar_quote(quote: &str) -> bool {
    dollar_rank(quote).is_some()
}

fn dollar_rank(quote: &str) -> Option<usize> {
    DOLLAR_QUOTES
        .iter()
        .position(|dollar| dollar.eq_ignore_ascii_case(quote))
}

/// Letters and digits only, upper-cased: `btc/usd`, `BTC-USD` and `BTC USD` are all `BTCUSD`.
pub fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_uppercase)
        .collect()
}

/// One provider's hits, best first. Hits that rank alike keep the provider's order.
pub(crate) fn rank_results(results: Vec<SearchResult>, query: &str) -> Vec<SearchResult> {
    merge_ranked(vec![results], query)
}

/// The hits of several providers as one list. `groups` are in the order the providers were asked,
/// which breaks ties between exchanges that list the same market.
pub fn merge_ranked(groups: Vec<Vec<SearchResult>>, query: &str) -> Vec<SearchResult> {
    let needle = normalize(query);
    let mut keyed: Vec<(Key, SearchResult)> = groups
        .into_iter()
        .enumerate()
        .flat_map(|(group, results)| {
            results
                .into_iter()
                .enumerate()
                .map(move |(offset, result)| (group, offset, result))
        })
        .map(|(group, offset, result)| {
            let key = Key::of(&result, &needle, group, offset);
            (key, result)
        })
        .collect();
    keyed.sort_by_key(|(key, _)| *key);
    keyed.into_iter().map(|(_, result)| result).collect()
}

/// The sort key of one hit.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Key {
    tier: u8,
    group: usize,
    quote: usize,
    offset: usize,
}

impl Key {
    fn of(result: &SearchResult, needle: &str, group: usize, offset: usize) -> Self {
        let (tier, quote) = if needle.is_empty() {
            (0, 0)
        } else {
            (tier(result, needle), quote_order(result))
        };
        Self {
            tier,
            group,
            quote,
            offset,
        }
    }
}

/// An exchange's main dollar quote first (USDT on Binance, USD on Coinbase), then the other dollar
/// quotes in their own order; a hit with no quote (a coin, a share) after the exchange pairs;
/// other quotes last.
fn quote_order(result: &SearchResult) -> usize {
    let home = match result.source {
        DataSource::Binance => Some("USDT"),
        DataSource::Coinbase => Some("USD"),
        _ => None,
    };
    match result.quote.as_deref() {
        Some(quote) if home.is_some_and(|home| home.eq_ignore_ascii_case(quote)) => 0,
        Some(quote) => dollar_rank(quote).map_or(DOLLAR_QUOTES.len() + 2, |rank| rank + 1),
        None => DOLLAR_QUOTES.len() + 1,
    }
}

fn tier(result: &SearchResult, needle: &str) -> u8 {
    let ticker = normalize(result.ticker());
    let base = normalize(&result.base_asset());
    let pair = match &result.quote {
        Some(quote) => format!("{base}{}", normalize(quote)),
        None => ticker.clone(),
    };
    let company = result.company().map(normalize).unwrap_or_default();
    // A coin or a share has no quote; it is priced in dollars.
    let dollar = result.quote.as_deref().is_none_or(is_dollar_quote);

    if pair == needle || ticker == needle || (base == needle && dollar) {
        0
    } else if dollar && needle.starts_with(&base) && pair.starts_with(needle) {
        1
    } else if base == needle {
        2
    } else if base.starts_with(needle) || pair.starts_with(needle) || company.starts_with(needle) {
        3
    } else if pair.contains(needle) || ticker.contains(needle) || company.contains(needle) {
        4
    } else {
        5
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource::{Alpaca, Binance, CoinGecko, Coinbase, DexScreener};

    fn binance(base: &str, quote: &str) -> SearchResult {
        SearchResult::new(Binance, format!("{base}{quote}"), format!("{base}{quote}"))
            .pair(base, quote)
    }

    fn coinbase(base: &str, quote: &str) -> SearchResult {
        SearchResult::new(
            Coinbase,
            format!("{base}-{quote}"),
            format!("{base}-{quote}"),
        )
        .pair(base, quote)
    }

    fn symbols(results: &[SearchResult]) -> Vec<&str> {
        results
            .iter()
            .map(|result| result.symbol.as_str())
            .collect()
    }

    #[test]
    fn the_asset_in_dollars_comes_before_odd_quotes_and_lookalikes() {
        let results = merge_ranked(
            vec![
                vec![
                    binance("ABTC", "USDT"),
                    binance("BTC", "TRY"),
                    binance("BTC", "EUR"),
                    binance("BTC", "USDC"),
                    binance("BTC", "USDT"),
                    binance("BTCB", "USDT"),
                ],
                vec![coinbase("BTC", "EUR"), coinbase("BTC", "USD")],
            ],
            "btc",
        );
        assert_eq!(
            symbols(&results),
            [
                "BTCUSDT", "BTCUSDC", "BTC-USD", "BTCTRY", "BTCEUR", "BTC-EUR", "BTCBUSDT",
                "ABTCUSDT"
            ]
        );
    }

    #[test]
    fn every_spelling_of_a_market_finds_the_same_markets() {
        let hits = || {
            vec![
                vec![binance("BTC", "TRY"), binance("BTC", "USDT")],
                vec![coinbase("BTC", "USD"), coinbase("BTC", "EUR")],
            ]
        };
        for query in ["BTCUSD", "btc/usd", "BTC-USD", "btc usd"] {
            let results = merge_ranked(hits(), query);
            let top: Vec<&str> = symbols(&results).into_iter().take(2).collect();
            // The exact market leads, then its dollar lookalikes; the odd quotes trail.
            let expected = ["BTC-USD", "BTCUSDT"];
            assert_eq!(top, expected, "{query}");
        }
    }

    #[test]
    fn an_exact_market_beats_the_asset_in_a_better_quote() {
        let results = rank_results(
            vec![binance("BTC", "USDT"), binance("BTC", "EUR")],
            "btceur",
        );
        assert_eq!(symbols(&results), ["BTCEUR", "BTCUSDT"]);
    }

    #[test]
    fn exchanges_keep_the_order_they_were_asked_in_within_a_tier() {
        let results = merge_ranked(
            vec![vec![coinbase("ETH", "USD")], vec![binance("ETH", "USDT")]],
            "eth",
        );
        assert_eq!(symbols(&results), ["ETH-USD", "ETHUSDT"]);
    }

    #[test]
    fn a_coin_or_a_share_with_the_queried_ticker_ranks_with_the_dollar_markets() {
        let gecko = SearchResult::new(CoinGecko, "BTC", "bitcoin");
        let share = SearchResult::new(Alpaca, "AAPL — Apple Inc.", "AAPL");
        let results = merge_ranked(
            vec![
                vec![binance("BTC", "TRY"), binance("BTC", "USDT")],
                vec![gecko],
            ],
            "btc",
        );
        assert_eq!(symbols(&results), ["BTCUSDT", "BTC", "BTCTRY"]);

        let results = merge_ranked(vec![vec![binance("AAPL", "USDT")], vec![share]], "aapl");
        assert_eq!(symbols(&results)[0], "AAPLUSDT");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn a_company_name_finds_its_shares() {
        let apple = SearchResult::new(Alpaca, "AAPL — Apple Inc.", "AAPL");
        let pineapple = SearchResult::new(Alpaca, "PAPL — Pineapple Holdings", "PAPL");
        let results = rank_results(vec![pineapple, apple], "apple");
        assert_eq!(results[0].full_symbol, "AAPL");
    }

    #[test]
    fn a_dex_pair_in_a_volatile_quote_trails_the_dollar_markets() {
        let dex = |quote: &str| {
            SearchResult::new(DexScreener, format!("PEPE/{quote}"), "0xpair").pair("PEPE", quote)
        };
        let results = rank_results(vec![dex("WETH"), dex("USDC")], "pepe");
        assert_eq!(symbols(&results), ["PEPE/USDC", "PEPE/WETH"]);
    }

    #[test]
    fn each_exchange_leads_with_its_main_dollar_quote() {
        let results = merge_ranked(
            vec![
                vec![
                    binance("BTC", "USD"),
                    binance("BTC", "USDC"),
                    binance("BTC", "USDT"),
                ],
                vec![coinbase("BTC", "USDT"), coinbase("BTC", "USD")],
            ],
            "btc",
        );
        assert_eq!(
            symbols(&results),
            ["BTCUSDT", "BTCUSD", "BTCUSDC", "BTC-USD", "BTC-USDT"]
        );
    }

    #[test]
    fn an_empty_query_keeps_the_providers_order() {
        let results = merge_ranked(
            vec![vec![binance("ETH", "BTC"), binance("BTC", "USDT")]],
            " ",
        );
        assert_eq!(symbols(&results), ["ETHBTC", "BTCUSDT"]);
    }

    #[test]
    fn dollar_quotes_are_recognised_in_any_case() {
        assert!(is_dollar_quote("usdt"));
        assert!(is_dollar_quote("USD"));
        assert!(!is_dollar_quote("EUR"));
        assert!(!is_dollar_quote("BTC"));
    }
}
