//! A day's notes: the plan before, the lessons after. One entry per local calendar day for the
//! whole journal, kept apart from the trades (a day with no trades can still have an entry). Pure.

use serde::{Deserialize, Serialize};

use super::reviews::clean_list;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct DailyEntry {
    /// `YYYY-MM-DD`, a local date.
    pub date: String,
    #[serde(default)]
    pub outlook: String,
    #[serde(default)]
    pub plan: String,
    #[serde(default)]
    pub goals: String,
    #[serde(default)]
    pub mood: Vec<String>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub mistakes: String,
    #[serde(default)]
    pub lessons: String,
    /// The end-of-day review.
    #[serde(default)]
    pub review: String,
    #[serde(default)]
    pub created_ms: i64,
    #[serde(default)]
    pub updated_ms: i64,
}

impl DailyEntry {
    pub fn new(date: &str) -> Self {
        Self {
            date: date.to_owned(),
            ..Self::default()
        }
    }

    pub fn tidy(&mut self) {
        self.mood = clean_list(&self.mood);
        for text in [
            &mut self.outlook,
            &mut self.plan,
            &mut self.goals,
            &mut self.notes,
            &mut self.mistakes,
            &mut self.lessons,
            &mut self.review,
        ] {
            *text = text.trim_end().to_owned();
        }
    }

    /// Nothing written: not worth a row.
    pub fn is_empty(&self) -> bool {
        self.mood.is_empty()
            && [
                &self.outlook,
                &self.plan,
                &self.goals,
                &self.notes,
                &self.mistakes,
                &self.lessons,
                &self.review,
            ]
            .iter()
            .all(|text| text.trim().is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_entry_is_empty_and_writing_anything_is_not() {
        let mut entry = DailyEntry::new("2026-11-09");
        assert!(entry.is_empty());
        entry.lessons = "  ".into();
        assert!(entry.is_empty(), "whitespace is nothing");
        entry.mood = vec!["calm".into()];
        assert!(!entry.is_empty());
    }

    #[test]
    fn tidying_normalises_text_and_mood() {
        let mut entry = DailyEntry::new("2026-11-09");
        entry.plan = "Trade the open.\n\n".into();
        entry.mood = vec![" calm ".into(), "Calm".into(), "".into()];
        entry.tidy();
        assert_eq!(entry.plan, "Trade the open.");
        assert_eq!(entry.mood, ["calm"]);
    }

    #[test]
    fn old_rows_missing_fields_decode() {
        let entry: DailyEntry = serde_json::from_str(r#"{"date":"2026-11-09"}"#).unwrap();
        assert!(entry.is_empty());
    }
}
