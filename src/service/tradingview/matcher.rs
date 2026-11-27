//! Finds the DegenView market for a TradingView symbol: the same market on its own exchange, or
//! the same pair on another. Only an exact name counts; nothing that merely looks alike is taken.

use crate::model::DataSource;
use crate::model::tradingview::TvSymbol;
use crate::model::tradingview::plan::{Market, Match, Unmatched};
use crate::model::tradingview::symbol;
use crate::service::{ProviderError, SearchResult};

pub fn market_of(result: &SearchResult) -> Market {
    Market {
        source: result.source,
        symbol: result.full_symbol.clone(),
        name: result.symbol.clone(),
        image_url: result
            .event
            .as_ref()
            .and_then(|event| event.image_url.clone()),
    }
}

/// A provider's ticker search.
pub type Search =
    dyn Fn(DataSource, &str) -> Result<Vec<SearchResult>, ProviderError> + Send + Sync;

/// Resolves `tv` through `search`.
pub fn resolve(tv: &TvSymbol, search: &Search) -> Match {
    if tv.is_expression() {
        return Match::Unmatched(Unmatched::Expression);
    }
    if tv.is_derivative() {
        return Match::Unmatched(Unmatched::Derivative);
    }
    let (mut asked, mut failed) = (0, 0);
    let mut exact = |source: DataSource, name: &str| -> Result<Option<Market>, ProviderError> {
        asked += 1;
        match search(source, name) {
            Ok(results) => Ok(results
                .iter()
                .find(|result| result.full_symbol.eq_ignore_ascii_case(name))
                .map(market_of)),
            Err(error) => {
                failed += 1;
                Err(error)
            }
        }
    };

    let own = symbol::native_name(tv);
    if let Some((source, name)) = &own {
        match exact(*source, name) {
            Ok(Some(market)) => return Match::Exact(market),
            Err(ProviderError::MissingKey(DataSource::Alpaca)) => {
                return Match::Unmatched(Unmatched::NeedsKey);
            }
            // Not knowing is not a reason to take another exchange's market: search again.
            Err(_) => return Match::Unmatched(Unmatched::Unreachable),
            Ok(None) => {}
        }
    }
    // A stock's ticker is not a pair, whatever it ends in.
    let pair = tv
        .pair()
        .filter(|_| tv.source() != Some(DataSource::Alpaca));
    if let Some((base, quote)) = pair {
        for source in symbol::substitutes(quote) {
            if own.as_ref().is_some_and(|(own, _)| *own == source) {
                continue;
            }
            let Some(name) = symbol::pair_on(source, base, quote) else {
                continue;
            };
            if let Ok(Some(market)) = exact(source, &name) {
                return Match::Substitute(market);
            }
        }
    }
    Match::Unmatched(if asked > 0 && failed == asked {
        Unmatched::Unreachable
    } else if tv.source().is_none() {
        Unmatched::Venue
    } else {
        Unmatched::NotFound
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(source: DataSource, name: &str) -> SearchResult {
        SearchResult {
            symbol: name.to_owned(),
            full_symbol: name.to_owned(),
            source,
            price: None,
            event: None,
            base: None,
            quote: None,
        }
    }

    /// A catalog of what each provider lists.
    fn catalog(source: DataSource, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        let listed: &[&str] = match source {
            DataSource::Binance => &["BTCUSDT", "BTCUSDTX"],
            DataSource::Coinbase => &["ETH-USD"],
            _ => &[],
        };
        Ok(listed
            .iter()
            .filter(|name| name.starts_with(query))
            .map(|name| hit(source, name))
            .collect())
    }

    fn resolve_text(text: &str) -> Match {
        resolve(&TvSymbol::read(text).unwrap(), &catalog)
    }

    #[test]
    fn the_symbols_own_exchange_wins() {
        let Match::Exact(market) = resolve_text("BINANCE:BTCUSDT") else {
            panic!();
        };
        assert_eq!(market.symbol, "BTCUSDT");
    }

    #[test]
    fn an_unserved_exchange_gets_the_same_pair_elsewhere() {
        let Match::Substitute(market) = resolve_text("BITSTAMP:ETHUSD") else {
            panic!();
        };
        assert_eq!(
            (market.source, market.symbol.as_str()),
            (DataSource::Coinbase, "ETH-USD")
        );
    }

    #[test]
    fn a_different_quote_is_never_substituted() {
        assert_eq!(
            resolve_text("BITSTAMP:ETHEUR"),
            Match::Unmatched(Unmatched::Venue)
        );
        assert_eq!(
            resolve_text("BINANCE:BTCUSDC"),
            Match::Unmatched(Unmatched::NotFound)
        );
        assert_eq!(
            resolve_text("BINANCE:BTCUSDT.P"),
            Match::Unmatched(Unmatched::Derivative)
        );
    }

    #[test]
    fn an_exchange_that_does_not_answer_is_not_replaced() {
        let binance_down = |source: DataSource, query: &str| match source {
            DataSource::Binance => Err(ProviderError::Other("timeout".into())),
            _ => Ok(vec![hit(source, query)]),
        };
        assert_eq!(
            resolve(&TvSymbol::read("BINANCE:BTCUSDT").unwrap(), &binance_down),
            Match::Unmatched(Unmatched::Unreachable)
        );
    }

    #[test]
    fn a_missing_key_and_a_dead_network_are_said() {
        let no_key = |source: DataSource, _: &str| Err(ProviderError::MissingKey(source));
        assert_eq!(
            resolve(&TvSymbol::read("NASDAQ:AAPL").unwrap(), &no_key),
            Match::Unmatched(Unmatched::NeedsKey)
        );
        let down = |_: DataSource, _: &str| Err(ProviderError::Other("timeout".into()));
        assert_eq!(
            resolve(&TvSymbol::read("BINANCE:BTCUSDT").unwrap(), &down),
            Match::Unmatched(Unmatched::Unreachable)
        );
    }
}
