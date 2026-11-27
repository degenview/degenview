use crate::model::market::binance::split_market;
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
    CoinGecko,
    DexScreener,
    Alpaca,
    Oanda,
    Polymarket,
    Kalshi,
    CoinMarketCap,
}

impl DataSource {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Binance => "Binance",
            Self::Coinbase => "Coinbase",
            Self::Okx => "OKX",
            Self::Kraken => "Kraken",
            Self::Bybit => "Bybit",
            Self::Gate => "Gate.io",
            Self::Mexc => "MEXC",
            Self::CoinGecko => "CoinGecko",
            Self::DexScreener => "DEXScreener",
            Self::Alpaca => "Alpaca (IEX)",
            Self::Oanda => "Oanda",
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
            Self::Okx => "OKX",
            Self::Kraken => "KRAKEN",
            Self::Bybit => "BYBIT",
            Self::Gate => "GATEIO",
            Self::Mexc => "MEXC",
            Self::CoinGecko => "COINGECKO",
            Self::DexScreener => "DEXSCREENER",
            Self::Alpaca => "ALPACA",
            Self::Oanda => "OANDA",
            Self::Polymarket => "POLYMARKET",
            Self::Kalshi => "KALSHI",
            Self::CoinMarketCap => "CMC",
        }
    }

    /// Every source, in the order the pickers list them.
    pub const ALL: [DataSource; 14] = [
        DataSource::Binance,
        DataSource::Coinbase,
        DataSource::Okx,
        DataSource::Kraken,
        DataSource::Bybit,
        DataSource::Gate,
        DataSource::Mexc,
        DataSource::CoinGecko,
        DataSource::DexScreener,
        DataSource::Alpaca,
        DataSource::Oanda,
        DataSource::Polymarket,
        DataSource::Kalshi,
        DataSource::CoinMarketCap,
    ];

    /// The source a `watchlist_slug` names, in any case.
    pub fn from_watchlist_slug(slug: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|source| source.watchlist_slug().eq_ignore_ascii_case(slug))
    }

    /// The sources whose forming candle a chart takes from a socket instead of waiting for the
    /// next refresh.
    pub const CANDLE_STREAMS: [DataSource; 6] = [
        DataSource::Binance,
        DataSource::Okx,
        DataSource::Kraken,
        DataSource::Bybit,
        DataSource::Gate,
        DataSource::Mexc,
    ];

    /// The sources whose watchlist prices come from a shared ticker socket, REST filling in.
    pub const QUOTE_STREAMS: [DataSource; 5] = [
        DataSource::Coinbase,
        DataSource::Okx,
        DataSource::Kraken,
        DataSource::Bybit,
        DataSource::Gate,
    ];

    /// Whether watchlist prices of this source come from a ticker socket.
    pub fn streams_quotes(self) -> bool {
        Self::QUOTE_STREAMS.contains(&self)
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
            Self::Binance
                | Self::Coinbase
                | Self::Okx
                | Self::Kraken
                | Self::Bybit
                | Self::Gate
                | Self::Mexc
                | Self::DexScreener
                | Self::Alpaca
                | Self::Oanda
        )
    }

    /// Whether this source's data is drawn as a line rather than candles.
    pub fn is_line_chart(self) -> bool {
        self.is_prediction_market() || matches!(self, Self::CoinMarketCap)
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
        match self {
            Self::Binance
            | Self::Coinbase
            | Self::Okx
            | Self::Kraken
            | Self::Bybit
            | Self::Gate
            | Self::Mexc
            | Self::CoinGecko
            | Self::Alpaca
            | Self::Oanda => raw.to_uppercase(),
            Self::CoinMarketCap => super::cmc::title(raw).to_owned(),
            _ => raw.to_owned(),
        }
    }

    /// The asset a market trades, for artwork and tickers: `BTC` for `BTCUSDT`, `BTC-USD`,
    /// `BTC/USDT` and `BTC — Grayscale Bitcoin Mini Trust`. A symbol that names no pair is returned
    /// as it is. Never shown as a market's name: use [`Self::display_symbol`] for that.
    pub fn base_asset(self, symbol: &str) -> String {
        let upper = symbol.trim().to_uppercase();
        let head = upper.split(['/', '-', ' ', '_']).next().unwrap_or(&upper);
        let head = match self {
            Self::Binance => split_market(head).0,
            _ => head,
        };
        match self {
            Self::Binance | Self::Bybit => split_binance_pair(head)
                .map(|(base, _)| base)
                .unwrap_or(head),
            Self::Mexc => split_mexc_pair(head).map(|(base, _)| base).unwrap_or(head),
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
            Self::Binance => {
                split_binance_pair(split_market(&upper).0).map(|(_, quote)| quote.to_owned())
            }
            Self::Bybit => split_binance_pair(&upper).map(|(_, quote)| quote.to_owned()),
            Self::Mexc => split_mexc_pair(&upper).map(|(_, quote)| quote.to_owned()),
            Self::Kraken => {
                let (base, quote) = upper.split_once('/')?;
                (!base.is_empty() && !quote.is_empty() && !quote.contains(['-', '/', ' ']))
                    .then(|| quote.to_owned())
            }
            Self::Gate | Self::Oanda => {
                let (base, quote) = upper.split_once('_')?;
                (!base.is_empty() && !quote.is_empty() && !quote.contains(['-', '/', ' ', '_']))
                    .then(|| quote.to_owned())
            }
            Self::Coinbase | Self::Okx => {
                let (base, quote) = upper.split_once('-')?;
                (!base.is_empty() && !quote.is_empty() && !quote.contains(['-', '/', ' ']))
                    .then(|| quote.to_owned())
            }
            _ => None,
        }
    }
}

/// Kraken's own spelling of an asset is the legacy one (`XBT`, `XDG`) in some places; its APIs take
/// the common ticker (`BTC`, `DOGE`) and refuse the legacy one.
pub fn kraken_asset(asset: &str) -> String {
    match asset.to_uppercase().as_str() {
        "XBT" => "BTC".to_owned(),
        "XDG" => "DOGE".to_owned(),
        other => other.to_owned(),
    }
}

/// Quote assets Binance pairs end in, longest first so BTCFDUSD is BTC/FDUSD instead of
/// BTCFD/USD. Matches Swift's MarketSymbol quote list, plus `USD1`.
const BINANCE_QUOTES: [&str; 17] = [
    "FDUSD", "USDT", "USDC", "USD1", "BUSD", "TUSD", "USDP", "DAI", "EUR", "GBP", "TRY", "BRL",
    "JPY", "USD", "BTC", "ETH", "BNB",
];

/// Quote assets MEXC's spot catalog ends its ids in, the dollar stablecoins first.
const MEXC_QUOTES: [&str; 9] = [
    "USDT", "USDC", "USD1", "USDE", "USDF", "EUR", "BRL", "ETH", "BTC",
];

/// Quotes before which a one-letter base is a real asset (`SUSDT` is S/USDT). Before any other
/// it is part of the asset's own name: `WBTC` is not W/BTC, `TUSD` not T/USD.
const ONE_LETTER_BASE_QUOTES: [&str; 6] = ["FDUSD", "USDT", "USDC", "USD1", "USDE", "USDF"];

/// Splits an upper-case id that writes base and quote together. The first of `quotes` that ends
/// it decides: a shorter quote inside it (`USD` in `BUSD`) is never tried.
fn split_joined<'a>(symbol: &'a str, quotes: &[&'static str]) -> Option<(&'a str, &'static str)> {
    let (base, quote) = quotes
        .iter()
        .find_map(|quote| Some((symbol.strip_suffix(quote)?, *quote)))?;
    let named = base.len() > 1 || (base.len() == 1 && ONE_LETTER_BASE_QUOTES.contains(&quote));
    named.then_some((base, quote))
}

/// Splits an upper-case Binance or Bybit pair like `SOLBNB` into `("SOL", "BNB")`, or `None`
/// when it names no pair (a bare asset such as `SOL` or `WBTC`).
pub fn split_binance_pair(symbol: &str) -> Option<(&str, &'static str)> {
    split_joined(symbol, &BINANCE_QUOTES)
}

/// Splits an upper-case MEXC id like `BTCUSD1` into `("BTC", "USD1")`, or `None` when it names
/// no pair of its catalog (a bare asset such as `SOL` or `WBTC`).
pub fn split_mexc_pair(symbol: &str) -> Option<(&str, &'static str)> {
    split_joined(symbol, &MEXC_QUOTES)
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
        assert_eq!(Oanda.quote_asset("eur_usd").as_deref(), Some("USD"));
        assert_eq!(Oanda.quote_asset("xau_usd").as_deref(), Some("USD"));
        assert_eq!(Oanda.quote_asset("EURUSD"), None);
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
        assert_eq!(DataSource::Oanda.display_symbol(" eur_usd "), "EUR_USD");
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
