//! The words and figures of a live broker's order ticket: how a number, a price, a status, a
//! notice and each reason an order cannot go ahead read. The rules themselves (what the typed text
//! means, what is wrong with it, what it would cost) are the crate's `model::ticket`, and each
//! reason it reports is a value; this module says them. Exact: nothing is rounded to read better.

use degenview_brokers::model::edit::{EditBlock, edit_block};
use degenview_brokers::model::reducer::{Notice, NoticeKind};
use degenview_brokers::model::ticket::{TicketField, TicketIssue, TicketProblem};
use degenview_brokers::{
    Capabilities, Decimal, LiveOrder, OrderSize, OrderStatus, OrderType, ProductSpec, Side,
};

/// What a problem with the typed ticket says, next to the field it concerns.
pub fn issue_text(issue: &TicketIssue) -> String {
    match &issue.problem {
        TicketProblem::Missing => format!(
            "Enter {}.",
            match issue.field {
                TicketField::Size => "a size",
                TicketField::Price => "a price",
                TicketField::StopPrice => "a stop price",
                TicketField::LimitPrice => "a limit price",
                TicketField::TimeInForce => "a time in force",
                TicketField::Type => "an order type",
            }
        ),
        TicketProblem::NotANumber => "That is not a number.".to_owned(),
        TicketProblem::NoStopReference => {
            "There is no current price to compare the stop with yet.".to_owned()
        }
        TicketProblem::Rule(found) => found.to_string(),
        TicketProblem::Funds(short) => format!(
            "You have {} {} available; this needs {}{}.",
            display(short.available),
            short.asset,
            if short.estimated { "about " } else { "" },
            display(short.needed),
        ),
    }
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

/// Why an order cannot be edited, or `None` when it can. The rule is the crate's
/// (`model::edit::edit_block`); these are the words.
pub fn edit_blocker(
    order: &LiveOrder,
    caps: Option<&Capabilities>,
    cancelling: bool,
) -> Option<&'static str> {
    edit_block(order, caps, cancelling).map(|block| match block {
        EditBlock::NotOpen => "Only open orders can be edited.",
        EditBlock::Cancelling => "A cancel is in progress for this order.",
        EditBlock::TypeNotEditable => "This type of order cannot be edited.",
        EditBlock::SizedByAmount => "Orders sized by an amount to spend cannot be edited.",
        EditBlock::NotSupported => "This broker does not support editing this market's orders.",
        EditBlock::RulesNotRead => "The market's rules have not been read yet.",
    })
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
    use degenview_brokers::model::validate::{Field, Issue, IssueKind, Shortfall};
    use degenview_brokers::{OrderId, ProductId, Restrictions, Side, TifKind, Tradability};
    use std::str::FromStr;

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

    fn said(field: TicketField, problem: TicketProblem) -> String {
        issue_text(&TicketIssue { field, problem })
    }

    #[test]
    fn each_problem_with_the_ticket_is_said_next_to_its_field() {
        assert_eq!(
            said(TicketField::Size, TicketProblem::Missing),
            "Enter a size."
        );
        assert_eq!(
            said(TicketField::StopPrice, TicketProblem::Missing),
            "Enter a stop price."
        );
        assert_eq!(
            said(TicketField::LimitPrice, TicketProblem::Missing),
            "Enter a limit price."
        );
        assert_eq!(
            said(TicketField::Price, TicketProblem::NotANumber),
            "That is not a number."
        );
        assert!(
            said(TicketField::StopPrice, TicketProblem::NoStopReference)
                .contains("no current price")
        );
        let off = Issue {
            field: Field::Price,
            kind: IssueKind::OffIncrement {
                increment: d("0.01"),
                nearest_below: d("90000"),
            },
        };
        assert!(said(TicketField::Price, TicketProblem::Rule(off)).contains("90000"));
        let short = Shortfall {
            asset: "USD".to_owned(),
            needed: d("45000"),
            available: d("1000"),
            estimated: false,
        };
        let text = said(TicketField::Size, TicketProblem::Funds(short.clone()));
        assert_eq!(text, "You have 1,000 USD available; this needs 45,000.");
        let guess = Shortfall {
            estimated: true,
            ..short
        };
        assert!(
            said(TicketField::Size, TicketProblem::Funds(guess)).contains("needs about 45,000")
        );
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
