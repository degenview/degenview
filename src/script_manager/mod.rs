//! The Script Manager window: file-backed Pine script library, editor, preview chart and drawer.
//! A port of DegenView's `ScriptManagerView` and its view models.
//!
//! Layout (see `workspace.rs`): a sidebar with the script list, and for the selected script a
//! split between the code editor and the preview chart, with the Inputs / Report / Problems
//! drawer under the chart.
#![allow(dead_code)] // transient: phase-2 pieces (completion, find bar) are not wired yet

mod bottom_bar;
mod completion;
pub mod debug;
#[cfg(test)]
mod debug_gallery;
mod debug_host;
pub mod debug_marks;
mod debug_panel;
pub mod debug_state;
#[cfg(test)]
mod debug_tests;
pub mod demo;
pub mod diff;
mod diff_view;
pub mod drawer;
mod drawer_intro;
#[cfg(test)]
mod drawer_tabs_tests;
pub mod edit_ops;
pub mod editor;
mod find;
pub mod inputs;
mod logs;
pub mod model;
pub mod multi_cursor;
pub mod name;
pub mod prefs;
mod preview;
mod problems;
#[cfg(test)]
mod problems_gallery;
#[cfg(test)]
mod problems_tests;
mod profile_marks;
mod profile_ui;
mod profiler;
#[cfg(test)]
mod profiler_gallery;
mod profiler_host;
#[cfg(test)]
mod profiler_tests;
pub mod registry;
mod sheets;
mod sidebar;
mod signature;
pub mod store;
pub mod symbols;
pub mod syntax;
mod tab_chrome;
mod unsaved;
mod workspace;

use crate::model::pine_instance::{PineInstance, StoredInput};
use crate::pine::compiler;
use crate::pine::token::Limits;
use crate::store::SharedStore;
use crate::ui::icons::{self, Icon};
use crate::ui::shortcut;
use crate::ui::split::{self, Axis, DefaultLength, SplitSpec, SplitState};
use crate::ui::theme;
use editor::EditorState;
use eframe::egui::{self, Align, Id, Layout, Margin, Ui};
use model::{CompileStatus, LocalScript, ScriptId, ScriptType};
use prefs::{ChartPosition, DrawerTab, LayoutPrefs};
use registry::LibraryRegistry;
use sidebar::{Group, Rename, RowKey};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};
use store::{COMPILER_VERSION, ScriptStore, hash_source};
pub use unsaved::{ExitDecision, ExitReason};
use unsaved::{ExitPrompt, UnsavedChoice, UnsavedScript};

/// What the Script Manager asks the app to do.
#[derive(Clone, Debug, PartialEq)]
pub enum ScriptManagerAction {
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

/// UI events collected while drawing and applied once the frame's closures have finished.
pub(crate) enum Cmd {
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
    Reveal(crate::pine::token::Diagnostic),
    ToggleSidebar,
    ToggleChart,
    SetChartPosition(ChartPosition),
    SetDrawerTab(DrawerTab),
    ToggleDrawer,
    /// Something chosen in the Logs tab.
    Logs(crate::ui::pine_logs_view::LogsAction),
    /// A debugger command from the toolbar or the keyboard.
    Debug(crate::pine::debug::DebugCommand),
    /// Something the debug panel asked for.
    DebugPanel(debug::PanelAction),
    /// A click or menu choice in the editor's breakpoint column.
    Gutter(debug_marks::GutterEvent),
    ToggleDebugPanel,
    /// The profiler's control: on or off, run (again), cancel.
    ProfilerToggle,
    ProfilerRun,
    ProfilerCancel,
    /// Show a one-based line of the open script.
    RevealLine(usize),
    OpenAlpacaSettings,
}

/// The script open in the editor.
pub(crate) struct OpenScript {
    pub id: ScriptId,
    pub name: String,
    pub script_type: ScriptType,
    pub editor: EditorState,
    pub has_alerts: bool,
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
    pub state: crate::ui::pine_alert_editor::PineAlertEditorState,
    pub call_sites: Vec<crate::pine::alert_call::AlertCallSite>,
    pub endpoints: Vec<crate::model::webhook::WebhookEndpoint>,
    pub duplicate_exists: bool,
}

pub struct ScriptManager {
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
    logs: logs::LogsPane,
    /// Breakpoints, watches and the debugging session of the open script.
    pub(crate) debug: debug::DebugPanel,
    /// Whether the profiler is on, the run in flight and the profile it made.
    pub(crate) profiler: profiler::Profiler,
    /// What the profile on screen would have to be of to be current; kept up to date while the
    /// profiler is on.
    profiler_now: Option<profiler::ProfileKey>,
    pub(crate) problems_copied: Option<Instant>,
    actions: Vec<ScriptManagerAction>,
    export_rx: Option<Receiver<(ScriptId, Option<PathBuf>)>>,
}

impl ScriptManager {
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
            logs: logs::LogsPane::load(&shared),
            debug: debug::DebugPanel::load(&shared),
            profiler: profiler::Profiler::default(),
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
            export_rx: None,
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
                unsaved::lines_changed(open.editor.saved_source(), &open.editor.source);
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
                let (added, removed) = unsaved::lines_changed(&script.source, &draft.source);
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

    pub fn exit_prompt_open(&self) -> bool {
        self.exit_prompt.is_some()
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
            None => "Script Manager".to_owned(),
        }
    }

    // MARK: Frame

    pub fn ui(&mut self, ui: &mut Ui, chart: Option<ChartInfo>) -> Vec<ScriptManagerAction> {
        self.chart = chart;
        let ctx = ui.ctx().clone();
        self.begin_frame(&ctx);
        self.shortcuts(&ctx);
        self.debug_shortcuts(&ctx);
        self.debug_frame(&ctx);
        self.profiler_shortcuts(&ctx);
        self.profiler_frame(&ctx);

        egui::Panel::top("script-manager-toolbar")
            .exact_size(38.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::chrome())
                    .inner_margin(Margin::symmetric(10, 5)),
            )
            .show(ui, |ui| self.toolbar(ui));

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
        if !self.loaded {
            self.first_load();
        } else if self.last_scan.elapsed() > Duration::from_millis(1500) {
            // Files edited, added or renamed outside the app show up.
            self.refresh();
        }
        ctx.request_repaint_after(Duration::from_millis(1500));
        if let Some(rx) = &self.export_rx
            && let Ok((id, destination)) = rx.try_recv()
        {
            self.export_rx = None;
            if let Some(path) = destination {
                self.with_store(|store| store.export(&id, &path));
            }
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
            let draft = model::ScriptDraft {
                script_id: open.id.clone(),
                source: open.editor.source.clone(),
                modified_at_ms: model::now_ms(),
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

    fn toolbar(&mut self, ui: &mut Ui) {
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let tip = shortcut::TOGGLE_SCRIPT_LIST.hint(if self.prefs.sidebar_visible {
                "Hide the script list"
            } else {
                "Show the script list"
            });
            if icons::button(ui, Icon::SidebarLeft, 28.0, theme::text_primary(), &tip).clicked() {
                self.cmds.push(Cmd::ToggleSidebar);
            }
            let position_icon = match self.prefs.chart_position {
                ChartPosition::Left => Icon::RectLeft,
                ChartPosition::Top => Icon::RectTop,
                ChartPosition::Bottom => Icon::RectBottom,
            };
            let button = icons::button(
                ui,
                position_icon,
                28.0,
                theme::text_primary(),
                "Where the preview chart sits next to the code",
            );
            egui::Popup::menu(&button).show(|ui| {
                ui.set_min_width(140.0);
                for position in ChartPosition::ALL {
                    let selected = position == self.prefs.chart_position;
                    let label = if selected {
                        format!("✓  {}", position.title())
                    } else {
                        format!("    {}", position.title())
                    };
                    if ui.button(label).clicked() {
                        self.cmds.push(Cmd::SetChartPosition(position));
                        ui.close();
                    }
                }
            });
            let (icon, tip) = if self.prefs.chart_visible {
                (Icon::ChartLine, "Hide the preview chart")
            } else {
                (Icon::ChartLineFlat, "Show the preview chart")
            };
            let tip = shortcut::TOGGLE_PREVIEW_CHART.hint(tip);
            if icons::button(ui, icon, 28.0, theme::text_primary(), &tip).clicked() {
                self.cmds.push(Cmd::ToggleChart);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if self.open.is_some() && debug_panel::panel_button(ui, self.prefs.debug_open) {
                    self.cmds.push(Cmd::ToggleDebugPanel);
                }
                if self.open.is_some() {
                    let status = self.profiler.status(self.profiler_now.as_ref());
                    for action in profile_ui::toolbar(ui, &status) {
                        self.cmds.push(Cmd::from(action));
                    }
                }
            });
        });
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
        let id = Id::new(("script-manager-root", self.id));
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
        operation: impl FnOnce(&mut ScriptStore) -> Result<T, store::ScriptStoreError>,
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
            .and_then(|s| s.setting(demo::SEEDED_KEY))
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
                            demo::add_missing(store)?;
                        }
                    }
                    Ok(())
                })
                .is_some()
            && let Ok(shared) = self.shared.lock()
        {
            let _ = shared.set_setting(demo::SEEDED_KEY, &true);
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

    /// Once: the input values the Swift app saved for its preview, matched to this library's
    /// scripts by file name (the two apps number their scripts differently). Values already tried
    /// here win, and a script the user later resets is not filled again.
    fn import_swift_inputs(&mut self) {
        let done: bool = self
            .shared
            .lock()
            .ok()
            .and_then(|s| s.setting(SWIFT_INPUTS_KEY))
            .unwrap_or(false);
        let Some(folder) = swift_data_dir().filter(|_| !done) else {
            return;
        };
        let by_swift_id = swift_script_names(&folder);
        let inputs = std::fs::read(folder.join("script_preview_inputs.json"))
            .map(|bytes| inputs::parse_swift_inputs(&bytes))
            .unwrap_or_default();
        let imported = inputs
            .into_iter()
            .filter_map(|(swift_id, values)| {
                let name = by_swift_id.get(&swift_id)?;
                let script = self.scripts.iter().find(|s| &s.name == name)?;
                Some((script.id.clone(), values))
            })
            .collect();
        self.preview.import_inputs(imported);
        if let Ok(shared) = self.shared.lock() {
            let _ = shared.set_setting(SWIFT_INPUTS_KEY, &true);
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

    /// Opens the script (or library) a Pine logs message was written by and puts the caret on the
    /// call. A script that was edited since the message was written is opened all the same, with a
    /// note that the line may have moved; one that is gone is told so and nothing else changes.
    pub(crate) fn reveal_source(&mut self, target: &crate::model::pine_logs_view::SourceTarget) {
        let Some(scripts) = self.with_store(|store| store.all_scripts()) else {
            return;
        };
        let found = match (&target.script_id, &target.library) {
            (Some(id), _) => scripts.iter().find(|script| &script.id == id),
            (None, Some(name)) => scripts.iter().find(|script| {
                script.script_type == ScriptType::Library
                    && (&script.name == name
                        || registry::title_of(&script.source).as_ref() == Some(name))
            }),
            (None, None) => None,
        };
        let Some(script) = found else {
            self.error = Some(match &target.library {
                Some(name) => format!("The library “{name}” is not in the Script Manager."),
                None => "That script is no longer in the Script Manager.".to_owned(),
            });
            return;
        };
        let changed = target
            .ran_source
            .as_ref()
            .is_some_and(|ran| *ran != script.source);
        let id = script.id.clone();
        self.renaming = None;
        self.selected_row = Some(RowKey {
            group: group_of(script),
            id: id.clone(),
        });
        // A script already in the editor stays as it is, with its caret and undo history.
        if self.open.as_ref().map(|open| open.id.as_str()) != Some(id.as_str()) {
            self.open_script(&id);
        }
        if let Some(open) = self.open.as_mut().filter(|open| open.id == id) {
            open.editor.reveal_offset(target.offset);
        }
        if changed {
            self.error = Some(
                "This script was edited after that message was written, so the line may have moved."
                    .to_owned(),
            );
        }
    }

    pub(crate) fn open_script(&mut self, id: &str) {
        self.flush_draft();
        let Some((script, draft)) = self.with_store(|store| {
            let script = store.script(id)?;
            let draft = store.draft(id)?;
            Ok((script, draft))
        }) else {
            return;
        };
        let Some(script) = script else {
            return;
        };
        let source = draft
            .filter(|d| d.source != script.source)
            .map(|d| d.source)
            .unwrap_or_else(|| script.source.clone());
        let mut editor = EditorState::new(&script.id, &source, &script.source);
        // A cached compile counts only when the compiler hasn't changed and the text matches.
        let cached = script.compile_record.as_ref().filter(|r| {
            r.compiler_version == COMPILER_VERSION && r.source_hash == hash_source(&source)
        });
        match cached {
            Some(record) => {
                editor.status = record.status;
                editor.diagnostics = record.diagnostics.clone();
            }
            None => compile_into(&mut editor),
        }
        self.preview.script_changed(&script.id);
        self.debug.script_changed(&script.id);
        self.profiler.script_changed();
        self.open = Some(OpenScript {
            id: script.id.clone(),
            name: script.name.clone(),
            script_type: script.script_type,
            editor,
            has_alerts: false,
        });
        self.prefs.selected_script_id = Some(script.id);
        self.refresh_debug_map();
    }

    fn close_script(&mut self) {
        self.flush_draft();
        self.open = None;
        self.selected_row = None;
        self.renaming = None;
        self.prefs.selected_script_id = None;
    }

    /// Writes the pending draft now (switching scripts or closing).
    fn flush_draft(&mut self) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        if open.editor.has_pending_draft() && open.editor.is_dirty() {
            let draft = model::ScriptDraft {
                script_id: open.id.clone(),
                source: open.editor.source.clone(),
                modified_at_ms: model::now_ms(),
                based_on_revision_id: None,
            };
            self.with_store(|store| store.save_draft(&draft));
        }
    }

    /// The text in the editor if `id` is the open script: what the person sees, which can be ahead
    /// of anything on disk.
    pub(crate) fn open_text(&self, id: &str) -> Option<String> {
        self.open
            .as_ref()
            .filter(|open| open.id == id)
            .map(|open| open.editor.source.clone())
    }

    /// Shows an edit made from outside in the open editor (`saved`: it is also the saved source
    /// now). Returns whether `id` was open. Recompiles, so the status chip and Problems follow.
    pub(crate) fn adopt_external_text(&mut self, id: &str, text: &str, saved: bool) -> bool {
        let Some(open) = self.open.as_mut().filter(|open| open.id == id) else {
            return false;
        };
        open.editor.replace_source(text, saved);
        self.editor_changed();
        if saved {
            self.refresh();
        }
        true
    }

    /// Recompiles the open script after an edit, feeding the diagnostics back to the editor.
    pub(crate) fn editor_changed(&mut self) {
        if let Some(open) = self.open.as_mut() {
            compile_into(&mut open.editor);
            let program =
                compiler::compile(&open.editor.source, Limits::default(), &LibraryRegistry);
            open.has_alerts = program.is_valid()
                && !crate::pine::alert_call::call_sites(&program.statements).is_empty();
            self.debug.set_map(
                program
                    .is_valid()
                    .then(|| crate::pine::debug::SourceMap::build(&program.statements)),
            );
        }
    }

    pub(crate) fn script_named(&self, id: &str) -> Option<&LocalScript> {
        self.scripts.iter().find(|s| s.id == id)
    }

    // MARK: Commands

    fn apply_cmds(&mut self, ctx: &egui::Context) {
        let cmds = std::mem::take(&mut self.cmds);
        for cmd in cmds {
            self.run(cmd, ctx);
        }
    }

    fn run(&mut self, cmd: Cmd, ctx: &egui::Context) {
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
            Cmd::Export(id) => self.begin_export(&id),
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
                self.with_store(demo::add_missing);
                self.refresh();
            }
            Cmd::AddToChart(id) => {
                if let Some(script) = self.script_named(&id) {
                    self.actions.push(ScriptManagerAction::AddToChart {
                        script_id: script.id.clone(),
                        name: script.name.clone(),
                        source: script.source.clone(),
                    });
                }
            }
            Cmd::CreateAlert(id) => self.open_alert_sheet(&id),
            Cmd::Save => self.save_open(),
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
            Cmd::ProfilerToggle => self.profiler_toggle(),
            Cmd::ProfilerRun => self.profiler_start(ctx),
            Cmd::ProfilerCancel => self.profiler.cancel(),
            Cmd::RevealLine(line) => {
                if let Some(open) = self.open.as_mut() {
                    open.editor.show_line(line);
                    ctx.request_repaint();
                }
            }
            Cmd::OpenAlpacaSettings => self.actions.push(ScriptManagerAction::OpenAlpacaSettings),
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
            crate::pine::alert_call::call_sites(&program.statements)
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

    fn commit_rename(&mut self) {
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

    fn save_open(&mut self) {
        let Some(open) = self.open.as_ref() else {
            return;
        };
        let id = open.id.clone();
        // The editor shows what lands on disk: a saved script always ends in a newline.
        let source = store::with_trailing_newline(&open.editor.source);
        if let Some(open) = self.open.as_mut() {
            open.editor.source.clone_from(&source);
        }
        if let Some(script) = self.with_store(|store| store.save(&id, &source)) {
            if let Some(open) = self.open.as_mut() {
                open.editor.mark_saved();
                open.script_type = script.script_type;
            }
            self.refresh();
        }
    }

    pub(crate) fn delete_script(&mut self, id: &str) {
        if self.open.as_ref().map(|o| o.id.as_str()) == Some(id) {
            self.close_script();
        }
        self.debug.script_deleted(id);
        self.profiler.script_deleted(id);
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

    /// The save panel runs off the UI thread; its result comes back through `export_rx`.
    fn begin_export(&mut self, id: &str) {
        let Some(script) = self.script_named(id) else {
            return;
        };
        let default = format!("{}.pine", script.name);
        let (sender, receiver) = mpsc::channel();
        self.export_rx = Some(receiver);
        let id = id.to_owned();
        std::thread::spawn(move || {
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
            let _ = sender.send((id, path));
        });
    }

    pub(crate) fn name_problem(&self, raw: &str) -> Option<String> {
        match name::validate(raw) {
            Err(error) => (!raw.trim().is_empty()).then(|| error.to_string()),
            Ok(clean) => self
                .scripts
                .iter()
                .any(|s| s.name.to_lowercase() == clean.to_lowercase())
                .then(|| store::ScriptStoreError::NameConflict.to_string()),
        }
    }
}

/// Where the Swift app keeps its data, to bring it across on first use.
fn swift_data_dir() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support/DegenView"))
}

fn swift_scripts_dir() -> Option<PathBuf> {
    swift_data_dir().map(|dir| dir.join("Scripts"))
}

/// Setting that records the Swift preview inputs were brought across.
const SWIFT_INPUTS_KEY: &str = "scripts.swiftInputsImported";

/// The Swift app's script UUID → script name, from its `ScriptMetadata/index.json` (a Swift
/// dictionary with `UUID` keys is encoded as a flat `[key, value, …]` array).
fn swift_script_names(data_dir: &Path) -> HashMap<String, String> {
    let Ok(bytes) = std::fs::read(data_dir.join("ScriptMetadata/index.json")) else {
        return HashMap::new();
    };
    parse_swift_index(&bytes)
}

fn parse_swift_index(bytes: &[u8]) -> HashMap<String, String> {
    let stem = |file: &str| file.strip_suffix(".pine").unwrap_or(file).to_owned();
    match serde_json::from_slice::<serde_json::Value>(bytes) {
        Ok(serde_json::Value::Array(flat)) => flat
            .as_chunks::<2>()
            .0
            .iter()
            .filter_map(|[id, file]| Some((id.as_str()?.to_owned(), stem(file.as_str()?))))
            .collect(),
        Ok(serde_json::Value::Object(map)) => map
            .into_iter()
            .filter_map(|(id, file)| Some((id, stem(file.as_str()?))))
            .collect(),
        _ => HashMap::new(),
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
mod tests {
    use super::*;
    use crate::store::Store;
    use std::sync::{Arc, Mutex};

    fn manager(tag: &str) -> (ScriptManager, PathBuf) {
        let root = std::env::temp_dir().join(format!("degenview-sm-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let shared: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let store = ScriptStore::new(root.join("Scripts"), root.join("ScriptMetadata"));
        (
            ScriptManager::with_script_store(1, shared, Some(store)),
            root,
        )
    }

    /// A manager with the example scripts loaded and the first one open and edited.
    fn edited(tag: &str) -> (ScriptManager, PathBuf, ScriptId) {
        let (mut manager, root) = manager(tag);
        manager.first_load();
        let id = manager.scripts[0].id.clone();
        manager.open_script(&id);
        let open = manager.open.as_mut().unwrap();
        open.editor.source.push_str("// edited\n");
        (manager, root, id)
    }

    /// With `DEGENVIEW_GALLERY_DIR` set, the sidebar is written as pictures: a row selected, the
    /// button idle and under the pointer, in both themes. Without it the test does nothing, so it
    /// never touches the shared theme.
    #[test]
    fn the_sidebar_gallery() {
        use crate::ui::render_tests::{Screen, gallery_dir};
        use crate::ui::theme::{self, AppTheme};
        use eframe::egui::{Event, Pos2, vec2};
        if gallery_dir().is_none() {
            return;
        }
        let (mut manager, root) = manager("sidebar-gallery");
        manager.first_load();
        let script = manager.scripts[2].clone();
        manager.selected_row = Some(RowKey {
            group: group_of(&script),
            id: script.id.clone(),
        });
        for (tag, appearance) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
            let mut screen = Screen::new(vec2(240.0, 420.0));
            theme::apply(&screen.ctx, appearance);
            for (name, pointer) in [
                ("idle", None),
                ("hover-button", Some(Pos2::new(120.0, 420.0 - 28.0 - 24.0))),
                ("hover-row", Some(Pos2::new(120.0, 120.0))),
            ] {
                let events = pointer
                    .map(|pos| vec![Event::PointerMoved(pos)])
                    .unwrap_or_default();
                screen.run(events.clone(), |ui| manager.sidebar_ui(ui));
                let output = screen.run(events, |ui| manager.sidebar_ui(ui));
                screen.save(&format!("sidebar-{tag}-{name}"), output);
            }
        }
        theme::apply(&egui::Context::default(), AppTheme::Dark);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn new_script_asks_for_a_name_unless_a_sheet_is_already_up() {
        let (mut manager, root) = manager("new-script");
        assert!(manager.can_create_script());
        manager.new_script();
        assert!(matches!(manager.cmds.as_slice(), [Cmd::NewScript]));
        manager.cmds.clear();
        manager.sheet = Some(Sheet::New {
            name: "kept".into(),
            error: None,
            needs_focus: false,
        });
        assert!(!manager.can_create_script());
        manager.new_script();
        assert!(manager.cmds.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn nothing_asks_while_every_script_is_saved() {
        let (mut manager, root) = manager("clean");
        manager.first_load();
        let id = manager.scripts[0].id.clone();
        manager.open_script(&id);
        assert!(!manager.begin_exit(ExitReason::CloseTab));
        assert!(manager.take_exit_decision().is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn saving_from_the_question_writes_the_script_and_lets_the_close_go() {
        let (mut manager, root, id) = edited("exit-save");
        assert!(manager.begin_exit(ExitReason::CloseTab));
        let prompt = manager.exit_prompt.clone().unwrap();
        assert_eq!(prompt.scripts.len(), 1);
        assert_eq!((prompt.scripts[0].added, prompt.scripts[0].removed), (1, 0));
        manager.resolve_exit(&prompt, UnsavedChoice::Save);
        assert_eq!(
            manager.take_exit_decision(),
            Some(ExitDecision {
                reason: ExitReason::CloseTab,
                proceed: true
            })
        );
        assert!(manager.exit_prompt.is_none());
        assert!(!manager.open.as_ref().unwrap().editor.is_dirty());
        let saved = manager.with_store(|store| store.script(&id)).flatten();
        assert!(saved.unwrap().source.contains("// edited"));
        assert!(!manager.begin_exit(ExitReason::Quit), "nothing left to ask");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn dont_save_puts_the_script_back_and_forgets_its_draft() {
        let (mut manager, root, id) = edited("exit-discard");
        let before = manager
            .open
            .as_ref()
            .unwrap()
            .editor
            .saved_source()
            .to_owned();
        manager.flush_draft();
        assert!(manager.begin_exit(ExitReason::Quit));
        let prompt = manager.exit_prompt.clone().unwrap();
        manager.resolve_exit(&prompt, UnsavedChoice::Discard);
        assert_eq!(manager.take_exit_decision().map(|d| d.proceed), Some(true));
        assert_eq!(manager.open.as_ref().unwrap().editor.source, before);
        assert!(
            manager
                .with_store(|store| store.draft(&id))
                .flatten()
                .is_none()
        );
        assert!(!manager.begin_exit(ExitReason::Quit));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cancel_keeps_the_edits_and_the_window() {
        let (mut manager, root, _) = edited("exit-cancel");
        assert!(manager.begin_exit(ExitReason::CloseTab));
        // Asking again while the question is up keeps it, and a quit retitles it.
        assert!(manager.begin_exit(ExitReason::Quit));
        let prompt = manager.exit_prompt.clone().unwrap();
        assert_eq!(prompt.reason, ExitReason::Quit);
        manager.resolve_exit(&prompt, UnsavedChoice::Cancel);
        assert_eq!(
            manager.take_exit_decision(),
            Some(ExitDecision {
                reason: ExitReason::Quit,
                proceed: false
            })
        );
        assert!(manager.open.as_ref().unwrap().editor.is_dirty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_script_edited_earlier_is_asked_about_through_its_draft() {
        let (mut manager, root, first) = edited("exit-drafts");
        // Switching away keeps the edit as a draft (typing leaves one behind after a pause).
        let edited = manager.open.as_ref().unwrap().editor.source.clone();
        let draft = model::ScriptDraft {
            script_id: first.clone(),
            source: edited,
            modified_at_ms: model::now_ms(),
            based_on_revision_id: None,
        };
        manager.with_store(|store| store.save_draft(&draft));
        let second = manager.scripts[1].id.clone();
        manager.open_script(&second);
        manager
            .open
            .as_mut()
            .unwrap()
            .editor
            .source
            .push_str("// also\n");
        assert!(manager.begin_exit(ExitReason::Quit));
        let prompt = manager.exit_prompt.clone().unwrap();
        let ids: Vec<&str> = prompt.scripts.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&first.as_str()) && ids.contains(&second.as_str()));
        manager.resolve_exit(&prompt, UnsavedChoice::Save);
        assert_eq!(manager.take_exit_decision().map(|d| d.proceed), Some(true));
        let saved = manager.with_store(|store| store.script(&first)).flatten();
        assert!(saved.unwrap().source.contains("// edited"));
        assert!(manager.unsaved_scripts().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_swift_index_reads_both_dictionary_encodings() {
        let flat =
            parse_swift_index(br#"["A-1","Delta Tether.pine","B-2","RSI Zones.pine","odd"]"#);
        assert_eq!(flat["A-1"], "Delta Tether");
        assert_eq!(flat.len(), 2);
        let object = parse_swift_index(br#"{"A-1":"Delta Tether.pine"}"#);
        assert_eq!(object["A-1"], "Delta Tether");
        assert!(parse_swift_index(b"nope").is_empty());
    }

    #[test]
    fn first_load_seeds_examples_once_and_never_for_existing_libraries() {
        let (mut manager, root) = manager("seed");
        manager.first_load();
        assert_eq!(manager.scripts.len(), 9);

        // A fresh settings store over the same folder: scripts exist, so nothing is added.
        let shared: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let store = ScriptStore::new(root.join("Scripts"), root.join("ScriptMetadata"));
        let mut again = ScriptManager::with_script_store(2, shared, Some(store));
        again.first_load();
        assert_eq!(again.scripts.len(), 9);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn create_select_rename_delete_flow() {
        let (mut manager, root) = manager("flow");
        manager.first_load();
        manager.create_script("Mine").unwrap();
        let cmds = std::mem::take(&mut manager.cmds);
        assert_eq!(cmds.len(), 1);
        for cmd in cmds {
            manager.run(cmd, &egui::Context::default());
        }
        assert_eq!(manager.open.as_ref().unwrap().name, "Mine");
        assert_eq!(manager.title(), "Mine");
        manager
            .open
            .as_mut()
            .unwrap()
            .editor
            .source
            .push_str("\n// edit");
        assert_eq!(manager.title(), "Mine — Edited");
        manager.save_open();
        assert_eq!(manager.title(), "Mine");

        assert!(
            manager.create_script("mine").is_err(),
            "case-insensitive conflict"
        );
        assert_eq!(
            manager.name_problem("MINE").as_deref(),
            Some("A script with that name already exists.")
        );
        assert_eq!(
            manager.name_problem("  "),
            None,
            "empty shows no error until it is submitted"
        );

        let id = manager.open.as_ref().unwrap().id.clone();
        manager.renaming = Some(Rename {
            row: RowKey {
                group: Group::Indicators,
                id: id.clone(),
            },
            text: "Renamed".into(),
            error: None,
            needs_focus: false,
        });
        manager.commit_rename();
        assert_eq!(manager.open.as_ref().unwrap().name, "Renamed");
        // A bad rename keeps editing and raises the alert.
        manager.renaming = Some(Rename {
            row: RowKey {
                group: Group::Indicators,
                id,
            },
            text: "a/b".into(),
            error: None,
            needs_focus: false,
        });
        manager.commit_rename();
        assert!(manager.renaming.is_some() && manager.error.is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_window_draws_headlessly_through_selection_and_layout_changes() {
        let (mut manager, root) = manager("draw");
        let ctx = egui::Context::default();
        let frame = |manager: &mut ScriptManager| {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                manager.ui(ui, None);
            });
            output.textures_delta.clear();
        };
        // Empty library and first load (seeds examples), then a selected script.
        frame(&mut manager);
        frame(&mut manager);
        assert_eq!(manager.scripts.len(), 9);
        let first = manager.scripts[0].clone();
        manager.cmds.push(Cmd::Select(RowKey {
            group: Group::Indicators,
            id: first.id,
        }));
        frame(&mut manager);
        frame(&mut manager);
        assert!(manager.open.is_some());
        for position in ChartPosition::ALL {
            manager.prefs.chart_position = position;
            frame(&mut manager);
        }
        for tab in DrawerTab::ALL {
            manager.cmds.push(Cmd::SetDrawerTab(tab));
            frame(&mut manager);
        }
        manager.cmds.push(Cmd::ToggleSidebar);
        manager.cmds.push(Cmd::ToggleChart);
        manager.cmds.push(Cmd::ToggleDrawer);
        manager.cmds.push(Cmd::NewScript);
        frame(&mut manager);
        frame(&mut manager);
        manager.sheet = None;
        manager.error = Some("boom".into());
        frame(&mut manager);
        let _ = std::fs::remove_dir_all(root);
    }

    fn target(
        script_id: Option<&str>,
        library: Option<&str>,
        ran_source: Option<&str>,
    ) -> crate::model::pine_logs_view::SourceTarget {
        crate::model::pine_logs_view::SourceTarget {
            script_id: script_id.map(str::to_owned),
            library: library.map(str::to_owned),
            offset: 17,
            ran_source: ran_source.map(str::to_owned),
        }
    }

    #[test]
    fn an_unknown_log_function_fails_the_editors_compile_not_just_the_run() {
        let source = "//@version=6\r\nindicator(\"T\")\r\n\r\nlog.hi(\"hiii\")\r\n";
        let mut editor = EditorState::new("id", source, source);
        compile_into(&mut editor);
        assert_eq!(editor.status, CompileStatus::Error);
        let problem = editor
            .diagnostics
            .iter()
            .find(|d| d.code == "PINE3051")
            .expect("log.hi is not a function")
            .clone();
        assert_eq!(
            (problem.range.start.line, problem.range.start.column),
            (4, 1)
        );
        // Revealing it puts the caret on the call, though the editor's text holds the CRs.
        editor.reveal(&problem);
        let index = syntax::compiled_utf16_to_char_index(
            &editor.source,
            editor.pending_jump().expect("a jump"),
        );
        assert!(
            editor
                .source
                .chars()
                .skip(index)
                .collect::<String>()
                .starts_with("log.hi")
        );
    }

    #[test]
    fn a_logged_call_opens_its_script_with_the_caret_on_it() {
        let (mut manager, root) = manager("reveal");
        manager.first_load();
        let script = manager.scripts[1].clone();
        manager.reveal_source(&target(Some(&script.id), None, Some(&script.source)));
        let open = manager.open.as_ref().expect("the script is open");
        assert_eq!(open.id, script.id);
        assert_eq!(open.editor.pending_jump(), Some(17));
        assert!(manager.error.is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_fresh_manager_survives_its_first_load_with_the_script_open_and_selected() {
        // The scripts are on disk from an earlier session; the Script Manager tab does not exist
        // yet when "Source code" is chosen, so the app opens it and asks before its first frame.
        let (mut seeded, root) = manager("reveal-fresh");
        seeded.first_load();
        let script = seeded.scripts[1].clone();
        let shared: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let store = ScriptStore::new(root.join("Scripts"), root.join("ScriptMetadata"));
        let mut fresh = ScriptManager::with_script_store(2, shared, Some(store));
        assert!(!fresh.loaded);
        fresh.reveal_source(&target(Some(&script.id), None, Some(&script.source)));
        let ctx = egui::Context::default();
        crate::ui::theme::install_offscreen(&ctx);
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                fresh.ui(ui, None);
            });
            output.textures_delta.clear();
        }
        assert!(fresh.loaded);
        assert_eq!(
            fresh.open.as_ref().map(|o| o.id.clone()),
            Some(script.id.clone())
        );
        assert_eq!(
            fresh.selected_row.as_ref().map(|row| row.id.clone()),
            Some(script.id)
        );
        assert!(fresh.error.is_none());
        // The editor took the caret request when it drew.
        assert_eq!(fresh.open.as_ref().unwrap().editor.pending_jump(), None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_script_already_in_the_editor_keeps_its_unsaved_text_and_history() {
        let (mut manager, root, id) = edited("reveal-open");
        let before = manager.open.as_ref().unwrap().editor.source.clone();
        assert!(before.ends_with("// edited\n"));
        manager.reveal_source(&target(Some(&id), None, None));
        let open = manager.open.as_ref().unwrap();
        assert_eq!(open.editor.source, before);
        assert_eq!(open.editor.pending_jump(), Some(17));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_script_edited_since_the_message_still_opens_with_a_note() {
        let (mut manager, root) = manager("reveal-changed");
        manager.first_load();
        let script = manager.scripts[1].clone();
        manager.reveal_source(&target(Some(&script.id), None, Some("// what ran then\n")));
        assert_eq!(manager.open.as_ref().map(|o| o.id.clone()), Some(script.id));
        assert!(
            manager
                .error
                .as_deref()
                .is_some_and(|note| note.contains("edited"))
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_script_that_is_gone_changes_nothing_and_says_so() {
        let (mut manager, root) = manager("reveal-gone");
        manager.first_load();
        let before = manager.open.as_ref().map(|o| o.id.clone());
        manager.reveal_source(&target(Some("no-such-script"), None, None));
        assert_eq!(manager.open.as_ref().map(|o| o.id.clone()), before);
        assert!(
            manager
                .error
                .as_deref()
                .is_some_and(|note| note.contains("no longer"))
        );
        manager.error = None;
        manager.reveal_source(&target(None, Some("Nope"), None));
        assert!(
            manager
                .error
                .as_deref()
                .is_some_and(|note| note.contains("library “Nope”"))
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_library_call_opens_the_library_by_name() {
        let (mut manager, root) = manager("reveal-library");
        manager.first_load();
        let library = manager
            .with_store(|store| {
                store.create(
                    "Dbg",
                    ScriptType::Library,
                    Some("//@version=6\nlibrary(\"Dbg\")\nexport f(float x) => x\n"),
                    false,
                )
            })
            .expect("a library");
        manager.reveal_source(&target(None, Some("Dbg"), None));
        let open = manager.open.as_ref().expect("the library is open");
        assert_eq!(open.id, library.id);
        assert_eq!(open.editor.pending_jump(), Some(17));
        assert!(manager.error.is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn create_alert_needs_a_chart_then_opens_the_sheet_for_the_script() {
        let (mut manager, root) = manager("alert");
        manager.first_load();
        let ctx = egui::Context::default();
        let demo = manager
            .scripts
            .iter()
            .find(|s| s.name == "Volume Spike Alert")
            .expect("a demo with alert() calls")
            .clone();

        manager.run(Cmd::CreateAlert(demo.id.clone()), &ctx);
        assert!(manager.sheet.is_none());
        assert!(
            manager
                .error
                .as_deref()
                .is_some_and(|e| e.contains("Open a chart"))
        );
        manager.error = None;

        manager.chart = Some(ChartInfo {
            chart_id: 3,
            title: "BTC/USDT".into(),
            source_name: "Binance".into(),
            timeframe: "1h".into(),
            symbol_key: "BINANCE:BTCUSDT".into(),
            instances: vec![PineInstance::new(
                11,
                Some(demo.id.clone()),
                &demo.name,
                &demo.source,
            )],
        });
        manager.run(Cmd::CreateAlert(demo.id), &ctx);
        let Some(Sheet::Alert(sheet)) = &manager.sheet else {
            panic!("the Create Script Alert sheet opens");
        };
        assert_eq!(sheet.name, "Volume Spike Alert");
        assert_eq!(sheet.instance.id, 11);
        assert!(!sheet.call_sites.is_empty(), "the example raises an alert");
        assert!(!sheet.duplicate_exists);
        // And it draws.
        for _ in 0..2 {
            let chart = manager.chart.clone();
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                manager.ui(ui, chart.clone());
            });
            output.textures_delta.clear();
        }
        assert!(
            matches!(manager.sheet, Some(Sheet::Alert(_))),
            "still open until chosen"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_outside_edit_reaches_the_open_editor_without_losing_or_duplicating_a_draft() {
        let (mut manager, root) = manager("external");
        manager.create_script("Ext").unwrap();
        for cmd in std::mem::take(&mut manager.cmds) {
            manager.run(cmd, &egui::Context::default());
        }
        let id = manager.open.as_ref().unwrap().id.clone();
        let saved_text = manager.open.as_ref().unwrap().editor.source.clone();
        assert_eq!(manager.open_text(&id).as_deref(), Some(saved_text.as_str()));
        assert_eq!(manager.open_text("other"), None);

        // A draft arrives: the editor shows it, dirty, with the saved text still the baseline.
        let draft = "//@version=6\nindicator(\"Ext\")\nplot(open)\n";
        assert!(manager.adopt_external_text(&id, draft, false));
        let open = manager.open.as_ref().unwrap();
        assert_eq!(open.editor.source, draft);
        assert!(open.editor.is_dirty());
        assert_eq!(open.editor.saved_source(), saved_text);
        assert!(
            !open.editor.has_pending_draft(),
            "the outside writer already stored it"
        );
        assert_eq!(open.editor.status, CompileStatus::Valid);

        // A bad draft shows its problems in the editor.
        assert!(manager.adopt_external_text(
            &id,
            "//@version=6\nindicator(\"Ext\")\nplot(\n",
            false
        ));
        let open = manager.open.as_ref().unwrap();
        assert_eq!(open.editor.status, CompileStatus::Error);
        assert!(!open.editor.diagnostics.is_empty());

        // A save: the text is now the baseline, so the editor is clean and the list is current.
        assert!(manager.adopt_external_text(&id, draft, true));
        let open = manager.open.as_ref().unwrap();
        assert!(!open.editor.is_dirty());
        assert_eq!(manager.title(), "Ext");

        // A script that is not open is left alone.
        assert!(!manager.adopt_external_text("not-open", draft, true));
        assert_eq!(manager.open.as_ref().unwrap().id, id);
        let _ = std::fs::remove_dir_all(root);
    }
}
