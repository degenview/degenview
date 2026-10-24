//! The part of the debugger that lives inside a [`Session`] and decides, statement by statement,
//! whether the script stops.
//!
//! A session without a debugger pays one `Option::is_some` test where these hooks are called, and
//! nothing else: the hooks themselves are `#[cold]` functions in `session.rs` that take this value
//! out of the session for the duration of the call, so the debugger gets `&mut Session` without
//! any aliasing.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::pine::ast::Statement;
use crate::pine::fast_hash::FastMap;
use crate::pine::logs::LogLevel;
use crate::pine::runtime_types::RuntimeContext;
use crate::pine::runtime_value::RuntimeValue;
use crate::pine::session::Session;
use crate::pine::{Diagnostic, DiagnosticCategory, SourceRange};

use super::breakpoints::{self, BreakpointSpec, CompiledBreakpoint};
use super::commands::Resume;
use super::evaluate;
use super::expression::DebugExpression;
use super::frames::{Frame, FrameInfo, FrameKind, Location};
use super::inspect::{self, Env, InspectError, Page, ValueRef, VariableGroup, Watched};
use super::source_map::SourceMap;
use super::stepping::StepMode;

/// Why the script stopped.
#[derive(Clone, Debug, PartialEq)]
pub enum StopReason {
    /// The breakpoint with this id.
    Breakpoint(u64),
    /// A step command finished.
    Step,
    /// The user asked for a pause.
    Pause,
    /// The bar the user asked to stop at has started.
    Bar,
    /// The script failed. The stop is read-only: it cannot be continued past.
    Exception(Diagnostic),
}

/// A stopped script, as told to the [`Debugger`].
#[derive(Clone, Debug, PartialEq)]
pub struct Stop {
    pub reason: StopReason,
    /// The statement about to run; for an exception, the statement that failed.
    pub location: Option<Location>,
    /// The chart bar index the script is executing.
    pub bar_index: i64,
    /// Increases at every stop. Handles to the stopped script's values carry it and stop working
    /// once the script resumes.
    pub generation: u64,
    /// Number of frames on the debugger's stack.
    pub depth: usize,
    /// Something to tell the person: a breakpoint condition that could not be evaluated.
    pub notice: Option<String>,
}

/// Whoever is debugging: told when the script stops, and answers how it goes on. It runs on the
/// script's thread, blocks the script until it returns, and may look at the stopped script through
/// the view.
pub trait Debugger: Send {
    fn stopped(&mut self, stop: &Stop, view: &mut StopView<'_>) -> Resume;
}

/// Where the script is, as the bar context panel shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct BarContext {
    pub bar_index: i64,
    pub time_ms: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub symbol: String,
    pub bar_seconds: f64,
    /// Replaying the chart's history, as opposed to a live bar.
    pub historical: bool,
    pub confirmed: bool,
    /// Set while a `request.security()` expression runs: the bar fields above are then the
    /// requested symbol's bar, and `bar_index` counts that series.
    pub request: Option<RequestContext>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RequestContext {
    pub symbol: String,
    pub interval_seconds: f64,
    /// The chart bar the request was made on.
    pub chart_bar_index: i64,
}

/// Where to stop next, regardless of which statement is running.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BarBreak {
    /// Stop at the start of the bar with this index.
    Index(i64),
    /// Stop at the start of the first bar that opens at or after this time (ms since the epoch).
    Time(i64),
}

impl BarBreak {
    fn reached(self, bar_index: i64, open_time: i64) -> bool {
        match self {
            Self::Index(index) => bar_index == index,
            Self::Time(time) => open_time >= time,
        }
    }
}

/// A look at a stopped script. Everything it offers leaves the script as it was.
pub struct StopView<'a> {
    session: &'a mut Session,
    frames: &'a [Frame],
    context: &'a RuntimeContext,
    map: &'a SourceMap,
    generation: u64,
    previous: &'a FastMap<String, String>,
    current: &'a mut FastMap<String, String>,
    hits: Vec<(u64, u64)>,
    chart_bar_index: i64,
}

impl StopView<'_> {
    fn env(&mut self) -> Env<'_> {
        Env {
            session: self.session,
            frames: self.frames,
            context: self.context,
            map: self.map,
            generation: self.generation,
        }
    }

    /// The frames from the innermost call outwards.
    pub fn stack(&self) -> Vec<FrameInfo> {
        self.frames
            .iter()
            .enumerate()
            .rev()
            .map(|(index, frame)| FrameInfo::of(index, frame))
            .collect()
    }

    /// The variables in scope in `frame` (an index from [`FrameInfo::index`]).
    pub fn variables(&mut self, frame: usize) -> Vec<VariableGroup> {
        let previous = self.previous;
        let groups = inspect::variables(&mut self.env(), frame, previous);
        if frame + 1 == self.frames.len() {
            for variable in groups.iter().flat_map(|group| &group.variables) {
                if variable.assigned {
                    self.current
                        .insert(variable.name.clone(), variable.full.clone());
                }
            }
        }
        groups
    }

    /// Rows `start..` of what `reference` holds.
    pub fn expand(
        &mut self,
        reference: &ValueRef,
        start: usize,
        limit: usize,
    ) -> Result<Page, InspectError> {
        inspect::expand(&mut self.env(), reference, start, limit)
    }

    /// Evaluates a checked expression in `frame`.
    pub fn watch(&mut self, frame: usize, expression: &DebugExpression) -> Watched {
        inspect::watch(&mut self.env(), frame, expression)
    }

    pub fn bar(&self) -> BarContext {
        let bar = &self.context.bar;
        let request = self
            .frames
            .iter()
            .rev()
            .find_map(|frame| match &frame.kind {
                FrameKind::Security {
                    symbol,
                    interval_seconds,
                } => Some(RequestContext {
                    symbol: symbol.clone(),
                    interval_seconds: *interval_seconds,
                    chart_bar_index: self.chart_bar_index,
                }),
                _ => None,
            });
        BarContext {
            request,
            bar_index: self.session.working.bar_index,
            time_ms: bar.open_time,
            open: bar.open,
            high: bar.high,
            low: bar.low,
            close: bar.close,
            volume: bar.volume,
            symbol: self.session.symbol.ticker_id.clone(),
            bar_seconds: self.session.bar_seconds,
            historical: !self.context.flags.is_realtime,
            confirmed: self.context.flags.is_confirmed,
        }
    }

    /// The `log.*()` calls and logpoints written since the last time this was asked, in order. A
    /// reader that applies every batch it is given sees the run's messages without a gap.
    pub fn take_logs(&mut self) -> crate::pine::logs::LogBatch {
        self.session.logs.take()
    }

    /// How many times each breakpoint has acted, by id.
    pub fn hit_counts(&self) -> &[(u64, u64)] {
        &self.hits
    }
}

#[derive(Debug, Default)]
struct Shared {
    pause: AtomicBool,
    breakpoints_changed: AtomicBool,
    breakpoints: Mutex<Vec<BreakpointSpec>>,
    bar_break_changed: AtomicBool,
    bar_break: Mutex<Option<BarBreak>>,
    hits: Mutex<Vec<(u64, u64)>>,
}

/// The handle the person's side keeps: requests that take effect at the next safepoint, and what
/// the session has counted.
#[derive(Clone, Debug, Default)]
pub struct DebugControl {
    shared: Arc<Shared>,
}

impl DebugControl {
    pub fn request_pause(&self) {
        self.shared.pause.store(true, Ordering::Release);
    }

    /// Replaces the breakpoints of a running session. Hit counts of breakpoints that keep their id
    /// carry over.
    pub fn set_breakpoints(&self, specs: Vec<BreakpointSpec>) {
        if let Ok(mut current) = self.shared.breakpoints.lock() {
            *current = specs;
        }
        self.shared
            .breakpoints_changed
            .store(true, Ordering::Release);
    }

    /// Stops at the start of a bar, whatever statements run in it. One-shot: it clears itself when
    /// it fires. `None` cancels it.
    pub fn set_bar_break(&self, target: Option<BarBreak>) {
        if let Ok(mut current) = self.shared.bar_break.lock() {
            *current = target;
        }
        self.shared.bar_break_changed.store(true, Ordering::Release);
    }

    /// `(breakpoint id, hits)` as of the last stop or the end of the run.
    pub fn hit_counts(&self) -> Vec<(u64, u64)> {
        self.shared
            .hits
            .lock()
            .map(|hits| hits.clone())
            .unwrap_or_default()
    }

    fn take_pause(&self) -> bool {
        self.shared.pause.swap(false, Ordering::AcqRel)
    }

    fn clear_pause(&self) {
        self.shared.pause.store(false, Ordering::Release);
    }

    fn take_breakpoints(&self) -> Option<Vec<BreakpointSpec>> {
        if self
            .shared
            .breakpoints_changed
            .swap(false, Ordering::AcqRel)
        {
            self.shared
                .breakpoints
                .lock()
                .ok()
                .map(|specs| specs.clone())
        } else {
            None
        }
    }

    fn take_bar_break(&self) -> Option<Option<BarBreak>> {
        if self.shared.bar_break_changed.swap(false, Ordering::AcqRel) {
            self.shared.bar_break.lock().ok().map(|target| *target)
        } else {
            None
        }
    }

    fn publish_hits(&self, hits: Vec<(u64, u64)>) {
        if let Ok(mut current) = self.shared.hits.lock() {
            *current = hits;
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct DebugOptions {
    /// Stop at the first statement instead of running to the first breakpoint.
    pub stop_on_entry: bool,
    pub breakpoints: Vec<BreakpointSpec>,
}

pub struct DebugRuntime {
    map: Arc<SourceMap>,
    debugger: Box<dyn Debugger>,
    control: DebugControl,
    frames: Vec<Frame>,
    step: StepMode,
    /// Sorted by statement.
    breakpoints: Vec<CompiledBreakpoint>,
    first_of_bar: bool,
    bar_break: Option<BarBreak>,
    /// The chart's bar index, which the interpreter's own counter stops being while a request runs.
    chart_bar_index: i64,
    generation: u64,
    /// The debugger said stop: every later hook unwinds the script.
    terminated: bool,
    /// An error has been shown for the run unwinding now.
    error_reported: bool,
    /// What the variables read at the last stop that was inspected, and at this one.
    previous: FastMap<String, String>,
    current: FastMap<String, String>,
}

fn cancelled() -> Diagnostic {
    Diagnostic::error(
        "PINE8008",
        DiagnosticCategory::Cancellation,
        "Debugging stopped.",
        SourceRange::ZERO,
    )
}

impl DebugRuntime {
    pub fn new(
        map: Arc<SourceMap>,
        debugger: Box<dyn Debugger>,
        control: DebugControl,
        options: DebugOptions,
    ) -> Self {
        let (breakpoints, _) = breakpoints::compile(&options.breakpoints, &map);
        Self {
            map,
            debugger,
            control,
            frames: Vec::new(),
            step: if options.stop_on_entry {
                StepMode::Into
            } else {
                StepMode::Run
            },
            breakpoints,
            first_of_bar: false,
            bar_break: None,
            chart_bar_index: 0,
            generation: 0,
            terminated: false,
            error_reported: false,
            previous: FastMap::default(),
            current: FastMap::default(),
        }
    }

    fn reload_breakpoints(&mut self, specs: &[BreakpointSpec]) {
        let (mut compiled, _) = breakpoints::compile(specs, &self.map);
        for breakpoint in &mut compiled {
            if let Some(old) = self.breakpoints.iter().find(|old| old.id == breakpoint.id) {
                breakpoint.hits = old.hits;
            }
        }
        self.breakpoints = compiled;
    }

    fn hit_counts(&self) -> Vec<(u64, u64)> {
        self.breakpoints
            .iter()
            .map(|breakpoint| (breakpoint.id, breakpoint.hits))
            .collect()
    }

    /// A bar is about to execute: the stack starts over with the script's own frame.
    pub(crate) fn begin_bar(&mut self, bar_index: i64) {
        self.chart_bar_index = bar_index;
        self.frames.clear();
        self.frames.push(Frame::script());
        self.first_of_bar = true;
        self.error_reported = false;
    }

    pub(crate) fn enter_function(
        &mut self,
        call_name: &str,
        call_site: SourceRange,
        scope: Option<&str>,
        shadowed: Vec<(String, Option<RuntimeValue>)>,
    ) {
        let kind = match scope {
            Some(scope) => FrameKind::Library {
                call_name: call_name.to_owned(),
                scope: scope.to_owned(),
            },
            None => FrameKind::Function(call_name.to_owned()),
        };
        self.frames
            .push(Frame::new(kind, Some(call_site), shadowed));
    }

    pub(crate) fn enter_security(&mut self, symbol: &str, interval_seconds: f64) {
        let mut frame = Frame::new(
            FrameKind::Security {
                symbol: symbol.to_owned(),
                interval_seconds,
            },
            None,
            Vec::new(),
        );
        // The request is made by whatever statement the caller is in.
        frame.location = self.frames.last().and_then(|caller| caller.location);
        self.frames.push(frame);
    }

    pub(crate) fn leave_function(&mut self) {
        // The script's own frame is never popped: a call is always entered before it is left.
        if self.frames.len() > 1 {
            self.frames.pop();
        }
    }

    /// `statement` ran to its end in the innermost frame.
    pub(crate) fn after_statement(&mut self, statement: &Statement, context: &RuntimeContext) {
        if context.scope.is_some() {
            return;
        }
        if let Some(index) = self.map.index_of(statement.range().start.offset)
            && let Some(frame) = self.frames.last_mut()
        {
            frame.mark_executed(index);
        }
    }

    pub(crate) fn before_statement(
        &mut self,
        session: &mut Session,
        statement: &Statement,
        context: &RuntimeContext,
    ) -> Result<(), Diagnostic> {
        if self.terminated {
            return Err(cancelled());
        }
        // Library code is stepped over: its frames exist, its statements are never stops.
        if context.scope.is_some() {
            return Ok(());
        }
        let offset = statement.range().start.offset;
        let Some(location) = self.map.at(offset).map(|executable| Location {
            line: executable.line,
            column: executable.column,
            offset,
        }) else {
            return Ok(());
        };
        if let Some(specs) = self.control.take_breakpoints() {
            self.reload_breakpoints(&specs);
        }
        if let Some(target) = self.control.take_bar_break() {
            self.bar_break = target;
        }
        if let Some(frame) = self.frames.last_mut() {
            frame.location = Some(location);
        }
        let depth = self.frames.len();
        let first_of_bar = std::mem::take(&mut self.first_of_bar);
        let mut notice = None;
        // Breakpoints are always looked at: a logpoint must write and a hit must be counted even on
        // a visit that stops for another reason.
        let mut reason = None;
        let first = self
            .breakpoints
            .partition_point(|breakpoint| breakpoint.offset < offset);
        let mut index = first;
        while index < self.breakpoints.len() && self.breakpoints[index].offset == offset {
            if let Verdict::Stop { id, note } =
                self.check_breakpoint(session, context, statement, index)
                && reason.is_none()
            {
                reason = Some(StopReason::Breakpoint(id));
                notice = note;
            }
            index += 1;
        }
        let pause_requested = self.control.take_pause();
        let bar_reached = first_of_bar
            && self.frames.len() == 1
            && self.bar_break.is_some_and(|target| {
                target.reached(session.working.bar_index, context.bar.open_time)
            });
        if bar_reached {
            self.bar_break = None;
        }
        if reason.is_none() && pause_requested {
            reason = Some(StopReason::Pause);
        }
        if reason.is_none() && bar_reached {
            reason = Some(StopReason::Bar);
        }
        if reason.is_none() && self.step.stops(depth, first_of_bar) {
            reason = Some(StopReason::Step);
        }
        match reason {
            Some(reason) => self.stop(session, context, reason, Some(location), notice),
            None => Ok(()),
        }
    }

    fn check_breakpoint(
        &mut self,
        session: &mut Session,
        context: &RuntimeContext,
        statement: &Statement,
        index: usize,
    ) -> Verdict {
        let top = self.frames.len().saturating_sub(1);
        let breakpoint = &mut self.breakpoints[index];
        if !breakpoint.enabled {
            return Verdict::Skip;
        }
        let is_logpoint = breakpoint.template.is_some();
        let mut note = None;
        if let Some(condition) = &breakpoint.condition {
            match evaluate::evaluate(session, &self.frames, top, context, condition) {
                Ok(value) => {
                    let holds = match &value {
                        RuntimeValue::Bool(flag) => *flag,
                        RuntimeValue::Na => false,
                        other => other.number().is_some_and(|number| number != 0.0),
                    };
                    if !holds {
                        return Verdict::Skip;
                    }
                }
                Err(error) if is_logpoint => {
                    session.logs.record(
                        LogLevel::Warning,
                        format!("Logpoint condition failed: {error}"),
                        None,
                        statement.range(),
                    );
                    return Verdict::Skip;
                }
                // A condition that cannot be evaluated stops, and says so, rather than letting a
                // typo silently turn the breakpoint off.
                Err(error) => note = Some(format!("The condition could not be evaluated: {error}")),
            }
        }
        if note.is_none() {
            breakpoint.hits += 1;
            if !breakpoint.hit.accepts(breakpoint.hits) {
                return Verdict::Skip;
            }
        }
        if let Some(template) = &breakpoint.template {
            let env = Env {
                session,
                frames: &self.frames,
                context,
                map: &self.map,
                generation: self.generation,
            };
            let text = template.render(|expression| {
                evaluate::evaluate(env.session, env.frames, top, env.context, expression)
                    .map(|value| inspect::plain_text(&env, &value))
            });
            session
                .logs
                .record(LogLevel::Info, text, None, statement.range());
            return Verdict::Skip;
        }
        Verdict::Stop {
            id: breakpoint.id,
            note,
        }
    }

    /// A statement failed. The innermost statement reports the failure to the debugger once, while
    /// the stack and the variables are still those of the failure; the error then goes on unwinding.
    pub(crate) fn failed(
        &mut self,
        session: &mut Session,
        context: &RuntimeContext,
        statement: &Statement,
        error: &Diagnostic,
    ) {
        if self.terminated
            || self.error_reported
            || error.category == DiagnosticCategory::Cancellation
        {
            return;
        }
        self.error_reported = true;
        let range = statement.range();
        let location = self
            .map
            .at(range.start.offset)
            .map(|executable| Location {
                line: executable.line,
                column: executable.column,
                offset: executable.offset,
            })
            .or_else(|| self.frames.last().and_then(|frame| frame.location));
        // Whatever the debugger answers, an error cannot be continued past.
        let _ = self.stop(
            session,
            context,
            StopReason::Exception(error.clone()),
            location,
            None,
        );
        self.terminated = true;
    }

    fn stop(
        &mut self,
        session: &mut Session,
        context: &RuntimeContext,
        reason: StopReason,
        location: Option<Location>,
        notice: Option<String>,
    ) -> Result<(), Diagnostic> {
        self.generation += 1;
        let depth = self.frames.len();
        let hits = self.hit_counts();
        self.control.publish_hits(hits.clone());
        let stop = Stop {
            reason,
            location,
            bar_index: session.working.bar_index,
            generation: self.generation,
            depth,
            notice,
        };
        let started = Instant::now();
        self.current.clear();
        let resume = {
            let mut view = StopView {
                session,
                frames: &self.frames,
                context,
                map: &self.map,
                generation: self.generation,
                previous: &self.previous,
                current: &mut self.current,
                hits,
                chart_bar_index: self.chart_bar_index,
            };
            self.debugger.stopped(&stop, &mut view)
        };
        if !self.current.is_empty() {
            self.previous = std::mem::take(&mut self.current);
        }
        // Time spent waiting for a person is not the script's: the deadline does not run while paused.
        if let Some(deadline) = session.deadline_at.as_mut() {
            *deadline += started.elapsed();
        }
        self.control.clear_pause();
        match StepMode::after(resume, depth) {
            Some(mode) => {
                self.step = mode;
                Ok(())
            }
            None => {
                self.terminated = true;
                Err(cancelled())
            }
        }
    }
}

impl Drop for DebugRuntime {
    fn drop(&mut self) {
        self.control.publish_hits(self.hit_counts());
    }
}

enum Verdict {
    Skip,
    Stop { id: u64, note: Option<String> },
}
