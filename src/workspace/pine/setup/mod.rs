//! Scripts whose inputs are being set from the chart (`input.time(…, confirm = true)` and the
//! like). A script being added waits here, off the chart, so nothing that reads a chart's scripts
//! (saving, sync, MCP, alerts, runs) sees one that was never confirmed: giving up just drops it.
//! "Reset points" waits here too, and changes its script only when every point is clicked. One
//! setup takes the clicks at a time, in the order they were asked for.

#[cfg(test)]
mod tests;

use crate::host::pine::declaration::ScriptDeclaration;
use crate::i18n::tr;
use crate::model::InstrumentId;
use crate::model::pine::command::{NewScript, ScriptChange, ScriptCommand};
use crate::model::pine::instance::PineInstance;
use crate::model::pine::setup::{InputSetup, Purpose, Step};
use crate::pine::inputs::points::{PointAxes, PointInput};
use crate::ui::pine::setup::dialog;
use crate::ui::{PendingScript, PickView, SetupEvent, SetupView};
use crate::workspace::ChartWorkspace;
use crate::workspace::chart::Chart;
use crate::workspace::undo::Snapshot;
use eframe::egui;
use std::collections::VecDeque;
use std::sync::Arc;

/// What became of a script a person added.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddOutcome {
    Added(u64),
    /// It will be on the chart, under this id, once its inputs are given.
    AwaitingInputs(u64),
    /// The chart takes no indicators.
    Refused,
}

enum Target {
    /// A script not yet on its chart.
    Add(Box<PineInstance>),
    /// "Reset points" of an applied script, as long as its source stays this one.
    Repick { instance: u64, source: String },
}

impl Target {
    fn instance(&self) -> u64 {
        match self {
            Self::Add(instance) => instance.id,
            Self::Repick { instance, .. } => *instance,
        }
    }
}

struct Session {
    chart_id: u64,
    /// The market the clicks are read on.
    market: InstrumentId,
    target: Target,
    name: String,
    declaration: Arc<ScriptDeclaration>,
    setup: InputSetup,
    /// The user closed the prompt for the point being asked.
    prompt_hidden: bool,
}

impl Session {
    /// The script a repick is for, if the chart still has it as it was.
    fn repicked<'a>(&self, chart: &'a Chart) -> Option<&'a PineInstance> {
        let Target::Repick { instance, source } = &self.target else {
            return None;
        };
        chart
            .chart
            .pine_instances()
            .iter()
            .find(|candidate| candidate.id == *instance && candidate.source == *source)
    }

    fn pick_view(&self, point: &PointInput) -> PickView {
        let (name, script) = (point.name.as_str(), self.name.as_str());
        let prompt = match point.axes {
            PointAxes::Both { .. } => tr!(
                "Set the “{name}” time and price for “{script}”",
                name = name,
                script = script
            ),
            PointAxes::Time(_) => tr!(
                "Set the “{name}” time for “{script}”",
                name = name,
                script = script
            ),
            PointAxes::Price(_) => tr!(
                "Set the “{name}” price for “{script}”",
                name = name,
                script = script
            ),
        };
        // A script that has run in its own pane reads prices on that pane's scale. One not yet
        // on the chart has drawn no scale, so its points are read from the price chart.
        let pane = match &self.target {
            Target::Repick { instance, .. } if !self.declaration.overlay => Some(*instance),
            _ => None,
        };
        PickView {
            axes: point.axes.clone(),
            pane,
            picked: self.setup.picked_points(),
            prompt: (!self.prompt_hidden).then_some(prompt),
        }
    }
}

#[derive(Default)]
pub(in crate::workspace) struct ScriptSetups {
    queue: VecDeque<Session>,
}

impl ScriptSetups {
    pub(in crate::workspace) fn clear(&mut self) {
        self.queue.clear();
    }

    /// Gives up every setup on a chart.
    pub(in crate::workspace) fn forget_chart(&mut self, chart_id: u64) {
        self.queue.retain(|session| session.chart_id != chart_id);
    }

    /// The chart whose clicks are being asked for.
    pub(in crate::workspace) fn asking_chart(&self) -> Option<u64> {
        self.queue
            .front()
            .filter(|session| matches!(session.setup.step(), Step::Pick(_)))
            .map(|session| session.chart_id)
    }

    /// Whether the Confirm Inputs sheet is up.
    pub(in crate::workspace) fn dialog_open(&self) -> bool {
        self.queue
            .front()
            .is_some_and(|session| session.setup.step() == Step::Confirm)
    }

    pub(in crate::workspace) fn view(&self, chart_id: u64) -> SetupView {
        let pending = self
            .queue
            .iter()
            .filter(|session| session.chart_id == chart_id)
            .filter_map(|session| match &session.target {
                Target::Add(instance) => Some(PendingScript {
                    id: instance.id,
                    name: instance.name.clone(),
                    overlay: session.declaration.overlay,
                }),
                Target::Repick { .. } => None,
            })
            .collect();
        let pick = self
            .queue
            .front()
            .filter(|session| session.chart_id == chart_id)
            .and_then(|session| match session.setup.step() {
                Step::Pick(point) => Some(session.pick_view(point)),
                _ => None,
            });
        SetupView { pending, pick }
    }
}

impl ChartWorkspace {
    /// A person adds a script to a chart. One that marks inputs `confirm` is asked for them
    /// first, and goes on the chart when they are given.
    pub fn add_script(&mut self, chart_id: u64, script: NewScript) -> AddOutcome {
        let Some(chart) = self
            .charts
            .iter_mut()
            .find(|chart| chart.id == chart_id && Chart::takes_indicators(chart))
        else {
            return AddOutcome::Refused;
        };
        let declaration = chart.chart.declaration(&script.source);
        let Some(setup) = declaration
            .valid
            .then(|| InputSetup::adding(&declaration.schema))
            .flatten()
        else {
            return match self.insert_script(chart_id, script) {
                Some(id) => AddOutcome::Added(id),
                None => AddOutcome::Refused,
            };
        };
        let id = chart.chart.reserve_instance_id();
        let mut instance = PineInstance::new(id, script.script_id, &script.name, &script.source);
        instance.set_input_values(&script.inputs);
        instance.visible = script.visible;
        let market = InstrumentId::new(chart.source, &chart.symbol);
        self.begin_setup(Session {
            chart_id,
            market,
            name: declaration.title.clone().unwrap_or(script.name),
            target: Target::Add(Box::new(instance)),
            declaration,
            setup,
            prompt_hidden: false,
        });
        AddOutcome::AwaitingInputs(id)
    }

    /// "Reset points": asks for every point of an applied script again.
    pub fn reset_points(&mut self, chart_id: u64, instance_id: u64) {
        if self
            .script_setups
            .queue
            .iter()
            .any(|session| session.chart_id == chart_id && session.target.instance() == instance_id)
        {
            return;
        }
        let Some(chart) = self.charts.iter().find(|chart| chart.id == chart_id) else {
            return;
        };
        let Some(instance) = chart
            .chart
            .pine_instances()
            .iter()
            .find(|instance| instance.id == instance_id)
        else {
            return;
        };
        let declaration = chart.chart.declaration(&instance.source);
        let Some(setup) = InputSetup::repicking(&declaration.schema) else {
            return;
        };
        let session = Session {
            chart_id,
            market: InstrumentId::new(chart.source, &chart.symbol),
            name: declaration
                .title
                .clone()
                .unwrap_or_else(|| instance.name.clone()),
            target: Target::Repick {
                instance: instance_id,
                source: instance.source.clone(),
            },
            declaration,
            setup,
            prompt_hidden: false,
        };
        self.begin_setup(session);
    }

    /// Whatever covers or holds the chart gives way, since the next thing wanted is a click on it.
    fn begin_setup(&mut self, session: Session) {
        let chart_id = session.chart_id;
        self.script_setups.queue.push_back(session);
        self.settings_chart = None;
        self.script_popover = None;
        self.overlay_popover = None;
        self.indicator_picker.close();
        self.put_down_tool();
        self.focus_chart(chart_id);
        if let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == chart_id) {
            chart.chart.restore_panes();
        }
    }

    /// Gives up the setup that is being asked: a script being added is not added, a reset leaves
    /// its script as it was. Returns whether there was one.
    pub(in crate::workspace) fn cancel_setup(&mut self) -> bool {
        self.script_setups.queue.pop_front().is_some()
    }

    /// Takes a waiting script off a chart before it was ever added.
    pub(in crate::workspace) fn cancel_pending_script(
        &mut self,
        chart_id: u64,
        instance_id: u64,
    ) -> bool {
        let before = self.script_setups.queue.len();
        self.script_setups.queue.retain(|session| {
            !(session.chart_id == chart_id
                && matches!(&session.target, Target::Add(instance) if instance.id == instance_id))
        });
        self.script_setups.queue.len() != before
    }

    /// Drops what no longer has its chart or script, and starts over a script being added whose
    /// chart shows another market now (the clicks were for the old one).
    pub(in crate::workspace) fn reconcile_setups(&mut self) {
        let charts = &self.charts;
        self.script_setups.queue.retain_mut(|session| {
            let Some(chart) = charts
                .iter()
                .find(|chart| chart.id == session.chart_id && Chart::takes_indicators(chart))
            else {
                return false;
            };
            let market = InstrumentId::new(chart.source, &chart.symbol);
            let moved = market != session.market;
            match session.setup.purpose() {
                Purpose::Add if moved => {
                    let Some(setup) = InputSetup::adding(&session.declaration.schema) else {
                        return false;
                    };
                    session.setup = setup;
                    session.market = market;
                    session.prompt_hidden = false;
                    true
                }
                Purpose::Add => true,
                Purpose::Repick => !moved && session.repicked(chart).is_some(),
            }
        });
    }

    /// Takes what a chart's card reports: a clicked point, or the prompt put away.
    pub(in crate::workspace) fn apply_setup_events(&mut self, chart_id: u64) {
        let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == chart_id) else {
            return;
        };
        for event in chart.chart.take_setup_events() {
            let Some(session) = self
                .script_setups
                .queue
                .front_mut()
                .filter(|session| session.chart_id == chart_id)
            else {
                break;
            };
            match event {
                SetupEvent::Picked { time_ms, price } => {
                    session.setup.pick(time_ms, price);
                    session.prompt_hidden = false;
                }
                SetupEvent::PromptClosed => session.prompt_hidden = true,
            }
        }
        self.finish_setups();
    }

    /// The Confirm Inputs sheet, while the setup being asked is at that step.
    pub(in crate::workspace) fn show_setup_dialog(&mut self, ctx: &egui::Context) {
        let Some(session) = self
            .script_setups
            .queue
            .front_mut()
            .filter(|session| session.setup.step() == Step::Confirm)
        else {
            return;
        };
        let held = match &session.target {
            Target::Add(instance) => instance.input_values(),
            Target::Repick { .. } => Default::default(),
        };
        let values = session.setup.values_on(&held);
        let outcome = dialog::show(ctx, session.chart_id, session.setup.schema(), &values);
        for edit in outcome.edits {
            session.setup.edit(edit);
        }
        match outcome.answer {
            Some(dialog::Answer::Apply) => session.setup.confirm(),
            Some(dialog::Answer::Cancel) => {
                self.cancel_setup();
            }
            None => {}
        }
        self.finish_setups();
    }

    /// Applies the setups that have everything they asked for, each as one undo step.
    fn finish_setups(&mut self) {
        while self
            .script_setups
            .queue
            .front()
            .is_some_and(|session| session.setup.step() == Step::Done)
        {
            let Some(session) = self.script_setups.queue.pop_front() else {
                return;
            };
            let Some(chart) = self
                .charts
                .iter_mut()
                .find(|chart| chart.id == session.chart_id)
            else {
                continue;
            };
            match session.target {
                Target::Add(ref instance) => {
                    let mut instance = instance.as_ref().clone();
                    instance.set_input_values(&session.setup.values_on(&instance.input_values()));
                    let before = Snapshot::indicators_of(chart);
                    chart.chart.insert_pine_instance(instance);
                    self.history.record(session.chart_id, before);
                }
                Target::Repick { instance, .. } => {
                    let Some(held) = session.repicked(chart).map(PineInstance::input_values) else {
                        continue;
                    };
                    let change = ScriptChange {
                        inputs: Some(session.setup.values_on(&held)),
                        ..ScriptChange::default()
                    };
                    self.edit_script(session.chart_id, &ScriptCommand::Change(instance, change));
                }
            }
        }
    }
}
