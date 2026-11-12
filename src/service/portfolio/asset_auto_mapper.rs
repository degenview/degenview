//! Finds the market a bare ticker ("FLOKI") should be valued with, for imports that carry only
//! a symbol. A port of `PortfolioAssetAutoMapper`: Binance first, because an exact BASE/QUOTE
//! market gives a direct, liquid price; then Coinbase; then CoinGecko, whose id is a stable
//! identity rather than a ticker; DEX pairs last.

// Transient: the portfolio host wires this up for the CoinMarketCap import.
#![allow(dead_code)]

use crate::model::DataSource;
use crate::model::portfolio::{PortfolioAsset, PortfolioCurrency};
use crate::service::SearchResult;
use std::collections::HashMap;

/// A portfolio asset for a search hit: `"<source>:<full symbol>"`, priced in the pair's quote
/// currency (stablecoins count as dollars).
pub fn asset_from_search_result(result: &SearchResult) -> PortfolioAsset {
    // A portfolio keeps a pair as BASE/QUOTE whatever the exchange calls it, so what it stored
    // before the picker showed exchange names still matches.
    let label = result.pair_label();
    let quote = result
        .quote
        .clone()
        .or_else(|| label.rsplit_once('/').map(|(_, quote)| quote.to_owned()));
    PortfolioAsset {
        key: format!("{}:{}", result.source.display_name(), result.full_symbol),
        symbol: label.clone(),
        name: label,
        source: result.source,
        quote_currency: PortfolioCurrency::from_quote_symbol(quote.as_deref()),
        metadata: HashMap::new(),
    }
}

/// Quotes to try, best first: the portfolio's own currency, then the dollar family.
pub fn quote_symbols(currency: PortfolioCurrency) -> Vec<String> {
    let dollars = ["USDT", "USDC", "USD"].map(str::to_owned);
    if currency == PortfolioCurrency::Usd {
        dollars.to_vec()
    } else {
        std::iter::once(currency.code().to_owned())
            .chain(dollars)
            .collect()
    }
}

fn base_symbol(pair: &str) -> String {
    pair.to_uppercase()
        .split('/')
        .next()
        .unwrap_or_default()
        .to_owned()
}

fn quote_symbol(pair: &str) -> Option<String> {
    let upper = pair.to_uppercase();
    let mut parts = upper.split('/');
    parts.next()?;
    parts.next().map(str::to_owned)
}

/// The result whose base is `token`, in the order of the preferred quotes.
pub fn best_pair<'a>(
    results: &'a [SearchResult],
    token: &str,
    preferred_quotes: &[String],
) -> Option<&'a SearchResult> {
    let token = token.to_uppercase();
    let exact: Vec<&SearchResult> = results
        .iter()
        .filter(|result| base_symbol(&result.symbol) == token)
        .collect();
    preferred_quotes.iter().find_map(|quote| {
        exact
            .iter()
            .find(|result| quote_symbol(&result.symbol).as_deref() == Some(quote.as_str()))
            .copied()
    })
}

/// Resolves `symbol` through `search` (a provider's ticker search; empty on failure).
pub fn resolve(
    symbol: &str,
    base_currency: PortfolioCurrency,
    search: &dyn Fn(DataSource, &str) -> Vec<SearchResult>,
) -> Option<PortfolioAsset> {
    let token = symbol.to_uppercase();
    let quotes = quote_symbols(base_currency);

    for (source, separator) in [(DataSource::Binance, ""), (DataSource::Coinbase, "-")] {
        for quote in &quotes {
            let pair = format!("{token}{separator}{quote}");
            let results = search(source, &pair);
            if let Some(found) = results
                .iter()
                .find(|result| result.full_symbol.eq_ignore_ascii_case(&pair))
            {
                return Some(asset_from_search_result(found));
            }
        }
    }

    let gecko = search(DataSource::CoinGecko, &token);
    if let Some(exact) = gecko
        .iter()
        .find(|result| result.symbol.eq_ignore_ascii_case(&token))
    {
        return Some(asset_from_search_result(exact));
    }

    let dex = search(DataSource::DexScreener, &token);
    best_pair(&dex, &token, &quotes)
        .or_else(|| {
            dex.iter()
                .find(|result| base_symbol(&result.symbol) == token)
        })
        .map(asset_from_search_result)
}

/// `resolve` over the real providers. Blocking: run it off the UI thread.
pub fn resolve_live(symbol: &str, base_currency: PortfolioCurrency) -> Option<PortfolioAsset> {
    resolve(symbol, base_currency, &|source, query| {
        crate::service::search(source, query).unwrap_or_default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn hit(source: DataSource, symbol: &str, full: &str) -> SearchResult {
        SearchResult {
            symbol: symbol.into(),
            full_symbol: full.into(),
            source,
            price: None,
            event: None,
            base: None,
            quote: None,
        }
    }

    /// A search over canned results that records every query.
    fn canned(
        results: Vec<SearchResult>,
    ) -> (RefCell<Vec<(DataSource, String)>>, Vec<SearchResult>) {
        (RefCell::new(Vec::new()), results)
    }

    fn run(
        symbol: &str,
        currency: PortfolioCurrency,
        results: Vec<SearchResult>,
    ) -> (Option<PortfolioAsset>, Vec<(DataSource, String)>) {
        let (calls, results) = canned(results);
        let asset = resolve(symbol, currency, &|source, query| {
            calls.borrow_mut().push((source, query.to_owned()));
            results
                .iter()
                .filter(|result| result.source == source)
                .cloned()
                .collect()
        });
        (asset, calls.into_inner())
    }

    #[test]
    fn binance_wins_with_the_first_exact_pair() {
        let (asset, calls) = run(
            "floki",
            PortfolioCurrency::Usd,
            vec![
                hit(DataSource::Binance, "FLOKI/USDT", "FLOKIUSDT"),
                hit(DataSource::Coinbase, "FLOKI/USD", "FLOKI-USD"),
            ],
        );
        let asset = asset.unwrap();
        assert_eq!(asset.key, "Binance:FLOKIUSDT");
        assert_eq!(asset.quote_currency, PortfolioCurrency::Usd);
        assert_eq!(calls, [(DataSource::Binance, "FLOKIUSDT".to_owned())]);
    }

    #[test]
    fn a_euro_portfolio_tries_euro_pairs_first() {
        let (asset, calls) = run(
            "BTC",
            PortfolioCurrency::Eur,
            vec![
                hit(DataSource::Binance, "BTC/EUR", "BTCEUR"),
                hit(DataSource::Binance, "BTC/USDT", "BTCUSDT"),
            ],
        );
        let asset = asset.unwrap();
        assert_eq!(asset.key, "Binance:BTCEUR");
        assert_eq!(asset.quote_currency, PortfolioCurrency::Eur);
        assert_eq!(calls.len(), 1);
        assert_eq!(
            quote_symbols(PortfolioCurrency::Eur),
            ["EUR", "USDT", "USDC", "USD"]
        );
        assert_eq!(
            quote_symbols(PortfolioCurrency::Usd),
            ["USDT", "USDC", "USD"]
        );
    }

    #[test]
    fn a_near_miss_is_not_a_match_so_the_next_source_is_asked() {
        let (asset, calls) = run(
            "BTC",
            PortfolioCurrency::Usd,
            vec![
                hit(DataSource::Binance, "BTC/USDT", "BTCUSDT_PERP"),
                hit(DataSource::Coinbase, "BTC/USDC", "BTC-USDC"),
            ],
        );
        assert_eq!(asset.unwrap().key, "Coinbase:BTC-USDC");
        let binance = calls
            .iter()
            .filter(|(source, _)| *source == DataSource::Binance)
            .count();
        assert_eq!(
            binance, 3,
            "every preferred quote is tried on Binance first"
        );
        assert!(calls.contains(&(DataSource::Coinbase, "BTC-USDT".to_owned())));
    }

    #[test]
    fn coingecko_matches_on_the_exact_symbol_then_dex_pairs_by_quote_preference() {
        let (asset, _) = run(
            "pepe",
            PortfolioCurrency::Usd,
            vec![
                hit(DataSource::CoinGecko, "PEPEX", "pepex"),
                hit(DataSource::CoinGecko, "pepe", "pepe"),
                hit(DataSource::DexScreener, "PEPE/WETH", "0xaaa"),
            ],
        );
        assert_eq!(asset.unwrap().key, "CoinGecko:pepe");

        let (asset, _) = run(
            "pepe",
            PortfolioCurrency::Usd,
            vec![
                hit(DataSource::DexScreener, "PEPE/WETH", "0xaaa"),
                hit(DataSource::DexScreener, "PEPE/USDC", "0xbbb"),
                hit(DataSource::DexScreener, "OTHER/USDT", "0xccc"),
            ],
        );
        assert_eq!(asset.unwrap().key, "DEXScreener:0xbbb", "USDC beats WETH");

        let (asset, _) = run(
            "pepe",
            PortfolioCurrency::Usd,
            vec![hit(DataSource::DexScreener, "PEPE/WETH", "0xaaa")],
        );
        assert_eq!(
            asset.unwrap().key,
            "DEXScreener:0xaaa",
            "any pair with the base beats nothing"
        );
    }

    #[test]
    fn an_unknown_ticker_maps_to_nothing() {
        let (asset, calls) = run("zzzz", PortfolioCurrency::Usd, vec![]);
        assert!(asset.is_none());
        assert_eq!(calls.len(), 3 + 3 + 1 + 1);
    }

    #[test]
    fn search_hits_become_assets_priced_in_their_quote() {
        let asset = asset_from_search_result(&hit(DataSource::Coinbase, "ETH/EUR", "ETH-EUR"));
        assert_eq!(asset.quote_currency, PortfolioCurrency::Eur);
        assert_eq!(asset.display_ticker(), "ETH");
        let stock = asset_from_search_result(&hit(DataSource::Alpaca, "AAPL — Apple Inc.", "AAPL"));
        assert_eq!(stock.key, "Alpaca (IEX):AAPL");
        assert_eq!(
            stock.quote_currency,
            PortfolioCurrency::Usd,
            "no pair: dollars"
        );
        assert_eq!(base_symbol("btc/usdt"), "BTC");
        assert_eq!(quote_symbol("btc"), None);
    }

    #[test]
    fn an_exchange_named_pair_is_kept_as_base_over_quote() {
        let hit = hit(DataSource::Binance, "BTCUSDT", "BTCUSDT").pair("BTC", "USDT");
        let asset = asset_from_search_result(&hit);
        assert_eq!(asset.symbol, "BTC/USDT");
        assert_eq!(asset.display_ticker(), "BTC");
        assert_eq!(asset.quote_currency, PortfolioCurrency::Usd);
    }
}
