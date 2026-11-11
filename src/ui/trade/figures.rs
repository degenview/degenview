//! The numbers the order sheet writes in words: a price move, a fee as a share of the order, what
//! the order comes to. Pure, so they are tested without a window.

use degenview_brokers::{Decimal, Side};

use crate::trading::confirm;

/// How many places a price of this size needs to read well, whatever the market allows.
fn places_for(value: Decimal) -> u32 {
    let magnitude = value.abs();
    if magnitude >= Decimal::from(1_000) {
        2
    } else if magnitude >= Decimal::ONE {
        4
    } else if magnitude >= Decimal::new(1, 2) {
        6
    } else {
        8
    }
}

/// A price clicked on the chart, moved onto the market's tick: the nearest multiple of
/// `increment`. A market counted in powers of ten (1e-8) also drops the places a price of this
/// size does not need, so a click on a bitcoin chart reads `90,123.45`, not eight decimals.
pub fn snap_price(value: Decimal, increment: Decimal) -> Decimal {
    if increment <= Decimal::ZERO {
        return value;
    }
    let snapped = (value / increment).round() * increment;
    let tick = increment.normalize();
    if tick.mantissa() == 1 {
        snapped.round_dp(tick.scale().min(places_for(snapped)))
    } else {
        snapped.normalize()
    }
}

/// A price at the market's own number of places (`1,234.50`), but never more than a price of its
/// size needs: a market that counts in 1e-8 does not make `90,000.00000000` of a bitcoin. Without
/// an increment, what it is.
pub fn price_label(value: Decimal, increment: Option<Decimal>) -> String {
    match increment {
        Some(increment) => {
            let places = increment.normalize().scale().min(places_for(value));
            confirm::price_text(value.round_dp(places), Decimal::new(1, places))
        }
        None => crate::model::broker_ticket::display(value.normalize()),
    }
}

/// `+0.00234 (+1.51%)`: a move in price and in percent. The price keeps the market's own number
/// of places (`increment`), so a move reads like the prices beside it.
pub fn price_move_text(delta: f64, percent: f64, increment: Option<Decimal>) -> String {
    let sign = if delta < 0.0 { '-' } else { '+' };
    let amount = Decimal::try_from(delta.abs()).ok().map(|magnitude| {
        price_label(
            if increment.is_some() {
                magnitude
            } else {
                magnitude.round_dp(4)
            },
            increment,
        )
    });
    match amount {
        Some(amount) => format!("{sign}{amount} ({percent:+.2}%)"),
        None => format!("{percent:+.2}%"),
    }
}

/// A fee as a percentage of the order's value (`0.10`), two places. None for an order worth nothing.
pub fn fee_percent(fee: Decimal, value: Decimal) -> Option<String> {
    if value <= Decimal::ZERO {
        return None;
    }
    let mut percent = (fee * Decimal::from(100) / value).round_dp(2);
    percent.rescale(2);
    Some(percent.to_string())
}

/// What the order comes to: the venue's own total when it has given one, else the value with the
/// fee on top for a buy and taken off for a sell. Without a fee, the value alone.
pub fn order_total(
    side: Side,
    value: Option<Decimal>,
    fee: Option<Decimal>,
    venue_total: Option<Decimal>,
) -> Option<Decimal> {
    if venue_total.is_some() {
        return venue_total;
    }
    match (value, fee) {
        (Some(value), Some(fee)) => Some(match side {
            Side::Buy => value + fee,
            Side::Sell => value - fee,
        }),
        (value, None) => value,
        (None, Some(_)) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    #[test]
    fn a_move_keeps_the_places_of_the_price() {
        assert_eq!(
            price_move_text(0.00234, 1.51, Some(dec("0.00001"))),
            "+0.00234 (+1.51%)"
        );
        assert_eq!(
            price_move_text(-120.5, -0.4, Some(dec("0.01"))),
            "-120.50 (-0.40%)"
        );
        assert_eq!(
            price_move_text(1234.5, 2.0, Some(dec("0.01"))),
            "+1,234.50 (+2.00%)"
        );
    }

    #[test]
    fn a_move_with_no_increment_is_rounded_and_one_that_is_not_a_number_is_only_a_percent() {
        assert_eq!(price_move_text(0.123456, 1.0, None), "+0.1235 (+1.00%)");
        assert_eq!(price_move_text(f64::NAN, 1.0, Some(dec("0.01"))), "+1.00%");
    }

    #[test]
    fn a_clicked_price_lands_on_the_tick() {
        // The raw float a chart click gives: far more places than any tick.
        assert_eq!(
            snap_price(dec("0.15671234567891234"), dec("0.00000001")),
            dec("0.156712")
        );
        assert_eq!(snap_price(dec("90123.4567"), dec("0.01")), dec("90123.46"));
        // A tick that is not a power of ten: the nearest multiple.
        assert_eq!(snap_price(dec("10.12"), dec("0.05")), dec("10.10"));
        assert_eq!(snap_price(dec("10.13"), dec("0.05")), dec("10.15"));
        // No tick known: left alone.
        assert_eq!(snap_price(dec("1.234"), Decimal::ZERO), dec("1.234"));
    }

    #[test]
    fn a_price_has_the_places_of_the_market() {
        assert_eq!(
            price_label(
                Decimal::try_from(0.1567000001).unwrap(),
                Some(dec("0.00001"))
            ),
            "0.15670"
        );
        assert_eq!(price_label(dec("61000"), Some(dec("0.01"))), "61,000.00");
        // A market counted in 1e-8 still shows a bitcoin price to the cent.
        assert_eq!(
            price_label(dec("90000"), Some(dec("0.00000001"))),
            "90,000.00"
        );
    }

    #[test]
    fn a_fee_is_a_share_of_the_value_to_two_places() {
        assert_eq!(
            fee_percent(dec("10"), dec("10000")).as_deref(),
            Some("0.10")
        );
        assert_eq!(fee_percent(dec("0.6"), dec("100")).as_deref(), Some("0.60"));
        assert_eq!(fee_percent(dec("1"), dec("0")), None);
    }

    #[test]
    fn the_total_adds_the_fee_to_a_buy_and_takes_it_from_a_sell() {
        let (value, fee) = (Some(dec("1000")), Some(dec("1")));
        assert_eq!(order_total(Side::Buy, value, fee, None), Some(dec("1001")));
        assert_eq!(order_total(Side::Sell, value, fee, None), Some(dec("999")));
        assert_eq!(order_total(Side::Buy, value, None, None), value);
        assert_eq!(order_total(Side::Buy, None, fee, None), None);
    }

    #[test]
    fn the_venues_own_total_wins() {
        assert_eq!(
            order_total(
                Side::Buy,
                Some(dec("1000")),
                Some(dec("1")),
                Some(dec("1002.5"))
            ),
            Some(dec("1002.5"))
        );
    }
}
