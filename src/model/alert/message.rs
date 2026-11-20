//! Price-alert webhooks: the message a trigger posts. The `{{placeholder}}` rules themselves
//! live in [`crate::bars::alert_message`], which Pine's `alert()` shares.

pub use crate::bars::alert_message::*;
use crate::model::alert::AlertTriggerEvent;

/// What an alert with a webhook but no message of its own posts: an empty message counts as none.
pub fn webhook_template(message: Option<&str>) -> &str {
    message
        .filter(|message| !message.is_empty())
        .unwrap_or(DEFAULT_PRICE_ALERT_TEMPLATE)
}

/// The placeholder values for a trigger. The candle supplies open/high/low/close/volume/time/
/// interval, so `{{close}}` is the bar's close and not the (possibly intrabar) value that
/// crossed the target; a catch-up event has no candle and leaves those placeholders in place.
pub fn context_for_event(event: &AlertTriggerEvent, now_ms: i64) -> AlertMessageContext {
    let candle = event.candle.as_ref();
    AlertMessageContext {
        ticker: Some(event.asset.instrument.api_symbol()),
        exchange: Some(event.asset.instrument.source.display_name().to_owned()),
        interval: candle.map(|candle| trading_view_interval(&candle.interval)),
        open: candle.map(|candle| candle.open),
        high: candle.map(|candle| candle.high),
        low: candle.map(|candle| candle.low),
        close: candle.map(|candle| candle.close),
        volume: candle.map(|candle| candle.volume),
        time_ms: candle.map(|candle| candle.open_time_ms),
        now_ms,
    }
}

/// The rendered webhook body for a trigger.
pub fn render_for_event(message: Option<&str>, event: &AlertTriggerEvent, now_ms: i64) -> String {
    render(webhook_template(message), &context_for_event(event, now_ms))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::alert::AlertEventOrigin;

    // 2024-05-01 13:45:30 UTC.
    const BAR: i64 = 1_714_571_130_000;

    fn trigger(origin: AlertEventOrigin, timestamp_ms: i64) -> AlertTriggerEvent {
        use crate::model::alert::{
            AlertAsset, AlertCandleSnapshot, AlertCurrency, AlertDeliveryRecord,
        };
        use crate::model::{DataSource, InstrumentId};
        AlertTriggerEvent {
            id: 1,
            alert_id: 2,
            asset: AlertAsset::new(
                InstrumentId::new(DataSource::Binance, "BTCUSDT"),
                "BTC",
                "Bitcoin",
            ),
            observed_value: 70_050.0,
            target: 70_000.0,
            currency: AlertCurrency::Usd,
            timestamp_ms,
            quote_fingerprint: "f".into(),
            origin,
            delivery: AlertDeliveryRecord::default(),
            candle: Some(AlertCandleSnapshot {
                open_time_ms: 1_714_571_130_000,
                open: 69_900.0,
                high: 70_100.0,
                low: 69_800.0,
                close: 70_020.5,
                volume: 12.0,
                interval: "1h".into(),
            }),
        }
    }

    #[test]
    fn an_empty_alert_message_falls_back_to_the_default_template() {
        assert_eq!(webhook_template(None), "{{ticker}} alert: {{close}}");
        assert_eq!(webhook_template(Some("")), "{{ticker}} alert: {{close}}");
        assert_eq!(webhook_template(Some("BUY {{ticker}}")), "BUY {{ticker}}");
    }

    #[test]
    fn close_is_the_candles_close_and_catch_up_events_keep_their_placeholders() {
        let event = trigger(AlertEventOrigin::Live, 0);
        assert_eq!(
            render_for_event(None, &event, BAR),
            "BTCUSDT alert: 70020.5",
            "the bar's close, not the 70050 that crossed the target"
        );
        assert_eq!(
            render_for_event(
                Some("{{exchange}} {{open}} {{interval}} {{time}}"),
                &event,
                BAR
            ),
            "Binance 69900 60 2024-05-01T13:45:30Z"
        );
        let mut catch_up = trigger(AlertEventOrigin::CatchUp, 0);
        catch_up.candle = None;
        assert_eq!(
            render_for_event(Some("{{ticker}} {{close}} {{time}}"), &catch_up, BAR),
            "BTCUSDT {{close}} {{time}}"
        );
    }
}
