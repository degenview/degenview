//! A source-level debugger for Pine scripts. See `docs/pine-debugger.md`.
//!
//! The interpreter stays the one interpreter: a [`Session`](super::session::Session) may carry a
//! [`DebugRuntime`], which it consults before each statement, around each user function call and at
//! the start of each bar. Without one, none of this runs. Everything here is pure engine code with
//! no UI types.

mod commands;
mod frames;
mod runtime;
mod source_map;
mod state;
mod stepping;

#[cfg(test)]
mod tests;

pub use commands::{DebugCommand, Resume};
pub use frames::{FrameInfo, FrameKind, Location};
pub use runtime::{DebugControl, DebugOptions, DebugRuntime, Debugger, Stop, StopReason, StopView};
pub use source_map::{Executable, LineResolution, NotExecutable, SourceMap};
pub use state::{DebugState, InvalidTransition, Trigger};
pub use stepping::StepMode;
