//! A debugging session: the script running on a thread of its own, and the handle the UI keeps.
//!
//! The thread owns a complete [`ExecutionController`] over a frozen copy of the bars, so a session
//! shares nothing with the chart's own run: no alerts, no orders, no preview state, and a paused
//! script holds no lock anyone else wants. The two sides talk through bounded channels. Whatever
//! needs the script's values (variables, expansions, watches) is asked of the thread while it is
//! stopped and answered by it, because the values live on its stack and nowhere else.
//!
//! Requests carry the generation of the stop they were made for, so a command or inspection issued
//! against a stop the script has left is ignored, not applied to the next one.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use crate::model::Kline;
use crate::pine::compiler::{self, LibraryResolver};
use crate::pine::execution::ExecutionController;
use crate::pine::logs::LogBatch;
use crate::pine::program::{ChartTheme, InputValue, SymbolInfo};
use crate::pine::runtime_support::SecurityDataProvider;
use crate::pine::{Diagnostic, Limits};

use super::breakpoints::BreakpointSpec;
use super::commands::{DebugCommand, Resume};
use super::expression::{self, DebugExpression};
use super::frames::FrameInfo;
use super::inspect::{InspectError, Page, ValueRef, VariableGroup, Watched};
use super::runtime::{
    BarBreak, BarContext, DebugControl, DebugOptions, DebugRuntime, Debugger, Stop, StopReason,
    StopView,
};
use super::source_map::SourceMap;
use super::state::{DebugState, Trigger};

const EVENT_CAPACITY: usize = 512;
const REQUEST_CAPACITY: usize = 256;
/// How often a stopped script looks up from its requests to see whether it was cancelled.
const POLL: Duration = Duration::from_millis(40);

pub struct DebugConfig {
    pub source: String,
    pub inputs: HashMap<String, InputValue>,
    pub symbol: SymbolInfo,
    /// The bars as the script receives them, frozen: later ticks and refetches of the chart do not
    /// reach a running session.
    pub bars: Vec<Kline>,
    pub now_ms: i64,
    pub visible_range: Option<(i64, i64)>,
    pub limits: Limits,
    pub theme: Option<ChartTheme>,
    pub bar_seconds: Option<f64>,
    pub libraries: Box<dyn LibraryResolver + Send>,
    pub security_data: Option<Box<dyn SecurityDataProvider>>,
    pub breakpoints: Vec<BreakpointSpec>,
    pub watches: Vec<String>,
    /// Stop at the first statement instead of running to the first breakpoint.
    pub stop_on_entry: bool,
    pub bar_break: Option<BarBreak>,
}

impl DebugConfig {
    pub fn new(
        source: impl Into<String>,
        symbol: SymbolInfo,
        bars: Vec<Kline>,
        now_ms: i64,
    ) -> Self {
        struct NoLibraries;
        impl LibraryResolver for NoLibraries {
            fn source_for_library(&self, _: &str) -> Option<String> {
                None
            }
        }
        Self {
            source: source.into(),
            inputs: HashMap::new(),
            symbol,
            bars,
            now_ms,
            visible_range: None,
            limits: Limits::default(),
            theme: None,
            bar_seconds: None,
            libraries: Box::new(NoLibraries),
            security_data: None,
            breakpoints: Vec::new(),
            watches: Vec::new(),
            stop_on_entry: false,
            bar_break: None,
        }
    }
}

#[derive(Debug)]
pub enum StartError {
    NoBars,
    /// The source does not compile; the diagnostic is the first error.
    Compile(Diagnostic),
    Thread(String),
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoBars => f.write_str("There are no bars to run the script on yet."),
            Self::Compile(error) => write!(f, "The script has errors: {}", error.message),
            Self::Thread(error) => write!(f, "Could not start the debugger: {error}"),
        }
    }
}

impl std::error::Error for StartError {}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// The script ran to the end of the bars.
    Completed,
    /// The person ended it.
    Stopped,
    /// The script failed.
    Failed(Diagnostic),
}

/// A watch and what it came to at a stop.
#[derive(Clone, Debug, PartialEq)]
pub struct WatchRow {
    pub text: String,
    pub result: Watched,
}

/// Everything the panels need the moment the script stops, so showing a stop costs no round trip.
#[derive(Clone, Debug)]
pub struct PausedSnapshot {
    pub stop: Stop,
    /// Innermost first.
    pub stack: Vec<FrameInfo>,
    pub bar: BarContext,
    /// The frame the variables and watches below belong to: the innermost.
    pub frame: usize,
    pub variables: Vec<VariableGroup>,
    pub watches: Vec<WatchRow>,
    pub hits: Vec<(u64, u64)>,
}

#[derive(Debug)]
pub enum DebugEvent {
    /// The script began running.
    Started,
    Paused(Box<PausedSnapshot>),
    Variables {
        token: u64,
        frame: usize,
        groups: Vec<VariableGroup>,
    },
    Expanded {
        token: u64,
        result: Result<Page, InspectError>,
    },
    Watches {
        token: u64,
        frame: usize,
        rows: Vec<WatchRow>,
    },
    /// The answer to a single expression (hover, console).
    Evaluated {
        token: u64,
        result: Watched,
    },
    /// A request made against a stop the script has since left.
    Stale {
        token: u64,
    },
    Logs(LogBatch),
    Finished {
        outcome: Outcome,
        hits: Vec<(u64, u64)>,
    },
}

enum Request {
    Resume {
        generation: u64,
        resume: Resume,
    },
    Variables {
        generation: u64,
        token: u64,
        frame: usize,
    },
    Expand {
        generation: u64,
        token: u64,
        reference: ValueRef,
        start: usize,
        limit: usize,
    },
    Watches {
        generation: u64,
        token: u64,
        frame: usize,
    },
    Evaluate {
        generation: u64,
        token: u64,
        frame: usize,
        text: String,
    },
    SetWatches(Vec<String>),
}

#[derive(Debug, Eq, PartialEq)]
pub enum Rejected {
    /// The command does not apply in the session's state.
    NotNow,
    /// The session has ended or its thread is gone.
    Closed,
}

/// The UI's end of a session.
pub struct DebugSession {
    events: Receiver<DebugEvent>,
    requests: SyncSender<Request>,
    control: DebugControl,
    cancel: Arc<AtomicBool>,
    map: Arc<SourceMap>,
    state: DebugState,
    /// The stop the script is at, while it is at one.
    paused_at: Option<u64>,
    paused_on_error: bool,
    next_token: u64,
    source_hash: u64,
    alive: Weak<()>,
}

impl DebugSession {
    /// Compiles the script and starts it on a new thread. `wake` is called from that thread
    /// whenever there is an event to poll, to ask the UI for a frame.
    pub fn start(
        config: DebugConfig,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, StartError> {
        if config.bars.is_empty() {
            return Err(StartError::NoBars);
        }
        let program = compiler::compile(&config.source, config.limits, config.libraries.as_ref());
        if let Some(error) = program
            .diagnostics
            .iter()
            .find(|d| d.severity == crate::pine::DiagnosticSeverity::Error)
        {
            return Err(StartError::Compile(error.clone()));
        }
        let map = Arc::new(SourceMap::build(&program.statements));
        let source_hash = hash(&config.source);
        let control = DebugControl::default();
        control.set_bar_break(config.bar_break);
        let cancel = Arc::new(AtomicBool::new(false));
        let (event_sender, events) = mpsc::sync_channel(EVENT_CAPACITY);
        let (requests, request_receiver) = mpsc::sync_channel(REQUEST_CAPACITY);

        let alive = Arc::new(());
        let liveness = Arc::downgrade(&alive);
        let thread = Thread {
            config,
            map: Arc::clone(&map),
            control: control.clone(),
            cancel: Arc::clone(&cancel),
            events: event_sender,
            requests: request_receiver,
            wake,
        };
        let spawned = std::thread::Builder::new()
            .name("pine-debug".to_owned())
            .spawn(move || {
                let _alive = alive;
                thread.run();
            });
        if let Err(error) = spawned {
            return Err(StartError::Thread(error.to_string()));
        }
        let state = DebugState::Idle
            .on(Trigger::Start)
            .unwrap_or(DebugState::Preparing);
        Ok(Self {
            events,
            requests,
            control,
            cancel,
            map,
            state,
            paused_at: None,
            paused_on_error: false,
            next_token: 0,
            source_hash,
            alive: liveness,
        })
    }

    /// Whether the session's thread is still running. A closed or finished session's thread ends
    /// within a moment.
    pub fn thread_alive(&self) -> bool {
        self.alive.strong_count() > 0
    }

    /// A handle that outlives the session, to watch its thread end.
    pub fn liveness(&self) -> Weak<()> {
        self.alive.clone()
    }

    pub fn state(&self) -> DebugState {
        self.state
    }

    pub fn paused_on_error(&self) -> bool {
        self.paused_on_error
    }

    /// The map of the program this session runs, for resolving lines to statements.
    pub fn source_map(&self) -> &Arc<SourceMap> {
        &self.map
    }

    /// Identifies the source text this session was started from. Editing the script does not
    /// change the session; compare this to the editor's text to say it is out of date.
    pub fn source_hash(&self) -> u64 {
        self.source_hash
    }

    pub fn is_for_source(&self, source: &str) -> bool {
        hash(source) == self.source_hash
    }

    pub fn control(&self) -> &DebugControl {
        &self.control
    }

    pub fn allows(&self, command: DebugCommand) -> bool {
        self.state.allows(command, self.paused_on_error)
    }

    /// Whether inspection requests can be made: the script is stopped.
    pub fn is_paused(&self) -> bool {
        self.paused_at.is_some()
    }

    /// Applies a command. Resuming commands are refused unless the script is stopped; there is no
    /// queue of commands to run later.
    pub fn command(&mut self, command: DebugCommand) -> Result<(), Rejected> {
        if !self.allows(command) {
            return Err(Rejected::NotNow);
        }
        match command {
            DebugCommand::Pause => {
                self.control.request_pause();
                self.transition(Trigger::PauseRequested);
            }
            DebugCommand::Stop => {
                self.cancel.store(true, Ordering::Release);
                if let Some(generation) = self.paused_at.take() {
                    // A stopped script is waiting for a request, not watching the flag closely.
                    let _ = self.requests.try_send(Request::Resume {
                        generation,
                        resume: Resume::Stop,
                    });
                }
                self.transition(Trigger::StopRequested);
            }
            DebugCommand::Restart => return Err(Rejected::NotNow),
            resuming => {
                let (Some(resume), Some(generation)) = (resuming.resume(), self.paused_at) else {
                    return Err(Rejected::NotNow);
                };
                self.requests
                    .try_send(Request::Resume { generation, resume })
                    .map_err(|_| Rejected::Closed)?;
                self.paused_at = None;
                self.transition(Trigger::Resumed);
            }
        }
        Ok(())
    }

    pub fn set_breakpoints(&self, breakpoints: Vec<BreakpointSpec>) {
        self.control.set_breakpoints(breakpoints);
    }

    pub fn set_bar_break(&self, target: Option<BarBreak>) {
        self.control.set_bar_break(target);
    }

    /// Replaces the watch list the thread evaluates at every stop.
    pub fn set_watches(&self, watches: Vec<String>) {
        let _ = self.requests.try_send(Request::SetWatches(watches));
    }

    fn token(&mut self) -> u64 {
        self.next_token += 1;
        self.next_token
    }

    /// Asks for the variables of `frame`. The answer arrives as [`DebugEvent::Variables`]. `None`
    /// when the script is not stopped.
    pub fn request_variables(&mut self, frame: usize) -> Option<u64> {
        let generation = self.paused_at?;
        let token = self.token();
        self.requests
            .try_send(Request::Variables {
                generation,
                token,
                frame,
            })
            .ok()
            .map(|()| token)
    }

    pub fn request_expand(
        &mut self,
        reference: ValueRef,
        start: usize,
        limit: usize,
    ) -> Option<u64> {
        let generation = self.paused_at?;
        let token = self.token();
        self.requests
            .try_send(Request::Expand {
                generation,
                token,
                reference,
                start,
                limit,
            })
            .ok()
            .map(|()| token)
    }

    pub fn request_watches(&mut self, frame: usize) -> Option<u64> {
        let generation = self.paused_at?;
        let token = self.token();
        self.requests
            .try_send(Request::Watches {
                generation,
                token,
                frame,
            })
            .ok()
            .map(|()| token)
    }

    /// Evaluates one expression in `frame`: a hover, or a line typed in the console.
    pub fn request_evaluate(&mut self, frame: usize, text: String) -> Option<u64> {
        let generation = self.paused_at?;
        let token = self.token();
        self.requests
            .try_send(Request::Evaluate {
                generation,
                token,
                frame,
                text,
            })
            .ok()
            .map(|()| token)
    }

    fn transition(&mut self, trigger: Trigger) {
        if let Ok(next) = self.state.on(trigger) {
            self.state = next;
        }
    }

    fn apply(&mut self, event: &DebugEvent) {
        match event {
            DebugEvent::Started => self.transition(Trigger::Began),
            DebugEvent::Paused(snapshot) => {
                // A pause that arrives after a stop was asked for does not undo the stop.
                if self.state != DebugState::Stopping {
                    self.transition(Trigger::Stopped);
                    self.paused_at = Some(snapshot.stop.generation);
                    self.paused_on_error = matches!(snapshot.stop.reason, StopReason::Exception(_));
                }
            }
            DebugEvent::Finished { outcome, .. } => {
                self.paused_at = None;
                match outcome {
                    Outcome::Failed(_) => self.transition(Trigger::Errored),
                    _ => self.transition(Trigger::Finished),
                }
            }
            _ => {}
        }
    }

    /// Everything the thread has said since the last poll, in order. Never blocks.
    pub fn poll(&mut self) -> Vec<DebugEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            self.apply(&event);
            events.push(event);
        }
        events
    }

    /// Waits up to `timeout` for the next event.
    pub fn wait(&mut self, timeout: Duration) -> Option<DebugEvent> {
        let event = self.events.recv_timeout(timeout).ok()?;
        self.apply(&event);
        Some(event)
    }
}

impl Drop for DebugSession {
    fn drop(&mut self) {
        // The thread ends at its next safepoint; dropping the channels wakes a stopped one.
        self.cancel.store(true, Ordering::Release);
    }
}

fn hash(text: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

// MARK: The thread

struct Thread {
    config: DebugConfig,
    map: Arc<SourceMap>,
    control: DebugControl,
    cancel: Arc<AtomicBool>,
    events: SyncSender<DebugEvent>,
    requests: Receiver<Request>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

/// What the thread and the debugger inside the script share.
struct Link {
    events: SyncSender<DebugEvent>,
    cancel: Arc<AtomicBool>,
    wake: Arc<dyn Fn() + Send + Sync>,
    user_stopped: Arc<AtomicBool>,
}

impl Link {
    /// Sends an event, waiting for room. Gives up when the session is cancelled or the UI is gone.
    fn emit(&self, event: DebugEvent) -> bool {
        let mut event = event;
        loop {
            if self.cancel.load(Ordering::Acquire) {
                return false;
            }
            match self.events.try_send(event) {
                Ok(()) => {
                    (self.wake)();
                    return true;
                }
                Err(TrySendError::Full(returned)) => {
                    event = returned;
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(TrySendError::Disconnected(_)) => return false,
            }
        }
    }

    /// The last events of a session are delivered even though the session is over.
    fn emit_final(&self, event: DebugEvent) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut event = event;
        while Instant::now() < deadline {
            match self.events.try_send(event) {
                Ok(()) => {
                    (self.wake)();
                    return;
                }
                Err(TrySendError::Full(returned)) => {
                    event = returned;
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(TrySendError::Disconnected(_)) => return,
            }
        }
    }
}

impl Thread {
    fn run(self) {
        let Self {
            config,
            map,
            control,
            cancel,
            events,
            requests,
            wake,
        } = self;
        let link = Arc::new(Link {
            events,
            cancel: Arc::clone(&cancel),
            wake,
            user_stopped: Arc::new(AtomicBool::new(false)),
        });
        let watches: Vec<(String, Result<DebugExpression, String>)> = config
            .watches
            .iter()
            .map(|text| (text.clone(), expression::parse(text).map_err(|e| e.message)))
            .collect();
        let debugger = ChannelDebugger {
            link: Arc::clone(&link),
            requests,
            watches,
        };
        let runtime = DebugRuntime::new(
            Arc::clone(&map),
            Box::new(debugger),
            control.clone(),
            DebugOptions {
                stop_on_entry: config.stop_on_entry,
                breakpoints: config.breakpoints.clone(),
            },
        );
        let built = ExecutionController::new(
            &config.source,
            config.inputs.clone(),
            config.symbol.clone(),
            config.visible_range,
            config.limits,
            config.libraries.as_ref(),
            config.security_data,
            Some(Arc::clone(&cancel)),
        );
        let mut controller = match built {
            Ok(controller) => controller.with_logs().with_debugger(runtime),
            Err(error) => {
                link.emit_final(DebugEvent::Finished {
                    outcome: Outcome::Failed(error),
                    hits: Vec::new(),
                });
                return;
            }
        };
        if let Some(theme) = config.theme {
            controller = controller.with_theme(theme);
        }
        if let Some(seconds) = config.bar_seconds {
            controller = controller.with_bar_seconds(seconds);
        }
        if !link.emit(DebugEvent::Started) {
            return;
        }
        let bars = config.bars;
        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            controller.update(&bars, config.visible_range, config.now_ms)
        }));
        let outcome = match run {
            Ok(update) => {
                if !update.logs.events.is_empty() || update.logs.reset || update.logs.dropped > 0 {
                    link.emit_final(DebugEvent::Logs(update.logs));
                }
                match update.error {
                    None if link.user_stopped.load(Ordering::Acquire) => Outcome::Stopped,
                    None => Outcome::Completed,
                    Some(error) if error.code == "PINE8008" => Outcome::Stopped,
                    Some(error) => Outcome::Failed(error),
                }
            }
            Err(_) => Outcome::Failed(Diagnostic::error(
                "PINE9999",
                crate::pine::DiagnosticCategory::Runtime,
                "The debugger stopped unexpectedly.",
                crate::pine::SourceRange::ZERO,
            )),
        };
        // Dropping the controller drops the runtime, which publishes the final hit counts.
        drop(controller);
        link.emit_final(DebugEvent::Finished {
            outcome,
            hits: control.hit_counts(),
        });
    }
}

struct ChannelDebugger {
    link: Arc<Link>,
    requests: Receiver<Request>,
    watches: Vec<(String, Result<DebugExpression, String>)>,
}

impl ChannelDebugger {
    fn watch_rows(&self, view: &mut StopView<'_>, frame: usize) -> Vec<WatchRow> {
        self.watches
            .iter()
            .map(|(text, parsed)| WatchRow {
                text: text.clone(),
                result: match parsed {
                    Ok(expression) => view.watch(frame, expression),
                    Err(message) => Watched::Error(message.clone()),
                },
            })
            .collect()
    }

    fn set_watches(&mut self, texts: Vec<String>) {
        self.watches = texts
            .into_iter()
            .map(|text| {
                let parsed = expression::parse(&text).map_err(|e| e.message);
                (text, parsed)
            })
            .collect();
    }
}

impl Debugger for ChannelDebugger {
    fn stopped(&mut self, stop: &Stop, view: &mut StopView<'_>) -> Resume {
        // Requests left over from while the script ran are stale, except a changed watch list.
        while let Ok(request) = self.requests.try_recv() {
            if let Request::SetWatches(texts) = request {
                self.set_watches(texts);
            }
        }
        let stack = view.stack();
        let top = stack.first().map_or(0, |frame| frame.index);
        let variables = view.variables(top);
        let watches = self.watch_rows(view, top);
        let logs = view.take_logs();
        if (!logs.events.is_empty() || logs.reset || logs.dropped > 0)
            && !self.link.emit(DebugEvent::Logs(logs))
        {
            return Resume::Stop;
        }
        let snapshot = PausedSnapshot {
            stop: stop.clone(),
            stack,
            bar: view.bar(),
            frame: top,
            variables,
            watches,
            hits: view.hit_counts().to_vec(),
        };
        if !self.link.emit(DebugEvent::Paused(Box::new(snapshot))) {
            return Resume::Stop;
        }
        loop {
            if self.link.cancel.load(Ordering::Acquire) {
                self.link.user_stopped.store(true, Ordering::Release);
                return Resume::Stop;
            }
            let request = match self.requests.recv_timeout(POLL) {
                Ok(request) => request,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    self.link.user_stopped.store(true, Ordering::Release);
                    return Resume::Stop;
                }
            };
            let current = stop.generation;
            let reply = match request {
                Request::Resume { generation, resume } => {
                    if generation == current {
                        if resume == Resume::Stop {
                            self.link.user_stopped.store(true, Ordering::Release);
                        }
                        return resume;
                    }
                    continue;
                }
                Request::SetWatches(texts) => {
                    self.set_watches(texts);
                    continue;
                }
                Request::Variables {
                    generation,
                    token,
                    frame,
                } => {
                    if generation == current {
                        DebugEvent::Variables {
                            token,
                            frame,
                            groups: view.variables(frame),
                        }
                    } else {
                        DebugEvent::Stale { token }
                    }
                }
                Request::Expand {
                    generation,
                    token,
                    reference,
                    start,
                    limit,
                } => {
                    if generation == current {
                        DebugEvent::Expanded {
                            token,
                            result: view.expand(&reference, start, limit),
                        }
                    } else {
                        DebugEvent::Stale { token }
                    }
                }
                Request::Watches {
                    generation,
                    token,
                    frame,
                } => {
                    if generation == current {
                        DebugEvent::Watches {
                            token,
                            frame,
                            rows: self.watch_rows(view, frame),
                        }
                    } else {
                        DebugEvent::Stale { token }
                    }
                }
                Request::Evaluate {
                    generation,
                    token,
                    frame,
                    text,
                } => {
                    if generation == current {
                        DebugEvent::Evaluated {
                            token,
                            result: match expression::parse(&text) {
                                Ok(expression) => view.watch(frame, &expression),
                                Err(error) => Watched::Error(error.message),
                            },
                        }
                    } else {
                        DebugEvent::Stale { token }
                    }
                }
            };
            if !self.link.emit(reply) {
                self.link.user_stopped.store(true, Ordering::Release);
                return Resume::Stop;
            }
        }
    }
}
