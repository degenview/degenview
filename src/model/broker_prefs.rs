//! How the user has chosen to trade at each broker. Local to this Mac (`broker.prefs` is never
//! synced): another Mac's confirmation habits must not decide what this one asks.
//!
//! Safety first: the defaults ask. Turning a confirmation off is allowed for resting orders and
//! single cancels only; a market order, an edit and "cancel all" always ask (see
//! [`crate::model::broker_ticket::TicketDraft::needs_confirmation`]), whatever is stored here.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const KEY: &str = "broker.prefs";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BrokerPrefs {
    /// Ask before sending a limit or stop-limit order.
    pub confirm_resting_orders: bool,
    /// Ask before cancelling one order.
    pub confirm_single_cancel: bool,
    /// Tell the user when an order is placed, filled, cancelled or rejected.
    pub notify: bool,
    /// Draw this broker's working orders on the chart.
    pub show_on_charts: bool,
    /// A market buy starts out stating an amount to spend rather than a size.
    pub spend_in_quote_by_default: bool,
}

impl Default for BrokerPrefs {
    fn default() -> Self {
        Self {
            confirm_resting_orders: true,
            confirm_single_cancel: false,
            notify: true,
            show_on_charts: true,
            spend_in_quote_by_default: false,
        }
    }
}

/// Every broker's choices, by broker id.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AllPrefs(BTreeMap<String, BrokerPrefs>);

impl AllPrefs {
    /// A broker with no saved choices has the defaults.
    pub fn of(&self, broker: &str) -> BrokerPrefs {
        self.0.get(broker).cloned().unwrap_or_default()
    }

    pub fn set(&mut self, broker: &str, prefs: BrokerPrefs) {
        self.0.insert(broker.to_owned(), prefs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_ask_and_tell() {
        let prefs = BrokerPrefs::default();
        assert!(prefs.confirm_resting_orders);
        assert!(prefs.notify && prefs.show_on_charts);
        assert!(
            !prefs.confirm_single_cancel,
            "a single cancel is quick and reversible"
        );
        assert!(!prefs.spend_in_quote_by_default);
    }

    #[test]
    fn missing_fields_take_the_safe_default_and_unknown_ones_are_ignored() {
        let all: AllPrefs =
            serde_json::from_str(r#"{"acme":{"notify":false,"future":1}}"#).unwrap();
        let acme = all.of("acme");
        assert!(!acme.notify);
        assert!(
            acme.confirm_resting_orders,
            "a field this version did not store asks"
        );
        assert_eq!(all.of("other"), BrokerPrefs::default());
    }

    #[test]
    fn choices_round_trip_per_broker() {
        let mut all = AllPrefs::default();
        all.set(
            "acme",
            BrokerPrefs {
                confirm_resting_orders: false,
                ..BrokerPrefs::default()
            },
        );
        let again: AllPrefs = serde_json::from_str(&serde_json::to_string(&all).unwrap()).unwrap();
        assert!(!again.of("acme").confirm_resting_orders);
        assert!(again.of("other").confirm_resting_orders);
    }
}
