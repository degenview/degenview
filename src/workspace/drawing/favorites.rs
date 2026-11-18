//! The favorite drawing tools: the stars in the tool menus and the floating bar of them over the
//! charts. Both are other ways to pick a tool, so a press on the bar goes through
//! `toggle_tool`, the call the tool rail's tiles use, and the tab's one armed tool is the only
//! state they share. The list and the bar's place live in the store (`store::drawing_favorites`),
//! the same for every tab and window.

use crate::lock::lock;
use crate::model::drawings::favorites::Favorite;
use crate::model::{CursorMode, Tool};
use crate::ui::workspace::favorites_bar::{self, FavoritesBarAction, FavoritesBarInput};
use crate::workspace::ChartWorkspace;
use eframe::egui::{Context, Id, Rect};
use std::sync::Arc;

impl ChartWorkspace {
    /// The starred tools and cursors, in the order they were starred.
    pub(in crate::workspace) fn favorites(&self) -> Arc<Vec<Favorite>> {
        lock(&self.store).drawing_favorites_cached()
    }

    /// Stars `item`, or takes its star away. Whatever is armed stays armed.
    pub(in crate::workspace) fn toggle_favorite(&mut self, item: Favorite) {
        // A preference: when the store cannot take it, the star simply does not change.
        let _ = lock(&self.store).toggle_favorite(item);
    }

    /// A press on a favorite's button: a tool goes through `toggle_tool` like its tile, a cursor
    /// is picked as in the cursor menu, and a lit cursor is put down (back to the cross) as a lit
    /// tool is.
    fn press_favorite(&mut self, item: Favorite) {
        match item {
            Favorite::Tool(tool) => self.toggle_tool(tool),
            Favorite::Cursor(mode) => {
                let lit = self.tools.active_tool() == Tool::Cursor
                    && !self.tools.is_zoom_armed()
                    && self.tools.cursor() == mode;
                let next = if lit { CursorMode::Cross } else { mode };
                self.tools.choose_cursor(next);
                self.drop_rulers();
            }
        }
    }

    fn favorites_bar_id(&self) -> Id {
        Id::new(("favorites-bar", self.id))
    }

    /// An open menu of the bar takes Escape before the charts do: the key must close it and
    /// nothing behind it (a draft, the armed tool, the selection). Call before the charts are drawn.
    pub(in crate::workspace) fn reserve_escape_for_favorites_menu(&self, ctx: &Context) {
        favorites_bar::close_menu_on_escape(ctx, self.favorites_bar_id());
    }

    /// Draws the bar over `area` (the charts) and applies what it asks for. It is not shown while
    /// a mode that owns the whole chart area is up.
    pub(in crate::workspace) fn show_favorites_bar(&mut self, ctx: &Context, area: Rect) {
        if self.is_selecting_portion() || self.replay.is_selecting_start() {
            return;
        }
        let (items, prefs) = {
            let store = lock(&self.store);
            (
                store.drawing_favorites_cached(),
                store.favorites_bar_cached(),
            )
        };
        let input = FavoritesBarInput {
            id: self.favorites_bar_id(),
            items: &items,
            active_tool: self.tools.active_tool(),
            cursor: self.tools.cursor(),
            zoom_armed: self.tools.is_zoom_armed(),
            prefs: *prefs,
            area,
        };
        for action in favorites_bar::show(ctx, &input) {
            match action {
                FavoritesBarAction::Select(item) => {
                    self.press_favorite(item);
                    self.save_drawing_prefs();
                }
                FavoritesBarAction::Moved(placement) => {
                    self.change_favorites_bar(|bar| bar.placement = Some(placement))
                }
                FavoritesBarAction::Hide => self.change_favorites_bar(|bar| bar.hidden = true),
            }
        }
    }

    /// Saves the bar's place or hidden flag, read from the store again first so one setting does
    /// not undo the other.
    fn change_favorites_bar(
        &mut self,
        change: impl FnOnce(&mut crate::model::drawings::favorites::BarPrefs),
    ) {
        let store = lock(&self.store);
        let mut bar = store.favorites_bar();
        change(&mut bar);
        let _ = store.set_favorites_bar(bar);
    }
}

#[cfg(test)]
mod tests {
    use crate::model::drawings::favorites::Favorite;
    use crate::model::{CursorMode, Tool};
    use crate::workspace::test_support::empty_workspace;

    #[test]
    fn a_star_is_shared_through_the_store_and_arms_nothing() {
        let mut workspace = empty_workspace(1);
        assert!(workspace.favorites().is_empty());
        workspace.toggle_tool(Tool::Rectangle);
        workspace.toggle_favorite(Tool::Brush.into());
        workspace.toggle_favorite(Tool::TrendLine.into());
        assert_eq!(
            *workspace.favorites(),
            [Favorite::Tool(Tool::Brush), Favorite::Tool(Tool::TrendLine)]
        );
        // Starring leaves the armed tool as it was.
        assert_eq!(workspace.tools.active_tool(), Tool::Rectangle);
    }

    #[test]
    fn taking_the_star_off_the_armed_tool_keeps_it_armed() {
        let mut workspace = empty_workspace(1);
        workspace.toggle_favorite(Tool::Brush.into());
        workspace.toggle_tool(Tool::Brush);
        workspace.toggle_favorite(Tool::Brush.into());
        assert!(workspace.favorites().is_empty());
        assert_eq!(workspace.tools.active_tool(), Tool::Brush);
    }

    #[test]
    fn a_cursors_star_is_the_same_list_and_its_button_picks_the_cursor_and_puts_it_down_again() {
        let mut workspace = empty_workspace(1);
        let _ = workspace.drawing_templates();
        workspace.toggle_favorite(Favorite::Cursor(CursorMode::Eraser));
        assert_eq!(
            *workspace.favorites(),
            [Favorite::Cursor(CursorMode::Eraser)]
        );
        // Starring arms nothing.
        assert_eq!(workspace.tools.cursor(), CursorMode::Cross);
        workspace.press_favorite(Favorite::Cursor(CursorMode::Eraser));
        assert_eq!(workspace.tools.cursor(), CursorMode::Eraser);
        // The lit button puts the cursor down, back to the cross.
        workspace.press_favorite(Favorite::Cursor(CursorMode::Eraser));
        assert_eq!(workspace.tools.cursor(), CursorMode::Cross);
        // A cursor button takes the pointer from an armed tool and from Zoom In.
        workspace.toggle_tool(Tool::Rectangle);
        workspace.press_favorite(Favorite::Cursor(CursorMode::Dot));
        assert_eq!(workspace.tools.active_tool(), Tool::Cursor);
        assert_eq!(workspace.tools.cursor(), CursorMode::Dot);
        workspace.toggle_zoom_in();
        workspace.press_favorite(Favorite::Cursor(CursorMode::Dot));
        assert!(
            !workspace.tools.is_zoom_armed(),
            "not lit under Zoom In: it takes the pointer"
        );
        assert_eq!(workspace.tools.cursor(), CursorMode::Dot);
    }
}
