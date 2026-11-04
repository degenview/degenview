use crate::model::DataSource;
use serde::{Deserialize, Serialize};

/// A market on one provider. Equality and identity are source-qualified: a
/// display title is never identity.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct InstrumentId {
    pub source: DataSource,
    pub symbol: String,
}

impl InstrumentId {
    pub fn new(source: DataSource, symbol: impl Into<String>) -> Self {
        Self {
            source,
            symbol: symbol.into().trim().to_owned(),
        }
    }

    /// Stable identity for equality, flags, list membership, quote lookup and sync merging:
    /// the market, however it was spelled. Binance and Coinbase spellings that the provider
    /// takes as one market (`BTC`, `btcusdt`, `BTC/USDT`; `ETH`, `ETH/USD`, `eth-usd`) share a
    /// key, because it is built from `api_symbol`. Otherwise case-insensitive where the
    /// provider is: DEX addresses and prediction-market ids are case-sensitive.
    pub fn key(&self) -> String {
        format!(
            "{}:{}",
            self.source.watchlist_slug(),
            Self::normalized(&self.symbol, self.source)
        )
    }

    /// The id the provider's API takes. A bare Binance asset (`BTC`) is quoted in
    /// USDT, and Coinbase ids are `BASE-QUOTE`.
    pub fn api_symbol(&self) -> String {
        match self.source {
            DataSource::Binance => {
                let upper = self.symbol.to_uppercase().replace('/', "");
                // A one-letter "base" is a bare wrapped asset (WBTC, WETH), not a pair.
                if crate::model::source::split_binance_pair(&upper)
                    .is_some_and(|(base, _)| base.len() > 1)
                {
                    upper
                } else {
                    format!("{upper}USDT")
                }
            }
            DataSource::Coinbase => {
                let id = self.symbol.to_uppercase().replace('/', "-");
                if id.contains('-') {
                    id
                } else {
                    format!("{id}-USD")
                }
            }
            _ => self.symbol.clone(),
        }
    }

    /// Whether both name one market on one provider, comparing by the id the
    /// provider's API takes.
    pub fn is_same_market(&self, other: &InstrumentId) -> bool {
        self.source == other.source && self.api_symbol().eq_ignore_ascii_case(&other.api_symbol())
    }

    /// The key of the market a stored `key()` string names, spelled the way `key()` spells
    /// it now; `None` for a string that is not a key. Flags saved before aliases shared a
    /// key (`BINANCE:BTC`) are moved to the key of the market they were set on.
    pub fn rekey(key: &str) -> Option<String> {
        let (slug, symbol) = key.split_once(':')?;
        let source = DataSource::from_watchlist_slug(slug)?;
        Some(Self::new(source, symbol).key())
    }

    fn normalized(symbol: &str, source: DataSource) -> String {
        match source {
            DataSource::Binance | DataSource::Coinbase => Self::new(source, symbol).api_symbol(),
            DataSource::Alpaca => symbol.to_uppercase(),
            DataSource::CoinGecko => symbol.to_lowercase(),
            _ => symbol.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_source_qualified_and_case_normalized() {
        let binance = InstrumentId::new(DataSource::Binance, "btcusdt");
        let binance_upper = InstrumentId::new(DataSource::Binance, "BTCUSDT");
        let coinbase = InstrumentId::new(DataSource::Coinbase, "BTCUSDT");
        assert_eq!(binance.key(), binance_upper.key());
        assert_ne!(binance.key(), coinbase.key());

        let coingecko = InstrumentId::new(DataSource::CoinGecko, "Bitcoin");
        assert_eq!(coingecko.key(), "COINGECKO:bitcoin");
    }

    #[test]
    fn spellings_of_one_market_share_a_key() {
        let key = |source, symbol: &str| InstrumentId::new(source, symbol).key();
        for symbol in ["BTC", "btc", "BTCUSDT", "btcusdt", "BTC/USDT"] {
            assert_eq!(
                key(DataSource::Binance, symbol),
                "BINANCE:BTCUSDT",
                "{symbol}"
            );
        }
        for symbol in ["ETH", "eth-usd", "ETH/USD", "ETH-USD"] {
            assert_eq!(
                key(DataSource::Coinbase, symbol),
                "COINBASE:ETH-USD",
                "{symbol}"
            );
        }
        // A different quote is a different market.
        assert_ne!(
            key(DataSource::Binance, "BTC"),
            key(DataSource::Binance, "BTCFDUSD")
        );
        assert_ne!(
            key(DataSource::Coinbase, "BTC-USD"),
            key(DataSource::Coinbase, "BTC-EUR")
        );
        // Case-sensitive providers keep their case.
        assert_ne!(
            key(DataSource::DexScreener, "AbC"),
            key(DataSource::DexScreener, "abc")
        );
        assert_eq!(key(DataSource::Alpaca, "aapl"), "ALPACA:AAPL");
    }

    #[test]
    fn a_stored_key_is_respelled_as_the_market_it_names() {
        assert_eq!(
            InstrumentId::rekey("BINANCE:BTC").as_deref(),
            Some("BINANCE:BTCUSDT")
        );
        assert_eq!(
            InstrumentId::rekey("COINBASE:ETH/USD").as_deref(),
            Some("COINBASE:ETH-USD")
        );
        assert_eq!(
            InstrumentId::rekey("COINGECKO:bitcoin").as_deref(),
            Some("COINGECKO:bitcoin")
        );
        assert_eq!(InstrumentId::rekey("nonsense"), None);
        assert_eq!(InstrumentId::rekey("NOPE:BTC"), None);
    }

    #[test]
    fn api_symbol_quotes_bare_binance_and_coinbase() {
        assert_eq!(
            InstrumentId::new(DataSource::Binance, "BTC").api_symbol(),
            "BTCUSDT"
        );
        assert_eq!(
            InstrumentId::new(DataSource::Binance, "ETHUSDT").api_symbol(),
            "ETHUSDT"
        );
        assert_eq!(
            InstrumentId::new(DataSource::Coinbase, "BTC").api_symbol(),
            "BTC-USD"
        );
        assert_eq!(
            InstrumentId::new(DataSource::Coinbase, "ETH/USD").api_symbol(),
            "ETH-USD"
        );
    }

    #[test]
    fn api_symbol_keeps_any_quoted_binance_pair() {
        for pair in ["SOLBNB", "XRPBTC", "ETHFDUSD", "sol/bnb"] {
            let api = InstrumentId::new(DataSource::Binance, pair).api_symbol();
            assert_eq!(api, pair.to_uppercase().replace('/', ""));
        }
        assert_eq!(
            InstrumentId::new(DataSource::Binance, "WBTC").api_symbol(),
            "WBTCUSDT"
        );
    }

    #[test]
    fn same_market_compares_by_api_symbol() {
        let bare = InstrumentId::new(DataSource::Binance, "BTC");
        let quoted = InstrumentId::new(DataSource::Binance, "BTCUSDT");
        assert!(bare.is_same_market(&quoted));
        assert!(!bare.is_same_market(&InstrumentId::new(DataSource::Coinbase, "BTC")));
    }
}
