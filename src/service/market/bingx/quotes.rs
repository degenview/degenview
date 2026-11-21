//! BingX's `<market>@ticker` topic: the last price, the price 24 hours ago and the volume of a
//! market, pushed about once a second while it is subscribed. Each market is its own topic, and an
//! unknown one is refused alone, so one connection serves a watchlist up to the 200 topics it
//! takes (the rest is asked over REST).

use super::socket::{TOPICS_PER_CONNECTION, session, subscribe, ticker_topic};
use super::ticker::Ticker;
use super::{Bingx, market_id};
use crate::model::DataSource;
use crate::service::net::ws::Session;
use crate::service::watchlist_quotes::provider::number;
use crate::service::watchlist_quotes::quote_stream::{QuoteProtocol, StreamTick};
use serde_json::Value;

/// `{"code":0,"timestamp":…,"dataType":"BTC-USDT@ticker","data":{"e":"24hTicker","E":…,
/// "s":"BTC-USDT","o":61694.10,"c":58270.01,"v":5031.92,"q":295887886.90,…}}`: the market and its
/// price. Acknowledgements, refusals, pings and other topics carry none.
fn parse_tick(text: &str) -> Option<StreamTick> {
    let value: Value = serde_json::from_str(text).ok()?;
    let data = value.get("data")?;
    if data.get("e")?.as_str()? != "24hTicker" {
        return None;
    }
    let product_id = data.get("s")?.as_str()?.to_owned();
    let ticker = Ticker {
        last: number(data.get("c")),
        // `o` is the price 24 hours ago, a rolling window.
        open: number(data.get("o")),
        base_volume: number(data.get("v")),
        quote_volume: number(data.get("q")),
        time_ms: data.get("E").and_then(Value::as_i64),
    };
    Some(StreamTick {
        source: DataSource::Bingx,
        quote: ticker.quote(&product_id)?,
        product_id,
    })
}

impl QuoteProtocol for Bingx {
    fn session(&self, products: &[String]) -> Session {
        let mut subscriptions: Vec<String> = Vec::new();
        for id in products.iter().filter_map(|product| market_id(product)) {
            let subscription = subscribe(&ticker_topic(&id));
            if !subscriptions.contains(&subscription) {
                subscriptions.push(subscription);
            }
        }
        subscriptions.truncate(TOPICS_PER_CONNECTION);
        session(subscriptions)
    }

    fn parse(&self, text: &str) -> Option<StreamTick> {
        parse_tick(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::watchlist::quote::VolumeKind;

    const PUSH: &str = r#"{"code":0,"timestamp":1795246659730,"dataType":"BTC-USDT@ticker","data":{"e":"24hTicker","E":1795246659730,"s":"BTC-USDT","p":-3424.09,"P":"-5.55%","o":61694.10,"h":62333.56,"l":55159.00,"c":58270.01,"v":5031.92,"q":295887886.90,"O":1795160259730,"C":1795246659730,"A":58270.02,"a":0.008203,"B":58270.01,"b":0.002311}}"#;

    #[test]
    fn parses_a_push_with_the_rolling_open_as_the_reference() {
        let tick = parse_tick(PUSH).expect("tick");
        assert_eq!(tick.source, DataSource::Bingx);
        assert_eq!(tick.product_id, "BTC-USDT");
        assert_eq!(tick.quote.price, 58_270.01);
        assert_eq!(tick.quote.previous_day_price, Some(61_694.10));
        assert_eq!(tick.quote.timestamp_ms, Some(1_795_246_659_730));
        // A dollar-quoted pair reports the turnover.
        assert_eq!(
            (tick.quote.volume_24h, tick.quote.volume_kind),
            (Some(295_887_886.90), Some(VolumeKind::QuoteCurrency))
        );

        // A pair quoted in a coin reports the units traded.
        let coin = PUSH.replace("BTC-USDT", "ETH-BTC");
        let tick = parse_tick(&coin).unwrap();
        assert_eq!(tick.product_id, "ETH-BTC");
        assert_eq!(
            (tick.quote.volume_24h, tick.quote.volume_kind),
            (Some(5031.92), Some(VolumeKind::Base))
        );
    }

    #[test]
    fn acknowledgements_refusals_pings_and_other_topics_are_not_prices() {
        for ignored in [
            r#"{"code":0,"id":"degenview","msg":"SUCCESS","timestamp":1795246659175}"#,
            r#"{"code":100400,"id":"degenview","msg":"dataType is error: NOPE-USDT@ticker","timestamp":1795246659175}"#,
            r#"{"ping":"5795035fdea241aa82008457e74b20c5","time":"2026-11-21T15:38:31.327+0800"}"#,
            r#"{"code":0,"data":{"E":1,"K":{"t":1,"c":"1","i":"1min"},"e":"kline","s":"BTC-USDT"},"dataType":"BTC-USDT@kline_1min"}"#,
            r#"{"code":0,"data":{"e":"24hTicker","E":1,"s":"BTC-USDT","o":1.0,"c":0.0}}"#,
            r#"{"code":0,"data":{"e":"24hTicker","s":"BTC-USDT"}}"#,
            "not json",
        ] {
            assert!(parse_tick(ignored).is_none(), "{ignored}");
        }
    }

    #[test]
    fn every_product_is_its_own_topic_up_to_what_a_connection_takes() {
        let session = QuoteProtocol::session(
            &Bingx,
            &["BTC-USDT".into(), "eth/usdt".into(), "sol".into()],
        );
        assert_eq!(session.url, "wss://open-api-ws.bingx.com/market");
        let topics: Vec<String> = session
            .subscribe
            .iter()
            .map(|message| {
                serde_json::from_str::<Value>(message).unwrap()["dataType"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        assert_eq!(
            topics,
            ["BTC-USDT@ticker", "ETH-USDT@ticker", "SOL-USDT@ticker"]
        );

        let many: Vec<String> = (0..250).map(|index| format!("COIN{index}-USDT")).collect();
        assert_eq!(
            QuoteProtocol::session(&Bingx, &many).subscribe.len(),
            TOPICS_PER_CONNECTION
        );
    }
}
