//! Venue prefixes: `BINANCE:BTCUSDT`, `nasdaq: aapl`. One reading for the watchlist import and
//! the symbol picker, so a line that imports also finds its market when typed.
//!
//! A prefix is a provider's watchlist slug ([`DataSource::watchlist_slug`]) or a US stock
//! exchange, which Alpaca serves. Anything else (OKX, BYBIT, HKEX) names a venue no provider here
//! lists; the parse keeps the prefix so callers can say so.

use crate::model::DataSource;

/// US listings on these venues map to Alpaca, which serves US equities.
pub const US_EXCHANGES: [&str; 7] = [
    "NASDAQ",
    "NYSE",
    "AMEX",
    "ARCA",
    "NYSEARCA",
    "BATS",
    "NYSEAMERICAN",
];

/// What `PREFIX:SYMBOL` says.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VenueQuery {
    /// The text before the first colon, trimmed and upper-cased.
    pub prefix: String,
    /// The provider that serves the prefix; `None` for a venue none does.
    pub source: Option<DataSource>,
    /// What follows the colon, trimmed. May be empty while the user is still typing.
    pub symbol: String,
}

/// The provider a venue prefix names, in any case and with stray spaces: a watchlist slug, or a
/// US stock exchange (Alpaca).
pub fn source_for_prefix(prefix: &str) -> Option<DataSource> {
    let prefix = prefix.trim();
    if US_EXCHANGES
        .iter()
        .any(|exchange| exchange.eq_ignore_ascii_case(prefix))
    {
        return Some(DataSource::Alpaca);
    }
    DataSource::from_watchlist_slug(prefix)
}

/// Splits `text` at its first colon into a venue and a symbol. `None` when there is no colon.
pub fn parse(text: &str) -> Option<VenueQuery> {
    let (prefix, symbol) = text.split_once(':')?;
    Some(VenueQuery {
        source: source_for_prefix(prefix),
        prefix: prefix.trim().to_uppercase(),
        symbol: symbol.trim().to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(text: &str) -> Option<DataSource> {
        parse(text).and_then(|query| query.source)
    }

    #[test]
    fn every_provider_slug_names_its_provider() {
        for (slug, expected) in [
            ("BINANCE", DataSource::Binance),
            ("COINBASE", DataSource::Coinbase),
            ("COINGECKO", DataSource::CoinGecko),
            ("DEXSCREENER", DataSource::DexScreener),
            ("ALPACA", DataSource::Alpaca),
            ("POLYMARKET", DataSource::Polymarket),
            ("KALSHI", DataSource::Kalshi),
            ("CMC", DataSource::CoinMarketCap),
        ] {
            assert_eq!(source(&format!("{slug}:X")), Some(expected), "{slug}");
        }
    }

    #[test]
    fn every_us_exchange_is_served_by_alpaca() {
        for exchange in US_EXCHANGES {
            assert_eq!(
                source(&format!("{exchange}:AAPL")),
                Some(DataSource::Alpaca),
                "{exchange}"
            );
        }
    }

    #[test]
    fn a_prefix_is_read_in_any_case_and_with_spaces() {
        let query = parse("  Binance : btcusdt ").unwrap();
        assert_eq!(query.source, Some(DataSource::Binance));
        assert_eq!(query.prefix, "BINANCE");
        assert_eq!(query.symbol, "btcusdt", "the symbol keeps its case");
        assert_eq!(source("nYsE:ibm"), Some(DataSource::Alpaca));
    }

    #[test]
    fn a_venue_no_provider_serves_keeps_its_prefix() {
        for prefix in ["OKX", "BYBIT", "HKEX", "SSE", "CME", "FX"] {
            let query = parse(&format!("{}:BTCUSDT", prefix.to_lowercase())).unwrap();
            assert_eq!(query.source, None, "{prefix}");
            assert_eq!(query.prefix, prefix);
            assert_eq!(query.symbol, "BTCUSDT");
        }
    }

    #[test]
    fn text_without_a_colon_is_no_venue_query() {
        assert_eq!(parse("BTCUSDT"), None);
        assert_eq!(parse(""), None);
    }

    #[test]
    fn only_the_first_colon_splits() {
        let query = parse("POLYMARKET:Fed: cut").unwrap();
        assert_eq!(query.source, Some(DataSource::Polymarket));
        assert_eq!(query.symbol, "Fed: cut");
    }

    #[test]
    fn a_prefix_with_nothing_after_it_has_an_empty_symbol() {
        let query = parse("binance:").unwrap();
        assert_eq!(query.source, Some(DataSource::Binance));
        assert_eq!(query.symbol, "");
        assert_eq!(parse(":BTC").unwrap().source, None);
    }
}
