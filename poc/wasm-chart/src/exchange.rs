//! The exchanges the chart can read candles from: where to ask, which candle lengths each serves,
//! how many it returns, and which of the crate's parsers reads its answer. Pure, so it is tested
//! natively; fetching is `fetch.rs`.

use degenview_market::bucket::FoldPlan;
use degenview_market::{
    Kline, ParseError, TimeUnit, Timeframe, binance, bybit, coinbase, gateio, kraken, normalize,
    okx,
};

use crate::error::ChartError;

/// The most bars the chart keeps: more would only make the first request heavier.
const MAX_SHOWN: usize = 500;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Exchange {
    Coinbase,
    Binance,
    Okx,
    Bybit,
    Kraken,
    Gate,
}

impl Exchange {
    pub fn parse(text: &str) -> Result<Self, ChartError> {
        match text {
            "coinbase" => Ok(Self::Coinbase),
            "binance" => Ok(Self::Binance),
            "okx" => Ok(Self::Okx),
            "bybit" => Ok(Self::Bybit),
            "kraken" => Ok(Self::Kraken),
            "gate" => Ok(Self::Gate),
            _ => Err(ChartError::UnknownExchange(text.to_owned())),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Coinbase => "Coinbase",
            Self::Binance => "Binance",
            Self::Okx => "OKX",
            Self::Bybit => "Bybit",
            Self::Kraken => "Kraken",
            Self::Gate => "Gate.io",
        }
    }

    /// Whether a websocket feed keeps the chart live; the others are polled.
    pub fn has_live_feed(self) -> bool {
        self == Self::Coinbase
    }

    /// The market of `asset` (`BTC`) against the dollar this exchange lists, in its own spelling.
    pub fn symbol(self, asset: &str) -> Result<String, ChartError> {
        let valid =
            (2..=10).contains(&asset.len()) && asset.chars().all(|c| c.is_ascii_alphanumeric());
        if !valid {
            return Err(ChartError::BadAsset(asset.to_owned()));
        }
        let asset = asset.to_ascii_uppercase();
        Ok(match self {
            Self::Coinbase => format!("{asset}-USD"),
            Self::Binance | Self::Bybit => format!("{asset}USDT"),
            Self::Okx => format!("{asset}-USDT"),
            Self::Kraken => format!("{asset}/USD"),
            Self::Gate => format!("{asset}_USDT"),
        })
    }

    /// The candle lengths the exchange serves itself; every other timeframe is folded from one.
    fn natives(self) -> Vec<Timeframe> {
        let lengths: &[&str] = match self {
            Self::Coinbase => &["1m", "5m", "15m", "1h", "6h", "1D"],
            Self::Binance => &[
                "1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "6h", "8h", "12h", "1D", "1W",
                "1M",
            ],
            Self::Okx => &[
                "1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "6h", "12h", "1D", "1W", "1M",
                "3M",
            ],
            Self::Bybit => &[
                "1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "6h", "12h", "1D", "1W", "1M",
            ],
            Self::Kraken => &["1m", "5m", "15m", "30m", "1h", "4h", "1D"],
            Self::Gate => &["1m", "5m", "15m", "30m", "1h", "4h", "8h", "1D", "1W", "1M"],
        };
        lengths
            .iter()
            .filter_map(|text| Timeframe::parse(text).ok())
            .collect()
    }

    /// The most candles one request returns.
    fn max_candles(self) -> usize {
        match self {
            Self::Coinbase | Self::Okx => 300,
            Self::Kraken => 720,
            Self::Binance | Self::Bybit | Self::Gate => 1_000,
        }
    }

    /// How `timeframe` is served from the exchange's own candle lengths.
    pub fn plan(self, timeframe: Timeframe) -> Result<FoldPlan, ChartError> {
        FoldPlan::new(timeframe, &self.natives()).ok_or(ChartError::Unsupported {
            exchange: self.label(),
            timeframe,
        })
    }

    /// How many whole candles of the plan's timeframe one request can fill. A fold needs one more
    /// candle's worth of base candles for the fragment the request's start cuts off.
    pub fn shown(self, plan: FoldPlan) -> usize {
        let fits = if plan.is_native() {
            self.max_candles()
        } else {
            (self.max_candles() / plan.per_target()).saturating_sub(1)
        };
        fits.clamp(1, MAX_SHOWN)
    }

    /// The request for `count` base candles of `symbol` (an exchange that takes no count sends
    /// its newest ones).
    pub fn candles_url(self, symbol: &str, base: Timeframe, count: usize) -> String {
        let count = count.min(self.max_candles());
        match self {
            Self::Coinbase => format!(
                "https://api.exchange.coinbase.com/products/{symbol}/candles?granularity={}",
                base.nominal_ms() / 1_000
            ),
            Self::Binance => format!(
                "https://api.binance.com/api/v3/klines?symbol={symbol}&interval={}&limit={count}",
                base.interval()
            ),
            Self::Okx => format!(
                "https://www.okx.com/api/v5/market/candles?instId={symbol}&bar={}&limit={count}",
                okx_bar(base)
            ),
            Self::Bybit => format!(
                "https://api.bybit.com/v5/market/kline?category=spot&symbol={symbol}&interval={}&limit={count}",
                bybit_interval(base)
            ),
            Self::Kraken => format!(
                "https://api.kraken.com/0/public/OHLC?pair={symbol}&interval={}",
                base.nominal_ms() / 60_000
            ),
            Self::Gate => format!(
                "https://api.gateio.ws/api/v4/spot/candlesticks?currency_pair={symbol}&interval={}&limit={count}",
                gate_interval(base)
            ),
        }
    }

    /// Reads an answer to [`Self::candles_url`] as candles of `base`, oldest first.
    pub fn parse_candles(self, body: &str, base: Timeframe) -> Result<Vec<Kline>, ChartError> {
        let parsed: Result<Vec<Kline>, ParseError> = match self {
            Self::Coinbase => coinbase::parse_candles(body, base.nominal_ms() / 1_000),
            Self::Binance => binance::parse_candles(body),
            Self::Okx => okx::parse_candles(body, base),
            Self::Bybit => bybit::parse_candles(body, base),
            Self::Kraken => kraken::parse_candles(body, base),
            Self::Gate => gateio::parse_candles(body, base),
        };
        parsed.map(normalize).map_err(|source| match source {
            ParseError::Refused { .. } => ChartError::Refused(source.to_string()),
            source => ChartError::Parse {
                exchange: self.label(),
                source,
            },
        })
    }
}

/// OKX's bar token: minutes as they are, hours under six as `1H`, the rest in UTC (`6Hutc`,
/// `1Dutc`), because its plain ones are cut at UTC+8.
fn okx_bar(base: Timeframe) -> String {
    let (count, unit) = (base.count(), base.unit());
    match unit {
        TimeUnit::Minute => format!("{count}m"),
        TimeUnit::Hour if count < 6 => format!("{count}H"),
        TimeUnit::Hour => format!("{count}Hutc"),
        TimeUnit::Day => format!("{count}Dutc"),
        TimeUnit::Week => format!("{count}Wutc"),
        TimeUnit::Month => format!("{count}Mutc"),
    }
}

/// Bybit's interval: minutes as a number (hours as minutes), and `D`, `W`, `M`.
fn bybit_interval(base: Timeframe) -> String {
    match base.unit() {
        TimeUnit::Minute => base.count().to_string(),
        TimeUnit::Hour => (u32::from(base.count()) * 60).to_string(),
        TimeUnit::Day => "D".to_owned(),
        TimeUnit::Week => "W".to_owned(),
        TimeUnit::Month => "M".to_owned(),
    }
}

/// Gate.io's interval: `1m`, `4h`, `1d`, and `7d` and `30d` for weeks and months.
fn gate_interval(base: Timeframe) -> String {
    let count = base.count();
    match base.unit() {
        TimeUnit::Minute => format!("{count}m"),
        TimeUnit::Hour => format!("{count}h"),
        TimeUnit::Day => format!("{count}d"),
        TimeUnit::Week => format!("{}d", u32::from(count) * 7),
        TimeUnit::Month => format!("{}d", u32::from(count) * 30),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tf(text: &str) -> Timeframe {
        Timeframe::parse(text).unwrap()
    }

    #[test]
    fn markets_are_spelled_the_way_each_exchange_lists_them() {
        let spelled: Vec<String> = [
            Exchange::Coinbase,
            Exchange::Binance,
            Exchange::Okx,
            Exchange::Bybit,
            Exchange::Kraken,
            Exchange::Gate,
        ]
        .iter()
        .map(|exchange| exchange.symbol("btc").unwrap())
        .collect();
        assert_eq!(
            spelled,
            [
                "BTC-USD", "BTCUSDT", "BTC-USDT", "BTCUSDT", "BTC/USD", "BTC_USDT"
            ]
        );
    }

    #[test]
    fn an_asset_that_could_alter_the_request_is_refused() {
        for bad in [
            "",
            "B",
            "BTC/../x",
            "BTC USD",
            "BTC&limit=1",
            "WAYTOOLONGASSET",
        ] {
            assert!(
                matches!(Exchange::Binance.symbol(bad), Err(ChartError::BadAsset(_))),
                "{bad:?}"
            );
        }
        assert!(matches!(
            Exchange::parse("ftx"),
            Err(ChartError::UnknownExchange(_))
        ));
    }

    #[test]
    fn a_native_length_is_asked_for_as_it_is() {
        let binance = Exchange::Binance;
        let plan = binance.plan(tf("4h")).unwrap();
        assert!(plan.is_native());
        assert_eq!(binance.shown(plan), 500);
        assert_eq!(
            binance.candles_url("BTCUSDT", plan.base(), 500),
            "https://api.binance.com/api/v3/klines?symbol=BTCUSDT&interval=4h&limit=500"
        );
    }

    #[test]
    fn other_lengths_fold_from_the_coarsest_native_that_nests() {
        // Coinbase has no 4h: four 1h bars, 74 candles per 300-bar request.
        let plan = Exchange::Coinbase.plan(tf("4h")).unwrap();
        assert_eq!(plan.base(), Timeframe::HOUR_1);
        assert_eq!(Exchange::Coinbase.shown(plan), 74);
        // Kraken has no 12h: three 4h bars, 239 candles per 720-bar request.
        let plan = Exchange::Kraken.plan(tf("12h")).unwrap();
        assert_eq!(plan.base(), Timeframe::HOUR_4);
        assert_eq!(Exchange::Kraken.shown(plan), 239);
        // OKX has 3M natively; Binance folds it from months.
        assert!(Exchange::Okx.plan(tf("3M")).unwrap().is_native());
        assert_eq!(
            Exchange::Binance.plan(tf("3M")).unwrap().base(),
            Timeframe::MONTH_1
        );
    }

    #[test]
    fn each_exchange_names_its_candle_lengths_its_own_way() {
        let hour = Timeframe::HOUR_1;
        let day = Timeframe::DAY_1;
        assert_eq!(okx_bar(Timeframe::MINUTE_5), "5m");
        assert_eq!(okx_bar(hour), "1H");
        assert_eq!(okx_bar(tf("6h")), "6Hutc");
        assert_eq!(okx_bar(day), "1Dutc");
        assert_eq!(okx_bar(Timeframe::MONTH_1), "1Mutc");
        assert_eq!(bybit_interval(Timeframe::MINUTE_15), "15");
        assert_eq!(bybit_interval(tf("4h")), "240");
        assert_eq!(bybit_interval(day), "D");
        assert_eq!(bybit_interval(Timeframe::WEEK_1), "W");
        assert_eq!(gate_interval(hour), "1h");
        assert_eq!(gate_interval(day), "1d");
        assert_eq!(gate_interval(Timeframe::WEEK_1), "7d");
        assert_eq!(gate_interval(Timeframe::MONTH_1), "30d");
    }

    #[test]
    fn every_exchange_builds_its_request() {
        let hour = Timeframe::HOUR_1;
        assert_eq!(
            Exchange::Coinbase.candles_url("BTC-USD", hour, 99),
            "https://api.exchange.coinbase.com/products/BTC-USD/candles?granularity=3600"
        );
        assert_eq!(
            Exchange::Okx.candles_url("BTC-USDT", hour, 1_000),
            "https://www.okx.com/api/v5/market/candles?instId=BTC-USDT&bar=1H&limit=300"
        );
        assert_eq!(
            Exchange::Bybit.candles_url("BTCUSDT", hour, 200),
            "https://api.bybit.com/v5/market/kline?category=spot&symbol=BTCUSDT&interval=60&limit=200"
        );
        assert_eq!(
            Exchange::Kraken.candles_url("BTC/USD", hour, 200),
            "https://api.kraken.com/0/public/OHLC?pair=BTC/USD&interval=60"
        );
        assert_eq!(
            Exchange::Gate.candles_url("BTC_USDT", hour, 200),
            "https://api.gateio.ws/api/v4/spot/candlesticks?currency_pair=BTC_USDT&interval=1h&limit=200"
        );
    }

    #[test]
    fn an_answer_is_read_by_the_exchanges_own_parser() {
        let hour = Timeframe::HOUR_1;
        let binance = r#"[[1700000000000,"10.0","12.0","9.0","11.0","42.5",1700003599999]]"#;
        assert_eq!(
            Exchange::Binance.parse_candles(binance, hour).unwrap()[0].close,
            11.0
        );
        let error = Exchange::Okx
            .parse_candles(
                r#"{"code":"51001","msg":"Instrument ID doesn't exist.","data":[]}"#,
                hour,
            )
            .unwrap_err();
        assert_eq!(error.to_string(), "OKX: Instrument ID doesn't exist.");
    }
}
