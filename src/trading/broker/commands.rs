//! What a connected broker is asked besides taking an order: look an unconfirmed order up again,
//! cancel, read older orders and fills, and (through [`EditVenue`]) change an open order in place.
//!
//! The screens hold this trait, never the broker's runtime: every call is a [`Task`] they poll on
//! later frames, and a call made while the broker is not connected answers with an error at once.

use degenview_brokers::model::orders::{CancelOutcome, CancelResult};

use crate::trading::edit::EditVenue;
use crate::trading::model::{FillRow, OrderRow, Page};
use crate::trading::task::Task;

/// A request that answers once, in words when it fails.
pub type Answer<T> = Task<Result<T, String>>;

/// What cancelling said, in one sentence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CancelReport {
    pub text: String,
    /// Whether any order is not (yet) known to be cancelling.
    pub error: bool,
}

/// Which finished orders the History tab lists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryChoice {
    All,
    Filled,
    Cancelled,
    Expired,
    Rejected,
}

pub trait BrokerCommands: EditVenue {
    /// Looks an unconfirmed order up at the broker (`id` is its row's id), for the account on
    /// screen: a record of another account is not this one's to look up. It is placed if the broker
    /// never saw it.
    fn retry(&self, id: &str) -> Answer<()>;

    /// Asks the broker to cancel the orders with these ids. An order stays listed until the broker
    /// confirms.
    fn cancel(&self, orders: Vec<String>) -> Answer<CancelReport>;

    /// One page of finished orders, newest first; `cursor` continues a list.
    fn history(&self, choice: HistoryChoice, cursor: Option<String>) -> Answer<Page<OrderRow>>;

    /// One page of fills, newest first; `cursor` continues a list.
    fn fills(&self, cursor: Option<String>) -> Answer<Page<FillRow>>;
}

/// What a set of cancel answers says, in one sentence, and whether it is a problem.
pub fn cancel_report(results: &[CancelResult]) -> CancelReport {
    let requested = results
        .iter()
        .filter(|r| r.outcome == CancelOutcome::Requested)
        .count();
    let mut problems: Vec<String> = Vec::new();
    for result in results {
        match &result.outcome {
            CancelOutcome::Requested => {}
            CancelOutcome::AlreadyFilled => problems.push("one had already filled".to_owned()),
            CancelOutcome::NotFound => {
                problems.push("one could not be found (it may already be finished)".to_owned());
            }
            CancelOutcome::BeingReplaced => {
                problems.push("one is being edited; try again in a moment".to_owned());
            }
            CancelOutcome::Rejected(text) => problems.push(text.clone()),
            CancelOutcome::Unknown(_) => problems.push(
                "one did not get an answer: it may or may not be cancelled, and is being read again"
                    .to_owned(),
            ),
        }
    }
    let asked = match requested {
        0 => String::new(),
        1 => "Cancel requested. The order stays until the broker confirms.".to_owned(),
        n => format!("{n} cancels requested. The orders stay until the broker confirms."),
    };
    let (text, error) = if problems.is_empty() {
        (asked, false)
    } else if asked.is_empty() {
        (capitalise(&problems.join("; ")), true)
    } else {
        (format!("{asked} But {}.", problems.join("; ")), true)
    };
    CancelReport { text, error }
}

fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |c| {
        c.to_uppercase().collect::<String>() + chars.as_str()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use degenview_brokers::OrderId;

    fn result(outcome: CancelOutcome) -> CancelResult {
        CancelResult {
            order_id: OrderId::new("a"),
            outcome,
        }
    }

    #[test]
    fn a_cancel_request_is_described_as_a_request_never_as_done() {
        let requested = || result(CancelOutcome::Requested);
        let report = cancel_report(&[requested()]);
        assert!(
            report.text.contains("requested") && report.text.contains("until the broker confirms")
        );
        assert!(!report.error);
        assert!(
            cancel_report(&[requested(), requested()])
                .text
                .starts_with("2 cancels requested")
        );
    }

    #[test]
    fn cancels_that_lost_a_race_or_were_refused_are_called_problems() {
        let report = cancel_report(&[result(CancelOutcome::AlreadyFilled)]);
        assert!(
            report.error && report.text.contains("already filled"),
            "{}",
            report.text
        );
        let report = cancel_report(&[
            result(CancelOutcome::Requested),
            result(CancelOutcome::Rejected(
                "This order cannot be cancelled.".to_owned(),
            )),
        ]);
        assert!(
            report.error
                && report.text.contains("Cancel requested")
                && report.text.contains("cannot be cancelled"),
            "{}",
            report.text
        );
        assert!(
            cancel_report(&[result(CancelOutcome::NotFound)])
                .text
                .contains("could not be found")
        );
        assert!(
            cancel_report(&[result(CancelOutcome::BeingReplaced)])
                .text
                .contains("edited")
        );
    }
}
