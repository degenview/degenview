use serde::{Deserialize, Serialize};

/// A market data provider. The symbol alone never identifies a market: `BTCUSDT`
/// on Binance and `BTC-USD` on Coinbase are different instruments.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DataSource {
    Binance,
    Coinbase,
    CoinGecko,
    DexScreener,
    Alpaca,
    Polymarket,
    Kalshi,
    CoinMarketCap,
}

impl DataSource {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Binance => "Binance",
            Self::Coinbase => "Coinbase",
            Self::CoinGecko => "CoinGecko",
            Self::DexScreener => "DEXScreener",
            Self::Alpaca => "Alpaca (IEX)",
            Self::Polymarket => "Polymarket",
            Self::Kalshi => "Kalshi",
            Self::CoinMarketCap => "CoinMarketCap",
        }
    }

    /// Upper-case provider prefix in the import/export text format.
    pub fn watchlist_slug(self) -> &'static str {
        match self {
            Self::Binance => "BINANCE",
            Self::Coinbase => "COINBASE",
            Self::CoinGecko => "COINGECKO",
            Self::DexScreener => "DEXSCREENER",
            Self::Alpaca => "ALPACA",
            Self::Polymarket => "POLYMARKET",
            Self::Kalshi => "KALSHI",
            Self::CoinMarketCap => "CMC",
        }
    }

    /// The source a `watchlist_slug` names, in any case.
    pub fn from_watchlist_slug(slug: &str) -> Option<Self> {
        const ALL: [DataSource; 8] = [
            DataSource::Binance,
            DataSource::Coinbase,
            DataSource::CoinGecko,
            DataSource::DexScreener,
            DataSource::Alpaca,
            DataSource::Polymarket,
            DataSource::Kalshi,
            DataSource::CoinMarketCap,
        ];
        ALL.into_iter()
            .find(|source| source.watchlist_slug().eq_ignore_ascii_case(slug))
    }

    /// Whether this source quotes probabilities in 0…1, charted as lines.
    pub fn is_prediction_market(self) -> bool {
        matches!(self, Self::Polymarket | Self::Kalshi)
    }

    /// Whether this source reports per-candle turnover for the volume bars to draw. CoinGecko's
    /// OHLC endpoint has no volume column, and the line charts report none at all.
    pub fn provides_volume(self) -> bool {
        matches!(
            self,
            Self::Binance | Self::Coinbase | Self::DexScreener | Self::Alpaca
        )
    }

    /// Whether this source can serve the candles before a given time at `timeframe`, which
    /// scrolling back past what a refresh brings needs. GeckoTerminal's free API reaches back only
    /// 180 days, so only its hourly bars have a second page; its daily ones arrive whole.
    pub fn supports_older_history(self, timeframe: super::Timeframe) -> bool {
        match self {
            Self::Binance | Self::Coinbase | Self::Alpaca => true,
            Self::DexScreener => timeframe == super::Timeframe::Hour,
            _ => false,
        }
    }

    /// Whether this source's data is drawn as a line rather than candles.
    pub fn is_line_chart(self) -> bool {
        self.is_prediction_market() || matches!(self, Self::CoinMarketCap)
    }

    /// The market's name as its exchange writes it: `BTCUSDT` on Binance, `BTC-USD` on Coinbase,
    /// `AAPL` on Alpaca. Nothing is reformatted, only upper-cased where the exchange's own ids are
    /// (CoinGecko ids and Alpaca tickers). A chart, a search hit and a watchlist row all use it.
    pub fn display_symbol(self, symbol: &str) -> String {
        let raw = symbol.trim();
        match self {
            Self::Binance | Self::Coinbase | Self::CoinGecko | Self::Alpaca => raw.to_uppercase(),
            Self::CoinMarketCap => crate::service::cmc::title(raw).to_owned(),
            _ => raw.to_owned(),
        }
    }

    /// The asset a market trades, for artwork and tickers: `BTC` for `BTCUSDT`, `BTC-USD`,
    /// `BTC/USDT` and `BTC — Grayscale Bitcoin Mini Trust`. A symbol that names no pair is returned
    /// as it is. Never shown as a market's name: use [`Self::display_symbol`] for that.
    pub fn base_asset(self, symbol: &str) -> String {
        let upper = symbol.trim().to_uppercase();
        let head = upper.split(['/', '-', ' ']).next().unwrap_or(&upper);
        match self {
            // A one-letter "base" is a bare wrapped asset (WBTC, WETH), not a pair.
            Self::Binance => split_binance_pair(head)
                .map(|(base, _)| base)
                .filter(|base| base.len() > 1)
                .unwrap_or(head),
            _ => head,
        }
        .to_owned()
    }
}

impl DataSource {
    /// The quote asset of a pair the exchange itself spells out: `EUR` of Coinbase's `ADA-EUR`, of
    /// Binance's `ADAEUR`. None for anything that does not name one (a stock, a prediction market,
    /// an aggregator's coin id): a pair's parts are never guessed.
    pub fn quote_asset(self, symbol: &str) -> Option<String> {
        let upper = symbol.trim().to_uppercase();
        match self {
            Self::Binance => split_binance_pair(&upper).map(|(_, quote)| quote.to_owned()),
            Self::Coinbase => {
                let (base, quote) = upper.split_once('-')?;
                (!base.is_empty() && !quote.is_empty() && !quote.contains(['-', '/', ' ']))
                    .then(|| quote.to_owned())
            }
            _ => None,
        }
    }
}

/// Quote assets Binance pairs end in, longest first so BTCFDUSD is BTC/FDUSD instead of
/// BTCFD/USD. Matches Swift's MarketSymbol quote list.
const BINANCE_QUOTES: [&str; 16] = [
    "FDUSD", "USDT", "USDC", "BUSD", "TUSD", "USDP", "DAI", "EUR", "GBP", "TRY", "BRL", "JPY",
    "USD", "BTC", "ETH", "BNB",
];

/// Splits an upper-case Binance pair like `SOLBNB` into `("SOL", "BNB")`, or `None` when no
/// known quote ends it (a bare asset such as `SOL`).
pub fn split_binance_pair(symbol: &str) -> Option<(&str, &'static str)> {
    BINANCE_QUOTES.iter().find_map(|quote| {
        symbol
            .strip_suffix(quote)
            .filter(|base| !base.is_empty())
            .map(|base| (base, *quote))
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_pair_the_exchange_spells_out_has_a_quote_and_nothing_else_does() {
        use super::DataSource::*;
        assert_eq!(Coinbase.quote_asset("ADA-EUR").as_deref(), Some("EUR"));
        assert_eq!(Binance.quote_asset("adaeur").as_deref(), Some("EUR"));
        assert_eq!(Binance.quote_asset("SOL"), None);
        assert_eq!(Coinbase.quote_asset("BTC"), None);
        assert_eq!(Alpaca.quote_asset("BRK-B"), None);
        assert_eq!(Kalshi.quote_asset("KXFED-23DEC"), None);
    }

    use super::*;

    #[test]
    fn markets_are_named_as_their_exchange_names_them() {
        assert_eq!(DataSource::Binance.display_symbol("btcusdt"), "BTCUSDT");
        assert_eq!(DataSource::Binance.display_symbol("BTC"), "BTC");
        assert_eq!(DataSource::Coinbase.display_symbol("btc-usd"), "BTC-USD");
        assert_eq!(DataSource::Alpaca.display_symbol(" aapl "), "AAPL");
        // A DEX pair address is an id, not a name: it is left alone.
        assert_eq!(DataSource::DexScreener.display_symbol("0xAbC"), "0xAbC");
    }

    #[test]
    fn the_base_asset_comes_out_of_any_spelling_of_a_market() {
        let base = |source: DataSource, symbol| source.base_asset(symbol);
        assert_eq!(base(DataSource::Binance, "BTCUSDT"), "BTC");
        assert_eq!(base(DataSource::Binance, "BTCFDUSD"), "BTC");
        assert_eq!(base(DataSource::Binance, "ETHBTC"), "ETH");
        assert_eq!(base(DataSource::Binance, "BTC"), "BTC");
        assert_eq!(base(DataSource::Binance, "WBTC"), "WBTC");
        assert_eq!(base(DataSource::Binance, "BTC/USDT"), "BTC");
        assert_eq!(base(DataSource::Coinbase, "btc-usd"), "BTC");
        assert_eq!(base(DataSource::Alpaca, "AAPL — Apple Inc."), "AAPL");
        assert_eq!(base(DataSource::DexScreener, "PEPE/WETH"), "PEPE");
    }

    #[test]
    fn only_sources_with_per_candle_turnover_provide_volume() {
        assert!(DataSource::Binance.provides_volume());
        assert!(DataSource::Alpaca.provides_volume());
        assert!(!DataSource::CoinGecko.provides_volume());
        assert!(!DataSource::Polymarket.provides_volume());
        assert!(!DataSource::CoinMarketCap.provides_volume());
    }

    #[test]
    fn older_pages_depend_on_the_source_and_the_timeframe() {
        use super::super::Timeframe;
        for timeframe in Timeframe::ALL {
            assert!(DataSource::Binance.supports_older_history(timeframe));
            assert!(DataSource::Coinbase.supports_older_history(timeframe));
            assert!(!DataSource::CoinGecko.supports_older_history(timeframe));
            assert!(!DataSource::Kalshi.supports_older_history(timeframe));
        }
        // GeckoTerminal's free API reaches back 180 days: only hourly bars have a second page.
        assert!(DataSource::DexScreener.supports_older_history(Timeframe::Hour));
        assert!(!DataSource::DexScreener.supports_older_history(Timeframe::Day));
        assert!(!DataSource::DexScreener.supports_older_history(Timeframe::Week));
    }
}
