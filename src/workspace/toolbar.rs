//! The workspace toolbar and what its buttons ask for.

use super::alerts::AlertRequest;
use super::{ChartWorkspace, WorkspaceAction};
use crate::model::{ChartType, DataSource};
use crate::ui::chart_settings::ChartSettingsState;
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
        let symbol = self
            .focused_market_chart()
            .map(super::chart::Chart::display_title);
        let hosting_note = match self.snapshot_hosting() {
            crate::host::snapshot_host::Hosting::Ready(_) => None,
            crate::host::snapshot_host::Hosting::Unavailable(reason) => Some(reason),
        };
        let snapshot_shortcut =
            |action| crate::ui::shortcut::snapshot(action).map(|shortcut| shortcut.label());
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
            symbol: symbol.as_deref(),
            chart: self
                .focused_market_chart()
                .map(|chart| workspace_toolbar::ToolbarChart {
                    id: chart.id,
                    chart_type: chart.chart.effective_chart_type(chart.source),
                    source: chart.source,
                    chart_types: !chart.source.is_line_chart(),
                    settings: chart.source != DataSource::CoinMarketCap,
                    compare: self.can_compare(chart.id),
                }),
            can_undo: !self.undo.is_empty(),
            can_redo: !self.redo.is_empty(),
            replay_open: self.replay.is_bar_visible(),
            can_replay: self
                .charts
                .iter()
                .any(|chart| chart.is_market() && chart.source != DataSource::CoinMarketCap),
            snapshot: crate::ui::snapshot_menu::SnapshotMenu {
                busy: self.snapshot_busy(),
                working: self.snapshot_working(),
                hosting_unavailable: hosting_note.as_deref(),
                shortcut: &snapshot_shortcut,
            },
        };
        for action in workspace_toolbar::show(ui, &input) {
            match action {
                ToolbarAction::SetTimeframe(timeframe) => {
                    let changed = self.timeframe != timeframe;
                    self.reload_for_timeframe(timeframe, ctx);
                    if changed {
                        self.announce_timeframe();
                    }
                }
                ToolbarAction::OpenSymbolSearch => self.open_symbol_picker(),
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
                ToolbarAction::SetChartType(chart_type) => {
                    self.set_focused_chart_type(chart_type);
                }
                ToolbarAction::OpenChartSettings => self.open_focused_chart_settings(),
                ToolbarAction::ToggleReplay => self.toggle_replay(),
                ToolbarAction::CreateAlert => self.create_alert_for_focused_chart(),
                ToolbarAction::Snapshot(action) => {
                    if let Some(chart_id) = self.focused_market_chart().map(|chart| chart.id) {
                        self.begin_snapshot(ctx, chart_id, action);
                    }
                }
                ToolbarAction::SnapshotSettings => self.snapshot.settings_requested = true,
                ToolbarAction::Undo => self.undo(),
                ToolbarAction::Redo => self.redo(),
                ToolbarAction::OpenIndicators => self.indicator_picker.open(),
                ToolbarAction::CompareSymbol => self.open_compare_picker(),
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

    /// Gives the focused market chart `chart_type`, and no other chart. A type its market cannot
    /// draw is refused (the menu greys those rows), as is a tab with no focused market chart.
    pub(super) fn set_focused_chart_type(&mut self, chart_type: ChartType) -> bool {
        let Some(chart_id) = self.focused_market_chart().map(|chart| chart.id) else {
            return false;
        };
        matches!(self.set_chart_type_for_mcp(chart_id, chart_type), Ok(true))
    }

    /// Starts a price alert on the focused market chart's market, as the card's bell used to. A
    /// market with no price feed (a CoinMarketCap gauge) is refused.
    pub(super) fn create_alert_for_focused_chart(&mut self) {
        if let Some(chart) = self.focused_market_chart()
            && crate::ui::supports_alerts(chart.source)
        {
            self.new_alert_request = Some(AlertRequest::market(Self::alert_asset(chart)));
        }
    }

    /// Opens the settings sheet of the focused market chart, as the header's title does.
    pub(super) fn open_focused_chart_settings(&mut self) {
        if let Some(chart_id) = self.focused_market_chart().map(|chart| chart.id) {
            self.settings_state = ChartSettingsState::default();
            self.settings_chart = Some(chart_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{chart_with_drawings, empty_workspace};
    use crate::model::ChartType;

    #[test]
    fn the_toolbar_chart_type_changes_the_focused_chart_alone() {
        let mut workspace = empty_workspace(1);
        let first = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        let second = chart_with_drawings(&mut workspace, "ETHUSDT", &[]);
        let type_of = |workspace: &super::ChartWorkspace, id: u64| {
            let chart = workspace
                .charts
                .iter()
                .find(|chart| chart.id == id)
                .unwrap();
            chart.chart.effective_chart_type(chart.source)
        };
        workspace.focused = Some(second);
        assert!(workspace.set_focused_chart_type(ChartType::HeikinAshi));
        assert_eq!(type_of(&workspace, second), ChartType::HeikinAshi);
        assert_eq!(type_of(&workspace, first), ChartType::Candles);
        // The same type again changes nothing.
        assert!(!workspace.set_focused_chart_type(ChartType::HeikinAshi));
    }

    #[test]
    fn the_toolbar_acts_on_nothing_without_a_focused_market_chart() {
        let mut workspace = empty_workspace(1);
        assert!(!workspace.set_focused_chart_type(ChartType::Line));
        workspace.open_focused_chart_settings();
        assert_eq!(workspace.settings_chart, None);
    }

    #[test]
    fn the_toolbar_settings_button_opens_the_focused_charts_sheet() {
        let mut workspace = empty_workspace(1);
        let first = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        let second = chart_with_drawings(&mut workspace, "ETHUSDT", &[]);
        workspace.focused = Some(first);
        workspace.open_focused_chart_settings();
        assert_eq!(workspace.settings_chart, Some(first));
        workspace.focused = Some(second);
        workspace.open_focused_chart_settings();
        assert_eq!(workspace.settings_chart, Some(second));
    }

    #[test]
    fn the_toolbar_alert_starts_an_alert_on_the_focused_market() {
        let mut workspace = empty_workspace(1);
        workspace.create_alert_for_focused_chart();
        assert!(workspace.new_alert_request.is_none(), "no chart, no alert");
        let first = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        let second = chart_with_drawings(&mut workspace, "ETHUSDT", &[]);
        workspace.focused = Some(first);
        workspace.create_alert_for_focused_chart();
        let request = workspace.new_alert_request.take().expect("a request");
        assert_eq!(request.asset.instrument.symbol, "BTCUSDT");
        workspace.focused = Some(second);
        workspace.create_alert_for_focused_chart();
        let request = workspace.new_alert_request.take().expect("a request");
        assert_eq!(request.asset.instrument.symbol, "ETHUSDT");
    }
}
