//! Hyperliquid's `activeAssetCtx` channel: the mark price, the price a day ago and the day's
//! volume of every market asked for, pushed when they change. One connection, one subscription
//! per market (a name Hyperliquid does not list is answered with an `error` message and the
//! connection stays). A spot pair is subscribed the same way and answered on `activeSpotAssetCtx`.

use super::Hyperliquid;
use super::socket::{channel_message, session};
use super::ticker::context_quote;
use crate::model::DataSource;
use crate::service::net::ws::Session;
use crate::service::watchlist_quotes::quote_stream::{QuoteProtocol, StreamTick};
use serde_json::json;

fn quote_session(products: &[String]) -> Session {
    let subscriptions: Vec<_> = products
        .iter()
        .map(|product| json!({"type": "activeAssetCtx", "coin": product}))
        .collect();
    session(&subscriptions)
}

/// `{"channel":"activeAssetCtx","data":{"coin":"BTC","ctx":{"markPx":"58413.0",
/// "prevDayPx":"62264.0","dayNtlVlm":"5622041538.7",…}}}`.
fn parse_tick(text: &str) -> Option<StreamTick> {
    let (channel, data) = channel_message(text)?;
    if !matches!(channel.as_str(), "activeAssetCtx" | "activeSpotAssetCtx") {
        return None;
    }
    Some(StreamTick {
        source: DataSource::Hyperliquid,
        product_id: data.get("coin")?.as_str()?.to_owned(),
        quote: context_quote(data.get("ctx")?)?,
    })
}

impl QuoteProtocol for Hyperliquid {
    fn session(&self, products: &[String]) -> Session {
        quote_session(products)
    }

    fn parse(&self, text: &str) -> Option<StreamTick> {
        parse_tick(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::watchlist::quote::VolumeKind;
    use serde_json::Value;

    #[test]
    fn every_product_is_a_subscription_of_one_connection() {
        let session = quote_session(&["BTC".into(), "xyz:SP500".into(), "@107".into()]);
        assert_eq!(session.url, "wss://api.hyperliquid.xyz/ws");
        let messages: Vec<Value> = session
            .subscribe
            .iter()
            .map(|message| serde_json::from_str(message).unwrap())
            .collect();
        assert_eq!(
            messages,
            [
                json!({"method": "subscribe", "subscription": {"type": "activeAssetCtx", "coin": "BTC"}}),
                json!({"method": "subscribe", "subscription": {"type": "activeAssetCtx", "coin": "xyz:SP500"}}),
                json!({"method": "subscribe", "subscription": {"type": "activeAssetCtx", "coin": "@107"}}),
            ]
        );
    }

    #[test]
    fn parses_a_perpetual_context_and_a_spot_one() {
        let perp = r#"{"channel":"activeAssetCtx","data":{"coin":"xyz:SP500","ctx":{"funding":"0.00000625","openInterest":"67681.16","prevDayPx":"7165.2","dayNtlVlm":"735223786.8","premium":"-0.0001","oraclePx":"6979.3","markPx":"6978.6","midPx":"6978.15","impactPxs":["6978.1","6978.7"],"dayBaseVlm":"104196.841"}}}"#;
        let tick = parse_tick(perp).expect("tick");
        assert_eq!(tick.source, DataSource::Hyperliquid);
        assert_eq!(tick.product_id, "xyz:SP500");
        assert_eq!(tick.quote.price, 6978.6);
        assert_eq!(tick.quote.previous_day_price, Some(7165.2));
        assert_eq!(
            (tick.quote.volume_24h, tick.quote.volume_kind),
            (Some(735_223_786.8), Some(VolumeKind::QuoteCurrency))
        );

        let spot = r#"{"channel":"activeSpotAssetCtx","data":{"coin":"@107","ctx":{"prevDayPx":"65.973","dayNtlVlm":"143513702.5","markPx":"59.077","midPx":"59.0775","circulatingSupply":"301479424.8","coin":"@107","totalSupply":"998706673.9","dayBaseVlm":"2308171.3"}}}"#;
        let tick = parse_tick(spot).expect("tick");
        assert_eq!(tick.product_id, "@107");
        assert_eq!(tick.quote.price, 59.077);
        assert_eq!(tick.quote.previous_day_price, Some(65.973));
    }

    #[test]
    fn acknowledgements_pongs_errors_candles_and_priceless_markets_are_no_ticks() {
        for ignored in [
            r#"{"channel":"subscriptionResponse","data":{"method":"subscribe","subscription":{"type":"activeAssetCtx","coin":"BTC"}}}"#,
            r#"{"channel":"pong"}"#,
            r#"{"channel":"error","data":"Invalid subscription {\"type\":\"activeAssetCtx\",\"coin\":\"NOPE\"}"}"#,
            r#"{"channel":"candle","data":{"t":1,"T":2,"s":"BTC","i":"1m","o":"1","c":"1","h":"1","l":"1","v":"1","n":1}}"#,
            r#"{"channel":"activeAssetCtx","data":{"coin":"xyz:VIX","ctx":{"markPx":"0","prevDayPx":"20.0"}}}"#,
            r#"{"channel":"activeAssetCtx","data":{"coin":"BTC"}}"#,
            "not json",
        ] {
            assert!(parse_tick(ignored).is_none(), "{ignored}");
        }
    }
}
