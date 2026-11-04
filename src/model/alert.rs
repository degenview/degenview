#![allow(dead_code)]
//! Price-alert domain: conditions, rules, trigger events, runtime health and the
//! snapshot a store persists. Pure data — evaluation lives in `alert_engine`, saving in
//! the store. Prices are `f64` (the Swift app used `Decimal`); crossings compare the
//! quote's own digits, so rounding at the display edge never moves a trigger.

use crate::model::watchlist::new_id;
use crate::model::{DataSource, InstrumentId};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub type AlertId = u64;

// MARK: Rules

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum AlertFrequency {
    #[default]
    Once,
    EveryTime,
}

impl AlertFrequency {
    pub const ALL: [Self; 2] = [Self::Once, Self::EveryTime];

    pub fn title(self) -> &'static str {
        match self {
            Self::Once => "Once",
            Self::EveryTime => "Every Time",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum AlertState {
    #[default]
    Active,
    Paused,
    Triggered,
    Unsupported,
}

/// What an alert watches for. `RisesBy`/`FallsBy` carry the reference price and the
/// target derived from it when the alert was made, so only `target` is evaluated.
#[derive(Clone, Debug, PartialEq)]
pub enum AlertCondition {
    CrossesAbove {
        target: f64,
    },
    CrossesBelow {
        target: f64,
    },
    RisesBy {
        percent: f64,
        reference: f64,
        target: f64,
    },
    FallsBy {
        percent: f64,
        reference: f64,
        target: f64,
    },
    /// A condition written by a newer version; kept so the row survives a round trip.
    Unsupported {
        tag: String,
    },
}

impl AlertCondition {
    pub fn rises_by(percent: f64, reference: f64) -> Self {
        Self::RisesBy {
            percent,
            reference,
            target: reference * (1.0 + percent / 100.0),
        }
    }

    pub fn falls_by(percent: f64, reference: f64) -> Self {
        Self::FallsBy {
            percent,
            reference,
            target: reference * (1.0 - percent / 100.0),
        }
    }

    pub fn target(&self) -> Option<f64> {
        match self {
            Self::CrossesAbove { target }
            | Self::CrossesBelow { target }
            | Self::RisesBy { target, .. }
            | Self::FallsBy { target, .. } => Some(*target),
            Self::Unsupported { .. } => None,
        }
    }

    /// Whether the price has to climb to the target (rather than fall to it).
    pub fn crosses_above(&self) -> bool {
        matches!(self, Self::CrossesAbove { .. } | Self::RisesBy { .. })
    }

    /// Whether a price standing at `price` is on the side of `target` the alert has yet
    /// to cross from — i.e. the alert is armed.
    pub fn is_armed_at(&self, price: f64) -> Option<bool> {
        let target = self.target()?;
        Some(if self.crosses_above() {
            price < target
        } else {
            price > target
        })
    }

    pub fn summary(&self) -> String {
        match self {
            Self::CrossesAbove { .. } => "Crosses above".to_owned(),
            Self::CrossesBelow { .. } => "Crosses below".to_owned(),
            Self::RisesBy { percent, .. } => format!("Rises by {percent}%"),
            Self::FallsBy { percent, .. } => format!("Falls by {percent}%"),
            Self::Unsupported { tag } => format!("Unsupported ({tag})"),
        }
    }
}

/// The on-disk shape: `{"tag":"crossesAbove","target":1.0}`, unknown tags tolerated.
#[derive(Deserialize, Serialize)]
struct RawCondition {
    tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reference: Option<f64>,
}

impl Serialize for AlertCondition {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let raw = |tag: &str, target, percent, reference| RawCondition {
            tag: tag.to_owned(),
            target,
            percent,
            reference,
        };
        match self {
            Self::CrossesAbove { target } => raw("crossesAbove", Some(*target), None, None),
            Self::CrossesBelow { target } => raw("crossesBelow", Some(*target), None, None),
            Self::RisesBy {
                percent,
                reference,
                target,
            } => raw("risesBy", Some(*target), Some(*percent), Some(*reference)),
            Self::FallsBy {
                percent,
                reference,
                target,
            } => raw("fallsBy", Some(*target), Some(*percent), Some(*reference)),
            Self::Unsupported { tag } => raw(tag, None, None, None),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AlertCondition {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawCondition::deserialize(deserializer)?;
        // A known tag missing a required field is as unusable as an unknown tag.
        let known = match raw.tag.as_str() {
            "crossesAbove" => raw.target.map(|target| Self::CrossesAbove { target }),
            "crossesBelow" => raw.target.map(|target| Self::CrossesBelow { target }),
            "risesBy" => match (raw.percent, raw.reference, raw.target) {
                (Some(percent), Some(reference), Some(target)) => Some(Self::RisesBy {
                    percent,
                    reference,
                    target,
                }),
                _ => None,
            },
            "fallsBy" => match (raw.percent, raw.reference, raw.target) {
                (Some(percent), Some(reference), Some(target)) => Some(Self::FallsBy {
                    percent,
                    reference,
                    target,
                }),
                _ => None,
            },
            _ => None,
        };
        Ok(known.unwrap_or(Self::Unsupported { tag: raw.tag }))
    }
}

/// The fiat currencies an alert can state its price in.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum AlertCurrency {
    #[default]
    Usd,
    Eur,
    Gbp,
    Jpy,
    Chf,
}

impl AlertCurrency {
    pub const ALL: [Self; 5] = [Self::Usd, Self::Eur, Self::Gbp, Self::Jpy, Self::Chf];

    pub fn code(self) -> &'static str {
        match self {
            Self::Usd => "USD",
            Self::Eur => "EUR",
            Self::Gbp => "GBP",
            Self::Jpy => "JPY",
            Self::Chf => "CHF",
        }
    }

    /// The sign written before a typed price: "$", "€", "CHF".
    pub fn glyph(self) -> &'static str {
        self.symbol().trim_end()
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Usd => "US Dollar",
            Self::Eur => "Euro",
            Self::Gbp => "British Pound",
            Self::Jpy => "Japanese Yen",
            Self::Chf => "Swiss Franc",
        }
    }

    fn symbol(self) -> &'static str {
        match self {
            Self::Usd => "$",
            Self::Eur => "€",
            Self::Gbp => "£",
            Self::Jpy => "¥",
            Self::Chf => "CHF ",
        }
    }

    /// A price as an alert states it: `$67,432.19`, `$1.2346`, `$0.00000278`. A converted
    /// quote carries a long fractional tail and a micro-cap token needs its leading zeros
    /// plus a few digits to mean anything, so precision follows magnitude: two decimals from
    /// 1,000 up, two to four from one unit, and below one four significant figures
    /// (never fewer than two decimals). Yen has no fractional unit from one up.
    pub fn format_price(self, value: f64) -> String {
        let text = self.digits_text(value.abs());
        let (whole, fraction) = match text.split_once('.') {
            Some((whole, fraction)) => (whole.to_owned(), format!(".{fraction}")),
            None => (text, String::new()),
        };
        let sign = if value < 0.0 { "-" } else { "" };
        format!("{sign}{}{}{fraction}", self.symbol(), group(&whole))
    }

    /// The same digits as `format_price`, with no symbol and no grouping: a plain number
    /// a text field can hold and parse back (`70000.50`, `0.00000278`).
    pub fn plain_price_text(self, value: f64) -> String {
        let sign = if value < 0.0 { "-" } else { "" };
        format!("{sign}{}", self.digits_text(value.abs()))
    }

    fn digits_text(self, magnitude: f64) -> String {
        let (min, max) = fraction_digits(magnitude, self == Self::Jpy);
        let mut text = format!("{magnitude:.max$}");
        // Trim trailing zeros down to the minimum digit count.
        if let Some(dot) = text.find('.') {
            let keep = dot + 1 + min;
            while text.len() > keep && text.ends_with('0') {
                text.pop();
            }
            if text.ends_with('.') {
                text.pop();
            }
        }
        text
    }
}

const MAX_PRICE_DECIMALS: usize = 12;

/// The `(minimum, maximum)` fraction digits for a price of this size.
fn fraction_digits(magnitude: f64, whole_units: bool) -> (usize, usize) {
    if whole_units && magnitude >= 1.0 {
        return (0, 0);
    }
    let floor = if whole_units { 0 } else { 2 };
    if magnitude >= 1_000.0 {
        return (floor.max(2), floor.max(2));
    }
    if magnitude >= 1.0 {
        return (floor, floor.max(4));
    }
    if magnitude == 0.0 {
        return (floor, floor);
    }
    let mut leading_zeros = 0;
    let mut scaled = magnitude;
    while scaled < 0.1 && leading_zeros < MAX_PRICE_DECIMALS {
        scaled *= 10.0;
        leading_zeros += 1;
    }
    (floor, floor.max(MAX_PRICE_DECIMALS.min(leading_zeros + 4)))
}

fn group(digits: &str) -> String {
    let mut output = String::new();
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(character);
    }
    output
}

/// The market an alert watches: identity plus the labels a row shows.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AlertAsset {
    pub instrument: InstrumentId,
    /// "BTC", "AAPL".
    pub symbol: String,
    /// "Bitcoin".
    pub name: String,
}

impl AlertAsset {
    pub fn new(
        instrument: InstrumentId,
        symbol: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            instrument,
            symbol: symbol.into(),
            name: name.into(),
        }
    }

    /// Source-qualified identity, the key quotes are matched on.
    pub fn key(&self) -> String {
        self.instrument.key()
    }

    pub fn source(&self) -> DataSource {
        self.instrument.source
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PriceAlert {
    pub id: AlertId,
    pub asset: AlertAsset,
    pub condition: AlertCondition,
    #[serde(default)]
    pub currency: AlertCurrency,
    #[serde(default)]
    pub frequency: AlertFrequency,
    #[serde(default)]
    pub state: AlertState,
    #[serde(default)]
    pub note: String,
    /// Whether the price is on the far side of the target, so the next cross counts.
    #[serde(default = "yes")]
    pub armed: bool,
    #[serde(default)]
    pub previous_value: Option<f64>,
    #[serde(default)]
    pub last_fingerprint: Option<String>,
    #[serde(default)]
    pub created_at_ms: i64,
    #[serde(default)]
    pub updated_at_ms: i64,
    /// Webhook endpoints notified when this alert triggers. Empty leaves delivery as it
    /// was before webhooks existed.
    #[serde(default)]
    pub webhook_endpoint_ids: Vec<u64>,
    /// The webhook body with `{{placeholders}}`; `None` sends the default template.
    #[serde(default)]
    pub webhook_message: Option<String>,
}

fn yes() -> bool {
    true
}

impl PriceAlert {
    pub fn new(asset: AlertAsset, condition: AlertCondition, now_ms: i64) -> Self {
        Self {
            id: new_id(),
            asset,
            condition,
            currency: AlertCurrency::Usd,
            frequency: AlertFrequency::Once,
            state: AlertState::Active,
            note: String::new(),
            armed: true,
            previous_value: None,
            last_fingerprint: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            webhook_endpoint_ids: Vec::new(),
            webhook_message: None,
        }
    }

    /// Whether `other` is this alert as far as a planned edit is concerned: the same market,
    /// condition, currency, frequency, state, note and webhook settings. What the runtime keeps
    /// moving on its own (armed flag, last price, fingerprint, timestamps) is not compared, so
    /// a quote tick does not invalidate a plan.
    pub fn same_definition(&self, other: &Self) -> bool {
        self.id == other.id
            && self.asset.key() == other.asset.key()
            && self.condition == other.condition
            && self.currency == other.currency
            && self.frequency == other.frequency
            && self.state == other.state
            && self.note == other.note
            && self.webhook_endpoint_ids == other.webhook_endpoint_ids
            && self.webhook_message == other.webhook_message
    }
}

// MARK: Events

/// The candle a price quote was read from. Webhook placeholders (`{{open}}`,
/// `{{volume}}`, …) come from here, in the asset's own quote currency.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AlertCandleSnapshot {
    pub open_time_ms: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    /// The candle size the quote was polled at, in DegenView's names (`1m`, `1h`).
    pub interval: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum AlertEventOrigin {
    #[default]
    Live,
    CatchUp,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum AlertDeliveryState {
    #[default]
    Pending,
    Delivered,
    Suppressed,
    Failed,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct AlertDeliveryRecord {
    pub state: AlertDeliveryState,
    #[serde(default)]
    pub attempt_count: u32,
    #[serde(default)]
    pub last_attempt_at_ms: Option<i64>,
    #[serde(default)]
    pub next_retry_at_ms: Option<i64>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AlertTriggerEvent {
    pub id: u64,
    pub alert_id: AlertId,
    pub asset: AlertAsset,
    pub observed_value: f64,
    pub target: f64,
    pub currency: AlertCurrency,
    pub timestamp_ms: i64,
    pub quote_fingerprint: String,
    #[serde(default)]
    pub origin: AlertEventOrigin,
    #[serde(default)]
    pub delivery: AlertDeliveryRecord,
    /// The candle the quote came from; `None` for catch-up events.
    #[serde(default)]
    pub candle: Option<AlertCandleSnapshot>,
}

/// A fresh price for one market, as the quote layer hands it to the engine.
#[derive(Clone, Debug, PartialEq)]
pub struct MarketQuote {
    pub asset: AlertAsset,
    pub price: f64,
    pub currency: AlertCurrency,
    pub source_timestamp_ms: i64,
    pub received_at_ms: i64,
    pub maximum_age_secs: f64,
    pub fingerprint: String,
    pub candle: Option<AlertCandleSnapshot>,
}

impl MarketQuote {
    /// A quote older than its window must not trigger an alert.
    pub fn is_fresh(&self) -> bool {
        (self.received_at_ms - self.source_timestamp_ms) as f64 / 1_000.0 <= self.maximum_age_secs
    }
}

/// Something that happened to an alert, published the moment the app learns of it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlertDomainEvent {
    /// A price alert fired: the id of its `AlertTriggerEvent`.
    PriceAlertTriggered { event_id: u64, at_ms: i64 },
    /// A Pine script alert was delivered: the id of its notification.
    ScriptAlertTriggered { event_id: u64, at_ms: i64 },
}

impl AlertDomainEvent {
    pub fn event_id(&self) -> u64 {
        match self {
            Self::PriceAlertTriggered { event_id, .. }
            | Self::ScriptAlertTriggered { event_id, .. } => *event_id,
        }
    }

    pub fn occurred_at_ms(&self) -> i64 {
        match self {
            Self::PriceAlertTriggered { at_ms, .. } | Self::ScriptAlertTriggered { at_ms, .. } => {
                *at_ms
            }
        }
    }
}

// MARK: Settings, health, snapshot

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct AlertNotificationSettings {
    pub delivery_enabled: bool,
    pub sound_enabled: bool,
    pub in_app_banners_enabled: bool,
    pub macos_notifications_enabled: bool,
}

impl Default for AlertNotificationSettings {
    fn default() -> Self {
        Self {
            delivery_enabled: true,
            sound_enabled: true,
            in_app_banners_enabled: true,
            macos_notifications_enabled: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum AlertRuntimeOwner {
    #[default]
    None,
    App,
    Agent,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum AlertServiceState {
    #[default]
    Unavailable,
    Disabled,
    Enabled,
    RequiresApproval,
    NotFound,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum AlertNotificationPermission {
    #[default]
    Unknown,
    NotDetermined,
    Denied,
    Authorized,
    Provisional,
    Ephemeral,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AlertProviderHealth {
    pub source: DataSource,
    #[serde(default)]
    pub last_successful_quote_ms: Option<i64>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub retry_at_ms: Option<i64>,
    #[serde(default)]
    pub has_data_gap: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct AlertRuntimeHealth {
    pub owner: AlertRuntimeOwner,
    pub service_state: AlertServiceState,
    pub heartbeat_ms: Option<i64>,
    pub notification_permission: AlertNotificationPermission,
    pub providers: Vec<AlertProviderHealth>,
    pub unreconciled_gaps: Vec<String>,
}

/// Everything the alert runtime persists, written whole with a rising revision.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct AlertPersistenceSnapshot {
    pub revision: u64,
    pub alerts: Vec<PriceAlert>,
    pub history: Vec<AlertTriggerEvent>,
    pub settings: AlertNotificationSettings,
    pub processed_command_ids: Vec<u64>,
    /// Commands whose precondition no longer held when they were applied, newest last. Their
    /// ids are also in `processed_command_ids`; this says why nothing happened.
    pub rejected_commands: Vec<AlertCommandRejection>,
    pub health: AlertRuntimeHealth,
}

/// What must still be true when the runtime applies a command. The planner checked it against a
/// snapshot that can be a second old (or older, when the background agent owns the runtime), and
/// several requests plan against the same snapshot, so the owner of the data checks again.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum AlertPrecondition {
    /// A `Save` that adds a new alert: its id is unused, there are fewer than `max_alerts`, and
    /// no other alert has the same market, condition, currency and frequency.
    Create { max_alerts: usize },
    /// A `Save` that edits an alert: it still exists and still has the definition the edit was
    /// planned from ([`PriceAlert::same_definition`]). Prevents lost updates and bringing a
    /// deleted alert back.
    Unchanged(Box<PriceAlert>),
}

/// Why a command was not applied.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum AlertRejectReason {
    /// An identical alert exists (or an alert with this id).
    Duplicate,
    LimitReached,
    /// The alert was deleted since the edit was planned.
    Missing,
    /// The alert was changed since the edit was planned.
    Changed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AlertCommandRejection {
    pub command_id: u64,
    pub reason: AlertRejectReason,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum AlertRuntimeCommandPayload {
    Save {
        alert: Box<PriceAlert>,
        baseline: Option<f64>,
    },
    SetState {
        id: AlertId,
        state: AlertState,
        baseline: Option<f64>,
    },
    Delete(AlertId),
    ClearHistory,
    UpdateSettings(AlertNotificationSettings),
    RequestNotificationAuthorization,
}

/// A change requested by the UI and applied exactly once by whoever owns the runtime
/// (the app, or the background agent), identified so a replay is ignored.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AlertRuntimeCommand {
    pub id: u64,
    pub created_at_ms: i64,
    /// Checked by the runtime right before the payload is applied; `None` always applies.
    #[serde(default)]
    pub precondition: Option<AlertPrecondition>,
    pub payload: AlertRuntimeCommandPayload,
}

impl AlertRuntimeCommand {
    pub fn new(payload: AlertRuntimeCommandPayload, now_ms: i64) -> Self {
        Self {
            id: new_id(),
            created_at_ms: now_ms,
            precondition: None,
            payload,
        }
    }

    pub fn with_precondition(mut self, precondition: AlertPrecondition) -> Self {
        self.precondition = Some(precondition);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset() -> AlertAsset {
        AlertAsset::new(
            InstrumentId::new(DataSource::Binance, "btcusdt"),
            "BTC",
            "Bitcoin",
        )
    }

    #[test]
    fn conditions_round_trip_and_unknown_tags_survive() {
        let conditions = [
            AlertCondition::CrossesAbove { target: 100.0 },
            AlertCondition::CrossesBelow { target: 90.5 },
            // Binary-exact targets: serde_json's default float parse can be an ulp off.
            AlertCondition::rises_by(50.0, 100.0),
            AlertCondition::falls_by(25.0, 200.0),
        ];
        for condition in conditions {
            let json = serde_json::to_string(&condition).unwrap();
            assert_eq!(
                serde_json::from_str::<AlertCondition>(&json).unwrap(),
                condition
            );
        }
        let future: AlertCondition =
            serde_json::from_str(r#"{"tag":"touchesVwap","target":1}"#).unwrap();
        assert_eq!(
            future,
            AlertCondition::Unsupported {
                tag: "touchesVwap".into()
            }
        );
        assert_eq!(future.target(), None);
        // The tag is preserved on the way back out.
        assert_eq!(
            serde_json::to_string(&future).unwrap(),
            r#"{"tag":"touchesVwap"}"#
        );
        // A known tag missing its fields is unusable, not a decode failure.
        let broken: AlertCondition =
            serde_json::from_str(r#"{"tag":"risesBy","target":1}"#).unwrap();
        assert_eq!(
            broken,
            AlertCondition::Unsupported {
                tag: "risesBy".into()
            }
        );
    }

    #[test]
    fn percent_conditions_derive_their_target() {
        let up = AlertCondition::rises_by(10.0, 100.0);
        assert!((up.target().unwrap() - 110.0).abs() < 1e-9);
        assert!(up.crosses_above());
        let down = AlertCondition::falls_by(25.0, 80.0);
        assert!((down.target().unwrap() - 60.0).abs() < 1e-9);
        assert!(!down.crosses_above());
    }

    #[test]
    fn armed_means_the_price_still_has_to_cross() {
        let above = AlertCondition::CrossesAbove { target: 100.0 };
        assert_eq!(above.is_armed_at(99.0), Some(true));
        assert_eq!(
            above.is_armed_at(100.0),
            Some(false),
            "standing on the target is already crossed"
        );
        let below = AlertCondition::CrossesBelow { target: 100.0 };
        assert_eq!(below.is_armed_at(101.0), Some(true));
        assert_eq!(below.is_armed_at(100.0), Some(false));
        assert_eq!(
            AlertCondition::Unsupported { tag: "x".into() }.is_armed_at(1.0),
            None
        );
    }

    #[test]
    fn old_alert_rows_decode_with_defaults() {
        let json = r#"{
            "id": 7,
            "asset": {"instrument":{"source":"binance","symbol":"BTCUSDT"},"symbol":"BTC","name":"Bitcoin"},
            "condition": {"tag":"crossesAbove","target":70000}
        }"#;
        let alert: PriceAlert = serde_json::from_str(json).unwrap();
        assert_eq!(alert.id, 7);
        assert!(alert.armed, "a missing flag arms the alert");
        assert_eq!(alert.state, AlertState::Active);
        assert_eq!(alert.frequency, AlertFrequency::Once);
        assert_eq!(alert.currency, AlertCurrency::Usd);
        assert!(alert.webhook_endpoint_ids.is_empty());
        assert_eq!(alert.webhook_message, None);
    }

    #[test]
    fn snapshots_round_trip_and_tolerate_missing_sections() {
        let mut snapshot = AlertPersistenceSnapshot::default();
        snapshot.alerts.push(PriceAlert::new(
            asset(),
            AlertCondition::CrossesAbove { target: 1.0 },
            5,
        ));
        snapshot.revision = 3;
        let json = serde_json::to_string(&snapshot).unwrap();
        assert_eq!(
            serde_json::from_str::<AlertPersistenceSnapshot>(&json).unwrap(),
            snapshot
        );
        let empty: AlertPersistenceSnapshot = serde_json::from_str("{}").unwrap();
        assert!(empty.settings.delivery_enabled);
        assert_eq!(empty.revision, 0);
    }

    #[test]
    fn quotes_go_stale_after_their_window() {
        let quote = |received: i64| MarketQuote {
            asset: asset(),
            price: 1.0,
            currency: AlertCurrency::Usd,
            source_timestamp_ms: 1_000_000,
            received_at_ms: received,
            maximum_age_secs: 30.0,
            fingerprint: "f".into(),
            candle: None,
        };
        assert!(quote(1_030_000).is_fresh());
        assert!(!quote(1_030_001).is_fresh());
    }

    #[test]
    fn domain_events_expose_their_id_and_time() {
        let price = AlertDomainEvent::PriceAlertTriggered {
            event_id: 4,
            at_ms: 9,
        };
        let script = AlertDomainEvent::ScriptAlertTriggered {
            event_id: 5,
            at_ms: 11,
        };
        assert_eq!((price.event_id(), price.occurred_at_ms()), (4, 9));
        assert_eq!((script.event_id(), script.occurred_at_ms()), (5, 11));
    }

    #[test]
    fn prices_read_naturally_at_every_magnitude() {
        let usd = AlertCurrency::Usd;
        assert_eq!(usd.format_price(67_432.19), "$67,432.19");
        assert_eq!(usd.format_price(67_432.199), "$67,432.20");
        assert_eq!(usd.format_price(1.234_567), "$1.2346");
        assert_eq!(usd.format_price(1.5), "$1.50");
        assert_eq!(usd.format_price(0.000_002_78), "$0.00000278");
        assert_eq!(usd.format_price(0.0), "$0.00");
        assert_eq!(usd.format_price(1_234_567.0), "$1,234,567.00");
        assert_eq!(usd.format_price(-12.5), "-$12.50");
    }

    #[test]
    fn plain_prices_parse_back() {
        assert_eq!(AlertCurrency::Usd.plain_price_text(67_432.19), "67432.19");
        assert_eq!(
            AlertCurrency::Usd.plain_price_text(0.000_002_78),
            "0.00000278"
        );
        assert_eq!(AlertCurrency::Jpy.plain_price_text(10_234.6), "10235");
        assert_eq!(AlertCurrency::Usd.plain_price_text(-3.5), "-3.50");
        for value in [67_432.19, 1.2346, 0.000_002_78] {
            let text = AlertCurrency::Usd.plain_price_text(value);
            assert!((text.parse::<f64>().unwrap() - value).abs() < value * 1e-3);
        }
    }

    #[test]
    fn yen_has_no_fraction_from_one_up_and_other_symbols_prefix() {
        assert_eq!(AlertCurrency::Jpy.format_price(10_234.6), "¥10,235");
        assert_eq!(AlertCurrency::Jpy.format_price(0.5), "¥0.5");
        assert_eq!(AlertCurrency::Eur.format_price(2.0), "€2.00");
        assert_eq!(AlertCurrency::Gbp.format_price(3.14161), "£3.1416");
        assert_eq!(AlertCurrency::Chf.format_price(1_000.0), "CHF 1,000.00");
        assert_eq!(AlertCurrency::Usd.code(), "USD");
    }

    #[test]
    fn frequency_titles_match_the_editor() {
        assert_eq!(AlertFrequency::Once.title(), "Once");
        assert_eq!(AlertFrequency::EveryTime.title(), "Every Time");
        assert_eq!(
            AlertCondition::rises_by(10.0, 1.0).summary(),
            "Rises by 10%"
        );
    }
}
