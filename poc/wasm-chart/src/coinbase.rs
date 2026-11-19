//! Coinbase Exchange public market data, fetched with the browser's `fetch`.

use thiserror::Error;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::Response;

use degenview_market::bucket::FoldPlan;
use degenview_market::coinbase::{ParseError, parse_candles};
use degenview_market::{Kline, TimeUnit, Timeframe, TimeframeParseError, normalize};

const ENDPOINT: &str = "https://api.exchange.coinbase.com/products";

/// The most candles one request returns.
const MAX_CANDLES: usize = 300;

/// The bar lengths the candles endpoint serves: seconds, and the same as a count of a unit. Every
/// other timeframe is folded from one of these.
const NATIVES: [(u32, u16, TimeUnit); 6] = [
    (60, 1, TimeUnit::Minute),
    (300, 5, TimeUnit::Minute),
    (900, 15, TimeUnit::Minute),
    (3_600, 1, TimeUnit::Hour),
    (21_600, 6, TimeUnit::Hour),
    (86_400, 1, TimeUnit::Day),
];

#[derive(Debug, Error)]
pub enum CoinbaseError {
    #[error("unknown product id {0:?}")]
    BadProduct(String),
    #[error("Coinbase does not serve {0}-second candles")]
    BadGranularity(u32),
    #[error("{0}")]
    Timeframe(#[from] TimeframeParseError),
    #[error("Coinbase has no candles {0} can be built from")]
    Unsupported(String),
    #[error("no browser window")]
    NoWindow,
    #[error("request failed: {0}")]
    Network(String),
    #[error("Coinbase answered HTTP {0}")]
    Http(u16),
    #[error("unexpected response: {0}")]
    Parse(#[from] ParseError),
}

impl From<CoinbaseError> for JsValue {
    fn from(error: CoinbaseError) -> Self {
        JsValue::from_str(&error.to_string())
    }
}

/// The newest candles of a product at a timeframe, and what they were made from.
pub struct Fetched {
    pub plan: FoldPlan,
    /// How many candles of the plan's timeframe one request can fill.
    pub limit: usize,
    /// The timeframe's candles, oldest first: Coinbase's own when it serves that length, else a
    /// finer length folded the way the app folds it.
    pub candles: Vec<Kline>,
    /// The candles Coinbase answered with, of the plan's base length.
    pub base: Vec<Kline>,
}

/// The newest candles of `product`, e.g. `BTC-USD`, at `timeframe`.
pub async fn fetch_timeframe(
    product: &str,
    timeframe: Timeframe,
) -> Result<Fetched, CoinbaseError> {
    let plan = plan(timeframe)?;
    let limit = window(plan);
    let requested = plan.base_count(limit);
    let base = fetch_candles(product, base_seconds(plan)).await?;
    Ok(Fetched {
        plan,
        limit,
        candles: plan.shape(base.clone(), requested, limit),
        base,
    })
}

/// How `timeframe` is served from Coinbase's own candle lengths.
fn plan(timeframe: Timeframe) -> Result<FoldPlan, CoinbaseError> {
    let natives: Vec<Timeframe> = NATIVES
        .iter()
        .filter_map(|&(_, count, unit)| Timeframe::new(count, unit).ok())
        .collect();
    FoldPlan::new(timeframe, &natives).ok_or_else(|| CoinbaseError::Unsupported(timeframe.label()))
}

fn base_seconds(plan: FoldPlan) -> u32 {
    (plan.base().nominal_ms() / 1_000) as u32
}

/// How many whole candles of the plan's timeframe one request can fill. A fold needs one more
/// candle's worth of base candles for the fragment the request's start cuts off.
fn window(plan: FoldPlan) -> usize {
    if plan.is_native() {
        MAX_CANDLES
    } else {
        (MAX_CANDLES / plan.per_target()).saturating_sub(1).max(1)
    }
}

async fn fetch_candles(product: &str, granularity: u32) -> Result<Vec<Kline>, CoinbaseError> {
    let url = candles_url(product, granularity)?;
    let window = web_sys::window().ok_or(CoinbaseError::NoWindow)?;
    let response: Response = JsFuture::from(window.fetch_with_str(&url))
        .await
        .map_err(describe)?
        .unchecked_into();
    if !response.ok() {
        return Err(CoinbaseError::Http(response.status()));
    }
    let body = JsFuture::from(response.text().map_err(describe)?)
        .await
        .map_err(describe)?;
    let candles = parse_candles(
        &body.as_string().unwrap_or_default(),
        i64::from(granularity),
    )?;
    Ok(normalize(candles))
}

fn candles_url(product: &str, granularity: u32) -> Result<String, CoinbaseError> {
    let valid = !product.is_empty()
        && product.len() <= 20
        && product
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !valid {
        return Err(CoinbaseError::BadProduct(product.to_owned()));
    }
    if !NATIVES.iter().any(|&(seconds, ..)| seconds == granularity) {
        return Err(CoinbaseError::BadGranularity(granularity));
    }
    Ok(format!(
        "{ENDPOINT}/{product}/candles?granularity={granularity}"
    ))
}

fn describe(value: JsValue) -> CoinbaseError {
    CoinbaseError::Network(value.as_string().unwrap_or_else(|| format!("{value:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_names_product_and_granularity() {
        assert_eq!(
            candles_url("BTC-USD", 3600).unwrap(),
            "https://api.exchange.coinbase.com/products/BTC-USD/candles?granularity=3600"
        );
    }

    #[test]
    fn url_rejects_a_product_that_could_alter_the_path() {
        assert!(matches!(
            candles_url("BTC-USD/../x", 60),
            Err(CoinbaseError::BadProduct(_))
        ));
        assert!(matches!(
            candles_url("", 60),
            Err(CoinbaseError::BadProduct(_))
        ));
    }

    fn served(text: &str) -> (Timeframe, u32, usize) {
        let timeframe = Timeframe::parse(text).unwrap();
        let plan = plan(timeframe).unwrap();
        (plan.base(), base_seconds(plan), window(plan))
    }

    #[test]
    fn a_length_coinbase_has_is_asked_for_as_it_is() {
        assert_eq!(served("1h"), (Timeframe::HOUR_1, 3_600, 300));
        assert_eq!(served("1D"), (Timeframe::DAY_1, 86_400, 300));
    }

    #[test]
    fn other_lengths_fold_from_the_coarsest_native_that_nests() {
        // Hours of four, twelve and a week are built from hours, 6-hour bars and days.
        assert_eq!(served("4h"), (Timeframe::HOUR_1, 3_600, 74));
        assert_eq!(served("12h").1, 21_600);
        assert_eq!(served("1W"), (Timeframe::DAY_1, 86_400, 41));
        assert_eq!(served("22m").1, 60);
    }

    #[test]
    fn url_rejects_an_unsupported_granularity() {
        assert!(matches!(
            candles_url("BTC-USD", 120),
            Err(CoinbaseError::BadGranularity(120))
        ));
    }
}
