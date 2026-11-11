//! Persistence for chart appearance templates, under the `chart_template` document table.
//!
//! [`ChartTemplateStore`] is what the settings sheet holds: the user's saved templates as a
//! [`TemplateBook`], read once when the sheet opens. Every change reads the table again first, so
//! a sheet open in another window cannot be overwritten with a stale list, then validates,
//! writes in one transaction and only then shows the result. A table that cannot be read is
//! never written: the sheet says so and the presets keep working.

use crate::model::ChartAppearance;
use crate::model::chart::template::{
    ChartAppearanceTemplate, TemplateBook, TemplateError, TemplateId, new_user_id,
};
use crate::model::watchlist::now_ms;
use crate::store::{Document, SharedStore, Store};

const TABLE: &str = "chart_template";

impl Document for ChartAppearanceTemplate {
    fn document_id(&self) -> String {
        self.id.to_string()
    }
}

impl Store {
    /// The saved templates in the order they were made. One row this version cannot decode fails
    /// the read, so a caller that writes the list back can never drop that row.
    pub fn chart_templates_strict(&self) -> rusqlite::Result<Vec<ChartAppearanceTemplate>> {
        self.documents_strict(TABLE)
    }

    pub fn replace_chart_templates(
        &self,
        templates: &[ChartAppearanceTemplate],
    ) -> rusqlite::Result<()> {
        self.replace_documents(TABLE, templates)
    }
}

pub struct ChartTemplateStore {
    store: SharedStore,
    book: TemplateBook,
    /// Why the saved templates could not be read; nothing is written while it is set.
    load_error: Option<String>,
}

impl ChartTemplateStore {
    pub fn new(store: SharedStore) -> Self {
        let loaded = store
            .lock()
            .expect("store lock poisoned")
            .chart_templates_strict();
        match loaded {
            Ok(templates) => Self {
                store,
                book: TemplateBook::new(templates),
                load_error: None,
            },
            Err(error) => Self {
                store,
                book: TemplateBook::default(),
                load_error: Some(error.to_string()),
            },
        }
    }

    pub fn book(&self) -> &TemplateBook {
        &self.book
    }

    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    /// Runs `change` on the table as it is now and saves the result. Returns what `change`
    /// returns, or why nothing was saved.
    fn mutate<R>(
        &mut self,
        change: impl FnOnce(&mut TemplateBook) -> Result<R, TemplateError>,
    ) -> Result<R, String> {
        if let Some(error) = &self.load_error {
            return Err(format!(
                "Saved templates could not be read ({error}), so none are changed."
            ));
        }
        let shared = self.store.clone();
        let store = shared.lock().expect("store lock poisoned");
        let fresh = match store.chart_templates_strict() {
            Ok(fresh) => fresh,
            Err(error) => {
                self.load_error = Some(error.to_string());
                return Err(format!(
                    "Saved templates could not be read ({error}), so none are changed."
                ));
            }
        };
        let mut book = TemplateBook::new(fresh);
        let result = change(&mut book).map_err(TemplateError::message);
        if result.is_ok() {
            store
                .replace_chart_templates(book.saved())
                .map_err(|error| format!("Could not save the template: {error}"))?;
        }
        // What the table holds now is what to show, whatever the change was.
        self.book = book;
        result
    }

    pub fn save(
        &mut self,
        name: &str,
        description: &str,
        appearance: &ChartAppearance,
    ) -> Result<TemplateId, String> {
        self.mutate(|book| book.save(new_user_id(), name, description, appearance, now_ms()))
    }

    pub fn rename(&mut self, id: TemplateId, name: &str, description: &str) -> Result<(), String> {
        self.mutate(|book| book.rename(id, name, description, now_ms()))
    }

    pub fn update_appearance(
        &mut self,
        id: TemplateId,
        appearance: &ChartAppearance,
    ) -> Result<(), String> {
        self.mutate(|book| book.update_appearance(id, appearance, now_ms()))
    }

    pub fn duplicate(&mut self, id: TemplateId) -> Result<TemplateId, String> {
        self.mutate(|book| book.duplicate(id, new_user_id(), now_ms()))
    }

    pub fn delete(&mut self, id: TemplateId) -> Result<(), String> {
        self.mutate(|book| book.delete(id))
    }
}

#[cfg(test)]
mod tests;
