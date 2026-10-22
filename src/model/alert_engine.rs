#![allow(dead_code)]
//! Price-alert evaluation: a state machine over `AlertPersistenceSnapshot`, free of
//! storage, threads and clocks (callers pass `now_ms`) so every rule is testable.
//!
//! An alert fires when the price *crosses* its target — from the far side to the near
//! side — not when it merely sits past it. After firing, an every-time alert re-arms
//! only once the price has gone back across; a once alert is spent. A quote is judged
//! once per alert (by fingerprint), so a poll that returns the same candle twice is not
//! two chances to fire.

use crate::model::alert::AlertCondition;
use crate::model::alert::{
    AlertAsset, AlertCurrency, AlertDeliveryRecord, AlertEventOrigin, AlertFrequency, AlertId,
    AlertNotificationSettings, AlertPersistenceSnapshot, AlertRuntimeCommand,
    AlertRuntimeCommandPayload, AlertRuntimeHealth, AlertState, AlertTriggerEvent, MarketQuote,
    PriceAlert,
};
use crate::model::watchlist::new_id;
use std::collections::{BTreeSet, HashMap, HashSet};

/// The most processed command ids remembered, to ignore a replay.
const PROCESSED_COMMAND_LIMIT: usize = 1_000;
/// Catch-up never invents crossings older than this.
const CATCH_UP_WINDOW_MS: i64 = 86_400_000;

pub struct AlertEngine {
    snapshot: AlertPersistenceSnapshot,
    /// Active alerts by asset key.
    index: HashMap<String, BTreeSet<AlertId>>,
    /// Alerts that stay defined here but are not evaluated on this Mac (synced from another Mac
    /// and not switched on here), so one alert never notifies from two Macs.
    muted: HashSet<AlertId>,
}

impl AlertEngine {
    pub fn new(snapshot: Option<AlertPersistenceSnapshot>) -> Self {
        let mut engine = Self {
            snapshot: snapshot.unwrap_or_default(),
            index: HashMap::new(),
            muted: HashSet::new(),
        };
        engine.rebuild_index();
        engine
    }

    pub fn snapshot(&self) -> &AlertPersistenceSnapshot {
        &self.snapshot
    }

    /// Every store write bumps the revision, so a reader can tell it is looking at news.
    fn touch(&mut self) {
        self.snapshot.revision = self.snapshot.revision.wrapping_add(1);
    }

    /// Whether this Mac judges the alert against quotes.
    pub fn evaluates(&self, alert: &PriceAlert) -> bool {
        alert.state == AlertState::Active && !self.muted.contains(&alert.id)
    }

    /// Sets the alerts this Mac must not evaluate. An alert that was muted and no longer is
    /// starts afresh: its next quote only says which side the price is on, so switching an alert
    /// on can never fire from a stale comparison. Returns whether anything changed.
    pub fn set_muted(&mut self, muted: HashSet<AlertId>) -> bool {
        if muted == self.muted {
            return false;
        }
        for alert in &mut self.snapshot.alerts {
            if self.muted.contains(&alert.id) && !muted.contains(&alert.id) {
                alert.previous_value = None;
                alert.last_fingerprint = None;
            }
        }
        self.muted = muted;
        self.rebuild_index();
        self.touch();
        true
    }

    fn rebuild_index(&mut self) {
        self.index.clear();
        for alert in &self.snapshot.alerts {
            if alert.state == AlertState::Active && !self.muted.contains(&alert.id) {
                self.index
                    .entry(alert.asset.key())
                    .or_default()
                    .insert(alert.id);
            }
        }
    }

    pub fn replace_health(&mut self, health: AlertRuntimeHealth) {
        self.snapshot.health = health;
        self.touch();
    }

    pub fn mark_delivery(&mut self, event_id: u64, record: AlertDeliveryRecord) {
        if let Some(event) = self
            .snapshot
            .history
            .iter_mut()
            .find(|event| event.id == event_id)
        {
            event.delivery = record;
            self.touch();
        }
    }

    // MARK: Commands

    /// Applies a UI request exactly once; a command id seen before is ignored.
    pub fn apply(&mut self, command: &AlertRuntimeCommand, now_ms: i64) {
        if self.snapshot.processed_command_ids.contains(&command.id) {
            return;
        }
        match &command.payload {
            AlertRuntimeCommandPayload::Save { alert, baseline } => {
                self.save_alert((**alert).clone(), *baseline, now_ms);
            }
            AlertRuntimeCommandPayload::SetState {
                id,
                state,
                baseline,
            } => self.set_state(*id, *state, *baseline, now_ms),
            AlertRuntimeCommandPayload::Delete(id) => self.delete(*id),
            AlertRuntimeCommandPayload::ClearHistory => self.clear_history(),
            AlertRuntimeCommandPayload::UpdateSettings(settings) => {
                self.update_settings(settings.clone());
            }
            AlertRuntimeCommandPayload::RequestNotificationAuthorization => {}
        }
        let ids = &mut self.snapshot.processed_command_ids;
        ids.push(command.id);
        if ids.len() > PROCESSED_COMMAND_LIMIT {
            let excess = ids.len() - PROCESSED_COMMAND_LIMIT;
            ids.drain(..excess);
        }
        self.touch();
    }

    /// Assets with an active alert, once each, for the quote layer to poll.
    pub fn active_assets(&self) -> Vec<AlertAsset> {
        let mut seen = HashSet::new();
        self.snapshot
            .alerts
            .iter()
            .filter(|alert| self.evaluates(alert))
            .filter(|alert| seen.insert(alert.asset.key()))
            .map(|alert| alert.asset.clone())
            .collect()
    }

    pub fn save_alert(&mut self, mut alert: PriceAlert, baseline: Option<f64>, now_ms: i64) {
        alert.updated_at_ms = now_ms;
        alert.previous_value = baseline;
        alert.last_fingerprint = None;
        if matches!(alert.condition, AlertCondition::Unsupported { .. }) {
            alert.state = AlertState::Unsupported;
            alert.armed = false;
        } else if let Some(armed) =
            baseline.and_then(|baseline| alert.condition.is_armed_at(baseline))
        {
            alert.armed = armed;
        }
        match self
            .snapshot
            .alerts
            .iter_mut()
            .find(|existing| existing.id == alert.id)
        {
            Some(existing) => *existing = alert,
            None => self.snapshot.alerts.push(alert),
        }
        self.rebuild_index();
        self.touch();
    }

    pub fn set_state(
        &mut self,
        id: AlertId,
        state: AlertState,
        baseline: Option<f64>,
        now_ms: i64,
    ) {
        let Some(alert) = self.snapshot.alerts.iter_mut().find(|alert| alert.id == id) else {
            return;
        };
        alert.state = state;
        alert.updated_at_ms = now_ms;
        if state == AlertState::Active {
            // Resuming judges the price afresh: the next quote must cross from where it is now.
            alert.previous_value = baseline;
            if let Some(armed) = baseline.and_then(|baseline| alert.condition.is_armed_at(baseline))
            {
                alert.armed = armed;
            }
        }
        self.rebuild_index();
        self.touch();
    }

    pub fn delete(&mut self, id: AlertId) {
        self.snapshot.alerts.retain(|alert| alert.id != id);
        self.rebuild_index();
        self.touch();
    }

    pub fn clear_history(&mut self) {
        self.snapshot.history.clear();
        self.touch();
    }

    pub fn update_settings(&mut self, settings: AlertNotificationSettings) {
        self.snapshot.settings = settings;
        self.touch();
    }

    /// An alert of the same asset, condition, currency and frequency already exists.
    pub fn is_identical(&self, candidate: &PriceAlert) -> bool {
        self.find_identical(candidate).is_some()
    }

    /// That alert, so a caller can hand it back instead of adding a duplicate.
    pub fn find_identical(&self, candidate: &PriceAlert) -> Option<&PriceAlert> {
        self.snapshot.alerts.iter().find(|alert| {
            alert.id != candidate.id
                && alert.asset.key() == candidate.asset.key()
                && alert.condition == candidate.condition
                && alert.currency == candidate.currency
                && alert.frequency == candidate.frequency
        })
    }

    // MARK: Evaluation

    /// Judges one quote against every active alert on its asset. `converted_price` and
    /// `currency` carry a quote already converted into an alert currency; alerts in a
    /// different currency are skipped.
    pub fn process(
        &mut self,
        quote: &MarketQuote,
        converted_price: Option<f64>,
        currency: Option<AlertCurrency>,
    ) -> Vec<AlertTriggerEvent> {
        if !quote.is_fresh() {
            return Vec::new();
        }
        let Some(ids) = self.index.get(&quote.asset.key()).cloned() else {
            return Vec::new();
        };
        let mut events = Vec::new();
        for id in ids {
            let Some(position) = self
                .snapshot
                .alerts
                .iter()
                .position(|alert| alert.id == id && self.evaluates(alert))
            else {
                continue;
            };
            let mut alert = self.snapshot.alerts[position].clone();
            if currency.is_some_and(|currency| alert.currency != currency) {
                continue;
            }
            let fingerprint = format!("{}:{}", quote.fingerprint, alert.currency.code());
            if alert.last_fingerprint.as_deref() == Some(fingerprint.as_str()) {
                continue;
            }
            let current = converted_price.unwrap_or(quote.price);
            let Some(target) = alert.condition.target() else {
                continue;
            };
            match alert.previous_value {
                Some(previous) => {
                    let above = alert.condition.crosses_above();
                    let crossed = if above {
                        previous < target && current >= target
                    } else {
                        previous > target && current <= target
                    };
                    let opposite = if above {
                        current < target
                    } else {
                        current > target
                    };
                    if !alert.armed && opposite {
                        alert.armed = true;
                    }
                    if alert.armed && crossed {
                        let event = AlertTriggerEvent {
                            id: new_id(),
                            alert_id: alert.id,
                            asset: alert.asset.clone(),
                            observed_value: current,
                            target,
                            currency: alert.currency,
                            timestamp_ms: quote.received_at_ms,
                            quote_fingerprint: quote.fingerprint.clone(),
                            origin: AlertEventOrigin::Live,
                            delivery: AlertDeliveryRecord::default(),
                            candle: quote.candle.clone(),
                        };
                        self.snapshot.history.push(event.clone());
                        events.push(event);
                        if alert.frequency == AlertFrequency::Once {
                            alert.state = AlertState::Triggered;
                        }
                        alert.armed = false;
                    }
                }
                // The first quote only says which side the price starts on.
                None => alert.armed = alert.condition.is_armed_at(current).unwrap_or(false),
            }
            alert.previous_value = Some(current);
            alert.last_fingerprint = Some(fingerprint);
            self.snapshot.alerts[position] = alert;
        }
        if !events.is_empty() {
            self.rebuild_index();
        }
        self.touch();
        events
    }

    /// Replays ordered quotes after wake or reconnect. A recovery intentionally emits at
    /// most one event per alert and never invents crossings beyond the 24-hour window.
    pub fn process_catch_up(
        &mut self,
        quotes: &[MarketQuote],
        now_ms: i64,
    ) -> Vec<AlertTriggerEvent> {
        let mut ordered: Vec<&MarketQuote> = quotes
            .iter()
            .filter(|quote| now_ms - quote.source_timestamp_ms <= CATCH_UP_WINDOW_MS)
            .collect();
        ordered.sort_by_key(|quote| quote.source_timestamp_ms);

        let mut emitted: HashSet<AlertId> = HashSet::new();
        let mut result = Vec::new();
        for quote in ordered {
            let replay = MarketQuote {
                asset: quote.asset.clone(),
                price: quote.price,
                currency: quote.currency,
                source_timestamp_ms: quote.source_timestamp_ms,
                received_at_ms: quote.source_timestamp_ms,
                maximum_age_secs: 86_400.0,
                fingerprint: quote.fingerprint.clone(),
                candle: None,
            };
            for event in self.process(&replay, None, None) {
                if !emitted.insert(event.alert_id) {
                    // A second crossing of the same alert is folded into the first.
                    self.snapshot
                        .history
                        .retain(|existing| existing.id != event.id);
                    continue;
                }
                if let Some(stored) = self
                    .snapshot
                    .history
                    .iter_mut()
                    .find(|existing| existing.id == event.id)
                {
                    stored.origin = AlertEventOrigin::CatchUp;
                    result.push(stored.clone());
                }
            }
        }
        self.touch();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::alert::AlertCondition;
    use crate::model::{DataSource, InstrumentId};

    const T0: i64 = 1_700_000_000_000;

    fn asset(symbol: &str) -> AlertAsset {
        AlertAsset::new(
            InstrumentId::new(DataSource::Binance, symbol),
            symbol,
            symbol,
        )
    }

    fn quote(symbol: &str, price: f64, at: i64, fingerprint: &str) -> MarketQuote {
        MarketQuote {
            asset: asset(symbol),
            price,
            currency: AlertCurrency::Usd,
            source_timestamp_ms: at,
            received_at_ms: at,
            maximum_age_secs: 60.0,
            fingerprint: fingerprint.to_owned(),
            candle: None,
        }
    }

    fn engine_with(condition: AlertCondition, frequency: AlertFrequency) -> (AlertEngine, AlertId) {
        let mut alert = PriceAlert::new(asset("BTCUSDT"), condition, T0);
        alert.frequency = frequency;
        let id = alert.id;
        let mut engine = AlertEngine::new(None);
        engine.save_alert(alert, None, T0);
        (engine, id)
    }

    fn feed(engine: &mut AlertEngine, prices: &[f64]) -> Vec<usize> {
        // Indexes of the quotes that fired.
        prices
            .iter()
            .enumerate()
            .filter_map(|(index, price)| {
                let at = T0 + index as i64 * 1_000;
                let events = engine.process(
                    &quote("BTCUSDT", *price, at, &format!("q{index}")),
                    None,
                    None,
                );
                (!events.is_empty()).then_some(index)
            })
            .collect()
    }

    #[test]
    fn crossing_above_fires_once_and_spends_a_once_alert() {
        let (mut engine, id) = engine_with(
            AlertCondition::CrossesAbove { target: 100.0 },
            AlertFrequency::Once,
        );
        assert_eq!(
            feed(&mut engine, &[90.0, 95.0, 101.0, 102.0, 90.0, 105.0]),
            vec![2]
        );
        let alert = &engine.snapshot().alerts[0];
        assert_eq!(alert.id, id);
        assert_eq!(alert.state, AlertState::Triggered);
        assert!(!alert.armed);
        assert_eq!(engine.snapshot().history.len(), 1);
        let event = &engine.snapshot().history[0];
        assert_eq!((event.observed_value, event.target), (101.0, 100.0));
        assert_eq!(event.origin, AlertEventOrigin::Live);
        assert!(
            engine.active_assets().is_empty(),
            "a spent alert is no longer polled"
        );
    }

    #[test]
    fn the_first_quote_only_establishes_the_side() {
        // Already above the target on the first quote: not a crossing.
        let (mut engine, _) = engine_with(
            AlertCondition::CrossesAbove { target: 100.0 },
            AlertFrequency::Once,
        );
        assert_eq!(feed(&mut engine, &[150.0, 160.0]), Vec::<usize>::new());
        assert!(!engine.snapshot().alerts[0].armed);
        // It re-arms once the price drops back below, then fires on the next cross.
        assert_eq!(feed(&mut engine, &[90.0]), Vec::<usize>::new());
        assert!(engine.snapshot().alerts[0].armed);
    }

    #[test]
    fn crossing_below_mirrors_crossing_above() {
        let (mut engine, _) = engine_with(
            AlertCondition::CrossesBelow { target: 100.0 },
            AlertFrequency::Once,
        );
        assert_eq!(feed(&mut engine, &[110.0, 105.0, 99.0]), vec![2]);
        // Touching the target exactly counts as crossed.
        let (mut engine, _) = engine_with(
            AlertCondition::CrossesBelow { target: 100.0 },
            AlertFrequency::Once,
        );
        assert_eq!(feed(&mut engine, &[110.0, 100.0]), vec![1]);
    }

    #[test]
    fn every_time_rearms_only_after_going_back_across() {
        let (mut engine, _) = engine_with(
            AlertCondition::CrossesAbove { target: 100.0 },
            AlertFrequency::EveryTime,
        );
        let fired = feed(
            &mut engine,
            &[90.0, 101.0, 105.0, 110.0, 95.0, 101.0, 102.0, 99.0, 100.0],
        );
        assert_eq!(
            fired,
            vec![1, 5, 8],
            "fires on each fresh crossing, never while it stays above"
        );
        assert_eq!(engine.snapshot().alerts[0].state, AlertState::Active);
        assert_eq!(engine.snapshot().history.len(), 3);
    }

    #[test]
    fn percent_conditions_fire_at_their_derived_target() {
        let (mut engine, _) =
            engine_with(AlertCondition::rises_by(50.0, 100.0), AlertFrequency::Once);
        assert_eq!(feed(&mut engine, &[100.0, 140.0, 150.0]), vec![2]);
        let (mut engine, _) =
            engine_with(AlertCondition::falls_by(50.0, 100.0), AlertFrequency::Once);
        assert_eq!(feed(&mut engine, &[100.0, 60.0, 50.0]), vec![2]);
    }

    #[test]
    fn stale_quotes_wrong_assets_and_repeated_fingerprints_are_ignored() {
        let (mut engine, _) = engine_with(
            AlertCondition::CrossesAbove { target: 100.0 },
            AlertFrequency::EveryTime,
        );
        engine.process(&quote("BTCUSDT", 90.0, T0, "a"), None, None);

        let mut stale = quote("BTCUSDT", 101.0, T0 + 1_000, "b");
        stale.received_at_ms = T0 + 120_000; // 119 s old against a 60 s window
        assert!(engine.process(&stale, None, None).is_empty());

        assert!(
            engine
                .process(&quote("ETHUSDT", 101.0, T0 + 2_000, "c"), None, None)
                .is_empty()
        );

        let first = engine.process(&quote("BTCUSDT", 101.0, T0 + 3_000, "d"), None, None);
        assert_eq!(first.len(), 1);
        // The very same candle again (a poll that returned it twice) is not a second chance...
        assert!(
            engine
                .process(&quote("BTCUSDT", 101.0, T0 + 3_500, "d"), None, None)
                .is_empty()
        );
        // ...but a dip and a fresh candle across the target is.
        engine.process(&quote("BTCUSDT", 90.0, T0 + 4_000, "e"), None, None);
        let again = engine.process(&quote("BTCUSDT", 101.0, T0 + 5_000, "e2"), None, None);
        assert_eq!(again.len(), 1);
    }

    #[test]
    fn alerts_in_another_currency_wait_for_a_matching_conversion() {
        let mut alert = PriceAlert::new(
            asset("BTCUSDT"),
            AlertCondition::CrossesAbove { target: 100.0 },
            T0,
        );
        alert.currency = AlertCurrency::Eur;
        let mut engine = AlertEngine::new(None);
        engine.save_alert(alert, Some(90.0), T0);
        // A USD-converted quote is not for this alert.
        assert!(
            engine
                .process(
                    &quote("BTCUSDT", 105.0, T0, "x"),
                    Some(105.0),
                    Some(AlertCurrency::Usd)
                )
                .is_empty()
        );
        // The EUR conversion is, and it is the converted figure that is compared.
        let events = engine.process(
            &quote("BTCUSDT", 500.0, T0 + 1_000, "x2"),
            Some(101.0),
            Some(AlertCurrency::Eur),
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].observed_value, 101.0);
        assert_eq!(events[0].currency, AlertCurrency::Eur);
    }

    #[test]
    fn saving_with_a_baseline_arms_by_side_and_resets_history_of_quotes() {
        let mut engine = AlertEngine::new(None);
        let mut alert = PriceAlert::new(
            asset("BTCUSDT"),
            AlertCondition::CrossesAbove { target: 100.0 },
            T0,
        );
        alert.last_fingerprint = Some("old".into());
        alert.armed = true;
        let id = alert.id;
        engine.save_alert(alert.clone(), Some(120.0), T0 + 5);
        let saved = &engine.snapshot().alerts[0];
        assert!(!saved.armed, "already above: has to dip first");
        assert_eq!(saved.previous_value, Some(120.0));
        assert_eq!(saved.last_fingerprint, None);
        assert_eq!(saved.updated_at_ms, T0 + 5);

        // Re-saving the same id replaces rather than duplicates.
        engine.save_alert(alert, Some(80.0), T0 + 9);
        assert_eq!(engine.snapshot().alerts.len(), 1);
        assert_eq!(engine.snapshot().alerts[0].id, id);
        assert!(engine.snapshot().alerts[0].armed);
    }

    #[test]
    fn unsupported_conditions_are_parked() {
        let mut engine = AlertEngine::new(None);
        let alert = PriceAlert::new(
            asset("BTCUSDT"),
            AlertCondition::Unsupported { tag: "x".into() },
            T0,
        );
        engine.save_alert(alert, Some(1.0), T0);
        let saved = &engine.snapshot().alerts[0];
        assert_eq!(saved.state, AlertState::Unsupported);
        assert!(!saved.armed);
        assert!(engine.active_assets().is_empty());
        assert!(
            engine
                .process(&quote("BTCUSDT", 1.0, T0, "f"), None, None)
                .is_empty()
        );
    }

    #[test]
    fn pausing_stops_evaluation_and_resuming_rebaselines() {
        let (mut engine, id) = engine_with(
            AlertCondition::CrossesAbove { target: 100.0 },
            AlertFrequency::Once,
        );
        feed(&mut engine, &[90.0]);
        engine.set_state(id, AlertState::Paused, None, T0 + 1);
        assert!(engine.active_assets().is_empty());
        assert!(
            engine
                .process(&quote("BTCUSDT", 120.0, T0 + 2_000, "p"), None, None)
                .is_empty()
        );
        // Resumed while the price is already above: no instant trigger, wait for a dip.
        engine.set_state(id, AlertState::Active, Some(120.0), T0 + 3);
        assert!(!engine.snapshot().alerts[0].armed);
        assert_eq!(engine.active_assets().len(), 1);
        assert!(
            engine
                .process(&quote("BTCUSDT", 121.0, T0 + 4_000, "p2"), None, None)
                .is_empty()
        );
        engine.set_state(999, AlertState::Active, None, T0);
    }

    #[test]
    fn commands_apply_exactly_once_and_remember_a_bounded_window() {
        let mut engine = AlertEngine::new(None);
        let alert = PriceAlert::new(
            asset("BTCUSDT"),
            AlertCondition::CrossesAbove { target: 1.0 },
            T0,
        );
        let id = alert.id;
        let save = AlertRuntimeCommand::new(
            AlertRuntimeCommandPayload::Save {
                alert: Box::new(alert),
                baseline: None,
            },
            T0,
        );
        engine.apply(&save, T0);
        engine.apply(&save, T0);
        assert_eq!(engine.snapshot().alerts.len(), 1);

        let delete = AlertRuntimeCommand::new(AlertRuntimeCommandPayload::Delete(id), T0);
        engine.apply(&delete, T0);
        assert!(engine.snapshot().alerts.is_empty());
        engine.apply(&delete, T0);

        let settings = AlertNotificationSettings {
            sound_enabled: false,
            ..Default::default()
        };
        engine.apply(
            &AlertRuntimeCommand::new(
                AlertRuntimeCommandPayload::UpdateSettings(settings.clone()),
                T0,
            ),
            T0,
        );
        assert_eq!(engine.snapshot().settings, settings);

        for _ in 0..1_100 {
            engine.apply(
                &AlertRuntimeCommand::new(AlertRuntimeCommandPayload::ClearHistory, T0),
                T0,
            );
        }
        assert_eq!(
            engine.snapshot().processed_command_ids.len(),
            PROCESSED_COMMAND_LIMIT
        );
    }

    #[test]
    fn every_write_bumps_the_revision() {
        let mut engine = AlertEngine::new(None);
        assert_eq!(engine.snapshot().revision, 0);
        engine.update_settings(AlertNotificationSettings::default());
        engine.clear_history();
        assert_eq!(engine.snapshot().revision, 2);
    }

    #[test]
    fn identical_alerts_are_detected_but_not_against_themselves() {
        let mut engine = AlertEngine::new(None);
        let alert = PriceAlert::new(
            asset("BTCUSDT"),
            AlertCondition::CrossesAbove { target: 100.0 },
            T0,
        );
        engine.save_alert(alert.clone(), None, T0);
        assert!(!engine.is_identical(&alert));
        let twin = PriceAlert::new(
            asset("BTCUSDT"),
            AlertCondition::CrossesAbove { target: 100.0 },
            T0,
        );
        assert!(engine.is_identical(&twin));
        let different = PriceAlert::new(
            asset("BTCUSDT"),
            AlertCondition::CrossesAbove { target: 101.0 },
            T0,
        );
        assert!(!engine.is_identical(&different));
    }

    #[test]
    fn catch_up_emits_at_most_one_event_per_alert_inside_24_hours() {
        let (mut engine, _) = engine_with(
            AlertCondition::CrossesAbove { target: 100.0 },
            AlertFrequency::EveryTime,
        );
        engine.save_alert(engine.snapshot().alerts[0].clone(), Some(90.0), T0);
        let now = T0 + 10 * 3_600_000;
        let quotes = vec![
            // Out of order, and one older than 24 h (ignored).
            quote("BTCUSDT", 95.0, now - 3_000_000, "m2"),
            quote("BTCUSDT", 80.0, now - 90_000_000, "ancient"),
            quote("BTCUSDT", 101.0, now - 2_000_000, "m3"),
            quote("BTCUSDT", 90.0, now - 1_500_000, "m4"),
            quote("BTCUSDT", 102.0, now - 1_000_000, "m5"),
        ];
        let events = engine.process_catch_up(&quotes, now);
        assert_eq!(events.len(), 1, "two crossings are folded into the first");
        assert_eq!(events[0].origin, AlertEventOrigin::CatchUp);
        assert_eq!(events[0].observed_value, 101.0);
        assert_eq!(events[0].candle, None);
        assert_eq!(engine.snapshot().history.len(), 1);
        assert_eq!(
            engine.snapshot().history[0].origin,
            AlertEventOrigin::CatchUp
        );
    }

    #[test]
    fn delivery_and_health_updates_land_on_the_right_rows() {
        let (mut engine, _) = engine_with(
            AlertCondition::CrossesAbove { target: 100.0 },
            AlertFrequency::Once,
        );
        feed(&mut engine, &[90.0, 101.0]);
        let event_id = engine.snapshot().history[0].id;
        let delivered = AlertDeliveryRecord {
            state: crate::model::alert::AlertDeliveryState::Delivered,
            attempt_count: 1,
            ..Default::default()
        };
        engine.mark_delivery(event_id, delivered.clone());
        engine.mark_delivery(424_242, AlertDeliveryRecord::default());
        assert_eq!(engine.snapshot().history[0].delivery, delivered);
        let health = AlertRuntimeHealth {
            heartbeat_ms: Some(5),
            ..Default::default()
        };
        engine.replace_health(health.clone());
        assert_eq!(engine.snapshot().health, health);
    }

    #[test]
    fn an_engine_resumes_from_a_loaded_snapshot() {
        let (mut engine, _) = engine_with(
            AlertCondition::CrossesAbove { target: 100.0 },
            AlertFrequency::Once,
        );
        feed(&mut engine, &[90.0]);
        let saved = engine.snapshot().clone();
        let mut resumed = AlertEngine::new(Some(saved));
        assert_eq!(resumed.active_assets().len(), 1);
        let events = resumed.process(&quote("BTCUSDT", 101.0, T0 + 9_000, "r"), None, None);
        assert_eq!(events.len(), 1, "previous_value survived the restart");
    }

    // MARK: Muted on this Mac

    #[test]
    fn a_muted_alert_is_defined_but_never_evaluated_or_polled() {
        let (mut engine, id) = engine_with(
            AlertCondition::CrossesAbove { target: 100.0 },
            AlertFrequency::Once,
        );
        assert!(engine.set_muted([id].into()));
        assert_eq!(
            feed(&mut engine, &[90.0, 101.0, 90.0, 101.0]),
            Vec::<usize>::new()
        );
        assert!(
            engine.active_assets().is_empty(),
            "a muted alert is not polled"
        );
        assert_eq!(engine.snapshot().alerts.len(), 1, "it is still defined");
        assert_eq!(engine.snapshot().alerts[0].state, AlertState::Active);
        assert!(engine.snapshot().history.is_empty());
    }

    #[test]
    fn switching_a_muted_alert_on_starts_afresh_and_cannot_fire_from_a_stale_comparison() {
        let (mut engine, id) = engine_with(
            AlertCondition::CrossesAbove { target: 100.0 },
            AlertFrequency::Once,
        );
        // It saw 90 before being muted, then the market moved far while muted.
        assert_eq!(feed(&mut engine, &[90.0]), Vec::<usize>::new());
        engine.set_muted([id].into());
        assert!(engine.set_muted(HashSet::new()));
        assert_eq!(engine.snapshot().alerts[0].previous_value, None);
        // The first quote after switching on is already above the target: not a crossing.
        let events = engine.process(&quote("BTCUSDT", 150.0, T0 + 9_000, "fresh-1"), None, None);
        assert!(events.is_empty());
        assert_eq!(engine.active_assets().len(), 1);
    }

    #[test]
    fn muting_other_alerts_leaves_this_one_running() {
        let (mut engine, id) = engine_with(
            AlertCondition::CrossesAbove { target: 100.0 },
            AlertFrequency::Once,
        );
        assert!(
            !engine.set_muted(HashSet::new()),
            "no change reports no change"
        );
        engine.set_muted([id + 1].into());
        assert_eq!(feed(&mut engine, &[90.0, 101.0]), vec![1]);
    }

    #[test]
    fn setting_the_same_mute_set_does_not_touch_the_revision() {
        let (mut engine, id) = engine_with(
            AlertCondition::CrossesAbove { target: 100.0 },
            AlertFrequency::Once,
        );
        engine.set_muted([id].into());
        let revision = engine.snapshot().revision;
        assert!(!engine.set_muted([id].into()));
        assert_eq!(engine.snapshot().revision, revision);
    }
}
