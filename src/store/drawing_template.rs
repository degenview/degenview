//! Persistence for drawing styles: the saved templates (the `drawing_template` document table),
//! each tool's default style (the `drawing.defaults` setting) and the strip's drawing options
//! (the `drawing.prefs` setting).
//!
//! [`DrawingTemplateStore`] mirrors [`ChartTemplateStore`](super::chart_template): every change
//! reads the table again first so a second window cannot overwrite a stale list, validates,
//! writes in one transaction and only then shows the result, and a table that cannot be read is
//! never written. None of it holds a drawing's coordinates.

use crate::model::chart_template::{TemplateError, TemplateId, new_user_id};
use crate::model::drawing_template::{DrawingTemplate, TemplateBook};
use crate::model::watchlist::now_ms;
use crate::model::{DrawingStyle, Tool};
use crate::store::{Document, SharedStore, Store};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const TABLE: &str = "drawing_template";
const DEFAULTS_KEY: &str = "drawing.defaults";
const PREFS_KEY: &str = "drawing.prefs";
/// How long the defaults read from the store are trusted before another tab's change is looked
/// for.
const REFRESH_AFTER: Duration = Duration::from_secs(3);

impl Document for DrawingTemplate {
    fn document_id(&self) -> String {
        self.id.to_string()
    }
}

/// The strip's drawing options, remembered between launches.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct DrawingPrefs {
    pub magnet: crate::model::MagnetMode,
    pub keep_drawing: bool,
    /// The tool the grouped brushes, arrows and shapes tile shows.
    pub group_tool: Tool,
    /// The tool the Lines tile shows.
    pub line_tool: Tool,
    /// The tool the text and notes tile shows.
    pub note_tool: Tool,
}

impl Default for DrawingPrefs {
    fn default() -> Self {
        Self {
            magnet: crate::model::MagnetMode::Off,
            keep_drawing: true,
            group_tool: Tool::Brush,
            line_tool: Tool::TrendLine,
            note_tool: Tool::Text,
        }
    }
}

impl DrawingPrefs {
    /// The prefs with each tile's tool brought back to one its menu holds, so a stored value
    /// that names something else (a hand-edited row) cannot leave a tile with the wrong glyph.
    pub fn sanitized(mut self) -> Self {
        use crate::model::ToolGroup;
        let defaults = Self::default();
        if !ToolGroup::MENU.contains(&self.group_tool.spec().group) {
            self.group_tool = defaults.group_tool;
        }
        if !ToolGroup::LINE_MENU.contains(&self.line_tool.spec().group) {
            self.line_tool = defaults.line_tool;
        }
        if !ToolGroup::NOTE_MENU.contains(&self.note_tool.spec().group) {
            self.note_tool = defaults.note_tool;
        }
        self
    }
}

impl Store {
    /// The saved templates in the order they were made. One row this version cannot decode fails
    /// the read, so a caller that writes the list back can never drop that row.
    pub fn drawing_templates_strict(&self) -> rusqlite::Result<Vec<DrawingTemplate>> {
        self.documents_strict(TABLE)
    }

    pub fn replace_drawing_templates(&self, templates: &[DrawingTemplate]) -> rusqlite::Result<()> {
        self.replace_documents(TABLE, templates)
    }
}

pub struct DrawingTemplateStore {
    store: SharedStore,
    book: TemplateBook,
    defaults: HashMap<Tool, DrawingStyle>,
    /// Why the saved templates could not be read; nothing is written while it is set.
    load_error: Option<String>,
    /// Why the defaults could not be read; they are not written while it is set.
    defaults_error: Option<String>,
    read_at: Instant,
}

impl DrawingTemplateStore {
    pub fn new(store: SharedStore) -> Self {
        let mut this = Self {
            store,
            book: TemplateBook::default(),
            defaults: HashMap::new(),
            load_error: None,
            defaults_error: None,
            read_at: Instant::now(),
        };
        this.reload();
        this
    }

    fn reload(&mut self) {
        let shared = self.store.clone();
        let store = shared.lock().expect("store lock poisoned");
        match store.drawing_templates_strict() {
            Ok(templates) => {
                self.book = TemplateBook::new(templates);
                self.load_error = None;
            }
            Err(error) => self.load_error = Some(error.to_string()),
        }
        match store.setting_strict::<HashMap<Tool, DrawingStyle>>(DEFAULTS_KEY) {
            Ok(defaults) => {
                self.defaults = defaults
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(tool, style)| (tool, style.sanitized()))
                    .collect();
                self.defaults_error = None;
            }
            Err(error) => self.defaults_error = Some(error.to_string()),
        }
        self.read_at = Instant::now();
    }

    /// Reads the store again when what was read is a few seconds old, so a default set in
    /// another tab reaches this one.
    pub fn refresh(&mut self, now: Instant) {
        if now.saturating_duration_since(self.read_at) >= REFRESH_AFTER {
            self.reload();
        }
    }

    pub fn book(&self) -> &TemplateBook {
        &self.book
    }

    /// The user's default styles by tool; a tool without one starts from its built-in look.
    pub fn defaults(&self) -> &HashMap<Tool, DrawingStyle> {
        &self.defaults
    }

    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    fn mutate<R>(
        &mut self,
        change: impl FnOnce(&mut TemplateBook) -> Result<R, TemplateError>,
    ) -> Result<R, String> {
        if let Some(error) = &self.load_error {
            return Err(format!(
                "Saved drawing templates could not be read ({error}), so none are changed."
            ));
        }
        let shared = self.store.clone();
        let store = shared.lock().expect("store lock poisoned");
        let fresh = match store.drawing_templates_strict() {
            Ok(fresh) => fresh,
            Err(error) => {
                self.load_error = Some(error.to_string());
                return Err(format!(
                    "Saved drawing templates could not be read ({error}), so none are changed."
                ));
            }
        };
        let mut book = TemplateBook::new(fresh);
        let result = change(&mut book).map_err(TemplateError::message);
        if result.is_ok() {
            store
                .replace_drawing_templates(book.saved())
                .map_err(|error| format!("Could not save the template: {error}"))?;
        }
        self.book = book;
        result
    }

    pub fn save(
        &mut self,
        name: &str,
        tool: Tool,
        style: &DrawingStyle,
    ) -> Result<TemplateId, String> {
        let style = *style;
        self.mutate(|book| book.save(new_user_id(), name, tool, style, now_ms()))
    }

    pub fn rename(&mut self, id: TemplateId, name: &str) -> Result<(), String> {
        self.mutate(|book| book.rename(id, name, now_ms()))
    }

    pub fn delete(&mut self, id: TemplateId) -> Result<(), String> {
        self.mutate(|book| book.delete(id))
    }

    /// Makes `style` the style new drawings of `tool` start with.
    pub fn set_default(&mut self, tool: Tool, style: &DrawingStyle) -> Result<(), String> {
        self.write_defaults(|defaults| {
            defaults.insert(tool, style.sanitized());
        })
    }

    /// Goes back to the built-in look for `tool`.
    pub fn reset_default(&mut self, tool: Tool) -> Result<(), String> {
        self.write_defaults(|defaults| {
            defaults.remove(&tool);
        })
    }

    fn write_defaults(
        &mut self,
        change: impl FnOnce(&mut HashMap<Tool, DrawingStyle>),
    ) -> Result<(), String> {
        if let Some(error) = &self.defaults_error {
            return Err(format!(
                "Default styles could not be read ({error}), so none are changed."
            ));
        }
        let shared = self.store.clone();
        let store = shared.lock().expect("store lock poisoned");
        let mut defaults = match store.setting_strict::<HashMap<Tool, DrawingStyle>>(DEFAULTS_KEY) {
            Ok(defaults) => defaults.unwrap_or_default(),
            Err(error) => {
                self.defaults_error = Some(error.to_string());
                return Err(format!(
                    "Default styles could not be read ({error}), so none are changed."
                ));
            }
        };
        change(&mut defaults);
        store
            .set_setting(DEFAULTS_KEY, &defaults)
            .map_err(|error| format!("Could not save the default: {error}"))?;
        self.defaults = defaults;
        Ok(())
    }

    /// The strip's remembered options.
    pub fn prefs(&self) -> DrawingPrefs {
        self.store
            .lock()
            .expect("store lock poisoned")
            .setting::<DrawingPrefs>(PREFS_KEY)
            .unwrap_or_default()
            .sanitized()
    }

    pub fn set_prefs(&self, prefs: &DrawingPrefs) {
        let _ = self
            .store
            .lock()
            .expect("store lock poisoned")
            .set_setting(PREFS_KEY, prefs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MagnetMode;
    use std::sync::{Arc, Mutex};

    fn shared() -> SharedStore {
        Arc::new(Mutex::new(Store::open_in_memory().expect("open")))
    }

    fn style(width: f32) -> DrawingStyle {
        let mut style = Tool::Rectangle.default_style();
        style.stroke.width = width;
        style
    }

    #[test]
    fn every_tools_template_and_default_keep_its_levels_and_settings() {
        let store = shared();
        let mut templates = DrawingTemplateStore::new(store.clone());
        for tool in Tool::GROUPED {
            let mut custom = tool.default_style();
            custom.pitchfork_levels[2].ratio = 0.45;
            custom.pitchfork_levels[2].line.enabled = true;
            custom.channel_levels[1].line.enabled = true;
            custom.regression.upper_dev = 1.25;
            custom.regression.pearson = false;
            custom.use_one_color = true;
            custom.text.size = 20.0;
            templates
                .save("Mine", tool, &custom)
                .unwrap_or_else(|error| panic!("{tool:?}: {error}"));
            templates.set_default(tool, &custom).unwrap();
        }
        // A new session reads every one back, for the tool it was saved for.
        let mut again = DrawingTemplateStore::new(store);
        again.refresh(Instant::now() + REFRESH_AFTER);
        for tool in Tool::GROUPED {
            let saved = again.book().for_tool(tool);
            assert_eq!(saved.len(), 1, "{tool:?}");
            let style = saved[0].style;
            assert_eq!(style.pitchfork_levels[2].ratio, 0.45, "{tool:?}");
            assert!(style.channel_levels[1].line.enabled && style.use_one_color);
            assert_eq!(style.regression.upper_dev, 1.25);
            assert!(!style.regression.pearson);
            assert_eq!(again.defaults()[&tool], style, "{tool:?}");
        }
        // And a new Ray starts from what its default says, not the built-in look.
        assert_eq!(again.defaults()[&Tool::Ray].text.size, 20.0);
    }

    #[test]
    fn templates_persist_and_a_new_session_reads_them_back() {
        let store = shared();
        let mut templates = DrawingTemplateStore::new(store.clone());
        let id = templates
            .save("Zone", Tool::Rectangle, &style(5.0))
            .expect("saved");
        let other = templates
            .save("Ring", Tool::Ellipse, &style(3.0))
            .expect("saved");
        templates.rename(other, "Halo").expect("renamed");

        let restarted = DrawingTemplateStore::new(store);
        let saved = restarted.book().saved();
        assert_eq!(saved.len(), 2);
        assert_eq!((saved[0].id, saved[0].name.as_str()), (id, "Zone"));
        assert_eq!(saved[0].style.stroke.width, 5.0);
        assert_eq!(saved[1].name, "Halo");
        assert!(restarted.load_error().is_none());
    }

    #[test]
    fn a_change_made_elsewhere_is_not_overwritten() {
        let store = shared();
        let mut first = DrawingTemplateStore::new(store.clone());
        let mut second = DrawingTemplateStore::new(store.clone());
        first.save("From one", Tool::Arc, &style(1.0)).unwrap();
        second.save("From two", Tool::Arc, &style(2.0)).unwrap();
        let reread = DrawingTemplateStore::new(store);
        let names: Vec<&str> = reread
            .book()
            .saved()
            .iter()
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(names, vec!["From one", "From two"]);
        // A clash is reported and changes nothing.
        let error = second.save("from ONE", Tool::Arc, &style(3.0)).unwrap_err();
        assert!(error.contains("already exists"), "{error}");
        assert_eq!(second.book().saved().len(), 2);
    }

    #[test]
    fn a_row_this_version_cannot_read_is_never_written_over() {
        let store = shared();
        store
            .lock()
            .unwrap()
            .connection
            .execute(
                "INSERT INTO drawing_template (id, position, payload) VALUES ('1', 0, '{not json')",
                [],
            )
            .unwrap();
        let mut templates = DrawingTemplateStore::new(store.clone());
        assert!(templates.load_error().is_some());
        let error = templates.save("New", Tool::Arc, &style(1.0)).unwrap_err();
        assert!(error.contains("could not be read"), "{error}");
        assert!(store.lock().unwrap().drawing_templates_strict().is_err());
    }

    #[test]
    fn defaults_are_set_and_reset_per_tool_and_survive_a_restart() {
        let store = shared();
        let mut templates = DrawingTemplateStore::new(store.clone());
        assert!(templates.defaults().is_empty());
        templates.set_default(Tool::Rectangle, &style(6.0)).unwrap();
        templates.set_default(Tool::Arc, &style(4.0)).unwrap();
        templates.reset_default(Tool::Arc).unwrap();
        let restarted = DrawingTemplateStore::new(store);
        assert_eq!(restarted.defaults().len(), 1);
        assert_eq!(restarted.defaults()[&Tool::Rectangle].stroke.width, 6.0);
    }

    #[test]
    fn a_default_set_in_another_tab_arrives_on_refresh() {
        let store = shared();
        let mut here = DrawingTemplateStore::new(store.clone());
        let mut there = DrawingTemplateStore::new(store);
        there.set_default(Tool::Circle, &style(8.0)).unwrap();
        assert!(here.defaults().is_empty());
        here.refresh(Instant::now());
        assert!(here.defaults().is_empty(), "too soon to look");
        here.refresh(Instant::now() + REFRESH_AFTER);
        assert_eq!(here.defaults()[&Tool::Circle].stroke.width, 8.0);
    }

    #[test]
    fn unreadable_defaults_are_not_replaced() {
        let store = shared();
        store
            .lock()
            .unwrap()
            .set_setting(DEFAULTS_KEY, &"not a map")
            .unwrap();
        let mut templates = DrawingTemplateStore::new(store.clone());
        assert!(templates.set_default(Tool::Arc, &style(2.0)).is_err());
        let kept: String = store.lock().unwrap().setting(DEFAULTS_KEY).unwrap();
        assert_eq!(kept, "not a map");
    }

    #[test]
    fn the_strips_options_are_remembered_and_default_to_keep_drawing() {
        let store = shared();
        let templates = DrawingTemplateStore::new(store.clone());
        assert_eq!(templates.prefs(), DrawingPrefs::default());
        assert!(templates.prefs().keep_drawing);
        let chosen = DrawingPrefs {
            magnet: MagnetMode::Strong,
            keep_drawing: false,
            group_tool: Tool::Curve,
            line_tool: Tool::SchiffPitchfork,
            note_tool: Tool::Pin,
        };
        templates.set_prefs(&chosen);
        assert_eq!(DrawingTemplateStore::new(store).prefs(), chosen);
    }

    #[test]
    fn prefs_saved_before_a_tile_open_on_its_first_tool_and_odd_tools_are_replaced() {
        let store = shared();
        store
            .lock()
            .unwrap()
            .set_setting(
                PREFS_KEY,
                &serde_json::json!({"magnet": "weak", "keep_drawing": false, "group_tool": "Arc"}),
            )
            .unwrap();
        let prefs = DrawingTemplateStore::new(store.clone()).prefs();
        assert_eq!(prefs.line_tool, Tool::TrendLine);
        assert_eq!(prefs.group_tool, Tool::Arc);
        // A tile can only show a tool its own menu holds.
        store
            .lock()
            .unwrap()
            .set_setting(
                PREFS_KEY,
                &serde_json::json!({"group_tool": "Ray", "line_tool": "Brush", "note_tool": "Arc"}),
            )
            .unwrap();
        let prefs = DrawingTemplateStore::new(store).prefs();
        assert_eq!(prefs.group_tool, Tool::Brush);
        assert_eq!(prefs.line_tool, Tool::TrendLine);
        assert_eq!(prefs.note_tool, Tool::Text);
    }
}
