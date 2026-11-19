//! Coinbase Exchange public market data, fetched with the browser's `fetch`.

use thiserror::Error;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::Response;

use degenview_market::coinbase::{ParseError, parse_candles};
use degenview_market::{Kline, normalize};

const ENDPOINT: &str = "https://api.exchange.coinbase.com/products";

/// Bar lengths, in seconds, that the candles endpoint accepts.
pub const GRANULARITIES: [u32; 6] = [60, 300, 900, 3600, 21_600, 86_400];

#[derive(Debug, Error)]
pub enum CoinbaseError {
    #[error("unknown product id {0:?}")]
    BadProduct(String),
    #[error("Coinbase does not serve {0}-second candles")]
    BadGranularity(u32),
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

/// The newest (up to 300) candles of `product`, e.g. `BTC-USD`, oldest first.
pub async fn fetch_candles(product: &str, granularity: u32) -> Result<Vec<Kline>, CoinbaseError> {
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
    if !GRANULARITIES.contains(&granularity) {
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

    #[test]
    fn url_rejects_an_unsupported_granularity() {
        assert!(matches!(
            candles_url("BTC-USD", 120),
            Err(CoinbaseError::BadGranularity(120))
        ));
    }
}
