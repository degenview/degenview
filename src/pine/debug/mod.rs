//! A source-level debugger for Pine scripts. See `docs/pine-debugger.md`.
//!
//! The interpreter stays the one interpreter: a [`Session`](super::session::Session) may carry a
//! [`DebugRuntime`], which it consults before each statement, around each user function call and at
//! the start of each bar. Without one, none of this runs. Everything here is pure engine code with
//! no UI types.

mod breakpoints;
mod commands;
mod evaluate;
mod expression;
mod frames;
mod inspect;
mod runtime;
mod source_map;
mod state;
mod stepping;

#[cfg(test)]
mod breakpoint_tests;
#[cfg(test)]
mod inspection_tests;
#[cfg(test)]
mod tests;

pub use breakpoints::{
    BreakpointSpec, BreakpointState, BreakpointStatus, CompiledBreakpoint, HitRule, Template,
    compile as compile_breakpoints,
};
pub use commands::{DebugCommand, Resume};
pub use expression::{DebugExpression, ExpressionError, parse as parse_expression};
pub use frames::{FrameInfo, FrameKind, Location};
pub use inspect::{
    Group, InspectError, PAGE_LIMIT, Page, ValueRef, Variable, VariableGroup, Watched,
};
pub use runtime::{
    BarContext, DebugControl, DebugOptions, DebugRuntime, Debugger, Stop, StopReason, StopView,
};
pub use source_map::{
    DeclarationKind, Executable, LineResolution, NotExecutable, ScopeEntry, SourceMap,
};
pub use state::{DebugState, InvalidTransition, Trigger};
pub use stepping::StepMode;
