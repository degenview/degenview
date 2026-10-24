//! The debugger's own call stack.
//!
//! The interpreter keeps no frame stack: a user function call is a Rust call and `depth` is only a
//! counter, with the caller's values parked in a local of `Session::invoke`. While a debugger is
//! attached, `invoke` reports each call here, so a stopped script can show who called whom and what
//! each caller's variables were.

use crate::pine::SourceRange;
use crate::pine::runtime_value::RuntimeValue;

/// Where a frame is, as the editor wants it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Location {
    /// One-based.
    pub line: usize,
    /// One-based.
    pub column: usize,
    /// UTF-16 offset in the compiled (CRLF-folded) source.
    pub offset: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FrameKind {
    /// The script's top level, run once per bar.
    Script,
    /// A user function, by the name it was called with.
    Function(String),
    /// A function of an imported library. Library code is stepped over, never stopped in.
    Library { call_name: String, scope: String },
}

#[derive(Clone, Debug)]
pub(crate) struct Frame {
    pub kind: FrameKind,
    /// The call expression in the caller that made this frame; `None` for the script.
    pub call_site: Option<SourceRange>,
    /// The statement this frame is about to run or is in the middle of. For a frame that has made a
    /// call, the statement that holds the call.
    pub location: Option<Location>,
    /// The caller's values of the names this function declares, taken off the variable table for the
    /// duration of the call. They are what the caller's frame sees.
    pub shadowed: Vec<(String, Option<RuntimeValue>)>,
}

impl Frame {
    pub fn script() -> Self {
        Self {
            kind: FrameKind::Script,
            call_site: None,
            location: None,
            shadowed: Vec::new(),
        }
    }
}

/// A frame as shown to the person debugging.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameInfo {
    /// Position from the bottom of the stack: 0 is the script.
    pub index: usize,
    pub kind: FrameKind,
    /// "Main Script", or "calculate()".
    pub label: String,
    pub location: Option<Location>,
    pub call_site: Option<SourceRange>,
}

impl FrameInfo {
    pub(crate) fn of(index: usize, frame: &Frame) -> Self {
        let label = match &frame.kind {
            FrameKind::Script => "Main Script".to_owned(),
            FrameKind::Function(name) => format!("{name}()"),
            FrameKind::Library { call_name, .. } => format!("{call_name}() (library)"),
        };
        Self {
            index,
            kind: frame.kind.clone(),
            label,
            location: frame.location,
            call_site: frame.call_site,
        }
    }
}
