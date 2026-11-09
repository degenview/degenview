//! A portable copy of everything the journal keeps: accounts, executions, reviews, playbooks,
//! daily entries and the screenshots reviews use. Versioned JSON, written by Back up and read by
//! Restore. Restoring only ever adds or updates: it never deletes and never overwrites what was
//! written later than the backup says, so restoring the same file twice, or an old file over a
//! journal that has moved on, loses nothing.
//!
//! Trades and statistics are not in it: they are rebuilt from the executions. No secrets are
//! kept anywhere in the journal, so none can be in the file.

use serde::{Deserialize, Serialize};

use super::daily::DailyEntry;
use super::model::{Execution, JournalAccount};
use super::playbook::Playbook;
use super::reviews::Review;

pub const FORMAT: &str = "degenview-journal-backup";
pub const VERSION: u32 = 1;

/// A screenshot as the asset store keeps it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BackupAsset {
    pub id: String,
    pub mime: String,
    pub width: u32,
    pub height: u32,
    /// The picture's bytes, base64.
    pub data: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Backup {
    pub format: String,
    pub version: u32,
    /// ISO-8601 UTC.
    pub exported_at: String,
    pub app_version: String,
    #[serde(default)]
    pub accounts: Vec<JournalAccount>,
    #[serde(default)]
    pub executions: Vec<Execution>,
    #[serde(default)]
    pub reviews: Vec<Review>,
    #[serde(default)]
    pub playbooks: Vec<Playbook>,
    #[serde(default)]
    pub days: Vec<DailyEntry>,
    #[serde(default)]
    pub assets: Vec<BackupAsset>,
}

impl Backup {
    pub fn empty(exported_at: String, app_version: &str) -> Self {
        Self {
            format: FORMAT.to_owned(),
            version: VERSION,
            exported_at,
            app_version: app_version.to_owned(),
            accounts: Vec::new(),
            executions: Vec::new(),
            reviews: Vec::new(),
            playbooks: Vec::new(),
            days: Vec::new(),
            assets: Vec::new(),
        }
    }

    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self).map_err(|error| error.to_string())
    }

    /// Reads a backup file's text. A file that is not a journal backup, or is from a newer
    /// version than this app understands, is refused with a reason.
    pub fn parse(text: &str) -> Result<Self, String> {
        let value: serde_json::Value = serde_json::from_str(text)
            .map_err(|_| "This is not a journal backup: the file is not JSON.".to_owned())?;
        if value.get("format").and_then(|format| format.as_str()) != Some(FORMAT) {
            return Err("This is not a DegenView journal backup.".to_owned());
        }
        let version = value
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if version == 0 || version > u64::from(VERSION) {
            return Err(format!(
                "This backup is version {version}; this app reads version {VERSION}. Update DegenView to restore it."
            ));
        }
        serde_json::from_value(value)
            .map_err(|error| format!("The backup could not be read: {error}."))
    }

    pub fn summary(&self) -> String {
        format!(
            "{} accounts, {} executions, {} reviews, {} playbooks, {} daily entries, {} screenshots",
            self.accounts.len(),
            self.executions.len(),
            self.reviews.len(),
            self.playbooks.len(),
            self.days.len(),
            self.assets.len()
        )
    }
}

/// What a restore did.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RestoreReport {
    pub accounts_added: usize,
    pub executions_added: usize,
    pub executions_present: usize,
    pub reviews_restored: usize,
    /// Reviews, playbooks or days left alone because the journal already had a newer edit.
    pub kept_newer: usize,
    pub playbooks_restored: usize,
    pub days_restored: usize,
    pub assets_added: usize,
    /// Things in the file that could not be used, with why.
    pub rejected: Vec<String>,
}

impl RestoreReport {
    pub fn changed(&self) -> bool {
        self.accounts_added
            + self.executions_added
            + self.reviews_restored
            + self.playbooks_restored
            + self.days_restored
            + self.assets_added
            > 0
    }

    pub fn summary(&self) -> String {
        let mut parts = vec![format!(
            "{} executions added ({} already there)",
            self.executions_added, self.executions_present
        )];
        parts.push(format!("{} reviews", self.reviews_restored));
        parts.push(format!("{} playbooks", self.playbooks_restored));
        parts.push(format!("{} daily entries", self.days_restored));
        parts.push(format!("{} screenshots", self.assets_added));
        let mut text = parts.join(", ");
        if self.kept_newer > 0 {
            text.push_str(&format!(
                "; {} left as they were because the journal had newer edits",
                self.kept_newer
            ));
        }
        if !self.rejected.is_empty() {
            text.push_str(&format!(
                "; {} items could not be used",
                self.rejected.len()
            ));
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_backup_round_trips_through_text() {
        let mut backup = Backup::empty("2026-11-10T00:00:00.000Z".into(), "0.6.0");
        backup.reviews.push(Review::new("k", "a"));
        let text = backup.to_json().unwrap();
        assert_eq!(Backup::parse(&text).unwrap(), backup);
        assert!(backup.summary().contains("1 reviews"));
    }

    #[test]
    fn other_files_and_newer_versions_are_refused_with_a_reason() {
        assert!(Backup::parse("nope").unwrap_err().contains("not JSON"));
        assert!(
            Backup::parse(r#"{"format":"something-else","version":1}"#)
                .unwrap_err()
                .contains("not a DegenView")
        );
        let newer = format!(r#"{{"format":"{FORMAT}","version":99}}"#);
        assert!(Backup::parse(&newer).unwrap_err().contains("version 99"));
        let broken = format!(r#"{{"format":"{FORMAT}","version":1,"accounts":"x"}}"#);
        assert!(
            Backup::parse(&broken)
                .unwrap_err()
                .contains("could not be read")
        );
        // A backup with only the header is a valid, empty one.
        let bare =
            format!(r#"{{"format":"{FORMAT}","version":1,"exported_at":"x","app_version":"y"}}"#);
        assert!(Backup::parse(&bare).unwrap().executions.is_empty());
    }

    #[test]
    fn the_report_says_what_happened() {
        let report = RestoreReport {
            executions_added: 3,
            kept_newer: 2,
            rejected: vec!["x".into()],
            ..RestoreReport::default()
        };
        let text = report.summary();
        assert!(text.contains("3 executions added"));
        assert!(text.contains("2 left as they were"));
        assert!(text.contains("1 items could not be used"));
        assert!(report.changed());
        assert!(!RestoreReport::default().changed());
    }
}
