//! HTX's `market.<market>.kline.<period>` topics: the forming candle of a market, pushed with each
//! trade, its volume the running total of the candle. Each market is its own `sub`, and one
//! unknown market is refused alone, so every market of a chart shares one connection. The push
//! names its market in its topic.

use super::parse::candle;
use super::socket::{kline_topic, session, subscribe};
use super::{Htx, period, symbol_id};
use crate::model::{Kline, Timeframe};
use crate::service::market::live::CandleProtocol;
use crate::service::net::ws::{Frame, Session};
use serde_json::Value;

/// `{"ch":"market.btcusdt.kline.1min","ts":1795239304805,"tick":{"id":1795239300,"open":55956.07,
/// "close":55946.5,"low":55946.5,"high":55956.07,"amount":0.9132,"vol":51092.9,"count":14}}`: the
/// market as the app spells it and its candle. Acknowledgements, refusals and other topics carry
/// none.
fn parse_kline(text: &str, base: Timeframe) -> Option<(String, Kline)> {
    let value: Value = serde_json::from_str(text).ok()?;
    let (id, tail) = value
        .get("ch")?
        .as_str()?
        .strip_prefix("market.")?
        .split_once('.')?;
    if tail.strip_prefix("kline.")? != period(base)? {
        return None;
    }
    Some((id.to_uppercase(), candle(value.get("tick")?, base)?))
}

impl CandleProtocol for Htx {
    fn sessions(&self, symbols: &[String], base: Timeframe) -> Option<Vec<Session>> {
        let period = period(base)?;
        let subscriptions: Vec<String> = symbols
            .iter()
            .filter_map(|symbol| symbol_id(symbol))
            .map(|id| subscribe(&kline_topic(&id, period)))
            .collect();
        (!subscriptions.is_empty()).then(|| vec![session(subscriptions)])
    }

    fn parse(&self, frame: Frame<'_>, base: Timeframe) -> Option<(String, Kline)> {
        parse_kline(frame.text()?, base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::net::ws::{Decode, Keepalive};

    #[test]
    fn every_market_is_its_own_subscription_on_one_gzip_connection() {
        let sessions = Htx
            .sessions(
                &["BTCUSDT".into(), "eth/usdt".into(), "sol".into()],
                Timeframe::HOUR_1,
            )
            .unwrap();
        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.url, "wss://api.huobi.pro/ws");
        assert_eq!(session.decode, Decode::Gzip);
        assert_eq!(
            session.keepalive,
            Keepalive::EchoJson {
                ask: "ping",
                answer: "pong"
            }
        );
        let topics: Vec<Value> = session
            .subscribe
            .iter()
            .map(|message| serde_json::from_str(message).unwrap())
            .collect();
        assert_eq!(
            topics
                .iter()
                .map(|topic| topic["sub"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "market.btcusdt.kline.60min",
                "market.ethusdt.kline.60min",
                "market.solusdt.kline.60min"
            ]
        );
    }

    #[test]
    fn a_length_htx_has_no_period_for_has_no_session() {
        for other in ["3m", "2h", "8h", "1D", "1W"] {
            let base = Timeframe::parse(other).unwrap();
            assert!(Htx.sessions(&["BTCUSDT".into()], base).is_none(), "{other}");
        }
    }

    #[test]
    fn text_that_cannot_name_a_market_is_not_subscribed() {
        assert!(
            Htx.sessions(&["BTCUSDT&x".into(), "".into()], Timeframe::MINUTE_1)
                .is_none()
        );
        let sessions = Htx
            .sessions(&["BTCUSDT&x".into(), "ETHUSDT".into()], Timeframe::MINUTE_1)
            .unwrap();
        assert_eq!(sessions[0].subscribe.len(), 1);
    }

    const PUSH: &str = r#"{"ch":"market.btcusdt.kline.1min","ts":1795239304805,"tick":{"id":1795239300,"open":55956.07,"close":55946.5,"low":55946.5,"high":55956.07,"amount":0.9132025118946602,"vol":51092.90415109158,"count":14}}"#;

    #[test]
    fn parses_a_push_with_the_close_of_its_bucket() {
        let (symbol, kline) = parse_kline(PUSH, Timeframe::MINUTE_1).expect("parse");
        assert_eq!(symbol, "BTCUSDT");
        assert_eq!(kline.open_time, 1_795_239_300_000);
        assert_eq!(kline.close_time, 1_795_239_359_999);
        assert_eq!(
            (kline.open, kline.high, kline.low, kline.close),
            (55_956.07, 55_956.07, 55_946.5, 55_946.5)
        );
        // The base asset, not the quote turnover.
        assert_eq!(kline.volume, 0.9132025118946602);
    }

    #[test]
    fn acknowledgements_refusals_pings_and_other_topics_are_not_candles() {
        for ignored in [
            r#"{"id":"degenview","status":"ok","subbed":"market.btcusdt.kline.1min","ts":1795239307550}"#,
            r#"{"status":"error","ts":1795239307550,"id":"degenview","err-code":"bad-request","err-msg":"invalid symbol nopexyz"}"#,
            r#"{"ping":1795239308084}"#,
            r#"{"ch":"market.btcusdt.ticker","ts":1,"tick":{"open":1,"close":1}}"#,
            // Another period than the one asked for.
            r#"{"ch":"market.btcusdt.kline.5min","ts":1,"tick":{"id":1795239300,"open":1,"close":1,"low":1,"high":1,"amount":1}}"#,
            r#"{"ch":"market.btcusdt.kline.1min","ts":1,"tick":{"id":1795239300,"open":1}}"#,
            r#"{"ch":"market.btcusdt.kline.1min","ts":1}"#,
            "not json",
        ] {
            assert!(
                parse_kline(ignored, Timeframe::MINUTE_1).is_none(),
                "{ignored}"
            );
        }
    }
}
