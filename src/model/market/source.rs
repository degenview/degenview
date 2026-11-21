use super::profile::{Casing, Class, Series};
use serde::{Deserialize, Serialize};

/// A market data provider. The symbol alone never identifies a market: `BTCUSDT`
/// on Binance and Bybit (the same name on both: they are still two markets), `BTC-USD` on Coinbase, `BTC-USDT` on OKX and `BTC/USD` on Kraken `BTC_USDT` on Gate.io and `BTCUSDT` on MEXC are different
/// instruments.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DataSource {
    Binance,
    Coinbase,
    Okx,
    Kraken,
    Bybit,
    Gate,
    Mexc,
    Hyperliquid,
    Aster,
    Lighter,
    Deribit,
    Htx,
    CoinGecko,
    DexScreener,
    Alpaca,
    Polymarket,
    Kalshi,
    CoinMarketCap,
}

impl DataSource {
    pub fn display_name(self) -> &'static str {
        self.profile().display_name
    }

    /// Upper-case provider prefix in the import/export text format.
    pub fn watchlist_slug(self) -> &'static str {
        self.profile().slug
    }

    /// The source a `watchlist_slug` names, in any case.
    pub fn from_watchlist_slug(slug: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|source| source.watchlist_slug().eq_ignore_ascii_case(slug))
    }

    /// Whether this source quotes probabilities in 0…1, charted as lines.
    pub fn is_prediction_market(self) -> bool {
        self.profile().class == Class::Prediction
    }

    /// Whether this source reports per-candle turnover for the volume bars to draw. CoinGecko's
    /// OHLC endpoint has no volume column, and the line charts report none at all.
    pub fn provides_volume(self) -> bool {
        self.profile().volume
    }

    /// Whether this source's data is drawn as a line rather than candles.
    pub fn is_line_chart(self) -> bool {
        self.profile().series == Series::Line
    }

    /// Whether a chart of this source can carry indicators and scripts. A prediction market or a
    /// CoinMarketCap gauge is a probability or an index line with no price to study.
    pub fn takes_indicators(self) -> bool {
        !self.is_line_chart()
    }

    /// The market's name as its exchange writes it: `BTCUSDT` on Binance, `BTC-USD` on Coinbase,
    /// `AAPL` on Alpaca. Nothing is reformatted, only upper-cased where the exchange's own ids are
    /// (CoinGecko ids and Alpaca tickers). A chart, a search hit and a watchlist row all use it.
    pub fn display_symbol(self, symbol: &str) -> String {
        let raw = symbol.trim();
        let profile = self.profile();
        if profile.class == Class::Index {
            return super::cmc::title(raw).to_owned();
        }
        match profile.display {
            Casing::Upper => raw.to_uppercase(),
            Casing::Lower => raw.to_lowercase(),
            Casing::AsGiven => raw.to_owned(),
        }
    }

    /// The asset a market trades, for artwork and tickers: `BTC` for `BTCUSDT`, `BTC-USD`,
    /// `BTC/USDT` and `BTC — Grayscale Bitcoin Mini Trust`. A symbol that names no pair is returned
    /// as it is. Never shown as a market's name: use [`Self::display_symbol`] for that.
    pub fn base_asset(self, symbol: &str) -> String {
        let upper = symbol.trim().to_uppercase();
        let head = upper.split(['/', '-', ' ', '_']).next().unwrap_or(&upper);
        self.profile().pair.base_of(head).to_owned()
    }

    /// The quote asset of a pair the exchange itself spells out: `EUR` of Coinbase's `ADA-EUR`, of
    /// Binance's `ADAEUR`. None for anything that does not name one (a stock, a prediction market,
    /// an aggregator's coin id): a pair's parts are never guessed.
    pub fn quote_asset(self, symbol: &str) -> Option<String> {
        self.profile().pair.quote_of(&symbol.trim().to_uppercase())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_pair_the_exchange_spells_out_has_a_quote_and_nothing_else_does() {
        use super::DataSource::*;
        assert_eq!(Coinbase.quote_asset("ADA-EUR").as_deref(), Some("EUR"));
        assert_eq!(Okx.quote_asset("sol-usdt").as_deref(), Some("USDT"));
        assert_eq!(Kraken.quote_asset("eth/eur").as_deref(), Some("EUR"));
        assert_eq!(Gate.quote_asset("sol_usdt").as_deref(), Some("USDT"));
        assert_eq!(Gate.quote_asset("SOLUSDT"), None);
        assert_eq!(Kraken.quote_asset("ETHEUR"), None);
        // A perpetual swap's id has three parts: not a spot pair.
        assert_eq!(Okx.quote_asset("BTC-USDT-SWAP"), None);
        assert_eq!(Binance.quote_asset("adaeur").as_deref(), Some("EUR"));
        assert_eq!(Bybit.quote_asset("adaeur").as_deref(), Some("EUR"));
        assert_eq!(Mexc.quote_asset("adausdc").as_deref(), Some("USDC"));
        assert_eq!(Mexc.quote_asset("BTCUSD1").as_deref(), Some("USD1"));
        assert_eq!(Mexc.quote_asset("SUSDT").as_deref(), Some("USDT"));
        assert_eq!(Mexc.quote_asset("WBTC"), None);
        assert_eq!(Bybit.quote_asset("SOL"), None);
        assert_eq!(Binance.quote_asset("SOL"), None);
        assert_eq!(Coinbase.quote_asset("BTC"), None);
        assert_eq!(Alpaca.quote_asset("BRK-B"), None);
        assert_eq!(Kalshi.quote_asset("KXFED-23DEC"), None);
    }

    use super::*;

    #[test]
    fn markets_are_named_as_their_exchange_names_them() {
        assert_eq!(DataSource::Binance.display_symbol("btcusdt"), "BTCUSDT");
        assert_eq!(DataSource::Bybit.display_symbol("btcusdt"), "BTCUSDT");
        assert_eq!(DataSource::Binance.display_symbol("BTC"), "BTC");
        assert_eq!(DataSource::Coinbase.display_symbol("btc-usd"), "BTC-USD");
        assert_eq!(DataSource::Okx.display_symbol("btc-usdt"), "BTC-USDT");
        assert_eq!(DataSource::Kraken.display_symbol("btc/usd"), "BTC/USD");
        assert_eq!(DataSource::Gate.display_symbol("btc_usdt"), "BTC_USDT");
        assert_eq!(DataSource::Mexc.display_symbol("btcusdt"), "BTCUSDT");
        assert_eq!(DataSource::Alpaca.display_symbol(" aapl "), "AAPL");
        // A DEX pair address is an id, not a name: it is left alone.
        assert_eq!(DataSource::DexScreener.display_symbol("0xAbC"), "0xAbC");
    }

    #[test]
    fn the_base_asset_comes_out_of_any_spelling_of_a_market() {
        let base = |source: DataSource, symbol| source.base_asset(symbol);
        assert_eq!(base(DataSource::Binance, "BTCUSDT"), "BTC");
        assert_eq!(base(DataSource::Binance, "BTCFDUSD"), "BTC");
        assert_eq!(base(DataSource::Bybit, "ethusdc"), "ETH");
        assert_eq!(base(DataSource::Bybit, "WBTC"), "WBTC");
        assert_eq!(base(DataSource::Binance, "ETHBTC"), "ETH");
        assert_eq!(base(DataSource::Binance, "BTC"), "BTC");
        assert_eq!(base(DataSource::Binance, "WBTC"), "WBTC");
        assert_eq!(base(DataSource::Binance, "BTC/USDT"), "BTC");
        assert_eq!(base(DataSource::Coinbase, "btc-usd"), "BTC");
        assert_eq!(base(DataSource::Okx, "eth-usdt"), "ETH");
        assert_eq!(base(DataSource::Kraken, "sol/usd"), "SOL");
        assert_eq!(base(DataSource::Gate, "eth_usdt"), "ETH");
        assert_eq!(base(DataSource::Mexc, "solusdt"), "SOL");
        assert_eq!(base(DataSource::Mexc, "BTCUSD1"), "BTC");
        assert_eq!(base(DataSource::Mexc, "susdt"), "S");
        assert_eq!(base(DataSource::Mexc, "WBTC"), "WBTC");
        assert_eq!(base(DataSource::Gate, "BTC3L_USDT"), "BTC3L");
        assert_eq!(base(DataSource::Alpaca, "AAPL — Apple Inc."), "AAPL");
        assert_eq!(base(DataSource::DexScreener, "PEPE/WETH"), "PEPE");
    }

    #[test]
    fn only_sources_with_per_candle_turnover_provide_volume() {
        assert!(DataSource::Binance.provides_volume());
        assert!(DataSource::Alpaca.provides_volume());
        assert!(DataSource::Okx.provides_volume());
        assert!(DataSource::Kraken.provides_volume());
        assert!(DataSource::Bybit.provides_volume());
        assert!(DataSource::Gate.provides_volume());
        assert!(DataSource::Mexc.provides_volume());
        assert!(!DataSource::CoinGecko.provides_volume());
        assert!(!DataSource::Polymarket.provides_volume());
        assert!(!DataSource::CoinMarketCap.provides_volume());
    }
}
