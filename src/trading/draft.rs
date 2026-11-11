//! What is typed into the order sheet, for any venue.
//!
//! The text is kept as typed: nothing is rounded to make an order acceptable. Each venue's adapter
//! reads it with its own rules (a broker's validation, the paper engine's) and answers with
//! [`DraftCheck`]: what is wrong, field by field, and what the order would cost.

use degenview_brokers::{Decimal, Side};

use super::model::{OrderClass, TicketCaps, TimeInForce};

/// Which input a problem belongs to.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DraftField {
    Size,
    Price,
    StopPrice,
    TakeProfit,
    StopLoss,
    TimeInForce,
    /// The order as a whole (its type, the market).
    Order,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DraftIssue {
    pub field: DraftField,
    pub text: String,
}

impl DraftIssue {
    pub fn new(field: DraftField, text: impl Into<String>) -> Self {
        Self {
            field,
            text: text.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TradeDraft {
    pub side: Side,
    pub class: OrderClass,
    pub size_text: String,
    /// A market buy states an amount to spend (in the quote asset) rather than a size to buy.
    pub spend_in_quote: bool,
    /// The limit price (a limit, or the limit of a stop-limit).
    pub price_text: String,
    pub stop_text: String,
    pub tif: TimeInForce,
    pub post_only: bool,
    pub reduce_only: bool,
    /// The take-profit and stop-loss section is open: its prices are part of the order.
    pub bracket: bool,
    pub take_profit_text: String,
    pub stop_loss_text: String,
}

impl TradeDraft {
    pub fn new(side: Side) -> Self {
        Self {
            side,
            class: OrderClass::Limit,
            size_text: String::new(),
            spend_in_quote: false,
            price_text: String::new(),
            stop_text: String::new(),
            tif: TimeInForce::GoodTilCancelled,
            post_only: false,
            reduce_only: false,
            bracket: false,
            take_profit_text: String::new(),
            stop_loss_text: String::new(),
        }
    }

    /// An order of `class` at a price clicked on the chart.
    pub fn at_price(side: Side, class: OrderClass, price: Decimal) -> Self {
        let mut draft = Self::new(side);
        draft.class = class;
        let text = price.normalize().to_string();
        match class {
            OrderClass::Market => {}
            OrderClass::Limit => draft.price_text = text,
            OrderClass::Stop => draft.stop_text = text,
            OrderClass::StopLimit => {
                draft.stop_text = text.clone();
                draft.price_text = text;
            }
        }
        draft
    }

    pub fn side_label(&self) -> &'static str {
        match self.side {
            Side::Buy => "Buy",
            Side::Sell => "Sell",
        }
    }

    pub fn uses_limit_price(&self) -> bool {
        matches!(self.class, OrderClass::Limit | OrderClass::StopLimit)
    }

    pub fn uses_stop_price(&self) -> bool {
        matches!(self.class, OrderClass::Stop | OrderClass::StopLimit)
    }

    /// Puts back what a venue cannot do, so a choice made for one venue never travels to another
    /// that has no such control: an order type it lacks, a time in force it lacks, a flag it lacks.
    pub fn fit_to(&mut self, caps: &TicketCaps) {
        if !caps.allows(self.class)
            && let Some(first) = caps.order_classes.first()
        {
            self.class = *first;
        }
        if !caps.time_in_force.contains(&self.tif)
            && let Some(first) = caps.time_in_force.first()
        {
            self.tif = *first;
        }
        if !caps.post_only {
            self.post_only = false;
        }
        if !caps.reduce_only {
            self.reduce_only = false;
        }
        if !caps.brackets {
            self.bracket = false;
        }
        if !caps.quote_sized_market || self.side == Side::Sell || self.class != OrderClass::Market {
            self.spend_in_quote = false;
        }
    }

    /// The fields this order type needs that are still empty.
    pub fn missing(&self) -> Vec<DraftField> {
        let mut missing = Vec::new();
        if self.size_text.trim().is_empty() {
            missing.push(DraftField::Size);
        }
        if self.uses_limit_price() && self.price_text.trim().is_empty() {
            missing.push(DraftField::Price);
        }
        if self.uses_stop_price() && self.stop_text.trim().is_empty() {
            missing.push(DraftField::StopPrice);
        }
        missing
    }

    /// What is typed, as it decides whether a fresh answer from the venue is needed: any change
    /// to it makes an earlier preview about a different order.
    pub fn key(&self) -> String {
        format!("{self:?}")
    }
}

/// `value` rounded down to a whole number of `increment`s (a size that can be sent).
pub fn floor_to_increment(value: Decimal, increment: Decimal) -> Decimal {
    if increment <= Decimal::ZERO {
        return value;
    }
    (value / increment).floor() * increment
}

/// What an order would cost, as far as it can be known before the venue's own preview.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Estimate {
    /// The price the figures assume.
    pub reference: Option<Decimal>,
    /// The order's value in the quote asset.
    pub notional: Option<Decimal>,
    pub fee: Option<Decimal>,
    /// Margin the order would hold, for a venue with leverage.
    pub margin: Option<Decimal>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DraftCheck {
    /// What is wrong with what was typed. A field left empty is `missing`, not an issue, so a fresh
    /// sheet does not open covered in red.
    pub issues: Vec<DraftIssue>,
    /// Required fields still empty.
    pub missing: Vec<DraftField>,
    pub estimate: Estimate,
}

impl DraftCheck {
    /// Nothing is wrong and nothing is missing: the venue may be asked about the order.
    pub fn is_complete(&self) -> bool {
        self.issues.is_empty() && self.missing.is_empty()
    }

    pub fn issue_for(&self, field: DraftField) -> Option<&str> {
        self.issues
            .iter()
            .find(|issue| issue.field == field)
            .map(|issue| issue.text.as_str())
    }
}

/// The venue's answer to "what would this order do".
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PreviewResult {
    /// The venue would take the order.
    pub accepted: bool,
    /// The total including fees.
    pub total: Option<Decimal>,
    pub fee: Option<Decimal>,
    pub average_price: Option<Decimal>,
    /// Said when the order would be refused.
    pub problems: Vec<String>,
    /// Said when it would be taken but the user should know.
    pub warnings: Vec<String>,
}

/// An order the venue took.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Submitted {
    pub id: String,
}

/// What the venue said about sending an order: it took it, or the user-facing reason it did not.
pub type SubmitResult = Result<Submitted, String>;

/// What the venue said about a cancel.
pub type CancelResult = Result<(), String>;

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(classes: &[OrderClass]) -> TicketCaps {
        TicketCaps {
            order_classes: classes.to_vec(),
            time_in_force: vec![TimeInForce::GoodTilCancelled],
            post_only: false,
            reduce_only: false,
            brackets: false,
            quote_sized_market: false,
            shorting: false,
            edit_in_place: false,
        }
    }

    #[test]
    fn a_choice_made_for_one_venue_is_put_back_where_another_cannot_do_it() {
        let mut draft = TradeDraft::new(Side::Buy);
        draft.class = OrderClass::Stop;
        draft.tif = TimeInForce::Day;
        draft.post_only = true;
        draft.reduce_only = true;
        draft.bracket = true;
        draft.fit_to(&caps(&[OrderClass::Limit, OrderClass::Market]));
        assert_eq!(draft.class, OrderClass::Limit);
        assert_eq!(draft.tif, TimeInForce::GoodTilCancelled);
        assert!(!draft.post_only && !draft.reduce_only && !draft.bracket);
        // What it can do is left alone.
        let mut kept = TradeDraft::new(Side::Sell);
        kept.class = OrderClass::Market;
        kept.fit_to(&caps(&[OrderClass::Limit, OrderClass::Market]));
        assert_eq!(kept.class, OrderClass::Market);
    }

    #[test]
    fn spending_an_amount_is_only_for_a_market_buy_where_the_venue_allows_it() {
        let mut allowed = caps(&[OrderClass::Market, OrderClass::Limit]);
        allowed.quote_sized_market = true;
        let mut buy = TradeDraft::new(Side::Buy);
        buy.class = OrderClass::Market;
        buy.spend_in_quote = true;
        buy.fit_to(&allowed);
        assert!(buy.spend_in_quote);
        buy.side = Side::Sell;
        buy.fit_to(&allowed);
        assert!(!buy.spend_in_quote, "a sell has no amount to spend");
        buy.side = Side::Buy;
        buy.spend_in_quote = true;
        buy.class = OrderClass::Limit;
        buy.fit_to(&allowed);
        assert!(!buy.spend_in_quote, "a limit order is sized");
    }

    #[test]
    fn an_order_at_a_clicked_price_fills_the_fields_its_type_uses() {
        let price = Decimal::new(9_000_050, 2);
        let limit = TradeDraft::at_price(Side::Buy, OrderClass::Limit, price);
        assert_eq!(limit.price_text, "90000.5");
        let stop_limit = TradeDraft::at_price(Side::Sell, OrderClass::StopLimit, price);
        assert_eq!(stop_limit.stop_text, "90000.5");
        assert_eq!(stop_limit.price_text, "90000.5");
        let stop = TradeDraft::at_price(Side::Sell, OrderClass::Stop, price);
        assert_eq!(
            (stop.stop_text.as_str(), stop.price_text.as_str()),
            ("90000.5", "")
        );
        assert!(
            TradeDraft::at_price(Side::Buy, OrderClass::Market, price)
                .price_text
                .is_empty()
        );
    }

    #[test]
    fn a_size_that_can_be_sent_is_a_whole_number_of_increments() {
        let inc = Decimal::new(1, 3);
        assert_eq!(
            floor_to_increment(Decimal::new(12_349, 4), inc),
            Decimal::new(1_234, 3)
        );
        assert_eq!(floor_to_increment(Decimal::new(5, 4), inc), Decimal::ZERO);
        assert_eq!(
            floor_to_increment(Decimal::new(7, 0), Decimal::ZERO),
            Decimal::new(7, 0)
        );
    }

    #[test]
    fn what_an_order_type_needs_is_missing_until_typed() {
        let mut draft = TradeDraft::new(Side::Buy);
        assert_eq!(draft.missing(), vec![DraftField::Size, DraftField::Price]);
        draft.class = OrderClass::StopLimit;
        assert_eq!(
            draft.missing(),
            vec![DraftField::Size, DraftField::Price, DraftField::StopPrice]
        );
        draft.class = OrderClass::Market;
        assert_eq!(draft.missing(), vec![DraftField::Size]);
        draft.size_text = " 0.1 ".to_owned();
        assert!(draft.missing().is_empty());
    }

    #[test]
    fn a_change_to_what_is_typed_changes_its_key() {
        let mut draft = TradeDraft::new(Side::Buy);
        let before = draft.key();
        assert_eq!(before, draft.key());
        draft.size_text = "1".to_owned();
        assert_ne!(before, draft.key());
    }
}
