//! The klines answer as candles: pure functions over the response text.

use crate::model::Kline;
use crate::service::ProviderError;
use serde_json::Value;

pub(in crate::service) fn parse_klines(body: &str) -> Result<Vec<Kline>, ProviderError> {
    let value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    let rows = value
        .as_array()
        .ok_or_else(|| ProviderError::data("expected a candle array"))?;
    rows.iter().map(parse_row).collect()
}

fn parse_row(row: &Value) -> Result<Kline, ProviderError> {
    let value = |index: usize| -> Result<f64, ProviderError> {
        row.get(index)
            .and_then(Value::as_str)
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| ProviderError::data(format!("invalid field {index}")))
    };
    let time = |index: usize| -> Result<i64, ProviderError> {
        row.get(index)
            .and_then(Value::as_i64)
            .ok_or_else(|| ProviderError::data(format!("invalid timestamp {index}")))
    };

    Ok(Kline {
        open_time: time(0)?,
        open: value(1)?,
        high: value(2)?,
        low: value(3)?,
        close: value(4)?,
        volume: value(5)?,
        close_time: time(6)?,
    })
}
