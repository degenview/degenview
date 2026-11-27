//! Binance's books. Spot: one combined socket with the 100 ms depth diffs and the aggregated
//! trades, a REST snapshot of 5,000 levels a side to start the book from, and the tick size from
//! the exchange info. USDⓈ-M futures: depth and trades are served under different socket paths,
//! the diffs chain by `pu`, the snapshot has 1,000 levels and the tick size comes from the list
//! of markets the search already holds.

use super::DepthError;
use super::levels;
use super::venue::{DepthVenue, Sessions};
use crate::model::depth::{
    BookEvent, BookSync, ChainedDiffStream, Delta, DiffStream, Event, Levels, Price, Snapshot,
    Trade,
};
use crate::model::exchange::Market;
use crate::model::taker_flow::Taker;
use crate::service::market::binance as markets;
use crate::service::net::http::{self, Retry};
use crate::service::net::url::encode_component;
use crate::service::net::ws::{Keepalive, Session};
use crate::service::taker_flow::trade_socket;
use serde_json::Value;
use std::time::Duration;

const HOST: &str = "stream.binance.com";
const REST: &str = "https://api.binance.com/api/v3";
const SNAPSHOT_LEVELS: u32 = 5_000;
const FUTURES_HOST: &str = "fstream.binance.com";
const FUTURES_REST: &str = "https://fapi.binance.com/fapi/v1";
/// The most the futures API gives.
const FUTURES_SNAPSHOT_LEVELS: u32 = 1_000;

pub(super) struct Binance;

impl DepthVenue for Binance {
    fn sessions(&self, symbol: &str) -> Sessions {
        let symbol = symbol.to_lowercase();
        Sessions {
            book: socket(
                HOST,
                format!("/stream?streams={symbol}@depth@100ms/{symbol}@aggTrade"),
            ),
            trades: None,
        }
    }

    fn tick_size(&self, symbol: &str) -> Result<Price, DepthError> {
        let body = get(&format!("{REST}/exchangeInfo?symbol={symbol}"), symbol)?;
        parse_tick_size(&body)
    }

    fn book_sync(&self) -> Box<dyn BookSync> {
        Box::new(DiffStream::default())
    }

    fn snapshot(&self, symbol: &str) -> Result<Snapshot, DepthError> {
        let url = format!("{REST}/depth?symbol={symbol}&limit={SNAPSHOT_LEVELS}");
        get(&url, symbol).and_then(|body| parse_snapshot(&body))
    }

    fn parse(&self, text: &str) -> Vec<Event> {
        parse_message(text).into_iter().collect()
    }
}

/// Binance answers an unknown symbol with HTTP 400.
fn get(url: &str, symbol: &str) -> Result<String, DepthError> {
    http::get_text(url, &[], Retry::Transient).map_err(|error| match error {
        http::Error::Status(400) => DepthError::UnknownSymbol(symbol.to_owned()),
        other => DepthError::Http(other),
    })
}

/// `{"symbols":[{"filters":[{"filterType":"PRICE_FILTER","tickSize":"0.01000000"},…]}]}`.
fn parse_tick_size(body: &str) -> Result<Price, DepthError> {
    let value: Value =
        serde_json::from_str(body).map_err(|_| DepthError::Parse("exchange info"))?;
    value
        .pointer("/symbols/0/filters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|filter| filter.get("filterType").and_then(Value::as_str) == Some("PRICE_FILTER"))
        .and_then(|filter| Price::parse(filter.get("tickSize")?.as_str()?))
        .filter(|tick| *tick > Price::ZERO)
        .ok_or(DepthError::Parse("tick size"))
}

/// `{"lastUpdateId":103300538249,"bids":[["59090.00000000","0.28354000"],…],"asks":[…]}`.
fn parse_snapshot(body: &str) -> Result<Snapshot, DepthError> {
    let value: Value = serde_json::from_str(body).map_err(|_| DepthError::Parse("depth"))?;
    let side = |key| value.get(key).and_then(levels::parse);
    Ok(Snapshot {
        id: value
            .get("lastUpdateId")
            .and_then(Value::as_u64)
            .ok_or(DepthError::Parse("depth"))?,
        levels: Levels {
            bids: side("bids").ok_or(DepthError::Parse("bids"))?,
            asks: side("asks").ok_or(DepthError::Parse("asks"))?,
        },
    })
}

/// A combined-stream message, `{"stream":"btcusdt@depth@100ms","data":{…}}`, or the event alone
/// as a single-stream socket sends it:
/// * `"e":"depthUpdate"`, `"U"` and `"u"` the first and last update ids, `"b"` and `"a"` the
///   changed bids and asks, a size of zero removing the level;
/// * `"e":"aggTrade"`, `"p"` and `"q"`, `"T"` the trade time and `"m"` whether the buyer was the
///   maker, in which case the taker sold.
fn parse_message(text: &str) -> Option<Event> {
    let value: Value = serde_json::from_str(text).ok()?;
    let data = value.get("data").unwrap_or(&value);
    match data.get("e")?.as_str()? {
        "depthUpdate" => Some(Event::Book(BookEvent::Delta(Delta {
            previous: data.get("pu").and_then(Value::as_u64),
            first: data.get("U")?.as_u64()?,
            last: data.get("u")?.as_u64()?,
            levels: Levels {
                bids: levels::parse(data.get("b")?)?,
                asks: levels::parse(data.get("a")?)?,
            },
        }))),
        "aggTrade" => {
            let size = data.get("q")?.as_str()?.parse::<f64>().ok()?;
            let taker = if data.get("m")?.as_bool()? {
                Taker::Sell
            } else {
                Taker::Buy
            };
            (size > 0.0).then_some(Event::Trade(Trade {
                time_ms: data.get("T")?.as_i64()?,
                price: Price::parse(data.get("p")?.as_str()?)?,
                size,
                taker,
            }))
        }
        _ => None,
    }
}

pub(super) struct BinanceFutures;

impl DepthVenue for BinanceFutures {
    fn sessions(&self, symbol: &str) -> Sessions {
        let stream = encode_component(&symbol.to_lowercase());
        Sessions {
            book: socket(
                FUTURES_HOST,
                format!("/public/stream?streams={stream}@depth@100ms"),
            ),
            trades: Some(trade_socket(Market::Perpetual, &stream)),
        }
    }

    fn tick_size(&self, symbol: &str) -> Result<Price, DepthError> {
        let tick = markets::tick_size(Market::Perpetual, symbol)
            .map_err(|_| DepthError::Parse("exchange info"))?
            .ok_or_else(|| DepthError::UnknownSymbol(symbol.to_owned()))?;
        Price::parse(&tick)
            .filter(|tick| *tick > Price::ZERO)
            .ok_or(DepthError::Parse("tick size"))
    }

    fn book_sync(&self) -> Box<dyn BookSync> {
        Box::new(ChainedDiffStream::default())
    }

    fn snapshot(&self, symbol: &str) -> Result<Snapshot, DepthError> {
        let url = format!(
            "{FUTURES_REST}/depth?symbol={}&limit={FUTURES_SNAPSHOT_LEVELS}",
            encode_component(symbol)
        );
        get(&url, symbol).and_then(|body| parse_snapshot(&body))
    }

    fn parse(&self, text: &str) -> Vec<Event> {
        parse_message(text).into_iter().collect()
    }
}

fn socket(host: &str, path: String) -> Session {
    Session {
        url: format!("wss://{host}{path}"),
        host: host.to_owned(),
        port: 443,
        subscribe: Vec::new(),
        // A thin market can be quiet for a long while: the ping says the socket is alive.
        max_data_silence: Duration::from_secs(600),
        keepalive: Keepalive::Frame,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = r#"{"stream":"btcusdt@depth@100ms","data":{"e":"depthUpdate","E":1795199748000,"s":"BTCUSDT","U":157,"u":160,"b":[["59090.00000000","0.50000000"],["59089.99000000","0.00000000"]],"a":[["59090.01000000","6.31874000"]]}}"#;
    const TRADE: &str = r#"{"stream":"btcusdt@aggTrade","data":{"e":"aggTrade","E":1795199748100,"s":"BTCUSDT","a":12345,"p":"59090.01000000","q":"0.25000000","f":100,"l":105,"T":1795199748090,"m":false}}"#;

    fn price(text: &str) -> Price {
        Price::parse(text).unwrap()
    }

    fn only(events: Vec<Event>) -> Event {
        assert_eq!(events.len(), 1, "{events:?}");
        events.into_iter().next().unwrap()
    }

    #[test]
    fn one_combined_socket_carries_the_diffs_and_the_trades_and_nothing_is_sent() {
        let session = Binance.sessions("BTCUSDT").book;
        assert_eq!(
            session.url,
            "wss://stream.binance.com/stream?streams=btcusdt@depth@100ms/btcusdt@aggTrade"
        );
        assert!(session.subscribe.is_empty());
        assert_eq!(session.host, "stream.binance.com");
        assert!(matches!(session.keepalive, Keepalive::Frame));
    }

    #[test]
    fn a_futures_book_and_its_trades_come_from_their_own_paths() {
        let sessions = BinanceFutures.sessions("BTCUSDT");
        assert_eq!(
            sessions.book.url,
            "wss://fstream.binance.com/public/stream?streams=btcusdt@depth@100ms"
        );
        let trades = sessions.trades.expect("a socket of their own");
        assert_eq!(
            trades.url,
            "wss://fstream.binance.com/market/ws/btcusdt@aggTrade"
        );
        let diff = DIFF.replace(r#""U":157,"#, r#""U":157,"pu":150,"#);
        let Event::Book(BookEvent::Delta(delta)) = only(BinanceFutures.parse(&diff)) else {
            panic!("not a delta");
        };
        assert_eq!(delta.previous, Some(150));
        // The trade socket sends the event unwrapped.
        let bare = r#"{"e":"aggTrade","s":"BTCUSDT","p":"1","q":"2","T":3,"m":false}"#;
        assert!(matches!(only(BinanceFutures.parse(bare)), Event::Trade(_)));
    }

    #[test]
    fn a_depth_update_is_a_delta_over_its_update_ids() {
        let Event::Book(BookEvent::Delta(delta)) = only(Binance.parse(DIFF)) else {
            panic!("not a delta");
        };
        assert_eq!((delta.first, delta.last), (157, 160));
        assert_eq!(
            delta.levels.bids,
            [(price("59090"), 0.5), (price("59089.99"), 0.0)]
        );
        assert_eq!(delta.levels.asks, [(price("59090.01"), 6.31874)]);
    }

    #[test]
    fn a_buyer_who_was_not_the_maker_is_a_taker_buy() {
        let Event::Trade(trade) = only(Binance.parse(TRADE)) else {
            panic!("not a trade");
        };
        assert_eq!(trade.taker, Taker::Buy);
        assert_eq!(trade.price, price("59090.01"));
        assert_eq!(trade.size, 0.25);
        assert_eq!(trade.time_ms, 1795199748090);
        let sold = TRADE.replace(r#""m":false"#, r#""m":true"#);
        let Event::Trade(trade) = only(Binance.parse(&sold)) else {
            panic!("not a trade");
        };
        assert_eq!(trade.taker, Taker::Sell);
    }

    #[test]
    fn anything_else_is_ignored() {
        for ignored in [
            "not json",
            "{}",
            r#"{"result":null,"id":1}"#,
            r#"{"stream":"x","data":{"e":"kline"}}"#,
            r#"{"stream":"x","data":{"e":"depthUpdate","U":1,"u":2,"b":[["1","x"]],"a":[]}}"#,
            r#"{"stream":"x","data":{"e":"depthUpdate","U":1,"u":2,"b":[["-1","1"]],"a":[]}}"#,
            r#"{"stream":"x","data":{"e":"aggTrade","p":"1","q":"0","T":1,"m":true}}"#,
            r#"{"stream":"x","data":{"e":"aggTrade","p":"1","q":"1","m":true}}"#,
        ] {
            assert!(Binance.parse(ignored).is_empty(), "{ignored}");
        }
    }

    #[test]
    fn the_snapshot_is_the_rest_depth_with_its_update_id() {
        let body = r#"{"lastUpdateId":103300538249,"bids":[["59090.00000000","0.28354000"]],"asks":[["59090.01000000","6.31874000"]]}"#;
        let snapshot = parse_snapshot(body).unwrap();
        assert_eq!(snapshot.id, 103300538249);
        assert_eq!(snapshot.levels.bids, [(price("59090"), 0.28354)]);
        assert_eq!(snapshot.levels.asks, [(price("59090.01"), 6.31874)]);
        for bad in [
            "nope",
            r#"{"bids":[],"asks":[]}"#,
            r#"{"lastUpdateId":1,"bids":[]}"#,
        ] {
            assert!(
                matches!(parse_snapshot(bad), Err(DepthError::Parse(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_tick_size_is_the_price_filters() {
        let body = r#"{"symbols":[{"symbol":"BTCUSDT","filters":[{"filterType":"LOT_SIZE","stepSize":"0.00001"},{"filterType":"PRICE_FILTER","minPrice":"0.01","maxPrice":"1000000","tickSize":"0.01000000"}]}]}"#;
        assert_eq!(parse_tick_size(body), Ok(price("0.01")));
        for bad in [
            "nope",
            r#"{"symbols":[]}"#,
            r#"{"symbols":[{"filters":[{"filterType":"PRICE_FILTER","tickSize":"0.00000000"}]}]}"#,
        ] {
            assert!(
                matches!(parse_tick_size(bad), Err(DepthError::Parse(_))),
                "{bad}"
            );
        }
    }
}
