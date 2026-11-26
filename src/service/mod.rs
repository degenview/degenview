pub mod alert;
pub mod annotation;
pub mod cache;
pub mod depth;
pub mod editor_theme;
pub mod image;
pub mod keychain;
pub mod liquidation;
pub mod llm;
pub mod market;
pub mod net;
pub mod portfolio;
pub mod snapshot;
pub mod sound;
pub mod stream;
pub mod system;
pub mod taker_flow;
pub mod tradingview;
pub mod update;
pub mod watchlist_quotes;
pub mod webhook;
pub mod work_queue;

pub use market::alpaca::test_connection as alpaca_test_connection;
pub use market::error::ProviderError;
pub use market::oanda::test_connection as oanda_test_connection;

use crate::model::{DataSource, Kline, PmSeries, Timeframe};
pub(crate) use market::candles::normalize;
pub(crate) use market::search_rank::rank_results;

/// A search hit on one provider: a display label, the provider's own id, and the
/// source it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchResult {
    /// The market as its exchange names it (`BTCUSDT`, `BTC-USD`, `PEPE/WETH`), or an equity's
    /// ticker and company (`AAPL — Apple Inc.`), or a prediction market's question.
    pub symbol: String,
    pub full_symbol: String,
    pub source: DataSource,
    pub price: Option<f64>,
    /// Prediction markets only: the event this choice belongs to and its artwork.
    pub event: Option<EventInfo>,
    /// The asset a pair trades, as the provider lists it, when the provider says. Binance's
    /// `BTCUSDT` and Coinbase's `BTC-USD` carry no separator, so a pair is never split by guessing.
    pub base: Option<String>,
    /// What the pair is priced in (`USDT`, `USD`).
    pub quote: Option<String>,
}

/// What separates an equity's ticker from its company name in [`SearchResult::symbol`].
pub const NAME_SEPARATOR: &str = " — ";

impl SearchResult {
    /// A hit with only its names; chain [`Self::pair`] for a market with a base and a quote.
    pub fn new(
        source: DataSource,
        symbol: impl Into<String>,
        full_symbol: impl Into<String>,
    ) -> Self {
        Self {
            symbol: symbol.into(),
            full_symbol: full_symbol.into(),
            source,
            price: None,
            event: None,
            base: None,
            quote: None,
        }
    }

    pub fn pair(mut self, base: impl Into<String>, quote: impl Into<String>) -> Self {
        self.base = Some(base.into());
        self.quote = Some(quote.into());
        self
    }

    /// The market's own name, without a company: `AAPL` for `AAPL — Apple Inc.`.
    pub fn ticker(&self) -> &str {
        self.symbol
            .split(NAME_SEPARATOR)
            .next()
            .unwrap_or(&self.symbol)
            .trim()
    }

    /// The company an equity belongs to, when the label carries one.
    pub fn company(&self) -> Option<&str> {
        let name = self.symbol.split_once(NAME_SEPARATOR)?.1.trim();
        (!name.is_empty()).then_some(name)
    }

    /// The asset the market trades: the provider's own base, else read out of the symbol.
    pub fn base_asset(&self) -> String {
        self.base.as_ref().map_or_else(
            || self.source.base_asset(&self.symbol),
            |base| base.to_uppercase(),
        )
    }

    /// `BASE/QUOTE` for a pair, else the symbol: the label portfolios keep for a market.
    pub fn pair_label(&self) -> String {
        match (&self.base, &self.quote) {
            (Some(base), Some(quote)) => format!("{base}/{quote}"),
            _ => self.symbol.clone(),
        }
    }
}

/// The parent event of a prediction-market choice.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EventInfo {
    pub title: String,
    pub image_url: Option<String>,
    /// The choices ticked to chart together, set on the pick the symbol picker builds from
    /// them; empty on a search hit.
    pub series: Vec<PmSeries>,
    /// Boxed: most hits are not events, and a hit is passed around by value.
    pub stats: Box<EventStats>,
}

/// What a provider says about an event beyond its choices, as far as it says it. Money is in whole
/// dollars (Kalshi's contracts settle at $1, so its counts stand for dollars).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EventStats {
    /// Traded since the event opened.
    pub volume: Option<u64>,
    pub volume_24h: Option<u64>,
    pub liquidity: Option<u64>,
    pub open_interest: Option<u64>,
    /// When the event closes, in epoch milliseconds.
    pub ends_at_ms: Option<i64>,
    pub category: Option<String>,
    /// The provider's own description, usually how the event resolves.
    pub about: Option<String>,
}

impl EventStats {
    /// Whether the provider said anything at all.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// A provider's number (a JSON number or a numeric string) as whole dollars; `None` for anything
/// that is not a finite, non-negative figure.
pub(crate) fn whole_dollars(raw: &serde_json::Value) -> Option<u64> {
    let value = raw
        .as_f64()
        .or_else(|| raw.as_str().and_then(|text| text.parse().ok()))?;
    (value.is_finite() && value >= 0.0).then(|| value.round() as u64)
}

/// A provider that can serve OHLC candles and, optionally, ticker search.
pub trait MarketSource {
    fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError>;

    /// Candles that open in `[start_ms, end_ms)`, oldest first, at most `max` of them (the newest if
    /// there are more). Only sources with a time-range endpoint implement it.
    fn fetch_klines_range(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, ProviderError> {
        let _ = (symbol, timeframe, start_ms, end_ms, max);
        Err(ProviderError::other(
            "This source cannot serve a time range of candles.",
        ))
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        let _ = query;
        Ok(Vec::new())
    }
}

pub fn for_source(source: DataSource) -> &'static dyn MarketSource {
    match source {
        DataSource::Binance => &market::binance::BINANCE,
        DataSource::Coinbase => &market::coinbase::COINBASE,
        DataSource::Okx => &market::okx::OKX,
        DataSource::Kraken => &market::kraken::KRAKEN,
        DataSource::Bybit => &market::bybit::BYBIT,
        DataSource::Gate => &market::gateio::GATE,
        DataSource::Mexc => &market::mexc::MEXC,
        DataSource::CoinGecko => &market::coingecko::COINGECKO,
        DataSource::DexScreener => &market::dexscreener::DEXSCREENER,
        DataSource::Alpaca => &market::alpaca::ALPACA,
        DataSource::Oanda => &market::oanda::OANDA,
        DataSource::Polymarket => &market::polymarket::POLYMARKET,
        DataSource::Kalshi => &market::kalshi::KALSHI,
        DataSource::CoinMarketCap => &market::cmc::COIN_MARKET_CAP,
    }
}

/// Fetch candles of `timeframe` from the provider behind `source`, oldest first. A source folds
/// finer candles for a length it lacks and answers `ProviderError::UnsupportedInterval` when it
/// cannot reach it, never a nearby one.
pub fn fetch_klines(
    source: DataSource,
    symbol: &str,
    timeframe: Timeframe,
    limit: u32,
) -> Result<Vec<Kline>, ProviderError> {
    let provider = for_source(source);
    let fetch = |limit: u32| {
        provider
            .fetch_klines(symbol, timeframe, limit)
            .map(normalize)
    };
    match cache::policy(source, timeframe) {
        Some(policy) => cache::fetch(source, symbol, timeframe, limit, policy, fetch),
        None => fetch(limit),
    }
}

/// A chart's candles and, for a streamed source that folds its candles, the base candles the
/// newest of them is made of.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LiveCandles {
    pub candles: Vec<Kline>,
    /// The base candles of the newest candle's bucket, which a live feed's klines of the same
    /// length merge with bar by bar. Empty when the candles are native, the source has no feed or
    /// the request for them failed.
    pub bars: Vec<Kline>,
}

impl From<Vec<Kline>> for LiveCandles {
    fn from(candles: Vec<Kline>) -> Self {
        Self {
            candles,
            bars: Vec::new(),
        }
    }
}

/// [`fetch_klines`], and for a source in [`DataSource::CANDLE_STREAMS`] that folds `timeframe`
/// from finer candles, one more small request for those candles of the newest bucket (one bucket's
/// worth, after the folded answer). A failed second request leaves `bars` empty: the candles are
/// still good.
pub fn fetch_klines_live(
    source: DataSource,
    symbol: &str,
    timeframe: Timeframe,
    limit: u32,
) -> Result<LiveCandles, ProviderError> {
    let candles = fetch_klines(source, symbol, timeframe, limit)?;
    let bars = source
        .plan(timeframe)
        .filter(|plan| !plan.is_native() && DataSource::CANDLE_STREAMS.contains(&source))
        .and_then(|plan| {
            let base = fetch_klines(source, symbol, plan.base(), plan.per_target() as u32).ok()?;
            Some(plan.newest_bucket(&base))
        })
        .unwrap_or_default();
    Ok(LiveCandles { candles, bars })
}

/// `MarketSource::fetch_klines_range` of the provider behind `source`.
pub fn fetch_klines_range(
    source: DataSource,
    symbol: &str,
    timeframe: Timeframe,
    start_ms: i64,
    end_ms: i64,
    max: usize,
) -> Result<Vec<Kline>, ProviderError> {
    for_source(source)
        .fetch_klines_range(symbol, timeframe, start_ms, end_ms, max)
        .map(normalize)
}

/// How many choices of an event are fetched at once.
const CHOICE_FETCH_PARALLELISM: usize = 6;

/// Price history for each of `choices`, in order. A choice that fails to load comes back empty:
/// the chart draws whatever arrived.
pub fn fetch_choices(
    source: DataSource,
    choices: &[PmSeries],
    timeframe: Timeframe,
    limit: u32,
) -> Vec<(String, Vec<Kline>)> {
    let mut histories = Vec::with_capacity(choices.len());
    for batch in choices.chunks(CHOICE_FETCH_PARALLELISM) {
        std::thread::scope(|scope| {
            let handles: Vec<_> = batch
                .iter()
                .map(|choice| {
                    scope.spawn(|| {
                        fetch_klines(source, &choice.token_id, timeframe, limit).unwrap_or_default()
                    })
                })
                .collect();
            for (choice, handle) in batch.iter().zip(handles) {
                histories.push((choice.token_id.clone(), handle.join().unwrap_or_default()));
            }
        });
    }
    histories
}

/// Search one provider.
pub fn search(source: DataSource, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
    for_source(source).search_tickers(query)
}

/// Whether a provider can say which markets are busiest right now (Kalshi cannot).
pub fn supports_trending(source: DataSource) -> bool {
    source == DataSource::Polymarket
}

/// The provider's busiest markets, or an empty list when it cannot say.
pub fn trending(source: DataSource, limit: usize) -> Result<Vec<SearchResult>, ProviderError> {
    match source {
        DataSource::Polymarket => market::polymarket::trending(limit),
        _ => Ok(Vec::new()),
    }
}

/// Thin to at most `count` points by uniform stride, always keeping the newest one.
pub(crate) fn downsample(candles: &[Kline], count: usize) -> Vec<Kline> {
    if count == 0 || candles.len() <= count {
        return candles.to_vec();
    }
    if count == 1 {
        return candles.last().cloned().into_iter().collect();
    }
    let stride = (candles.len() - 1) as f64 / (count - 1) as f64;
    (0..count)
        .map(|i| {
            let index = (i as f64 * stride).round() as usize;
            candles[index.min(candles.len() - 1)].clone()
        })
        .collect()
}

/// Overlay the executable YES ask on the final sample so the chart's endpoint,
/// current-price line, and headline match the provider's market page.
pub(crate) fn replace_last_price(mut candles: Vec<Kline>, price: f64) -> Vec<Kline> {
    if price.is_finite()
        && (0.0..=1.0).contains(&price)
        && let Some(last) = candles.last_mut()
    {
        last.close = price;
        last.high = price;
        last.low = price;
    }
    candles
}
