//! A user's request to be notified when one chart's applied Pine script raises `alert()` events,
//! and the rules for when such a request may be created. A port of `PineAlertSubscription` and
//! the blocker logic of `PineAlertEditor`.
//!
//! A subscription is bound to the chart, symbol, timeframe and the exact source that was applied,
//! so a different script or a chart showing another market never fires it by accident.

use crate::model::pine::instance::StoredInput;
use crate::pine::builtins::alert::call::AlertCallSite;
use crate::pine::builtins::alert::frequency::AlertFrequency;
use crate::pine::compiler::program::InputValue;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Where a subscription stands.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PineAlertState {
    #[default]
    Active,
    /// Switched off by the user.
    Paused,
    /// The chart's script no longer matches `source_hash`. Silent until the user re-arms.
    ScriptChanged,
    /// The saved script it was created from no longer exists.
    ScriptDeleted,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PineAlertSubscription {
    pub id: u64,
    pub chart_id: u64,
    /// The applied indicator this alert watches. Two instances of one script, with different
    /// inputs, are separate alerts.
    #[serde(default)]
    pub instance_id: Option<u64>,
    #[serde(default)]
    pub script_id: Option<String>,
    pub script_name: String,
    /// The script source the subscription runs. Stored with the row so the runtime host can run it
    /// without a separate script store. Empty for rows saved before sources were persisted.
    #[serde(default)]
    pub source: String,
    /// Input overrides captured from the applied instance when the alert was armed.
    #[serde(default)]
    pub inputs: HashMap<String, StoredInput>,
    /// `"<source>:<ticker>"`, matching the dataset's symbol key.
    pub symbol_key: String,
    pub timeframe: String,
    pub source_hash: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub state: PineAlertState,
    pub created_at_ms: i64,
    /// Webhook endpoints this alert also posts to. Ids, never URLs, so renaming or disabling an
    /// endpoint needs no change here. Alerts saved before webhooks existed have none.
    #[serde(default)]
    pub webhook_endpoint_ids: Vec<u64>,
}

impl PineAlertSubscription {
    pub fn is_active(&self) -> bool {
        self.state == PineAlertState::Active
    }

    /// Whether this is the market the subscription was created for.
    pub fn watches(&self, symbol_key: &str, timeframe: &str) -> bool {
        self.symbol_key == symbol_key && self.timeframe == timeframe
    }

    pub fn input_values(&self) -> HashMap<String, InputValue> {
        self.inputs
            .iter()
            .map(|(id, value)| (id.clone(), value.clone().into()))
            .collect()
    }
}

/// How a call's frequency reads in the UI, rather than its Pine constant.
pub fn frequency_name(frequency: AlertFrequency) -> &'static str {
    match frequency {
        AlertFrequency::All => "Every update",
        AlertFrequency::OncePerBar => "Once per bar",
        AlertFrequency::OncePerBarClose => "Once per bar close",
    }
}

/// What one `alert()` / `alertcondition()` line says in the sheet's list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallRow {
    /// The title, else the message, else a note that the script sets it.
    pub headline: String,
    /// The message under a title.
    pub detail: Option<String>,
    pub frequency: &'static str,
    pub line: usize,
    /// "alertcondition()" or "alert()".
    pub help: &'static str,
    pub is_condition: bool,
}

pub fn call_row(site: &AlertCallSite) -> CallRow {
    CallRow {
        headline: site
            .title
            .clone()
            .or_else(|| site.message.clone())
            .unwrap_or_else(|| "Message set by the script".to_owned()),
        detail: site.title.as_ref().and(site.message.clone()),
        frequency: site.frequency.map_or("Set by script", frequency_name),
        line: site.range.start.line,
        help: if site.is_condition {
            "alertcondition()"
        } else {
            "alert()"
        },
        is_condition: site.is_condition,
    }
}

/// Why a script alert cannot be created yet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Blocker {
    pub title: &'static str,
    pub detail: &'static str,
}

/// What the sheet knows about the chart and the applied indicator.
#[derive(Clone, Copy, Debug)]
pub struct ArmingFacts {
    /// False until the applied script has been compiled; nothing is blocked while it is unknown.
    pub loaded: bool,
    /// The indicator is still applied to the chart.
    pub instance_exists: bool,
    /// The indicator's script has been loaded on the chart (its source hash is known).
    pub source_loaded: bool,
    pub call_sites: usize,
    /// An active alert on this chart already watches this indicator and market.
    pub duplicate_exists: bool,
}

pub fn blocker(facts: ArmingFacts) -> Option<Blocker> {
    if !facts.loaded {
        return None;
    }
    if !facts.instance_exists {
        return Some(Blocker {
            title: "Indicator removed",
            detail: "This indicator is no longer applied to the chart.",
        });
    }
    if !facts.source_loaded {
        return Some(Blocker {
            title: "Script not loaded",
            detail: "The indicator's script hasn't loaded yet. Try again in a moment.",
        });
    }
    if facts.call_sites == 0 {
        return Some(Blocker {
            title: "No alerts in this script",
            detail: "The script has no alert() or alertcondition() calls.",
        });
    }
    if facts.duplicate_exists {
        return Some(Blocker {
            title: "Already armed",
            detail: "This indicator already has an active alert on this chart.",
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::syntax::token::{SourcePosition, SourceRange};

    fn site(is_condition: bool, title: Option<&str>, message: Option<&str>) -> AlertCallSite {
        AlertCallSite {
            frequency: Some(AlertFrequency::OncePerBar),
            is_condition,
            range: SourceRange::new(SourcePosition::new(7, 1, 0), SourcePosition::new(7, 9, 8)),
            title: title.map(str::to_owned),
            message: message.map(str::to_owned),
        }
    }

    fn subscription() -> PineAlertSubscription {
        PineAlertSubscription {
            id: 1,
            chart_id: 2,
            instance_id: Some(3),
            script_id: None,
            script_name: "RSI".into(),
            source: String::new(),
            inputs: HashMap::new(),
            symbol_key: "BINANCE:BTCUSDT".into(),
            timeframe: "1h".into(),
            source_hash: "abc".into(),
            note: String::new(),
            state: PineAlertState::Active,
            created_at_ms: 0,
            webhook_endpoint_ids: vec![9],
        }
    }

    fn facts() -> ArmingFacts {
        ArmingFacts {
            loaded: true,
            instance_exists: true,
            source_loaded: true,
            call_sites: 2,
            duplicate_exists: false,
        }
    }

    #[test]
    fn a_subscription_watches_one_market_and_timeframe() {
        let alert = subscription();
        assert!(alert.is_active());
        assert!(alert.watches("BINANCE:BTCUSDT", "1h"));
        assert!(!alert.watches("BINANCE:BTCUSDT", "4h"));
        assert!(!alert.watches("COINBASE:BTC-USD", "1h"));
        let paused = PineAlertSubscription {
            state: PineAlertState::Paused,
            ..alert
        };
        assert!(!paused.is_active());
    }

    #[test]
    fn old_rows_without_webhooks_still_decode() {
        let json = r#"{"id":1,"chart_id":2,"script_name":"RSI","symbol_key":"BINANCE:BTCUSDT",
            "timeframe":"1h","source_hash":"abc","created_at_ms":5}"#;
        let decoded: PineAlertSubscription = serde_json::from_str(json).unwrap();
        assert!(decoded.webhook_endpoint_ids.is_empty());
        assert_eq!(decoded.state, PineAlertState::Active);
        assert_eq!(decoded.note, "");
        assert!(decoded.inputs.is_empty());
        let round_trip: PineAlertSubscription =
            serde_json::from_str(&serde_json::to_string(&subscription()).unwrap()).unwrap();
        assert_eq!(round_trip, subscription());
        assert_eq!(
            serde_json::to_string(&PineAlertState::ScriptChanged).unwrap(),
            "\"scriptChanged\""
        );
    }

    #[test]
    fn frequencies_read_in_words() {
        assert_eq!(frequency_name(AlertFrequency::All), "Every update");
        assert_eq!(frequency_name(AlertFrequency::OncePerBar), "Once per bar");
        assert_eq!(
            frequency_name(AlertFrequency::OncePerBarClose),
            "Once per bar close"
        );
    }

    #[test]
    fn call_rows_prefer_the_title_and_say_when_the_script_decides() {
        let row = call_row(&site(true, Some("Cross up"), Some("RSI crossed")));
        assert_eq!(row.headline, "Cross up");
        assert_eq!(row.detail.as_deref(), Some("RSI crossed"));
        assert_eq!(
            (row.line, row.help, row.is_condition),
            (7, "alertcondition()", true)
        );
        assert_eq!(row.frequency, "Once per bar");
        let message_only = call_row(&site(false, None, Some("Buy now")));
        assert_eq!(
            (message_only.headline.as_str(), message_only.detail),
            ("Buy now", None)
        );
        assert_eq!(message_only.help, "alert()");
        let computed = call_row(&AlertCallSite {
            frequency: None,
            ..site(false, None, None)
        });
        assert_eq!(computed.headline, "Message set by the script");
        assert_eq!(computed.frequency, "Set by script");
    }

    #[test]
    fn nothing_blocks_until_the_script_is_known_then_the_first_problem_wins() {
        assert_eq!(
            blocker(ArmingFacts {
                loaded: false,
                instance_exists: false,
                ..facts()
            }),
            None
        );
        assert_eq!(blocker(facts()), None);
        let removed = blocker(ArmingFacts {
            instance_exists: false,
            source_loaded: false,
            ..facts()
        });
        assert_eq!(removed.unwrap().title, "Indicator removed");
        assert_eq!(
            blocker(ArmingFacts {
                source_loaded: false,
                ..facts()
            })
            .unwrap()
            .title,
            "Script not loaded"
        );
        let empty = blocker(ArmingFacts {
            call_sites: 0,
            duplicate_exists: true,
            ..facts()
        });
        assert_eq!(empty.unwrap().title, "No alerts in this script");
        let duplicate = blocker(ArmingFacts {
            duplicate_exists: true,
            ..facts()
        })
        .unwrap();
        assert_eq!(duplicate.title, "Already armed");
        assert!(duplicate.detail.contains("already has an active alert"));
    }
}
