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

#[derive(Clone, Debug, PartialEq)]
pub enum FrameKind {
    /// The script's top level, run once per bar.
    Script,
    /// A user function, by the name it was called with.
    Function(String),
    /// A function of an imported library. Library code is stepped over, never stopped in.
    Library { call_name: String, scope: String },
    /// A `request.security()` expression being evaluated on the requested symbol and timeframe.
    /// Not a call: the interpreter evaluates the expression in place, inside a state of its own, so
    /// this frame marks the boundary between the chart's variables and the expression's.
    Security {
        symbol: String,
        interval_seconds: f64,
    },
}

/// "1m", "4h", "1D": a timeframe as TradingView writes it.
pub fn timeframe_label(seconds: f64) -> String {
    if !seconds.is_finite() || seconds <= 0.0 {
        return "?".to_owned();
    }
    let whole = seconds.round() as u64;
    for (unit, length) in [("W", 604_800), ("D", 86_400), ("h", 3_600), ("m", 60)] {
        if whole >= length && whole.is_multiple_of(length) {
            return format!("{}{unit}", whole / length);
        }
    }
    format!("{whole}s")
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
    /// The statements (by position in the source map) this frame has finished since it began. A name
    /// the interpreter's table still holds from the previous bar is only a value of this bar once
    /// the statement that declares it has run.
    executed: Vec<u64>,
}

impl Frame {
    pub fn new(
        kind: FrameKind,
        call_site: Option<SourceRange>,
        shadowed: Vec<(String, Option<RuntimeValue>)>,
    ) -> Self {
        Self {
            kind,
            call_site,
            location: None,
            shadowed,
            executed: Vec::new(),
        }
    }

    pub fn script() -> Self {
        Self::new(FrameKind::Script, None, Vec::new())
    }

    pub fn is_security(&self) -> bool {
        matches!(self.kind, FrameKind::Security { .. })
    }

    pub fn mark_executed(&mut self, index: usize) {
        let word = index / 64;
        if self.executed.len() <= word {
            self.executed.resize(word + 1, 0);
        }
        self.executed[word] |= 1 << (index % 64);
    }

    pub fn has_executed(&self, index: usize) -> bool {
        self.executed
            .get(index / 64)
            .is_some_and(|word| word & (1 << (index % 64)) != 0)
    }
}

/// A frame as shown to the person debugging.
#[derive(Clone, Debug, PartialEq)]
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
            FrameKind::Security {
                symbol,
                interval_seconds,
            } => format!(
                "request.security()  {symbol} / {}",
                timeframe_label(*interval_seconds)
            ),
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
