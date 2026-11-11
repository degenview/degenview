//! What the code editor shows of a debugging session, and what it reports back from its gutter.
//!
//! The editor knows nothing of sessions. Each frame it is handed the marks to draw (breakpoint
//! markers, the line the script is stopped on, the values to show when hovering a name) and it
//! returns the clicks the person made in the gutter. Lines are one-based, as in the source.

use std::collections::HashMap;

/// A breakpoint marker in the gutter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BreakpointMark {
    pub line: usize,
    pub kind: MarkKind,
    pub enabled: bool,
    /// The line holds no code the script runs, or the script does not compile yet: the marker is
    /// drawn hollow, and the breakpoint will not stop anything until it resolves.
    pub unverified: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarkKind {
    Line,
    /// Has a condition or a hit-count rule.
    Conditional,
    /// Writes a message and does not stop.
    Logpoint,
}

/// The line a stopped script is at in the frame being looked at.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StopMark {
    pub line: usize,
    /// The innermost frame, as opposed to a caller the person selected in the call stack.
    pub innermost: bool,
    /// The script failed here.
    pub error: bool,
}

#[derive(Clone, Debug, Default)]
pub struct DebugMarks {
    pub breakpoints: Vec<BreakpointMark>,
    pub stop: Option<StopMark>,
    /// Name → (value, type), for the variables visible where the script is stopped. Shown in a
    /// tooltip over the name and at the end of the stopped line.
    pub values: HashMap<String, (String, String)>,
}

impl DebugMarks {
    pub fn breakpoint_at(&self, line: usize) -> Option<&BreakpointMark> {
        self.breakpoints.iter().find(|mark| mark.line == line)
    }
}

/// What the person did in the gutter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GutterEvent {
    /// A click on a line: add or remove its breakpoint.
    Toggle { line: usize },
    /// Its menu: switch the breakpoint on a line on or off.
    SetEnabled { line: usize, enabled: bool },
    /// Its menu: edit the breakpoint on a line (adding one if there is none).
    Edit { line: usize },
    /// Its menu: add a breakpoint that writes a message and does not stop.
    AddLogpoint { line: usize },
}
