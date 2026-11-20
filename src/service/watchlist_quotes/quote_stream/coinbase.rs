//! Coinbase's `ticker` channel. It has no candle channel, but the ticker carries the last trade
//! plus the 24h open and volume.

use super::StreamTick;
use crate::model::DataSource;
use crate::model::watchlist::quote::SourceQuote;
use crate::service::net::ws::{Keepalive, Session};
use crate::service::watchlist_quotes::provider::coinbase_volume;
use degenview_market::coinbase::{STREAM_HOST as HOST, parse_ticker, subscribe_message};
use std::time::Duration;

pub(super) fn session(products: &[String]) -> Session {
    Session {
        url: format!("wss://{HOST}"),
        host: HOST.to_owned(),
        port: 443,
        subscribe: vec![subscribe_message(products)],
        max_data_silence: Duration::from_secs(15),
        keepalive: Keepalive::Frame,
    }
}

/// `{"type":"ticker","product_id":"BTC-USD","price":"67000.1","open_24h":"66000","volume_24h":"123.4","time":"…Z"}`
/// as a quote with the change measured against the rolling 24h open.
pub(super) fn parse_tick(text: &str) -> Option<StreamTick> {
    let ticker = parse_ticker(text)?;
    let mut quote = SourceQuote {
        price: ticker.price,
        previous_day_price: ticker.open_24h,
        timestamp_ms: ticker.time_ms,
        ..SourceQuote::default()
    };
    if let Some(base) = ticker.volume_24h {
        let (volume, kind) = coinbase_volume(&ticker.product_id, base, ticker.price);
        quote.volume_24h = Some(volume);
        quote.volume_kind = Some(kind);
    }
    Some(StreamTick {
        source: DataSource::Coinbase,
        product_id: ticker.product_id,
        quote,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::watchlist::quote::VolumeKind;

    #[test]
    fn a_ticker_message_becomes_a_coinbase_quote() {
        let text = r#"{"type":"ticker","product_id":"btc-usd","price":"67000.1","open_24h":"66000","volume_24h":"10","time":"2024-01-15T15:00:00.250000Z"}"#;
        let tick = parse_tick(text).unwrap();
        assert_eq!(tick.source, DataSource::Coinbase);
        assert_eq!(tick.product_id, "BTC-USD");
        assert_eq!(tick.quote.price, 67_000.1);
        assert_eq!(tick.quote.previous_day_price, Some(66_000.0));
        assert_eq!(tick.quote.timestamp_ms, Some(1_705_330_800_250));
        assert!(parse_tick(r#"{"type":"heartbeat","product_id":"BTC-USD"}"#).is_none());
    }

    #[test]
    fn a_tick_is_a_quote_with_the_24h_open_as_reference() {
        let text = |open: &str, volume: &str| {
            format!(
                r#"{{"type":"ticker","product_id":"ETH-USD","price":"2000","open_24h":"{open}","volume_24h":"{volume}"}}"#
            )
        };
        let quote = parse_tick(&text("1000", "5")).unwrap().quote;
        assert_eq!(quote.previous_day_price, Some(1_000.0));
        assert_eq!(
            (quote.volume_24h, quote.volume_kind),
            (Some(10_000.0), Some(VolumeKind::QuoteCurrency))
        );
        let quote = parse_tick(&text("0", "x")).unwrap().quote;
        assert_eq!(
            quote.previous_day_price, None,
            "a zero open is no reference"
        );
        assert_eq!(quote.volume_24h, None);
    }
}
