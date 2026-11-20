//! The shape both exchanges send a side of the book in: `[["59090.00","0.283"],…]`, price and size
//! as decimal strings.

use crate::model::depth::Price;
use serde_json::Value;

/// The levels `value` lists; `None` when it is not a list of price and size pairs, so that a
/// damaged message is dropped whole rather than half applied.
pub(super) fn parse(value: &Value) -> Option<Vec<(Price, f64)>> {
    value
        .as_array()?
        .iter()
        .map(|level| {
            let price = Price::parse(level.get(0)?.as_str()?)?;
            let size = level.get(1)?.as_str()?.parse::<f64>().ok()?;
            (size >= 0.0).then_some((price, size))
        })
        .collect()
}
