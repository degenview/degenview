//! The favorite drawing tools: the stars in the tool menus and the floating bar of them over the
//! charts. Both are other ways to pick a tool, so a press on the bar goes through
//! `toggle_tool`, the call the tool rail's tiles use, and the tab's one armed tool is the only
//! state they share. The list and the bar's place live in the store (`store::drawing_favorites`),
//! the same for every tab and window.

use crate::lock::lock;
use crate::model::Tool;
use crate::ui::workspace::favorites_bar::{self, FavoritesBarAction, FavoritesBarInput};
use crate::workspace::ChartWorkspace;
use eframe::egui::{Context, Id, Rect};
use std::sync::Arc;

impl ChartWorkspace {
    /// The starred tools, in the order they were starred.
    pub(in crate::workspace) fn favorite_tools(&self) -> Arc<Vec<Tool>> {
        lock(&self.store).drawing_favorites_cached()
    }

    /// Stars `tool`, or takes its star away. Whatever is armed stays armed.
    pub(in crate::workspace) fn toggle_favorite_tool(&mut self, tool: Tool) {
        // A preference: when the store cannot take it, the star simply does not change.
        let _ = lock(&self.store).toggle_favorite_tool(tool);
    }

    /// Draws the bar over `area` (the charts) and applies what it asks for. It is not shown while
    /// a mode that owns the whole chart area is up.
    pub(in crate::workspace) fn show_favorites_bar(&mut self, ctx: &Context, area: Rect) {
        if self.is_selecting_portion() || self.replay.is_selecting_start() {
            return;
        }
        let (tools, prefs) = {
            let store = lock(&self.store);
            (
                store.drawing_favorites_cached(),
                store.favorites_bar_cached(),
            )
        };
        let input = FavoritesBarInput {
            id: Id::new(("favorites-bar", self.id)),
            tools: &tools,
            active_tool: self.tools.active_tool(),
            prefs: *prefs,
            area,
        };
        for action in favorites_bar::show(ctx, &input) {
            match action {
                FavoritesBarAction::Select(tool) => {
                    self.toggle_tool(tool);
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
    use crate::model::Tool;
    use crate::workspace::test_support::empty_workspace;

    #[test]
    fn a_star_is_shared_through_the_store_and_arms_nothing() {
        let mut workspace = empty_workspace(1);
        assert!(workspace.favorite_tools().is_empty());
        workspace.toggle_tool(Tool::Rectangle);
        workspace.toggle_favorite_tool(Tool::Brush);
        workspace.toggle_favorite_tool(Tool::TrendLine);
        assert_eq!(*workspace.favorite_tools(), [Tool::Brush, Tool::TrendLine]);
        // Starring leaves the armed tool as it was.
        assert_eq!(workspace.tools.active_tool(), Tool::Rectangle);
    }

    #[test]
    fn taking_the_star_off_the_armed_tool_keeps_it_armed() {
        let mut workspace = empty_workspace(1);
        workspace.toggle_favorite_tool(Tool::Brush);
        workspace.toggle_tool(Tool::Brush);
        workspace.toggle_favorite_tool(Tool::Brush);
        assert!(workspace.favorite_tools().is_empty());
        assert_eq!(workspace.tools.active_tool(), Tool::Brush);
    }
}
