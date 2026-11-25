//! What a connected broker is asked besides taking an order: look an unconfirmed order up again,
//! cancel, read older orders and fills, and (through [`EditVenue`]) change an open order in place.
//!
//! The screens hold this trait, never the broker's runtime: every call is a [`Task`] they poll on
//! later frames, and a call made while the broker is not connected answers with an error at once.

use crate::i18n::{tr, tr_n};
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

/// What a set of cancel answers says, and whether it is a problem: a sentence for the requests and
/// one for each answer that is not "requested", each whole, so a language words each its own way.
pub fn cancel_report(results: &[CancelResult]) -> CancelReport {
    let requested = results
        .iter()
        .filter(|r| r.outcome == CancelOutcome::Requested)
        .count();
    let mut sentences: Vec<String> = Vec::new();
    if requested > 0 {
        sentences.push(tr_n!(
            requested,
            "Cancel requested. The order stays until the broker confirms.",
            "{count} cancels requested. The orders stay until the broker confirms."
        ));
    }
    let mut error = false;
    for result in results {
        let sentence = match &result.outcome {
            CancelOutcome::Requested => continue,
            CancelOutcome::AlreadyFilled => tr!("One order had already filled.").to_owned(),
            CancelOutcome::NotFound => {
                tr!("One order could not be found (it may already be finished).").to_owned()
            }
            CancelOutcome::BeingReplaced => {
                tr!("One order is being edited; try again in a moment.").to_owned()
            }
            CancelOutcome::Rejected(text) => text.clone(),
            CancelOutcome::Unknown(_) => tr!(
                "One order did not get an answer: it may or may not be cancelled, and is being read again."
            )
            .to_owned(),
        };
        error = true;
        sentences.push(sentence);
    }
    CancelReport {
        text: crate::i18n::join_sentences(&sentences),
        error,
    }
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

    #[test]
    fn a_cancel_report_follows_the_language() {
        use crate::i18n::{Locale, with_locale};
        let results = [
            result(CancelOutcome::Requested),
            result(CancelOutcome::NotFound),
        ];
        let english = cancel_report(&results);
        let pseudo = with_locale(Locale::PSEUDO, || cancel_report(&results));
        assert_ne!(pseudo.text, english.text);
        assert_eq!(pseudo.error, english.error);
    }
}
