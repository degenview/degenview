//! The Pine logs pane, docked on a chart's right edge: which chart and script it shows, what it
//! reads from the chart's runs, and what its controls change.
//!
//! The messages stay with the chart card's runs (`ChartCard::pine_logs`); this only lends them to
//! the view. The pane's options (levels, search flags) are remembered in the `pineLogs.prefs`
//! setting, local to the machine; the messages themselves are never stored or synced.

use super::*;
use crate::model::pine_instance::PineInstance;
use crate::model::pine_logs_view::{LogsPrefs, SourceTarget};
use crate::pine::logs::{LogEvent, LogHistory};
use crate::ui::Scrolled;
use crate::ui::pine_logs_view::{self as logs_view, Feed, LogsAction, ScriptEntry};

const PREFS_KEY: &str = "pineLogs.prefs";

/// What the pane keeps between frames.
#[derive(Default)]
pub(super) struct PineLogsPane {
    /// The chart the pane shows, when open.
    pub(super) chart: Option<u64>,
    pub(super) view: logs_view::LogsState,
    prefs_loaded: bool,
    prefs_saved: LogsPrefs,
    /// What the pane asks of the app (open the Script Manager), handed on with the frame's actions.
    outbox: Vec<WorkspaceAction>,
}

/// The scripts the pane can pick: each instance by name, the later ones of a repeated name
/// numbered so two copies of one script can be told apart.
pub(super) fn script_entries(instances: &[PineInstance]) -> Vec<ScriptEntry> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    instances
        .iter()
        .map(|instance| {
            let ordinal = seen.entry(instance.name.as_str()).or_insert(0);
            *ordinal += 1;
            ScriptEntry {
                id: instance.id,
                label: if *ordinal == 1 {
                    instance.name.clone()
                } else {
                    format!("{} #{}", instance.name, ordinal)
                },
            }
        })
        .collect()
}

/// Where a message's call is: in the saved script the instance came from, or in the library whose
/// code made it. An instance that did not come from a saved script has no source to open.
fn source_target(instance: &PineInstance, event: &LogEvent) -> Result<SourceTarget, &'static str> {
    let offset = event.range.start.offset;
    match (&event.library, &instance.script_id) {
        (Some(path), _) => Ok(SourceTarget {
            script_id: None,
            // `user/Name/version`: a library is found by its name.
            library: path
                .split('/')
                .filter(|part| !part.is_empty())
                .nth(1)
                .map(str::to_owned),
            offset,
            ran_source: None,
        }),
        (None, Some(script_id)) => Ok(SourceTarget {
            script_id: Some(script_id.clone()),
            library: None,
            offset,
            ran_source: Some(instance.source.clone()),
        }),
        (None, None) => {
            Err("This script was not added from a saved script, so there is no source to open.")
        }
    }
}

impl ChartWorkspace {
    /// Opens the pane on `chart_id`, on `instance` when one is named. An open pane is only
    /// pointed elsewhere, never opened twice.
    pub(super) fn open_pine_logs(&mut self, chart_id: u64, instance: Option<u64>) {
        self.load_logs_prefs();
        self.context.logs.chart = Some(chart_id);
        if let Some(instance) = instance {
            self.context.logs.view.selected = Some(instance);
        }
    }

    fn load_logs_prefs(&mut self) {
        let pane = &mut self.context.logs;
        if pane.prefs_loaded {
            return;
        }
        pane.prefs_loaded = true;
        let prefs = self
            .store
            .lock()
            .ok()
            .and_then(|store| store.setting::<LogsPrefs>(PREFS_KEY))
            .unwrap_or_default();
        pane.view.filter.levels = prefs.levels;
        pane.view.filter.options = prefs.search;
        pane.prefs_saved = prefs;
    }

    fn save_logs_prefs(&mut self) {
        let prefs = self.context.logs.view.prefs();
        if prefs == self.context.logs.prefs_saved {
            return;
        }
        self.context.logs.prefs_saved = prefs;
        if let Ok(store) = self.store.lock() {
            let _ = store.set_setting(PREFS_KEY, &prefs);
        }
    }

    pub(super) fn show_logs_panel(&mut self, ui: &mut egui::Ui) {
        let Some(chart_id) = self.context.logs.chart else {
            return;
        };
        let Some(index) = self.chart_index(chart_id) else {
            return;
        };
        let card = &self.charts[index].chart;
        let scripts = script_entries(card.pine_instances());
        self.context.logs.view.select_valid(&scripts);
        let selected = self.context.logs.view.selected;
        // A script that has not run yet (or is hidden) has nothing written, not nothing to show.
        let nothing = LogHistory::default();
        let feed = selected.map(|id| match card.pine_logs(id) {
            Some(logs) => Feed {
                epoch: logs.epoch,
                history: logs.history,
                logging: card.pine_logging(id),
            },
            None => Feed {
                epoch: 0,
                history: &nothing,
                logging: card.pine_logging(id),
            },
        });
        let actions = logs_view::show(ui, &mut self.context.logs.view, &scripts, feed);
        for action in actions {
            self.apply_logs_action(chart_id, action);
        }
        self.save_logs_prefs();
    }

    fn apply_logs_action(&mut self, chart_id: u64, action: LogsAction) {
        match action {
            LogsAction::Close => self.context.logs.chart = None,
            LogsAction::Select(id) => self.context.logs.view.selected = Some(id),
            LogsAction::SetLogging(id, enabled) => {
                if let Some(index) = self.chart_index(chart_id) {
                    self.charts[index].chart.set_pine_logging(id, enabled);
                }
            }
            LogsAction::Source(sequence) => self.open_log_source(chart_id, sequence),
            LogsAction::ScrollToBar(sequence) => self.scroll_to_log_bar(chart_id, sequence),
        }
    }

    /// The message with `sequence` in the selected script's history, with the instance it is from.
    fn selected_log(&self, chart_id: u64, sequence: u64) -> Option<(PineInstance, LogEvent)> {
        let card = &self.charts[self.chart_index(chart_id)?].chart;
        let id = self.context.logs.view.selected?;
        let instance = card.pine_instances().iter().find(|i| i.id == id)?.clone();
        let event = card
            .pine_logs(id)?
            .history
            .buffer()
            .iter()
            .find(|event| event.sequence == sequence)?
            .clone();
        Some((instance, event))
    }

    /// "Source code": the Script Manager on the call that wrote the message.
    fn open_log_source(&mut self, chart_id: u64, sequence: u64) {
        let Some((instance, event)) = self.selected_log(chart_id, sequence) else {
            return;
        };
        match source_target(&instance, &event) {
            Ok(target) => self
                .context
                .logs
                .outbox
                .push(WorkspaceAction::OpenPineSource(target)),
            Err(reason) => self.say(reason, true),
        }
    }

    /// "Scroll to bar": the chart goes to the bar the message was written on.
    fn scroll_to_log_bar(&mut self, chart_id: u64, sequence: u64) {
        let Some((_, event)) = self.selected_log(chart_id, sequence) else {
            return;
        };
        self.scroll_chart_to(chart_id, event.bar_time_ms);
    }

    /// Takes the chart to the bar holding `time_ms`. A bar older than the candles held is not
    /// fetched for: paging older history starts the script over and clears its messages.
    fn scroll_chart_to(&mut self, chart_id: u64, time_ms: i64) {
        let Some(index) = self.chart_index(chart_id) else {
            return;
        };
        self.focus_chart(chart_id);
        let replay_ms = self.replay_cut();
        let chart = &mut self.charts[index];
        let candles = displayed_candles(&chart.candles, replay_ms);
        match chart.chart.scroll_to_time(time_ms, candles) {
            Scrolled::Done => {}
            Scrolled::BeforeHistory => self.say(
                "That bar is older than the candles this chart holds, so it went to the oldest bar.",
                false,
            ),
            Scrolled::NoBars => self.say("The chart has no bars to scroll to.", true),
        }
    }

    pub(super) fn take_logs_actions(&mut self) -> Vec<WorkspaceAction> {
        std::mem::take(&mut self.context.logs.outbox)
    }
}

#[cfg(test)]
mod tests;
