//! A profiler for Pine scripts. See `docs/pine-profiler.md`.
//!
//! The interpreter stays the one interpreter: a [`Session`](super::runtime::session::Session) may carry a
//! [`ProfileRuntime`], which it calls at the start and end of each statement, each user function
//! call and each bar. Without one, none of this runs. Everything here is pure engine code with no
//! UI types.

mod clock;
mod recorder;
mod regions;
mod report;
mod session;
mod wait;

#[cfg(test)]
pub(crate) mod support;
#[cfg(test)]
mod tests;

#[cfg(test)]
pub use clock::MonotonicClock;
pub use recorder::{BarOutcome, ProfileRuntime};
pub use regions::{RegionKind, RegionTable};
pub use report::{Completeness, Interruption, LineStats, ProfileReport};
pub use session::{ProfileConfig, ProfileOutcome, ProfileSession};
