//! The one way scripts are put on a chart and changed there: each call is one undo step.

use crate::model::pine::command::{NewScript, ScriptCommand};
use crate::workspace::ChartWorkspace;
use crate::workspace::chart::Chart;
use crate::workspace::undo::Snapshot;

impl ChartWorkspace {
    /// Adds a script to a chart, after the ones already there. `None` when the chart is not one
    /// that takes indicators in this workspace.
    pub fn add_script(&mut self, chart_id: u64, script: NewScript) -> Option<u64> {
        let chart = self
            .charts
            .iter_mut()
            .find(|chart| chart.id == chart_id && Chart::takes_indicators(chart))?;
        let before = Snapshot::indicators_of(chart);
        let id = chart
            .chart
            .add_pine_instance(script.script_id, &script.name, &script.source);
        chart.chart.set_pine_instance_inputs(id, &script.inputs);
        chart.chart.set_pine_visible(id, script.visible);
        self.history.record(chart_id, before);
        Some(id)
    }

    /// Adds a script to the focused chart (the Pine Studio's "Add to Chart").
    pub fn add_script_on_focused(&mut self, script: NewScript) -> Option<u64> {
        let id = self.resolved_focused_id()?;
        self.add_script(id, script)
    }

    /// The eye of an applied script: shows it, or hides it and keeps its settings.
    pub fn toggle_script(&mut self, chart_id: u64, instance_id: u64) {
        let visible = self
            .charts
            .iter()
            .find(|chart| chart.id == chart_id)
            .and_then(|chart| {
                chart
                    .chart
                    .pine_instances()
                    .iter()
                    .find(|instance| instance.id == instance_id)
            })
            .map(|instance| instance.visible);
        if let Some(visible) = visible {
            self.edit_script(chart_id, &ScriptCommand::show(instance_id, !visible));
        }
    }

    /// Changes a script on a chart. Returns whether anything differed (only then is there an undo
    /// step), or `None` when the chart has no such script.
    pub fn edit_script(&mut self, chart_id: u64, command: &ScriptCommand) -> Option<bool> {
        let chart = self.charts.iter_mut().find(|chart| chart.id == chart_id)?;
        let instance = command.instance();
        if !chart
            .chart
            .pine_instances()
            .iter()
            .any(|candidate| candidate.id == instance)
        {
            return None;
        }
        let before = Snapshot::indicators_of(chart);
        let changed = chart.chart.apply_script_command(command);
        if changed {
            self.history.record(chart_id, before);
        }
        Some(changed)
    }
}
