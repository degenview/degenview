//! The source of time for a profile.
//!
//! Production reads the operating system's monotonic clock; tests install a clock they control, so
//! that the invariants of the timing model are asserted exactly instead of within a tolerance.

use std::time::Instant;

/// A monotonic clock in nanoseconds since an origin of its own.
pub trait Clock: Send {
    fn now_ns(&mut self) -> u64;
}

/// `std::time::Instant`: monotonic, unaffected by changes to the wall clock.
pub struct MonotonicClock {
    origin: Instant,
}

impl MonotonicClock {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for MonotonicClock {
    #[inline]
    fn now_ns(&mut self) -> u64 {
        self.origin.elapsed().as_nanos() as u64
    }
}

/// A clock that advances by a fixed step on every read. With it, the time a region takes is the
/// number of clock reads made inside it times the step, which the nesting of regions determines.
pub struct SteppingClock {
    now: u64,
    step: u64,
}

impl SteppingClock {
    pub fn new(step_ns: u64) -> Self {
        Self {
            now: 0,
            step: step_ns,
        }
    }
}

impl Clock for SteppingClock {
    fn now_ns(&mut self) -> u64 {
        let now = self.now;
        self.now += self.step;
        now
    }
}

/// The smallest step the clock reports: the gap between two reads that differ, over many reads. A
/// time shorter than this is not distinguishable from none.
pub fn resolution_ns(clock: &mut dyn Clock) -> u64 {
    let mut smallest = u64::MAX;
    for _ in 0..4096 {
        let first = clock.now_ns();
        let mut second = clock.now_ns();
        let mut spins = 0;
        while second == first && spins < 10_000 {
            second = clock.now_ns();
            spins += 1;
        }
        if second > first {
            smallest = smallest.min(second - first);
        }
    }
    if smallest == u64::MAX { 1 } else { smallest }
}
