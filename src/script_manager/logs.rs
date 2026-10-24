//! The Pine logs pane inside the Script Manager: what the script in the editor wrote on the
//! preview chart, next to the code that wrote it. Opened from the toolbar's More menu, as
//! TradingView opens it from the editor's.
//!
//! It is the chart workspace's pane (`ui::pine_logs_view`) over the preview's run, so its filters
//! and rows are the same. "Source code" lands in the editor, and "Scroll to bar" moves the
//! preview chart, which only holds the candles it fetched.

use super::{Cmd, ScriptManager};
use crate::model::pine_logs_view::{LogsPrefs, SourceTarget};
use crate::pine::logs::LogHistory;
use crate::ui::pine_logs_view::{self as view, Feed, LogsAction, LogsState, ScriptEntry};
use crate::ui::{PREVIEW_INSTANCE, Scrolled, kit, theme};
use eframe::egui::{self, Ui};

const PREFS_KEY: &str = "pineLogs.prefs";

/// What the pane keeps between frames.
#[derive(Default)]
pub(super) struct LogsPane {
    pub(super) open: bool,
    view: LogsState,
    prefs_loaded: bool,
    prefs_saved: LogsPrefs,
}

impl ScriptManager {
    /// The toolbar's More menu: the one place the pane is opened from here.
    pub(super) fn more_menu(&mut self, button: &egui::Response) {
        kit::menu_popup(button).width(180.0).show(|ui| {
            kit::menu_scope(ui, 160.0);
            let mut row = kit::MenuRow::new("Pine logs");
            row.icon = Some(crate::ui::icons::Icon::DocText);
            row.checked = self.logs.open;
            if kit::menu_row(ui, row).clicked() {
                self.cmds.push(Cmd::ToggleLogs);
                ui.close();
            }
        });
    }

    pub(super) fn toggle_logs(&mut self) {
        self.logs.open = !self.logs.open;
        if self.logs.open && !self.logs.prefs_loaded {
            self.logs.prefs_loaded = true;
            let prefs = self
                .shared
                .lock()
                .ok()
                .and_then(|store| store.setting::<LogsPrefs>(PREFS_KEY))
                .unwrap_or_default();
            self.logs.view.filter.levels = prefs.levels;
            self.logs.view.filter.options = prefs.search;
            self.logs.prefs_saved = prefs;
        }
    }

    pub(super) fn show_logs_panel(&mut self, ui: &mut Ui) {
        let label = self
            .open
            .as_ref()
            .map_or_else(|| "Preview".to_owned(), |open| open.name.clone());
        let scripts = vec![ScriptEntry {
            id: PREVIEW_INSTANCE,
            label,
        }];
        self.logs.view.select_valid(&scripts);
        let nothing = LogHistory::default();
        let feed = match self.preview.logs() {
            Some(logs) => Feed {
                epoch: logs.epoch,
                history: logs.history,
                logging: self.preview.logging(),
            },
            None => Feed {
                epoch: 0,
                history: &nothing,
                logging: self.preview.logging(),
            },
        };
        let actions = view::show(ui, &mut self.logs.view, &scripts, Some(feed));
        for action in actions {
            self.apply_logs_action(action);
        }
        let prefs = self.logs.view.prefs();
        if prefs != self.logs.prefs_saved {
            self.logs.prefs_saved = prefs;
            if let Ok(store) = self.shared.lock() {
                let _ = store.set_setting(PREFS_KEY, &prefs);
            }
        }
    }

    pub(super) fn logs_panel_frame() -> egui::Frame {
        egui::Frame::new()
            .fill(theme::chrome())
            .inner_margin(egui::Margin::ZERO)
    }

    fn apply_logs_action(&mut self, action: LogsAction) {
        match action {
            LogsAction::Close => self.logs.open = false,
            LogsAction::Select(_) => {}
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
