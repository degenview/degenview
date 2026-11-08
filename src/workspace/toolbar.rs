//! The workspace toolbar and what its buttons ask for.

use super::{ChartWorkspace, WorkspaceAction};
use crate::ui::layout_prompts::NamePrompt;
use crate::ui::workspace_toolbar::{self, ToolbarAction};
use eframe::egui;

impl ChartWorkspace {
    pub(super) fn show_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
    ) -> Vec<WorkspaceAction> {
        let mut actions = Vec::new();
        let active_view_id = self.active_view_id;
        let layout_index = self.layout_index();
        let mut save_clicked = false;
        let mut copy_clicked = false;
        let mut rename_clicked = false;
        let mut autosave_toggled = false;
        let mut open_id: Option<u64> = None;
        let mut delete_id: Option<u64> = None;
        let layouts = &layout_index.layouts;
        let dirty = self.is_dirty();
        let autosave_on = active_view_id.is_some_and(|id| layout_index.autosave.contains(&id));
        // An Autosave layout also saves drawing-only edits; the toolbar's "unsaved" state does not
        // count them, as before.
        self.tick_autosave(
            ctx,
            dirty || (autosave_on && self.drawings_dirty()),
            autosave_on,
        );
        let input = workspace_toolbar::ToolbarInput {
            timeframe: self.timeframe,
            layout_name: self.active_view_name.as_deref().unwrap_or("Unnamed"),
            // Autosave keeps a layout clean in the user's eyes: "(unsaved)" only shows while
            // nothing is going to save it.
            unsaved: dirty && !(autosave_on && !self.autosave.failed),
            autosave: workspace_toolbar::AutosaveState {
                available: active_view_id.is_some(),
                on: autosave_on,
                failed: self.autosave.failed,
            },
            layouts,
            active_layout: active_view_id,
            watchlist_visible: self.watchlist_visible,
        };
        for action in workspace_toolbar::show(ui, &input) {
            match action {
                ToolbarAction::SetTimeframe(timeframe) => {
                    self.reload_for_timeframe(timeframe, ctx);
                }
                ToolbarAction::SaveLayout => save_clicked = true,
                ToolbarAction::CopyLayout => copy_clicked = true,
                ToolbarAction::RenameLayout => rename_clicked = true,
                ToolbarAction::ToggleAutosave => autosave_toggled = true,
                ToolbarAction::OpenLayout(id) => open_id = Some(id),
                ToolbarAction::DeleteLayout(id) => delete_id = Some(id),
                ToolbarAction::ToggleWatchlist => {
                    self.watchlist_visible = !self.watchlist_visible;
                }
                ToolbarAction::OpenPortfolio => actions.push(WorkspaceAction::OpenPortfolio),
                ToolbarAction::OpenPineStudio => {
                    actions.push(WorkspaceAction::OpenPineScripts);
                }
                ToolbarAction::AddChart => self.add_chart.open(),
                ToolbarAction::OpenIndicators => self.indicator_picker.open(),
            }
        }

        if save_clicked {
            self.request_save(ctx);
        }
        if autosave_toggled {
            self.set_autosave(!autosave_on);
        }
        if copy_clicked {
            self.open_name_prompt(NamePrompt::Copy, ctx);
        }
        if rename_clicked {
            self.open_name_prompt(NamePrompt::Rename, ctx);
        }
        if let Some(id) = open_id {
            self.request_open_saved_view(id, ctx);
        }
        if let Some(id) = delete_id {
            self.delete_saved_view(id);
        }
        self.show_layout_sheets(ctx);
        actions
    }
}
