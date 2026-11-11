//! Everything the transaction editor needs to know about what is typed into it: parsed values,
//! the price/total relationship, the summary figures and which fields are wrong — kept out of the
//! view so it can be tested. A port of `TransactionEditorDraft`.
//!
//! Price and total describe the same number twice, so only one of them is typed at a time: the one
//! edited last is the source of truth and the other is derived from it and the quantity. The
//! ledger (`portfolio::accounting::validate`) stays the authority on what it accepts; this only
//! warns before saving.

// Transient: the portfolio host opens the editor once it is wired.
#![allow(dead_code)]

use crate::format::civil_from_days;
use crate::model::paper::ticket::parse_decimal;
use crate::model::portfolio::csv::parse_iso8601;
use crate::model::portfolio::{PortfolioCurrency, PortfolioTransaction, PortfolioTransactionType};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EditorField {
    Quantity,
    Price,
    Total,
    Fee,
    Date,
}

/// The five groups the type selector offers; each holds one or more transaction types.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Category {
    Buy,
    Sell,
    Transfer,
    Income,
    Other,
}

impl Category {
    pub const ALL: [Self; 5] = [
        Self::Buy,
        Self::Sell,
        Self::Transfer,
        Self::Income,
        Self::Other,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Buy => "Buy",
            Self::Sell => "Sell",
            Self::Transfer => "Transfer",
            Self::Income => "Income",
            Self::Other => "Other",
        }
    }

    pub fn types(self) -> &'static [PortfolioTransactionType] {
        use PortfolioTransactionType::*;
        match self {
            Self::Buy => &[Buy],
            Self::Sell => &[Sell],
            Self::Transfer => &[TransferIn, TransferOut],
            Self::Income => &[Reward, StakingReward, Airdrop, Mining, Interest],
            Self::Other => &[Fee, Adjustment],
        }
    }

    pub fn of(kind: PortfolioTransactionType) -> Self {
        Self::ALL
            .into_iter()
            .find(|category| category.types().contains(&kind))
            .unwrap_or(Self::Other)
    }
}

/// Which of the two linked fields the user typed in last.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PricedField {
    Price,
    Total,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub value: f64,
    pub fee: f64,
    /// "Total cost" or "Net proceeds" with the figure; None for types where it has no meaning.
    pub net: Option<(&'static str, f64)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TransactionDraft {
    pub kind: PortfolioTransactionType,
    pub quantity_text: String,
    pub fee_text: String,
    /// "2024-03-05 14:30" in the viewer's time zone.
    pub date_text: String,
    pub notes: String,
    last_edited: PricedField,
    price_text: String,
    total_text: String,
    currency: PortfolioCurrency,
    utc_offset_secs: i64,
    /// The timestamp the draft opened with and how it read: untouched, it is kept to the
    /// millisecond (the text only shows minutes).
    opened_ms: i64,
    opened_text: String,
}

// MARK: Number text

/// The window of fraction digits a price of this size is written with: two decimals from 1,000
/// up, up to four from one unit, four significant figures below it.
pub fn price_fraction_digits(
    magnitude: f64,
    whole_units: bool,
    minimum: Option<usize>,
) -> (usize, usize) {
    if whole_units && magnitude >= 1.0 {
        return (0, 0);
    }
    let floor = if whole_units { 0 } else { minimum.unwrap_or(2) };
    if magnitude >= 1000.0 {
        let digits = floor.max(2);
        return (digits, digits);
    }
    if magnitude >= 1.0 {
        return (floor, floor.max(4));
    }
    if magnitude == 0.0 {
        return (floor, floor);
    }
    let mut leading_zeros = 0;
    let mut scaled = magnitude;
    while scaled < 0.1 && leading_zeros < 12 {
        scaled *= 10.0;
        leading_zeros += 1;
    }
    (floor, floor.max(12.min(leading_zeros + 4)))
}

fn group_digits(digits: &str) -> String {
    let mut output = String::new();
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(character);
    }
    output
}

/// `value` grouped, with between `min` and `max` fraction digits (trailing zeros trimmed down to
/// `min`).
pub fn format_grouped(value: f64, min: usize, max: usize) -> String {
    let negative = value < 0.0;
    let mut text = format!("{:.max$}", value.abs());
    if let Some(dot) = text.find('.') {
        let mut end = text.len();
        while end > dot + 1 + min && text.as_bytes()[end - 1] == b'0' {
            end -= 1;
        }
        text.truncate(end);
        if text.ends_with('.') {
            text.pop();
        }
    }
    let (whole, fraction) = match text.split_once('.') {
        Some((whole, fraction)) => (whole.to_owned(), Some(fraction.to_owned())),
        None => (text, None),
    };
    let mut result = group_digits(&whole);
    if let Some(fraction) = fraction {
        result.push('.');
        result.push_str(&fraction);
    }
    if negative && result.chars().any(|c| c.is_ascii_digit() && c != '0') {
        result.insert(0, '-');
    }
    result
}

/// Every digit the stored value has, grouped, with at least `minimum_fraction` decimals, so
/// opening a transaction and saving it unchanged cannot alter it.
fn exact(value: f64, minimum_fraction: usize) -> String {
    let shortest = format!("{value}");
    let (whole, fraction) = match shortest.split_once('.') {
        Some((whole, fraction)) => (whole.to_owned(), fraction.to_owned()),
        None => (shortest, String::new()),
    };
    let negative = whole.starts_with('-');
    let digits = whole.trim_start_matches('-');
    let mut fraction = fraction;
    while fraction.len() < minimum_fraction {
        fraction.push('0');
    }
    let mut result = group_digits(digits);
    if !fraction.is_empty() {
        result.push('.');
        result.push_str(&fraction);
    }
    if negative {
        result.insert(0, '-');
    }
    result
}

fn round8(value: f64) -> f64 {
    (value * 1e8).round() / 1e8
}

// MARK: Dates

/// "2024-03-05 14:30" in the time zone `utc_offset_secs` east of UTC.
pub fn date_text(epoch_ms: i64, utc_offset_secs: i64) -> String {
    let local = epoch_ms.div_euclid(1_000) + utc_offset_secs;
    let (year, month, day) = civil_from_days(local.div_euclid(86_400));
    let rest = local.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        rest / 3_600,
        rest % 3_600 / 60
    )
}

/// Reads "2024-03-05 14:30" (or "…T14:30", with optional ":SS") as local time; a bare date is
/// midnight.
pub fn parse_date_text(text: &str, utc_offset_secs: i64) -> Option<i64> {
    let text = text.trim().replace('T', " ");
    let (date, time) = match text.split_once(' ') {
        Some((date, time)) => (date.trim(), time.trim()),
        None => (text.as_str(), "00:00"),
    };
    let time = if time.matches(':').count() == 1 {
        format!("{time}:00")
    } else {
        time.to_owned()
    };
    // Checks the calendar and clock ranges the same way the CSV importer does.
    let utc = parse_iso8601(&format!("{date}T{time}Z"))?;
    Some(utc - utc_offset_secs * 1_000)
}

impl TransactionDraft {
    /// The zone the draft's date text is written in, seconds east of UTC.
    pub fn utc_offset_secs(&self) -> i64 {
        self.utc_offset_secs
    }

    pub fn new(transaction: &PortfolioTransaction, utc_offset_secs: i64) -> Self {
        Self {
            kind: transaction.kind,
            quantity_text: if transaction.quantity == 0.0 {
                String::new()
            } else {
                exact(transaction.quantity, 0)
            },
            fee_text: if transaction.fee == 0.0 {
                String::new()
            } else {
                exact(transaction.fee, 0)
            },
            date_text: date_text(transaction.timestamp, utc_offset_secs),
            opened_ms: transaction.timestamp,
            opened_text: date_text(transaction.timestamp, utc_offset_secs),
            notes: transaction.notes.clone(),
            last_edited: PricedField::Price,
            price_text: transaction
                .price
                .map(|price| exact(price, 2))
                .unwrap_or_default(),
            total_text: String::new(),
            currency: transaction.price_currency,
            utc_offset_secs,
        }
    }

    // MARK: Type

    pub fn category(&self) -> Category {
        Category::of(self.kind)
    }

    /// Moves to `category`, keeping the current type when it already belongs there.
    pub fn select(&mut self, category: Category) {
        if category.types().contains(&self.kind) {
            return;
        }
        if let Some(first) = category.types().first() {
            self.kind = *first;
        }
    }

    /// Transfer-outs, fees and adjustments carry no price.
    pub fn uses_price(&self) -> bool {
        !matches!(
            self.kind,
            PortfolioTransactionType::TransferOut
                | PortfolioTransactionType::Fee
                | PortfolioTransactionType::Adjustment
        )
    }

    /// The ledger rejects a buy or sell without one.
    pub fn requires_price(&self) -> bool {
        matches!(
            self.kind,
            PortfolioTransactionType::Buy | PortfolioTransactionType::Sell
        )
    }

    // MARK: Typed values

    fn parse(text: &str) -> Option<f64> {
        parse_decimal(text, false)
    }

    pub fn quantity(&self) -> Option<f64> {
        Self::parse(&self.quantity_text)
    }

    pub fn fee(&self) -> Option<f64> {
        if self.fee_text.trim().is_empty() {
            Some(0.0)
        } else {
            Self::parse(&self.fee_text)
        }
    }

    pub fn timestamp(&self) -> Option<i64> {
        if self.date_text == self.opened_text {
            return Some(self.opened_ms);
        }
        parse_date_text(&self.date_text, self.utc_offset_secs)
    }

    /// The price per unit: typed, or derived from the typed total.
    pub fn price(&self) -> Option<f64> {
        match self.last_edited {
            PricedField::Price => Self::parse(&self.price_text),
            PricedField::Total => {
                let total = Self::parse(&self.total_text)?;
                let quantity = self.quantity().filter(|quantity| *quantity > 0.0)?;
                Some(round8(total / quantity))
            }
        }
    }

    /// The total: typed, or the quantity at the price.
    pub fn total(&self) -> Option<f64> {
        match self.last_edited {
            PricedField::Total => Self::parse(&self.total_text),
            PricedField::Price => {
                let quantity = self.quantity().filter(|quantity| *quantity > 0.0)?;
                Some(round8(quantity * Self::parse(&self.price_text)?))
            }
        }
    }

    pub fn last_edited(&self) -> PricedField {
        self.last_edited
    }

    /// What the price field shows: its own text while it is the source, else the derived figure.
    pub fn displayed_price(&self) -> String {
        match self.last_edited {
            PricedField::Price => self.price_text.clone(),
            PricedField::Total => self
                .price()
                .map(|price| self.text_for_market_price(price))
                .unwrap_or_default(),
        }
    }

    pub fn displayed_total(&self) -> String {
        match self.last_edited {
            PricedField::Total => self.total_text.clone(),
            PricedField::Price => self
                .total()
                .map(|total| self.text_for_total(total))
                .unwrap_or_default(),
        }
    }

    pub fn set_price(&mut self, text: &str) {
        if text != self.displayed_price() {
            self.price_text = text.to_owned();
            self.last_edited = PricedField::Price;
        }
    }

    pub fn set_total(&mut self, text: &str) {
        if text != self.displayed_total() {
            self.total_text = text.to_owned();
            self.last_edited = PricedField::Total;
        }
    }

    /// A price as it should read in the price field: grouped, with the precision of its size.
    pub fn text_for_market_price(&self, value: f64) -> String {
        let (min, max) = price_fraction_digits(
            value.abs(),
            self.currency == PortfolioCurrency::Jpy,
            (self.currency == PortfolioCurrency::Btc).then_some(0),
        );
        format_grouped(value, min, max)
    }

    /// A derived total in the currency's own precision: cents, whole yen, satoshis.
    pub fn text_for_total(&self, value: f64) -> String {
        let (min, max) = match self.currency {
            PortfolioCurrency::Jpy => (0, 0),
            PortfolioCurrency::Btc => (0, 8),
            _ => (2, 2),
        };
        format_grouped(value, min, max)
    }

    /// Fills the price with `value` — but only while the user has not entered a price or a total,
    /// so a market price arriving late never overwrites what they typed.
    pub fn prefill_price(&mut self, value: f64) {
        if value > 0.0
            && self.last_edited == PricedField::Price
            && self.price_text.trim().is_empty()
        {
            self.price_text = self.text_for_market_price(value);
        }
    }

    // MARK: Validation

    fn number_issue(text: &str, allows_zero: bool) -> Option<&'static str> {
        if text.trim().is_empty() {
            return None;
        }
        let Some(value) = Self::parse(text) else {
            return Some("Enter a number.");
        };
        if value < 0.0 {
            return Some("Can't be negative.");
        }
        if value == 0.0 && !allows_zero {
            return Some("Must be greater than zero.");
        }
        None
    }

    /// Fields with something wrong. An empty field is missing, not wrong, so a fresh form never
    /// opens red.
    pub fn issues(&self) -> HashMap<EditorField, String> {
        let mut result = HashMap::new();
        let mut note = |field, issue: Option<&'static str>| {
            if let Some(issue) = issue {
                result.insert(field, issue.to_owned());
            }
        };
        note(
            EditorField::Quantity,
            Self::number_issue(&self.quantity_text, false),
        );
        if self.uses_price() {
            match self.last_edited {
                PricedField::Price => note(
                    EditorField::Price,
                    Self::number_issue(&self.price_text, true),
                ),
                PricedField::Total => note(
                    EditorField::Total,
                    Self::number_issue(&self.total_text, true),
                ),
            }
        }
        note(EditorField::Fee, Self::number_issue(&self.fee_text, true));
        if self.timestamp().is_none() {
            result.insert(EditorField::Date, "Use YYYY-MM-DD HH:MM.".to_owned());
        }
        result
    }

    pub fn can_submit(&self) -> bool {
        let Some(quantity) = self.quantity() else {
            return false;
        };
        if quantity <= 0.0 || self.fee().is_none() || self.timestamp().is_none() {
            return false;
        }
        if !self.issues().is_empty() {
            return false;
        }
        !(self.uses_price() && self.requires_price()) || self.price().is_some()
    }

    // MARK: Summary

    /// The cost or proceeds; None until there is a quantity and a price.
    pub fn summary(&self) -> Option<Summary> {
        if !self.uses_price() {
            return None;
        }
        let (total, fee) = (self.total()?, self.fee()?);
        let net = match self.kind {
            PortfolioTransactionType::Buy | PortfolioTransactionType::TransferIn => {
                Some(("Total cost", total + fee))
            }
            PortfolioTransactionType::Sell => Some(("Net proceeds", total - fee)),
            _ => None,
        };
        Some(Summary {
            value: total,
            fee,
            net,
        })
    }

    // MARK: Output

    /// `base` with the typed values applied; None while the draft cannot be submitted. A price
    /// the chosen type does not use is dropped rather than kept from an earlier choice.
    pub fn make_transaction(
        &self,
        base: &PortfolioTransaction,
        now_ms: i64,
    ) -> Option<PortfolioTransaction> {
        if !self.can_submit() {
            return None;
        }
        let mut transaction = base.clone();
        transaction.kind = self.kind;
        transaction.quantity = self.quantity()?;
        transaction.price = if self.uses_price() {
            self.price()
        } else {
            None
        };
        transaction.fee = self.fee()?;
        transaction.timestamp = self.timestamp()?;
        transaction.notes = self.notes.clone();
        transaction.updated_at = now_ms;
        Some(transaction)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::{PortfolioAsset, PortfolioSource};
    use std::collections::HashMap;

    // 2024-03-05 14:30 UTC.
    const NOON: i64 = 1_709_649_000_000;

    fn transaction() -> PortfolioTransaction {
        PortfolioTransaction {
            id: 5,
            portfolio_id: 1,
            asset: PortfolioAsset {
                key: "Binance:BTCUSDT".into(),
                symbol: "BTC/USDT".into(),
                name: "Bitcoin".into(),
                source: DataSource::Binance,
                quote_currency: PortfolioCurrency::Usd,
                metadata: HashMap::new(),
            },
            kind: PortfolioTransactionType::Buy,
            quantity: 0.5,
            price: Some(60_000.25),
            price_currency: PortfolioCurrency::Usd,
            fee: 1.5,
            fee_currency: PortfolioCurrency::Usd,
            timestamp: NOON,
            notes: "first".into(),
            source: PortfolioSource::Manual,
            external_transaction_id: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn blank() -> PortfolioTransaction {
        PortfolioTransaction {
            quantity: 0.0,
            price: None,
            fee: 0.0,
            notes: String::new(),
            ..transaction()
        }
    }

    #[test]
    fn precision_follows_the_size_of_the_price() {
        assert_eq!(price_fraction_digits(67_432.19, false, None), (2, 2));
        assert_eq!(price_fraction_digits(12.3456, false, None), (2, 4));
        assert_eq!(price_fraction_digits(0.5, false, None), (2, 4));
        assert_eq!(price_fraction_digits(0.00000278, false, None), (2, 9));
        assert_eq!(price_fraction_digits(0.0, false, None), (2, 2));
        assert_eq!(
            price_fraction_digits(150.0, true, None),
            (0, 0),
            "whole yen"
        );
        assert_eq!(price_fraction_digits(2_000.0, false, Some(0)), (2, 2));
        assert_eq!(price_fraction_digits(0.05, false, Some(0)), (0, 5));
    }

    #[test]
    fn numbers_group_and_trim() {
        assert_eq!(format_grouped(1_234_567.891, 2, 2), "1,234,567.89");
        assert_eq!(format_grouped(12.5, 2, 4), "12.50");
        assert_eq!(format_grouped(12.5, 0, 4), "12.5");
        assert_eq!(format_grouped(-0.001, 2, 2), "0.00", "no negative zero");
        assert_eq!(format_grouped(-1234.5, 2, 2), "-1,234.50");
        assert_eq!(exact(60_000.25, 2), "60,000.25");
        assert_eq!(exact(0.1, 2), "0.10");
        assert_eq!(exact(1234.0, 0), "1,234");
        assert_eq!(exact(0.000_000_01, 0), "0.00000001");
    }

    #[test]
    fn an_unchanged_edit_round_trips_every_digit() {
        let mut original = transaction();
        original.quantity = 0.123_456_78;
        original.price = Some(61_234.567_89);
        let draft = TransactionDraft::new(&original, 0);
        assert_eq!(draft.quantity(), Some(0.123_456_78));
        assert_eq!(draft.price(), Some(61_234.567_89));
        let saved = draft.make_transaction(&original, 99).unwrap();
        assert_eq!(saved.quantity, original.quantity);
        assert_eq!(saved.price, original.price);
        assert_eq!(saved.timestamp, original.timestamp);
        assert_eq!(saved.updated_at, 99);
        assert_eq!(saved.id, original.id);
    }

    #[test]
    fn the_last_edited_of_price_and_total_wins_and_the_other_follows() {
        let mut draft = TransactionDraft::new(&blank(), 0);
        draft.quantity_text = "2".into();
        draft.set_price("100");
        assert_eq!(draft.displayed_total(), "200.00");
        assert_eq!(draft.total(), Some(200.0));
        draft.set_total("250");
        assert_eq!(draft.last_edited(), PricedField::Total);
        assert_eq!(draft.price(), Some(125.0));
        assert_eq!(draft.displayed_price(), "125.00");
        draft.quantity_text = "4".into();
        assert_eq!(
            draft.price(),
            Some(62.5),
            "the total stays; the price follows the quantity"
        );
        draft.set_price("62.5");
        draft.set_price("62.5");
        assert_eq!(draft.last_edited(), PricedField::Price);
        draft.quantity_text = "0".into();
        assert_eq!(draft.total(), None);
    }

    #[test]
    fn a_late_market_price_never_overwrites_what_was_typed() {
        let mut draft = TransactionDraft::new(&blank(), 0);
        draft.prefill_price(67432.1987);
        assert_eq!(draft.displayed_price(), "67,432.20");
        draft.prefill_price(1.0);
        assert_eq!(draft.displayed_price(), "67,432.20", "already filled");
        let mut typed = TransactionDraft::new(&blank(), 0);
        typed.set_total("500");
        typed.prefill_price(10.0);
        assert_eq!(typed.last_edited(), PricedField::Total);
        let mut zero = TransactionDraft::new(&blank(), 0);
        zero.prefill_price(0.0);
        assert_eq!(zero.displayed_price(), "");
    }

    #[test]
    fn categories_group_the_types() {
        let mut draft = TransactionDraft::new(&transaction(), 0);
        assert_eq!(draft.category(), Category::Buy);
        draft.select(Category::Transfer);
        assert_eq!(draft.kind, PortfolioTransactionType::TransferIn);
        draft.kind = PortfolioTransactionType::TransferOut;
        draft.select(Category::Transfer);
        assert_eq!(
            draft.kind,
            PortfolioTransactionType::TransferOut,
            "kept inside its group"
        );
        draft.select(Category::Income);
        assert_eq!(draft.kind, PortfolioTransactionType::Reward);
        for kind in crate::model::portfolio::csv::ALL_TYPES {
            assert!(Category::of(kind).types().contains(&kind));
        }
    }

    #[test]
    fn some_types_carry_no_price_and_a_buy_or_sell_needs_one() {
        let mut draft = TransactionDraft::new(&blank(), 0);
        draft.quantity_text = "1".into();
        assert!(!draft.can_submit(), "a buy needs a price");
        draft.set_price("10");
        assert!(draft.can_submit());
        draft.kind = PortfolioTransactionType::Reward;
        draft.set_price("");
        assert!(draft.can_submit(), "income may be priceless");
        assert!(draft.uses_price() && !draft.requires_price());
        draft.kind = PortfolioTransactionType::Fee;
        assert!(!draft.uses_price());
        draft.set_price("5");
        let saved = draft.make_transaction(&blank(), 1).unwrap();
        assert_eq!(
            saved.price, None,
            "a price the type does not use is dropped"
        );
        assert_eq!(draft.summary(), None);
    }

    #[test]
    fn empty_fields_are_missing_not_wrong_and_bad_ones_say_why() {
        let mut draft = TransactionDraft::new(&blank(), 0);
        assert!(draft.issues().is_empty());
        assert!(!draft.can_submit());
        draft.quantity_text = "abc".into();
        draft.fee_text = "-1".into();
        draft.set_price("-5");
        let issues = draft.issues();
        assert_eq!(issues[&EditorField::Quantity], "Enter a number.");
        assert_eq!(issues[&EditorField::Fee], "Can't be negative.");
        assert_eq!(issues[&EditorField::Price], "Can't be negative.");
        draft.quantity_text = "0".into();
        assert_eq!(
            draft.issues()[&EditorField::Quantity],
            "Must be greater than zero."
        );
        draft.quantity_text = "1,5".into();
        draft.set_price("0");
        draft.fee_text.clear();
        assert!(draft.issues().is_empty(), "a zero price is allowed");
        draft.date_text = "someday".into();
        assert_eq!(draft.issues()[&EditorField::Date], "Use YYYY-MM-DD HH:MM.");
        assert!(!draft.can_submit());
    }

    #[test]
    fn the_summary_names_cost_or_proceeds() {
        let mut draft = TransactionDraft::new(&transaction(), 0);
        let summary = draft.summary().unwrap();
        assert_eq!(summary.value, 30_000.125);
        assert_eq!(summary.net, Some(("Total cost", 30_001.625)));
        draft.kind = PortfolioTransactionType::Sell;
        assert_eq!(
            draft.summary().unwrap().net,
            Some(("Net proceeds", 29_998.625))
        );
        draft.kind = PortfolioTransactionType::Reward;
        assert_eq!(draft.summary().unwrap().net, None);
        draft.quantity_text.clear();
        assert_eq!(draft.summary(), None);
    }

    #[test]
    fn dates_read_and_write_in_the_viewers_time_zone() {
        assert_eq!(date_text(NOON, 0), "2024-03-05 14:30");
        assert_eq!(date_text(NOON, 2 * 3_600), "2024-03-05 16:30");
        assert_eq!(parse_date_text("2024-03-05 16:30", 2 * 3_600), Some(NOON));
        assert_eq!(parse_date_text("2024-03-05T14:30:00", 0), Some(NOON));
        assert_eq!(
            parse_date_text("2024-03-05", 0),
            Some(NOON - 14 * 3_600_000 - 30 * 60_000)
        );
        for bad in ["", "2024-13-01 10:00", "2024-03-05 25:00", "05/03/2024"] {
            assert_eq!(parse_date_text(bad, 0), None, "{bad}");
        }
        let mut draft = TransactionDraft::new(&transaction(), 3_600);
        assert_eq!(draft.date_text, "2024-03-05 15:30");
        assert_eq!(draft.timestamp(), Some(NOON));
        draft.date_text = "2024-03-06 09:00".into();
        let saved = draft.make_transaction(&transaction(), 0).unwrap();
        assert_eq!(
            saved.timestamp, 1_709_712_000_000,
            "09:00 at UTC+1 is 08:00Z"
        );
    }

    #[test]
    fn an_untouched_date_keeps_its_seconds() {
        let mut original = transaction();
        original.timestamp = NOON + 37_123;
        let mut draft = TransactionDraft::new(&original, 0);
        assert_eq!(
            draft.date_text, "2024-03-05 14:30",
            "the text shows minutes only"
        );
        assert_eq!(
            draft.make_transaction(&original, 0).unwrap().timestamp,
            original.timestamp
        );
        draft.date_text = "2024-03-05 14:31".into();
        assert_eq!(
            draft.timestamp(),
            Some(NOON + 60_000),
            "an edited date is exactly what was typed"
        );
    }
}
