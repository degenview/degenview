use thiserror::Error;
use wasm_bindgen::JsValue;

use degenview_market::{ParseError, Timeframe, TimeframeParseError};

#[derive(Debug, Error)]
pub enum ChartError {
    #[error("unknown exchange {0:?}")]
    UnknownExchange(String),
    #[error("{0:?} is not an asset symbol")]
    BadAsset(String),
    #[error("{0}")]
    Timeframe(#[from] TimeframeParseError),
    #[error("{exchange} has no candles {timeframe} can be built from")]
    Unsupported {
        exchange: &'static str,
        timeframe: Timeframe,
    },
    #[error("no browser window")]
    NoWindow,
    #[error("request failed: {0}")]
    Network(String),
    #[error("{exchange} answered HTTP {status}")]
    Http { exchange: &'static str, status: u16 },
    /// The exchange answered with its own error, in its own words.
    #[error("{0}")]
    Refused(String),
    #[error("{exchange} sent something unexpected: {source}")]
    Parse {
        exchange: &'static str,
        source: ParseError,
    },
}

impl From<ChartError> for JsValue {
    fn from(error: ChartError) -> Self {
        JsValue::from_str(&error.to_string())
    }
}
