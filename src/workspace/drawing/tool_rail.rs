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
        let input = tool_sidebar::ToolSidebarInput {
            active_tool: self.active_tool,
            zoom_armed: self.zoom_armed,
            can_zoom_out: self
                .focused
                .and_then(|id| self.charts.iter().find(|chart| chart.id == id))
                .is_some_and(|chart| chart.chart.can_zoom_out()),
            group_tool: self.group_tool,
            line_tool: self.line_tool,
            note_tool: self.note_tool,
            pattern_tool: self.pattern_tool,
            forecast_tool: self.forecast_tool,
            fib_tool: self.fib_tool,
            magnet: self.magnet,
            keep_drawing: self.keep_drawing,
            hide_all: self.hide_all_drawings,
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
        };
        let mut actions = Vec::new();
        for action in tool_sidebar::show(ui, &input) {
            match action {
                ToolSidebarAction::Select(tool) => self.toggle_tool(tool),
                ToolSidebarAction::ToggleZoomIn => self.toggle_zoom_in(),
                ToolSidebarAction::ZoomOut => self.zoom_out_focused(),
                ToolSidebarAction::Arm(tool) => {
                    self.active_tool = tool;
                    self.remember_group_tool(tool);
                    self.drop_rulers();
                }
                ToolSidebarAction::SetMagnet(mode) => self.magnet = mode,
                ToolSidebarAction::ToggleKeepDrawing => self.keep_drawing = !self.keep_drawing,
                ToolSidebarAction::ToggleHideAll => {
                    self.hide_all_drawings = !self.hide_all_drawings
                }
                ToolSidebarAction::ToggleLockAll => {
                    let (_, all_locked) = self.drawing_lock_state();
                    self.set_all_locked(!all_locked);
                }
                ToolSidebarAction::TogglePaper => {
                    self.trade.toggle_panel();
                }
                ToolSidebarAction::OpenAlerts => actions.push(WorkspaceAction::OpenAlerts),
                ToolSidebarAction::SetSync(sync) => self.set_sync(sync),
            }
        }
        self.save_drawing_prefs();
        actions
    }

    /// Arms the Zoom In mode, or puts it down when it is armed. It takes the pointer from any
    /// drawing tool, as a drawing tool takes it from Zoom In.
    pub(in crate::workspace) fn toggle_zoom_in(&mut self) {
        self.zoom_armed = !self.zoom_armed;
        if self.zoom_armed {
            self.active_tool = Tool::Cursor;
            self.drop_rulers();
        }
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

    /// Arms `tool`, or goes back to the crosshair when it is already armed (or is the crosshair).
    pub(in crate::workspace) fn toggle_tool(&mut self, tool: Tool) {
        if tool == Tool::Cursor {
            self.zoom_armed = false;
        }
        self.active_tool = if self.active_tool == tool || tool == Tool::Cursor {
            Tool::Cursor
        } else {
            tool
        };
        self.remember_group_tool(tool);
        self.drop_rulers();
    }

    /// Remembers `tool` as the one the tile of its menu shows, when it belongs to one.
    fn remember_group_tool(&mut self, tool: Tool) {
        use crate::model::ToolGroup;
        let group = tool.spec().group;
        if ToolGroup::MENU.contains(&group) {
            self.group_tool = tool;
        } else if ToolGroup::LINE_MENU.contains(&group) {
            self.line_tool = tool;
        } else if ToolGroup::NOTE_MENU.contains(&group) {
            self.note_tool = tool;
        } else if ToolGroup::PATTERN_MENU.contains(&group) {
            self.pattern_tool = tool;
        } else if ToolGroup::FORECAST_MENU.contains(&group) {
            self.forecast_tool = tool;
        } else if ToolGroup::FIB_MENU.contains(&group) {
            self.fib_tool = tool;
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
        assert_eq!(workspace.active_tool, Tool::TrendLine);
        // Arming Zoom In takes the pointer from the drawing tool.
        workspace.toggle_zoom_in();
        assert!(workspace.zoom_armed);
        assert_eq!(workspace.active_tool, Tool::Cursor);
        // Arming a drawing tool puts Zoom In down, and so does the crosshair tile.
        workspace.toggle_tool(Tool::Rectangle);
        workspace.toggle_zoom_in();
        workspace.toggle_tool(Tool::Cursor);
        assert!(!workspace.zoom_armed);
        // A second click on the tile puts it down.
        workspace.toggle_zoom_in();
        workspace.toggle_zoom_in();
        assert!(!workspace.zoom_armed);
    }

    #[test]
    fn zoom_out_with_no_focused_chart_does_nothing() {
        let mut workspace = empty_workspace(1);
        workspace.zoom_out_focused();
        assert!(!workspace.zoom_armed);
    }
}
