//! What an alert says to the user, and how a macOS notification delivery is retried:
//! the title and body of the notification and the in-app banner, the user's notification
//! settings applied, and the delivery state machine (pending → delivered, or failed with
//! a growing back-off until it is given up on). Pure: the OS call itself is the host's.

// Transient: the alert runtime host wires this when the alert store lands.
#![allow(dead_code)]

use crate::model::alert::{
    AlertDeliveryRecord, AlertDeliveryState, AlertEventOrigin, AlertNotificationSettings,
    AlertTriggerEvent,
};

/// A notification attempt is abandoned after this many failures.
pub const MAXIMUM_ATTEMPTS: u32 = 5;
/// The longest the back-off between attempts grows to.
const MAXIMUM_RETRY_DELAY_SECS: f64 = 900.0;

/// What to hand the OS notification centre.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationContent {
    pub title: String,
    pub body: String,
    /// Request identifier: the trigger's own id, so re-posting the same trigger replaces
    /// rather than duplicates it.
    pub identifier: String,
    pub sound: bool,
    /// Carried through so a click can find the alert.
    pub user_info: Vec<(String, String)>,
}

/// Whether anything is shown at all: the master switch and the macOS switch.
pub fn notifications_enabled(settings: &AlertNotificationSettings) -> bool {
    settings.delivery_enabled && settings.macos_notifications_enabled
}

/// Whether the in-app banner shows: the master switch and the banner switch.
pub fn banners_enabled(settings: &AlertNotificationSettings) -> bool {
    settings.delivery_enabled && settings.in_app_banners_enabled
}

/// "BTC price alert" / "Delayed BTC price alert" for one that was seen late.
pub fn price_alert_title(event: &AlertTriggerEvent) -> String {
    let symbol = &event.asset.symbol;
    if event.origin == AlertEventOrigin::CatchUp {
        format!("Delayed {symbol} price alert")
    } else {
        format!("{symbol} price alert")
    }
}

/// "Reached $70,000.00".
pub fn price_alert_body(event: &AlertTriggerEvent) -> String {
    format!("Reached {}", event.currency.format_price(event.target))
}

pub fn price_alert_content(
    event: &AlertTriggerEvent,
    settings: &AlertNotificationSettings,
) -> NotificationContent {
    NotificationContent {
        title: price_alert_title(event),
        body: price_alert_body(event),
        identifier: event.id.to_string(),
        sound: settings.sound_enabled,
        user_info: vec![
            ("alertID".to_owned(), event.alert_id.to_string()),
            ("eventID".to_owned(), event.id.to_string()),
        ],
    }
}

/// A Pine script alert: the title names the script, symbol and timeframe, the body is the
/// message the script raised ("Alert" when it raised none).
pub fn script_alert_content(
    notification_id: u64,
    subscription_id: u64,
    title: &str,
    message: &str,
    settings: &AlertNotificationSettings,
) -> NotificationContent {
    NotificationContent {
        title: title.to_owned(),
        body: if message.is_empty() {
            "Alert".to_owned()
        } else {
            message.to_owned()
        },
        identifier: notification_id.to_string(),
        sound: settings.sound_enabled,
        user_info: vec![(
            "pineAlertSubscriptionID".to_owned(),
            subscription_id.to_string(),
        )],
    }
}

// MARK: Delivery state machine

/// Seconds to wait before the next attempt after `attempt_count` failures: 30 s, 60 s,
/// 2 min… capped at 15 min.
pub fn retry_delay_secs(attempt_count: u32) -> f64 {
    (2_f64.powi(attempt_count.min(30) as i32) * 15.0).min(MAXIMUM_RETRY_DELAY_SECS)
}

/// A trigger still owed a notification: never tried, or failed and due for another go.
pub fn needs_delivery(record: &AlertDeliveryRecord, now_ms: i64) -> bool {
    match record.state {
        AlertDeliveryState::Pending => true,
        AlertDeliveryState::Failed => record.next_retry_at_ms.is_none_or(|due| due <= now_ms),
        AlertDeliveryState::Delivered | AlertDeliveryState::Suppressed => false,
    }
}

/// The triggers in `history` that are owed a notification, oldest first.
pub fn pending_deliveries(history: &[AlertTriggerEvent], now_ms: i64) -> Vec<&AlertTriggerEvent> {
    let mut owed: Vec<&AlertTriggerEvent> = history
        .iter()
        .filter(|event| needs_delivery(&event.delivery, now_ms))
        .collect();
    owed.sort_by_key(|event| event.timestamp_ms);
    owed
}

/// Notifications are switched off: the trigger is recorded as suppressed, not tried.
pub fn suppressed(record: &AlertDeliveryRecord) -> AlertDeliveryRecord {
    AlertDeliveryRecord {
        state: AlertDeliveryState::Suppressed,
        ..record.clone()
    }
}

/// The record just before an attempt is made.
pub fn begin_attempt(record: &AlertDeliveryRecord, now_ms: i64) -> AlertDeliveryRecord {
    AlertDeliveryRecord {
        attempt_count: record.attempt_count + 1,
        last_attempt_at_ms: Some(now_ms),
        ..record.clone()
    }
}

/// The OS accepted the notification.
pub fn delivered(attempted: &AlertDeliveryRecord) -> AlertDeliveryRecord {
    AlertDeliveryRecord {
        state: AlertDeliveryState::Delivered,
        error: None,
        next_retry_at_ms: None,
        ..attempted.clone()
    }
}

/// The OS refused it: try again later, or give up after `MAXIMUM_ATTEMPTS`.
pub fn failed(attempted: &AlertDeliveryRecord, error: &str, now_ms: i64) -> AlertDeliveryRecord {
    let give_up = attempted.attempt_count >= MAXIMUM_ATTEMPTS;
    AlertDeliveryRecord {
        state: if give_up {
            AlertDeliveryState::Suppressed
        } else {
            AlertDeliveryState::Failed
        },
        error: Some(error.to_owned()),
        next_retry_at_ms: Some(
            now_ms + (retry_delay_secs(attempted.attempt_count) * 1_000.0) as i64,
        ),
        ..attempted.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::alert::{AlertAsset, AlertCurrency};
    use crate::model::{DataSource, InstrumentId};

    fn event(origin: AlertEventOrigin) -> AlertTriggerEvent {
        AlertTriggerEvent {
            id: 42,
            alert_id: 7,
            asset: AlertAsset::new(
                InstrumentId::new(DataSource::Binance, "BTCUSDT"),
                "BTC",
                "Bitcoin",
            ),
            observed_value: 70_050.0,
            target: 70_000.0,
            currency: AlertCurrency::Usd,
            timestamp_ms: 1_000,
            quote_fingerprint: "f".into(),
            origin,
            delivery: AlertDeliveryRecord::default(),
            candle: None,
        }
    }

    #[test]
    fn a_price_alert_says_what_fired_and_at_what_level() {
        let settings = AlertNotificationSettings::default();
        let content = price_alert_content(&event(AlertEventOrigin::Live), &settings);
        assert_eq!(content.title, "BTC price alert");
        assert_eq!(
            content.body, "Reached $70,000.00",
            "the target, not the overshoot"
        );
        assert_eq!(content.identifier, "42");
        assert!(content.sound);
        assert_eq!(
            content.user_info,
            vec![
                ("alertID".into(), "7".into()),
                ("eventID".into(), "42".into())
            ]
        );
    }

    #[test]
    fn a_late_alert_says_so() {
        let content = price_alert_content(
            &event(AlertEventOrigin::CatchUp),
            &AlertNotificationSettings::default(),
        );
        assert_eq!(content.title, "Delayed BTC price alert");
    }

    #[test]
    fn the_sound_setting_is_respected() {
        let quiet = AlertNotificationSettings {
            sound_enabled: false,
            ..Default::default()
        };
        assert!(!price_alert_content(&event(AlertEventOrigin::Live), &quiet).sound);
        assert!(!script_alert_content(1, 2, "t", "m", &quiet).sound);
    }

    #[test]
    fn a_script_alert_defaults_its_body_and_carries_the_subscription() {
        let settings = AlertNotificationSettings::default();
        let content = script_alert_content(9, 3, "Trend · BTCUSDT 1h", "", &settings);
        assert_eq!(
            (content.title.as_str(), content.body.as_str()),
            ("Trend · BTCUSDT 1h", "Alert")
        );
        assert_eq!(content.identifier, "9");
        assert_eq!(
            content.user_info,
            vec![("pineAlertSubscriptionID".into(), "3".into())]
        );
        assert_eq!(
            script_alert_content(9, 3, "t", "Crossed!", &settings).body,
            "Crossed!"
        );
    }

    #[test]
    fn the_master_switch_silences_everything_and_each_channel_has_its_own() {
        let on = AlertNotificationSettings::default();
        assert!(notifications_enabled(&on) && banners_enabled(&on));
        let master_off = AlertNotificationSettings {
            delivery_enabled: false,
            ..Default::default()
        };
        assert!(!notifications_enabled(&master_off) && !banners_enabled(&master_off));
        let no_macos = AlertNotificationSettings {
            macos_notifications_enabled: false,
            ..Default::default()
        };
        assert!(!notifications_enabled(&no_macos) && banners_enabled(&no_macos));
        let no_banner = AlertNotificationSettings {
            in_app_banners_enabled: false,
            ..Default::default()
        };
        assert!(notifications_enabled(&no_banner) && !banners_enabled(&no_banner));
    }

    #[test]
    fn the_retry_delay_doubles_to_a_fifteen_minute_cap() {
        let delays: Vec<f64> = (1..=8).map(retry_delay_secs).collect();
        assert_eq!(
            delays,
            [30.0, 60.0, 120.0, 240.0, 480.0, 900.0, 900.0, 900.0]
        );
        assert_eq!(retry_delay_secs(u32::MAX), 900.0, "no overflow");
    }

    #[test]
    fn a_failed_delivery_is_retried_only_once_it_is_due() {
        let mut record = AlertDeliveryRecord::default();
        assert!(needs_delivery(&record, 0));
        record = failed(&begin_attempt(&record, 1_000), "boom", 1_000);
        assert_eq!(record.state, AlertDeliveryState::Failed);
        assert_eq!(record.error.as_deref(), Some("boom"));
        assert_eq!(record.next_retry_at_ms, Some(31_000));
        assert!(!needs_delivery(&record, 30_999));
        assert!(needs_delivery(&record, 31_000));
        // A failure with no retry time is due at once rather than stranded.
        record.next_retry_at_ms = None;
        assert!(needs_delivery(&record, 0));
    }

    #[test]
    fn delivered_and_suppressed_are_final() {
        let attempted = begin_attempt(&AlertDeliveryRecord::default(), 5);
        assert_eq!(attempted.attempt_count, 1);
        assert_eq!(attempted.last_attempt_at_ms, Some(5));
        let done = delivered(&failed(&attempted, "earlier", 5));
        assert_eq!(done.state, AlertDeliveryState::Delivered);
        assert_eq!(
            (done.error.clone(), done.next_retry_at_ms),
            (None, None),
            "success clears the failure"
        );
        assert!(!needs_delivery(&done, i64::MAX));
        let off = suppressed(&AlertDeliveryRecord::default());
        assert_eq!(off.state, AlertDeliveryState::Suppressed);
        assert!(!needs_delivery(&off, i64::MAX));
    }

    #[test]
    fn five_failures_give_up() {
        let mut record = AlertDeliveryRecord::default();
        for attempt in 1..=MAXIMUM_ATTEMPTS {
            record = failed(&begin_attempt(&record, 0), "no", 0);
            let expected = if attempt < MAXIMUM_ATTEMPTS {
                AlertDeliveryState::Failed
            } else {
                AlertDeliveryState::Suppressed
            };
            assert_eq!(record.state, expected, "attempt {attempt}");
        }
        assert!(!needs_delivery(&record, i64::MAX), "given up for good");
        assert_eq!(record.attempt_count, 5);
    }

    #[test]
    fn owed_deliveries_come_oldest_first_and_skip_the_settled() {
        let make = |id: u64, at: i64, state: AlertDeliveryState, retry: Option<i64>| {
            let mut e = event(AlertEventOrigin::Live);
            e.id = id;
            e.timestamp_ms = at;
            e.delivery = AlertDeliveryRecord {
                state,
                next_retry_at_ms: retry,
                ..Default::default()
            };
            e
        };
        let history = vec![
            make(1, 300, AlertDeliveryState::Pending, None),
            make(2, 100, AlertDeliveryState::Failed, Some(50)),
            make(3, 200, AlertDeliveryState::Failed, Some(9_999)),
            make(4, 50, AlertDeliveryState::Delivered, None),
            make(5, 60, AlertDeliveryState::Suppressed, None),
        ];
        let ids: Vec<u64> = pending_deliveries(&history, 1_000)
            .iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, [2, 1]);
    }
}
