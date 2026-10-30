//! Persistence for the editor's themes: the custom themes in the `editor_theme` document table and
//! the choices (selected theme, typography) under the `editor.prefs` setting.
//!
//! Reading is forgiving, writing is careful. A theme row is read on its own, so one row this
//! version cannot decode never hides the others; but the table is then not rewritten (the
//! undecodable row would be lost) and the caller is told why. The choices are one JSON value read
//! field by field ([`EditorPrefs`]); a value that is not even an object is replaced by the
//! defaults, since there is nothing in it to keep.

use crate::model::editor_theme::prefs::KEY;
use crate::model::editor_theme::{EditorPrefs, EditorTheme, ThemeBook};
use crate::store::{Document, Store};

const TABLE: &str = "editor_theme";

impl Document for EditorTheme {
    fn document_id(&self) -> String {
        self.id.to_string()
    }
}

/// What the store held, as far as it could be read.
pub struct Loaded {
    pub book: ThemeBook,
    pub prefs: EditorPrefs,
    /// Why the themes cannot be saved, when a row could not be read. They still work.
    pub themes_unwritable: Option<String>,
}

impl Store {
    /// The custom themes in the order they were made. One row this version cannot decode fails
    /// the read, so a caller that writes the list back can never drop that row.
    pub fn editor_themes_strict(&self) -> rusqlite::Result<Vec<EditorTheme>> {
        self.documents_strict(TABLE)
    }

    pub fn replace_editor_themes(&self, themes: &[EditorTheme]) -> rusqlite::Result<()> {
        self.replace_documents(TABLE, themes)
    }

    /// The saved choices, or the defaults for nothing stored or something unreadable. A value
    /// that is there but unreadable is reported in the error.
    pub fn editor_prefs(&self) -> rusqlite::Result<EditorPrefs> {
        Ok(self
            .setting_strict::<EditorPrefs>(KEY)?
            .unwrap_or_default()
            .sanitized())
    }

    pub fn set_editor_prefs(&self, prefs: &EditorPrefs) -> rusqlite::Result<()> {
        self.set_setting(KEY, prefs)
    }

    /// Everything the editor's appearance is made of.
    pub fn load_editor_themes(&self) -> Loaded {
        let (themes, themes_unwritable) = match self.editor_themes_strict() {
            Ok(themes) => (themes, None),
            Err(error) => (
                self.documents(TABLE),
                Some(format!(
                    "A saved theme could not be read ({error}), so changes to themes are not saved."
                )),
            ),
        };
        Loaded {
            book: ThemeBook::new(themes),
            prefs: self.editor_prefs().unwrap_or_default().sanitized(),
            themes_unwritable,
        }
    }
}

#[cfg(test)]
mod tests;
