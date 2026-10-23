//! Which price alerts a chart draws as lines, and where.
//!
//! An alert watches one market for one level. A chart of that market shows each live alert as a
//! horizontal line at its level, in the chart's own price units. Pure: it reads the alerts and
//! the FX rates the app holds and says nothing about drawing.

use super::InstrumentId;
use super::alert::{AlertCondition, AlertCurrency, AlertId, AlertState, PriceAlert};
use super::chart_context::{AlertSide, alert_side};
use super::paper_ticket::rounded_to_increment;
use std::collections::HashMap;

/// One alert on one chart.
#[derive(Clone, Debug, PartialEq)]
pub struct AlertLine {
    pub id: AlertId,
    /// The level in the chart's prices (USD).
    pub price: f64,
    pub paused: bool,
    /// Whether the alert fires when the price rises to the level.
    pub above: bool,
    /// Whether dragging the line may move the level. A percentage alert's level follows from its
    /// percent and reference, so moving it would make it a different alert.
    pub editable: bool,
}

/// Whether an alert dragged to `price` fires on a rise: the side the price lies on from `last`
/// (as for "Add alert at this price"), or the side it had when the market is unknown or level.
pub fn above_after_drag(was_above: bool, price: f64, last: Option<f64>) -> bool {
    match alert_side(price, last) {
        Some(AlertSide::Above) => true,
        Some(AlertSide::Below) => false,
        None => was_above,
    }
}

/// Where a line dragged to `price` lands: on the market's tick, `None` unless that is a price an
/// alert can have.
pub fn dropped_level(price: f64, tick: f64) -> Option<f64> {
    let level = rounded_to_increment(price, tick);
    (level.is_finite() && level > 0.0).then_some(level)
}

/// The lines for the chart of `market`: its active and paused alerts that have a level. Levels
/// are converted to USD with `rates` (USD → currency); an alert in a currency with no rate is
/// left off rather than drawn at a guessed level.
pub fn lines_for(
    alerts: &[PriceAlert],
    market: &InstrumentId,
    rates: &HashMap<AlertCurrency, f64>,
) -> Vec<AlertLine> {
    alerts
        .iter()
        .filter(|alert| alert.asset.instrument == *market)
        .filter(|alert| matches!(alert.state, AlertState::Active | AlertState::Paused))
        .filter_map(|alert| {
            let level = alert.condition.target().filter(|level| level.is_finite())?;
            let rate = if alert.currency == AlertCurrency::Usd {
                1.0
            } else {
                *rates.get(&alert.currency).filter(|rate| **rate > 0.0)?
            };
            Some(AlertLine {
                id: alert.id,
                price: level / rate,
                paused: alert.state == AlertState::Paused,
                above: alert.condition.crosses_above(),
                editable: matches!(
                    alert.condition,
                    AlertCondition::CrossesAbove { .. } | AlertCondition::CrossesBelow { .. }
                ),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::alert::{AlertAsset, AlertCondition};

    fn market(symbol: &str) -> InstrumentId {
        InstrumentId::new(DataSource::Binance, symbol)
    }

    fn alert(id: u64, symbol: &str, condition: AlertCondition) -> PriceAlert {
        let mut alert = PriceAlert::new(
            AlertAsset::new(market(symbol), symbol.to_owned(), symbol.to_owned()),
            condition,
            0,
        );
        alert.id = id;
        alert
    }

    #[test]
    fn only_this_markets_live_alerts_become_lines() {
        let mut paused = alert(2, "BTCUSDT", AlertCondition::CrossesBelow { target: 90.0 });
        paused.state = AlertState::Paused;
        let mut done = alert(3, "BTCUSDT", AlertCondition::CrossesAbove { target: 120.0 });
        done.state = AlertState::Triggered;
        let alerts = [
            alert(1, "BTCUSDT", AlertCondition::CrossesAbove { target: 110.0 }),
            paused,
            done,
            alert(4, "ETHUSDT", AlertCondition::CrossesAbove { target: 5.0 }),
        ];
        let lines = lines_for(&alerts, &market("BTCUSDT"), &HashMap::new());
        assert_eq!(
            lines,
            [
                AlertLine {
                    id: 1,
                    price: 110.0,
                    paused: false,
                    above: true,
                    editable: true
                },
                AlertLine {
                    id: 2,
                    price: 90.0,
                    paused: true,
                    above: false,
                    editable: true
                },
            ]
        );
    }

    #[test]
    fn another_currency_is_converted_or_left_off() {
        let mut euro = alert(1, "BTCUSDT", AlertCondition::CrossesAbove { target: 90.0 });
        euro.currency = AlertCurrency::Eur;
        let alerts = [euro];
        assert!(lines_for(&alerts, &market("BTCUSDT"), &HashMap::new()).is_empty());
        let rates = HashMap::from([(AlertCurrency::Eur, 0.9)]);
        let lines = lines_for(&alerts, &market("BTCUSDT"), &rates);
        assert!((lines[0].price - 100.0).abs() < 1e-9);
        let bad = HashMap::from([(AlertCurrency::Eur, 0.0)]);
        assert!(lines_for(&alerts, &market("BTCUSDT"), &bad).is_empty());
    }

    #[test]
    fn a_level_that_is_not_a_number_is_not_drawn() {
        let alerts = [alert(
            1,
            "BTCUSDT",
            AlertCondition::CrossesAbove { target: f64::NAN },
        )];
        assert!(lines_for(&alerts, &market("BTCUSDT"), &HashMap::new()).is_empty());
    }

    #[test]
    fn only_a_plain_crossing_can_be_dragged() {
        let alerts = [
            alert(1, "BTCUSDT", AlertCondition::CrossesBelow { target: 90.0 }),
            alert(2, "BTCUSDT", AlertCondition::rises_by(5.0, 100.0)),
            alert(3, "BTCUSDT", AlertCondition::falls_by(5.0, 100.0)),
        ];
        let editable: Vec<bool> = lines_for(&alerts, &market("BTCUSDT"), &HashMap::new())
            .iter()
            .map(|line| line.editable)
            .collect();
        assert_eq!(editable, [true, false, false]);
    }

    #[test]
    fn a_dragged_alert_watches_the_side_its_level_is_on() {
        assert!(above_after_drag(false, 110.0, Some(100.0)));
        assert!(!above_after_drag(true, 90.0, Some(100.0)));
        // Level with the market, or no market: it keeps the side it had.
        assert!(above_after_drag(true, 100.0, Some(100.0)));
        assert!(!above_after_drag(false, 110.0, None));
        assert!(!above_after_drag(false, 110.0, Some(f64::NAN)));
    }

    #[test]
    fn a_drop_lands_on_the_tick_and_never_on_a_non_price() {
        assert_eq!(dropped_level(101.237, 0.01), Some(101.24));
        assert_eq!(dropped_level(101.237, 0.0), Some(101.237));
        assert_eq!(dropped_level(0.004, 0.01), None);
        assert_eq!(dropped_level(-3.0, 0.01), None);
        assert_eq!(dropped_level(f64::NAN, 0.01), None);
    }
}
