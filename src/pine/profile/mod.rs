//! A profiler for Pine scripts. See `docs/pine-profiler.md`.
//!
//! The interpreter stays the one interpreter: a [`Session`](super::session::Session) may carry a
//! [`ProfileRuntime`], which it calls at the start and end of each statement, each user function
//! call and each bar. Without one, none of this runs. Everything here is pure engine code with no
//! UI types.

mod clock;
mod recorder;
mod regions;
mod report;
mod session;

#[cfg(test)]
mod tests;

pub use clock::{Clock, MonotonicClock, SteppingClock, resolution_ns};
pub use recorder::ProfileRuntime;
pub use regions::{Region, RegionKind, RegionTable};
pub use report::{
    CallSiteStats, Completeness, Interruption, LineStats, ProfileReport, RegionStats, Summary,
};
pub use session::{ProfileConfig, ProfileOutcome, ProfileSession, StartError};
