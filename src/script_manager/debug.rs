//! The debugger as the Script Manager drives it: one session at a time on the open script, the
//! breakpoints and watches saved for each script, and everything the panels show of a stopped
//! script.
//!
//! This is the model. It holds no egui types: `ui::pine_debug` draws it and returns
//! [`PanelAction`]s, the editor's gutter returns [`GutterEvent`]s, and the manager feeds both back
//! through [`DebugPanel::apply`]. Nothing here touches the interpreter: it sends commands to a
//! [`DebugSession`] and reads the events that come back.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use crate::model::Kline;
use crate::pine::debug::{
    BarBreak, BarContext, BreakpointState, BreakpointStatus, DebugCommand, DebugConfig, DebugEvent,
    DebugSession, DebugState, FrameInfo, HitRule, Outcome, PausedSnapshot, SourceMap, StopReason,
    ValueRef, Variable, VariableGroup, WatchRow, Watched, compile_breakpoints, parse_expression,
};
use crate::pine::logs::LogHistory;
use crate::pine::program::{ChartTheme, InputValue, SymbolInfo};
use crate::pine::{Diagnostic, DiagnosticSeverity};
use crate::store::SharedStore;

use super::debug_marks::{BreakpointMark, DebugMarks, GutterEvent, MarkKind, StopMark};
use super::debug_state::{DebugStates, KEY, ScriptDebugState, StoredBreakpoint};

/// What a session is started on: the preview chart's bars as they are now.
pub struct DebugFeed {
    pub bars: Vec<Kline>,
    pub symbol: SymbolInfo,
    pub inputs: HashMap<String, InputValue>,
    pub now_ms: i64,
    pub theme: ChartTheme,
    /// "BTCUSDT · 1D": shown beside the bar context.
    pub label: String,
}

/// How a session begins.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartMode {
    /// Run to the first breakpoint, or to the end.
    Run,
    /// Stop at the first statement.
    Entry,
}

/// A message under the toolbar, for what a click could not do.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Notice {
    pub text: String,
    pub error: bool,
}

/// One line of the debug console.
#[derive(Clone, Debug, PartialEq)]
pub struct ConsoleLine {
    pub input: String,
    pub result: Option<Watched>,
}

/// A change to a breakpoint made in its editor.
#[derive(Clone, Debug, PartialEq)]
pub enum BreakpointEdit {
    Enabled(bool),
    Condition(String),
    Hit(HitRule),
    LogMessage(Option<String>),
}

/// What the panels ask for.
#[derive(Clone, Debug, PartialEq)]
pub enum PanelAction {
    Command(DebugCommand),
    SelectFrame(usize),
    /// Open or close a row of the variables or watches tree. `key` names it by its path of names,
    /// so it stays open from stop to stop.
    ToggleRow {
        key: String,
        reference: ValueRef,
    },
    LoadMore {
        reference: ValueRef,
    },
    /// A row of the variables tree was picked: the arrow keys move from it.
    Cursor(String),
    AddWatch(String),
    EditWatch {
        index: usize,
        text: String,
    },
    RemoveWatch(usize),
    SetBreakpointEnabled {
        id: u64,
        enabled: bool,
    },
    EditBreakpoint {
        id: u64,
        edit: BreakpointEdit,
    },
    RemoveBreakpoint(u64),
    RemoveAllBreakpoints,
    EnableAllBreakpoints(bool),
    /// Show a source line in the editor.
    GotoLine(usize),
    /// Stop at the start of a bar: `text` is a bar index, or a time `YYYY-MM-DD HH:MM` in UTC.
    BreakOnBar(String),
    ClearBarBreak,
    Evaluate(String),
    ClearConsole,
    Close,
}

/// The sections of the debug panel's second column; each has a bit in `LayoutPrefs::debug_sections`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Section {
    Watches = 1,
    Stack = 2,
    Breakpoints = 4,
    Console = 8,
}

/// Which hit rule the breakpoint editor is composing.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HitKind {
    #[default]
    Always,
    Equals,
    EveryNth,
    GreaterThan,
}

impl HitKind {
    pub const ALL: [Self; 4] = [
        Self::Always,
        Self::Equals,
        Self::EveryNth,
        Self::GreaterThan,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Always => "Every time",
            Self::Equals => "On the hit number",
            Self::EveryNth => "Every nth hit",
            Self::GreaterThan => "After the nth hit",
        }
    }

    pub fn of(rule: HitRule) -> (Self, u64) {
        match rule {
            HitRule::Always => (Self::Always, 1),
            HitRule::Equals(n) => (Self::Equals, n),
            HitRule::EveryNth(n) => (Self::EveryNth, n),
            HitRule::GreaterThan(n) => (Self::GreaterThan, n),
        }
    }

    pub fn rule(self, n: u64) -> HitRule {
        match self {
            Self::Always => HitRule::Always,
            Self::Equals => HitRule::Equals(n),
            Self::EveryNth => HitRule::EveryNth(n),
            Self::GreaterThan => HitRule::GreaterThan(n),
        }
    }
}

/// The breakpoint being edited in its popover.
#[derive(Clone, Debug, PartialEq)]
pub struct BreakpointEditor {
    pub id: u64,
    pub condition: String,
    pub hit: HitKind,
    pub count: String,
    pub is_logpoint: bool,
    pub message: String,
}

/// The panel's own state: text being typed, what is open, a popover. Nothing here is the debugger's.
#[derive(Default)]
pub struct PanelUi {
    pub filter: String,
    pub watch_input: String,
    pub editing_watch: Option<(usize, String)>,
    pub console_input: String,
    pub bar_input: String,
    /// The variables row the keyboard is on, by its path of names.
    pub cursor: Option<String>,
    /// The index of the variables line the keyboard just moved to, until it is brought into view.
    pub reveal: Option<usize>,
    /// A scroll offset to apply to the variables list next frame, to bring the cursor into view.
    pub scroll_to: Option<f32>,
    pub editor: Option<BreakpointEditor>,
    /// A line whose breakpoint menu was asked for in the gutter, and where.
    pub menu_line: Option<usize>,
    pub copied_at: Option<std::time::Instant>,
}

impl PanelUi {
    /// Opens the editor for a breakpoint, or closes it if it is the one open.
    pub fn toggle_editor(&mut self, stored: &StoredBreakpoint) {
        if self
            .editor
            .as_ref()
            .is_some_and(|editor| editor.id == stored.spec.id)
        {
            self.editor = None;
        } else {
            self.open_editor(stored, false);
        }
    }

    /// Opens the editor for a breakpoint; `logpoint` starts it as one that logs a message.
    pub fn open_editor(&mut self, stored: &StoredBreakpoint, logpoint: bool) {
        let (hit, count) = HitKind::of(stored.spec.hit);
        self.editor = Some(BreakpointEditor {
            id: stored.spec.id,
            condition: stored.spec.condition.clone().unwrap_or_default(),
            hit,
            count: count.to_string(),
            is_logpoint: logpoint || stored.spec.log_message.is_some(),
            message: stored.spec.log_message.clone().unwrap_or_default(),
        });
    }
}

/// What a request in flight is for.
#[derive(Debug)]
enum Pending {
    Variables { frame: usize },
    Watches { frame: usize },
    Expand { reference: ValueRef },
    Console { index: usize },
}

/// Rows loaded under one expandable value.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Loaded {
    pub total: usize,
    pub items: Vec<Variable>,
    pub error: Option<String>,
}

/// What the panels show of the script while it is stopped.
pub struct Shown {
    pub snapshot: Box<PausedSnapshot>,
    /// The frame whose variables and watches are shown.
    pub frame: usize,
    pub variables: Vec<VariableGroup>,
    pub watches: Vec<WatchRow>,
    pub loaded: HashMap<ValueRef, Loaded>,
    /// Rows that are open, by path.
    pub open: HashSet<String>,
    requested: HashSet<ValueRef>,
}

pub struct DebugPanel {
    shared: SharedStore,
    states: DebugStates,
    script_id: Option<String>,
    pub state: ScriptDebugState,
    saved: ScriptDebugState,
    session: Option<DebugSession>,
    /// The text the session was started from.
    started_from: String,
    pub shown: Option<Shown>,
    pub outcome: Option<Outcome>,
    pub notice: Option<Notice>,
    pub logs: LogHistory,
    /// Changes each time a run starts, so the Logs view discards what it kept of the last.
    pub log_epoch: u64,
    pub hits: Vec<(u64, u64)>,
    pub label: String,
    pub bar_break: Option<String>,
    pub console: Vec<ConsoleLine>,
    /// Rows of the variables tree that stay open across stops.
    open_rows: HashSet<String>,
    pending: HashMap<u64, Pending>,
    map: Option<Arc<SourceMap>>,
    /// Counts the maps set, so the statuses are recomputed when there is a new one.
    map_serial: u64,
    /// The text the breakpoints' lines are those of.
    text: Option<String>,
    statuses: Vec<BreakpointStatus>,
    statuses_key: Option<(u64, u64)>,
    /// A change to `state` that has not been saved yet.
    dirty: bool,
    pub ui: PanelUi,
}

fn now_epoch() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static EPOCH: AtomicU64 = AtomicU64::new(1 << 40);
    EPOCH.fetch_add(1, Ordering::Relaxed)
}

impl DebugPanel {
    pub fn load(shared: &SharedStore) -> Self {
        let states = shared
            .lock()
            .ok()
            .and_then(|store| store.setting::<DebugStates>(KEY))
            .unwrap_or_default();
        Self {
            shared: shared.clone(),
            states,
            script_id: None,
            state: ScriptDebugState::default(),
            saved: ScriptDebugState::default(),
            session: None,
            started_from: String::new(),
            shown: None,
            outcome: None,
            notice: None,
            logs: LogHistory::default(),
            log_epoch: now_epoch(),
            hits: Vec::new(),
            label: String::new(),
            bar_break: None,
            console: Vec::new(),
            open_rows: HashSet::new(),
            pending: HashMap::new(),
            map: None,
            map_serial: 0,
            text: None,
            statuses: Vec::new(),
            statuses_key: None,
            dirty: false,
            ui: PanelUi::default(),
        }
    }

    // MARK: Scripts

    /// Another script is open: its breakpoints and watches replace these, and a session on the
    /// old one ends.
    pub fn script_changed(&mut self, id: &str) {
        if self.script_id.as_deref() == Some(id) {
            return;
        }
        self.save();
        self.end_session();
        self.script_id = Some(id.to_owned());
        self.state = self.states.get(id);
        self.saved = self.state.clone();
        self.map = None;
        self.text = None;
        self.statuses_key = None;
        self.notice = None;
        self.console.clear();
        self.bar_break = None;
    }

    pub fn script_deleted(&mut self, id: &str) {
        self.states.forget(id);
        self.write_states();
        if self.script_id.as_deref() == Some(id) {
            self.end_session();
            self.script_id = None;
            self.state = ScriptDebugState::default();
            self.saved = ScriptDebugState::default();
        }
    }

    fn write_states(&self) {
        if let Ok(store) = self.shared.lock() {
            let _ = store.set_setting(KEY, &self.states);
        }
    }

    /// Writes this script's breakpoints and watches if they changed.
    pub fn save(&mut self) {
        let Some(id) = self.script_id.clone() else {
            return;
        };
        if self.state != self.saved {
            self.states.set(&id, self.state.clone());
            self.write_states();
            self.saved = self.state.clone();
        }
    }

    /// The text changed from `old` to `new`: breakpoints follow their lines.
    pub fn follow_edit(&mut self, old: &str, new: &str) {
        self.state.follow_edit(old, new);
        self.sync_session_breakpoints();
    }

    // MARK: Compile map

    /// Keeps the statement map of the editor's text, which says which lines a breakpoint can stop
    /// at. A text that does not compile has none; the last good one stays until the next does.
    pub fn set_map(&mut self, map: Option<SourceMap>) {
        if let Some(map) = map {
            self.map = Some(Arc::new(map));
            self.map_serial += 1;
            self.statuses_key = None;
        }
    }

    /// The editor's text this frame: breakpoints follow the lines it moves.
    pub fn observe_text(&mut self, text: &str) {
        match &self.text {
            Some(previous) if previous == text => {}
            Some(previous) => {
                let previous = previous.clone();
                self.follow_edit(&previous, text);
                self.save();
                self.text = Some(text.to_owned());
            }
            None => self.text = Some(text.to_owned()),
        }
    }

    pub fn map(&self) -> Option<&Arc<SourceMap>> {
        self.map.as_ref()
    }

    /// What became of each breakpoint against the current text. Recomputed when either changes.
    pub fn statuses(&mut self) -> &[BreakpointStatus] {
        let key = (self.map_serial, self.revision());
        if self.statuses_key != Some(key) {
            self.statuses = match &self.map {
                Some(map) => compile_breakpoints(&self.state.specs(), map).1,
                None => Vec::new(),
            };
            self.statuses_key = Some(key);
        }
        &self.statuses
    }

    fn revision(&self) -> u64 {
        // The breakpoints' content, cheaply: any edit changes the serialised length or order.
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for stored in &self.state.breakpoints {
            for part in [
                stored.spec.id,
                stored.spec.line as u64,
                u64::from(stored.spec.enabled),
                stored
                    .spec
                    .condition
                    .as_ref()
                    .map_or(0, |c| c.len() as u64 + 1),
                stored
                    .spec
                    .log_message
                    .as_ref()
                    .map_or(0, |m| m.len() as u64 + 1),
                match stored.spec.hit {
                    HitRule::Always => 0,
                    HitRule::Equals(n) => 1 + (n << 3),
                    HitRule::EveryNth(n) => 2 + (n << 3),
                    HitRule::GreaterThan(n) => 3 + (n << 3),
                },
            ] {
                hash ^= part;
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        hash
    }

    pub fn status_of(&mut self, id: u64) -> Option<BreakpointState> {
        self.statuses()
            .iter()
            .find(|status| status.id == id)
            .map(|status| status.state.clone())
    }

    // MARK: Session

    pub fn state_of_session(&self) -> DebugState {
        self.session
            .as_ref()
            .map_or(DebugState::Idle, DebugSession::state)
    }

    pub fn has_session(&self) -> bool {
        self.session.is_some()
    }

    /// A handle that outlives the session, to watch its thread end.
    pub fn liveness(&self) -> Option<std::sync::Weak<()>> {
        self.session.as_ref().map(DebugSession::liveness)
    }

    pub fn is_paused(&self) -> bool {
        self.session.as_ref().is_some_and(DebugSession::is_paused)
    }

    pub fn paused_on_error(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(DebugSession::paused_on_error)
    }

    /// The script was edited after the session started: it is still running the old text.
    pub fn is_out_of_date(&self, current: &str) -> bool {
        self.session
            .as_ref()
            .is_some_and(|session| session.state().is_active() && !session.is_for_source(current))
            || (self.session.is_some() && self.started_from != current && self.outcome.is_none())
    }

    /// Whether the toolbar should offer `command`: the session accepts it, or there is none and it
    /// would start one.
    pub fn allows(&self, command: DebugCommand) -> bool {
        match &self.session {
            Some(session) if session.state().is_active() => session.allows(command),
            _ => {
                matches!(
                    command,
                    DebugCommand::Continue | DebugCommand::StepInto | DebugCommand::StepOver
                ) || (command == DebugCommand::Restart && self.session.is_some())
            }
        }
    }

    /// True when `Continue` would start a new run rather than resume one.
    pub fn would_start(&self) -> bool {
        self.session
            .as_ref()
            .is_none_or(|session| !session.state().is_active())
    }

    pub fn start(
        &mut self,
        source: &str,
        feed: DebugFeed,
        mode: StartMode,
        libraries: Box<dyn crate::pine::compiler::LibraryResolver + Send>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) {
        self.end_session();
        self.notice = None;
        self.outcome = None;
        let mut config = DebugConfig::new(source, feed.symbol, feed.bars, feed.now_ms);
        config.inputs = feed.inputs;
        config.theme = Some(feed.theme);
        config.libraries = libraries;
        config.security_data = Some(Box::new(crate::host::pine_host::KlineSecurityDataProvider));
        config.breakpoints = self.state.specs();
        config.watches = self.state.watches.clone();
        config.stop_on_entry = mode == StartMode::Entry;
        config.bar_break = self.bar_break.as_deref().and_then(parse_bar_break);
        match DebugSession::start(config, wake) {
            Ok(session) => {
                self.session = Some(session);
                self.started_from = source.to_owned();
                self.label = feed.label;
                self.logs = LogHistory::default();
                self.log_epoch = now_epoch();
                self.hits.clear();
            }
            Err(error) => {
                self.notice = Some(Notice {
                    text: error.to_string(),
                    error: true,
                });
            }
        }
    }

    /// Ends the session and forgets what it showed. Its breakpoints and watches stay.
    pub fn end_session(&mut self) {
        self.session = None;
        self.shown = None;
        self.pending.clear();
    }

    /// Applies the toolbar's commands. `Restart` is the manager's, because it needs a fresh feed.
    pub fn command(&mut self, command: DebugCommand) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        let accepted = session.command(command).is_ok();
        let resuming = matches!(
            command,
            DebugCommand::Continue
                | DebugCommand::StepInto
                | DebugCommand::StepOver
                | DebugCommand::StepOut
                | DebugCommand::StepBar
        );
        if accepted && resuming {
            self.pending.clear();
            if let Some(shown) = self.shown.as_mut() {
                // The panels keep showing the last stop, dimmed, until the next one arrives.
                shown.requested.clear();
            }
        }
        accepted
    }

    /// Reads what the session has said. Returns whether anything changed.
    pub fn poll(&mut self) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        let events = session.poll();
        let changed = !events.is_empty();
        for event in events {
            self.receive(event);
        }
        changed
    }

    fn receive(&mut self, event: DebugEvent) {
        match event {
            DebugEvent::Started => {}
            DebugEvent::Paused(snapshot) => {
                self.hits.clone_from(&snapshot.hits);
                self.pending.clear();
                if let Some(notice) = &snapshot.stop.notice {
                    self.notice = Some(Notice {
                        text: notice.clone(),
                        error: false,
                    });
                } else if self.notice.as_ref().is_some_and(|n| !n.error) {
                    self.notice = None;
                }
                let frame = snapshot.frame;
                self.shown = Some(Shown {
                    variables: snapshot.variables.clone(),
                    watches: snapshot.watches.clone(),
                    frame,
                    snapshot,
                    loaded: HashMap::new(),
                    open: self.open_rows.clone(),
                    requested: HashSet::new(),
                });
            }
            DebugEvent::Variables {
                token,
                frame,
                groups,
            } => {
                if let Some(Pending::Variables { .. }) = self.pending.remove(&token)
                    && let Some(shown) = self.shown.as_mut()
                    && shown.frame == frame
                {
                    shown.variables = groups;
                }
            }
            DebugEvent::Watches { token, frame, rows } => {
                if let Some(Pending::Watches { .. }) = self.pending.remove(&token)
                    && let Some(shown) = self.shown.as_mut()
                    && shown.frame == frame
                {
                    shown.watches = rows;
                }
            }
            DebugEvent::Expanded { token, result } => {
                if let Some(Pending::Expand { reference }) = self.pending.remove(&token)
                    && let Some(shown) = self.shown.as_mut()
                {
                    let entry = shown.loaded.entry(reference.clone()).or_default();
                    match result {
                        Ok(page) => {
                            entry.total = page.total;
                            entry.error = None;
                            entry.items.extend(page.items);
                        }
                        Err(error) => entry.error = Some(error.to_string()),
                    }
                    shown.requested.remove(&reference);
                }
            }
            DebugEvent::Evaluated { token, result } => {
                if let Some(Pending::Console { index }) = self.pending.remove(&token)
                    && let Some(line) = self.console.get_mut(index)
                {
                    line.result = Some(result);
                }
            }
            DebugEvent::Stale { token } => {
                self.pending.remove(&token);
            }
            DebugEvent::Logs(batch) => self.logs.apply(batch),
            DebugEvent::Finished { outcome, hits } => {
                self.hits = hits;
                if let Outcome::Failed(error) = &outcome
                    && self.shown.is_none()
                {
                    self.notice = Some(Notice {
                        text: error.message.clone(),
                        error: true,
                    });
                }
                self.outcome = Some(outcome);
            }
        }
    }

    /// Whether the Logs tab should show this panel's run rather than the preview chart's.
    pub fn has_logs(&self) -> bool {
        self.session.is_some()
    }

    pub fn wants_frames(&self) -> Option<Duration> {
        self.session
            .as_ref()
            .filter(|s| s.state().is_active())
            .map(|_| Duration::from_millis(100))
    }

    // MARK: Breakpoints from the editor

    /// A click or menu choice in the editor's gutter. Returns whether the breakpoints list should
    /// be shown, because something there is waiting to be edited.
    pub fn gutter(&mut self, event: GutterEvent) -> bool {
        match event {
            GutterEvent::Toggle { line } => {
                if self.state.breakpoint_at(line).is_none() && !self.allow_line(line) {
                    return false;
                }
                self.state.toggle(line);
                self.after_breakpoints_changed();
                false
            }
            GutterEvent::SetEnabled { line, enabled } => {
                if let Some(id) = self.state.breakpoint_at(line).map(|b| b.spec.id)
                    && let Some(stored) = self.state.breakpoint_mut(id)
                {
                    stored.spec.enabled = enabled;
                }
                self.after_breakpoints_changed();
                false
            }
            GutterEvent::Edit { line } | GutterEvent::AddLogpoint { line } => {
                let logpoint = matches!(event, GutterEvent::AddLogpoint { .. });
                if self.state.breakpoint_at(line).is_none() {
                    if !self.allow_line(line) {
                        return false;
                    }
                    self.state.toggle(line);
                    self.after_breakpoints_changed();
                }
                if let Some(stored) = self.state.breakpoint_at(line).cloned() {
                    self.ui.open_editor(&stored, logpoint);
                }
                true
            }
        }
    }

    /// Whether a breakpoint may be set on `line`: false, with the reason in the notice, when the
    /// compiled script has no code there.
    fn allow_line(&mut self, line: usize) -> bool {
        if let Some(map) = &self.map
            && let crate::pine::debug::LineResolution::NotExecutable(reason) =
                map.resolve_line(line)
        {
            self.notice = Some(Notice {
                text: format!("Line {line}: {}", reason.describe()),
                error: false,
            });
            return false;
        }
        true
    }

    fn after_breakpoints_changed(&mut self) {
        self.notice = self.notice.take().filter(|n| n.error);
        self.save();
        self.sync_session_breakpoints();
    }

    fn sync_session_breakpoints(&self) {
        if let Some(session) = &self.session {
            session.set_breakpoints(self.state.specs());
        }
    }

    // MARK: Actions

    /// Applies what a panel asked for. Returns a source line to show, if any.
    pub fn apply(&mut self, action: PanelAction) -> Option<usize> {
        match action {
            PanelAction::Command(_) | PanelAction::Close => None,
            PanelAction::SelectFrame(frame) => {
                self.select_frame(frame);
                self.shown
                    .as_ref()
                    .and_then(|shown| shown.snapshot.stack.iter().find(|f| f.index == frame))
                    .and_then(|f| f.location.map(|l| l.line))
            }
            PanelAction::ToggleRow { key, reference } => {
                let opening = self.open_rows.insert(key.clone());
                if !opening {
                    self.open_rows.remove(&key);
                }
                if let Some(shown) = self.shown.as_mut() {
                    if opening {
                        shown.open.insert(key);
                    } else {
                        shown.open.remove(&key);
                    }
                }
                if opening {
                    self.request_rows(&reference, 0);
                }
                None
            }
            PanelAction::Cursor(key) => {
                self.ui.cursor = Some(key);
                None
            }
            PanelAction::LoadMore { reference } => {
                let loaded = self
                    .shown
                    .as_ref()
                    .and_then(|shown| shown.loaded.get(&reference))
                    .map_or(0, |loaded| loaded.items.len());
                self.request_rows(&reference, loaded);
                None
            }
            PanelAction::AddWatch(text) => {
                match parse_expression(&text) {
                    Ok(_) => {
                        if self.state.add_watch(&text) {
                            self.watches_changed();
                        }
                    }
                    Err(_) => {
                        // A watch that cannot work is still kept, so its reason shows next to it.
                        if self.state.add_watch(&text) {
                            self.watches_changed();
                        }
                    }
                }
                None
            }
            PanelAction::EditWatch { index, text } => {
                if self.state.edit_watch(index, &text) {
                    self.watches_changed();
                }
                None
            }
            PanelAction::RemoveWatch(index) => {
                if self.state.remove_watch(index) {
                    self.watches_changed();
                }
                None
            }
            PanelAction::SetBreakpointEnabled { id, enabled } => {
                if let Some(stored) = self.state.breakpoint_mut(id) {
                    stored.spec.enabled = enabled;
                }
                self.after_breakpoints_changed();
                None
            }
            PanelAction::EditBreakpoint { id, edit } => {
                if let Some(stored) = self.state.breakpoint_mut(id) {
                    match edit {
                        BreakpointEdit::Enabled(enabled) => stored.spec.enabled = enabled,
                        BreakpointEdit::Condition(text) => {
                            stored.spec.condition = Some(text).filter(|t| !t.trim().is_empty());
                        }
                        BreakpointEdit::Hit(rule) => stored.spec.hit = rule,
                        BreakpointEdit::LogMessage(message) => {
                            stored.spec.log_message = message.filter(|m| !m.is_empty());
                        }
                    }
                }
                self.after_breakpoints_changed();
                None
            }
            PanelAction::RemoveBreakpoint(id) => {
                self.state.remove(id);
                self.after_breakpoints_changed();
                None
            }
            PanelAction::RemoveAllBreakpoints => {
                self.state.remove_all();
                self.after_breakpoints_changed();
                None
            }
            PanelAction::EnableAllBreakpoints(enabled) => {
                self.state.enable_all(enabled);
                self.after_breakpoints_changed();
                None
            }
            PanelAction::GotoLine(line) => Some(line),
            PanelAction::BreakOnBar(text) => {
                match parse_bar_break(&text) {
                    Some(target) => {
                        self.bar_break = Some(text.trim().to_owned());
                        if let Some(session) = &self.session {
                            session.set_bar_break(Some(target));
                        }
                        self.notice = None;
                    }
                    None => {
                        self.notice = Some(Notice {
                            text: "Enter a bar index, or a time like 2026-10-24 12:00 (UTC)."
                                .to_owned(),
                            error: true,
                        });
                    }
                }
                None
            }
            PanelAction::ClearBarBreak => {
                self.bar_break = None;
                if let Some(session) = &self.session {
                    session.set_bar_break(None);
                }
                None
            }
            PanelAction::Evaluate(text) => {
                self.evaluate(text);
                None
            }
            PanelAction::ClearConsole => {
                self.console.clear();
                None
            }
        }
    }

    fn watches_changed(&mut self) {
        self.save();
        if let Some(session) = &self.session {
            session.set_watches(self.state.watches.clone());
        }
        // The rows shown are the old list; ask for them again.
        let frame = self.shown.as_ref().map(|shown| shown.frame);
        if let (Some(frame), Some(session)) = (frame, self.session.as_mut())
            && session.is_paused()
            && let Some(token) = session.request_watches(frame)
        {
            self.pending.insert(token, Pending::Watches { frame });
        }
    }

    fn select_frame(&mut self, frame: usize) {
        let Some(shown) = self.shown.as_mut() else {
            return;
        };
        if shown.frame == frame {
            return;
        }
        shown.frame = frame;
        shown.loaded.clear();
        shown.requested.clear();
        let Some(session) = self.session.as_mut() else {
            return;
        };
        if let Some(token) = session.request_variables(frame) {
            self.pending.insert(token, Pending::Variables { frame });
        }
        if let Some(token) = session.request_watches(frame) {
            self.pending.insert(token, Pending::Watches { frame });
        }
    }

    fn request_rows(&mut self, reference: &ValueRef, start: usize) {
        let (Some(shown), Some(session)) = (self.shown.as_mut(), self.session.as_mut()) else {
            return;
        };
        if shown.requested.contains(reference) {
            return;
        }
        if start == 0 && shown.loaded.contains_key(reference) {
            return;
        }
        if let Some(token) =
            session.request_expand(reference.clone(), start, crate::pine::debug::PAGE_LIMIT)
        {
            shown.requested.insert(reference.clone());
            self.pending.insert(
                token,
                Pending::Expand {
                    reference: reference.clone(),
                },
            );
        }
    }

    /// Asks for the rows of an open row that has not been loaded. Called by the panel while drawing.
    pub fn ensure_rows(&mut self, reference: &ValueRef) {
        let missing = self
            .shown
            .as_ref()
            .is_some_and(|shown| !shown.loaded.contains_key(reference));
        if missing {
            self.request_rows(reference, 0);
        }
    }

    pub fn is_loading(&self, reference: &ValueRef) -> bool {
        self.shown
            .as_ref()
            .is_some_and(|shown| shown.requested.contains(reference))
    }

    fn evaluate(&mut self, text: String) {
        let text = text.trim().to_owned();
        if text.is_empty() {
            return;
        }
        let index = self.console.len();
        self.console.push(ConsoleLine {
            input: text.clone(),
            result: None,
        });
        if self.console.len() > 200 {
            self.console.remove(0);
        }
        let index = index.min(self.console.len() - 1);
        let frame = self.shown.as_ref().map(|shown| shown.frame);
        let requested = match (frame, self.session.as_mut()) {
            (Some(frame), Some(session)) => session.request_evaluate(frame, text),
            _ => None,
        };
        match requested {
            Some(token) => {
                self.pending.insert(token, Pending::Console { index });
            }
            None => {
                self.console[index].result = Some(Watched::Error(
                    "Stop the script at a breakpoint to evaluate expressions.".to_owned(),
                ));
            }
        }
    }

    // MARK: For the editor

    /// The breakpoint markers and the stopped line to draw, and the values for hovering.
    pub fn marks(&mut self) -> DebugMarks {
        let resolved = self.map.is_some();
        let statuses: HashMap<u64, bool> = self
            .statuses()
            .iter()
            .map(|status| {
                (
                    status.id,
                    matches!(status.state, BreakpointState::Verified { .. }),
                )
            })
            .collect();
        let breakpoints = self
            .state
            .breakpoints
            .iter()
            .filter(|stored| !stored.unresolved)
            .map(|stored| BreakpointMark {
                line: stored.spec.line,
                kind: if stored.spec.log_message.is_some() {
                    MarkKind::Logpoint
                } else if stored.spec.condition.is_some() || stored.spec.hit != HitRule::Always {
                    MarkKind::Conditional
                } else {
                    MarkKind::Line
                },
                enabled: stored.spec.enabled,
                unverified: !resolved || !statuses.get(&stored.spec.id).copied().unwrap_or(false),
            })
            .collect();
        let mut marks = DebugMarks {
            breakpoints,
            ..DebugMarks::default()
        };
        if let Some(shown) = &self.shown
            && self.is_paused()
        {
            let innermost = shown.snapshot.stack.first().map(|f| f.index);
            let line = shown
                .snapshot
                .stack
                .iter()
                .find(|frame| frame.index == shown.frame)
                .and_then(|frame| frame.location.map(|l| l.line));
            if let Some(line) = line {
                marks.stop = Some(StopMark {
                    line,
                    innermost: innermost == Some(shown.frame),
                    error: matches!(shown.snapshot.stop.reason, StopReason::Exception(_)),
                });
            }
            for group in &shown.variables {
                for variable in &group.variables {
                    if variable.assigned {
                        marks.values.insert(
                            variable.name.clone(),
                            (variable.value.clone(), variable.type_name.clone()),
                        );
                    }
                }
            }
        }
        marks
    }

    /// The call stack, innermost first, when stopped.
    pub fn stack(&self) -> &[FrameInfo] {
        self.shown
            .as_ref()
            .map_or(&[], |shown| shown.snapshot.stack.as_slice())
    }

    pub fn bar(&self) -> Option<&BarContext> {
        self.shown.as_ref().map(|shown| &shown.snapshot.bar)
    }

    /// An error that stopped the script.
    pub fn failure(&self) -> Option<&Diagnostic> {
        match (&self.shown, &self.outcome) {
            (Some(shown), _) => match &shown.snapshot.stop.reason {
                StopReason::Exception(error) => Some(error),
                _ => None,
            },
            (None, Some(Outcome::Failed(error))) => Some(error),
            _ => None,
        }
    }

    pub fn has_errors(&self, diagnostics: &[Diagnostic]) -> bool {
        diagnostics
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error)
    }
}

impl Drop for DebugPanel {
    fn drop(&mut self) {
        self.save();
    }
}

/// A bar index (`500`), or a UTC time (`2026-10-24 12:00`, `2026-10-24T12:00`, `2026-10-24`).
pub fn parse_bar_break(text: &str) -> Option<BarBreak> {
    let text = text.trim();
    if let Ok(index) = text.parse::<i64>() {
        return (index >= 0).then_some(BarBreak::Index(index));
    }
    let (date, time) = match text.split_once([' ', 'T']) {
        Some((date, time)) => (date, time.trim_end_matches('Z')),
        None => (text, "00:00"),
    };
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let mut clock = time.split(':');
    let hour: i64 = clock.next()?.parse().ok()?;
    let minute: i64 = clock.next().map_or(Some(0), |m| m.parse().ok())?;
    let second: i64 = clock.next().map_or(Some(0), |s| s.parse().ok())?;
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) || !(0..60).contains(&second) {
        return None;
    }
    // Days since 1970-01-01 (Howard Hinnant's civil-from-days, inverted).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some(BarBreak::Time(
        (days * 86_400 + hour * 3_600 + minute * 60 + second) * 1_000,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bar_break_is_an_index_or_a_utc_time() {
        assert_eq!(parse_bar_break("500"), Some(BarBreak::Index(500)));
        assert_eq!(parse_bar_break(" 0 "), Some(BarBreak::Index(0)));
        assert_eq!(parse_bar_break("-3"), None);
        assert_eq!(
            parse_bar_break("1970-01-02 00:00"),
            Some(BarBreak::Time(86_400_000))
        );
        assert_eq!(
            parse_bar_break("2026-10-24 12:00"),
            Some(BarBreak::Time(1_792_843_200_000))
        );
        assert_eq!(
            parse_bar_break("2026-10-24T12:00:30Z"),
            Some(BarBreak::Time(1_792_843_230_000))
        );
        assert_eq!(
            parse_bar_break("2000-03-01"),
            Some(BarBreak::Time(951_868_800_000))
        );
        assert_eq!(parse_bar_break("2026-13-01"), None);
        assert_eq!(parse_bar_break("2026-10-24 25:00"), None);
        assert_eq!(parse_bar_break("soon"), None);
        assert_eq!(parse_bar_break(""), None);
    }
}
