//! The drawing tool rail beside the charts and which tool each of its grouped tiles shows.

use super::{ChartWorkspace, WorkspaceAction};
use crate::model::{DataSource, Tool};
use crate::ui::tool_sidebar::{self, ToolSidebarAction};
use eframe::egui;

impl ChartWorkspace {
    pub(super) fn show_tool_rail(&mut self, ui: &mut egui::Ui) -> Vec<WorkspaceAction> {
        // The strip shows the remembered options, so they are read before it is drawn.
        let _ = self.drawing_templates();
        let (has_drawings, all_locked) = self.drawing_lock_state();
        let input = tool_sidebar::ToolSidebarInput {
            active_tool: self.active_tool,
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
            paper_open: self.paper_panel_visible,
            replay_open: self.replay.is_bar_visible(),
            has_market: self
                .charts
                .iter()
                .any(|chart| chart.is_market() && chart.source != DataSource::CoinMarketCap),
            unseen_alerts: self.unseen_alerts,
        };
        let mut actions = Vec::new();
        for action in tool_sidebar::show(ui, &input) {
            match action {
                ToolSidebarAction::Select(tool) => self.toggle_tool(tool),
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
                    self.paper_panel_visible = !self.paper_panel_visible;
                }
                ToolSidebarAction::ToggleReplay => self.toggle_replay(),
                ToolSidebarAction::OpenAlerts => actions.push(WorkspaceAction::OpenAlerts),
            }
        }
        self.save_drawing_prefs();
        actions
    }

    /// Arms `tool`, or goes back to the crosshair when it is already armed (or is the crosshair).
    pub(super) fn toggle_tool(&mut self, tool: Tool) {
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
