//! The chart settings window and applying one chart's appearance to the others.

use super::{ChartWorkspace, WorkspaceAction};
use crate::model::DataSource;
use crate::ui::chart_settings::{self, ChartSettingsAction, ChartSettingsContext};
use eframe::egui;

impl ChartWorkspace {
    /// The chart settings sheet over the focused window; returns what the workspace's host should
    /// do (opening the Script Manager).
    pub(super) fn show_settings_window(&mut self, ctx: &egui::Context) -> Vec<WorkspaceAction> {
        let Some(chart_id) = self.settings_chart else {
            return Vec::new();
        };
        let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == chart_id) else {
            self.settings_chart = None;
            return Vec::new();
        };
        let title = chart.display_title();
        let outcome = chart_settings::show(
            ctx,
            &mut self.settings_state,
            ChartSettingsContext {
                title: &title,
                symbol: &chart.symbol,
                source: chart.source,
                loaded_candles: chart.candles.len(),
                indicators: &mut chart.indicators,
                card: &mut chart.chart,
                scale_mode: chart.scale_mode,
                store: &self.store,
            },
        );
        let mut actions = Vec::new();
        match outcome {
            Some(ChartSettingsAction::Done) => {
                self.settings_chart = None;
                self.script_popover = None;
            }
            Some(ChartSettingsAction::RemoveChart) => {
                self.settings_chart = None;
                self.script_popover = None;
                self.remove_chart(chart_id);
            }
            Some(ChartSettingsAction::OpenScriptManager) => {
                self.settings_chart = None;
                self.script_popover = None;
                actions.push(WorkspaceAction::OpenPineScripts);
            }
            Some(ChartSettingsAction::ConfigureScript(id, anchor)) => {
                self.toggle_script_popover(chart_id, id, anchor);
            }
            Some(ChartSettingsAction::SetScaleMode(mode)) => self.set_scale_mode(chart_id, mode),
            Some(ChartSettingsAction::ApplyToAll(scope)) => {
                let appearance = self
                    .charts
                    .iter()
                    .find(|chart| chart.id == chart_id)
                    .map(|chart| chart.chart.appearance());
                if let Some(appearance) = appearance {
                    let message = self.apply_appearance_to_all(chart_id, &appearance, scope);
                    self.settings_state.say(message);
                }
            }
            None => {}
        }
        actions
    }

    /// Gives every other chart of this workspace `appearance`. The plan is made first, for every
    /// chart, so none is left half-restyled; each chart then takes it through its own card, so a
    /// new chart type resets that chart's scroll and bars the way a pick in its selector does.
    /// Other workspaces, saved layouts and everything else a chart holds are not touched.
    fn apply_appearance_to_all(
        &mut self,
        source_chart: u64,
        appearance: &crate::model::ChartAppearance,
        scope: crate::model::chart_template::ApplyScope,
    ) -> String {
        let markets: Vec<(u64, DataSource)> = self
            .charts
            .iter()
            .filter(|chart| chart.is_market())
            .map(|chart| (chart.id, chart.source))
            .collect();
        let plan = crate::model::chart_template::plan_apply_to_all(
            &markets,
            appearance,
            scope,
            source_chart,
        );
        for (id, step) in &plan.steps {
            if let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == *id) {
                chart.chart.apply_template(appearance, *step);
            }
        }
        plan.message(appearance.style.chart_type.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::InstrumentId;
    use crate::workspace::test_support::empty_workspace;

    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn apply_to_all_restyles_the_other_charts_of_this_workspace_only() {
        use crate::model::canvas::BackgroundMode;
        use crate::model::chart_template::ApplyScope;
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(3);
        let mut control = empty_workspace(4);
        let ids: Vec<u64> = ["BTCUSDT", "ETHUSDT", "SOLUSDT"]
            .iter()
            .map(|symbol| {
                workspace
                    .add_chart_for(&InstrumentId::new(DataSource::Binance, *symbol), &ctx)
                    .chart_id
            })
            .collect();
        let other = control
            .add_chart_for(&InstrumentId::new(DataSource::Binance, "XRPUSDT"), &ctx)
            .chart_id;
        let untouched = control.charts[0].chart.appearance();
        // The second chart carries a script and a Kagi type of its own.
        workspace.charts[1].chart.add_pine_instance(
            None,
            "RSI",
            "//@version=6\nindicator(\"x\")\nplot(close)\n",
        );
        workspace.charts[1]
            .chart
            .set_chart_type(crate::model::ChartType::Kagi);
        workspace.mark_clean();

        let mut look = crate::model::ChartAppearance::default();
        look.bullish = [9, 8, 7];
        look.canvas.background.mode = BackgroundMode::Gradient;
        look.canvas.margins.right = 21;
        look.style.chart_type = crate::model::ChartType::Renko;
        let message = workspace.apply_appearance_to_all(ids[0], &look, ApplyScope::ColorsAndCanvas);
        assert_eq!(message, "Applied the appearance to 2 charts.");
        for chart in &workspace.charts[1..] {
            let appearance = chart.chart.appearance();
            assert_eq!(appearance.bullish, [9, 8, 7]);
            assert_eq!(appearance.canvas, look.canvas);
        }
        // Colours and canvas only: each chart keeps the type it had.
        assert_eq!(
            workspace.charts[1].chart.appearance().style.chart_type,
            crate::model::ChartType::Kagi
        );
        assert_eq!(
            workspace.charts[2].chart.appearance().style.chart_type,
            crate::model::ChartType::Candles
        );
        assert_eq!(
            workspace
                .charts
                .iter()
                .map(|c| c.symbol.as_str())
                .collect::<Vec<_>>(),
            ["BTCUSDT", "ETHUSDT", "SOLUSDT"]
        );
        assert_eq!(workspace.charts[1].chart.pine_instances().len(), 1);
        assert!(workspace.is_dirty(), "the layout notices the new look");
        // The source chart and the other workspace are untouched.
        assert_eq!(
            workspace.charts[0].chart.appearance().bullish,
            crate::model::ChartAppearance::default().bullish
        );
        assert_eq!(control.charts[0].chart.appearance(), untouched);
        assert_eq!(control.charts[0].id, other);

        // With the type too, charts whose market can draw it take it.
        workspace.apply_appearance_to_all(ids[0], &look, ApplyScope::AppearanceAndCanvas);
        assert_eq!(
            workspace.charts[2].chart.appearance().style.chart_type,
            crate::model::ChartType::Renko
        );
        let alone = control.apply_appearance_to_all(other, &look, ApplyScope::AppearanceAndCanvas);
        assert_eq!(alone, "There are no other charts in this workspace.");
    }
}
