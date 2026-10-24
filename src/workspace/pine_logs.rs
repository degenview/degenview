//! The Pine logs pane, docked on a chart's right edge: which chart and script it shows, what it
//! reads from the chart's runs, and what its controls change.
//!
//! The messages stay with the chart card's runs (`ChartCard::pine_logs`); this only lends them to
//! the view. The pane's options (levels, search flags) are remembered in the `pineLogs.prefs`
//! setting, local to the machine; the messages themselves are never stored or synced.

use super::*;
use crate::model::pine_instance::PineInstance;
use crate::model::pine_logs_view::LogsPrefs;
use crate::pine::logs::LogHistory;
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
        }
    }
}

#[cfg(test)]
mod tests;
