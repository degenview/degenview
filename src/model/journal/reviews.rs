//! What the user writes about a trade. Kept apart from everything derived from executions: a
//! review is editable, keyed by the trade's key (the uid of the execution that opened it), and
//! never changes a balance, an order or a trade's figures. Editing one cannot reach paper trading.
//!
//! A trade whose executions vanish (an account purged, a source correction) leaves its review
//! behind as an *orphan*, listed rather than deleted.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// How far along a trade's review is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewStatus {
    /// Nothing written.
    Pending,
    /// Something written, not marked reviewed.
    Draft,
    Reviewed,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Review {
    pub trade_key: String,
    pub account_uid: String,
    #[serde(default)]
    pub notes: String,
    /// The playbook this trade followed (its id).
    #[serde(default)]
    pub playbook: Option<String>,
    /// The playbook checklist items ticked, by their text.
    #[serde(default)]
    pub checks: std::collections::BTreeSet<String>,
    #[serde(default)]
    pub setup: String,
    #[serde(default)]
    pub entry_rationale: String,
    #[serde(default)]
    pub exit_rationale: String,
    /// 1 to 5, how well the plan was followed.
    #[serde(default)]
    pub discipline: Option<u8>,
    /// 1 to 5, how sure the trader was going in.
    #[serde(default)]
    pub confidence: Option<u8>,
    /// What the trader intended to enter at, when it differs from the fill.
    #[serde(default)]
    pub planned_entry: Option<Decimal>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub mistakes: Vec<String>,
    #[serde(default)]
    pub emotions: Vec<String>,
    /// 1 to 5.
    #[serde(default)]
    pub rating: Option<u8>,
    #[serde(default)]
    pub reviewed: bool,
    /// A stop the user entered. Overrides the one attached to the entry order.
    #[serde(default)]
    pub stop: Option<Decimal>,
    #[serde(default)]
    pub target: Option<Decimal>,
    #[serde(default)]
    pub created_ms: i64,
    #[serde(default)]
    pub updated_ms: i64,
}

/// Trims, drops empties and removes case-insensitive duplicates, keeping the first spelling.
pub fn clean_list(items: &[String]) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    items
        .iter()
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty() && seen.insert(item.to_lowercase()))
        .collect()
}

/// Splits typed text on commas and newlines into list items.
pub fn parse_list(text: &str) -> Vec<String> {
    clean_list(
        &text
            .split([',', '\n'])
            .map(str::to_owned)
            .collect::<Vec<_>>(),
    )
}

/// Emotions and behaviours offered as one-click tags. Users add their own by typing; everything
/// already used in a review is offered too.
pub const DEFAULT_EMOTIONS: [&str; 9] = [
    "FOMO",
    "Hesitation",
    "Impatience",
    "Calm",
    "Confident",
    "Anxious",
    "Frustrated",
    "Following plan",
    "Good execution",
];

pub const DEFAULT_MISTAKES: [&str; 7] = [
    "Revenge trading",
    "Overtrading",
    "Breaking rules",
    "Poor risk management",
    "Emotional exit",
    "Moved stop",
    "Chased entry",
];

/// `defaults` followed by anything in `used` that is not already there (ignoring case), the used
/// ones in alphabetical order.
pub fn suggestions<'a>(
    defaults: &[&str],
    used: impl IntoIterator<Item = &'a String>,
) -> Vec<String> {
    let mut out: Vec<String> = defaults.iter().map(|d| (*d).to_owned()).collect();
    let mut extra: Vec<&String> = used.into_iter().collect();
    extra.sort_by_key(|item| item.to_lowercase());
    for item in extra {
        if !out
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(item))
        {
            out.push(item.clone());
        }
    }
    out
}

impl Review {
    pub fn new(trade_key: &str, account_uid: &str) -> Self {
        Self {
            trade_key: trade_key.to_owned(),
            account_uid: account_uid.to_owned(),
            ..Self::default()
        }
    }

    /// Normalises the fields the way they are stored.
    pub fn tidy(&mut self) {
        self.notes = self.notes.trim_end().to_owned();
        self.playbook = self.playbook.take().filter(|id| !id.trim().is_empty());
        self.setup = self.setup.trim().to_owned();
        self.entry_rationale = self.entry_rationale.trim_end().to_owned();
        self.exit_rationale = self.exit_rationale.trim_end().to_owned();
        self.checks.retain(|item| !item.trim().is_empty());
        self.discipline = self.discipline.filter(|rating| (1..=5).contains(rating));
        self.confidence = self.confidence.filter(|rating| (1..=5).contains(rating));
        self.planned_entry = self.planned_entry.filter(|price| *price > Decimal::ZERO);
        self.tags = clean_list(&self.tags);
        self.mistakes = clean_list(&self.mistakes);
        self.emotions = clean_list(&self.emotions);
        self.rating = self.rating.filter(|rating| (1..=5).contains(rating));
        self.stop = self.stop.filter(|stop| *stop > Decimal::ZERO);
        self.target = self.target.filter(|target| *target > Decimal::ZERO);
    }

    /// Nothing written and nothing marked: not worth a row.
    pub fn is_empty(&self) -> bool {
        self.notes.trim().is_empty()
            && self.playbook.is_none()
            && self.checks.is_empty()
            && self.setup.is_empty()
            && self.entry_rationale.trim().is_empty()
            && self.exit_rationale.trim().is_empty()
            && self.discipline.is_none()
            && self.confidence.is_none()
            && self.planned_entry.is_none()
            && self.tags.is_empty()
            && self.mistakes.is_empty()
            && self.emotions.is_empty()
            && self.rating.is_none()
            && !self.reviewed
            && self.stop.is_none()
            && self.target.is_none()
    }

    pub fn status(&self) -> ReviewStatus {
        if self.reviewed {
            ReviewStatus::Reviewed
        } else if self.is_empty() {
            ReviewStatus::Pending
        } else {
            ReviewStatus::Draft
        }
    }
}

pub fn status_of(review: Option<&Review>) -> ReviewStatus {
    review.map_or(ReviewStatus::Pending, Review::status)
}

impl ReviewStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Draft => "Draft",
            Self::Reviewed => "Reviewed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    #[test]
    fn lists_are_trimmed_and_deduplicated_ignoring_case() {
        let cleaned = clean_list(&[" FOMO ".into(), "fomo".into(), "".into(), "Revenge".into()]);
        assert_eq!(cleaned, ["FOMO", "Revenge"]);
        assert_eq!(parse_list("a, B ,,a\nc"), ["a", "B", "c"]);
        assert!(parse_list("  ,\n ").is_empty());
    }

    #[test]
    fn suggestions_are_the_defaults_then_what_was_used() {
        let used = [
            "late fill".to_owned(),
            "fomo".to_owned(),
            "Anxious".to_owned(),
            "chart gap".to_owned(),
        ];
        let list = suggestions(&["FOMO", "Anxious"], used.iter());
        assert_eq!(list, ["FOMO", "Anxious", "chart gap", "late fill"]);
        assert_eq!(suggestions(&[], std::iter::empty()), Vec::<String>::new());
    }

    #[test]
    fn a_new_review_is_empty_and_pending() {
        let review = Review::new("k", "acct");
        assert!(review.is_empty());
        assert_eq!(review.status(), ReviewStatus::Pending);
        assert_eq!(status_of(None), ReviewStatus::Pending);
    }

    #[test]
    fn writing_anything_makes_a_draft_and_marking_it_makes_it_reviewed() {
        let mut review = Review::new("k", "acct");
        review.notes = "  held too long".into();
        assert_eq!(review.status(), ReviewStatus::Draft);
        review.reviewed = true;
        assert_eq!(review.status(), ReviewStatus::Reviewed);
        review.notes.clear();
        assert_eq!(
            review.status(),
            ReviewStatus::Reviewed,
            "marked reviewed is not empty"
        );
    }

    #[test]
    fn tidy_normalises_what_is_stored() {
        let mut review = Review::new("k", "acct");
        review.notes = "text  \n\n".into();
        review.playbook = Some(" p1 ".into());
        review.discipline = Some(9);
        review.planned_entry = Some(d("0"));
        review.tags = vec!["a".into(), "A".into(), " ".into()];
        review.rating = Some(9);
        review.stop = Some(d("0"));
        review.target = Some(d("12.5"));
        review.tidy();
        assert_eq!(review.notes, "text");
        assert_eq!(review.playbook.as_deref(), Some(" p1 "));
        assert_eq!(review.discipline, None);
        assert_eq!(review.planned_entry, None);
        assert_eq!(review.tags, ["a"]);
        assert_eq!(review.rating, None);
        assert_eq!(review.stop, None);
        assert_eq!(review.target, Some(d("12.5")));
    }

    #[test]
    fn a_rating_alone_counts_as_written() {
        let mut review = Review::new("k", "acct");
        review.rating = Some(4);
        assert!(!review.is_empty());
        assert_eq!(review.status(), ReviewStatus::Draft);
    }

    #[test]
    fn old_rows_missing_newer_fields_decode() {
        let review: Review =
            serde_json::from_str(r#"{"trade_key":"k","account_uid":"a"}"#).unwrap();
        assert!(review.is_empty());
    }
}
