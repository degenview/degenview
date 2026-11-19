//! Reading the rows exchanges send as JSON arrays of strings.

use serde_json::Value;

use crate::ParseError;

/// The string at `index` of `row`.
pub(crate) fn text(row: &Value, index: usize) -> Result<&str, ParseError> {
    row.get(index)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_field(index))
}

/// The number written as a string at `index` of `row`.
pub(crate) fn number(row: &Value, index: usize) -> Result<f64, ParseError> {
    text(row, index)?.parse().map_err(|_| invalid_field(index))
}

/// The integer written as a string at `index` of `row`: a timestamp.
pub(crate) fn timestamp_text(row: &Value, index: usize) -> Result<i64, ParseError> {
    text(row, index)?
        .parse()
        .map_err(|_| ParseError::invalid("invalid timestamp"))
}

/// The JSON integer at `index` of `row`: a timestamp.
pub(crate) fn timestamp_int(row: &Value, index: usize) -> Result<i64, ParseError> {
    row.get(index)
        .and_then(Value::as_i64)
        .ok_or_else(|| ParseError::invalid("invalid timestamp"))
}

fn invalid_field(index: usize) -> ParseError {
    ParseError::invalid(format!("invalid field {index}"))
}

/// The array at the top of `body`.
pub(crate) fn candle_array(body: &str) -> Result<Vec<Value>, ParseError> {
    match serde_json::from_str(body)? {
        Value::Array(rows) => Ok(rows),
        _ => Err(ParseError::invalid("expected a candle array")),
    }
}

/// The array at the top of `body`; for an object, `message_key`'s text as the exchange's refusal.
pub(crate) fn candle_array_or_message(
    body: &str,
    exchange: &'static str,
    message_key: &str,
) -> Result<Vec<Value>, ParseError> {
    match serde_json::from_str(body)? {
        Value::Array(rows) => Ok(rows),
        value => Err(value.get(message_key).and_then(Value::as_str).map_or_else(
            || ParseError::invalid("expected a candle array"),
            |message| ParseError::Refused {
                exchange,
                message: message.to_owned(),
                rate_limited: false,
            },
        )),
    }
}
