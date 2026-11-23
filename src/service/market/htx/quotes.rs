//! HTX's `market.<market>.ticker` topic: the last trade, the price 24 hours ago and the volume of
//! every market asked for, pushed every 100 ms while it trades. Each market is its own `sub`, and
//! one unknown market is refused alone, so one connection serves a whole watchlist.

use super::socket::{session, subscribe, ticker_topic};
use super::ticker::price_and_volume;
use super::{Htx, symbol_id};
use crate::model::DataSource;
use crate::service::net::ws::Session;
use crate::service::watchlist_quotes::provider::number;
use crate::service::watchlist_quotes::quote_stream::{QuoteProtocol, StreamTick};
use serde_json::Value;

/// `{"ch":"market.btcusdt.ticker","ts":1795239307400,"tick":{"open":61633.84,"close":55946.5,
/// "amount":7265.5,"vol":4.3e8,"lastPrice":55946.51,…}}`: the market as the app spells it and its
/// price. Acknowledgements, refusals, pings and other topics carry none.
fn parse_tick(text: &str) -> Option<StreamTick> {
    let value: Value = serde_json::from_str(text).ok()?;
    let id = value
        .get("ch")?
        .as_str()?
        .strip_prefix("market.")?
        .strip_suffix(".ticker")?;
    let product_id = id.to_uppercase();
    let tick = value.get("tick")?;
    let mut quote = price_and_volume(&product_id, tick)?;
    // The topic's `open` is the price 24 hours ago, a rolling window.
    quote.previous_day_price = number(tick.get("open")).filter(|open| *open > 0.0);
    quote.timestamp_ms = value.get("ts").and_then(Value::as_i64);
    Some(StreamTick {
        source: DataSource::Htx,
        product_id,
        quote,
    })
}

impl QuoteProtocol for Htx {
    fn session(&self, products: &[String]) -> Session {
        session(
            products
                .iter()
                .filter_map(|product| symbol_id(product))
                .map(|id| subscribe(&ticker_topic(&id)))
                .collect(),
        )
    }

    fn parse(&self, text: &str) -> Option<StreamTick> {
        parse_tick(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::watchlist::quote::VolumeKind;

    #[test]
    fn every_product_is_its_own_subscription() {
        let session = QuoteProtocol::session(&Htx, &["BTCUSDT".into(), "ETHBTC".into()]);
        assert_eq!(session.url, "wss://api.huobi.pro/ws");
        let sub = |message: &str| -> String {
            serde_json::from_str::<Value>(message).unwrap()["sub"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        assert_eq!(
            session
                .subscribe
                .iter()
                .map(|message| sub(message))
                .collect::<Vec<_>>(),
            ["market.btcusdt.ticker", "market.ethbtc.ticker"]
        );
    }

    #[test]
    fn parses_a_ticker_with_the_rolling_open_as_the_reference_and_ignores_the_rest() {
        let text = r#"{"ch":"market.btcusdt.ticker","ts":1795239307400,"tick":{"open":61633.84,"high":62358.21,"low":55555.55,"close":55946.5,"amount":7265.545529621383,"vol":4.304102704577051E8,"count":158019,"bid":55940.74,"bidSize":2.3E-4,"ask":55940.75,"askSize":0.03,"lastPrice":55946.51,"lastSize":0.01}}"#;
        let tick = parse_tick(text).expect("tick");
        assert_eq!(tick.source, DataSource::Htx);
        assert_eq!(tick.product_id, "BTCUSDT");
        assert_eq!(tick.quote.price, 55_946.51);
        assert_eq!(tick.quote.previous_day_price, Some(61_633.84));
        assert_eq!(tick.quote.timestamp_ms, Some(1_795_239_307_400));
        assert_eq!(
            (tick.quote.volume_24h, tick.quote.volume_kind),
            (Some(4.304102704577051E8), Some(VolumeKind::QuoteCurrency))
        );

        // A pair quoted in a coin reports the units traded.
        let coin = text.replace("btcusdt", "ethbtc");
        let tick = parse_tick(&coin).unwrap();
        assert_eq!(tick.product_id, "ETHBTC");
        assert_eq!(
            (tick.quote.volume_24h, tick.quote.volume_kind),
            (Some(7265.545529621383), Some(VolumeKind::Base))
        );

        for ignored in [
            r#"{"id":"degenview","status":"ok","subbed":"market.btcusdt.ticker","ts":1}"#,
            r#"{"status":"error","ts":1,"id":"degenview","err-code":"bad-request","err-msg":"invalid symbol nopexyz"}"#,
            r#"{"ping":1795239308084}"#,
            r#"{"ch":"market.btcusdt.kline.1min","ts":1,"tick":{"id":1,"close":1}}"#,
            r#"{"ch":"market.btcusdt.ticker","ts":1,"tick":{"open":1.0,"close":0.0}}"#,
            r#"{"ch":"market.btcusdt.ticker","ts":1}"#,
            "not json",
        ] {
            assert!(parse_tick(ignored).is_none(), "{ignored}");
        }
    }
}
