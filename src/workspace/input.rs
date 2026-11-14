//! Keyboard shortcuts on the workspace.

use super::ChartWorkspace;
use crate::model::Tool;
use crate::model::chart::price_scale::PriceScaleMode;
use crate::model::chart::snapshot::SnapshotAction;
use crate::ui::shortcut;
use eframe::egui;

impl ChartWorkspace {
    pub(super) fn handle_keyboard(&mut self, ui: &egui::Ui) {
        // Not under another picker, and not while typing: the field keeps Ctrl+K where it means
        // something else.
        if !self.symbol_picker.is_open()
            && !self.indicator_picker.is_open()
            && ui.input_mut(|input| shortcut::SYMBOL_SEARCH.consume(input))
        {
            self.open_symbol_picker();
            return;
        }
        // A text field (the properties dialog's) keeps its own undo.
        let typing = ui.ctx().egui_wants_keyboard_input();
        let (undo, redo) =
            ui.input(|input| (shortcut::UNDO.peek(input), shortcut::REDO.peek(input)));
        if undo && !typing {
            self.undo(ui.ctx());
        }
        if redo && !typing {
            self.redo(ui.ctx());
        }
        if !typing && ui.input_mut(|input| shortcut::HIDE_DRAWINGS.consume(input)) {
            self.hide_all_drawings = !self.hide_all_drawings;
        }
        // Alt (Option) and a letter arm a line tool, like the strip's tile does. Option+letter
        // types a character on a Mac, so a text field keeps it.
        if !typing {
            for tool in Tool::GROUPED {
                let pressed = shortcut::tool(tool)
                    .is_some_and(|shortcut| ui.input_mut(|input| shortcut.consume(input)));
                if pressed {
                    self.toggle_tool(tool);
                    self.save_drawing_prefs();
                }
            }
        }
        // Alt+P and Alt+L switch the focused chart's price scale, and press again to go back to
        // Regular. They are letters on a Mac too, so a text field keeps them.
        if !typing && let Some(chart_id) = self.resolved_focused_id() {
            for target in [PriceScaleMode::Percent, PriceScaleMode::Logarithmic] {
                let pressed = shortcut::scale_mode(target)
                    .is_some_and(|shortcut| ui.input_mut(|input| shortcut.consume(input)));
                if let Some(chart) = self.charts.iter().find(|chart| chart.id == chart_id)
                    && pressed
                {
                    self.set_scale_mode(chart_id, chart.scale_mode.toggled(target));
                }
            }
        }
        // The snapshot shortcuts act on the focused chart, the one whose frame is lit. Option+S
        // types a letter on a Mac, so they leave a text field alone.
        if !typing && let Some(chart_id) = self.resolved_focused_id() {
            for action in [
                SnapshotAction::Download,
                SnapshotAction::CopyImage,
                SnapshotAction::CopyLink,
            ] {
                let pressed = shortcut::snapshot(action)
                    .is_some_and(|shortcut| ui.input_mut(|input| shortcut.consume(input)));
                if pressed {
                    self.begin_snapshot(ui.ctx(), chart_id, action);
                }
            }
        }
        // macOS owns Save in its menu (`NativeCommand::SaveCurrentLayout`); everywhere else the
        // shortcut is handled here, in the tab that has focus.
        if !shortcut::is_mac()
            && !self.layout.is_prompting()
            && ui.input_mut(|input| shortcut::SAVE.consume(input))
        {
            self.save_layout(ui.ctx());
        }
    }
}
