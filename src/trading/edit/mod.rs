//! Editing an open order in place, as the Edit sheet shows it.
//!
//! The rules and the state machine are the crate's (`degenview_brokers::edit_session`): a change is
//! sent only after the broker's preview of exactly that change, on exactly that order, on this
//! connection, while that preview is fresh; a review stands only while the order is unchanged; the
//! amend carries the reviewed order's fingerprint. This module is what is left on this side: the two
//! text boxes, the words for every reason the crate reports as a value, and the [`EditView`] the
//! sheet draws. Nothing here draws or waits.

use std::time::Instant;

use degenview_brokers::edit_session::{
    self as session, Closing, EditNote, EditSource, Entry, NotAChange, NotANumber,
};
use degenview_brokers::model::edit::EditIssue;
use degenview_brokers::model::orders::{EditPreview, EditWorld};
use degenview_brokers::model::preview::PreviewGate;
use degenview_brokers::{
    AccountId, Decimal, LiveOrder, OrderId, OrderSize, OrderType, ProductSpec, Side,
};

use crate::model::broker::ticket::{self as rules};

/// What the venue does for an edit: the crate's [`EditSource`] (preview and amend), and the order's
/// world to read each frame. Implemented by a connected broker.
pub trait EditVenue: EditSource {
    /// The order with this id and the account, connection and market rules around it.
    fn world(&self, order: &OrderId) -> EditWorld;
}

/// What the venue says about the world this frame.
pub struct EditFrame<'a> {
    pub now: Instant,
    /// The order as the venue has it now; `None` once it is gone.
    pub live: Option<&'a LiveOrder>,
    /// The account the orders go to; `None` while there is none.
    pub account: Option<&'a AccountId>,
    /// Which connection the account is read on; a preview says nothing about another one.
    pub generation: u64,
    pub spec: Option<&'a ProductSpec>,
}

pub use session::EditEvent;

/// A message for the user once the sheet has closed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Notice {
    pub text: String,
    pub error: bool,
}

#[derive(Debug)]
pub enum Step {
    Open,
    Closed(Option<Notice>),
}

pub struct EditSession {
    inner: session::EditSession,
    price_text: String,
    size_text: String,
    /// What the market's rules say is wrong with the change, in words, as of the last frame.
    problems: Vec<String>,
    /// The last error shown on the sheet.
    error: Option<String>,
}

impl EditSession {
    /// Starts editing `order`; `price` is where a dragged order line was dropped, if it was.
    pub fn new(order: LiveOrder, price: Option<Decimal>, now: Instant) -> Self {
        let price_text = price
            .or(order.limit_price)
            .map(|p| p.to_string())
            .unwrap_or_default();
        let size_text = match order.size {
            Some(OrderSize::Base(size)) => size.to_string(),
            _ => String::new(),
        };
        Self {
            inner: session::EditSession::new(order, now),
            price_text,
            size_text,
            problems: Vec::new(),
            error: None,
        }
    }

    /// The two text boxes: price (the stop price of a stop-limit order) and size.
    pub fn fields_mut(&mut self) -> (&mut String, &mut String) {
        (&mut self.price_text, &mut self.size_text)
    }

    /// The order as the venue last reported it.
    pub fn order(&self) -> &LiveOrder {
        self.inner.order()
    }

    #[cfg(test)]
    pub fn is_sending(&self) -> bool {
        self.inner.is_sending()
    }

    #[cfg(test)]
    pub fn is_confirming(&self) -> bool {
        self.inner.is_confirming()
    }

    /// Whether the broker's accepting answer for what is typed has arrived.
    #[cfg(test)]
    pub fn is_ready(&self) -> bool {
        self.inner.is_ready()
    }

    /// Whether "Review change" was enabled in the last frame.
    #[cfg(test)]
    pub fn can_review(&self) -> bool {
        self.inner.state().can_review
    }

    #[cfg(test)]
    pub fn preview_failed(&self) -> bool {
        self.inner.preview_failed()
    }

    /// The filled size of the order, as the sheet shows it.
    #[cfg(test)]
    pub fn filled_size(&self) -> Decimal {
        self.inner.order().filled_size
    }

    #[cfg(test)]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn entry(&self) -> Entry {
        let read = |text: &str| rules::parse_decimal(text).map_err(|()| NotANumber);
        Entry {
            price: read(&self.price_text),
            size: read(&self.size_text),
        }
    }

    /// Reconciles the sheet with the world for one frame and keeps the broker's preview current.
    pub fn update(&mut self, frame: &EditFrame, venue: Option<&dyn EditVenue>) -> Step {
        let Some(venue) = venue else {
            return Step::Closed(None);
        };
        let crate_frame = session::EditFrame {
            now: frame.now,
            live: frame.live,
            account: frame.account,
            generation: frame.generation,
            spec: frame.spec,
        };
        let step = self.inner.update(&crate_frame, &self.entry(), venue);
        if let session::Step::Closed(closing) = step {
            return Step::Closed(closing_notice(&closing));
        }
        let state = self.inner.state();
        self.problems = state
            .issues
            .iter()
            .filter_map(|issue| frame.spec.map(|spec| issue_text(issue, spec)))
            .collect();
        self.error = state.note.map(note_text);
        Step::Open
    }

    /// What the user pressed.
    pub fn handle(&mut self, event: EditEvent, now: Instant, venue: &dyn EditVenue) -> Step {
        match self.inner.handle(event, now, venue) {
            session::Step::Open => {
                self.error = self.inner.state().note.map(note_text);
                Step::Open
            }
            session::Step::Closed(closing) => Step::Closed(closing_notice(&closing)),
        }
    }

    /// What to draw for the last frame.
    pub fn view(&self) -> EditView {
        let state = self.inner.state();
        if let Some(review) = &state.review {
            return EditView::Reviewing(review_lines(state.order, review.change, review.preview));
        }
        let order = state.order;
        EditView::Editing(EditForm {
            heading: format!(
                "{} {} {} · {}",
                if order.side == Side::Buy {
                    "Buy"
                } else {
                    "Sell"
                },
                rules::size_text(order),
                order.product,
                rules::type_label(order)
            ),
            price_title: if order.order_type == OrderType::StopLimit {
                "Stop price"
            } else {
                "Limit price"
            },
            typed_error: match state.change {
                Err(NotAChange::PriceNotANumber) => Some("The price is not a number.".to_owned()),
                Err(NotAChange::SizeNotANumber) => Some("The size is not a number.".to_owned()),
                // Unchanged is not an error, only nothing to review.
                Err(NotAChange::Unchanged) | Ok(_) => None,
            },
            problems: self.problems.clone(),
            broker_problems: state
                .preview
                .map(|preview| {
                    preview
                        .problems
                        .iter()
                        .map(|problem| problem.text.clone())
                        .collect()
                })
                .unwrap_or_default(),
            broker_total: state
                .preview
                .filter(|preview| preview.is_acceptable())
                .and_then(|preview| preview.order_total)
                .map(rules::display),
            asking: state.asking,
            gate: state.gate.clone(),
            error: self.error.clone(),
            can_review: state.can_review,
            sending: state.sending,
        })
    }
}

fn closing_notice(closing: &Closing) -> Option<Notice> {
    let closed = |text: &str, error: bool| {
        Some(Notice {
            text: text.to_owned(),
            error,
        })
    };
    match closing {
        Closing::Dismissed => None,
        Closing::NotOpen => closed("The order is no longer open, so it was not edited.", true),
        Closing::NotConnected => closed(
            "The broker is not connected, so the order was not edited.",
            true,
        ),
        Closing::Sent => closed(
            "Edit sent. The order updates when the broker confirms it.",
            false,
        ),
        Closing::OutcomeUnknown(error) => closed(
            &format!("{error} The order is being read again to see what happened."),
            true,
        ),
    }
}

fn note_text(note: &EditNote) -> String {
    match note {
        EditNote::NotChecked => "This change has not been checked.".to_owned(),
        EditNote::OrderChanged => {
            "The order changed while you were reviewing it. Nothing was sent. Review it again."
                .to_owned()
        }
        EditNote::PreviewOutOfDate => {
            "The broker's check of this change is out of date. Review it again.".to_owned()
        }
        EditNote::AmendFailed(error) => error.to_string(),
    }
}

/// What a market rule that a change breaks says, in words.
fn issue_text(issue: &EditIssue, spec: &ProductSpec) -> String {
    match issue {
        EditIssue::PriceOffIncrement {
            increment,
            nearest_below,
        } => format!(
            "The price must be a multiple of {}. The nearest valid price below is {}.",
            rules::display(*increment),
            rules::price_display(*nearest_below, spec),
        ),
        EditIssue::SizeOffIncrement { increment } => {
            format!(
                "The size must be a multiple of {}.",
                rules::display(*increment)
            )
        }
        EditIssue::SizeBelowMinimum { minimum } => {
            format!(
                "The size is below the minimum of {}.",
                rules::display(*minimum)
            )
        }
        EditIssue::SizeBelowFilled => {
            "The size cannot be less than what has already filled.".to_owned()
        }
    }
}

fn review_lines(
    order: &LiveOrder,
    change: &degenview_brokers::model::orders::OrderChange,
    preview: &EditPreview,
) -> Vec<ReviewLine> {
    let mut lines = vec![
        ReviewLine::new("Market", order.product.to_string()),
        ReviewLine::new(
            "Action",
            if order.side == Side::Buy {
                "Buy"
            } else {
                "Sell"
            }
            .to_owned(),
        ),
    ];
    if let Some(new) = change.price.or(change.stop_price) {
        let old = if change.price.is_some() {
            order.limit_price
        } else {
            order.stop_price
        };
        lines.push(ReviewLine::new(
            "Price",
            format!(
                "{} → {}",
                old.map_or("—".to_owned(), rules::display),
                rules::display(new)
            ),
        ));
    }
    if let Some(new) = change.size {
        lines.push(ReviewLine::new(
            "Size",
            format!("{} → {}", rules::size_text(order), rules::display(new)),
        ));
    }
    // The broker's figures for the changed order, from a preview of exactly this change.
    if let Some(total) = preview.order_total {
        lines.push(ReviewLine::new(
            "Total (broker's estimate)",
            rules::display(total),
        ));
    }
    if let Some(fee) = preview.fees {
        lines.push(ReviewLine::new(
            "Fee (broker's estimate)",
            rules::display(fee),
        ));
    }
    lines
}

/// What the Edit sheet shows.
pub enum EditView {
    Editing(EditForm),
    /// The change laid out for approval.
    Reviewing(Vec<ReviewLine>),
}

pub struct EditForm {
    /// "Sell 0.5 BTC-USD · Limit".
    pub heading: String,
    pub price_title: &'static str,
    /// Why what is typed is not a change (not a number), unless it is simply unchanged.
    pub typed_error: Option<String>,
    /// What the market's rules say is wrong with the change.
    pub problems: Vec<String>,
    /// What the broker's preview says is wrong with it.
    pub broker_problems: Vec<String>,
    /// The broker's figure for the changed order, when it would accept it.
    pub broker_total: Option<String>,
    /// Waiting on the broker's preview of a valid change.
    pub asking: bool,
    pub gate: PreviewGate,
    pub error: Option<String>,
    pub can_review: bool,
    pub sending: bool,
}

pub struct ReviewLine {
    pub label: &'static str,
    pub value: String,
}

impl ReviewLine {
    fn new(label: &'static str, value: String) -> Self {
        Self { label, value }
    }
}

#[cfg(test)]
mod tests;
