//! The Logs tab of the Script Manager's drawer, under the preview chart: what the script in the
//! editor wrote on the preview chart, next to the code that wrote it.
//!
//! It is the chart workspace's pane (`ui::pine_logs_view`, without its title) over the preview's
//! run, so its filters and rows are the same. "Source code" lands in the editor, and "Scroll to
//! bar" moves the preview chart, which only holds the candles it fetched.

use super::{Cmd, ScriptManager};
use crate::model::pine_logs_view::{LogsPrefs, SourceTarget};
use crate::pine::logs::LogHistory;
use crate::store::SharedStore;
use crate::ui::pine_logs_view::{self as view, Feed, LogsAction, LogsState, ScriptEntry};
use crate::ui::{PREVIEW_INSTANCE, Scrolled};
use eframe::egui::Ui;
use std::cell::RefCell;

const PREFS_KEY: &str = "pineLogs.prefs";

/// What the tab keeps between frames.
pub(super) struct LogsPane {
    pub(super) view: LogsState,
    prefs_saved: LogsPrefs,
}

impl LogsPane {
    pub(super) fn load(shared: &SharedStore) -> Self {
        let prefs = shared
            .lock()
            .ok()
            .and_then(|store| store.setting::<LogsPrefs>(PREFS_KEY))
            .unwrap_or_default();
        let mut view = LogsState::default();
        view.embedded = true;
        view.filter.levels = prefs.levels;
        view.filter.options = prefs.search;
        Self {
            view,
            prefs_saved: prefs,
        }
    }
}

/// The tab's content: the pane over the preview's run of the script being edited.
pub(super) fn tab(
    ui: &mut Ui,
    preview: &super::workspace::PreviewState,
    debug: &super::debug::DebugPanel,
    state: &mut LogsState,
    name: &str,
    cmds: &RefCell<Vec<Cmd>>,
) {
    // While there is a debugging session its messages are shown: the script's own `log.*()` calls,
    // and the logpoints. The pane is the same; only what feeds it changes.
    let debugging = debug.has_logs();
    let scripts = vec![ScriptEntry {
        id: PREVIEW_INSTANCE,
        label: if debugging {
            format!("{name} (debugger)")
        } else {
            name.to_owned()
        },
    }];
    state.select_valid(&scripts);
    let nothing = LogHistory::default();
    let feed = if debugging {
        Feed {
            epoch: debug.log_epoch,
            history: &debug.logs,
            logging: true,
        }
    } else {
        match preview.logs() {
            Some(logs) => Feed {
                epoch: logs.epoch,
                history: logs.history,
                logging: preview.logging(),
            },
            None => Feed {
                epoch: 0,
                history: &nothing,
                logging: preview.logging(),
            },
        }
    };
    for action in view::show(ui, state, &scripts, Some(feed)) {
        cmds.borrow_mut().push(Cmd::Logs(action));
    }
}

impl ScriptManager {
    pub(super) fn save_logs_prefs(&mut self) {
        let prefs = self.logs.view.prefs();
        if prefs != self.logs.prefs_saved {
            self.logs.prefs_saved = prefs;
            if let Ok(store) = self.shared.lock() {
                let _ = store.set_setting(PREFS_KEY, &prefs);
            }
        }
    }

    /// Once a frame, before the panes draw: tells the tab whether the chart waits for a drag, takes
    /// the bars the drag marked, and hands the chart the band to show.
    pub(super) fn sync_logs_range(&mut self) {
        self.logs.view.picking = self.preview.is_picking_range();
        if let Some((from_ms, to_ms)) = self.preview.take_picked_range() {
            self.logs.view.set_range(from_ms, to_ms);
        }
        self.preview.set_logs_range(
            self.logs
                .view
                .filter
                .range
                .map(|range| (range.from_ms, range.to_ms)),
        );
    }

    pub(super) fn apply_logs_action(&mut self, action: LogsAction) {
        match action {
            LogsAction::Close | LogsAction::Select(_) => {}
            LogsAction::PickRange => {
                // The chart has to be there to be dragged on.
                self.prefs.chart_visible = true;
                self.preview.begin_range_pick();
            }
            LogsAction::CancelPick => self.preview.cancel_range_pick(),
            LogsAction::Clear if self.debug.has_logs() => self.debug.logs.clear(),
            LogsAction::Clear => self.preview.clear_logs(),
            LogsAction::SetLogging(_, enabled) => self.preview.set_logging(enabled),
            LogsAction::Source(sequence) => {
                let Some(event) = self.logged(sequence) else {
                    return;
                };
                let offset = event.range.start.offset;
                match (&event.library, self.open.as_mut()) {
                    // The call is in the script on screen: the caret goes there, nothing reopens.
                    (None, Some(open)) => open.editor.reveal_offset(offset),
                    (Some(path), _) => self.reveal_source(&SourceTarget {
                        script_id: None,
                        library: path
                            .split('/')
                            .filter(|part| !part.is_empty())
                            .nth(1)
                            .map(str::to_owned),
                        offset,
                        ran_source: None,
                    }),
                    (None, None) => {}
                }
            }
            LogsAction::ScrollToBar(sequence) => {
                let Some(event) = self.logged(sequence) else {
                    return;
                };
                match self.preview.scroll_to_time(event.bar_time_ms) {
                    Scrolled::Done => {}
                    Scrolled::BeforeHistory => {
                        self.error = Some(
                            "That bar is older than the candles the preview chart has loaded."
                                .to_owned(),
                        )
                    }
                    Scrolled::NoBars => {
                        self.error = Some("The preview chart has no bars yet.".to_owned())
                    }
                }
            }
        }
    }

    fn logged(&self, sequence: u64) -> Option<crate::pine::logs::LogEvent> {
        if self.debug.has_logs() {
            return self
                .debug
                .logs
                .buffer()
                .iter()
                .find(|event| event.sequence == sequence)
                .cloned();
        }
        self.preview
            .logs()?
            .history
            .buffer()
            .iter()
            .find(|event| event.sequence == sequence)
            .cloned()
    }
}

#[cfg(test)]
mod tests;
