//! A chart's drawings filed by the market they were drawn on.
//!
//! A chart slot keeps a separate set for each market it has shown: switching the symbol swaps the
//! set and switching back restores it. Saved, a chart's drawings are one flat array (so a layout's
//! drawing records and their ids stay as they were) and each drawing's body names its market.
//! A drawing that names none is dropped on reading.

use super::{DRAWING_FORMAT, Drawing, lenient};
use crate::model::InstrumentId;
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::collections::BTreeMap;

/// Drawings by market key (`InstrumentId::key`). A market with no drawing has no entry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarketDrawings {
    sets: BTreeMap<String, Vec<Drawing>>,
}

impl MarketDrawings {
    /// How many drawings there are in all.
    pub fn count(&self) -> usize {
        self.all().count()
    }

    /// The drawings of `market`, bottom first.
    pub fn get(&self, market: &str) -> &[Drawing] {
        self.sets.get(market).map_or(&[], Vec::as_slice)
    }

    /// Every drawing of every market.
    pub fn all(&self) -> impl Iterator<Item = &Drawing> {
        self.sets.values().flatten()
    }

    /// The markets that have drawings, with them.
    pub fn sets(&self) -> impl Iterator<Item = (&str, &[Drawing])> {
        self.sets
            .iter()
            .map(|(market, set)| (market.as_str(), set.as_slice()))
    }

    /// Takes the drawings of `market` out.
    pub fn take(&mut self, market: &str) -> Vec<Drawing> {
        self.sets.remove(market).unwrap_or_default()
    }

    /// Sets the drawings of `market`; none leaves it without an entry.
    pub fn put(&mut self, market: &str, drawings: Vec<Drawing>) {
        if drawings.is_empty() {
            self.sets.remove(market);
        } else {
            self.sets.insert(market.to_owned(), drawings);
        }
    }

    /// The highest id any drawing was stored with, a newer build's included.
    pub fn highest_id(&self) -> Option<u64> {
        self.all().map(Drawing::stored_id).max()
    }
}

impl Serialize for MarketDrawings {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut flat = serializer.serialize_seq(Some(self.count()))?;
        for (market, drawings) in self.sets() {
            for drawing in drawings {
                flat.serialize_element(
                    &filed(market, drawing).map_err(serde::ser::Error::custom)?,
                )?;
            }
        }
        flat.end()
    }
}

impl<'de> Deserialize<'de> for MarketDrawings {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut drawings = Self::default();
        for value in Vec::<Value>::deserialize(deserializer)? {
            let Some(market) = market_of(&value) else {
                continue;
            };
            let market = InstrumentId::current_key(&market).unwrap_or(market);
            drawings
                .sets
                .entry(market)
                .or_default()
                .push(lenient::read(value));
        }
        Ok(drawings)
    }
}

/// The body of a stored drawing, `{"Shape": {…body…}}`.
fn body(value: &mut Value) -> Option<&mut serde_json::Map<String, Value>> {
    value.as_object_mut()?.values_mut().next()?.as_object_mut()
}

/// The market a stored drawing names.
fn market_of(value: &Value) -> Option<String> {
    value
        .as_object()?
        .values()
        .next()?
        .get("market")?
        .as_str()
        .filter(|market| !market.is_empty())
        .map(str::to_owned)
}

/// One drawing as stored: with its market, and the format that wrote it so an older build keeps it
/// whole instead of dropping the field. A drawing this build cannot read was filed by the market
/// it names, so it is written back as it was; one that names none (it was never read from a
/// row) takes the market it is filed under.
fn filed(market: &str, drawing: &Drawing) -> Result<Value, serde_json::Error> {
    if let Drawing::Unreadable(raw) = drawing {
        let mut raw = raw.clone();
        if market_of(&raw).is_none()
            && let Some(body) = body(&mut raw)
        {
            body.insert("market".into(), market.into());
        }
        return Ok(raw);
    }
    let mut value = serde_json::to_value(drawing)?;
    if let Some(body) = body(&mut value) {
        body.insert("market".into(), market.into());
        body.insert("since".into(), DRAWING_FORMAT.into());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawings_a_release_filed_under_a_requoted_pair_come_back_on_the_pairs_market() {
        let mut stored = MarketDrawings::default();
        stored.put("BINANCE:SUSDTUSDT", vec![line(1)]);
        stored.put("BINANCE:SUSDT", vec![line(2)]);
        stored.put("BINANCE:BTCUSDT", vec![line(3)]);

        let back: MarketDrawings =
            serde_json::from_value(serde_json::to_value(&stored).unwrap()).unwrap();

        assert_eq!(back.get("BINANCE:SUSDT").len(), 2);
        assert!(back.get("BINANCE:SUSDTUSDT").is_empty());
        assert_eq!(back.get("BINANCE:BTCUSDT").len(), 1);
        // Saved again, each names the market it is filed under.
        let saved = serde_json::to_string(&back).unwrap();
        assert!(!saved.contains("SUSDTUSDT"), "{saved}");
    }
    use crate::model::drawings::{Anchor, Tool};
    use serde_json::json;

    fn line(id: u64) -> Drawing {
        Drawing::sample(
            Tool::TrendLine,
            id,
            &[
                Anchor {
                    time_ms: 1_000,
                    price: 10.0,
                },
                Anchor {
                    time_ms: 2_000,
                    price: 20.0,
                },
            ],
            Tool::TrendLine.default_style(),
        )
        .expect("a trend line")
    }

    fn stored(drawings: &MarketDrawings) -> Vec<Value> {
        match serde_json::to_value(drawings).unwrap() {
            Value::Array(values) => values,
            other => panic!("{other}"),
        }
    }

    #[test]
    fn saved_drawings_are_one_flat_list_that_names_each_market_and_its_format() {
        let mut drawings = MarketDrawings::default();
        drawings.put("BINANCE:BTCUSDT", vec![line(1), line(2)]);
        drawings.put("COINBASE:ADA-EUR", vec![line(3)]);
        let values = stored(&drawings);
        assert_eq!(values.len(), 3);
        let markets: Vec<_> = values.iter().map(|v| market_of(v).unwrap()).collect();
        assert_eq!(
            markets,
            ["BINANCE:BTCUSDT", "BINANCE:BTCUSDT", "COINBASE:ADA-EUR"]
        );
        for value in &values {
            let body = value.as_object().unwrap().values().next().unwrap();
            assert_eq!(body["since"], DRAWING_FORMAT);
        }
    }

    #[test]
    fn they_come_back_filed_under_their_markets_in_their_order() {
        let mut drawings = MarketDrawings::default();
        drawings.put("BINANCE:BTCUSDT", vec![line(2), line(1)]);
        drawings.put("COINBASE:ADA-EUR", vec![line(3)]);
        let back: MarketDrawings =
            serde_json::from_value(serde_json::to_value(&drawings).unwrap()).unwrap();
        assert_eq!(back, drawings);
        let ids: Vec<u64> = back
            .get("BINANCE:BTCUSDT")
            .iter()
            .map(Drawing::id)
            .collect();
        assert_eq!(ids, [2, 1]);
        assert!(back.get("OKX:BTC-USDT").is_empty());
        assert_eq!(back.highest_id(), Some(3));
    }

    #[test]
    fn a_drawing_that_names_no_market_is_dropped() {
        let mut bare = serde_json::to_value(line(1)).unwrap();
        body(&mut bare).unwrap().remove("market");
        let named = {
            let mut one = MarketDrawings::default();
            one.put("BINANCE:BTCUSDT", vec![line(2)]);
            stored(&one).remove(0)
        };
        let mut empty = serde_json::to_value(line(3)).unwrap();
        body(&mut empty).unwrap().insert("market".into(), "".into());
        let back: MarketDrawings =
            serde_json::from_value(json!([bare, named, empty, "garbage"])).unwrap();
        assert_eq!(back.count(), 1);
        assert_eq!(back.get("BINANCE:BTCUSDT")[0].id(), 2);
    }

    #[test]
    fn a_drawing_this_build_cannot_read_keeps_its_market_and_its_bytes() {
        let future = json!({"Shape": {
            "id": 41, "market": "BINANCE:BTCUSDT", "since": 9,
            "geometry": {"hologram": {}},
        }});
        let back: MarketDrawings = serde_json::from_value(json!([future.clone()])).unwrap();
        assert!(matches!(
            back.get("BINANCE:BTCUSDT")[0],
            Drawing::Unreadable(_)
        ));
        assert_eq!(back.highest_id(), Some(41));
        assert_eq!(stored(&back), [future]);
    }

    #[test]
    fn an_unreadable_drawing_that_names_no_market_takes_the_one_it_is_filed_under() {
        let mut drawings = MarketDrawings::default();
        drawings.put(
            "BINANCE:BTCUSDT",
            vec![Drawing::Unreadable(
                json!({"Shape": {"id": 5, "geometry": {"x": 1}}}),
            )],
        );
        let back: MarketDrawings =
            serde_json::from_value(serde_json::to_value(&drawings).unwrap()).unwrap();
        assert_eq!(back.get("BINANCE:BTCUSDT").len(), 1);
        assert_eq!(back.highest_id(), Some(5));
    }

    #[test]
    fn a_geometry_it_cannot_read_stays_with_the_market_it_was_drawn_on() {
        let unknown = json!({"Shape": {
            "id": 7, "market": "OKX:BTC-USDT", "geometry": {"hologram": {}},
        }});
        let back: MarketDrawings = serde_json::from_value(json!([unknown.clone()])).unwrap();
        assert_eq!(back.get("OKX:BTC-USDT").len(), 1);
        assert_eq!(stored(&back), [unknown]);
    }

    #[test]
    fn an_empty_set_leaves_no_entry() {
        let mut drawings = MarketDrawings::default();
        drawings.put("BINANCE:BTCUSDT", vec![line(1)]);
        assert_eq!(drawings.take("BINANCE:BTCUSDT").len(), 1);
        assert_eq!(drawings.count(), 0);
        drawings.put("BINANCE:BTCUSDT", Vec::new());
        assert_eq!(drawings.count(), 0);
        assert_eq!(serde_json::to_value(&drawings).unwrap(), json!([]));
    }
}
