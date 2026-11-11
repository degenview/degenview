mod alpaca;
pub use alpaca::MISSING_KEY as ALPACA_MISSING_KEY;
pub use alpaca::test_connection as alpaca_test_connection;
pub mod asset_auto_mapper;
pub mod asset_host;
pub mod background_service;
mod binance;
mod cache;
pub mod cache_control;
mod candles;
pub mod catalog;
pub mod cmc;
pub mod code_signature;
mod coinbase;
mod coingecko;
mod dexscreener;
pub mod disk_cache;
pub mod entitlements;
pub mod file_dialog;
pub mod font_catalog;
pub mod fx_rates;
mod geckoterminal;
mod http;
pub mod icon;
pub mod image_ingest;
pub mod image_loader;
mod kalshi;
pub mod keychain_ai_keys;
pub mod keychain_api_keys;
pub mod keychain_broker_secrets;
pub mod keychain_mcp_seal;
pub mod keychain_snapshot_token;
pub mod keychain_webhook_secrets;
pub mod link_fetch;
pub mod live;
pub mod llm;
pub mod lookup_cache;
pub mod macos_notifications;
mod polymarket;
pub mod portfolio_history;
pub mod portfolio_performers;
pub mod portfolio_quotes;
mod rate_limit;
pub mod sandbox;
pub mod search_rank;
pub mod secret_service;
pub mod self_update;
pub mod snapshot_export;
pub mod snapshot_job;
pub mod snapshot_raster;
pub mod snapshot_upload;
pub mod tag_signature;
pub mod theme_files;
pub mod timezone;
pub mod update_archive;
pub mod update_check;
pub mod update_download;
pub mod watchlist_quotes;
pub mod webhook_delivery;
pub mod windows_credentials;
pub mod work_queue;
mod ws;

use crate::model::{DataSource, Kline, PmSeries};
pub(crate) use candles::{fill_gaps, fold_calendar, next_bucket_start_ms, normalize};
pub(crate) use search_rank::rank_results;

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
    /// The choices ticked to chart together, set on the pick the Add Chart sheet builds from
    /// them; empty on a search hit.
    pub series: Vec<PmSeries>,
}

/// A provider that can serve OHLC candles and, optionally, ticker search.
pub trait MarketSource {
    fn fetch_klines(&self, symbol: &str, interval: &str, limit: u32) -> Result<Vec<Kline>, String>;

    /// Candles that open in `[start_ms, end_ms)`, oldest first, at most `max` of them (the newest if
    /// there are more). Only sources with a time-range endpoint implement it.
    fn fetch_klines_range(
        &self,
        symbol: &str,
        interval: &str,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, String> {
        let _ = (symbol, interval, start_ms, end_ms, max);
        Err("This source cannot serve a time range of candles.".to_owned())
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, String> {
        let _ = query;
        Ok(Vec::new())
    }
}

pub fn for_source(source: DataSource) -> &'static dyn MarketSource {
    match source {
        DataSource::Binance => &binance::BINANCE,
        DataSource::Coinbase => &coinbase::COINBASE,
        DataSource::CoinGecko => &coingecko::COINGECKO,
        DataSource::DexScreener => &dexscreener::DEXSCREENER,
        DataSource::Alpaca => &alpaca::ALPACA,
        DataSource::Polymarket => &polymarket::POLYMARKET,
        DataSource::Kalshi => &kalshi::KALSHI,
        DataSource::CoinMarketCap => &cmc::COIN_MARKET_CAP,
    }
}

/// Fetch candles from the provider behind `source`, oldest first.
pub fn fetch_klines(
    source: DataSource,
    symbol: &str,
    interval: &str,
    limit: u32,
) -> Result<Vec<Kline>, String> {
    let provider = for_source(source);
    let fetch = |limit: u32| {
        provider
            .fetch_klines(symbol, interval, limit)
            .map(normalize)
    };
    match cache::policy(source) {
        Some(policy) => cache::fetch(source, symbol, interval, limit, policy, fetch),
        None => fetch(limit),
    }
}

/// `MarketSource::fetch_klines_range` of the provider behind `source`.
pub fn fetch_klines_range(
    source: DataSource,
    symbol: &str,
    interval: &str,
    start_ms: i64,
    end_ms: i64,
    max: usize,
) -> Result<Vec<Kline>, String> {
    for_source(source)
        .fetch_klines_range(symbol, interval, start_ms, end_ms, max)
        .map(normalize)
}

/// How many choices of an event are fetched at once.
const CHOICE_FETCH_PARALLELISM: usize = 6;

/// Price history for each of `choices`, in order. A choice that fails to load comes back empty:
/// the chart draws whatever arrived.
pub fn fetch_choices(
    source: DataSource,
    choices: &[PmSeries],
    interval: &str,
    limit: u32,
) -> Vec<(String, Vec<Kline>)> {
    let mut histories = Vec::with_capacity(choices.len());
    for batch in choices.chunks(CHOICE_FETCH_PARALLELISM) {
        std::thread::scope(|scope| {
            let handles: Vec<_> = batch
                .iter()
                .map(|choice| {
                    scope.spawn(|| {
                        fetch_klines(source, &choice.token_id, interval, limit).unwrap_or_default()
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
pub fn search(source: DataSource, query: &str) -> Result<Vec<SearchResult>, String> {
    for_source(source).search_tickers(query)
}

/// Whether a provider can say which markets are busiest right now (Kalshi cannot).
pub fn supports_trending(source: DataSource) -> bool {
    source == DataSource::Polymarket
}

/// The provider's busiest markets, or an empty list when it cannot say.
pub fn trending(source: DataSource, limit: usize) -> Result<Vec<SearchResult>, String> {
    match source {
        DataSource::Polymarket => polymarket::trending(limit),
        _ => Ok(Vec::new()),
    }
}

/// Binance-style interval token → seconds.
pub(crate) fn interval_seconds(token: &str) -> Option<i64> {
    match token {
        "1m" => Some(60),
        "5m" => Some(300),
        "15m" => Some(900),
        "30m" => Some(1_800),
        "1h" => Some(3_600),
        "4h" => Some(14_400),
        "1d" => Some(86_400),
        "1w" => Some(604_800),
        "1M" => Some(2_592_000),
        "3M" => Some(7_776_000),
        "1Y" => Some(31_536_000),
        _ => None,
    }
}

/// Milliseconds since the Unix epoch, now.
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

/// Open time of the calendar bucket that contains `epoch_ms`. Sub-week intervals
/// floor onto the epoch; weeks start on Monday and months/quarters/years on their
/// calendar start, matching Binance's candle boundaries.
pub(crate) fn bucket_start_ms(epoch_ms: i64, interval_secs: i64) -> i64 {
    if interval_secs < 604_800 {
        return epoch_ms.div_euclid(interval_secs * 1_000) * interval_secs * 1_000;
    }
    let days = epoch_ms.div_euclid(86_400_000);
    if interval_secs < 2_419_200 {
        // Week, starting Monday.
        let monday_days = (days - 4).div_euclid(7) * 7 + 4;
        return monday_days * 86_400_000;
    }
    let (year, month, _) = crate::format::civil_from_days(days);
    let (target_year, target_month) = if interval_secs < 7_000_000 {
        (year, month)
    } else if interval_secs < 30_000_000 {
        (year, (month - 1) / 3 * 3 + 1)
    } else {
        (year, 1)
    };
    crate::format::days_from_civil(target_year, target_month, 1) * 86_400_000
}

/// Span a prediction-market line chart covers at an interval, in days.
pub(crate) fn effective_span_days(interval: &str) -> i64 {
    match interval {
        "1h" => 2,
        "1d" => 60,
        "1w" => 182,
        "1M" => 360,
        "3M" => 90,
        "1Y" => 364,
        _ => 60,
    }
}

/// Points a prediction-market line chart draws at an interval.
pub(crate) fn line_chart_point_count(interval: &str) -> usize {
    match interval {
        "1h" => 48,
        "1d" => 60,
        "1w" => 26,
        "1M" => 12,
        "3M" => 90,
        "1Y" => 52,
        _ => 60,
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
