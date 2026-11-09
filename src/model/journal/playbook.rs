//! Playbooks: the setups a trader repeats, written down so a trade can be measured against them.
//! Pure. A review points at a playbook by id and ticks the playbook's checklist; **rule
//! adherence** is the share of the checklist ticked.
//!
//! The checklist is matched by the item's text, so renaming an item resets its ticks and removing
//! one drops it from the share. Adherence is a measure of what the user recorded about their own
//! process; it is never mixed into the Edge Score.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::reviews::{Review, clean_list};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Playbook {
    /// Stable across devices (a nanosecond id as text).
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub setup_criteria: String,
    #[serde(default)]
    pub entry_rules: String,
    #[serde(default)]
    pub exit_rules: String,
    #[serde(default)]
    pub risk_rules: String,
    #[serde(default)]
    pub checklist: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub created_ms: i64,
    #[serde(default)]
    pub updated_ms: i64,
}

impl Playbook {
    pub fn new(id: String, name: &str) -> Self {
        Self {
            id,
            name: name.trim().to_owned(),
            ..Self::default()
        }
    }

    pub fn tidy(&mut self) {
        self.name = self.name.trim().to_owned();
        self.checklist = clean_list(&self.checklist);
        self.tags = clean_list(&self.tags);
    }

    /// A playbook needs a name.
    pub fn is_valid(&self) -> bool {
        !self.name.trim().is_empty()
    }
}

/// The share of `playbook`'s checklist ticked in `review`, or `None` when the review has no
/// playbook, the playbook has no checklist, or the review is not for this playbook.
pub fn rule_adherence(review: &Review, playbook: &Playbook) -> Option<f64> {
    if review.playbook.as_deref() != Some(playbook.id.as_str()) || playbook.checklist.is_empty() {
        return None;
    }
    let ticked: BTreeSet<&str> = review.checks.iter().map(String::as_str).collect();
    let done = playbook
        .checklist
        .iter()
        .filter(|item| ticked.contains(item.as_str()))
        .count();
    Some(done as f64 / playbook.checklist.len() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playbook() -> Playbook {
        let mut p = Playbook::new("1".into(), "  Breakout ");
        p.checklist = vec![
            "Volume spike".into(),
            "Above VWAP".into(),
            "Stop placed".into(),
            "volume spike".into(),
        ];
        p.tidy();
        p
    }

    #[test]
    fn tidying_trims_and_deduplicates() {
        let p = playbook();
        assert_eq!(p.name, "Breakout");
        assert_eq!(p.checklist, ["Volume spike", "Above VWAP", "Stop placed"]);
        assert!(p.is_valid());
        assert!(!Playbook::new("2".into(), "   ").is_valid());
    }

    #[test]
    fn adherence_is_the_share_of_the_checklist_ticked() {
        let p = playbook();
        let mut review = Review::new("t", "a");
        review.playbook = Some("1".into());
        assert_eq!(rule_adherence(&review, &p), Some(0.0));
        review.checks = ["Volume spike".to_owned(), "Stop placed".to_owned()].into();
        assert!((rule_adherence(&review, &p).unwrap() - 2.0 / 3.0).abs() < 1e-12);
        // A tick for an item that is no longer on the list does not count.
        review.checks.insert("Old rule".into());
        assert!((rule_adherence(&review, &p).unwrap() - 2.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn no_playbook_no_checklist_or_another_playbook_has_no_adherence() {
        let p = playbook();
        let mut review = Review::new("t", "a");
        assert_eq!(rule_adherence(&review, &p), None);
        review.playbook = Some("other".into());
        assert_eq!(rule_adherence(&review, &p), None);
        let mut empty = Playbook::new("1".into(), "x");
        empty.tidy();
        review.playbook = Some("1".into());
        assert_eq!(rule_adherence(&review, &empty), None);
    }
}
