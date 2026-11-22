//! The drawing tools a user starred and where the floating bar of them sits, two `setting`s shared
//! by every chart tab and window.
//!
//! The list (`drawing.favorites`) is a portable setting, so it follows the user to another Mac. The
//! bar's place and whether it is hidden (`drawing.favoritesBar`) suit one screen and stay on this
//! device (`sync::record::policy`).

use super::Store;
use crate::model::drawings::favorites::{BarPrefs, Favorite, FavoriteTools};

/// The starred tools, as their stored names.
pub const KEY: &str = "drawing.favorites";
/// The bar's remembered place and hidden flag.
pub const BAR_KEY: &str = "drawing.favoritesBar";

impl Store {
    /// The starred tools and cursors this build knows, in the order they were starred.
    pub fn drawing_favorites(&self) -> Vec<Favorite> {
        self.setting::<FavoriteTools>(KEY)
            .unwrap_or_default()
            .items()
    }

    /// Stars `item`, or takes its star away. The stored list is read again first, so a change that
    /// arrived from another Mac or window is not undone, and one this build cannot read is never
    /// written over. Starring a tool also shows the bar again when it was hidden.
    pub fn toggle_favorite(&self, item: impl Into<Favorite>) -> rusqlite::Result<()> {
        let mut favorites = self
            .setting_strict::<FavoriteTools>(KEY)?
            .unwrap_or_default();
        let starred = favorites.toggle(item);
        self.set_setting(KEY, &favorites)?;
        if starred {
            let bar = self.favorites_bar();
            if bar.hidden {
                self.set_favorites_bar(BarPrefs {
                    hidden: false,
                    ..bar
                })?;
            }
        }
        Ok(())
    }

    /// Where the bar sits and whether it is hidden; the defaults when nothing is saved.
    pub fn favorites_bar(&self) -> BarPrefs {
        self.setting(BAR_KEY).unwrap_or_default()
    }

    pub fn set_favorites_bar(&self, prefs: BarPrefs) -> rusqlite::Result<()> {
        self.set_setting(BAR_KEY, &prefs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CursorMode, Tool};

    fn tools(tools: &[Tool]) -> Vec<Favorite> {
        tools.iter().copied().map(Favorite::Tool).collect()
    }
    use crate::model::drawings::favorites::BarPlacement;
    use std::sync::Arc;

    fn store() -> Store {
        Store::open_in_memory().expect("store")
    }

    #[test]
    fn a_star_is_kept_and_comes_back_in_order() {
        let store = store();
        assert!(store.drawing_favorites().is_empty());
        store.toggle_favorite(Tool::Brush).unwrap();
        store.toggle_favorite(Tool::Highlighter).unwrap();
        assert_eq!(
            store.drawing_favorites(),
            tools(&[Tool::Brush, Tool::Highlighter])
        );
        store.toggle_favorite(Tool::Brush).unwrap();
        assert_eq!(store.drawing_favorites(), tools(&[Tool::Highlighter]));
        store.toggle_favorite(Tool::Highlighter).unwrap();
        assert!(store.drawing_favorites().is_empty());
    }

    #[test]
    fn the_stored_form_is_a_list_of_names() {
        let store = store();
        store.toggle_favorite(Tool::TrendLine).unwrap();
        let raw: Vec<String> = store.setting(KEY).unwrap();
        assert_eq!(raw, ["TrendLine"]);
    }

    #[test]
    fn a_name_a_newer_build_wrote_survives_a_change_here() {
        let store = store();
        store.set_setting(KEY, &["FutureTool", "Brush"]).unwrap();
        assert_eq!(store.drawing_favorites(), tools(&[Tool::Brush]));
        store.toggle_favorite(Tool::Circle).unwrap();
        let raw: Vec<String> = store.setting(KEY).unwrap();
        assert_eq!(raw, ["FutureTool", "Brush", "Circle"]);
    }

    #[test]
    fn a_cursor_is_starred_in_the_same_list_and_an_older_build_keeps_its_name() {
        let store = store();
        store.toggle_favorite(Tool::Brush).unwrap();
        store
            .toggle_favorite(Favorite::Cursor(CursorMode::Eraser))
            .unwrap();
        let raw: Vec<String> = store.setting(KEY).unwrap();
        assert_eq!(raw, ["Brush", "cursor:Eraser"]);
        assert_eq!(
            store.drawing_favorites(),
            [
                Favorite::Tool(Tool::Brush),
                Favorite::Cursor(CursorMode::Eraser)
            ]
        );
        store
            .toggle_favorite(Favorite::Cursor(CursorMode::Eraser))
            .unwrap();
        assert_eq!(store.drawing_favorites(), tools(&[Tool::Brush]));
    }

    #[test]
    fn a_value_that_cannot_be_read_is_not_written_over() {
        let store = store();
        store
            .set_setting(KEY, &serde_json::json!({"not": "a list"}))
            .unwrap();
        assert!(store.toggle_favorite(Tool::Brush).is_err());
        let raw: serde_json::Value = store.setting(KEY).unwrap();
        assert_eq!(raw, serde_json::json!({"not": "a list"}));
        assert!(store.drawing_favorites().is_empty());
    }

    #[test]
    fn the_bar_defaults_and_remembers_its_place() {
        let store = store();
        assert_eq!(store.favorites_bar(), BarPrefs::default());
        let prefs = BarPrefs {
            placement: Some(BarPlacement { x: 0.25, y: 0.75 }),
            window: true,
            hidden: true,
        };
        store.set_favorites_bar(prefs).unwrap();
        assert_eq!(store.favorites_bar(), prefs);
    }

    #[test]
    fn starring_a_tool_shows_a_hidden_bar_again_and_keeps_its_place() {
        let store = store();
        let place = Some(BarPlacement { x: 0.1, y: 0.2 });
        store
            .set_favorites_bar(BarPrefs {
                placement: place,
                window: true,
                hidden: true,
            })
            .unwrap();
        store.toggle_favorite(Tool::Brush).unwrap();
        let bar = store.favorites_bar();
        assert!(!bar.hidden);
        assert_eq!(bar.placement, place);
        // Taking a star away does not hide it.
        store.toggle_favorite(Tool::Brush).unwrap();
        assert!(!store.favorites_bar().hidden);
    }

    #[test]
    fn the_cached_reads_follow_a_write() {
        let store = store();
        let before = store.drawing_favorites_cached();
        assert!(Arc::ptr_eq(&before, &store.drawing_favorites_cached()));
        store.toggle_favorite(Tool::Brush).unwrap();
        let after = store.drawing_favorites_cached();
        assert_eq!(*after, tools(&[Tool::Brush]));
        assert!(!Arc::ptr_eq(&before, &after));

        let bar = store.favorites_bar_cached();
        store
            .set_favorites_bar(BarPrefs {
                placement: None,
                window: false,
                hidden: true,
            })
            .unwrap();
        assert!(store.favorites_bar_cached().hidden);
        assert!(!bar.hidden);
    }
}
