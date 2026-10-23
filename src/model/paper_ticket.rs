//! Everything the order ticket needs to know about what is typed into it: parsed values, the
//! figures in the summary and which fields are wrong. Kept out of the view so it can be tested.
//!
//! The margin and tick rules mirror `paper_engine` (`required_margin`, `validate_step`): margin is
//! charged only for the part of an order that adds exposure, and quantities and prices must sit on
//! the instrument's increments. The engine stays the authority; this only warns before submitting.

// Transient: the order ticket sheet hosts this once paper trading is wired.
#![allow(dead_code)]

use crate::model::paper::{
    PaperCommissionConfiguration, PaperInstrument, PaperOrderRequest, PaperOrderSide,
    PaperOrderType, PaperTimeInForce,
};
use crate::model::paper_format::{self as fmt, decimal_places};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TicketField {
    Quantity,
    LimitPrice,
    StopPrice,
    TakeProfit,
    StopLoss,
}

// MARK: Typed numbers

/// Reads a number the way a person types it, in either separator convention.
///
/// Spaces and apostrophes are dropped. A lone `,` or `.` is a decimal point, unless the other
/// separator is this locale's decimal and exactly three digits follow it ("1.234" in a comma
/// locale, "1,234" otherwise, is a thousands group). With both present the last one is the decimal
/// point. "0,500" and ",500" are fractions: a leading zero is never a group.
pub fn parse_decimal(text: &str, comma_locale: bool) -> Option<f64> {
    let cleaned: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '\'' && *c != '\u{2019}')
        .collect();
    if cleaned.is_empty() {
        return None;
    }
    let normalized = normalize(&cleaned, comma_locale);
    if !is_plain_number(&normalized) {
        return None;
    }
    let padded = if normalized.starts_with('.') {
        format!("0{normalized}")
    } else if normalized.starts_with("-.") {
        format!("-0{}", &normalized[1..])
    } else {
        normalized
    };
    let value: f64 = padded.parse().ok()?;
    value.is_finite().then_some(value)
}

/// A value as it should sit in an input field: the locale's decimal separator, no grouping, so
/// `parse_decimal` reads it back unchanged.
pub fn decimal_text(value: f64, comma_locale: bool) -> String {
    let text = format!("{value}");
    if comma_locale {
        text.replace('.', ",")
    } else {
        text
    }
}

fn normalize(text: &str, comma_locale: bool) -> String {
    let commas = text.matches(',').count();
    let dots = text.matches('.').count();
    match (commas, dots) {
        (0, 0) => text.to_owned(),
        (0, 1) => {
            if comma_locale
                && digits_after_last(text, '.') == 3
                && has_leading_group_digits(text, '.')
            {
                text.replace('.', "")
            } else {
                text.to_owned()
            }
        }
        (1, 0) => {
            if !comma_locale
                && digits_after_last(text, ',') == 3
                && has_leading_group_digits(text, ',')
            {
                text.replace(',', "")
            } else {
                text.replace(',', ".")
            }
        }
        (_, 0) => text.replace(',', ""),
        (0, _) => text.replace('.', ""),
        _ => {
            let last_comma = text.rfind(',').unwrap_or(0);
            let last_dot = text.rfind('.').unwrap_or(0);
            if last_comma > last_dot {
                text.replace('.', "").replace(',', ".")
            } else {
                text.replace(',', "")
            }
        }
    }
}

fn digits_after_last(text: &str, separator: char) -> usize {
    text.rfind(separator).map_or(0, |index| {
        text[index + separator.len_utf8()..].chars().count()
    })
}

/// "1,234" groups; "0,500" and ",500" do not.
fn has_leading_group_digits(text: &str, separator: char) -> bool {
    let Some(index) = text.find(separator) else {
        return false;
    };
    let head = &text[..index];
    (1..=3).contains(&head.chars().count())
        && !head.starts_with('0')
        && head.chars().all(|c| c.is_ascii_digit())
}

fn is_plain_number(text: &str) -> bool {
    let body = text.strip_prefix('-').unwrap_or(text);
    !body.is_empty()
        && body.matches('.').count() <= 1
        && body.chars().any(|c| c.is_ascii_digit())
        && body.chars().all(|c| c.is_ascii_digit() || c == '.')
}

// MARK: Increments

/// Whether `value` is a whole number of `increment` steps. The ticket and the engine both ask
/// this, so what the ticket lets through the engine accepts.
///
/// It counts steps in integers: `value` and `increment` are scaled by the increment's decimal
/// places, so a price of 83,105.43 on a 0.00000001 tick is 8,310,543,000,000 steps of 1, not a
/// quotient whose float noise (about 0.001 at that size) would read as a fraction. The tolerance
/// grows with the magnitude for the same reason, and nothing finer than a float can hold fails.
pub fn is_aligned(value: f64, increment: f64) -> bool {
    if increment <= 0.0 {
        return true;
    }
    if !value.is_finite() || !increment.is_finite() {
        return false;
    }
    let scale = 10_f64.powi(decimal_places(increment).min(15) as i32);
    let scaled = value * scale;
    let steps = scaled.round();
    let tolerance = 1e-6_f64.max(scaled.abs() * 8.0 * f64::EPSILON);
    if (scaled - steps).abs() > tolerance {
        return false;
    }
    let increment_steps = (increment * scale).round();
    increment_steps < 1.0 || steps % increment_steps == 0.0
}

/// `value` rounded down to a multiple of `increment`, with the float noise cleaned off.
pub fn rounded_down(value: f64, increment: f64) -> f64 {
    if increment <= 0.0 {
        return value;
    }
    // The nudge keeps 0.3 / 0.1 (2.9999999999999996) from losing a whole step.
    let steps = (value / increment + 1e-9).floor();
    let scale = 10_f64.powi(decimal_places(increment).min(15) as i32);
    (steps * increment * scale).round() / scale
}

/// A price rounded to the nearest tick.
pub fn rounded_to_tick(value: f64, instrument: &PaperInstrument) -> f64 {
    rounded_to_increment(value, instrument.tick_size)
}

/// `value` rounded to the nearest multiple of `tick`, with the float noise cleaned off; a tick
/// that is not positive leaves it alone.
pub fn rounded_to_increment(value: f64, tick: f64) -> f64 {
    if tick <= 0.0 {
        return value;
    }
    let scale = 10_f64.powi(decimal_places(tick).min(15) as i32);
    let ticks = (value / tick).round();
    (ticks * tick * scale).round() / scale
}

/// The asset being traded: `BTC` for `BTC/USDT`; a name without a pair falls back to the symbol.
pub fn base_symbol(instrument: &PaperInstrument) -> String {
    if let Some((base, _)) = instrument.display_name.split_once('/')
        && !base.is_empty()
    {
        return base.to_uppercase();
    }
    instrument.symbol.to_uppercase()
}

/// Whole-unit instruments (stocks) start at one share; fractional ones start empty, where a
/// default of "1" would be a full coin.
pub fn initial_quantity_text(instrument: &PaperInstrument) -> String {
    if instrument.quantity_increment >= 1.0 {
        "1".to_owned()
    } else {
        String::new()
    }
}

// MARK: Draft

#[derive(Clone, Debug)]
pub struct TicketDraft {
    // Market context
    pub instrument: PaperInstrument,
    pub bid: Option<f64>,
    pub ask: Option<f64>,
    pub last: Option<f64>,
    pub available_funds: Option<f64>,
    pub leverage: f64,
    pub commission: PaperCommissionConfiguration,
    /// The open position in this instrument: positive long, negative short, zero none.
    pub existing_signed_quantity: f64,
    /// Whether the viewer writes decimals with a comma.
    pub comma_locale: bool,

    // What is typed
    pub side: PaperOrderSide,
    pub kind: PaperOrderType,
    pub time_in_force: PaperTimeInForce,
    pub quantity_text: String,
    pub limit_price_text: String,
    pub stop_price_text: String,
    pub take_profit_text: String,
    pub stop_loss_text: String,
}

impl TicketDraft {
    pub fn new(instrument: PaperInstrument) -> Self {
        Self {
            instrument,
            bid: None,
            ask: None,
            last: None,
            available_funds: None,
            leverage: 1.0,
            commission: PaperCommissionConfiguration::None,
            existing_signed_quantity: 0.0,
            comma_locale: false,
            side: PaperOrderSide::Buy,
            kind: PaperOrderType::Market,
            time_in_force: PaperTimeInForce::GoodTilCanceled,
            quantity_text: String::new(),
            limit_price_text: String::new(),
            stop_price_text: String::new(),
            take_profit_text: String::new(),
            stop_loss_text: String::new(),
        }
    }

    /// A fresh ticket for `side`: the quantity default, and the reference price in both price
    /// fields (rounded to a tick) so a limit or stop starts at the market.
    pub fn opening(
        instrument: PaperInstrument,
        reference_price: Option<f64>,
        side: PaperOrderSide,
    ) -> Self {
        let mut draft = Self::new(instrument);
        draft.side = side;
        draft.quantity_text = initial_quantity_text(&draft.instrument);
        if let Some(price) = reference_price {
            let text = decimal_text(
                rounded_to_tick(price, &draft.instrument),
                draft.comma_locale,
            );
            draft.limit_price_text = text.clone();
            draft.stop_price_text = text;
        }
        draft
    }

    /// A fresh ticket for a pending `kind` order (a limit or a stop) at `price`, rounded to a
    /// tick. The quantity is the instrument's default; nothing is submitted by opening it.
    pub fn pending(
        instrument: PaperInstrument,
        side: PaperOrderSide,
        kind: PaperOrderType,
        price: f64,
    ) -> Self {
        let mut draft = Self::opening(instrument, Some(price), side);
        draft.kind = kind;
        draft
    }

    // MARK: Parsed values

    fn parse(&self, text: &str) -> Option<f64> {
        parse_decimal(text, self.comma_locale)
    }

    pub fn quantity(&self) -> Option<f64> {
        self.parse(&self.quantity_text)
    }

    pub fn limit_price(&self) -> Option<f64> {
        self.parse(&self.limit_price_text)
    }

    pub fn stop_price(&self) -> Option<f64> {
        self.parse(&self.stop_price_text)
    }

    pub fn take_profit(&self) -> Option<f64> {
        self.parse(&self.take_profit_text)
    }

    pub fn stop_loss(&self) -> Option<f64> {
        self.parse(&self.stop_loss_text)
    }

    pub fn uses_limit_price(&self) -> bool {
        matches!(self.kind, PaperOrderType::Limit | PaperOrderType::StopLimit)
    }

    pub fn uses_stop_price(&self) -> bool {
        matches!(self.kind, PaperOrderType::Stop | PaperOrderType::StopLimit)
    }

    // MARK: Pricing

    /// What a market order would fill at: the ask to buy, the bid to sell, else the last trade.
    pub fn market_price(&self) -> Option<f64> {
        let book = if self.side.is_buy() {
            self.ask
        } else {
            self.bid
        };
        book.or(self.last)
    }

    /// The price the order is valued at: its own limit or stop, else the market.
    pub fn valuation_price(&self) -> Option<f64> {
        match self.kind {
            PaperOrderType::Market => self.market_price(),
            PaperOrderType::Limit | PaperOrderType::StopLimit => self.limit_price(),
            PaperOrderType::Stop => self.stop_price(),
        }
    }

    pub fn has_book(&self) -> bool {
        self.bid.is_some() && self.ask.is_some()
    }

    pub fn spread(&self) -> Option<f64> {
        let (bid, ask) = (self.bid?, self.ask?);
        (ask >= bid).then_some(ask - bid)
    }

    pub fn notional(&self) -> Option<f64> {
        Some(self.quantity()? * self.valuation_price()? * self.instrument.contract_multiplier)
    }

    /// The part of this order that closes the opposite open position.
    pub fn reducible_quantity(&self) -> f64 {
        let existing = self.existing_signed_quantity;
        let reduces =
            (existing > 0.0 && !self.side.is_buy()) || (existing < 0.0 && self.side.is_buy());
        if reduces { existing.abs() } else { 0.0 }
    }

    /// Larger than the open position on the other side: closes it and opens the rest.
    pub fn flips_position(&self) -> bool {
        match self.quantity() {
            Some(quantity) => {
                self.reducible_quantity() > 0.0 && quantity > self.reducible_quantity()
            }
            None => false,
        }
    }

    pub fn required_margin(&self) -> Option<f64> {
        let quantity = self.quantity()?;
        let price = self.valuation_price()?;
        let exposure = (quantity - self.reducible_quantity()).max(0.0);
        Some(exposure * price * self.instrument.contract_multiplier / self.leverage.max(1.0))
    }

    pub fn estimated_fee(&self) -> Option<f64> {
        let quantity = self.quantity()?;
        let price = self.valuation_price()?;
        match self.commission {
            PaperCommissionConfiguration::None => None,
            PaperCommissionConfiguration::FixedPerOrder { value } => Some(value),
            PaperCommissionConfiguration::Percentage { value } => {
                Some(quantity * price * self.instrument.contract_multiplier * value / 100.0)
            }
            PaperCommissionConfiguration::PerContract { value } => Some(quantity * value),
        }
    }

    pub fn exceeds_funds(&self) -> bool {
        match (self.required_margin(), self.available_funds) {
            (Some(required), Some(funds)) => required > funds,
            _ => false,
        }
    }

    /// How much margin is missing, when the order needs more than the account has free.
    pub fn shortfall(&self) -> Option<f64> {
        if !self.exceeds_funds() {
            return None;
        }
        Some(self.required_margin()? - self.available_funds?)
    }

    // MARK: Validation

    /// Fields that hold something wrong. A field left empty is `missing_fields`, not an issue, so
    /// a fresh ticket does not open covered in red.
    pub fn issues(&self) -> HashMap<TicketField, String> {
        let mut result = HashMap::new();
        if !self.quantity_text.trim().is_empty() {
            let instrument = &self.instrument;
            let issue = match self.quantity() {
                None => Some("Enter a number.".to_owned()),
                Some(quantity) if quantity <= 0.0 => Some("Must be greater than zero.".to_owned()),
                Some(quantity) if quantity < instrument.minimum_quantity => Some(format!(
                    "Minimum is {}.",
                    fmt::quantity(instrument.minimum_quantity, instrument)
                )),
                Some(quantity) if !is_aligned(quantity, instrument.quantity_increment) => {
                    Some(format!(
                        "Must be a multiple of {}.",
                        fmt::quantity(instrument.quantity_increment, instrument)
                    ))
                }
                Some(_) => None,
            };
            if let Some(issue) = issue {
                result.insert(TicketField::Quantity, issue);
            }
        }
        if self.uses_limit_price()
            && let Some(issue) = self.price_issue(&self.limit_price_text, self.limit_price())
        {
            result.insert(TicketField::LimitPrice, issue);
        }
        if self.uses_stop_price()
            && let Some(issue) = self.price_issue(&self.stop_price_text, self.stop_price())
        {
            result.insert(TicketField::StopPrice, issue);
        }
        if let Some(issue) = self.protection_issue(&self.take_profit_text, self.take_profit(), true)
        {
            result.insert(TicketField::TakeProfit, issue);
        }
        if let Some(issue) = self.protection_issue(&self.stop_loss_text, self.stop_loss(), false) {
            result.insert(TicketField::StopLoss, issue);
        }
        result
    }

    /// Required fields still empty.
    pub fn missing_fields(&self) -> Vec<TicketField> {
        let mut result = Vec::new();
        if self.quantity_text.trim().is_empty() {
            result.push(TicketField::Quantity);
        }
        if self.uses_limit_price() && self.limit_price_text.trim().is_empty() {
            result.push(TicketField::LimitPrice);
        }
        if self.uses_stop_price() && self.stop_price_text.trim().is_empty() {
            result.push(TicketField::StopPrice);
        }
        result
    }

    pub fn can_submit(&self) -> bool {
        self.quantity().is_some()
            && self.issues().is_empty()
            && self.missing_fields().is_empty()
            && !self.exceeds_funds()
    }

    // MARK: Actions

    /// The quantity that would use `fraction` (0...1) of the available funds at the valuation
    /// price, rounded down to the instrument's increment. None when funds, price or minimum size
    /// rule it out.
    pub fn quantity_for_funds_fraction(&self, fraction: f64) -> Option<f64> {
        let funds = self.available_funds.filter(|funds| *funds > 0.0)?;
        if fraction <= 0.0 {
            return None;
        }
        let price = self.valuation_price().or_else(|| self.market_price())?;
        if price <= 0.0 {
            return None;
        }
        let raw = funds * fraction.min(1.0) * self.leverage.max(1.0)
            / (price * self.instrument.contract_multiplier);
        let rounded = rounded_down(raw, self.instrument.quantity_increment);
        (rounded >= self.instrument.minimum_quantity && rounded > 0.0).then_some(rounded)
    }

    pub fn request(&self, account_id: u64) -> Option<PaperOrderRequest> {
        let quantity = self.quantity()?;
        Some(PaperOrderRequest {
            account_id,
            instrument: self.instrument.clone(),
            side: self.side,
            kind: self.kind,
            quantity,
            limit_price: if self.uses_limit_price() {
                self.limit_price()
            } else {
                None
            },
            stop_price: if self.uses_stop_price() {
                self.stop_price()
            } else {
                None
            },
            time_in_force: self.time_in_force,
            take_profit: self.take_profit(),
            stop_loss: self.stop_loss(),
        })
    }

    // MARK: Words

    /// "Buy 0.5 BTC" once there is a quantity; "Buy BTC" before.
    pub fn submit_title(&self) -> String {
        let symbol = base_symbol(&self.instrument);
        match self.quantity() {
            Some(quantity) if quantity > 0.0 => format!(
                "{} {} {symbol}",
                self.side.label(),
                fmt::quantity(quantity, &self.instrument)
            ),
            _ => format!("{} {symbol}", self.side.label()),
        }
    }

    /// How the order changes an open position in the same instrument; None when there is none.
    pub fn position_note(&self) -> Option<String> {
        let existing = self.existing_signed_quantity;
        if existing == 0.0 {
            return None;
        }
        let held = existing.abs();
        let (direction, opposite) = if existing > 0.0 {
            ("long", "short")
        } else {
            ("short", "long")
        };
        let text = |value: f64| fmt::quantity(value, &self.instrument);
        let Some(quantity) = self.quantity().filter(|quantity| *quantity > 0.0) else {
            return Some(format!("{} {direction} open", text(held)));
        };
        Some(if self.reducible_quantity() == 0.0 {
            format!("Adds to {direction} {}", text(held))
        } else if (quantity - held).abs() < f64::EPSILON * held.max(1.0) * 4.0 {
            format!("Closes {direction} {}", text(held))
        } else if quantity < held {
            format!("Reduces {direction} by {}", text(quantity))
        } else {
            format!(
                "Closes {direction}, opens {opposite} {}",
                text(quantity - held)
            )
        })
    }

    fn price_issue(&self, text: &str, value: Option<f64>) -> Option<String> {
        if text.trim().is_empty() {
            return None;
        }
        let Some(value) = value else {
            return Some("Enter a number.".to_owned());
        };
        if value <= 0.0 {
            return Some("Must be greater than zero.".to_owned());
        }
        if !is_aligned(value, self.instrument.tick_size) {
            return Some(format!(
                "Must be a multiple of {}.",
                fmt::price(self.instrument.tick_size, &self.instrument)
            ));
        }
        None
    }

    /// A take profit sits beyond the entry in the trade's favour; a stop loss sits behind it.
    fn protection_issue(
        &self,
        text: &str,
        value: Option<f64>,
        is_take_profit: bool,
    ) -> Option<String> {
        if let Some(issue) = self.price_issue(text, value) {
            return Some(issue);
        }
        let (value, entry) = (value?, self.valuation_price()?);
        let above = self.side.is_buy() == is_take_profit;
        let shown = fmt::price(entry, &self.instrument);
        if above && value <= entry {
            return Some(format!("Must be above {shown}."));
        }
        if !above && value >= entry {
            return Some(format!("Must be below {shown}."));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;

    fn btc() -> PaperInstrument {
        let mut instrument = PaperInstrument::chart("BTCUSDT", "BTC/USDT", DataSource::Binance);
        instrument.tick_size = 0.01;
        instrument.minimum_quantity = 0.001;
        instrument.quantity_increment = 0.001;
        instrument
    }

    fn draft() -> TicketDraft {
        let mut draft = TicketDraft::new(btc());
        draft.bid = Some(99.0);
        draft.ask = Some(101.0);
        draft.last = Some(100.0);
        draft.available_funds = Some(10_000.0);
        draft
    }

    #[test]
    fn a_pending_ticket_from_a_chart_click_has_no_price_complaint() {
        // The chart's own instrument: the default crypto tick of 0.00000001, and a clicked price
        // that is any float. The prefilled prices are rounded onto the tick and must validate.
        let instrument = PaperInstrument::chart("BTCUSDT", "BTC/USDT", DataSource::Binance);
        for clicked in [
            83_105.123_456_789,
            118_150.99,
            83_105.0,
            0.000_012_345_678_9,
        ] {
            for (side, kind) in [
                (PaperOrderSide::Sell, PaperOrderType::Limit),
                (PaperOrderSide::Buy, PaperOrderType::Stop),
                (PaperOrderSide::Buy, PaperOrderType::Limit),
                (PaperOrderSide::Sell, PaperOrderType::Stop),
            ] {
                let mut draft = TicketDraft::pending(instrument.clone(), side, kind, clicked);
                draft.quantity_text = "0.5".to_owned();
                let issues = draft.issues();
                assert!(
                    !issues.contains_key(&TicketField::LimitPrice)
                        && !issues.contains_key(&TicketField::StopPrice),
                    "{clicked} {side:?} {kind:?}: {issues:?}"
                );
            }
        }
    }

    #[test]
    fn a_pending_ticket_opens_on_its_side_type_and_tick_rounded_price() {
        let sell_limit = TicketDraft::pending(
            btc(),
            PaperOrderSide::Sell,
            PaperOrderType::Limit,
            118_150.994,
        );
        assert_eq!(sell_limit.side, PaperOrderSide::Sell);
        assert_eq!(sell_limit.kind, PaperOrderType::Limit);
        assert_eq!(sell_limit.limit_price(), Some(118_150.99));
        let buy_stop =
            TicketDraft::pending(btc(), PaperOrderSide::Buy, PaperOrderType::Stop, 120.0);
        assert_eq!(buy_stop.kind, PaperOrderType::Stop);
        assert_eq!(buy_stop.stop_price(), Some(120.0));
        // Opening is not submitting: the quantity is the instrument's default and nothing else
        // about the draft is decided for the user.
        assert_eq!(buy_stop.time_in_force, PaperTimeInForce::GoodTilCanceled);
    }

    #[test]
    fn typed_numbers_read_in_either_convention() {
        for (text, comma, expected) in [
            ("1.5", false, Some(1.5)),
            ("1,5", false, Some(1.5)),
            (".5", false, Some(0.5)),
            (",5", false, Some(0.5)),
            ("1,234", false, Some(1234.0)),
            ("1,234", true, Some(1.234)),
            ("1.234", true, Some(1234.0)),
            ("1.234", false, Some(1.234)),
            ("0,500", false, Some(0.5)),
            ("0.500", true, Some(0.5)),
            ("1,234.56", false, Some(1234.56)),
            ("1.234,56", true, Some(1234.56)),
            ("1 234,5", false, Some(1234.5)),
            ("1'234.5", false, Some(1234.5)),
            ("1,234,567", false, Some(1_234_567.0)),
            ("-2", false, Some(-2.0)),
            ("-.5", false, Some(-0.5)),
            ("", false, None),
            ("  ", false, None),
            ("abc", false, None),
            ("1.2.3", true, Some(123.0)),
            ("1e5", false, None),
            ("-", false, None),
            (".", false, None),
        ] {
            assert_eq!(
                parse_decimal(text, comma),
                expected,
                "{text:?} comma={comma}"
            );
        }
    }

    #[test]
    fn field_text_reads_back_unchanged() {
        for value in [0.001, 1.5, 1234.567, 0.1 + 0.2, 100.0, 0.000_000_01] {
            for comma in [false, true] {
                let text = decimal_text(value, comma);
                assert!(!text.contains("e"), "{text}");
                assert_eq!(
                    parse_decimal(&text, comma),
                    Some(value),
                    "{text} comma={comma}"
                );
            }
        }
        assert_eq!(decimal_text(1.5, true), "1,5");
    }

    #[test]
    fn increments_round_down_without_float_noise() {
        assert!(is_aligned(0.3, 0.1));
        assert!(is_aligned(0.007, 0.001));
        assert!(!is_aligned(0.0005, 0.001));
        assert!(is_aligned(5.0, 0.0));
        // Prices far larger than their tick: the float noise of the quotient is not a fraction.
        for price in [83_105.43, 118_150.99, 67_432.19, 99_999.999_999_99] {
            assert!(is_aligned(price, 0.000_000_01), "{price}");
        }
        assert!(!is_aligned(83_105.000_000_001, 0.000_000_01));
        assert!(is_aligned(100.5, 0.5));
        assert!(!is_aligned(100.25, 0.5));
        assert!(is_aligned(5.0, 1.0));
        assert!(!is_aligned(5.5, 1.0));
        assert!(is_aligned(118_150.99, 0.01));
        assert!(!is_aligned(118_150.995, 0.01));
        assert!(!is_aligned(f64::NAN, 0.01));
        assert_eq!(rounded_down(0.3, 0.1), 0.3);
        assert_eq!(rounded_down(1.2345, 0.001), 1.234);
        assert_eq!(rounded_down(7.9, 1.0), 7.0);
        assert_eq!(rounded_down(1.234, 0.0), 1.234);
        assert_eq!(rounded_to_tick(100.004, &btc()), 100.0);
        assert_eq!(rounded_to_tick(100.006, &btc()), 100.01);
    }

    #[test]
    fn the_base_symbol_comes_from_the_pair() {
        assert_eq!(base_symbol(&btc()), "BTC");
        let mut stock = PaperInstrument::chart("aapl", "Apple", DataSource::Alpaca);
        assert_eq!(base_symbol(&stock), "AAPL");
        stock.display_name = "/X".into();
        assert_eq!(base_symbol(&stock), "AAPL", "an empty base falls back");
    }

    #[test]
    fn a_fresh_ticket_starts_sensibly() {
        let stock = PaperInstrument::chart("AAPL", "AAPL", DataSource::Alpaca);
        let ticket = TicketDraft::opening(stock, Some(189.499), PaperOrderSide::Sell);
        assert_eq!(
            ticket.quantity_text, "1",
            "whole-unit instruments start at one"
        );
        assert_eq!(ticket.side, PaperOrderSide::Sell);
        let crypto = TicketDraft::opening(btc(), Some(100.004), PaperOrderSide::Buy);
        assert_eq!(
            crypto.quantity_text, "",
            "a fractional default of 1 would be a full coin"
        );
        assert_eq!(crypto.limit_price_text, "100");
        assert_eq!(crypto.stop_price_text, "100");
        assert!(
            TicketDraft::opening(btc(), None, PaperOrderSide::Buy)
                .limit_price_text
                .is_empty()
        );
    }

    #[test]
    fn market_orders_value_at_the_side_they_trade() {
        let mut ticket = draft();
        ticket.quantity_text = "2".into();
        assert_eq!(ticket.market_price(), Some(101.0), "buy at the ask");
        ticket.side = PaperOrderSide::Sell;
        assert_eq!(ticket.market_price(), Some(99.0), "sell at the bid");
        ticket.bid = None;
        ticket.ask = None;
        assert_eq!(
            ticket.market_price(),
            Some(100.0),
            "last when there is no book"
        );
        assert!(!ticket.has_book());
        assert_eq!(ticket.spread(), None);
        assert_eq!(draft().spread(), Some(2.0));
    }

    #[test]
    fn limits_and_stops_value_at_their_own_price() {
        let mut ticket = draft();
        ticket.quantity_text = "2".into();
        ticket.kind = PaperOrderType::Limit;
        ticket.limit_price_text = "95".into();
        assert_eq!(ticket.valuation_price(), Some(95.0));
        assert_eq!(ticket.notional(), Some(190.0));
        ticket.kind = PaperOrderType::Stop;
        ticket.stop_price_text = "110".into();
        assert_eq!(ticket.valuation_price(), Some(110.0));
        ticket.kind = PaperOrderType::StopLimit;
        assert!(ticket.uses_limit_price() && ticket.uses_stop_price());
        assert_eq!(
            ticket.valuation_price(),
            Some(95.0),
            "stop-limit values at the limit"
        );
    }

    #[test]
    fn margin_is_charged_only_for_added_exposure() {
        let mut ticket = draft();
        ticket.quantity_text = "10".into();
        ticket.leverage = 5.0;
        assert_eq!(ticket.required_margin(), Some(10.0 * 101.0 / 5.0));
        ticket.side = PaperOrderSide::Sell;
        ticket.existing_signed_quantity = 4.0;
        assert_eq!(ticket.reducible_quantity(), 4.0);
        assert!(ticket.flips_position());
        assert_eq!(ticket.required_margin(), Some(6.0 * 99.0 / 5.0));
        ticket.quantity_text = "3".into();
        assert_eq!(
            ticket.required_margin(),
            Some(0.0),
            "a pure reduction needs no margin"
        );
        assert!(!ticket.flips_position());
        ticket.side = PaperOrderSide::Buy;
        assert_eq!(ticket.reducible_quantity(), 0.0, "buying adds to a long");
        ticket.leverage = 0.2;
        assert_eq!(
            ticket.required_margin(),
            Some(3.0 * 101.0),
            "leverage never goes below 1"
        );
    }

    #[test]
    fn fees_follow_the_commission_setting() {
        let mut ticket = draft();
        ticket.quantity_text = "2".into();
        assert_eq!(ticket.estimated_fee(), None);
        ticket.commission = PaperCommissionConfiguration::FixedPerOrder { value: 1.5 };
        assert_eq!(ticket.estimated_fee(), Some(1.5));
        ticket.commission = PaperCommissionConfiguration::Percentage { value: 0.1 };
        assert!((ticket.estimated_fee().unwrap() - 0.202).abs() < 1e-9);
        ticket.commission = PaperCommissionConfiguration::PerContract { value: 0.5 };
        assert_eq!(ticket.estimated_fee(), Some(1.0));
    }

    #[test]
    fn running_out_of_funds_blocks_the_ticket_and_names_the_shortfall() {
        let mut ticket = draft();
        ticket.quantity_text = "1".into();
        assert!(ticket.can_submit());
        assert_eq!(ticket.shortfall(), None);
        ticket.available_funds = Some(60.0);
        assert!(ticket.exceeds_funds());
        assert!(!ticket.can_submit());
        assert_eq!(ticket.shortfall(), Some(41.0));
        ticket.available_funds = None;
        assert!(!ticket.exceeds_funds(), "unknown funds never block");
    }

    #[test]
    fn an_empty_ticket_is_missing_not_wrong() {
        let mut ticket = draft();
        assert!(ticket.issues().is_empty());
        assert_eq!(ticket.missing_fields(), vec![TicketField::Quantity]);
        assert!(!ticket.can_submit());
        ticket.kind = PaperOrderType::StopLimit;
        assert_eq!(
            ticket.missing_fields(),
            vec![
                TicketField::Quantity,
                TicketField::LimitPrice,
                TicketField::StopPrice
            ]
        );
    }

    #[test]
    fn quantities_are_checked_against_the_instrument() {
        let mut ticket = draft();
        for (text, issue) in [
            ("abc", Some("Enter a number.")),
            ("0", Some("Must be greater than zero.")),
            ("-1", Some("Must be greater than zero.")),
            ("0.0005", Some("Minimum is 0.001.")),
            ("0.0015", Some("Must be a multiple of 0.001.")),
            ("0.002", None),
            ("1,5", None),
        ] {
            ticket.quantity_text = text.into();
            assert_eq!(
                ticket
                    .issues()
                    .get(&TicketField::Quantity)
                    .map(String::as_str),
                issue,
                "{text}"
            );
        }
    }

    #[test]
    fn prices_must_sit_on_a_tick() {
        let mut ticket = draft();
        ticket.quantity_text = "1".into();
        ticket.kind = PaperOrderType::Limit;
        ticket.limit_price_text = "99.999".into();
        assert_eq!(
            ticket
                .issues()
                .get(&TicketField::LimitPrice)
                .map(String::as_str),
            Some("Must be a multiple of 0.01.")
        );
        ticket.limit_price_text = "99.99".into();
        assert!(ticket.issues().is_empty());
        ticket.limit_price_text = "x".into();
        assert_eq!(
            ticket
                .issues()
                .get(&TicketField::LimitPrice)
                .map(String::as_str),
            Some("Enter a number.")
        );
        ticket.kind = PaperOrderType::Market;
        assert!(
            ticket.issues().is_empty(),
            "a hidden price field is never an issue"
        );
    }

    #[test]
    fn protection_sits_on_the_right_side_of_the_entry() {
        let mut ticket = draft();
        ticket.quantity_text = "1".into();
        ticket.take_profit_text = "100".into();
        ticket.stop_loss_text = "110".into();
        let issues = ticket.issues();
        assert_eq!(issues[&TicketField::TakeProfit], "Must be above 101.");
        assert_eq!(issues[&TicketField::StopLoss], "Must be below 101.");
        ticket.take_profit_text = "120".into();
        ticket.stop_loss_text = "95".into();
        assert!(ticket.issues().is_empty());
        ticket.side = PaperOrderSide::Sell;
        let issues = ticket.issues();
        assert_eq!(issues[&TicketField::TakeProfit], "Must be below 99.");
        assert_eq!(issues[&TicketField::StopLoss], "Must be above 99.");
    }

    #[test]
    fn funds_shortcuts_round_down_and_respect_the_minimum() {
        let mut ticket = draft();
        assert_eq!(ticket.quantity_for_funds_fraction(0.5), Some(49.504));
        assert_eq!(
            ticket.quantity_for_funds_fraction(2.0),
            Some(99.009),
            "capped at all funds"
        );
        assert_eq!(ticket.quantity_for_funds_fraction(0.0), None);
        ticket.leverage = 2.0;
        assert_eq!(ticket.quantity_for_funds_fraction(1.0), Some(198.019));
        ticket.available_funds = Some(0.0);
        assert_eq!(ticket.quantity_for_funds_fraction(1.0), None);
        ticket.available_funds = Some(0.05);
        assert_eq!(
            ticket.quantity_for_funds_fraction(1.0),
            None,
            "below the minimum size"
        );
        ticket.available_funds = None;
        assert_eq!(ticket.quantity_for_funds_fraction(1.0), None);
    }

    #[test]
    fn the_request_carries_only_the_prices_the_type_uses() {
        let mut ticket = draft();
        ticket.quantity_text = "0.5".into();
        ticket.limit_price_text = "95".into();
        ticket.stop_price_text = "90".into();
        ticket.take_profit_text = "120".into();
        let request = ticket.request(7).unwrap();
        assert_eq!(request.account_id, 7);
        assert_eq!(request.quantity, 0.5);
        assert_eq!((request.limit_price, request.stop_price), (None, None));
        assert_eq!(request.take_profit, Some(120.0));
        ticket.kind = PaperOrderType::StopLimit;
        let request = ticket.request(7).unwrap();
        assert_eq!(
            (request.limit_price, request.stop_price),
            (Some(95.0), Some(90.0))
        );
        ticket.quantity_text.clear();
        assert_eq!(ticket.request(7), None);
    }

    #[test]
    fn words_describe_what_the_order_does() {
        let mut ticket = draft();
        assert_eq!(ticket.submit_title(), "Buy BTC");
        ticket.quantity_text = "0.5".into();
        assert_eq!(ticket.submit_title(), "Buy 0.5 BTC");
        ticket.side = PaperOrderSide::Sell;
        assert_eq!(ticket.submit_title(), "Sell 0.5 BTC");

        assert_eq!(ticket.position_note(), None);
        ticket.existing_signed_quantity = 2.0;
        ticket.quantity_text.clear();
        assert_eq!(ticket.position_note().as_deref(), Some("2 long open"));
        ticket.quantity_text = "0.5".into();
        assert_eq!(
            ticket.position_note().as_deref(),
            Some("Reduces long by 0.5")
        );
        ticket.quantity_text = "2".into();
        assert_eq!(ticket.position_note().as_deref(), Some("Closes long 2"));
        ticket.quantity_text = "3".into();
        assert_eq!(
            ticket.position_note().as_deref(),
            Some("Closes long, opens short 1")
        );
        ticket.side = PaperOrderSide::Buy;
        assert_eq!(ticket.position_note().as_deref(), Some("Adds to long 2"));
    }
}
