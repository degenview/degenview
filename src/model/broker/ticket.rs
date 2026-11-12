//! The order ticket's rules for a live broker: what the typed text means, what is wrong with it,
//! what it would cost, and what the confirmation says. Pure, and exact: every number is a
//! `Decimal`, and nothing is rounded to make an order acceptable. A value off the market's
//! increment is reported with the nearest valid value as a suggestion; the user decides.
//!
//! The market's own rules (increments, minimums, what an order type allows) are the broker crate's
//! `validate`; this module parses the text into an order, hands it over, and puts the answers next
//! to the right field.

use std::str::FromStr;

use std::time::Duration;

use degenview_brokers::model::orders::FeeSummary;
use degenview_brokers::model::reducer::{Notice, NoticeKind};
use degenview_brokers::validate::{self, Field};
use degenview_brokers::{
    Balance, Capabilities, Decimal, LiveOrder, OrderDraft, OrderKind, OrderSize, OrderStatus,
    OrderType, ProductSpec, Quote, Side, StopDirection, TimeInForce,
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TicketKind {
    Market,
    Limit,
    StopLimit,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TicketTif {
    GoodTilCancelled,
    FillOrKill,
}

/// Which input a problem belongs to.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TicketField {
    Size,
    Price,
    StopPrice,
    LimitPrice,
    TimeInForce,
    Type,
}

/// What is typed in the ticket.
#[derive(Clone, Debug, PartialEq)]
pub struct TicketDraft {
    pub side: Side,
    pub kind: TicketKind,
    pub size_text: String,
    /// A market buy states an amount to spend (in the quote asset) rather than a size to buy.
    pub spend_in_quote: bool,
    pub price_text: String,
    pub stop_text: String,
    pub tif: TicketTif,
    pub post_only: bool,
}

impl TicketDraft {
    pub fn new(side: Side) -> Self {
        Self {
            side,
            kind: TicketKind::Limit,
            size_text: String::new(),
            spend_in_quote: false,
            price_text: String::new(),
            stop_text: String::new(),
            tif: TicketTif::GoodTilCancelled,
            post_only: false,
        }
    }

    /// A ticket opened at a clicked price on the chart.
    #[allow(dead_code)] // The chart's menu opens it next.
    pub fn at_price(side: Side, kind: TicketKind, price: Decimal) -> Self {
        let mut draft = Self::new(side);
        draft.kind = kind;
        let text = price.normalize().to_string();
        match kind {
            TicketKind::Market => {}
            TicketKind::Limit => draft.price_text = text,
            TicketKind::StopLimit => {
                draft.stop_text = text.clone();
                draft.price_text = text;
            }
        }
        draft
    }
}

/// What the ticket knows about the market and the account.
pub struct TicketContext<'a> {
    pub spec: &'a ProductSpec,
    pub caps: &'a Capabilities,
    pub quote: Option<&'a Quote>,
    pub balances: &'a [Balance],
    pub fees: Option<&'a FeeSummary>,
    pub now_ms: i64,
}

/// A problem, next to the field it concerns.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TicketIssue {
    pub field: TicketField,
    pub text: String,
}

fn issue(field: TicketField, text: impl Into<String>) -> TicketIssue {
    TicketIssue {
        field,
        text: text.into(),
    }
}

/// `1234.5`, or `1234,5` when there is no `.`. Thousands separators are not accepted: a comma
/// next to a point is ambiguous, and an order must mean exactly what it says.
pub fn parse_decimal(text: &str) -> Result<Option<Decimal>, ()> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let normalised = if text.contains('.') {
        if text.contains(',') {
            return Err(());
        }
        text.to_owned()
    } else {
        text.replace(',', ".")
    };
    if normalised.matches('.').count() > 1
        || !normalised.chars().all(|c| c.is_ascii_digit() || c == '.')
        || normalised == "."
    {
        return Err(());
    }
    Decimal::from_str(&normalised).map(Some).map_err(|_| ())
}

fn number(
    text: &str,
    field: TicketField,
    what: &str,
    issues: &mut Vec<TicketIssue>,
) -> Option<Decimal> {
    match parse_decimal(text) {
        Ok(Some(value)) => Some(value),
        Ok(None) => {
            issues.push(issue(field, format!("Enter {what}.")));
            None
        }
        Err(()) => {
            issues.push(issue(field, "That is not a number."));
            None
        }
    }
}

/// The price a stop is compared with: the last trade, else the middle of the book.
fn reference_for_stop(quote: Option<&Quote>) -> Option<Decimal> {
    let quote = quote?;
    quote.last.or_else(|| match (quote.bid, quote.ask) {
        (Some(bid), Some(ask)) => Some((bid + ask) / Decimal::TWO),
        (Some(price), None) | (None, Some(price)) => Some(price),
        (None, None) => None,
    })
}

impl TicketDraft {
    /// The order this text means, or why it does not mean one yet. Only the *typed* faults are here;
    /// the market's rules come from [`TicketDraft::issues`].
    pub fn build(&self, ctx: &TicketContext) -> Result<OrderDraft, Vec<TicketIssue>> {
        let mut issues = Vec::new();
        let size = number(&self.size_text, TicketField::Size, "a size", &mut issues);
        let kind = match self.kind {
            TicketKind::Market => size.map(|size| OrderKind::Market {
                size: if self.spend_in_quote && self.side == Side::Buy {
                    OrderSize::Quote(size)
                } else {
                    OrderSize::Base(size)
                },
            }),
            TicketKind::Limit => {
                let price = number(&self.price_text, TicketField::Price, "a price", &mut issues);
                size.zip(price).map(|(base_size, price)| OrderKind::Limit {
                    base_size,
                    price,
                    tif: match self.tif {
                        TicketTif::GoodTilCancelled => TimeInForce::GoodTilCancelled,
                        TicketTif::FillOrKill => TimeInForce::FillOrKill,
                    },
                    post_only: self.post_only,
                })
            }
            TicketKind::StopLimit => {
                let stop = number(
                    &self.stop_text,
                    TicketField::StopPrice,
                    "a stop price",
                    &mut issues,
                );
                let limit = number(
                    &self.price_text,
                    TicketField::LimitPrice,
                    "a limit price",
                    &mut issues,
                );
                match (size, stop, limit) {
                    (Some(base_size), Some(stop_price), Some(limit_price)) => {
                        match reference_for_stop(ctx.quote) {
                            Some(reference) => Some(OrderKind::StopLimit {
                                base_size,
                                stop_price,
                                limit_price,
                                direction: StopDirection::for_stop(
                                    self.side, stop_price, reference,
                                ),
                                tif: TimeInForce::GoodTilCancelled,
                            }),
                            None => {
                                issues.push(issue(
                                    TicketField::StopPrice,
                                    "There is no current price to compare the stop with yet.",
                                ));
                                None
                            }
                        }
                    }
                    _ => None,
                }
            }
        };
        match kind {
            Some(kind) if issues.is_empty() => Ok(OrderDraft {
                product: ctx.spec.id.clone(),
                side: self.side,
                kind,
            }),
            _ => Err(issues),
        }
    }

    /// Everything wrong with the ticket as typed, field by field: what does not parse, then what
    /// the market's rules refuse, then (as a note on the size) what the account cannot afford.
    pub fn issues(&self, ctx: &TicketContext) -> Vec<TicketIssue> {
        let draft = match self.build(ctx) {
            Ok(draft) => draft,
            Err(issues) => return issues,
        };
        let mut issues: Vec<TicketIssue> =
            validate::validate_draft(ctx.spec, ctx.caps, &draft, ctx.now_ms)
                .into_iter()
                .map(|found| {
                    let field = match found.field {
                        Field::Size => TicketField::Size,
                        Field::Price => TicketField::Price,
                        Field::StopPrice => TicketField::StopPrice,
                        Field::LimitPrice => TicketField::LimitPrice,
                        Field::TimeInForce => TicketField::TimeInForce,
                        Field::Type | Field::Product => TicketField::Type,
                    };
                    TicketIssue {
                        field,
                        text: found.to_string(),
                    }
                })
                .collect();
        if issues.is_empty()
            && let Some(short) =
                validate::funds_shortfall(ctx.spec, &draft, ctx.balances, self.reference_price(ctx))
        {
            issues.push(issue(
                TicketField::Size,
                format!(
                    "You have {} {} available; this needs {}{}.",
                    display(short.available),
                    short.asset,
                    if short.estimated { "about " } else { "" },
                    display(short.needed),
                ),
            ));
        }
        issues
    }

    /// The price a market order is expected to trade at: the ask for a buy, the bid for a sell.
    fn reference_price(&self, ctx: &TicketContext) -> Option<Decimal> {
        match self.side {
            Side::Buy => ctx.quote.and_then(|q| q.ask),
            Side::Sell => ctx.quote.and_then(|q| q.bid),
        }
    }

    #[allow(dead_code)] // The chart's quick order buttons ask it next.
    pub fn can_submit(&self, ctx: &TicketContext) -> bool {
        self.issues(ctx).is_empty() && self.build(ctx).is_ok()
    }

    /// What the order would cost, as far as it can be known before the broker's preview.
    pub fn estimate(&self, ctx: &TicketContext) -> Estimate {
        let Ok(draft) = self.build(ctx) else {
            return Estimate::default();
        };
        let reference = match &draft.kind {
            OrderKind::Market { .. } => self.reference_price(ctx),
            OrderKind::Limit { price, .. } => Some(*price),
            OrderKind::StopLimit { limit_price, .. } => Some(*limit_price),
        };
        let notional = match &draft.kind {
            OrderKind::Market {
                size: OrderSize::Quote(amount),
            } => Some(*amount),
            _ => draft
                .base_size()
                .zip(reference)
                .map(|(size, price)| size * price),
        };
        // A resting post-only order pays the maker rate; anything that may take liquidity pays the
        // taker rate, which is the safer figure to show.
        let rate = ctx.fees.and_then(|fees| {
            if self.post_only && self.kind == TicketKind::Limit {
                fees.maker_rate
            } else {
                fees.taker_rate
            }
        });
        Estimate {
            reference,
            notional,
            fee_rate: rate,
            fee: notional.zip(rate).map(|(notional, rate)| notional * rate),
        }
    }
}

/// What an order would cost, estimated from the fee schedule.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Estimate {
    /// The price the figures assume.
    pub reference: Option<Decimal>,
    /// The order's value in the quote asset.
    pub notional: Option<Decimal>,
    pub fee_rate: Option<Decimal>,
    pub fee: Option<Decimal>,
}

/// A figure for people: grouped thousands, no trailing zeros.
pub fn display(value: Decimal) -> String {
    group_thousands(&value.normalize().to_string())
}

/// A price with at least the market's price precision (`90,000.00`, not `90,000`), never fewer
/// digits than the value carries.
pub fn price_at(value: Decimal, increment: Decimal) -> String {
    let mut shown = value;
    let places = increment.normalize().scale();
    if shown.scale() < places {
        shown.rescale(places);
    }
    group_thousands(&shown.to_string())
}

/// [`price_at`] in a market's price step.
pub fn price_display(value: Decimal, spec: &ProductSpec) -> String {
    price_at(value, spec.price_increment)
}

/// Groups the whole part of a plain decimal in thousands, keeping every digit it carries.
fn group_thousands(text: &str) -> String {
    let (sign, rest) = text
        .strip_prefix('-')
        .map_or(("", text), |rest| ("-", rest));
    let (whole, fraction) = rest
        .split_once('.')
        .map_or((rest, None), |(whole, fraction)| (whole, Some(fraction)));
    let mut grouped = String::new();
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    match fraction {
        Some(fraction) => format!("{sign}{grouped}.{fraction}"),
        None => format!("{sign}{grouped}"),
    }
}

/// How long the broker's answer to a preview may be acted on. A preview is a statement about the
/// market a moment ago; after this it is asked for again.
pub const PREVIEW_FRESH: Duration = Duration::from_secs(60);

/// How long typing must pause before the broker is asked to preview what is typed.
pub const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(450);

/// What the broker has said about the order as it stands, from the ticket's or the edit sheet's
/// point of view.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreviewAnswer {
    /// Not asked yet, or asked and not answered.
    Waiting,
    /// The question could not be answered (the connection, a refusal of the preview itself).
    Failed(String),
    /// The broker answered that it would refuse the order.
    Problems,
    /// The broker answered that it would accept the order.
    Accepted,
}

/// Whether the order may be reviewed. **Only a fresh, accepting answer from the broker allows it**:
/// no answer, a failed answer and an old answer do not. (The broker's create endpoint does not
/// require a preview, so this is this application's rule, not the broker's.)
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreviewGate {
    Waiting,
    Failed(String),
    Blocked,
    Stale,
    Ready,
}

impl PreviewGate {
    pub fn allows_review(&self) -> bool {
        *self == Self::Ready
    }
}

/// `age` is how long ago the accepting answer arrived.
pub fn preview_gate(answer: PreviewAnswer, age: Option<Duration>) -> PreviewGate {
    match answer {
        PreviewAnswer::Waiting => PreviewGate::Waiting,
        PreviewAnswer::Failed(text) => PreviewGate::Failed(text),
        PreviewAnswer::Problems => PreviewGate::Blocked,
        PreviewAnswer::Accepted => match age {
            Some(age) if age <= PREVIEW_FRESH => PreviewGate::Ready,
            _ => PreviewGate::Stale,
        },
    }
}

/// What the user is told when something happens to an order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoticeText {
    pub title: String,
    pub body: String,
    pub error: bool,
    /// A stable id, so a repeat replaces rather than stacks.
    pub identifier: String,
}

pub fn notice_text(notice: &Notice, broker: &str, spec: Option<&ProductSpec>) -> NoticeText {
    let order: &LiveOrder = &notice.order;
    let side = match order.side {
        Side::Buy => "Buy",
        Side::Sell => "Sell",
    };
    let base = spec.map_or("", |s| s.base.as_str());
    let size = match order.size {
        Some(OrderSize::Base(size)) => format!("{} {base}", display(size)).trim().to_owned(),
        Some(OrderSize::Quote(amount)) => format!(
            "{} {}",
            display(amount),
            spec.map_or("", |s| s.quote.as_str())
        )
        .trim()
        .to_owned(),
        None => String::new(),
    };
    let price = order
        .average_price
        .or(order.limit_price)
        .map_or(String::new(), |p| format!(" at {}", display(p)));
    let what = format!("{side} {size} {}{price} on {broker}", order.product).replace("  ", " ");
    let (title, error) = match notice.kind {
        NoticeKind::Accepted => ("Order placed", false),
        NoticeKind::PartiallyFilled => ("Order partly filled", false),
        NoticeKind::Filled => ("Order filled", false),
        NoticeKind::Cancelled => ("Order cancelled", false),
        NoticeKind::Rejected => ("Order rejected", true),
        NoticeKind::Expired => ("Order expired", false),
    };
    let body = match (&order.reason, notice.kind) {
        (Some(reason), NoticeKind::Rejected | NoticeKind::Cancelled) => format!("{what}. {reason}"),
        (_, NoticeKind::PartiallyFilled) => {
            format!("{what}: {} filled.", display(order.filled_size))
        }
        _ => what,
    };
    NoticeText {
        title: title.to_owned(),
        body,
        error,
        identifier: format!(
            "broker-{}-{:?}-{}",
            order.id,
            notice.kind,
            if notice.kind == NoticeKind::PartiallyFilled {
                order.filled_size.normalize().to_string()
            } else {
                String::new()
            }
        ),
    }
}

/// How an order's status reads.
pub fn status_label(status: OrderStatus) -> (&'static str, StatusTone) {
    match status {
        OrderStatus::Pending => ("Pending", StatusTone::Neutral),
        OrderStatus::Open => ("Open", StatusTone::Good),
        OrderStatus::CancelQueued => ("Cancelling", StatusTone::Warning),
        OrderStatus::EditQueued => ("Updating", StatusTone::Warning),
        OrderStatus::Filled => ("Filled", StatusTone::Good),
        OrderStatus::Cancelled => ("Cancelled", StatusTone::Neutral),
        OrderStatus::Expired => ("Expired", StatusTone::Neutral),
        OrderStatus::Failed => ("Rejected", StatusTone::Bad),
        OrderStatus::Unknown => ("Unknown", StatusTone::Warning),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusTone {
    Good,
    Warning,
    Bad,
    Neutral,
}

/// Why an order cannot be edited, or `None` when it can.
pub fn edit_blocker(
    order: &LiveOrder,
    caps: Option<&Capabilities>,
    cancelling: bool,
) -> Option<&'static str> {
    if order.status != OrderStatus::Open {
        return Some("Only open orders can be edited.");
    }
    if cancelling {
        return Some("A cancel is in progress for this order.");
    }
    if !matches!(order.order_type, OrderType::Limit | OrderType::StopLimit) {
        return Some("This type of order cannot be edited.");
    }
    if matches!(order.size, Some(OrderSize::Quote(_))) || order.size.is_none() {
        return Some("Orders sized by an amount to spend cannot be edited.");
    }
    match caps {
        Some(caps) if caps.native_edit => None,
        Some(_) => Some("This broker does not support editing this market's orders."),
        None => Some("The market's rules have not been read yet."),
    }
}

/// An order's size as the lists show it: the base amount, or the amount to spend.
pub fn size_text(order: &LiveOrder) -> String {
    match order.size {
        Some(OrderSize::Base(size)) => display(size),
        Some(OrderSize::Quote(amount)) => format!("{} (spend)", display(amount)),
        None => "—".to_owned(),
    }
}

/// The type column.
pub fn type_label(order: &LiveOrder) -> String {
    match order.order_type {
        OrderType::Market => "Market".to_owned(),
        OrderType::Limit if order.post_only => "Limit, post-only".to_owned(),
        OrderType::Limit => "Limit".to_owned(),
        OrderType::StopLimit => "Stop-limit".to_owned(),
        OrderType::Other => "Other".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use degenview_brokers::{OrderId, ProductId, Restrictions, Side, TifKind, Tradability};

    fn d(text: &str) -> Decimal {
        Decimal::from_str(text).unwrap()
    }

    const NOW: i64 = 1_800_000_000_000;

    fn spec() -> ProductSpec {
        ProductSpec {
            id: ProductId::new("BTC-USD"),
            base: "BTC".to_owned(),
            quote: "USD".to_owned(),
            display_name: "BTC-USD".to_owned(),
            price_increment: d("0.01"),
            base_increment: d("0.00000001"),
            quote_increment: d("0.01"),
            base_min: d("0.00001"),
            base_max: Some(d("1000")),
            quote_min: d("1"),
            quote_max: None,
            restrictions: Restrictions::default(),
            fetched_ms: NOW,
        }
    }

    fn caps() -> Capabilities {
        Capabilities {
            tradable: Tradability::Yes,
            order_types: vec![OrderType::Market, OrderType::Limit, OrderType::StopLimit],
            tif: vec![TifKind::GoodTilCancelled, TifKind::FillOrKill],
            stop_limit_tif: vec![TifKind::GoodTilCancelled],
            post_only: true,
            quote_sized_market: true,
            native_edit: true,
            atomic_replace: false,
            brackets: false,
        }
    }

    fn quote() -> Quote {
        Quote {
            bid: Some(d("89999.50")),
            ask: Some(d("90000.50")),
            last: Some(d("90000.00")),
            time_ms: NOW,
        }
    }

    fn balances() -> Vec<Balance> {
        vec![Balance {
            account: degenview_brokers::AccountId::new("a"),
            asset: "USD".to_owned(),
            available: d("1000"),
            held: d("0"),
        }]
    }

    fn with<R>(f: impl FnOnce(&TicketContext) -> R) -> R {
        let (spec, caps, quote, balances) = (spec(), caps(), quote(), balances());
        let fees = FeeSummary {
            tier: "t".to_owned(),
            maker_rate: Some(d("0.004")),
            taker_rate: Some(d("0.006")),
        };
        f(&TicketContext {
            spec: &spec,
            caps: &caps,
            quote: Some(&quote),
            balances: &balances,
            fees: Some(&fees),
            now_ms: NOW,
        })
    }

    fn limit(size: &str, price: &str) -> TicketDraft {
        TicketDraft {
            size_text: size.to_owned(),
            price_text: price.to_owned(),
            ..TicketDraft::new(Side::Buy)
        }
    }

    #[test]
    fn typed_numbers_mean_exactly_what_they_say() {
        assert_eq!(parse_decimal("0.001"), Ok(Some(d("0.001"))));
        assert_eq!(parse_decimal(" 12,5 "), Ok(Some(d("12.5"))));
        assert_eq!(parse_decimal(""), Ok(None));
        for bad in ["1,234.5", "1.2.3", "abc", "-1", "1e3", ".", "1 000"] {
            assert_eq!(parse_decimal(bad), Err(()), "{bad}");
        }
        // The digits survive untouched, trailing zeros included.
        assert_eq!(
            parse_decimal("90000.10").unwrap().unwrap().to_string(),
            "90000.10"
        );
    }

    #[test]
    fn a_good_limit_order_builds_and_can_be_submitted() {
        with(|ctx| {
            let ticket = limit("0.001", "90000.00");
            assert!(ticket.issues(ctx).is_empty());
            assert!(ticket.can_submit(ctx));
            let draft = ticket.build(ctx).unwrap();
            assert_eq!(draft.working_price(), Some(d("90000.00")));
        });
    }

    #[test]
    fn missing_and_malformed_fields_are_marked_where_they_are() {
        with(|ctx| {
            let issues = limit("", "x").issues(ctx);
            assert_eq!(
                issues,
                vec![
                    issue(TicketField::Size, "Enter a size."),
                    issue(TicketField::Price, "That is not a number."),
                ]
            );
        });
    }

    #[test]
    fn the_markets_rules_arrive_on_the_field_they_concern_and_nothing_is_rounded() {
        with(|ctx| {
            let ticket = limit("0.001", "90000.005");
            let issues = ticket.issues(ctx);
            assert_eq!(issues.len(), 1);
            assert_eq!(issues[0].field, TicketField::Price);
            assert!(issues[0].text.contains("90000"), "{}", issues[0].text);
            // The text is what the user typed.
            assert_eq!(ticket.price_text, "90000.005");
            assert!(!ticket.can_submit(ctx));

            let small = limit("0.000001", "90000.00").issues(ctx);
            assert_eq!(small[0].field, TicketField::Size);
            assert!(small[0].text.contains("minimum"));
        });
    }

    #[test]
    fn not_having_the_money_is_said_on_the_size() {
        with(|ctx| {
            let issues = limit("0.5", "90000.00").issues(ctx);
            assert_eq!(issues.len(), 1);
            assert_eq!(issues[0].field, TicketField::Size);
            assert!(
                issues[0].text.contains("1,000 USD available"),
                "{}",
                issues[0].text
            );
            assert!(issues[0].text.contains("45,000"), "{}", issues[0].text);
        });
    }

    #[test]
    fn a_market_buy_can_state_an_amount_to_spend_and_a_sell_cannot() {
        with(|ctx| {
            let mut buy = TicketDraft::new(Side::Buy);
            buy.kind = TicketKind::Market;
            buy.spend_in_quote = true;
            buy.size_text = "25.00".to_owned();
            assert!(matches!(
                buy.build(ctx).unwrap().kind,
                OrderKind::Market {
                    size: OrderSize::Quote(_)
                }
            ));
            assert!(buy.can_submit(ctx));

            let mut sell = TicketDraft::new(Side::Sell);
            sell.kind = TicketKind::Market;
            sell.spend_in_quote = true; // ignored for a sell
            sell.size_text = "0.001".to_owned();
            assert!(matches!(
                sell.build(ctx).unwrap().kind,
                OrderKind::Market {
                    size: OrderSize::Base(_)
                }
            ));
        });
    }

    #[test]
    fn a_stop_limit_derives_which_way_the_stop_triggers_and_needs_a_price_to_do_so() {
        with(|ctx| {
            let mut sell = TicketDraft::new(Side::Sell);
            sell.kind = TicketKind::StopLimit;
            sell.size_text = "0.001".to_owned();
            sell.stop_text = "85000".to_owned();
            sell.price_text = "84990".to_owned();
            let draft = sell.build(ctx).unwrap();
            assert!(matches!(
                draft.kind,
                OrderKind::StopLimit {
                    direction: StopDirection::Down,
                    ..
                }
            ));
        });
        let (spec, caps) = (spec(), caps());
        let blind = TicketContext {
            spec: &spec,
            caps: &caps,
            quote: None,
            balances: &[],
            fees: None,
            now_ms: NOW,
        };
        let mut sell = TicketDraft::new(Side::Sell);
        sell.kind = TicketKind::StopLimit;
        sell.size_text = "0.001".to_owned();
        sell.stop_text = "85000".to_owned();
        sell.price_text = "84990".to_owned();
        let issues = sell.issues(&blind);
        assert_eq!(issues[0].field, TicketField::StopPrice);
        assert!(issues[0].text.contains("no current price"));
    }

    #[test]
    fn the_estimate_uses_the_maker_rate_only_for_a_post_only_limit() {
        with(|ctx| {
            let mut ticket = limit("0.001", "90000.00");
            let taker = ticket.estimate(ctx);
            assert_eq!(taker.notional, Some(d("90")));
            assert_eq!(taker.fee, Some(d("0.54")));
            ticket.post_only = true;
            assert_eq!(ticket.estimate(ctx).fee, Some(d("0.36")));
            // A market buy is priced from the ask.
            let mut market = TicketDraft::new(Side::Buy);
            market.kind = TicketKind::Market;
            market.size_text = "0.001".to_owned();
            assert_eq!(market.estimate(ctx).notional, Some(d("90.0005")));
            // Nothing is guessed from a ticket that does not parse yet.
            assert_eq!(limit("", "").estimate(ctx), Estimate::default());
        });
    }

    #[test]
    fn only_a_fresh_accepting_answer_from_the_broker_allows_a_review() {
        let fresh = Some(Duration::from_secs(5));
        assert_eq!(
            preview_gate(PreviewAnswer::Waiting, None),
            PreviewGate::Waiting
        );
        assert_eq!(
            preview_gate(PreviewAnswer::Failed("down".to_owned()), None),
            PreviewGate::Failed("down".to_owned())
        );
        assert_eq!(
            preview_gate(PreviewAnswer::Problems, fresh),
            PreviewGate::Blocked
        );
        assert_eq!(
            preview_gate(PreviewAnswer::Accepted, fresh),
            PreviewGate::Ready
        );
        assert_eq!(
            preview_gate(
                PreviewAnswer::Accepted,
                Some(PREVIEW_FRESH + Duration::from_secs(1))
            ),
            PreviewGate::Stale,
            "an old answer is not an answer"
        );
        assert_eq!(
            preview_gate(PreviewAnswer::Accepted, None),
            PreviewGate::Stale,
            "an answer of unknown age is not fresh"
        );
        for gate in [
            PreviewGate::Waiting,
            PreviewGate::Failed("x".to_owned()),
            PreviewGate::Blocked,
            PreviewGate::Stale,
        ] {
            assert!(!gate.allows_review(), "{gate:?}");
        }
        assert!(PreviewGate::Ready.allows_review());
    }

    #[test]
    fn figures_group_thousands_without_inventing_digits() {
        assert_eq!(display(d("1234567.8900")), "1,234,567.89");
        assert_eq!(display(d("-1234")), "-1,234");
        assert_eq!(display(d("0.00000100")), "0.000001");
        assert_eq!(display(d("999")), "999");
        assert_eq!(display(d("1000")), "1,000");
    }

    #[test]
    fn an_order_clicked_on_the_chart_opens_at_that_price() {
        let ticket = TicketDraft::at_price(Side::Sell, TicketKind::Limit, d("91000.50"));
        assert_eq!(ticket.price_text, "91000.5");
        let stop = TicketDraft::at_price(Side::Sell, TicketKind::StopLimit, d("85000"));
        assert_eq!(
            (stop.stop_text.as_str(), stop.price_text.as_str()),
            ("85000", "85000")
        );
        assert!(
            TicketDraft::at_price(Side::Buy, TicketKind::Market, d("1"))
                .price_text
                .is_empty()
        );
    }

    #[test]
    fn notices_are_plain_sentences_and_a_rejection_carries_the_reason() {
        use degenview_brokers::{LiveOrder, OrderId};
        let order = |status, reason: Option<&str>| LiveOrder {
            id: OrderId::new("o1"),
            client_id: None,
            product: ProductId::new("BTC-USD"),
            side: Side::Buy,
            order_type: OrderType::Limit,
            status,
            tif: None,
            size: Some(OrderSize::Base(d("0.001"))),
            limit_price: Some(d("90000")),
            stop_price: None,
            stop_direction: None,
            post_only: false,
            filled_size: d("0.0004"),
            filled_value: d("0"),
            average_price: None,
            fees: d("0"),
            created_ms: None,
            updated_ms: None,
            reason: reason.map(str::to_owned),
            stop_triggered: false,
        };
        let spec = spec();
        let text = notice_text(
            &Notice {
                kind: NoticeKind::Filled,
                order: order(OrderStatus::Filled, None),
            },
            "Acme",
            Some(&spec),
        );
        assert_eq!(text.title, "Order filled");
        assert_eq!(text.body, "Buy 0.001 BTC BTC-USD at 90,000 on Acme");
        assert!(!text.error);
        let rejected = notice_text(
            &Notice {
                kind: NoticeKind::Rejected,
                order: order(OrderStatus::Failed, Some("Not enough USD")),
            },
            "Acme",
            Some(&spec),
        );
        assert!(rejected.error);
        assert!(rejected.body.ends_with("Not enough USD"));
        let partial = notice_text(
            &Notice {
                kind: NoticeKind::PartiallyFilled,
                order: order(OrderStatus::Open, None),
            },
            "Acme",
            None,
        );
        assert!(partial.body.contains("0.0004 filled"));
        // The same event gives the same identifier (a replay replaces), a later partial a new one.
        let again = notice_text(
            &Notice {
                kind: NoticeKind::Filled,
                order: order(OrderStatus::Filled, None),
            },
            "Acme",
            Some(&spec),
        );
        assert_eq!(again.identifier, text.identifier);
        assert_ne!(partial.identifier, text.identifier);
    }

    #[test]
    fn every_status_has_words() {
        for status in [
            OrderStatus::Pending,
            OrderStatus::Open,
            OrderStatus::CancelQueued,
            OrderStatus::EditQueued,
            OrderStatus::Filled,
            OrderStatus::Cancelled,
            OrderStatus::Expired,
            OrderStatus::Failed,
            OrderStatus::Unknown,
        ] {
            assert!(!status_label(status).0.is_empty());
        }
        assert_eq!(status_label(OrderStatus::CancelQueued).0, "Cancelling");
    }

    fn live_order(status: OrderStatus, kind: OrderType, size: Option<OrderSize>) -> LiveOrder {
        LiveOrder {
            id: OrderId::new("o"),
            client_id: None,
            product: ProductId::new("BTC-USD"),
            side: Side::Buy,
            order_type: kind,
            status,
            tif: None,
            size,
            limit_price: Some(d("100")),
            stop_price: None,
            stop_direction: None,
            post_only: false,
            filled_size: d("0"),
            filled_value: d("0"),
            average_price: None,
            fees: d("0"),
            created_ms: Some(0),
            updated_ms: None,
            reason: None,
            stop_triggered: false,
        }
    }

    fn edit_caps(native_edit: bool) -> Capabilities {
        Capabilities {
            tradable: Tradability::Yes,
            order_types: vec![OrderType::Limit],
            tif: vec![TifKind::GoodTilCancelled],
            stop_limit_tif: vec![],
            post_only: false,
            quote_sized_market: false,
            native_edit,
            atomic_replace: false,
            brackets: false,
        }
    }

    #[test]
    fn only_an_open_priced_base_sized_order_with_native_edit_can_be_edited() {
        let open = live_order(
            OrderStatus::Open,
            OrderType::Limit,
            Some(OrderSize::Base(d("1"))),
        );
        assert_eq!(edit_blocker(&open, Some(&edit_caps(true)), false), None);
        assert!(
            edit_blocker(&open, Some(&edit_caps(false)), false)
                .unwrap()
                .contains("does not support")
        );
        assert!(
            edit_blocker(&open, None, false)
                .unwrap()
                .contains("not been read")
        );
        assert!(
            edit_blocker(&open, Some(&edit_caps(true)), true)
                .unwrap()
                .contains("cancel is in progress")
        );
        let queued = live_order(
            OrderStatus::CancelQueued,
            OrderType::Limit,
            Some(OrderSize::Base(d("1"))),
        );
        assert!(
            edit_blocker(&queued, Some(&edit_caps(true)), false)
                .unwrap()
                .contains("Only open")
        );
        let market = live_order(
            OrderStatus::Open,
            OrderType::Other,
            Some(OrderSize::Base(d("1"))),
        );
        assert!(
            edit_blocker(&market, Some(&edit_caps(true)), false)
                .unwrap()
                .contains("type of order")
        );
        let spend = live_order(
            OrderStatus::Open,
            OrderType::Limit,
            Some(OrderSize::Quote(d("10"))),
        );
        assert!(
            edit_blocker(&spend, Some(&edit_caps(true)), false)
                .unwrap()
                .contains("amount to spend")
        );
    }

    #[test]
    fn rows_say_what_they_are() {
        let mut order = live_order(
            OrderStatus::Open,
            OrderType::Limit,
            Some(OrderSize::Base(d("0.5"))),
        );
        order.post_only = true;
        assert_eq!(type_label(&order), "Limit, post-only");
        assert_eq!(size_text(&order), "0.5");
        assert_eq!(
            size_text(&live_order(
                OrderStatus::Open,
                OrderType::Market,
                Some(OrderSize::Quote(d("25")))
            )),
            "25 (spend)"
        );
    }
}
