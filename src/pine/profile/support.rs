//! Profiles of small scripts for the tests of whatever draws a profile.

use std::sync::Arc;

use crate::pine::bench_tests::{bars, run_script};

use super::{Completeness, ProfileReport, ProfileRuntime, RegionTable, SteppingClock};

/// The profile of `body` (after an `indicator()` line) over `bar_count` bars on a clock that steps
/// `step_ns` on every read, so its times are exact.
pub(crate) fn report(body: &str, bar_count: usize, step_ns: u64) -> ProfileReport {
    let source = format!("//@version=6\nindicator(\"T\")\n{body}");
    let data = bars(bar_count, 60_000);
    let mut session = run_script(&source, &data, false, |session| {
        let table = Arc::new(RegionTable::build(&session.program));
        session.attach_profiler(ProfileRuntime::with_clock(
            table,
            Box::new(SteppingClock::new(step_ns)),
        ));
    });
    session
        .detach_profiler()
        .expect("a recorder")
        .finish(Completeness::Complete)
}
