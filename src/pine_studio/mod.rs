//! The Pine Studio window: file-backed Pine script library, editor, preview chart and drawer.
//! A port of DegenView's `PineStudioView` and its view models.
//!
//! Layout (see `workspace.rs`): a sidebar with the script list, and for the selected script a
//! split between the code editor and the preview chart, with the Inputs / Report / Problems
//! drawer under the chart.

pub mod ai;
mod bar;
mod commands;
pub mod debug;
pub mod drawer;
pub mod editor;
pub mod inputs;
pub mod library;
mod migrate_swift;
mod open_script;
mod preview;
pub mod profiler;
mod sheets;
pub mod theme_preview;
mod workspace;

use crate::model::pine::instance::{PineInstance, StoredInput};
use crate::model::pine::script::{
    CompileStatus, LocalScript, ScriptDraft, ScriptId, ScriptType, now_ms,
};
use crate::pine::compiler;
use crate::pine::syntax::token::Limits;
use crate::service::work_queue::Pending;
use crate::store::SharedStore;
use crate::store::scripts::ScriptStore;
use crate::ui::kit::split::{self, Axis, DefaultLength, SplitSpec, SplitState};
use crate::ui::shortcut;
use crate::ui::theme;
use commands::Cmd;
use editor::EditorState;
use eframe::egui::{self, Id, Ui};
use library::prefs::LayoutPrefs;
use library::registry::LibraryRegistry;
use library::sidebar::{Group, Rename, RowKey};
pub use library::unsaved::{ExitDecision, ExitReason};
use library::unsaved::{ExitPrompt, UnsavedChoice, UnsavedScript};
use migrate_swift::swift_scripts_dir;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// What the Pine Studio asks the app to do.
#[derive(Clone, Debug, PartialEq)]
pub enum PineStudioAction {
    /// Add the script to the focused chart.
    AddToChart {
        script_id: String,
        name: String,
        source: String,
    },
    /// Create a Pine alert subscription on the focused chart.
    CreateAlert {
        chart_id: u64,
        instance_id: u64,
        script_id: Option<String>,
        script_name: String,
        source: String,
        inputs: std::collections::HashMap<String, StoredInput>,
        symbol_key: String,
        timeframe: String,
        note: String,
        webhook_endpoint_ids: Vec<u64>,
    },
    /// Open Settings on the Alpaca tab.
    OpenAlpacaSettings,
    /// Open the webhooks list in Settings.
    OpenWebhookSettings,
    /// Open Settings on the AI Assistant page.
    OpenAiSettings,
}

/// The focused chart a script alert would watch: its market and timeframe.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartInfo {
    pub chart_id: u64,
    /// "BTC/USDT".
    pub title: String,
    /// "Binance".
    pub source_name: String,
    /// "1h".
    pub timeframe: String,
    /// `"<SLUG>:<ticker>"`, what a subscription stores.
    pub symbol_key: String,
    /// Applied scripts on this chart. Alerts are bound to one of these instances, never merely
    /// to a similarly named library script.
    pub instances: Vec<PineInstance>,
}

/// How the open editor stands against the file on disk. A write from outside the editor (the
/// MCP server) must honour it the way the editor's own Save does.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OpenDiskState {
    /// The file's text as the editor last read it.
    pub saved_source: String,
    /// The file changed under unsaved typing and the person has not chosen which to keep.
    pub conflict: bool,
}

/// The script open in the editor.
pub(crate) struct OpenScript {
    pub id: ScriptId,
    pub name: String,
    pub script_type: ScriptType,
    pub editor: EditorState,
    pub has_alerts: bool,
    /// Whether the file is being edited outside the app, and the question that raises.
    pub watch: library::external::Watch,
}

pub(crate) enum Sheet {
    New {
        name: String,
        error: Option<String>,
        needs_focus: bool,
    },
    ConfirmDelete {
        id: ScriptId,
        name: String,
    },
    /// The Create Script Alert sheet for one script.
    Alert(Box<AlertSheet>),
}

pub(crate) struct AlertSheet {
    pub chart: ChartInfo,
    pub instance: PineInstance,
    pub name: String,
    pub source: String,
    pub state: crate::ui::alert::pine_editor::PineAlertEditorState,
    pub call_sites: Vec<crate::pine::builtins::alert::call::AlertCallSite>,
    pub endpoints: Vec<crate::model::webhook::WebhookEndpoint>,
    pub duplicate_exists: bool,
}

pub struct PineStudio {
    id: u64,
    shared: SharedStore,
    script_store: Option<ScriptStore>,
    pub(crate) scripts: Vec<LocalScript>,
    loaded: bool,
    last_scan: Instant,
    pub(crate) prefs: LayoutPrefs,
    saved_prefs: LayoutPrefs,
    pub(crate) search: String,
    pub(crate) collapsed: HashSet<Group>,
    pub(crate) selected_row: Option<RowKey>,
    pub(crate) renaming: Option<Rename>,
    pub(crate) last_click: Option<(RowKey, Instant)>,
    pub(crate) open: Option<OpenScript>,
    pub(crate) sheet: Option<Sheet>,
    /// "Save changes?" over a tab close or a quit, while it waits for an answer.
    exit_prompt: Option<ExitPrompt>,
    /// The answer, for the app to read once with `take_exit_decision`.
    exit_decision: Option<ExitDecision>,
    pub(crate) chart: Option<ChartInfo>,
    pub(crate) error: Option<String>,
    pub(crate) cmds: Vec<Cmd>,
    pub(crate) preview: workspace::PreviewState,
    /// The Logs tab's state.
    logs: drawer::logs::LogsPane,
    /// Breakpoints, watches and the debugging session of the open script.
    pub(crate) debug: debug::DebugPanel,
    /// Whether the profiler is on, the run in flight and the profile it made.
    pub(crate) profiler: profiler::Profiler,
    /// The AI assistant's conversation about the open script.
    pub(crate) ai: ai::Assistant,
    /// What the profile on screen would have to be of to be current; kept up to date while the
    /// profiler is on.
    profiler_now: Option<profiler::ProfileKey>,
    pub(crate) problems_copied: Option<Instant>,
    actions: Vec<PineStudioAction>,
    export: Option<Pending<(ScriptId, Option<PathBuf>)>>,
}

impl PineStudio {
    pub fn new(id: u64, shared: SharedStore) -> Self {
        Self::with_script_store(id, shared, ScriptStore::default_location())
    }

    pub fn with_script_store(
        id: u64,
        shared: SharedStore,
        script_store: Option<ScriptStore>,
    ) -> Self {
        let prefs = LayoutPrefs::load(&shared);
        Self {
            id,
            preview: workspace::PreviewState::load(&shared),
            logs: drawer::logs::LogsPane::load(&shared),
            debug: debug::DebugPanel::load(&shared),
            profiler: profiler::Profiler::default(),
            ai: ai::Assistant::default(),
            profiler_now: None,
            shared,
            script_store,
            scripts: Vec::new(),
            loaded: false,
            last_scan: Instant::now(),
            saved_prefs: prefs.clone(),
            prefs,
            search: String::new(),
            collapsed: HashSet::new(),
            selected_row: None,
            renaming: None,
            last_click: None,
            open: None,
            sheet: None,
            exit_prompt: None,
            exit_decision: None,
            chart: None,
            error: None,
            cmds: Vec::new(),
            actions: Vec::new(),
            export: None,
            problems_copied: None,
        }
    }

    /// Saves the open script on the next frame: the File menu's ⌘S reaches the app, not the
    /// editor, so the app forwards it here.
    pub fn request_save(&mut self) {
        if self.open.is_some() {
            self.cmds.push(Cmd::Save);
        }
    }

    /// What the Edit menu may do for the open script's editor: nothing unless it has the keyboard.
    pub fn edit_menu(&self) -> crate::native_bridge::EditMenu {
        self.open
            .as_ref()
            .map(|open| open.editor.edit_menu())
            .unwrap_or_default()
    }

    /// An Edit-menu item chosen by the app, run by the editor on its next frame.
    pub fn edit_command(&mut self, action: editor::EditAction) {
        if let Some(open) = self.open.as_mut() {
            open.editor.request_edit(action);
        }
    }

    /// Whether File ▸ New Script may run: not while another sheet is up, which it would replace.
    pub fn can_create_script(&self) -> bool {
        self.sheet.is_none()
    }

    /// The New Script button's action, from the menu or the keyboard.
    pub fn new_script(&mut self) {
        if self.can_create_script() {
            self.cmds.push(Cmd::NewScript);
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    // MARK: iCloud Sync

    /// The open script, if its editor holds text that is not saved. Cheap: iCloud Sync asks every
    /// frame so it never overwrites what is being typed.
    pub fn open_unsaved_id(&self) -> Option<String> {
        self.open
            .as_ref()
            .filter(|open| open.editor.is_dirty())
            .map(|open| open.id.clone())
    }

    /// Changes whenever the library does: a script created, saved, renamed, starred or deleted.
    pub fn library_signature(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for script in &self.scripts {
            (
                &script.id,
                &script.name,
                script.modified_at_ms,
                script.is_favorite,
            )
                .hash(&mut hasher);
        }
        hasher.finish()
    }

    // MARK: Unsaved changes

    /// The scripts whose text differs from what is saved: the open editor's, and the drafts kept
    /// for scripts opened earlier.
    pub fn unsaved_scripts(&mut self) -> Vec<UnsavedScript> {
        if !self.loaded {
            return Vec::new();
        }
        let mut found = Vec::new();
        let open_id = self.open.as_ref().map(|open| open.id.clone());
        if let Some(open) = self.open.as_ref().filter(|open| open.editor.is_dirty()) {
            let (added, removed) =
                library::unsaved::lines_changed(open.editor.saved_source(), &open.editor.source);
            found.push(UnsavedScript {
                id: open.id.clone(),
                name: open.name.clone(),
                script_type: open.script_type,
                added,
                removed,
            });
        }
        let others: Vec<LocalScript> = self
            .scripts
            .iter()
            .filter(|script| Some(&script.id) != open_id.as_ref())
            .cloned()
            .collect();
        for script in others {
            let draft = self
                .with_store(|store| store.draft(&script.id))
                .flatten()
                .filter(|draft| draft.source != script.source);
            if let Some(draft) = draft {
                let (added, removed) =
                    library::unsaved::lines_changed(&script.source, &draft.source);
                found.push(UnsavedScript {
                    id: script.id,
                    name: script.name,
                    script_type: script.script_type,
                    added,
                    removed,
                });
            }
        }
        found
    }

    /// Asks before `reason` goes ahead. False when nothing is unsaved (go ahead); true when the
    /// question is up, and `take_exit_decision` reports the answer. Asking again while it is up
    /// keeps the same question.
    pub fn begin_exit(&mut self, reason: ExitReason) -> bool {
        if let Some(prompt) = self.exit_prompt.as_mut() {
            prompt.reason = reason;
            return true;
        }
        let scripts = self.unsaved_scripts();
        if scripts.is_empty() {
            return false;
        }
        self.exit_prompt = Some(ExitPrompt { reason, scripts });
        true
    }

    /// Drops the question without an answer (the quit it was asked for was called off).
    pub fn cancel_exit(&mut self) {
        self.exit_prompt = None;
    }

    pub fn take_exit_decision(&mut self) -> Option<ExitDecision> {
        self.exit_decision.take()
    }

    fn resolve_exit(&mut self, prompt: &ExitPrompt, choice: UnsavedChoice) {
        let proceed = match choice {
            UnsavedChoice::Save => self.save_unsaved(&prompt.scripts),
            UnsavedChoice::Discard => {
                self.discard_unsaved(&prompt.scripts);
                true
            }
            UnsavedChoice::Cancel => false,
        };
        self.exit_prompt = None;
        self.exit_decision = Some(ExitDecision {
            reason: prompt.reason,
            proceed,
        });
    }

    /// Saves each script's edited text; false if any could not be written (the error alert says
    /// why, and the window stays).
    fn save_unsaved(&mut self, scripts: &[UnsavedScript]) -> bool {
        let mut all_saved = true;
        for script in scripts {
            if self.open.as_ref().is_some_and(|open| open.id == script.id) {
                self.save_open();
                all_saved &= self
                    .open
                    .as_ref()
                    .is_none_or(|open| !open.editor.is_dirty());
                continue;
            }
            let saved = self
                .with_store(|store| {
                    let Some(draft) = store.draft(&script.id)? else {
                        return Ok(());
                    };
                    store.save(&script.id, &draft.source).map(|_| ())
                })
                .is_some();
            all_saved &= saved;
        }
        self.refresh();
        all_saved
    }

    /// Puts each script back to its saved text and forgets the drafts.
    fn discard_unsaved(&mut self, scripts: &[UnsavedScript]) {
        for script in scripts {
            self.with_store(|store| store.discard_draft(&script.id));
            if let Some(open) = self.open.as_mut().filter(|open| open.id == script.id) {
                let saved = open.editor.saved_source().to_owned();
                open.editor.replace_source(&saved, true);
            }
        }
        self.editor_changed();
    }

    /// The window title: the open script's name (and " — Edited" while unsaved).
    pub fn title(&self) -> String {
        match &self.open {
            Some(open) if open.editor.is_dirty() => {
                format!("{} — Edited", open.name)
            }
            Some(open) => open.name.clone(),
            None => "Pine Studio".to_owned(),
        }
    }

    // MARK: Frame

    pub fn ui(&mut self, ui: &mut Ui, chart: Option<ChartInfo>) -> Vec<PineStudioAction> {
        self.chart = chart;
        let ctx = ui.ctx().clone();
        self.begin_frame(&ctx);
        self.shortcuts(&ctx);
        self.debug_shortcuts(&ctx);
        self.debug_frame(&ctx);
        self.profiler_shortcuts(&ctx);
        self.profiler_frame(&ctx);

        egui::Panel::top("pine-studio-toolbar")
            .exact_size(crate::ui::kit::bar::HEIGHT)
            .resizable(false)
            .frame(egui::Frame::new().fill(theme::chrome()))
            .show(ui, |ui| self.show_bar(ui));

        self.sync_logs_range();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::background()))
            .show(ui, |ui| self.root(ui));

        self.sheets_ui(&ctx);
        self.apply_cmds(&ctx);
        self.save_logs_prefs();
        self.end_frame(&ctx);
        std::mem::take(&mut self.actions)
    }

    fn begin_frame(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let mut scanned = false;
        if !self.loaded {
            self.first_load();
        } else if self.last_scan.elapsed() > Duration::from_millis(1500) {
            // Files edited, added or renamed outside the app show up.
            self.refresh();
            scanned = true;
        } else if self.external_scan_due(now) {
            // The open script's own file moved: look at it now, not at the next full scan.
            self.refresh();
            scanned = true;
        }
        if scanned {
            self.reconcile_open(ctx, now);
        }
        // A script being open is a file to follow, a little more often than the full scan.
        ctx.request_repaint_after(if self.open.is_some() {
            library::external::FAST_POLL
        } else {
            Duration::from_millis(1500)
        });
        self.ai_begin_frame(ctx);
        let exported = self.export.as_mut().and_then(Pending::poll);
        if self
            .export
            .as_ref()
            .is_some_and(|export| !export.is_pending())
        {
            self.export = None;
        }
        if let Some((id, Some(path))) = exported {
            self.with_store(|store| store.export(&id, &path));
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let (save, sidebar, chart, new_script) = ctx.input(|i| {
            (
                shortcut::SAVE.peek(i),
                shortcut::TOGGLE_SCRIPT_LIST.peek(i),
                shortcut::TOGGLE_PREVIEW_CHART.peek(i),
                // macOS owns New Script in its File menu (`NativeCommand::NewScript`).
                !shortcut::is_mac() && shortcut::NEW_SCRIPT.peek(i),
            )
        });
        if new_script {
            self.new_script();
        }
        if save && self.open.is_some() {
            self.cmds.push(Cmd::Save);
        }
        if sidebar {
            self.cmds.push(Cmd::ToggleSidebar);
        }
        if chart {
            self.cmds.push(Cmd::ToggleChart);
        }
    }

    fn end_frame(&mut self, ctx: &egui::Context) {
        if let Some(open) = self.open.as_mut()
            && open.editor.take_draft_due(Instant::now())
        {
            let draft = ScriptDraft {
                script_id: open.id.clone(),
                source: open.editor.source.clone(),
                modified_at_ms: now_ms(),
                based_on_revision_id: None,
            };
            self.with_store(|store| store.save_draft(&draft));
        }
        // Compile once typing pauses, not on every keystroke.
        if let Some(open) = self.open.as_ref() {
            if open.editor.compile_due(Instant::now()) {
                self.editor_changed();
            } else if open.editor.compile_pending() {
                ctx.request_repaint_after(editor::COMPILE_DELAY);
            }
        }
        self.preview.tick(ctx);
        if self.prefs != self.saved_prefs {
            self.prefs.save(&self.shared);
            self.saved_prefs = self.prefs.clone();
        }
    }

    fn root(&mut self, ui: &mut Ui) {
        let spec = SplitSpec {
            axis: Axis::Horizontal,
            secondary_first: true,
            min_primary: 420.0,
            min_secondary: 200.0,
            max_secondary: 320.0,
            default: DefaultLength::Points(240.0),
        };
        let mut state = SplitState {
            length: self.prefs.sidebar_width,
            visible: self.prefs.sidebar_visible,
        };
        let id = Id::new(("pine-studio-root", self.id));
        let this = std::cell::RefCell::new(&mut *self);
        split::show(
            ui,
            id,
            spec,
            &mut state,
            |ui| this.borrow_mut().detail_ui(ui),
            |ui| this.borrow_mut().sidebar_ui(ui),
        );
        let _ = this;
        self.prefs.sidebar_width = state.length;
    }

    // MARK: Store

    /// Runs a store operation; a failure becomes the alert.
    fn with_store<T>(
        &mut self,
        operation: impl FnOnce(&mut ScriptStore) -> Result<T, crate::store::scripts::ScriptStoreError>,
    ) -> Option<T> {
        let store = self.script_store.as_mut()?;
        match operation(store) {
            Ok(value) => Some(value),
            Err(error) => {
                self.error = Some(error.to_string());
                None
            }
        }
    }

    fn first_load(&mut self) {
        self.loaded = true;
        let seeded: bool = self
            .shared
            .lock()
            .ok()
            .and_then(|s| s.setting(library::demo::SEEDED_KEY))
            .unwrap_or(false);
        if !seeded
            && self
                .with_store(|store| {
                    // A user who already has scripts is not offered the examples again; one
                    // coming from the Swift app first gets their own scripts.
                    if store.all_scripts()?.is_empty() {
                        let brought = match swift_scripts_dir() {
                            Some(folder) if folder.is_dir() => store.import_folder(&folder)?,
                            _ => 0,
                        };
                        if brought == 0 {
                            library::demo::add_missing(store)?;
                        }
                    }
                    Ok(())
                })
                .is_some()
            && let Ok(shared) = self.shared.lock()
        {
            let _ = shared.set_setting(library::demo::SEEDED_KEY, &true);
        }
        self.refresh();
        self.import_swift_inputs();
        // Reopen where the user left off.
        if let Some(id) = self.prefs.selected_script_id.clone()
            && let Some(script) = self.scripts.iter().find(|s| s.id == id)
        {
            let group = group_of(script);
            self.selected_row = Some(RowKey {
                group,
                id: id.clone(),
            });
            self.open_script(&id);
        }
    }

    pub(crate) fn refresh(&mut self) {
        self.last_scan = Instant::now();
        let Some(scripts) = self.with_store(|store| store.all_scripts()) else {
            return;
        };
        self.scripts = scripts;
        // The open script vanished (deleted in Finder): close it.
        if let Some(open) = &self.open
            && !self.scripts.iter().any(|s| s.id == open.id)
        {
            self.close_script();
        } else if let Some(open) = self.open.as_mut()
            && let Some(script) = self.scripts.iter().find(|s| s.id == open.id)
        {
            open.name = script.name.clone();
            open.script_type = script.script_type;
        }
    }

    pub(crate) fn script_named(&self, id: &str) -> Option<&LocalScript> {
        self.scripts.iter().find(|s| s.id == id)
    }
}

fn group_of(script: &LocalScript) -> Group {
    match script.script_type {
        ScriptType::Indicator => Group::Indicators,
        ScriptType::Strategy => Group::Strategies,
        ScriptType::Library => Group::Libraries,
    }
}

/// Compiles the editor's text and stores the diagnostics and status the chip and Problems read.
fn compile_into(editor: &mut EditorState) {
    let program = compiler::compile(&editor.source, Limits::default(), &LibraryRegistry);
    editor.status = CompileStatus::of(&program.diagnostics);
    editor.diagnostics = program.diagnostics;
    editor.compiled();
}

#[cfg(test)]
mod tests;
