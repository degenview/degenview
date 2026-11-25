//! The drawing tool rail beside the charts and which tool each of its grouped tiles shows.

use crate::model::{DataSource, Tool};
use crate::ui::ViewMove;
use crate::ui::workspace::tool_sidebar::{self, ToolSidebarAction};
use crate::workspace::chart::Chart;
use crate::workspace::{ChartWorkspace, WorkspaceAction};
use eframe::egui;

impl ChartWorkspace {
    pub(in crate::workspace) fn show_tool_rail(
        &mut self,
        ui: &mut egui::Ui,
    ) -> Vec<WorkspaceAction> {
        // The strip shows the remembered options, so they are read before it is drawn.
        let _ = self.drawing_templates();
        let (has_drawings, all_locked) = self.drawing_lock_state();
        let favorites = self.favorites();
        let input = tool_sidebar::ToolSidebarInput {
            active_tool: self.tools.active,
            cursor: self.tools.chosen_cursor(),
            values_tooltip: self.tools.values_tooltip(),
            zoom_armed: self.tools.zoom_armed,
            can_zoom_out: self
                .focused
                .and_then(|id| self.charts.iter().find(|chart| chart.id == id))
                .is_some_and(|chart| chart.chart.can_zoom_out()),
            group_tool: self.tools.prefs.group_tool,
            line_tool: self.tools.prefs.line_tool,
            note_tool: self.tools.prefs.note_tool,
            pattern_tool: self.tools.prefs.pattern_tool,
            forecast_tool: self.tools.prefs.forecast_tool,
            fib_tool: self.tools.prefs.fib_tool,
            magnet: self.tools.prefs.magnet,
            keep_drawing: self.tools.prefs.keep_drawing,
            hide_all: self.tools.hide_all,
            all_locked,
            has_drawings,
            paper_open: self.trade.is_panel_open(),
            has_market: self
                .charts
                .iter()
                .any(|chart| chart.is_market() && chart.source != DataSource::CoinMarketCap),
            unseen_alerts: self.alerts.unseen(),
            sync: self.sync,
            can_link: self.market_charts() > 1,
            unlinkable: self
                .charts
                .iter()
                .filter(|chart| chart.is_market() && !chart.chart.takes_part_in_sync(chart.source))
                .map(Chart::display_title)
                .collect(),
            favorites,
        };
        let mut actions = Vec::new();
        for action in tool_sidebar::show(ui, &input) {
            match action {
                ToolSidebarAction::Arm(tool) => self.arm_tool(tool),
                ToolSidebarAction::ArmZoomIn => self.arm_zoom_in(),
                ToolSidebarAction::ZoomOut => self.zoom_out_focused(),
                ToolSidebarAction::SetCursor(mode) => {
                    self.tools.choose_cursor(mode);
                    self.drop_rulers();
                }
                ToolSidebarAction::ToggleValuesTooltip => self.tools.toggle_values_tooltip(),
                ToolSidebarAction::SetMagnet(mode) => self.tools.prefs.magnet = mode,
                ToolSidebarAction::ToggleKeepDrawing => {
                    self.tools.prefs.keep_drawing = !self.tools.prefs.keep_drawing
                }
                ToolSidebarAction::ToggleHideAll => self.tools.hide_all = !self.tools.hide_all,
                ToolSidebarAction::ToggleLockAll => {
                    let (_, all_locked) = self.drawing_lock_state();
                    self.set_all_locked(!all_locked);
                }
                ToolSidebarAction::TogglePaper => {
                    self.trade.toggle_panel();
                }
                ToolSidebarAction::OpenAlerts => actions.push(WorkspaceAction::OpenAlerts),
                ToolSidebarAction::SetSync(sync) => self.set_sync(sync),
                ToolSidebarAction::ToggleFavorite(item) => self.toggle_favorite(item),
            }
        }
        self.save_drawing_prefs();
        actions
    }

    /// Back to the crosshair after the drawing layer put the held tool down (a one-shot drawing was
    /// placed, the ruler's measurement was read, or the chart was right-clicked). A measurement goes
    /// away with its tool, as on every other way of putting it down.
    pub(in crate::workspace) fn put_down_tool(&mut self) {
        self.tools.put_down();
        self.drop_rulers();
    }

    /// An open menu of the tool strip takes Escape before the charts do: the key must close it and
    /// nothing behind it (a draft, the armed tool, the selection). egui closes a popup on Escape
    /// while it draws and leaves the key to the rest of the frame, so call this before the strip is
    /// drawn (`ChartWorkspace::ui` does, first).
    pub(in crate::workspace) fn reserve_escape_for_tool_menus(&self, ctx: &egui::Context) {
        tool_sidebar::close_menu_on_escape(ctx);
    }

    /// Arms the Zoom In mode. It takes the pointer from any drawing tool, as a drawing tool takes
    /// it from Zoom In; arming it again changes nothing.
    pub(in crate::workspace) fn arm_zoom_in(&mut self) {
        if self.tools.arm_zoom() {
            self.drop_rulers();
        }
    }

    /// Puts `tool` in hand, remembering it as the one its group's tile shows. Arming the tool that
    /// is already armed changes nothing: the tiles and the menu rows never put a tool down.
    pub(in crate::workspace) fn arm_tool(&mut self, tool: Tool) {
        if self.tools.arm(tool) {
            self.drop_rulers();
        }
        self.remember_group_tool(tool);
    }

    /// Zoom out on the focused chart: back to the view its last zoom came from. It is an undo
    /// step, so Undo puts the zoom back.
    pub(in crate::workspace) fn zoom_out_focused(&mut self) {
        let focused = self.focused;
        let left = self
            .charts
            .iter_mut()
            .find(|chart| Some(chart.id) == focused)
            .and_then(|chart| chart.chart.zoom_out().map(|left| (chart.id, left)));
        if let Some((chart_id, left)) = left {
            self.remember_view(chart_id, left, ViewMove::Forward);
        }
    }

    /// Arms `tool`, or goes back to the crosshair when it is already armed (or is the crosshair):
    /// what a shortcut and the favorites bar do. The tool strip's tiles use `arm_tool`.
    pub(in crate::workspace) fn toggle_tool(&mut self, tool: Tool) {
        if tool == Tool::Cursor || self.tools.active == tool {
            self.tools.put_down();
        } else {
            self.tools.arm(tool);
        }
        self.remember_group_tool(tool);
        self.drop_rulers();
    }

    /// Remembers `tool` as the one the tile of its menu shows, when it belongs to one.
    fn remember_group_tool(&mut self, tool: Tool) {
        use crate::model::ToolGroup;
        let group = tool.spec().group;
        if ToolGroup::MENU.contains(&group) {
            self.tools.prefs.group_tool = tool;
        } else if ToolGroup::LINE_MENU.contains(&group) {
            self.tools.prefs.line_tool = tool;
        } else if ToolGroup::NOTE_MENU.contains(&group) {
            self.tools.prefs.note_tool = tool;
        } else if ToolGroup::PATTERN_MENU.contains(&group) {
            self.tools.prefs.pattern_tool = tool;
        } else if ToolGroup::FORECAST_MENU.contains(&group) {
            self.tools.prefs.forecast_tool = tool;
        } else if ToolGroup::FIB_MENU.contains(&group) {
            self.tools.prefs.fib_tool = tool;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::model::Tool;
    use crate::workspace::test_support::empty_workspace;

    #[test]
    fn zoom_in_and_a_drawing_tool_are_one_choice() {
        let mut workspace = empty_workspace(1);
        workspace.toggle_tool(Tool::TrendLine);
        assert_eq!(workspace.tools.active, Tool::TrendLine);
        // Arming Zoom In takes the pointer from the drawing tool.
        workspace.arm_zoom_in();
        assert!(workspace.tools.zoom_armed);
        assert_eq!(workspace.tools.active, Tool::Cursor);
        // Arming a drawing tool puts Zoom In down, and so does the crosshair tile.
        workspace.toggle_tool(Tool::Rectangle);
        workspace.arm_zoom_in();
        workspace.toggle_tool(Tool::Cursor);
        assert!(!workspace.tools.zoom_armed);
        // A second click on the tile keeps it armed, as on every tile.
        workspace.arm_zoom_in();
        workspace.arm_zoom_in();
        assert!(workspace.tools.zoom_armed);
    }

    #[test]
    fn arming_the_armed_ruler_keeps_its_measurement_and_arming_another_tool_takes_it() {
        use crate::model::{Anchor, Drawing, DrawingCommand};
        use crate::workspace::test_support::{chart_with_drawings, drawing_ids};
        let mut workspace = empty_workspace(1);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        workspace.arm_tool(Tool::Ruler);
        let ruler = Drawing::Ruler {
            id: 50,
            a: Anchor {
                time_ms: 0,
                price: 1.0,
            },
            b: Anchor {
                time_ms: 9,
                price: 2.0,
            },
        };
        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Add(ruler)]);
        // A click on the lit tile changes nothing: the measurement is still on the chart.
        workspace.arm_tool(Tool::Ruler);
        assert_eq!(drawing_ids(&workspace, chart), vec![1, 50]);
        // Another tool in hand takes the measurement with the Ruler, as every way of leaving it does.
        workspace.arm_tool(Tool::Brush);
        assert_eq!(drawing_ids(&workspace, chart), vec![1]);
    }

    #[test]
    fn arming_says_whether_what_is_in_hand_changed() {
        let mut workspace = empty_workspace(1);
        assert!(workspace.tools.arm(Tool::Ray));
        assert!(!workspace.tools.arm(Tool::Ray), "already in hand");
        assert!(workspace.tools.arm_zoom(), "Zoom In takes the pointer");
        assert!(!workspace.tools.arm_zoom());
        assert!(workspace.tools.arm(Tool::Ray), "and a tool takes it back");
    }

    #[test]
    fn a_tile_arms_the_tool_and_arming_it_again_changes_nothing() {
        let mut workspace = empty_workspace(1);
        workspace.arm_tool(Tool::Ray);
        workspace.arm_tool(Tool::Ray);
        assert_eq!(workspace.tools.active, Tool::Ray, "no toggle");
        // It is what the Lines tile shows from now on, and arming another one replaces it.
        workspace.arm_tool(Tool::Brush);
        assert_eq!(workspace.tools.active, Tool::Brush);
        workspace.arm_zoom_in();
        workspace.arm_tool(Tool::Rectangle);
        assert!(
            !workspace.tools.zoom_armed,
            "a tool takes the pointer from Zoom In"
        );
    }

    #[test]
    fn putting_down_lets_go_of_whatever_holds_the_pointer_and_says_whether_it_did() {
        let mut workspace = empty_workspace(1);
        assert!(!workspace.tools.put_down(), "nothing was held");
        workspace.toggle_tool(Tool::TrendLine);
        assert!(workspace.tools.put_down());
        assert_eq!(workspace.tools.active, Tool::Cursor);
        workspace.arm_zoom_in();
        assert!(workspace.tools.put_down());
        assert!(!workspace.tools.zoom_armed);
        assert!(
            !workspace.tools.put_down(),
            "and a second time holds nothing"
        );
    }

    #[test]
    fn escape_puts_down_what_is_held_before_it_clears_the_selection() {
        let mut workspace = empty_workspace(1);
        workspace.tools.select(7, vec![1]);
        workspace.toggle_tool(Tool::Rectangle);
        workspace.tools.escape();
        assert_eq!(workspace.tools.active, Tool::Cursor);
        assert!(workspace.tools.selection().is_some(), "one step per Esc");
        workspace.tools.escape();
        assert!(workspace.tools.selection().is_none());
    }

    #[test]
    fn zoom_out_with_no_focused_chart_does_nothing() {
        let mut workspace = empty_workspace(1);
        workspace.zoom_out_focused();
        assert!(!workspace.tools.zoom_armed);
    }

    #[test]
    fn picking_a_cursor_puts_down_the_tool_and_zoom_and_is_remembered() {
        use crate::model::CursorMode;
        let mut workspace = empty_workspace(1);
        assert_eq!(workspace.tools.cursor(), CursorMode::Cross);
        workspace.toggle_tool(Tool::TrendLine);
        workspace.tools.choose_cursor(CursorMode::Dot);
        assert_eq!(workspace.tools.active, Tool::Cursor);
        assert_eq!(workspace.tools.cursor(), CursorMode::Dot);
        workspace.arm_zoom_in();
        workspace.tools.choose_cursor(CursorMode::Arrow);
        assert!(!workspace.tools.zoom_armed);
        // Saved with the strip's options, so the next tab starts on it.
        workspace.save_drawing_prefs();
        assert_eq!(
            workspace.drawing_templates().prefs().cursor,
            CursorMode::Arrow
        );
    }

    #[test]
    fn an_armed_tool_or_zoom_holds_the_cross_and_the_choice_waits() {
        use crate::model::CursorMode;
        let mut workspace = empty_workspace(1);
        workspace.tools.choose_cursor(CursorMode::Arrow);
        workspace.toggle_tool(Tool::Rectangle);
        assert_eq!(workspace.tools.cursor(), CursorMode::Cross);
        assert_eq!(workspace.tools.chosen_cursor(), CursorMode::Arrow);
        workspace.toggle_tool(Tool::Cursor);
        assert_eq!(
            workspace.tools.cursor(),
            CursorMode::Arrow,
            "Esc or the strip gives it back"
        );
        workspace.arm_zoom_in();
        assert_eq!(workspace.tools.cursor(), CursorMode::Cross);
    }

    #[test]
    fn a_click_on_the_cursor_tile_puts_down_the_armed_tool_and_zoom_and_keeps_the_choice() {
        use crate::model::CursorMode;
        let mut workspace = empty_workspace(1);
        workspace.tools.choose_cursor(CursorMode::Arrow);
        // The tile's click is `SetCursor(chosen_cursor())`.
        workspace.toggle_tool(Tool::Rectangle);
        workspace
            .tools
            .choose_cursor(workspace.tools.chosen_cursor());
        assert_eq!(workspace.tools.active, Tool::Cursor);
        assert_eq!(workspace.tools.cursor(), CursorMode::Arrow);
        workspace.arm_zoom_in();
        workspace
            .tools
            .choose_cursor(workspace.tools.chosen_cursor());
        assert!(!workspace.tools.zoom_armed);
        assert_eq!(workspace.tools.cursor(), CursorMode::Arrow);
        // With nothing armed it changes nothing.
        workspace
            .tools
            .choose_cursor(workspace.tools.chosen_cursor());
        assert_eq!(workspace.tools.cursor(), CursorMode::Arrow);
    }

    #[test]
    fn the_eraser_and_demonstration_are_modes_of_the_moment_that_are_never_remembered() {
        use crate::model::CursorMode;
        for moment in [CursorMode::Eraser, CursorMode::Demonstration] {
            a_mode_of_the_moment(moment);
        }
    }

    fn a_mode_of_the_moment(moment: crate::model::CursorMode) {
        use crate::model::CursorMode;
        let mut workspace = empty_workspace(1);
        // The strip opens the saved options before anything is picked.
        let _ = workspace.drawing_templates();
        workspace.tools.choose_cursor(CursorMode::Dot);
        workspace.tools.select(1, vec![4, 5]);
        workspace.tools.choose_cursor(moment);
        assert_eq!(workspace.tools.cursor(), moment);
        assert!(
            workspace.tools.selection().is_none(),
            "nothing stays picked"
        );
        workspace.save_drawing_prefs();
        assert_eq!(
            workspace.drawing_templates().prefs().cursor,
            CursorMode::Dot,
            "the next tab starts on the resting cursor"
        );
        // Esc puts the Eraser down, back to the resting cursor.
        workspace.tools.escape();
        assert_eq!(workspace.tools.cursor(), CursorMode::Dot);
        // Arming a tool holds the cross; Esc then ends the tool, not the (resting) cursor.
        workspace.tools.choose_cursor(moment);
        workspace.toggle_tool(Tool::Rectangle);
        assert_eq!(workspace.tools.cursor(), CursorMode::Cross);
        workspace.tools.escape();
        assert_eq!(
            workspace.tools.cursor(),
            moment,
            "the mode waits under the tool"
        );
        workspace.tools.escape();
        assert_eq!(workspace.tools.cursor(), CursorMode::Dot);
    }

    #[test]
    fn the_values_tooltip_switch_is_saved_with_the_options() {
        let mut workspace = empty_workspace(1);
        let _ = workspace.drawing_templates();
        assert!(workspace.tools.values_tooltip());
        workspace.tools.toggle_values_tooltip();
        workspace.save_drawing_prefs();
        assert!(!workspace.drawing_templates().prefs().values_tooltip);
    }
}
