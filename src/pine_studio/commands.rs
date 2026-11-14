//! The commands a frame's views collect (`Cmd`) and what running each one does: selecting,
//! renaming, duplicating, exporting and deleting scripts, saving, the panels and the sheets.

use super::library::prefs::{ChartPosition, DrawerTab};
use super::library::registry::LibraryRegistry;
use super::library::sidebar::{Rename, RowKey};
use super::{AlertSheet, PineStudio, PineStudioAction, Sheet, ai, debug, group_of, library};
use crate::model::pine::script::{ScriptId, ScriptType};
use crate::pine::compiler;
use crate::pine::syntax::token::Limits;
use crate::service::work_queue;
use crate::store::scripts::hash_source;
use eframe::egui;
use std::path::PathBuf;

/// UI events collected while drawing and applied once the frame's closures have finished.
pub(crate) enum Cmd {
    /// The file changed outside the app over unsaved typing: take the file's text.
    DiskReload,
    /// ... or keep the typing.
    DiskKeep,
    Select(RowKey),
    ToggleFavorite(ScriptId),
    BeginRename(RowKey),
    CommitRename,
    CancelRename,
    Duplicate(ScriptId),
    RevealFile(ScriptId),
    Export(ScriptId),
    AskDelete(ScriptId),
    NewScript,
    AddExamples,
    AddToChart(ScriptId),
    CreateAlert(ScriptId),
    Save,
    Reveal(crate::pine::syntax::token::Diagnostic),
    ToggleSidebar,
    ToggleChart,
    SetChartPosition(ChartPosition),
    SetDrawerTab(DrawerTab),
    ToggleDrawer,
    /// Something chosen in the Logs tab.
    Logs(crate::ui::pine::logs_view::LogsAction),
    /// A debugger command from the toolbar or the keyboard.
    Debug(crate::pine::debug::DebugCommand),
    /// Something the debug panel asked for.
    DebugPanel(debug::PanelAction),
    /// A click or menu choice in the editor's breakpoint column.
    Gutter(debug::marks::GutterEvent),
    ToggleDebugPanel,
    ToggleAiPanel,
    /// Something the assistant's panel asked for.
    Ai(ai::panel::PanelAction),
    /// The profiler's control: on or off, run (again), cancel.
    ProfilerToggle,
    ProfilerRun,
    ProfilerCancel,
    /// Show a one-based line of the open script.
    RevealLine(usize),
    OpenAlpacaSettings,
}

impl PineStudio {
    pub(super) fn apply_cmds(&mut self, ctx: &egui::Context) {
        let cmds = std::mem::take(&mut self.cmds);
        for cmd in cmds {
            self.run(cmd, ctx);
        }
    }

    pub(super) fn run(&mut self, cmd: Cmd, ctx: &egui::Context) {
        match cmd {
            Cmd::Select(key) => {
                self.renaming = None;
                let id = key.id.clone();
                self.selected_row = Some(key);
                if self.open.as_ref().map(|o| o.id.as_str()) != Some(id.as_str()) {
                    self.open_script(&id);
                }
            }
            Cmd::ToggleFavorite(id) => {
                let favorite = self.script_named(&id).is_some_and(|s| s.is_favorite);
                self.with_store(|store| store.set_favorite(&id, !favorite));
                self.refresh();
            }
            Cmd::BeginRename(key) => {
                if let Some(script) = self.script_named(&key.id) {
                    self.renaming = Some(Rename {
                        row: key.clone(),
                        text: script.name.clone(),
                        error: None,
                        needs_focus: true,
                    });
                    self.selected_row = Some(key);
                }
            }
            Cmd::CommitRename => self.commit_rename(),
            Cmd::CancelRename => self.renaming = None,
            Cmd::Duplicate(id) => {
                if let Some(copy) = self.with_store(|store| store.duplicate(&id)) {
                    self.refresh();
                    let group = group_of(&copy);
                    self.cmds.push(Cmd::Select(RowKey { group, id: copy.id }));
                }
            }
            Cmd::RevealFile(id) => {
                if let Some(Some(path)) = self.with_store(|store| store.file_path(&id)) {
                    crate::platform::reveal_in_file_manager(&path);
                }
            }
            Cmd::Export(id) => self.begin_export(ctx, &id),
            Cmd::AskDelete(id) => {
                if let Some(script) = self.script_named(&id) {
                    self.sheet = Some(Sheet::ConfirmDelete {
                        name: script.name.clone(),
                        id,
                    });
                }
            }
            Cmd::NewScript => {
                self.sheet = Some(Sheet::New {
                    name: String::new(),
                    error: None,
                    needs_focus: true,
                });
            }
            Cmd::AddExamples => {
                self.with_store(library::demo::add_missing);
                self.refresh();
            }
            Cmd::AddToChart(id) => {
                if let Some(script) = self.script_named(&id) {
                    self.actions.push(PineStudioAction::AddToChart {
                        script_id: script.id.clone(),
                        name: script.name.clone(),
                        source: script.source.clone(),
                    });
                }
            }
            Cmd::CreateAlert(id) => self.open_alert_sheet(&id),
            Cmd::Save => self.save_open(),
            Cmd::DiskReload => {
                let text = self
                    .open
                    .as_ref()
                    .and_then(|open| open.watch.conflict.as_ref())
                    .map(|conflict| conflict.text.clone());
                if let Some(text) = text {
                    self.adopt_disk_text(ctx, &text);
                }
            }
            Cmd::DiskKeep => {
                if let Some(open) = self.open.as_mut()
                    && let Some(conflict) = open.watch.conflict.clone()
                {
                    open.editor.rebase_saved(&conflict.text);
                    open.watch.keep();
                }
            }
            Cmd::Reveal(diagnostic) => {
                if let Some(open) = self.open.as_mut() {
                    open.editor.reveal(&diagnostic);
                    // The jump is applied by the editor's next frame; nothing else asks for one.
                    ctx.request_repaint();
                }
            }
            Cmd::ToggleSidebar => self.prefs.sidebar_visible = !self.prefs.sidebar_visible,
            Cmd::ToggleChart => self.prefs.chart_visible = !self.prefs.chart_visible,
            Cmd::SetChartPosition(position) => self.prefs.chart_position = position,
            Cmd::SetDrawerTab(tab) => {
                self.prefs.drawer_tab = tab;
                self.prefs.drawer_open = true;
            }
            Cmd::ToggleDrawer => self.prefs.drawer_open = !self.prefs.drawer_open,
            Cmd::Logs(action) => self.apply_logs_action(action),
            Cmd::Debug(command) => self.debug_command(command, ctx),
            Cmd::DebugPanel(action) => self.debug_action(action, ctx),
            Cmd::Gutter(event) => self.debug_gutter(event),
            Cmd::ToggleDebugPanel => self.prefs.debug_open = !self.prefs.debug_open,
            Cmd::ToggleAiPanel => self.ai_toggle_panel(),
            Cmd::Ai(action) => self.ai_action(action, ctx),
            Cmd::ProfilerToggle => self.profiler_toggle(),
            Cmd::ProfilerRun => self.profiler_start(ctx),
            Cmd::ProfilerCancel => self.profiler.cancel(),
            Cmd::RevealLine(line) => {
                if let Some(open) = self.open.as_mut() {
                    open.editor.show_line(line);
                    ctx.request_repaint();
                }
            }
            Cmd::OpenAlpacaSettings => self.actions.push(PineStudioAction::OpenAlpacaSettings),
        }
        let _ = ctx;
    }

    /// Opens the Create Script Alert sheet for a script, against the focused chart's market.
    fn open_alert_sheet(&mut self, id: &str) {
        let Some(script) = self.script_named(id).cloned() else {
            return;
        };
        let Some(chart) = self.chart.clone() else {
            self.error = Some(
                "Open a chart first: a script alert watches one chart's market and timeframe."
                    .to_owned(),
            );
            return;
        };
        let matching: Vec<_> = chart
            .instances
            .iter()
            .filter(|instance| instance.script_id.as_deref() == Some(id))
            .cloned()
            .collect();
        let instance = match matching.as_slice() {
            [] => {
                self.error = Some(
                    "Add this script to the focused chart before creating an alert.".to_owned(),
                );
                return;
            }
            [instance] => instance.clone(),
            _ => {
                self.error = Some(
                    "This script is applied more than once. Remove the duplicate instance or arm it from the chart instance you want to watch."
                        .to_owned(),
                );
                return;
            }
        };
        let program = compiler::compile(&instance.source, Limits::default(), &LibraryRegistry);
        let call_sites = if program.is_valid() {
            crate::pine::builtins::alert::call::call_sites(&program.statements)
        } else {
            Vec::new()
        };
        let hash = hash_source(&instance.source);
        let (endpoints, duplicate_exists) = self
            .shared
            .lock()
            .map(|store| {
                let duplicate = store.pine_subscriptions().iter().any(|s| {
                    s.is_active()
                        && s.chart_id == chart.chart_id
                        && s.instance_id == Some(instance.id)
                        && s.source_hash == hash
                        && s.symbol_key == chart.symbol_key
                        && s.timeframe == chart.timeframe
                });
                (store.webhook_endpoints(), duplicate)
            })
            .unwrap_or_default();
        self.sheet = Some(Sheet::Alert(Box::new(AlertSheet {
            chart,
            instance,
            name: script.name,
            source: program.source,
            state: Default::default(),
            call_sites,
            endpoints,
            duplicate_exists,
        })));
    }

    pub(super) fn commit_rename(&mut self) {
        let Some(rename) = self.renaming.take() else {
            return;
        };
        let current = self.script_named(&rename.row.id).map(|s| s.name.clone());
        if current.as_deref() == Some(rename.text.trim()) {
            return;
        }
        let id = rename.row.id.clone();
        let Some(store) = self.script_store.as_mut() else {
            return;
        };
        match store.rename(&id, &rename.text) {
            Ok(_) => self.refresh(),
            Err(error) => {
                // An invalid commit shows the alert and stays in edit mode.
                self.error = Some(error.to_string());
                self.renaming = Some(Rename {
                    error: Some(error.to_string()),
                    needs_focus: true,
                    ..rename
                });
            }
        }
    }

    pub(super) fn save_open(&mut self) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        // The file changed under unsaved typing and the person has not said which to keep: saving
        // now would overwrite the outside edit.
        if open.watch.conflict.is_some() {
            return;
        }
        let id = open.id.clone();
        // The editor shows what lands on disk: a saved script always ends in a newline.
        let source = crate::store::scripts::with_trailing_newline(&open.editor.source);
        // What the editor last read from the file. If the file is no longer that, someone else wrote
        // it and this save would overwrite their work.
        let expected = open.editor.saved_source().to_owned();
        let outcome = self
            .script_store
            .as_mut()
            .map(|store| store.save_if_unchanged(&id, &source, &expected));
        match outcome {
            None => {}
            Some(Ok(script)) => {
                if let Some(open) = self.open.as_mut() {
                    open.editor.source.clone_from(&source);
                    open.editor.mark_saved();
                    open.script_type = script.script_type;
                }
                self.refresh();
            }
            Some(Err(crate::store::scripts::ScriptStoreError::ChangedOnDisk)) => {
                // Not saved. Look at the file at once, without waiting for it to settle, so the
                // question appears over the typing.
                if let Some(open) = self.open.as_mut() {
                    open.watch.force = true;
                }
            }
            Some(Err(error)) => self.error = Some(error.to_string()),
        }
    }

    pub(crate) fn delete_script(&mut self, id: &str) {
        if self.open.as_ref().map(|o| o.id.as_str()) == Some(id) {
            self.close_script();
        }
        self.debug.script_deleted(id);
        self.profiler.script_deleted(id);
        self.ai_forget_history(id);
        self.with_store(|store| store.delete(id));
        self.refresh();
    }

    pub(crate) fn create_script(&mut self, name: &str) -> Result<(), String> {
        let Some(store) = self.script_store.as_mut() else {
            return Err("The scripts folder isn't available.".to_owned());
        };
        match store.create(name, ScriptType::Indicator, None, false) {
            Ok(script) => {
                self.refresh();
                let group = group_of(&script);
                self.cmds.push(Cmd::Select(RowKey {
                    group,
                    id: script.id,
                }));
                Ok(())
            }
            Err(error) => Err(error.to_string()),
        }
    }

    /// The save panel runs on the workers that wait on the user; its answer comes back through
    /// `export`.
    fn begin_export(&mut self, ctx: &egui::Context, id: &str) {
        let Some(script) = self.script_named(id) else {
            return;
        };
        let default = format!("{}.pine", script.name);
        let id = id.to_owned();
        self.export = Some(work_queue::SNAPSHOTS.start(Some(ctx), move || {
            let script = format!(
                "POSIX path of (choose file name with prompt \"Export Script\" default name \"{}\")",
                default.replace('\\', "\\\\").replace('"', "\\\"")
            );
            let output = std::process::Command::new("osascript")
                .args(["-e", &script])
                .output();
            let path = output
                .ok()
                .filter(|o| o.status.success())
                .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim().to_owned()))
                .filter(|p| !p.as_os_str().is_empty());
            (id, path)
        }));
    }

    pub(crate) fn name_problem(&self, raw: &str) -> Option<String> {
        match library::name::validate(raw) {
            Err(error) => (!raw.trim().is_empty()).then(|| error.to_string()),
            Ok(clean) => self
                .scripts
                .iter()
                .any(|s| s.name.to_lowercase() == clean.to_lowercase())
                .then(|| crate::store::scripts::ScriptStoreError::NameConflict.to_string()),
        }
    }
}
