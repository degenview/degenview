//! What a market's assets are called. A provider's own name always wins; where it lists none, the
//! name comes from a directory that fits the provider: coin directories for exchanges, never for
//! stocks, forex or DEX pools, whose tickers mean something else.

use super::coin_index::{self, CoinIndex};
use crate::model::DataSource;
use crate::model::market::asset_name::{
    AssetNames, company_name, currency_name, popular_company, sanitize,
};
use crate::service::SearchResult;
use std::collections::HashMap;
use std::sync::Arc;

/// Asset names by upper-case ticker, as the exchanges that publish them list their markets.
pub type Listed = HashMap<String, String>;

/// The names gathered from elsewhere than the market's own provider. Empty until loaded, which
/// only leaves some names unknown.
#[derive(Clone, Default)]
pub struct Directory {
    pub coins: Arc<CoinIndex>,
    pub listed: Arc<Listed>,
}

impl Directory {
    /// What `result`'s two sides are called, as far as anything says.
    pub fn names(&self, result: &SearchResult) -> AssetNames {
        let own = (*result.names).clone();
        let base = result.base_asset();
        let quote = result
            .quote
            .clone()
            .or_else(|| result.source.quote_asset(&result.symbol));
        match result.source {
            DataSource::Binance
            | DataSource::Coinbase
            | DataSource::Okx
            | DataSource::Kraken
            | DataSource::Bybit
            | DataSource::Gate
            | DataSource::Mexc => AssetNames {
                base: own.base.or_else(|| self.coin(&base)),
                quote: own.quote.or_else(|| {
                    let quote = quote?;
                    currency_name(&quote)
                        .map(str::to_owned)
                        .or_else(|| self.coin(&quote))
                }),
            },
            DataSource::CoinGecko => AssetNames {
                base: own
                    .base
                    .or_else(|| {
                        let coin = self.coins.by_id(&result.full_symbol)?;
                        sanitize(&coin.name)
                    })
                    .or_else(|| sanitize(&title_case(&result.full_symbol))),
                quote: None,
            },
            // A pool's tokens name themselves; a ticker would name a different token.
            DataSource::DexScreener => own,
            DataSource::Alpaca => AssetNames {
                base: result
                    .company()
                    .map(company_name)
                    .or_else(|| popular_company(result.ticker()).map(str::to_owned)),
                quote: None,
            },
            // Oanda's own name describes the whole instrument ("Germany 30").
            DataSource::Oanda if own.base.is_some() => own,
            DataSource::Oanda => AssetNames {
                base: currency_name(&base).map(str::to_owned),
                quote: quote
                    .and_then(|quote| currency_name(&quote))
                    .map(str::to_owned),
            },
            DataSource::Polymarket | DataSource::Kalshi | DataSource::CoinMarketCap => {
                AssetNames::default()
            }
        }
    }

    /// The name of the coin or currency trading as `ticker` on an exchange. The coin directories
    /// come first: `MNT` there is a coin, not the tugrik.
    fn coin(&self, ticker: &str) -> Option<String> {
        self.coins
            .by_symbol(ticker)
            .and_then(|coin| sanitize(&coin.name))
            .or_else(|| self.listed.get(&ticker.to_uppercase()).cloned())
            .or_else(|| currency_name(ticker).map(str::to_owned))
    }
}

/// The top coins by market cap. Blocks, and may wait for a rate-limit slot.
pub fn load_coins() -> Arc<CoinIndex> {
    coin_index::load().0
}

/// The names Gate.io and MEXC publish with their markets, from their day-long market lists.
/// Blocks.
pub fn load_listed() -> Arc<Listed> {
    let mut listed = Listed::new();
    let names = super::gateio::asset_names()
        .into_iter()
        .chain(super::mexc::asset_names());
    for (ticker, name) in names {
        if let Some(name) = sanitize(&name) {
            listed.entry(ticker).or_insert(name);
        }
    }
    Arc::new(listed)
}

/// `wrapped-bitcoin` → `Wrapped Bitcoin`.
fn title_case(id: &str) -> String {
    id.split('-')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory() -> Directory {
        Directory {
            coins: Arc::new(CoinIndex::of(&[
                ("bitcoin", "btc", "Bitcoin"),
                ("mantle", "mnt", "Mantle"),
            ])),
            listed: Arc::new(Listed::from([("PEPE".to_owned(), "Pepe".to_owned())])),
        }
    }

    fn names(result: SearchResult) -> (Option<String>, Option<String>) {
        let names = directory().names(&result);
        (names.base, names.quote)
    }

    fn pair(source: DataSource, symbol: &str, base: &str, quote: &str) -> SearchResult {
        SearchResult::new(source, symbol, symbol).pair(base, quote)
    }

    fn some(text: &str) -> Option<String> {
        Some(text.to_owned())
    }

    #[test]
    fn an_exchange_without_names_borrows_them_by_ticker() {
        use DataSource::Binance;
        assert_eq!(
            names(pair(Binance, "BTCUSDT", "BTC", "USDT")),
            (some("Bitcoin"), some("Tether"))
        );
        assert_eq!(
            names(pair(Binance, "PEPEEUR", "PEPE", "EUR")),
            (some("Pepe"), some("Euro"))
        );
        assert_eq!(
            names(pair(Binance, "MNTUSDT", "MNT", "USDT")).0,
            some("Mantle")
        );
        // A recent from before pairs were stored still names itself.
        assert_eq!(
            names(SearchResult::new(Binance, "BTCUSDT", "BTCUSDT")),
            (some("Bitcoin"), some("Tether"))
        );
    }

    #[test]
    fn a_provider_s_own_name_wins() {
        let result =
            pair(DataSource::Gate, "BTC_USDT", "BTC", "USDT").named(Some("Big Coin"), None);
        assert_eq!(names(result).0, some("Big Coin"));
    }

    #[test]
    fn a_pool_a_stock_and_forex_never_borrow_a_coin_s_name() {
        assert_eq!(
            names(pair(DataSource::DexScreener, "BTC/USDC", "BTC", "USDC")),
            (None, None)
        );
        assert_eq!(
            names(SearchResult::new(
                DataSource::Alpaca,
                "BTC — Grayscale Bitcoin Mini Trust",
                "BTC"
            )),
            (some("Grayscale Bitcoin Mini Trust"), None)
        );
        assert_eq!(
            names(pair(DataSource::Oanda, "EUR_USD", "EUR", "USD")),
            (some("Euro"), some("US Dollar"))
        );
        assert_eq!(
            names(pair(DataSource::Oanda, "BTC_USD", "BTC", "USD")).0,
            None
        );
    }

    #[test]
    fn a_stock_and_a_coin_id_are_named_without_a_search() {
        assert_eq!(
            names(SearchResult::new(DataSource::Alpaca, "AAPL", "AAPL")).0,
            some("Apple Inc.")
        );
        assert_eq!(
            names(SearchResult::new(DataSource::CoinGecko, "BTC", "bitcoin")).0,
            some("Bitcoin")
        );
        assert_eq!(
            names(SearchResult::new(
                DataSource::CoinGecko,
                "WBTC",
                "wrapped-bitcoin"
            ))
            .0,
            some("Wrapped Bitcoin")
        );
    }
}
