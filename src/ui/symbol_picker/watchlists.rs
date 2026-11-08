//! The Watchlists tab: the markets of the user's watchlists, in their order, grouped by list.
//! Nothing is searched; the query filters what the lists already hold.

use super::search::PROVIDERS;
use crate::model::DataSource;
use crate::model::watchlist::{Watchlist, WatchlistInstrument};
use crate::service::{NAME_SEPARATOR, SearchResult, search_rank::normalize};

/// One watchlist and the markets of it the picker can open.
#[derive(Clone, Debug, PartialEq)]
pub struct WatchlistGroup {
    pub name: String,
    pub markets: Vec<SearchResult>,
}

/// The groups for `lists`, leaving out lists with nothing the picker can open (empty ones, and
/// ones holding only prediction markets, indices or portfolio cards).
pub fn groups(lists: &[Watchlist]) -> Vec<WatchlistGroup> {
    lists
        .iter()
        .filter_map(|list| {
            let markets: Vec<SearchResult> = list.instruments().filter_map(market).collect();
            (!markets.is_empty()).then(|| WatchlistGroup {
                name: list.name.clone(),
                markets,
            })
        })
        .collect()
}

/// A row of a watchlist as a search hit, named as its exchange names it. `None` for what the
/// picker does not open: a portfolio card, a prediction market, a CoinMarketCap index.
fn market(item: &WatchlistInstrument) -> Option<SearchResult> {
    let source = item.instrument.source;
    if item.card.is_some() || !PROVIDERS.contains(&source) {
        return None;
    }
    let id = item.instrument.symbol.as_str();
    Some(match source {
        DataSource::Alpaca => {
            let ticker = id.to_uppercase();
            let symbol = if item.name.is_empty() || item.name.eq_ignore_ascii_case(&ticker) {
                ticker.clone()
            } else {
                format!("{ticker}{NAME_SEPARATOR}{}", item.name)
            };
            SearchResult::new(source, symbol, id)
        }
        DataSource::DexScreener => {
            let title = item.display_name.as_deref().unwrap_or(&item.name);
            let result = SearchResult::new(source, title, id);
            match title.split_once('/') {
                Some((base, quote)) => result.pair(base, quote),
                None => result,
            }
        }
        DataSource::CoinGecko => {
            let ticker = if item.label.is_empty() {
                id.to_uppercase()
            } else {
                item.label.to_uppercase()
            };
            SearchResult::new(source, ticker, id)
        }
        _ => {
            // Binance and Coinbase: the name is the id; the pair is read from it.
            let symbol = source.display_symbol(id);
            let base = source.base_asset(id);
            let quote = symbol
                .strip_prefix(&base)
                .map(|rest| rest.trim_start_matches(['-', '/']))
                .filter(|quote| !quote.is_empty());
            let result = SearchResult::new(source, symbol.clone(), id);
            match quote {
                Some(quote) => result.pair(base.clone(), quote),
                None => result,
            }
        }
    })
}

/// Whether `result` answers `query` (letters and digits only; empty matches all): its ticker, its
/// asset or its company contains it.
pub fn matches(result: &SearchResult, query: &str) -> bool {
    let needle = normalize(query);
    needle.is_empty()
        || normalize(result.ticker()).contains(&needle)
        || normalize(&result.base_asset()).contains(&needle)
        || result
            .company()
            .is_some_and(|company| normalize(company).contains(&needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::InstrumentId;

    fn item(source: DataSource, id: &str, name: &str) -> WatchlistInstrument {
        WatchlistInstrument::new(InstrumentId::new(source, id), name, "")
    }

    #[test]
    fn watchlist_rows_read_as_the_exchange_names_them() {
        let btc = market(&item(DataSource::Binance, "BTCUSDT", "BTC/USDT")).unwrap();
        assert_eq!(btc.symbol, "BTCUSDT");
        assert_eq!(
            (btc.base.as_deref(), btc.quote.as_deref()),
            (Some("BTC"), Some("USDT"))
        );
        let eth = market(&item(DataSource::Coinbase, "eth-usd", "ETH/USD")).unwrap();
        assert_eq!(eth.symbol, "ETH-USD");
        assert_eq!(
            (eth.base.as_deref(), eth.quote.as_deref()),
            (Some("ETH"), Some("USD"))
        );
        let apple = market(&item(DataSource::Alpaca, "AAPL", "Apple Inc.")).unwrap();
        assert_eq!(apple.symbol, "AAPL — Apple Inc.");
        assert_eq!(apple.full_symbol, "AAPL");
        let dex = market(&item(DataSource::DexScreener, "0xpair", "PEPE/WETH")).unwrap();
        assert_eq!(dex.symbol, "PEPE/WETH");
        assert_eq!(dex.quote.as_deref(), Some("WETH"));
    }

    #[test]
    fn what_the_picker_cannot_open_is_left_out() {
        assert!(market(&item(DataSource::Polymarket, "token", "Will it?")).is_none());
        assert!(market(&item(DataSource::CoinMarketCap, "fear-greed", "Fear")).is_none());
    }

    #[test]
    fn lists_with_nothing_to_open_are_dropped_and_order_is_kept() {
        let mut crypto = Watchlist::new("Crypto");
        crypto
            .add(item(DataSource::Binance, "ETHUSDT", "ETH"), None)
            .unwrap();
        crypto
            .add(item(DataSource::Binance, "BTCUSDT", "BTC"), None)
            .unwrap();
        let mut bets = Watchlist::new("Bets");
        bets.add(item(DataSource::Polymarket, "token", "Q?"), None)
            .unwrap();
        let groups = groups(&[crypto, bets, Watchlist::new("Empty")]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "Crypto");
        let names: Vec<&str> = groups[0]
            .markets
            .iter()
            .map(|m| m.symbol.as_str())
            .collect();
        assert_eq!(names, ["ETHUSDT", "BTCUSDT"]);
    }

    #[test]
    fn the_query_filters_by_ticker_asset_or_company() {
        let apple = SearchResult::new(DataSource::Alpaca, "AAPL — Apple Inc.", "AAPL");
        let btc = SearchResult::new(DataSource::Binance, "BTCUSDT", "BTCUSDT").pair("BTC", "USDT");
        assert!(matches(&apple, ""));
        assert!(matches(&apple, "apple"));
        assert!(matches(&apple, "aap"));
        assert!(!matches(&apple, "btc"));
        assert!(matches(&btc, "btc-usdt"));
        assert!(matches(&btc, "BTC"));
        assert!(!matches(&btc, "eth"));
    }
}
