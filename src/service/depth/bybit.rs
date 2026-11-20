//! Bybit spot: one socket with the 200-level order book topic (a snapshot when it opens, and
//! again whenever Bybit restarts the book, then deltas) and the public trades, and the tick size
//! from the instrument list. The socket sends its own snapshots, so none is fetched.

use super::DepthError;
use super::levels;
use super::venue::DepthVenue;
use crate::model::depth::{
    BookEvent, BookSync, Delta, Event, Levels, Price, Snapshot, SnapshotAndDeltas, Trade,
};
use crate::model::taker_flow::Taker;
use crate::service::net::http::{self, Retry};
use crate::service::net::ws::{Keepalive, Session};
use serde_json::{Value, json};
use std::time::Duration;

const HOST: &str = "stream.bybit.com";
const REST: &str = "https://api.bybit.com/v5/market";
const BOOK_LEVELS: u32 = 200;

pub(super) struct Bybit;

impl DepthVenue for Bybit {
    fn session(&self, symbol: &str) -> Session {
        let topics = [
            format!("orderbook.{BOOK_LEVELS}.{symbol}"),
            format!("publicTrade.{symbol}"),
        ];
        Session {
            url: format!("wss://{HOST}/v5/public/spot"),
            host: HOST.to_owned(),
            port: 443,
            subscribe: vec![json!({"op": "subscribe", "args": topics}).to_string()],
            // A thin market can be quiet for a long while: the pong says the socket is alive.
            max_data_silence: Duration::from_secs(600),
            keepalive: Keepalive::BYBIT,
        }
    }

    fn tick_size(&self, symbol: &str) -> Result<Price, DepthError> {
        let url = format!("{REST}/instruments-info?category=spot&symbol={symbol}");
        let body = http::get_text(&url, &[], Retry::Transient)?;
        parse_tick_size(&body, symbol)
    }

    fn book_sync(&self) -> Box<dyn BookSync> {
        Box::new(SnapshotAndDeltas::default())
    }

    fn snapshot(&self, _symbol: &str) -> Result<Snapshot, DepthError> {
        Err(DepthError::Unsupported)
    }

    fn parse(&self, text: &str) -> Vec<Event> {
        parse_message(text)
    }
}

/// `{"retCode":0,"result":{"list":[{"symbol":"BTCUSDT","priceFilter":{"tickSize":"0.1"}}]}}`; an
/// empty list is a symbol Bybit does not have.
fn parse_tick_size(body: &str, symbol: &str) -> Result<Price, DepthError> {
    let value: Value =
        serde_json::from_str(body).map_err(|_| DepthError::Parse("instrument info"))?;
    if value.get("retCode").and_then(Value::as_i64) != Some(0) {
        return Err(DepthError::Parse("instrument info"));
    }
    let list = value
        .pointer("/result/list")
        .and_then(Value::as_array)
        .ok_or(DepthError::Parse("instrument info"))?;
    let Some(instrument) = list.first() else {
        return Err(DepthError::UnknownSymbol(symbol.to_owned()));
    };
    instrument
        .pointer("/priceFilter/tickSize")
        .and_then(Value::as_str)
        .and_then(Price::parse)
        .filter(|tick| *tick > Price::ZERO)
        .ok_or(DepthError::Parse("tick size"))
}

/// A topic message, `{"topic":…,"type":…,"data":…}`:
/// * `orderbook.200.<SYMBOL>`: `type` is `snapshot` or `delta`, `data` has `b` and `a` (changed
///   bids and asks, a size of zero removing the level) and `u`, the update id. A `u` of 1 is
///   Bybit restarting the book: a snapshot, whatever `type` says;
/// * `publicTrade.<SYMBOL>`: rows of `T` (ms), `S` (the taker's side), `v` and `p`.
fn parse_message(text: &str) -> Vec<Event> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let (Some(topic), Some(data)) = (
        value.get("topic").and_then(Value::as_str),
        value.get("data"),
    ) else {
        return Vec::new();
    };
    if topic.starts_with("orderbook.") {
        let snapshot = value.get("type").and_then(Value::as_str) == Some("snapshot");
        parse_book(data, snapshot).into_iter().collect()
    } else if topic.starts_with("publicTrade.") {
        data.as_array()
            .into_iter()
            .flatten()
            .filter_map(parse_trade)
            .map(Event::Trade)
            .collect()
    } else {
        Vec::new()
    }
}

fn parse_book(data: &Value, snapshot: bool) -> Option<Event> {
    let id = data.get("u")?.as_u64()?;
    let levels = Levels {
        bids: levels::parse(data.get("b")?)?,
        asks: levels::parse(data.get("a")?)?,
    };
    Some(Event::Book(if snapshot || id == 1 {
        BookEvent::Snapshot(Snapshot { id, levels })
    } else {
        BookEvent::Delta(Delta {
            first: id,
            last: id,
            levels,
        })
    }))
}

fn parse_trade(row: &Value) -> Option<Trade> {
    let taker = match row.get("S")?.as_str()? {
        "Buy" => Taker::Buy,
        "Sell" => Taker::Sell,
        _ => return None,
    };
    let size = row.get("v")?.as_str()?.parse::<f64>().ok()?;
    (size > 0.0).then_some(Trade {
        time_ms: row.get("T")?.as_i64()?,
        price: Price::parse(row.get("p")?.as_str()?)?,
        size,
        taker,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SNAPSHOT: &str = r#"{"topic":"orderbook.200.BTCUSDT","type":"snapshot","ts":1795199748000,"data":{"s":"BTCUSDT","b":[["59090.0","0.5"],["59089.9","1.2"]],"a":[["59090.1","0.3"]],"u":18521288,"seq":7961638724},"cts":1795199747990}"#;
    const DELTA: &str = r#"{"topic":"orderbook.200.BTCUSDT","type":"delta","ts":1795199748200,"data":{"s":"BTCUSDT","b":[["59089.9","0"]],"a":[["59090.1","0.8"]],"u":18521289,"seq":7961638725},"cts":1795199748190}"#;
    const TRADES: &str = r#"{"topic":"publicTrade.BTCUSDT","type":"snapshot","ts":1795199748300,"data":[{"T":1795199748290,"s":"BTCUSDT","S":"Buy","v":"0.001","p":"59090.1","i":"x","BT":false},{"T":1795199748291,"s":"BTCUSDT","S":"Sell","v":"0.5","p":"59090.0","i":"y","BT":false}]}"#;

    fn price(text: &str) -> Price {
        Price::parse(text).unwrap()
    }

    fn only(events: Vec<Event>) -> Event {
        assert_eq!(events.len(), 1, "{events:?}");
        events.into_iter().next().unwrap()
    }

    #[test]
    fn one_subscribe_names_the_book_and_the_trades_of_the_market() {
        let session = Bybit.session("BTCUSDT");
        assert_eq!(session.url, "wss://stream.bybit.com/v5/public/spot");
        let sent: Value = serde_json::from_str(&session.subscribe[0]).unwrap();
        assert_eq!(
            sent,
            json!({"op": "subscribe", "args": ["orderbook.200.BTCUSDT", "publicTrade.BTCUSDT"]})
        );
        assert_eq!(session.subscribe.len(), 1);
        assert_eq!(session.keepalive, Keepalive::BYBIT);
    }

    #[test]
    fn the_socket_sends_its_own_snapshot_so_none_is_fetched() {
        assert_eq!(Bybit.snapshot("BTCUSDT"), Err(DepthError::Unsupported));
        assert!(!Bybit.book_sync().needs_snapshot());
    }

    #[test]
    fn a_snapshot_message_is_a_snapshot_with_its_update_id() {
        let Event::Book(BookEvent::Snapshot(snapshot)) = only(Bybit.parse(SNAPSHOT)) else {
            panic!("not a snapshot");
        };
        assert_eq!(snapshot.id, 18521288);
        assert_eq!(snapshot.levels.bids.len(), 2);
        assert_eq!(snapshot.levels.asks, [(price("59090.1"), 0.3)]);
    }

    #[test]
    fn a_delta_message_is_a_delta_and_one_with_update_id_one_restarts_the_book() {
        let Event::Book(BookEvent::Delta(delta)) = only(Bybit.parse(DELTA)) else {
            panic!("not a delta");
        };
        assert_eq!((delta.first, delta.last), (18521289, 18521289));
        assert_eq!(delta.levels.bids, [(price("59089.9"), 0.0)]);
        let restart = DELTA.replace("18521289", "1");
        assert!(matches!(
            Bybit.parse(&restart)[..],
            [Event::Book(BookEvent::Snapshot(Snapshot { id: 1, .. }))]
        ));
    }

    #[test]
    fn a_trades_side_is_the_takers() {
        let events = Bybit.parse(TRADES);
        let sides: Vec<_> = events
            .iter()
            .map(|event| match event {
                Event::Trade(trade) => (trade.taker, trade.price, trade.size, trade.time_ms),
                other => panic!("not a trade: {other:?}"),
            })
            .collect();
        assert_eq!(
            sides,
            [
                (Taker::Buy, price("59090.1"), 0.001, 1795199748290),
                (Taker::Sell, price("59090"), 0.5, 1795199748291),
            ]
        );
    }

    #[test]
    fn acknowledgements_pongs_other_topics_and_bad_rows_are_ignored() {
        for ignored in [
            "not json",
            r#"{"success":true,"ret_msg":"","op":"subscribe"}"#,
            r#"{"success":true,"ret_msg":"pong","op":"ping"}"#,
            r#"{"topic":"kline.1.BTCUSDT","data":[{"T":1,"S":"Buy","v":"1","p":"1"}]}"#,
            r#"{"topic":"orderbook.200.BTCUSDT","type":"delta","data":{"b":[["x","1"]],"a":[],"u":5}}"#,
            r#"{"topic":"orderbook.200.BTCUSDT","type":"delta","data":{"b":[],"a":[]}}"#,
            r#"{"topic":"publicTrade.BTCUSDT","data":[{"T":1,"S":"None","v":"1","p":"1"}]}"#,
            r#"{"topic":"publicTrade.BTCUSDT","data":[{"T":1,"S":"Buy","v":"0","p":"1"}]}"#,
        ] {
            assert!(Bybit.parse(ignored).is_empty(), "{ignored}");
        }
    }

    #[test]
    fn the_tick_size_is_the_instruments_price_filter() {
        let body = r#"{"retCode":0,"result":{"category":"spot","list":[{"symbol":"BTCUSDT","priceFilter":{"tickSize":"0.1"}}]}}"#;
        assert_eq!(parse_tick_size(body, "BTCUSDT"), Ok(price("0.1")));
        let unknown = r#"{"retCode":0,"result":{"category":"spot","list":[]}}"#;
        assert_eq!(
            parse_tick_size(unknown, "NOPEUSDT"),
            Err(DepthError::UnknownSymbol("NOPEUSDT".to_owned()))
        );
        for bad in [
            "nope",
            r#"{"retCode":10001,"result":{"list":[]}}"#,
            r#"{"retCode":0,"result":{"list":[{"priceFilter":{"tickSize":"0"}}]}}"#,
        ] {
            assert!(
                matches!(parse_tick_size(bad, "X"), Err(DepthError::Parse(_))),
                "{bad}"
            );
        }
    }
}
