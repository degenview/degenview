use crate::model::exchange::Market;
use crate::model::market::binance::{join_market, split_market};
use crate::model::{DataSource, Listing};
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
    /// the market, however it was spelled. Binance, Coinbase and OKX spellings that the provider
    /// takes as one market (`BTC`, `btcusdt`, `BTC/USDT`; `ETH`, `ETH/USD`, `eth-usd`) share a
    /// key, because it is built from `canonical_symbol`. Otherwise case-insensitive where the
    /// provider is: DEX addresses and prediction-market ids are case-sensitive.
    pub fn key(&self) -> String {
        format!(
            "{}:{}",
            self.source.watchlist_slug(),
            Self::normalized(&self.symbol, self.source)
        )
    }

    /// Which of its source's markets this is.
    pub fn market(&self) -> Market {
        match self.source {
            DataSource::Binance => split_market(&self.symbol).1,
            _ => Market::Spot,
        }
    }

    pub fn listing(&self) -> Listing {
        Listing::new(self.source, self.market())
    }

    /// The short name of this market, given `asset`, the name of what it trades: `BTC`, or
    /// `BTC.P` for a perpetual, which is not to be read as the asset's spot market.
    pub fn short_name(&self, asset: &str) -> String {
        join_market(asset, self.market())
    }

    /// The one spelling of the market the app stores and compares, which a provider's answers
    /// are named by again. A bare Binance asset (`BTC`) is quoted in USDT, Coinbase ids are
    /// `BASE-QUOTE` (a bare asset is quoted in USD) and so are OKX's (in USDT); Kraken's are
    /// `BASE/QUOTE` (USD) in the common tickers, never the legacy `XBT`. It is also the id the
    /// provider's API takes, except for a Binance perpetual (`BTCUSDT.P`), whose suffix the
    /// Binance adapter takes off (`split_market`).
    pub fn canonical_symbol(&self) -> String {
        match self.source {
            DataSource::Mexc => {
                let upper = self.symbol.to_uppercase().replace(['/', '-', '_', ' '], "");
                // Its own quotes (`USDE`, `USDF`).
                if crate::model::market::source::split_mexc_pair(&upper).is_some() {
                    upper
                } else {
                    format!("{upper}USDT")
                }
            }
            DataSource::Binance => {
                let upper = self.symbol.to_uppercase().replace(['/', '-'], "");
                let (pair, market) = split_market(&upper);
                join_market(&quoted_in_usdt(pair), market)
            }
            DataSource::Bybit => {
                quoted_in_usdt(&self.symbol.to_uppercase().replace(['/', '-'], ""))
            }
            DataSource::Coinbase | DataSource::Okx => {
                let id = self.symbol.to_uppercase().replace('/', "-");
                if id.contains('-') {
                    id
                } else if self.source == DataSource::Okx {
                    format!("{id}-USDT")
                } else {
                    format!("{id}-USD")
                }
            }
            DataSource::Gate => {
                let id = self.symbol.to_uppercase().replace(['/', '-', ' '], "_");
                if id.contains('_') {
                    id
                } else {
                    format!("{id}_USDT")
                }
            }
            DataSource::Kraken => {
                let id = self.symbol.to_uppercase().replace(['-', ' '], "/");
                match id.split_once('/') {
                    Some((base, quote)) => format!(
                        "{}/{}",
                        crate::model::market::source::kraken_asset(base),
                        crate::model::market::source::kraken_asset(quote)
                    ),
                    None => format!("{}/USD", crate::model::market::source::kraken_asset(&id)),
                }
            }
            _ => self.symbol.clone(),
        }
    }

    /// Whether both name one market on one provider, comparing by the id the
    /// provider's API takes.
    pub fn is_same_market(&self, other: &InstrumentId) -> bool {
        self.source == other.source
            && self
                .canonical_symbol()
                .eq_ignore_ascii_case(&other.canonical_symbol())
    }

    /// The key of the market a stored `key()` string names, spelled the way `key()` spells
    /// it now; `None` for a string that is not a key. Flags saved before aliases shared a
    /// key (`BINANCE:BTC`) are moved to the key of the market they were set on.
    pub fn rekey(key: &str) -> Option<String> {
        let (slug, symbol) = key.split_once(':')?;
        let source = DataSource::from_watchlist_slug(slug)?;
        Some(Self::new(source, symbol).key())
    }

    /// `key` as it is spelled now, when it is a stored key releases up to 0.6.0-rc.37 made for a
    /// market by quoting its pair in USDT again (`BINANCE:SUSDTUSDT` for `SUSDT`): they did not
    /// read the pair and took it for a bare asset. Drawings and script alerts are filed by key,
    /// and read theirs through this.
    pub fn current_key(key: &str) -> Option<String> {
        let (slug, symbol) = key.split_once(':')?;
        let source = DataSource::from_watchlist_slug(slug)?;
        if !matches!(source, DataSource::Binance | DataSource::Bybit) {
            return None;
        }
        let pair = symbol.strip_suffix("USDT")?;
        let instrument = Self::new(source, pair);
        (!released_as_pair(pair) && instrument.canonical_symbol() == pair).then(|| instrument.key())
    }

    fn normalized(symbol: &str, source: DataSource) -> String {
        match source {
            DataSource::Binance
            | DataSource::Bybit
            | DataSource::Mexc
            | DataSource::Coinbase
            | DataSource::Okx
            | DataSource::Kraken
            | DataSource::Gate => Self::new(source, symbol).canonical_symbol(),
            DataSource::Alpaca | DataSource::Oanda => symbol.to_uppercase(),
            DataSource::CoinGecko => symbol.to_lowercase(),
            _ => symbol.to_owned(),
        }
    }
}

/// The quotes releases up to 0.6.0-rc.37 read in Binance and Bybit ids. Keys they stored were
/// spelled by this list, so it does not change.
const RELEASED_QUOTES: [&str; 16] = [
    "FDUSD", "USDT", "USDC", "BUSD", "TUSD", "USDP", "DAI", "EUR", "GBP", "TRY", "BRL", "JPY",
    "USD", "BTC", "ETH", "BNB",
];

/// Whether those releases read `symbol` as a pair, and so left it as it was.
fn released_as_pair(symbol: &str) -> bool {
    RELEASED_QUOTES
        .iter()
        .find_map(|quote| symbol.strip_suffix(quote).filter(|base| !base.is_empty()))
        .is_some_and(|base| base.len() > 1)
}

/// `pair` as Binance and Bybit write it: a bare asset is quoted in USDT.
fn quoted_in_usdt(pair: &str) -> String {
    if crate::model::market::source::split_binance_pair(pair).is_some() {
        pair.to_owned()
    } else {
        format!("{pair}USDT")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_a_release_made_by_quoting_a_pair_again_is_read_as_the_pairs() {
        for source in ["BINANCE", "BYBIT"] {
            let current = |symbol: &str| InstrumentId::current_key(&format!("{source}:{symbol}"));
            for (stored, pair) in [("SUSDTUSDT", "SUSDT"), ("AUSD1USDT", "AUSD1")] {
                assert_eq!(
                    current(stored),
                    Some(format!("{source}:{pair}")),
                    "{stored}"
                );
            }
            // Keys that name a USDT market, as the releases and this build both spell it.
            for kept in [
                "BTCUSDT",
                "WBTCUSDT",
                "TUSDUSDT",
                "BUSDUSDT",
                "XBUSDUSDT",
                "SUSDT",
                "ETHBTC",
            ] {
                assert_eq!(current(kept), None, "{kept}");
            }
        }
        assert_eq!(InstrumentId::current_key("COINBASE:SUSDTUSDT"), None);
        assert_eq!(InstrumentId::current_key("nonsense"), None);
    }

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
        for symbol in ["BTC", "btc", "BTCUSDT", "btc-usdt", "BTC/USDT"] {
            assert_eq!(key(DataSource::Bybit, symbol), "BYBIT:BTCUSDT", "{symbol}");
        }
        for symbol in ["BTC", "btc_usdt", "BTC/USDT", "btc-usdt"] {
            assert_eq!(key(DataSource::Gate, symbol), "GATEIO:BTC_USDT", "{symbol}");
        }
        for symbol in ["BTC", "btcusdt", "btc-usdt", "BTC/USDT"] {
            assert_eq!(key(DataSource::Mexc, symbol), "MEXC:BTCUSDT", "{symbol}");
        }
        // Ids MEXC lists as they are: a quote that is not Binance's, a one-letter base.
        for (symbol, id) in [
            ("BTCUSD1", "BTCUSD1"),
            ("btc/usd1", "BTCUSD1"),
            ("SUSDT", "SUSDT"),
            ("s-usdt", "SUSDT"),
            ("ETHUSDE", "ETHUSDE"),
            ("ETHBTC", "ETHBTC"),
            ("SOLEUR", "SOLEUR"),
            // A bare asset is quoted in USDT, a wrapped one included.
            ("WBTC", "WBTCUSDT"),
            ("S", "SUSDT"),
            ("USD1", "USD1USDT"),
        ] {
            assert_eq!(
                key(DataSource::Mexc, symbol),
                format!("MEXC:{id}"),
                "{symbol}"
            );
        }
        for symbol in ["BTC", "btc-usdt", "BTC/USDT", "BTC-USDT"] {
            assert_eq!(key(DataSource::Okx, symbol), "OKX:BTC-USDT", "{symbol}");
        }
        for symbol in ["BTC", "btc/usd", "XBT/USD", "xbt-usd", "BTC USD"] {
            assert_eq!(
                key(DataSource::Kraken, symbol),
                "KRAKEN:BTC/USD",
                "{symbol}"
            );
        }
        assert_eq!(key(DataSource::Kraken, "xdg/eur"), "KRAKEN:DOGE/EUR");
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
    fn canonical_symbol_quotes_bare_binance_and_coinbase() {
        assert_eq!(
            InstrumentId::new(DataSource::Binance, "BTC").canonical_symbol(),
            "BTCUSDT"
        );
        assert_eq!(
            InstrumentId::new(DataSource::Binance, "ETHUSDT").canonical_symbol(),
            "ETHUSDT"
        );
        assert_eq!(
            InstrumentId::new(DataSource::Coinbase, "BTC").canonical_symbol(),
            "BTC-USD"
        );
        assert_eq!(
            InstrumentId::new(DataSource::Coinbase, "ETH/USD").canonical_symbol(),
            "ETH-USD"
        );
    }

    #[test]
    fn canonical_symbol_keeps_any_quoted_binance_pair() {
        for pair in ["SOLBNB", "XRPBTC", "ETHFDUSD", "sol/bnb"] {
            let api = InstrumentId::new(DataSource::Binance, pair).canonical_symbol();
            assert_eq!(api, pair.to_uppercase().replace('/', ""));
        }
        assert_eq!(
            InstrumentId::new(DataSource::Binance, "WBTC").canonical_symbol(),
            "WBTCUSDT"
        );
    }

    #[test]
    fn a_one_letter_base_is_an_asset_before_money_only() {
        for source in [DataSource::Binance, DataSource::Bybit] {
            for (typed, api) in [
                ("SUSDT", "SUSDT"),
                ("w/usdc", "WUSDC"),
                ("AUSD1", "AUSD1"),
                ("S", "SUSDT"),
                ("WBTC", "WBTCUSDT"),
                ("TUSD", "TUSDUSDT"),
                ("BUSD", "BUSDUSDT"),
                // The longer quote decides: these are not XB/USD, ST/USD or AB/USD.
                ("XBUSD", "XBUSDUSDT"),
                ("STUSD", "STUSDUSDT"),
                ("ABUSD", "ABUSDUSDT"),
            ] {
                assert_eq!(
                    InstrumentId::new(source, typed).canonical_symbol(),
                    api,
                    "{typed}"
                );
            }
            assert_eq!(source.base_asset("SUSDT"), "S");
            assert_eq!(source.quote_asset("SUSDT").as_deref(), Some("USDT"));
            assert_eq!(source.quote_asset("WBTC"), None);
        }
    }

    #[test]
    fn a_listed_pair_keeps_its_name_whatever_it_is_priced_in() {
        let canonical = |source, symbol: &str| InstrumentId::new(source, symbol).canonical_symbol();
        for source in [DataSource::Binance, DataSource::Bybit] {
            for pair in [
                "BTCARS",
                "BTCMXN",
                "BTCCOP",
                "USDTZAR",
                "BTCIDR",
                "BTCTHB",
                "USDTKZT",
                "USDTAED",
                "BTCPLN",
                "BTCUSDS",
                "BTCUSDE",
                "BTCRLUSD",
                "EUREURI",
                "BTCU",
                "TRXXRP",
                "BNSOLSOL",
                "PENGUUSDT",
                "STRY",
                "SEUR",
                "UIDR",
                "UUSD",
            ] {
                assert_eq!(canonical(source, pair), pair, "{source:?}");
            }
            // Halted markets, whose history is still asked for.
            for pair in [
                "BNBPAX",
                "BTCAUD",
                "BTCRUB",
                "BTCNGN",
                "BTCUAH",
                "BTCRON",
                "BTCBKRW",
                "BTCIDRT",
                "BTCBVND",
                "BTCBIDR",
                "BTCUST",
                "BTCVAI",
                "BNBUSDSOLD",
                "BTTTRX",
            ] {
                assert_eq!(canonical(source, pair), pair, "{source:?}");
            }
            // Bare assets that end like a quote, or are one letter before a coin or `U`.
            for asset in [
                "PENGU", "XAU", "TRU", "BNSOL", "WBETH", "STETH", "PYUSD", "SUSD", "GUSD", "MU",
                "WBTC", "WSOL", "WXRP", "TRUST", "KORUB", "BABYDOGE",
            ] {
                assert_eq!(
                    canonical(source, asset),
                    format!("{asset}USDT"),
                    "{source:?}"
                );
            }
        }
        assert_eq!(canonical(DataSource::Binance, "btcu.p"), "BTCU.P");
    }

    #[test]
    fn a_binance_perpetual_is_another_market_than_its_spot_pair() {
        let spot = InstrumentId::new(DataSource::Binance, "BTCUSDT");
        let perp = InstrumentId::new(DataSource::Binance, "btc.p");
        assert_eq!(perp.canonical_symbol(), "BTCUSDT.P");
        assert_ne!(perp.key(), spot.key());
        assert!(!perp.is_same_market(&spot));
        assert!(perp.is_same_market(&InstrumentId::new(DataSource::Binance, "BTC/USDT.P")));
        assert_eq!(DataSource::Binance.base_asset("BTCUSDT.P"), "BTC");
        assert_eq!(
            DataSource::Binance.quote_asset("btcusdt.p").as_deref(),
            Some("USDT")
        );
        assert_eq!(DataSource::Binance.base_asset("SUSDT.P"), "S");
    }

    #[test]
    fn same_market_compares_by_canonical_symbol() {
        let bare = InstrumentId::new(DataSource::Binance, "BTC");
        let quoted = InstrumentId::new(DataSource::Binance, "BTCUSDT");
        assert!(bare.is_same_market(&quoted));
        assert!(!bare.is_same_market(&InstrumentId::new(DataSource::Coinbase, "BTC")));
    }
}
