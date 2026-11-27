//! Where watchlist prices come from: one batched call per source. The live provider
//! wraps each exchange's endpoint; tests substitute a recorder. Parsers are pure so the
//! response shapes are covered without a network.

use super::engine::Outcome;
use crate::i18n::tr;
use crate::lock::lock;
use crate::model::exchange::Market;
use crate::model::market::binance::split_market;
use crate::model::market::oanda::OandaEnvironment;
use crate::model::watchlist::quote::{SourceQuote, VolumeKind};
use crate::model::{DataSource, Kline, Listing, Timeframe};
use crate::service::ProviderError;
use crate::service::keychain::api_keys::OandaCredentials;
use crate::service::market::binance;
use crate::service::net::url::encode_component;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// One market to price. `symbol` is the id the provider's quote call takes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuoteRequest {
    pub symbol: String,
    /// The network of a DEX pair; other sources leave it `None`.
    pub chain: Option<String>,
}

/// Why a batch of quotes did not come back.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum QuoteFailure {
    RateLimited,
    /// No credentials of the provider are saved.
    CredentialsMissing(DataSource),
    /// The provider refused the request itself, as it does for an unknown symbol.
    /// Retrying a smaller batch can find which symbol it was.
    Rejected(String),
    /// Network trouble or a server error; the whole source is treated as down for a moment.
    Failed(String),
}

impl QuoteFailure {
    pub fn message(&self) -> String {
        match self {
            Self::RateLimited => tr!("Rate limited by the provider").to_owned(),
            Self::CredentialsMissing(source) => match source {
                DataSource::Alpaca => {
                    tr!("Add your Alpaca API key in Settings to see stock quotes").to_owned()
                }
                DataSource::Oanda => {
                    tr!("Add your Oanda credentials in Settings to see Oanda quotes").to_owned()
                }
                _ => tr!("Add this provider's API key in Settings to see its quotes").to_owned(),
            },
            Self::Rejected(message) | Self::Failed(message) => message.clone(),
        }
    }
}

/// Quotes keyed by `QuoteRequest::symbol`; a symbol the provider did not answer is omitted.
/// An adapter that makes several requests reports the ones that failed in the `Outcome`
/// and keeps the rest; `Err` is for a call that failed as a whole.
pub trait QuoteProvider: Send + Sync {
    /// `requests` are all of `listing`'s markets.
    fn quotes(&self, listing: Listing, requests: &[QuoteRequest]) -> Result<Outcome, QuoteFailure>;

    /// The network of a DEX pair saved without one, from its address.
    fn resolve_chain(&self, address: &str) -> Option<String>;
}

/// The most symbols one request carries, per listing.
pub fn chunk_size(listing: Listing) -> usize {
    match listing.source {
        // Without a batch size one call answers every market of the listing.
        DataSource::Binance => binance::endpoints(listing.market)
            .ticker_batch
            .unwrap_or(usize::MAX),
        DataSource::Alpaca => 200,
        DataSource::CoinGecko => 250,
        DataSource::Coinbase => 20,
        // One call answers every spot market, so a round is one request however many rows.
        DataSource::Okx => 2_000,
        DataSource::Kraken => 100,
        // One call answers every spot market, so a round is one request however many rows.
        DataSource::Bybit => 2_000,
        DataSource::Gate => 2_500,
        DataSource::Mexc => 2_500,
        DataSource::DexScreener => 30,
        _ => 100,
    }
}

// MARK: Live provider

pub struct LiveProvider {
    /// Oanda's pricing answer carries no 24h figure; one hourly candle per instrument supplies it.
    oanda_day_ago: Arc<Mutex<DayAgoReferences>>,
    /// Set while a background refresh of those prices runs, so rounds do not stack them.
    oanda_refreshing: Arc<AtomicBool>,
}

impl Default for LiveProvider {
    fn default() -> Self {
        Self {
            oanda_day_ago: Arc::default(),
            oanda_refreshing: Arc::new(AtomicBool::new(false)),
        }
    }
}

const COINBASE_URL: &str = "https://api.exchange.coinbase.com";
const OKX_URL: &str = "https://www.okx.com/api/v5";
const KRAKEN_URL: &str = crate::service::market::kraken::BASE_URL;
const BYBIT_URL: &str = crate::service::market::bybit::BASE_URL;
const GATE_URL: &str = crate::service::market::gateio::BASE_URL;
const MEXC_URL: &str = crate::service::market::mexc::BASE_URL;
const DEX_URL: &str = "https://api.dexscreener.com/latest/dex";

impl QuoteProvider for LiveProvider {
    fn quotes(&self, listing: Listing, requests: &[QuoteRequest]) -> Result<Outcome, QuoteFailure> {
        if requests.is_empty() {
            return Ok(Outcome::default());
        }
        let source = listing.source;
        match source {
            DataSource::Binance => binance_quotes(listing.market, requests),
            DataSource::Coinbase => Ok(coinbase(requests)),
            DataSource::Okx => okx(requests).map(Outcome::from_quotes),
            DataSource::Kraken => kraken(requests).map(Outcome::from_quotes),
            DataSource::Bybit => bybit(requests).map(Outcome::from_quotes),
            DataSource::Gate => gateio(requests).map(Outcome::from_quotes),
            DataSource::Mexc => mexc(requests).map(Outcome::from_quotes),
            DataSource::CoinGecko => coingecko(requests).map(Outcome::from_quotes),
            DataSource::DexScreener => Ok(dexscreener(requests)),
            DataSource::Alpaca => alpaca(requests).map(Outcome::from_quotes),
            DataSource::Oanda => oanda(requests, &self.oanda_day_ago, &self.oanda_refreshing)
                .map(Outcome::from_quotes),
            DataSource::Polymarket | DataSource::Kalshi => Ok(prediction(source, requests)),
            DataSource::CoinMarketCap => Ok(Outcome::default()),
        }
    }

    fn resolve_chain(&self, address: &str) -> Option<String> {
        let body = get(
            DataSource::DexScreener,
            &format!("{DEX_URL}/search?q={address}"),
            &[],
        )
        .ok()?;
        parse_chain(&body, address)
    }
}

/// A GET that maps HTTP failures onto `QuoteFailure`.
fn get(source: DataSource, url: &str, headers: &[(&str, &str)]) -> Result<String, QuoteFailure> {
    match crate::service::net::http::get_text(url, headers, crate::service::net::http::Retry::Never)
    {
        Ok(body) => Ok(body),
        Err(crate::service::net::http::Error::Status(code)) => Err(classify_status(source, code)),
        Err(error) => Err(QuoteFailure::Failed(tr!(
            "Network error: {error}",
            error = error
        ))),
    }
}

/// What an HTTP status means for a quote request. Only a status the provider documents
/// as "a symbol in this request is unknown" is `Rejected`, which makes the engine search
/// for the bad symbol; every other refusal (credentials, region, a changed endpoint, a
/// ban) belongs to the whole source and backs it off instead.
pub fn classify_status(source: DataSource, code: u16) -> QuoteFailure {
    match (source, code) {
        (_, 429) => QuoteFailure::RateLimited,
        (DataSource::Alpaca, 401 | 403) => QuoteFailure::Failed(
            tr!("Alpaca refused your API key. Check it in Settings").to_owned(),
        ),
        (DataSource::Oanda, 401 | 403) => QuoteFailure::Failed(
            tr!("Oanda refused your access token. Check it in Settings").to_owned(),
        ),
        // Binance answers an unknown symbol in a batch with 400; so do Alpaca and Oanda.
        (DataSource::Binance | DataSource::Alpaca | DataSource::Oanda, 400) => {
            QuoteFailure::Rejected(tr!("The provider answered HTTP 400").to_owned())
        }
        // Bybit refuses some countries, and a client that asks too often, with a 403.
        (DataSource::Bybit, 403) => QuoteFailure::Failed(
            tr!("Bybit refused the request (HTTP 403): it is not available in every country")
                .to_owned(),
        ),
        // Gate.io refuses some countries, and a client that asks too often, with a 403.
        (DataSource::Gate, 403) => QuoteFailure::Failed(
            tr!("Gate.io refused the request (HTTP 403): it is not available in every country")
                .to_owned(),
        ),
        // MEXC refuses some countries with a 403.
        (DataSource::Mexc, 403) => QuoteFailure::Failed(
            tr!("MEXC refused the request (HTTP 403): it is not available in every country")
                .to_owned(),
        ),
        // Coinbase is asked one product at a time: a 404 names that product.
        (DataSource::Coinbase, 404) => {
            QuoteFailure::Rejected(tr!("The provider answered HTTP 404").to_owned())
        }
        _ => QuoteFailure::Failed(tr!("The provider answered HTTP {code}", code = code)),
    }
}

/// The 24-hour ticker URL of `market` for `requests`. Spot takes a list: `symbols` is a JSON
/// array, so its brackets and quotes are percent-encoded: sent raw they are not valid in a URL,
/// the HTTP client refuses the request, and every Binance quote in every watchlist stays empty.
/// The futures API ignores a list: one market is asked for by name, more as the whole exchange.
pub fn binance_url(market: Market, requests: &[QuoteRequest]) -> String {
    let endpoints = binance::endpoints(market);
    if endpoints.ticker_batch.is_none() {
        return match requests {
            [one] => format!(
                "{}?symbol={}",
                endpoints.ticker_url,
                encode_component(&binance_pair(&one.symbol))
            ),
            _ => endpoints.ticker_url.to_owned(),
        };
    }
    let list = requests
        .iter()
        .map(|request| {
            let pair = binance_pair(&request.symbol);
            format!("%22{}%22", encode_component(&pair))
        })
        .collect::<Vec<_>>()
        .join("%2C");
    format!("{}?symbols=%5B{list}%5D&type=MINI", endpoints.ticker_url)
}

/// The pair a symbol of the app names, as Binance's answers write it.
fn binance_pair(symbol: &str) -> String {
    split_market(&symbol.to_uppercase()).0.to_owned()
}

fn binance_quotes(market: Market, requests: &[QuoteRequest]) -> Result<Outcome, QuoteFailure> {
    match get(DataSource::Binance, &binance_url(market, requests), &[]) {
        Ok(body) => binance_outcome(market, &body, requests),
        Err(failure) => binance_refusal(market, requests, failure),
    }
}

/// What a failed request says. A futures market asked for by name and refused is one the exchange
/// never listed; a refused spot batch is left to be halved.
pub fn binance_refusal(
    market: Market,
    requests: &[QuoteRequest],
    failure: QuoteFailure,
) -> Result<Outcome, QuoteFailure> {
    match (failure, requests) {
        (QuoteFailure::Rejected(_), [one]) if binance::endpoints(market).ticker_batch.is_none() => {
            Ok(Outcome {
                rejected: HashSet::from([one.symbol.clone()]),
                complete: true,
                ..Outcome::default()
            })
        }
        (failure, _) => Err(failure),
    }
}

/// The quotes of an answer. A futures answer also says which of the asked markets do not exist,
/// those it lacks: a spot request for an unknown one is refused instead, and found by halving.
pub fn binance_outcome(
    market: Market,
    body: &str,
    requests: &[QuoteRequest],
) -> Result<Outcome, QuoteFailure> {
    let mut outcome = Outcome::from_quotes(parse_binance(body, requests)?);
    if binance::endpoints(market).ticker_batch.is_none() {
        let answer = json(body)?;
        let listed: HashSet<String> = binance_tickers(&answer)
            .into_iter()
            .flatten()
            .filter_map(|ticker| Some(ticker.get("symbol")?.as_str()?.to_uppercase()))
            .collect();
        outcome.rejected = requests
            .iter()
            .filter(|request| !listed.contains(&binance_pair(&request.symbol)))
            .map(|request| request.symbol.clone())
            .collect();
        outcome.complete = true;
    }
    Ok(outcome)
}

/// How many answers in a row may fail before an adapter stops asking: the endpoint is
/// down, and the rest of the batch would only add requests.
const CONSECUTIVE_FAILURES: usize = 3;

fn mexc(requests: &[QuoteRequest]) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let body = get(DataSource::Mexc, &format!("{MEXC_URL}/ticker/24hr"), &[])?;
    parse_mexc(&body, requests)
}

fn gateio(requests: &[QuoteRequest]) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let body = get(DataSource::Gate, &format!("{GATE_URL}/tickers"), &[])?;
    parse_gateio(&body, requests)
}

fn bybit(requests: &[QuoteRequest]) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let body = get(
        DataSource::Bybit,
        &format!("{BYBIT_URL}/tickers?category=spot"),
        &[],
    )?;
    parse_bybit(&body, requests)
}

fn kraken(requests: &[QuoteRequest]) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    // One unknown pair refuses the whole call: the engine bisects for it.
    let pairs: Vec<&str> = requests
        .iter()
        .map(|request| request.symbol.as_str())
        .collect();
    // No wait for Kraken's one-a-second slot here: a round joins every source's request before
    // anything is applied, so a sleeping Kraken call (up to `ISOLATION_BUDGET` of them while a
    // refused pair is isolated) would hold back every other source's prices. A round is one call
    // and Kraken's own refusal backs the source off.
    let body = get(
        DataSource::Kraken,
        &format!("{KRAKEN_URL}/Ticker?pair={}", pairs.join(",")),
        &[],
    )?;
    parse_kraken(&body, requests)
}

fn okx(requests: &[QuoteRequest]) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let body = get(
        DataSource::Okx,
        &format!("{OKX_URL}/market/tickers?instType=SPOT"),
        &[],
    )?;
    parse_okx(&body, requests)
}

fn coinbase(requests: &[QuoteRequest]) -> Outcome {
    // No multi-product price call; `/stats` is one small request per product, paced under
    // the exchange's public limit. A product that fails is reported, and the rest still answer.
    let mut outcome = Outcome::default();
    let mut streak = 0;
    for (index, request) in requests.iter().enumerate() {
        if index > 0 {
            std::thread::sleep(Duration::from_millis(125));
        }
        let id = crate::service::market::coinbase::product_id(&request.symbol);
        match get(
            DataSource::Coinbase,
            &format!("{COINBASE_URL}/products/{id}/stats"),
            &[],
        ) {
            Ok(body) => {
                streak = 0;
                if let Some(quote) = parse_coinbase_stats(&body, &id) {
                    outcome.quotes.insert(request.symbol.clone(), quote);
                }
            }
            Err(QuoteFailure::Rejected(_)) => {
                streak = 0;
                outcome.rejected.insert(request.symbol.clone());
            }
            Err(failure) => {
                let stop = matches!(failure, QuoteFailure::RateLimited) || {
                    streak += 1;
                    streak >= CONSECUTIVE_FAILURES
                };
                let upto = if stop { requests.len() } else { index + 1 };
                outcome
                    .failed
                    .push((symbols_of(&requests[index..upto]), failure));
                if stop {
                    break;
                }
            }
        }
    }
    outcome
}

fn symbols_of(requests: &[QuoteRequest]) -> Vec<String> {
    requests
        .iter()
        .map(|request| request.symbol.clone())
        .collect()
}

fn coingecko(requests: &[QuoteRequest]) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let ids = requests
        .iter()
        .map(|request| request.symbol.to_lowercase())
        .collect::<Vec<_>>()
        .join(",");
    // One `/coins/markets` call prices every coin id, costing the limiter a single slot.
    super::super::net::rate_limit::coingecko().wait_for_slot();
    let url = format!(
        "{}/coins/markets?vs_currency=usd&ids={ids}\
         &sparkline=false&price_change_percentage=24h&per_page={}",
        crate::service::market::coingecko::BASE_URL,
        requests.len().min(250)
    );
    parse_coingecko_markets(&get(DataSource::CoinGecko, &url, &[])?, requests)
}

fn dexscreener(requests: &[QuoteRequest]) -> Outcome {
    // One request per chain for up to 30 pools each. Pairs without a known chain are left out.
    let mut by_chain: Vec<(&str, Vec<&QuoteRequest>)> = Vec::new();
    for request in requests {
        if let Some(chain) = request.chain.as_deref().filter(|chain| *chain != "unknown") {
            match by_chain.iter_mut().find(|(name, _)| *name == chain) {
                Some((_, pairs)) => pairs.push(request),
                None => by_chain.push((chain, vec![request])),
            }
        }
    }
    let mut outcome = Outcome::default();
    let mut limited = false;
    for (chain, pairs) in by_chain {
        for page in pairs.chunks(chunk_size(DataSource::DexScreener.into())) {
            let names: Vec<String> = page.iter().map(|request| request.symbol.clone()).collect();
            if limited {
                outcome.failed.push((names, QuoteFailure::RateLimited));
                continue;
            }
            let addresses = names.join(",");
            match get(
                DataSource::DexScreener,
                &format!("{DEX_URL}/pairs/{chain}/{addresses}"),
                &[],
            ) {
                Ok(body) => outcome.quotes.extend(parse_dex_pairs(&body, page)),
                Err(failure) => {
                    limited = matches!(failure, QuoteFailure::RateLimited);
                    outcome.failed.push((names, failure));
                }
            }
        }
    }
    outcome
}

fn alpaca(requests: &[QuoteRequest]) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let Some((key, secret)) = crate::service::keychain::api_keys::alpaca_credentials() else {
        return Err(QuoteFailure::CredentialsMissing(DataSource::Alpaca));
    };
    let symbols = requests
        .iter()
        .map(|request| request.symbol.to_uppercase())
        .collect::<Vec<_>>()
        .join(",");
    let url = format!("https://data.alpaca.markets/v2/stocks/snapshots?symbols={symbols}&feed=iex");
    let body = get(
        DataSource::Alpaca,
        &url,
        &[("APCA-API-KEY-ID", &key), ("APCA-API-SECRET-KEY", &secret)],
    )?;
    parse_alpaca_snapshots(&body, requests)
}

/// Oanda's `/accounts/{id}/pricing`: the mid of the inside bid and ask of every instrument
/// asked for, plus the price about a day ago so the change column has a basis. The day-ago
/// prices are read from the cache only; the ones it lacks are refreshed in the background, so
/// one slow candle request cannot stall the quote round for every provider.
fn oanda(
    requests: &[QuoteRequest],
    day_ago: &Arc<Mutex<DayAgoReferences>>,
    refreshing: &Arc<AtomicBool>,
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let Some(login) = crate::service::keychain::api_keys::oanda_credentials() else {
        return Err(QuoteFailure::CredentialsMissing(DataSource::Oanda));
    };
    let symbols = requests
        .iter()
        .map(|request| request.symbol.to_uppercase())
        .collect::<Vec<_>>()
        .join(",");
    let request = crate::service::market::oanda::api_request(
        &login,
        &format!(
            "/accounts/{}/pricing?instruments={symbols}",
            login.account_id
        ),
    );
    let body = get(
        DataSource::Oanda,
        &request.url,
        &[("Authorization", request.authorization.as_str())],
    )?;
    let mut quotes = parse_oanda_pricing(&body, requests)?;
    let now = crate::clock::now_ms();
    let environment = login.environment;
    let stale = {
        let references = lock(day_ago);
        references.fill(environment, &mut quotes, requests, now);
        references.stale(environment, requests, now)
    };
    if !stale.is_empty()
        && refreshing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    {
        let references = Arc::clone(day_ago);
        let flag = Arc::clone(refreshing);
        crate::service::work_queue::submit_for(DataSource::Oanda, move || {
            for instrument in stale {
                if let Ok(reference) = oanda_day_ago_price(&login, &instrument, now) {
                    lock(&references).insert(environment, instrument, reference);
                }
            }
            flag.store(false, Ordering::Release);
        });
    }
    Ok(quotes)
}

const HOUR_MS: i64 = 3_600_000;

#[derive(Clone, Copy)]
struct DayAgoReference {
    hour: i64,
    price: Option<f64>,
}

fn day_ago_hour(now: i64) -> i64 {
    (now - 24 * HOUR_MS).div_euclid(HOUR_MS) * HOUR_MS
}

/// Each Oanda instrument's reference for a given cutoff hour, apart by environment: a practice
/// quote is never measured against a live candle, nor the other way round.
#[derive(Default)]
struct DayAgoReferences(HashMap<(OandaEnvironment, String), DayAgoReference>);

impl DayAgoReferences {
    fn current(
        &self,
        environment: OandaEnvironment,
        instrument: &str,
        now: i64,
    ) -> Option<DayAgoReference> {
        self.0
            .get(&(environment, instrument.to_uppercase()))
            .copied()
            .filter(|reference| reference.hour == day_ago_hour(now))
    }

    /// Attach references already obtained for the current cutoff hour.
    fn fill(
        &self,
        environment: OandaEnvironment,
        quotes: &mut HashMap<String, SourceQuote>,
        requests: &[QuoteRequest],
        now: i64,
    ) {
        for request in requests {
            if let Some(quote) = quotes.get_mut(&request.symbol)
                && let Some(reference) = self.current(environment, &request.symbol, now)
            {
                quote.previous_day_price = reference.price;
            }
        }
    }

    /// The instruments that have no reference for the current cutoff hour.
    fn stale(
        &self,
        environment: OandaEnvironment,
        requests: &[QuoteRequest],
        now: i64,
    ) -> Vec<String> {
        requests
            .iter()
            .map(|request| request.symbol.to_uppercase())
            .filter(|instrument| self.current(environment, instrument, now).is_none())
            .collect()
    }

    fn insert(
        &mut self,
        environment: OandaEnvironment,
        instrument: String,
        reference: DayAgoReference,
    ) {
        self.0.insert((environment, instrument), reference);
    }
}

/// The opening midpoint of the candle covering 24 hours ago. A closed market
/// has no reference for that hour.
fn oanda_day_ago_price(
    login: &OandaCredentials,
    instrument: &str,
    now: i64,
) -> Result<DayAgoReference, ProviderError> {
    day_ago_reference(now, |start, end| {
        crate::service::market::oanda::klines_range(
            login,
            instrument,
            Timeframe::HOUR_1,
            start,
            end,
            1,
        )
        .map(crate::service::normalize)
    })
}

fn day_ago_reference(
    now: i64,
    fetch: impl FnOnce(i64, i64) -> Result<Vec<Kline>, ProviderError>,
) -> Result<DayAgoReference, ProviderError> {
    let hour = day_ago_hour(now);
    let price = fetch(hour, hour + HOUR_MS)?
        .into_iter()
        .find(|candle| candle.open_time == hour)
        .map(|candle| candle.open);
    Ok(DayAgoReference { hour, price })
}

/// `{"prices": [{"instrument": "EUR_USD", "bids": [{"price": "1.08501"}], "asks": [{"price":
/// "1.08521"}], "time": ISO}]}`: the quote of a market is the mid of its inside bid and ask.
/// A market that is not tradeable right now has empty level arrays, so the closeout prices
/// are the fallback.
pub fn parse_oanda_pricing(
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let value = json(body)?;
    let prices = value
        .get("prices")
        .and_then(Value::as_array)
        .ok_or_else(|| QuoteFailure::Failed(tr!("Unexpected Oanda response").to_owned()))?;
    let mut quotes = HashMap::new();
    for request in requests {
        let Some(price) = prices.iter().find(|price| {
            price
                .get("instrument")
                .and_then(Value::as_str)
                .is_some_and(|name| name.eq_ignore_ascii_case(&request.symbol))
        }) else {
            continue;
        };
        let bid = number(price.pointer("/bids/0/price"))
            .or_else(|| number(price.pointer("/closeoutBid")));
        let ask = number(price.pointer("/asks/0/price"))
            .or_else(|| number(price.pointer("/closeoutAsk")));
        let mid = match (bid, ask) {
            (Some(bid), Some(ask)) => (bid + ask) / 2.0,
            _ => continue,
        };
        if mid > 0.0 {
            quotes.insert(
                request.symbol.clone(),
                SourceQuote {
                    price: mid,
                    previous_day_price: None,
                    volume_24h: None,
                    volume_kind: None,
                    timestamp_ms: price
                        .pointer("/time")
                        .and_then(Value::as_str)
                        .and_then(crate::format::parse_iso8601),
                },
            );
        }
    }
    Ok(quotes)
}

/// The hourly points a prediction market's quote reads: a month of them, from which the price a
/// day before the last one is picked.
const PREDICTION_POINTS: u32 = 720;

/// The price a day before the last point of `series` (oldest first), or the oldest one when the
/// market has not been traded that long.
fn day_before(series: &[Kline]) -> Option<f64> {
    const DAY_MS: i64 = 86_400_000;
    let last = series.last()?;
    series
        .iter()
        .find(|point| point.open_time >= last.open_time - DAY_MS)
        .map(|point| point.close)
}

/// Last price of a prediction market's hourly series, and its price a day earlier as the
/// comparison.
fn prediction(source: DataSource, requests: &[QuoteRequest]) -> Outcome {
    let mut outcome = Outcome::default();
    let mut streak = 0;
    for (index, request) in requests.iter().enumerate() {
        let series = match crate::service::fetch_klines(
            source,
            &request.symbol,
            Timeframe::HOUR_1,
            PREDICTION_POINTS,
        ) {
            Ok(series) => {
                streak = 0;
                series
            }
            Err(error) => {
                streak += 1;
                let stop = streak >= CONSECUTIVE_FAILURES;
                let upto = if stop { requests.len() } else { index + 1 };
                outcome.failed.push((
                    symbols_of(&requests[index..upto]),
                    QuoteFailure::Failed(error.to_string()),
                ));
                if stop {
                    break;
                }
                continue;
            }
        };
        if let (Some(before), Some(last)) = (day_before(&series), series.last()) {
            outcome.quotes.insert(
                request.symbol.clone(),
                SourceQuote {
                    price: last.close,
                    previous_day_price: Some(before),
                    ..SourceQuote::default()
                },
            );
        }
    }
    outcome
}

// MARK: Parsers

fn json(body: &str) -> Result<Value, QuoteFailure> {
    serde_json::from_str(body)
        .map_err(|error| QuoteFailure::Failed(tr!("Data error: {error}", error = error)))
}

fn number(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

const DOLLAR_QUOTES: [&str; 8] = [
    "USDT", "USDC", "BUSD", "FDUSD", "TUSD", "USDP", "DAI", "USD",
];

/// The tickers of a `/ticker/24hr` answer: a list, or the one object of a market asked for alone.
/// A futures market asked for alone that no longer trades is answered with an empty object. An
/// empty list says nothing about any market, so it is no answer.
fn binance_tickers(answer: &Value) -> Option<&[Value]> {
    match answer {
        Value::Array(tickers) if !tickers.is_empty() => Some(tickers),
        Value::Object(ticker) if ticker.is_empty() => Some(&[]),
        Value::Object(ticker) if ticker.contains_key("symbol") => {
            Some(std::slice::from_ref(answer))
        }
        _ => None,
    }
}

/// `/ticker/24hr?type=MINI`: `openPrice` is the rolling 24h open, a truer day-change
/// reference than a candle picked from the last 25 hours.
pub fn parse_binance(
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let value = json(body)?;
    let tickers: HashMap<String, &Value> = binance_tickers(&value)
        .ok_or_else(|| QuoteFailure::Failed(tr!("Unexpected Binance response").to_owned()))?
        .iter()
        .filter_map(|ticker| Some((ticker.get("symbol")?.as_str()?.to_uppercase(), ticker)))
        .collect();
    let mut quotes = HashMap::new();
    for request in requests {
        let symbol = binance_pair(&request.symbol);
        let Some(ticker) = tickers.get(&symbol) else {
            continue;
        };
        let Some(price) = number(ticker.get("lastPrice")).filter(|price| *price > 0.0) else {
            continue;
        };
        let mut quote = SourceQuote {
            price,
            previous_day_price: number(ticker.get("openPrice")),
            timestamp_ms: ticker.get("closeTime").and_then(Value::as_i64),
            ..SourceQuote::default()
        };
        // Turnover is only a dollar figure on a dollar-quoted pair; otherwise report units traded.
        let dollar_quoted = DOLLAR_QUOTES.iter().any(|quote| symbol.ends_with(quote));
        if let (true, Some(turnover)) = (dollar_quoted, number(ticker.get("quoteVolume"))) {
            quote.volume_24h = Some(turnover);
            quote.volume_kind = Some(VolumeKind::QuoteCurrency);
        } else if let Some(units) = number(ticker.get("volume")) {
            quote.volume_24h = Some(units);
            quote.volume_kind = Some(VolumeKind::Base);
        }
        quotes.insert(request.symbol.clone(), quote);
    }
    Ok(quotes)
}

/// `/ticker/24hr`: every spot market in one answer (its `symbols` parameter is ignored); the
/// requested ones are kept. `openPrice` is the price 24 hours ago, so the change is a real rolling
/// one, and the quote-currency volume of a dollar-quoted pair is `quoteVolume`.
pub fn parse_mexc(
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let value = json(body)?;
    let tickers: HashMap<String, SourceQuote> = value
        .as_array()
        .ok_or_else(|| QuoteFailure::Failed(tr!("Unexpected MEXC response").to_owned()))?
        .iter()
        .filter_map(|ticker| {
            let id = ticker.get("symbol")?.as_str()?.to_uppercase();
            let price = number(ticker.get("lastPrice")).filter(|price| *price > 0.0)?;
            let mut quote = SourceQuote {
                price,
                previous_day_price: number(ticker.get("openPrice")).filter(|open| *open > 0.0),
                ..SourceQuote::default()
            };
            let dollar_quoted = DOLLAR_QUOTES.iter().any(|quote| id.ends_with(quote));
            if let (true, Some(turnover)) = (dollar_quoted, number(ticker.get("quoteVolume"))) {
                quote.volume_24h = Some(turnover);
                quote.volume_kind = Some(VolumeKind::QuoteCurrency);
            } else if let Some(units) = number(ticker.get("volume")) {
                quote.volume_24h = Some(units);
                quote.volume_kind = Some(VolumeKind::Base);
            }
            Some((id, quote))
        })
        .collect();
    Ok(requests
        .iter()
        .filter_map(|request| {
            let quote = tickers.get(&request.symbol.to_uppercase())?;
            Some((request.symbol.clone(), *quote))
        })
        .collect())
}

/// One Gate.io spot ticker (`/spot/tickers` and the `spot.tickers` channel carry the same fields):
/// the market and its price, with the price a day ago taken from the 24h change in percent
/// (`change_percentage`), and the quote-currency volume of a dollar-quoted pair, the units traded
/// otherwise.
pub fn gateio_ticker(ticker: &Value) -> Option<(String, SourceQuote)> {
    let id = ticker.get("currency_pair")?.as_str()?.to_uppercase();
    let price = number(ticker.get("last")).filter(|price| *price > 0.0)?;
    let mut quote = SourceQuote {
        price,
        previous_day_price: number(ticker.get("change_percentage"))
            .filter(|percent| percent.is_finite() && *percent > -100.0)
            .map(|percent| price / (1.0 + percent / 100.0)),
        ..SourceQuote::default()
    };
    let dollar_quoted = DOLLAR_QUOTES.iter().any(|quote| id.ends_with(quote));
    if let (true, Some(turnover)) = (dollar_quoted, number(ticker.get("quote_volume"))) {
        quote.volume_24h = Some(turnover);
        quote.volume_kind = Some(VolumeKind::QuoteCurrency);
    } else if let Some(units) = number(ticker.get("base_volume")) {
        quote.volume_24h = Some(units);
        quote.volume_kind = Some(VolumeKind::Base);
    }
    Some((id, quote))
}

/// `/spot/tickers`: every spot market in one answer; the requested ones are kept.
pub fn parse_gateio(
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let value = json(body)?;
    let tickers: HashMap<String, SourceQuote> = value
        .as_array()
        .ok_or_else(|| QuoteFailure::Failed(tr!("Unexpected Gate.io response").to_owned()))?
        .iter()
        .filter_map(gateio_ticker)
        .collect();
    Ok(requests
        .iter()
        .filter_map(|request| {
            let quote = tickers.get(&request.symbol.to_uppercase())?;
            Some((request.symbol.clone(), *quote))
        })
        .collect())
}

/// One Bybit spot ticker (`/market/tickers` and the `tickers` topic carry the same fields): the
/// market and its price, with the change measured against the price 24 hours ago
/// (`prevPrice24h`) and the turnover (`turnover24h`, in the quote currency) as the volume of a
/// dollar-quoted pair, the units traded (`volume24h`) otherwise.
pub fn bybit_ticker(ticker: &Value) -> Option<(String, SourceQuote)> {
    let id = ticker.get("symbol")?.as_str()?.to_uppercase();
    let price = number(ticker.get("lastPrice")).filter(|price| *price > 0.0)?;
    let mut quote = SourceQuote {
        price,
        previous_day_price: number(ticker.get("prevPrice24h")).filter(|open| *open > 0.0),
        ..SourceQuote::default()
    };
    let dollar_quoted = DOLLAR_QUOTES.iter().any(|quote| id.ends_with(quote));
    if let (true, Some(turnover)) = (dollar_quoted, number(ticker.get("turnover24h"))) {
        quote.volume_24h = Some(turnover);
        quote.volume_kind = Some(VolumeKind::QuoteCurrency);
    } else if let Some(units) = number(ticker.get("volume24h")) {
        quote.volume_24h = Some(units);
        quote.volume_kind = Some(VolumeKind::Base);
    }
    Some((id, quote))
}

/// `/market/tickers?category=spot`: every spot market in one answer; the requested ones are kept.
pub fn parse_bybit(
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let value = json(body)?;
    let code = value
        .get("retCode")
        .and_then(Value::as_i64)
        .ok_or_else(|| QuoteFailure::Failed(tr!("Unexpected Bybit response").to_owned()))?;
    if code != 0 {
        return Err(if crate::service::market::bybit::is_rate_limit(code) {
            QuoteFailure::RateLimited
        } else {
            let message = value
                .get("retMsg")
                .and_then(Value::as_str)
                .unwrap_or_default();
            QuoteFailure::Failed(tr!("Bybit error: {message}", message = message))
        });
    }
    let tickers: HashMap<String, SourceQuote> = value
        .pointer("/result/list")
        .and_then(Value::as_array)
        .ok_or_else(|| QuoteFailure::Failed(tr!("Unexpected Bybit response").to_owned()))?
        .iter()
        .filter_map(bybit_ticker)
        .collect();
    Ok(requests
        .iter()
        .filter_map(|request| {
            let quote = tickers.get(&request.symbol.to_uppercase())?;
            Some((request.symbol.clone(), *quote))
        })
        .collect())
}

/// 24h volume as a dollar turnover where the pair is dollar-quoted, otherwise in base units.
/// Kraken reports the base volume and the 24h vwap, so turnover is their product.
pub fn kraken_volume(pair: &str, base_volume: f64, vwap: f64) -> (f64, VolumeKind) {
    let pair = pair.to_uppercase();
    if vwap > 0.0 && DOLLAR_QUOTES.iter().any(|quote| pair.ends_with(quote)) {
        (base_volume * vwap, VolumeKind::QuoteCurrency)
    } else {
        (base_volume, VolumeKind::Base)
    }
}

/// `/Ticker?pair=BTC/USD,…`: the last trade `c[0]`, the 24h volume `v[1]` and vwap `p[1]`. Its
/// `o` is the UTC day's open, not what the price was a day ago, and Kraken's REST has no rolling
/// 24h open, so no change is reported: the ticker socket's `change` gives it, and the engine keeps
/// that reference for a REST refresh of a market the socket has gone quiet about. A refusal of the
/// call is the provider's own code.
pub fn parse_kraken(
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let value = json(body)?;
    if let Some(code) = value
        .get("error")
        .and_then(Value::as_array)
        .and_then(|errors| errors.first())
        .and_then(Value::as_str)
    {
        let text = code
            .split_once(':')
            .map_or(code, |(_, text)| text)
            .to_owned();
        return Err(if crate::service::market::kraken::is_rate_limit(code) {
            QuoteFailure::RateLimited
        } else if code.starts_with("EQuery:") {
            QuoteFailure::Rejected(text)
        } else {
            QuoteFailure::Failed(tr!("Kraken error: {text}", text = text))
        });
    }
    let tickers = value
        .get("result")
        .and_then(Value::as_object)
        .ok_or_else(|| QuoteFailure::Failed(tr!("Unexpected Kraken response").to_owned()))?;
    let mut quotes = HashMap::new();
    for request in requests {
        let Some(ticker) = tickers.get(&request.symbol) else {
            continue;
        };
        let pair = |key: &str, index: usize| number(ticker.get(key)?.get(index));
        let Some(price) = pair("c", 0).filter(|price| *price > 0.0) else {
            continue;
        };
        let mut quote = SourceQuote {
            price,
            timestamp_ms: Some(crate::clock::now_ms()),
            ..SourceQuote::default()
        };
        if let (Some(base), Some(vwap)) = (pair("v", 1), pair("p", 1)) {
            let (volume, kind) = kraken_volume(&request.symbol, base, vwap);
            quote.volume_24h = Some(volume);
            quote.volume_kind = Some(kind);
        }
        quotes.insert(request.symbol.clone(), quote);
    }
    Ok(quotes)
}

/// One OKX spot ticker (`/market/tickers` and the `tickers` channel carry the same fields): the
/// market's id and its price, with the change measured against the rolling 24h open.
pub fn okx_ticker(ticker: &Value) -> Option<(String, SourceQuote)> {
    let id = ticker.get("instId")?.as_str()?.to_uppercase();
    let price = number(ticker.get("last")).filter(|price| *price > 0.0)?;
    let mut quote = SourceQuote {
        price,
        previous_day_price: number(ticker.get("open24h")).filter(|open| *open > 0.0),
        timestamp_ms: number(ticker.get("ts")).map(|ms| ms as i64),
        ..SourceQuote::default()
    };
    // Turnover is only a dollar figure on a dollar-quoted pair; otherwise report units traded.
    let dollar_quoted = DOLLAR_QUOTES.iter().any(|quote| id.ends_with(quote));
    if let (true, Some(turnover)) = (dollar_quoted, number(ticker.get("volCcy24h"))) {
        quote.volume_24h = Some(turnover);
        quote.volume_kind = Some(VolumeKind::QuoteCurrency);
    } else if let Some(units) = number(ticker.get("vol24h")) {
        quote.volume_24h = Some(units);
        quote.volume_kind = Some(VolumeKind::Base);
    }
    Some((id, quote))
}

/// `/market/tickers?instType=SPOT`: every spot market in one answer; the requested ones are kept.
pub fn parse_okx(
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let value = json(body)?;
    if value.get("code").and_then(Value::as_str) != Some("0") {
        let message = value.get("msg").and_then(Value::as_str).unwrap_or_default();
        return Err(QuoteFailure::Failed(format!("OKX error: {message}")));
    }
    let tickers: HashMap<String, SourceQuote> = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| QuoteFailure::Failed(tr!("Unexpected OKX response").to_owned()))?
        .iter()
        .filter_map(okx_ticker)
        .collect();
    Ok(requests
        .iter()
        .filter_map(|request| {
            let quote = tickers.get(&request.symbol.to_uppercase())?;
            Some((request.symbol.clone(), *quote))
        })
        .collect())
}

/// 24h volume as a dollar turnover where the product is dollar-quoted, otherwise in base
/// units. Coinbase reports base volume only, so turnover is that volume priced at the last trade.
pub fn coinbase_volume(product_id: &str, base_volume: f64, price: f64) -> (f64, VolumeKind) {
    let quote = product_id.rsplit('-').next().unwrap_or("").to_uppercase();
    if ["USD", "USDC", "USDT"].contains(&quote.as_str()) {
        (base_volume * price, VolumeKind::QuoteCurrency)
    } else {
        (base_volume, VolumeKind::Base)
    }
}

/// `/products/{id}/stats`: `{"open","high","low","last","volume"}` as strings.
pub fn parse_coinbase_stats(body: &str, product_id: &str) -> Option<SourceQuote> {
    let value: Value = serde_json::from_str(body).ok()?;
    let price = number(value.get("last")).filter(|price| *price > 0.0)?;
    let mut quote = SourceQuote {
        price,
        previous_day_price: number(value.get("open")),
        timestamp_ms: Some(crate::clock::now_ms()),
        ..SourceQuote::default()
    };
    if let Some(base) = number(value.get("volume")) {
        let (volume, kind) = coinbase_volume(product_id, base, price);
        quote.volume_24h = Some(volume);
        quote.volume_kind = Some(kind);
    }
    Some(quote)
}

/// Sources that report a 24h percent change rather than a price:
/// `previous = price / (1 + pct / 100)`.
pub fn from_change_percent(price: f64, change_percent_24h: Option<f64>) -> SourceQuote {
    SourceQuote {
        price,
        previous_day_price: change_percent_24h
            .filter(|percent| *percent > -100.0)
            .map(|percent| price / (1.0 + percent / 100.0)),
        ..SourceQuote::default()
    }
}

pub fn parse_coingecko_markets(
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let value = json(body)?;
    let coins: HashMap<String, &Value> = value
        .as_array()
        .ok_or_else(|| QuoteFailure::Failed(tr!("Unexpected CoinGecko response").to_owned()))?
        .iter()
        .filter_map(|coin| Some((coin.get("id")?.as_str()?.to_lowercase(), coin)))
        .collect();
    let mut quotes = HashMap::new();
    for request in requests {
        let Some(coin) = coins.get(&request.symbol.to_lowercase()) else {
            continue;
        };
        let Some(price) = number(coin.get("current_price")).filter(|price| *price > 0.0) else {
            continue;
        };
        let mut quote = from_change_percent(price, number(coin.get("price_change_percentage_24h")));
        quote.volume_24h = number(coin.get("total_volume"));
        quote.volume_kind = quote.volume_24h.map(|_| VolumeKind::QuoteCurrency);
        quote.timestamp_ms = coin
            .get("last_updated")
            .and_then(Value::as_str)
            .and_then(crate::format::parse_iso8601);
        quotes.insert(request.symbol.clone(), quote);
    }
    Ok(quotes)
}

/// `{"pairs":[{"pairAddress","priceUsd","priceChange":{"h24"},"volume":{"h24"}}]}`.
pub fn parse_dex_pairs(body: &str, requests: &[&QuoteRequest]) -> HashMap<String, SourceQuote> {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return HashMap::new();
    };
    let pairs: HashMap<String, &Value> = value
        .get("pairs")
        .and_then(Value::as_array)
        .map(|pairs| {
            pairs
                .iter()
                .filter_map(|pair| Some((pair.get("pairAddress")?.as_str()?.to_lowercase(), pair)))
                .collect()
        })
        .unwrap_or_default();
    let now = crate::clock::now_ms();
    let mut quotes = HashMap::new();
    for request in requests {
        let Some(pair) = pairs.get(&request.symbol.to_lowercase()) else {
            continue;
        };
        let Some(price) = number(pair.get("priceUsd")).filter(|price| *price > 0.0) else {
            continue;
        };
        let mut quote = from_change_percent(price, number(pair.pointer("/priceChange/h24")));
        quote.volume_24h = number(pair.pointer("/volume/h24"));
        quote.volume_kind = quote.volume_24h.map(|_| VolumeKind::QuoteCurrency);
        quote.timestamp_ms = Some(now);
        quotes.insert(request.symbol.clone(), quote);
    }
    quotes
}

/// The network of the pair whose address is `address`, from a DEXScreener search.
pub fn parse_chain(body: &str, address: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    value
        .get("pairs")?
        .as_array()?
        .iter()
        .find(|pair| {
            pair.get("pairAddress")
                .and_then(Value::as_str)
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(address))
        })?
        .get("chainId")?
        .as_str()
        .map(str::to_owned)
}

/// `/v2/stocks/snapshots`: the latest trade, and the previous session's close as the
/// day-change reference. The daily bar stands in while a symbol has no trade yet.
pub fn parse_alpaca_snapshots(
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let value = json(body)?;
    let snapshots = value
        .as_object()
        .ok_or_else(|| QuoteFailure::Failed(tr!("Unexpected Alpaca response").to_owned()))?;
    let mut quotes = HashMap::new();
    for request in requests {
        let Some(snapshot) = snapshots.get(&request.symbol.to_uppercase()) else {
            continue;
        };
        let Some(price) = number(snapshot.pointer("/latestTrade/p"))
            .or_else(|| number(snapshot.pointer("/dailyBar/c")))
            .filter(|price| *price > 0.0)
        else {
            continue;
        };
        let volume = number(snapshot.pointer("/dailyBar/v"));
        quotes.insert(
            request.symbol.clone(),
            SourceQuote {
                price,
                previous_day_price: number(snapshot.pointer("/prevDailyBar/c")),
                volume_24h: volume,
                volume_kind: volume.map(|_| VolumeKind::Shares),
                timestamp_ms: snapshot
                    .pointer("/latestTrade/t")
                    .and_then(Value::as_str)
                    .and_then(crate::format::parse_iso8601),
            },
        );
    }
    Ok(quotes)
}

#[cfg(test)]
mod tests {
    #[test]
    fn one_futures_market_is_asked_by_name_and_more_as_the_whole_list() {
        let ask = |symbol: &str| QuoteRequest {
            symbol: symbol.to_owned(),
            chain: None,
        };
        assert_eq!(
            binance_url(Market::Perpetual, &[ask("BTCUSDT.P")]),
            "https://fapi.binance.com/fapi/v1/ticker/24hr?symbol=BTCUSDT"
        );
        assert_eq!(
            binance_url(Market::Perpetual, &[ask("BTCUSDT.P"), ask("ETHUSDT.P")]),
            "https://fapi.binance.com/fapi/v1/ticker/24hr"
        );
        let one = r#"{"symbol":"BTCUSDT","lastPrice":"2","openPrice":"1","quoteVolume":"5"}"#;
        let outcome = binance_outcome(Market::Perpetual, one, &[ask("BTCUSDT.P")]).expect("parsed");
        assert_eq!(outcome.quotes["BTCUSDT.P"].price, 2.0);
        assert!(outcome.rejected.is_empty());
        let body = r#"[{"symbol":"BTCUSDT","lastPrice":"2","openPrice":"1","quoteVolume":"5"}]"#;
        let quotes = parse_binance(body, &[ask("BTCUSDT.P")]).expect("parsed");
        assert_eq!(quotes["BTCUSDT.P"].price, 2.0);
        assert_eq!(quotes["BTCUSDT.P"].volume_24h, Some(5.0), "dollar quoted");
    }

    #[test]
    fn a_prediction_quote_compares_with_the_price_a_day_earlier() {
        use Kline;
        let point = |hours: i64, price: f64| Kline {
            open_time: hours * 3_600_000,
            close_time: hours * 3_600_000,
            open: price,
            high: price,
            low: price,
            close: price,
            volume: 0.0,
        };
        let month: Vec<Kline> = (0..=720).map(|hour| point(hour, hour as f64)).collect();
        assert_eq!(super::day_before(&month), Some(696.0));
        // A market younger than a day compares with its first price.
        assert_eq!(
            super::day_before(&[point(0, 0.4), point(5, 0.5)]),
            Some(0.4)
        );
        assert_eq!(super::day_before(&[]), None);
    }

    #[test]
    fn only_a_symbol_level_refusal_makes_the_engine_search_for_the_bad_symbol() {
        use super::classify_status as classify;
        let rejected = |source, code| matches!(classify(source, code), QuoteFailure::Rejected(_));
        // Documented "unknown symbol in this request" answers.
        assert!(rejected(DataSource::Binance, 400));
        assert!(rejected(DataSource::Alpaca, 400));
        assert!(rejected(DataSource::Coinbase, 404));
        // Everything that is about the request, the key, the region or the endpoint.
        for (source, code) in [
            (DataSource::Binance, 401),
            (DataSource::Binance, 403),
            (DataSource::Binance, 404),
            (DataSource::Binance, 418),
            (DataSource::Binance, 451),
            (DataSource::Alpaca, 401),
            (DataSource::Alpaca, 403),
            (DataSource::Alpaca, 404),
            (DataSource::CoinGecko, 400),
            (DataSource::CoinGecko, 404),
            (DataSource::DexScreener, 404),
            (DataSource::Coinbase, 400),
            (DataSource::Coinbase, 403),
        ] {
            assert!(
                matches!(classify(source, code), QuoteFailure::Failed(_)),
                "{source:?} {code}"
            );
        }
        for source in [
            DataSource::Binance,
            DataSource::Alpaca,
            DataSource::Coinbase,
        ] {
            assert_eq!(classify(source, 429), QuoteFailure::RateLimited);
        }
        assert!(
            classify(DataSource::Alpaca, 401)
                .message()
                .contains("API key"),
            "a refused key says so"
        );
    }

    use super::*;

    fn request(symbol: &str) -> QuoteRequest {
        QuoteRequest {
            symbol: symbol.to_owned(),
            chain: None,
        }
    }

    #[test]
    fn okx_answers_every_spot_market_and_the_requested_ones_are_kept() {
        let body = r#"{"code":"0","data":[
            {"instId":"BTC-USDT","last":"64370.7","open24h":"64561.6","vol24h":"10021.2","volCcy24h":"648492231.3","ts":"1794904542764"},
            {"instId":"ETH-BTC","last":"0.05","open24h":"0.049","vol24h":"2000","volCcy24h":"100","ts":"1794904542765"},
            {"instId":"DEAD-USDT","last":"0","open24h":"1","vol24h":"1","volCcy24h":"1","ts":"1"},
            {"instId":"SOL-USDT","last":"150","open24h":"","vol24h":"5","volCcy24h":"750","ts":"1794904542766"}
        ]}"#;
        let quotes = parse_okx(
            body,
            &[
                request("BTC-USDT"),
                request("ETH-BTC"),
                request("DEAD-USDT"),
                request("MISSING-USDT"),
            ],
        )
        .unwrap();
        assert_eq!(
            quotes.len(),
            2,
            "unrequested, unlisted and priceless are left out"
        );
        let btc = &quotes["BTC-USDT"];
        assert_eq!(btc.price, 64_370.7);
        assert_eq!(btc.previous_day_price, Some(64_561.6));
        assert_eq!(btc.timestamp_ms, Some(1_794_904_542_764));
        assert_eq!(
            (btc.volume_24h, btc.volume_kind),
            (Some(648_492_231.3), Some(VolumeKind::QuoteCurrency))
        );
        // A pair quoted in BTC reports the units traded, not a dollar figure.
        let eth = &quotes["ETH-BTC"];
        assert_eq!(
            (eth.volume_24h, eth.volume_kind),
            (Some(2_000.0), Some(VolumeKind::Base))
        );
        // An empty open is no reference.
        let (_, sol) = okx_ticker(
            &serde_json::json!({"instId":"SOL-USDT","last":"150","open24h":"","vol24h":"5","volCcy24h":"750"}),
        )
        .unwrap();
        assert_eq!(sol.previous_day_price, None);
    }

    #[test]
    fn mexc_answers_every_spot_market_with_the_price_24_hours_ago() {
        let body = r#"[
            {"symbol":"BTCUSDT","lastPrice":"64990.71","openPrice":"64185.22","volume":"5486.66","quoteVolume":"354947733.48"},
            {"symbol":"ETHBTC","lastPrice":"0.05","openPrice":"0.049","volume":"2000","quoteVolume":"100"},
            {"symbol":"DEADUSDT","lastPrice":"0","openPrice":"1","volume":"1","quoteVolume":"1"},
            {"symbol":"NEWUSDT","lastPrice":"2","openPrice":"0","volume":"1","quoteVolume":"2"}
        ]"#;
        let quotes = parse_mexc(
            body,
            &[
                request("BTCUSDT"),
                request("ETHBTC"),
                request("DEADUSDT"),
                request("NEWUSDT"),
                request("MISSINGUSDT"),
            ],
        )
        .unwrap();
        assert_eq!(
            quotes.len(),
            3,
            "unrequested, unlisted and priceless are left out"
        );
        let btc = &quotes["BTCUSDT"];
        assert_eq!(btc.price, 64_990.71);
        assert_eq!(btc.previous_day_price, Some(64_185.22));
        assert_eq!(
            (btc.volume_24h, btc.volume_kind),
            (Some(354_947_733.48), Some(VolumeKind::QuoteCurrency))
        );
        // A pair quoted in a coin reports the units traded.
        let eth = &quotes["ETHBTC"];
        assert_eq!(
            (eth.volume_24h, eth.volume_kind),
            (Some(2_000.0), Some(VolumeKind::Base))
        );
        // A market with no open price has no reference rather than a division by zero.
        assert_eq!(quotes["NEWUSDT"].previous_day_price, None);
    }

    #[test]
    fn mexc_odd_answers_and_refusals() {
        assert!(parse_mexc("not json", &[]).is_err());
        assert!(parse_mexc(r#"{"code":-1121,"msg":"Invalid symbol."}"#, &[]).is_err());
        assert!(matches!(
            classify_status(DataSource::Mexc, 403),
            QuoteFailure::Failed(message) if message.contains("not available in every country")
        ));
        assert_eq!(
            classify_status(DataSource::Mexc, 429),
            QuoteFailure::RateLimited
        );
    }

    #[test]
    fn gate_answers_every_spot_market_with_the_price_a_day_ago_from_the_change() {
        let body = r#"[
            {"currency_pair":"BTC_USDT","last":"65100","change_percentage":"2","base_volume":"14040.69","quote_volume":"908513512.9"},
            {"currency_pair":"ETH_BTC","last":"0.05","change_percentage":"-1.5","base_volume":"2000","quote_volume":"100"},
            {"currency_pair":"DEAD_USDT","last":"0","change_percentage":"0","base_volume":"1","quote_volume":"1"},
            {"currency_pair":"WILD_USDT","last":"1","change_percentage":"-100","base_volume":"1","quote_volume":"1"}
        ]"#;
        let quotes = parse_gateio(
            body,
            &[
                request("BTC_USDT"),
                request("ETH_BTC"),
                request("DEAD_USDT"),
                request("WILD_USDT"),
                request("MISSING_USDT"),
            ],
        )
        .unwrap();
        assert_eq!(
            quotes.len(),
            3,
            "unrequested, unlisted and priceless are left out"
        );
        let btc = &quotes["BTC_USDT"];
        assert_eq!(btc.price, 65_100.0);
        assert!((btc.previous_day_price.unwrap() - 63_823.529).abs() < 0.001);
        assert_eq!(
            (btc.volume_24h, btc.volume_kind),
            (Some(908_513_512.9), Some(VolumeKind::QuoteCurrency))
        );
        // A pair quoted in a coin reports the units traded; a fall of 1.5 % has a higher past.
        let eth = &quotes["ETH_BTC"];
        assert!(eth.previous_day_price.unwrap() > 0.05);
        assert_eq!(
            (eth.volume_24h, eth.volume_kind),
            (Some(2_000.0), Some(VolumeKind::Base))
        );
        // A change of -100 % leaves no price a day ago to divide by.
        assert_eq!(quotes["WILD_USDT"].previous_day_price, None);
    }

    #[test]
    fn gate_odd_answers_and_refusals() {
        assert!(parse_gateio("not json", &[]).is_err());
        assert!(parse_gateio(r#"{"label":"INVALID_CURRENCY"}"#, &[]).is_err());
        assert!(matches!(
            classify_status(DataSource::Gate, 403),
            QuoteFailure::Failed(message) if message.contains("not available in every country")
        ));
        assert_eq!(
            classify_status(DataSource::Gate, 429),
            QuoteFailure::RateLimited
        );
    }

    #[test]
    fn bybit_answers_every_spot_market_with_the_real_24h_reference() {
        let body = r#"{"retCode":0,"retMsg":"OK","result":{"category":"spot","list":[
            {"symbol":"BTCUSDT","lastPrice":"64298.1","prevPrice24h":"64675.6","volume24h":"12398.7","turnover24h":"801728667.2"},
            {"symbol":"ETHBTC","lastPrice":"0.05","prevPrice24h":"0.049","volume24h":"2000","turnover24h":"100"},
            {"symbol":"DEADUSDT","lastPrice":"0","prevPrice24h":"1","volume24h":"1","turnover24h":"1"}
        ]},"time":1}"#;
        let quotes = parse_bybit(
            body,
            &[
                request("BTCUSDT"),
                request("ETHBTC"),
                request("DEADUSDT"),
                request("MISSINGUSDT"),
            ],
        )
        .unwrap();
        assert_eq!(
            quotes.len(),
            2,
            "unrequested, unlisted and priceless are left out"
        );
        let btc = &quotes["BTCUSDT"];
        assert_eq!(btc.price, 64_298.1);
        assert_eq!(btc.previous_day_price, Some(64_675.6));
        assert_eq!(
            (btc.volume_24h, btc.volume_kind),
            (Some(801_728_667.2), Some(VolumeKind::QuoteCurrency))
        );
        // A pair quoted in a coin reports the units traded.
        let eth = &quotes["ETHBTC"];
        assert_eq!(
            (eth.volume_24h, eth.volume_kind),
            (Some(2_000.0), Some(VolumeKind::Base))
        );
    }

    #[test]
    fn bybit_reports_its_own_refusals() {
        let refused = |code: i64| {
            parse_bybit(&format!(r#"{{"retCode":{code},"retMsg":"No"}}"#), &[]).unwrap_err()
        };
        assert_eq!(refused(10_006), QuoteFailure::RateLimited);
        assert_eq!(refused(10_018), QuoteFailure::RateLimited);
        assert_eq!(
            refused(10_001),
            QuoteFailure::Failed("Bybit error: No".to_owned())
        );
        assert!(parse_bybit("not json", &[]).is_err());
        assert!(parse_bybit(r#"{"retCode":0}"#, &[]).is_err());
        assert!(matches!(
            classify_status(DataSource::Bybit, 403),
            QuoteFailure::Failed(message) if message.contains("not available in every country")
        ));
        assert_eq!(
            classify_status(DataSource::Bybit, 429),
            QuoteFailure::RateLimited
        );
    }

    #[test]
    fn kraken_answers_by_the_pairs_asked_for_with_the_open_and_the_24h_volume() {
        let body = r#"{"error":[],"result":{
            "BTC/USD":{"c":["64297.1","0.0003"],"o":"64813.7","v":["1383.5","4526.9"],"p":["64571.4","64719.4"]},
            "ETH/BTC":{"c":["0.05","1"],"o":"0.049","v":["10","2000"],"p":["0.05","0.05"]},
            "DEAD/USD":{"c":["0","0"],"o":"1","v":["1","1"],"p":["1","1"]}
        }}"#;
        let quotes = parse_kraken(
            body,
            &[
                request("BTC/USD"),
                request("ETH/BTC"),
                request("DEAD/USD"),
                request("MISSING/USD"),
            ],
        )
        .unwrap();
        assert_eq!(quotes.len(), 2, "priceless and unlisted pairs are left out");
        let btc = &quotes["BTC/USD"];
        assert_eq!(btc.price, 64_297.1);
        // `o` is the UTC day's open, not what the price was a day ago: no change is reported.
        assert_eq!(btc.previous_day_price, None);
        assert_eq!(
            (btc.volume_24h, btc.volume_kind),
            (Some(4526.9 * 64_719.4), Some(VolumeKind::QuoteCurrency))
        );
        // A pair quoted in a coin reports the units traded.
        let eth = &quotes["ETH/BTC"];
        assert_eq!(
            (eth.volume_24h, eth.volume_kind),
            (Some(2_000.0), Some(VolumeKind::Base))
        );
    }

    #[test]
    fn kraken_refusals_are_told_apart_by_their_code() {
        let refused =
            |code: &str| parse_kraken(&format!(r#"{{"error":["{code}"]}}"#), &[]).unwrap_err();
        assert_eq!(
            refused("EQuery:Unknown asset pair"),
            QuoteFailure::Rejected("Unknown asset pair".to_owned())
        );
        assert_eq!(
            refused("EAPI:Rate limit exceeded"),
            QuoteFailure::RateLimited
        );
        assert_eq!(
            refused("EGeneral:Too many requests"),
            QuoteFailure::RateLimited
        );
        assert_eq!(
            refused("EService:Unavailable"),
            QuoteFailure::Failed("Kraken error: Unavailable".to_owned())
        );
        assert!(parse_kraken("not json", &[]).is_err());
        assert!(parse_kraken(r#"{"error":[]}"#, &[]).is_err());
    }

    #[test]
    fn okx_reports_its_own_error_and_odd_answers() {
        let failure = parse_okx(
            r#"{"code":"50011","msg":"Too many requests","data":[]}"#,
            &[],
        );
        assert_eq!(
            failure.unwrap_err(),
            QuoteFailure::Failed("OKX error: Too many requests".to_owned())
        );
        assert!(parse_okx("not json", &[]).is_err());
        assert!(parse_okx(r#"{"code":"0"}"#, &[]).is_err());
    }

    #[test]
    fn binance_mini_ticker_with_dollar_and_base_volume() {
        let body = r#"[
            {"symbol":"BTCUSDT","lastPrice":"67000.5","openPrice":"66000","volume":"100","quoteVolume":"6700000","closeTime":1700000000000},
            {"symbol":"ETHBTC","lastPrice":"0.05","openPrice":"0.049","volume":"2000","quoteVolume":"100","closeTime":1700000000001}
        ]"#;
        let quotes = parse_binance(
            body,
            &[request("BTCUSDT"), request("ETHBTC"), request("GONEUSDT")],
        )
        .unwrap();
        assert_eq!(quotes.len(), 2, "an unanswered symbol is omitted");
        let btc = &quotes["BTCUSDT"];
        assert_eq!(
            (btc.price, btc.previous_day_price),
            (67_000.5, Some(66_000.0))
        );
        assert_eq!(
            (btc.volume_24h, btc.volume_kind),
            (Some(6_700_000.0), Some(VolumeKind::QuoteCurrency))
        );
        assert_eq!(btc.timestamp_ms, Some(1_700_000_000_000));
        let eth = &quotes["ETHBTC"];
        assert_eq!(
            (eth.volume_24h, eth.volume_kind),
            (Some(2_000.0), Some(VolumeKind::Base)),
            "no dollar figure on a non-dollar pair"
        );
    }

    #[test]
    fn the_binance_url_encodes_its_symbol_list() {
        let url = binance_url(Market::Spot, &[request("btcusdt"), request("ETHUSDT")]);
        assert_eq!(
            url,
            "https://api.binance.com/api/v3/ticker/24hr?symbols=%5B%22BTCUSDT%22%2C%22ETHUSDT%22%5D&type=MINI"
        );
        assert!(!url.contains(['[', ']', '"']), "only URL-safe characters");
    }

    #[test]
    fn binance_rejects_non_arrays_and_zero_prices() {
        assert!(matches!(
            parse_binance(r#"{"code":-1121}"#, &[request("X")]),
            Err(QuoteFailure::Failed(_))
        ));
        let zero = r#"[{"symbol":"AUSDT","lastPrice":"0","openPrice":"1"}]"#;
        assert!(parse_binance(zero, &[request("AUSDT")]).unwrap().is_empty());
    }

    #[test]
    fn coinbase_stats_price_volume_in_dollars_only_for_dollar_pairs() {
        let body = r#"{"open":"100","high":"120","low":"90","last":"110","volume":"50"}"#;
        let usd = parse_coinbase_stats(body, "BTC-USD").unwrap();
        assert_eq!((usd.price, usd.previous_day_price), (110.0, Some(100.0)));
        assert_eq!(
            (usd.volume_24h, usd.volume_kind),
            (Some(5_500.0), Some(VolumeKind::QuoteCurrency))
        );
        let btc = parse_coinbase_stats(body, "ETH-BTC").unwrap();
        assert_eq!(
            (btc.volume_24h, btc.volume_kind),
            (Some(50.0), Some(VolumeKind::Base))
        );
        assert!(parse_coinbase_stats(r#"{"last":"0"}"#, "BTC-USD").is_none());
        assert!(parse_coinbase_stats("nope", "BTC-USD").is_none());
    }

    #[test]
    fn coingecko_derives_the_previous_price_from_the_percent_change() {
        let body = r#"[
            {"id":"bitcoin","current_price":110,"price_change_percentage_24h":10,"total_volume":5000,"last_updated":"2024-01-15T15:00:00.123Z"},
            {"id":"dead","current_price":null}
        ]"#;
        let quotes = parse_coingecko_markets(
            body,
            &[request("Bitcoin"), request("dead"), request("absent")],
        )
        .unwrap();
        assert_eq!(quotes.len(), 1);
        let btc = &quotes["Bitcoin"];
        assert!((btc.previous_day_price.unwrap() - 100.0).abs() < 1e-9);
        assert_eq!(btc.volume_kind, Some(VolumeKind::QuoteCurrency));
        assert_eq!(btc.timestamp_ms, Some(1_705_330_800_123));
    }

    #[test]
    fn percent_change_of_minus_100_or_worse_has_no_reference() {
        assert_eq!(
            from_change_percent(5.0, Some(-100.0)).previous_day_price,
            None
        );
        assert_eq!(
            from_change_percent(5.0, Some(-150.0)).previous_day_price,
            None
        );
        assert_eq!(from_change_percent(5.0, None).previous_day_price, None);
        assert_eq!(
            from_change_percent(50.0, Some(-50.0)).previous_day_price,
            Some(100.0)
        );
    }

    #[test]
    fn dex_pairs_match_by_address_ignoring_case() {
        let body = r#"{"pairs":[{"pairAddress":"0xAbC","priceUsd":"0.5","priceChange":{"h24":-50},"volume":{"h24":1234.5}}]}"#;
        let wanted = QuoteRequest {
            symbol: "0xabc".to_owned(),
            chain: Some("solana".to_owned()),
        };
        let other = QuoteRequest {
            symbol: "0xdef".to_owned(),
            chain: Some("solana".to_owned()),
        };
        let quotes = parse_dex_pairs(body, &[&wanted, &other]);
        assert_eq!(quotes.len(), 1);
        let quote = &quotes["0xabc"];
        assert_eq!(quote.price, 0.5);
        assert!((quote.previous_day_price.unwrap() - 1.0).abs() < 1e-9);
        assert_eq!(quote.volume_24h, Some(1234.5));
        assert!(parse_dex_pairs("garbage", &[&wanted]).is_empty());
        assert!(parse_dex_pairs(r#"{"pairs":null}"#, &[&wanted]).is_empty());
    }

    #[test]
    fn chain_comes_from_the_matching_pair() {
        let body = r#"{"pairs":[
            {"pairAddress":"0x111","chainId":"ethereum"},
            {"pairAddress":"0xAAA","chainId":"base"}]}"#;
        assert_eq!(parse_chain(body, "0xaaa").as_deref(), Some("base"));
        assert_eq!(parse_chain(body, "0x999"), None);
        assert_eq!(parse_chain("{}", "0x1"), None);
    }

    #[test]
    fn alpaca_prefers_the_latest_trade_and_falls_back_to_the_daily_bar() {
        let body = r#"{
            "AAPL":{"latestTrade":{"p":190.5,"t":"2024-01-15T15:00:00Z"},"dailyBar":{"c":190,"v":1000},"prevDailyBar":{"c":188}},
            "TSLA":{"dailyBar":{"c":200,"v":5}},
            "ZERO":{"latestTrade":{"p":0}}
        }"#;
        let quotes = parse_alpaca_snapshots(
            body,
            &[
                request("aapl"),
                request("TSLA"),
                request("ZERO"),
                request("MISSING"),
            ],
        )
        .unwrap();
        assert_eq!(quotes.len(), 2);
        let aapl = &quotes["aapl"];
        assert_eq!((aapl.price, aapl.previous_day_price), (190.5, Some(188.0)));
        assert_eq!(aapl.volume_kind, Some(VolumeKind::Shares));
        assert_eq!(aapl.timestamp_ms, Some(1_705_330_800_000));
        assert_eq!(quotes["TSLA"].price, 200.0);
        assert_eq!(quotes["TSLA"].previous_day_price, None);
    }

    #[test]
    fn oanda_quotes_are_the_mid_of_the_inside_bid_and_ask() {
        // Oanda answers with arrays of price levels; the first of each is the inside market.
        let body = r#"{
            "prices":[
                {"instrument":"EUR_USD","bids":[{"price":"1.08501","liquidity":10000000}],"asks":[{"price":"1.08521","liquidity":10000000}],"status":"tradeable","time":"2026-10-09T13:00:00Z"},
                {"instrument":"XAU_USD","bids":[{"price":"2650.10"}],"asks":[{"price":"2650.30"}],"status":"tradeable","time":"2026-10-09T13:00:00Z"},
                {"instrument":"HALTED","bids":[],"asks":[],"closeoutBid":"1.10000","closeoutAsk":"1.10020","status":"halted","time":"2026-10-09T13:00:00Z"},
                {"instrument":"NOLEVELS","status":"halted"}
            ]
        }"#;
        let quotes = parse_oanda_pricing(
            body,
            &[
                request("eur_usd"),
                request("XAU_USD"),
                request("HALTED"),
                request("NOLEVELS"),
                request("ABSENT"),
            ],
        )
        .unwrap();
        assert_eq!(quotes.len(), 3);
        assert_eq!(quotes["eur_usd"].price, 1.08511);
        assert_eq!(
            quotes["eur_usd"].timestamp_ms,
            Some(crate::format::days_from_civil(2026, 10, 9) * 86_400_000 + 13 * 3_600_000)
        );
        assert_eq!(quotes["XAU_USD"].price, 2650.2);
        // A halted market still has a price, from its closeout levels.
        assert_eq!(quotes["HALTED"].price, 1.10010);
        assert!(parse_oanda_pricing("not json", &[request("EUR_USD")]).is_err());
        assert!(
            parse_oanda_pricing(r#"{"errorMessage":"bad"}"#, &[request("EUR_USD")])
                .expect_err("no prices")
                == QuoteFailure::Failed("Unexpected Oanda response".to_owned())
        );
    }

    #[test]
    fn the_day_ago_reference_requests_the_hour_covering_the_cutoff() {
        let hour = crate::format::parse_iso8601("2026-11-25T17:00:00Z").unwrap();
        let now = hour + 24 * HOUR_MS + 123_456;
        let reference = day_ago_reference(now, |start, end| {
            assert_eq!((start, end), (hour, hour + HOUR_MS));
            Ok(vec![Kline {
                open_time: start,
                close_time: end - 1,
                open: 5100.0,
                high: 5110.0,
                low: 5090.0,
                close: 5105.0,
                volume: 1.0,
            }])
        })
        .unwrap();
        assert_eq!(reference.price, Some(5100.0));
    }

    #[test]
    fn a_closed_hour_has_no_reference_and_is_not_fetched_again() {
        let now = crate::format::parse_iso8601("2026-11-29T23:00:00Z").unwrap();
        let reference = day_ago_reference(now, |_, _| Ok(Vec::new())).unwrap();
        assert_eq!(reference.price, None);
        let mut references = DayAgoReferences::default();
        references.insert(OandaEnvironment::Live, "EUR_USD".to_owned(), reference);
        assert!(
            references
                .stale(OandaEnvironment::Live, &[request("EUR_USD")], now + 1)
                .is_empty()
        );
    }

    fn quote(price: f64) -> SourceQuote {
        SourceQuote {
            price,
            previous_day_price: None,
            volume_24h: None,
            volume_kind: None,
            timestamp_ms: None,
        }
    }

    #[test]
    fn the_publication_path_reads_only_the_cache_and_the_rest_is_stale() {
        let requests = vec![
            request("EUR_USD"),
            request("SPX500_USD"),
            request("NOQUOTE"),
        ];
        let mut quotes: HashMap<String, SourceQuote> = requests
            .iter()
            .map(|request| (request.symbol.clone(), quote(1.0)))
            .collect();
        let now = 25 * HOUR_MS - 1;
        let live = OandaEnvironment::Live;
        let mut references = DayAgoReferences::default();
        references.insert(
            live,
            "EUR_USD".to_owned(),
            DayAgoReference {
                hour: day_ago_hour(now),
                price: Some(1.05),
            },
        );
        references.fill(live, &mut quotes, &requests, now);
        assert_eq!(quotes["EUR_USD"].previous_day_price, Some(1.05));
        assert_eq!(
            quotes["SPX500_USD"].previous_day_price, None,
            "no network read on the publication path"
        );
        assert_eq!(
            references.stale(live, &requests, now),
            ["SPX500_USD", "NOQUOTE"],
            "the instruments to refresh in the background"
        );
        // Once the entry ages out it is stale too, and a fresh quote is left without a change.
        let aged = now + 1;
        assert_eq!(
            references.stale(live, &requests, aged),
            ["EUR_USD", "SPX500_USD", "NOQUOTE"]
        );
        let mut fresh: HashMap<String, SourceQuote> =
            HashMap::from([(request("EUR_USD").symbol, quote(1.1))]);
        references.fill(live, &mut fresh, &requests, aged);
        assert_eq!(fresh["EUR_USD"].previous_day_price, None);
    }

    #[test]
    fn a_reference_serves_only_the_environment_it_was_read_from() {
        let now = 25 * HOUR_MS;
        let requests = vec![request("EUR_USD")];
        let mut references = DayAgoReferences::default();
        references.insert(
            OandaEnvironment::Live,
            "EUR_USD".to_owned(),
            DayAgoReference {
                hour: day_ago_hour(now),
                price: Some(1.05),
            },
        );
        let mut quotes = HashMap::from([(requests[0].symbol.clone(), quote(1.1))]);
        references.fill(OandaEnvironment::Practice, &mut quotes, &requests, now);
        assert_eq!(quotes["EUR_USD"].previous_day_price, None);
        assert_eq!(
            references.stale(OandaEnvironment::Practice, &requests, now),
            ["EUR_USD"]
        );
    }

    #[test]
    fn failures_describe_themselves() {
        assert_eq!(
            QuoteFailure::RateLimited.message(),
            "Rate limited by the provider"
        );
        assert!(
            QuoteFailure::CredentialsMissing(DataSource::Alpaca)
                .message()
                .contains("Alpaca")
        );
        assert_eq!(QuoteFailure::Failed("x".into()).message(), "x");
    }

    #[test]
    fn chunk_sizes_respect_each_providers_limit() {
        assert_eq!(chunk_size(DataSource::Coinbase.into()), 20);
        assert_eq!(chunk_size(DataSource::DexScreener.into()), 30);
        assert_eq!(chunk_size(DataSource::CoinGecko.into()), 250);
    }

    #[test]
    fn a_quote_failure_is_worded_in_the_language() {
        use crate::i18n::{Locale, with_locale};
        for failure in [
            QuoteFailure::RateLimited,
            QuoteFailure::CredentialsMissing(DataSource::Oanda),
        ] {
            let english = failure.message();
            assert_ne!(with_locale(Locale::PSEUDO, || failure.message()), english);
        }
        let provider_text = QuoteFailure::Failed("HTTP 500 from the provider".to_owned());
        assert_eq!(
            with_locale(Locale::PSEUDO, || provider_text.message()),
            "HTTP 500 from the provider"
        );
    }
}
