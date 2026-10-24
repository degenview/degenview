//! The part of the debugger that lives inside a [`Session`] and decides, statement by statement,
//! whether the script stops.
//!
//! A session without a debugger pays one `Option::is_some` test where these hooks are called, and
//! nothing else: the hooks themselves are `#[cold]` functions in `session.rs` that take this value
//! out of the session for the duration of the call, so the debugger gets `&mut Session` without
//! any aliasing.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crate::pine::ast::Statement;
use crate::pine::runtime_types::RuntimeContext;
use crate::pine::runtime_value::RuntimeValue;
use crate::pine::session::Session;
use crate::pine::{Diagnostic, DiagnosticCategory, SourceRange};

use super::commands::Resume;
use super::frames::{Frame, FrameInfo, FrameKind, Location};
use super::source_map::SourceMap;
use super::stepping::StepMode;

/// Why the script stopped.
#[derive(Clone, Debug, PartialEq)]
pub enum StopReason {
    Breakpoint,
    /// A step command finished.
    Step,
    /// The user asked for a pause.
    Pause,
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
}

/// Whoever is debugging: told when the script stops, and answers how it goes on. It runs on the
/// script's thread, blocks the script until it returns, and may look at the stopped script through
/// the view.
pub trait Debugger: Send {
    fn stopped(&mut self, stop: &Stop, view: &mut StopView<'_>) -> Resume;
}

/// A read-only look at a stopped script.
pub struct StopView<'a> {
    pub(crate) session: &'a mut Session,
    pub(crate) frames: &'a [Frame],
}

impl StopView<'_> {
    /// The frames from the innermost call outwards.
    pub fn stack(&self) -> Vec<FrameInfo> {
        self.frames
            .iter()
            .enumerate()
            .rev()
            .map(|(index, frame)| FrameInfo::of(index, frame))
            .collect()
    }
}

/// The handle the person's side keeps: a request that takes effect at the next safepoint.
#[derive(Clone, Debug, Default)]
pub struct DebugControl {
    pause: Arc<AtomicBool>,
}

impl DebugControl {
    pub fn request_pause(&self) {
        self.pause.store(true, Ordering::Release);
    }

    fn take_pause(&self) -> bool {
        self.pause.swap(false, Ordering::AcqRel)
    }

    fn clear_pause(&self) {
        self.pause.store(false, Ordering::Release);
    }
}

#[derive(Clone, Debug, Default)]
pub struct DebugOptions {
    /// Stop at the first statement instead of running to the first breakpoint.
    pub stop_on_entry: bool,
    /// Offsets of statements to stop at, from [`SourceMap::resolve_line`].
    pub breakpoints: Vec<usize>,
}

pub struct DebugRuntime {
    map: Arc<SourceMap>,
    debugger: Box<dyn Debugger>,
    control: DebugControl,
    frames: Vec<Frame>,
    step: StepMode,
    /// Sorted.
    breakpoints: Vec<usize>,
    first_of_bar: bool,
    generation: u64,
    /// The debugger said stop: every later hook unwinds the script.
    terminated: bool,
    /// An error has been shown for the run unwinding now.
    error_reported: bool,
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
        let mut breakpoints = options.breakpoints;
        breakpoints.sort_unstable();
        breakpoints.dedup();
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
            generation: 0,
            terminated: false,
            error_reported: false,
        }
    }

    /// A bar is about to execute: the stack starts over with the script's own frame.
    pub(crate) fn begin_bar(&mut self) {
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
        self.frames.push(Frame {
            kind,
            call_site: Some(call_site),
            location: None,
            shadowed,
        });
    }

    pub(crate) fn leave_function(&mut self) {
        // The script's own frame is never popped: a call is always entered before it is left.
        if self.frames.len() > 1 {
            self.frames.pop();
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
        if let Some(frame) = self.frames.last_mut() {
            frame.location = Some(location);
        }
        let depth = self.frames.len();
        let first_of_bar = std::mem::take(&mut self.first_of_bar);
        let reason = if self.control.take_pause() {
            Some(StopReason::Pause)
        } else if self.breakpoints.binary_search(&offset).is_ok() {
            Some(StopReason::Breakpoint)
        } else if self.step.stops(depth, first_of_bar) {
            Some(StopReason::Step)
        } else {
            None
        };
        match reason {
            Some(reason) => self.stop(session, reason, Some(location)),
            None => Ok(()),
        }
    }

    /// A statement failed. The innermost statement reports the failure to the debugger once, while
    /// the stack and the variables are still those of the failure; the error then goes on unwinding.
    pub(crate) fn failed(
        &mut self,
        session: &mut Session,
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
        let _ = self.stop(session, StopReason::Exception(error.clone()), location);
        self.terminated = true;
    }

    fn stop(
        &mut self,
        session: &mut Session,
        reason: StopReason,
        location: Option<Location>,
    ) -> Result<(), Diagnostic> {
        self.generation += 1;
        let depth = self.frames.len();
        let stop = Stop {
            reason,
            location,
            bar_index: session.working.bar_index,
            generation: self.generation,
            depth,
        };
        let started = Instant::now();
        let resume = {
            let mut view = StopView {
                session,
                frames: &self.frames,
            };
            self.debugger.stopped(&stop, &mut view)
        };
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
