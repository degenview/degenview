//! A drawing as TradingView stores it.

use super::{TvSymbol, interval};
use serde_json::{Map, Value};

#[derive(Clone, Debug, PartialEq)]
pub struct RawDrawing {
    pub id: String,
    pub symbol: TvSymbol,
    /// TradingView's tool class: `LineToolTrendLine`.
    pub tool: String,
    /// The tool's own settings.
    pub props: Map<String, Value>,
    pub points: Vec<RawPoint>,
    /// The resolution it was drawn on.
    pub interval: String,
    /// Stacking order; larger is nearer the front.
    pub z: f64,
    /// An X post's address, on a post drawing.
    pub post_url: Option<String>,
}

/// One placement point: a bar's open time in epoch seconds, plus `offset` bars of `interval` for a
/// point past the newest bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RawPoint {
    pub time_s: i64,
    pub offset: i64,
    pub bar_ms: i64,
    pub price: f64,
}

impl RawPoint {
    pub fn time_ms(&self) -> i64 {
        self.time_s
            .saturating_mul(1_000)
            .saturating_add(self.offset.saturating_mul(self.bar_ms))
    }
}

impl RawDrawing {
    /// Reads one entry of a sources response; `None` when it is not a drawing on a market.
    pub fn read(id: &str, source: &Value) -> Option<Self> {
        let state = source.get("state")?;
        let tool = state.get("type")?.as_str()?;
        if !tool.starts_with("LineTool") {
            return None;
        }
        let props = state.get("state")?.as_object()?.clone();
        let symbol = source
            .get("symbol")
            .or_else(|| props.get("symbol"))
            .and_then(Value::as_str)
            .and_then(TvSymbol::read)?;
        let interval = props
            .get("interval")
            .and_then(Value::as_str)
            .unwrap_or("D")
            .to_owned();
        let points = state
            .get("points")
            .and_then(Value::as_array)?
            .iter()
            .map(|point| {
                let of = point
                    .get("interval")
                    .and_then(Value::as_str)
                    .unwrap_or(&interval);
                Some(RawPoint {
                    time_s: point.get("time_t").and_then(Value::as_i64)?,
                    offset: point.get("offset").and_then(Value::as_i64).unwrap_or(0),
                    bar_ms: interval::bar_ms(of).unwrap_or(0),
                    price: point.get("price").and_then(Value::as_f64)?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let post_url = state
            .get("tweetData")
            .or_else(|| source.get("tweetData"))
            .and_then(|tweet| tweet.get("tweetUrl"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        Some(Self {
            id: id.to_owned(),
            symbol,
            tool: tool.to_owned(),
            props,
            points,
            interval,
            z: state.get("zorder").and_then(Value::as_f64).unwrap_or(0.0),
            post_url,
        })
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        self.props.get(key).and_then(Value::as_str)
    }

    pub fn number(&self, key: &str) -> Option<f64> {
        self.props.get(key).and_then(Value::as_f64)
    }

    pub fn flag(&self, key: &str) -> Option<bool> {
        self.props.get(key).and_then(Value::as_bool)
    }
}
