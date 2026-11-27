//! Binance's `<symbol>@aggTrade` stream, spot and USDⓈ-M futures: the stream is in the URL, so
//! nothing is sent. An aggregated trade is the fills of one taker order at one price, which is what
//! flow needs.

use crate::model::exchange::{Market, Venue};
use crate::model::taker_flow::{Taker, Trade};
use crate::service::net::ws::{Keepalive, Session};
use serde_json::Value;
use std::time::Duration;

const SPOT_HOST: &str = "stream.binance.com";
const FUTURES_HOST: &str = "fstream.binance.com";

/// The `{ASSET}USDT` market of `market`; `asset` is upper case letters and digits.
pub(super) fn session(market: Market, asset: &str) -> Session {
    trade_socket(market, &format!("{}usdt", asset.to_lowercase()))
}

/// The trades of `pair`, written as a stream names it: lower case.
pub(in crate::service) fn trade_socket(market: Market, pair: &str) -> Session {
    let stream = format!("{pair}@aggTrade");
    let (host, path) = match market {
        Market::Spot => (SPOT_HOST, format!("/ws/{stream}")),
        Market::Perpetual => (FUTURES_HOST, format!("/market/ws/{stream}")),
    };
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

/// `{"e":"aggTrade","s":"BTCUSDT","p":"16578.5","q":"0.001","T":1672515782136,"m":true,…}`. `m` is
/// whether the buyer was the market maker: then the taker sold.
pub(super) fn parse(venue: Venue, text: &str) -> Vec<Trade> {
    parse_trade(venue, text).into_iter().collect()
}

fn parse_trade(venue: Venue, text: &str) -> Option<Trade> {
    let value: Value = serde_json::from_str(text).ok()?;
    if value.get("e")?.as_str()? != "aggTrade" {
        return None;
    }
    let number = |key: &str| value.get(key)?.as_str()?.parse::<f64>().ok();
    let notional = number("p")? * number("q")?;
    let taker = if value.get("m")?.as_bool()? {
        Taker::Sell
    } else {
        Taker::Buy
    };
    let time_ms = value.get("T")?.as_i64()?;
    (notional > 0.0).then_some(Trade {
        venue,
        notional,
        taker,
        time_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRADE: &str = r#"{"e":"aggTrade","E":1672515782136,"s":"BTCUSDT","a":12345,"p":"20000.5","q":"0.25","f":100,"l":105,"T":1672515782130,"m":false}"#;

    fn venue() -> Venue {
        Venue::ALL[0]
    }

    #[test]
    fn spot_and_futures_streams_are_in_the_url_and_nothing_is_sent() {
        let spot = session(Market::Spot, "BTC");
        assert_eq!(spot.url, "wss://stream.binance.com/ws/btcusdt@aggTrade");
        assert!(spot.subscribe.is_empty());
        let perp = session(Market::Perpetual, "1000PEPE");
        assert_eq!(
            perp.url,
            "wss://fstream.binance.com/market/ws/1000pepeusdt@aggTrade"
        );
        assert_eq!(perp.host, "fstream.binance.com");
        assert!(matches!(perp.keepalive, Keepalive::Frame));
    }

    #[test]
    fn a_buyer_who_was_not_the_maker_is_a_taker_buy() {
        let [trade] = parse(venue(), TRADE).try_into().unwrap();
        assert_eq!(trade.taker, Taker::Buy);
        assert_eq!(trade.notional, 20000.5 * 0.25);
        assert_eq!(trade.time_ms, 1672515782130);
        assert_eq!(trade.venue, venue());
    }

    #[test]
    fn a_buyer_who_was_the_maker_means_the_taker_sold() {
        let sold = TRADE.replace(r#""m":false"#, r#""m":true"#);
        let [trade] = parse(venue(), &sold).try_into().unwrap();
        assert_eq!(trade.taker, Taker::Sell);
    }

    #[test]
    fn anything_else_is_ignored() {
        for ignored in [
            "not json",
            "{}",
            r#"{"result":null,"id":1}"#,
            r#"{"e":"kline","s":"BTCUSDT"}"#,
            r#"{"e":"aggTrade","s":"BTCUSDT","p":"0","q":"1","T":1,"m":true}"#,
            r#"{"e":"aggTrade","s":"BTCUSDT","p":"1","q":"1","m":true}"#,
        ] {
            assert!(parse(venue(), ignored).is_empty(), "{ignored}");
        }
    }
}
