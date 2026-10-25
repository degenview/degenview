//! The recorder: what the interpreter calls to time regions, and the aggregation it keeps.
//!
//! Regions nest properly: a statement runs the statements and calls inside it before it ends. A
//! stack of open regions gives each region's *exclusive* time directly, because when a region closes
//! its whole elapsed time is added to what its parent has spent in children. Nothing is estimated and
//! nothing is counted twice; the exclusive times of all regions, and the time no region covers, add up
//! to the time of the bars exactly.
//!
//! Only aggregates are kept, in one array sized by the program: nothing is stored per execution, so
//! a long history costs no more memory than a short one.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::pine::fast_hash::FastMap;

use super::clock::{Clock, MonotonicClock, resolution_ns};
use super::regions::RegionTable;
use super::report::{CallSiteStats, Completeness, ProfileReport, RegionStats, Summary};

#[derive(Clone, Copy, Default)]
struct Counters {
    executions: u64,
    inclusive: u64,
    exclusive: u64,
    slowest: u64,
    /// Executions of this region that are open right now: more than one means recursion.
    active: u32,
}

#[derive(Clone, Copy)]
struct CallSite {
    function: u32,
    calls: u64,
    inclusive: u64,
    exclusive: u64,
    active: u32,
}

/// A region that has started and not yet ended.
struct Frame {
    region: u32,
    /// The call site, for a function frame.
    call: Option<usize>,
    start: u64,
    /// Elapsed time of the regions that have ended inside this one.
    children: u64,
    /// The first open execution of its region (recursion adds a region's time only once).
    outermost: bool,
    /// The same, for the call site.
    outermost_call: bool,
}

pub struct ProfileRuntime {
    table: Arc<RegionTable>,
    clock: Box<dyn Clock>,
    counters: Vec<Counters>,
    call_sites: FastMap<(usize, u32), CallSite>,
    stack: Vec<Frame>,
    bar_started: u64,
    bar_frames: u64,
    bars: u64,
    realtime_bars: u64,
    total_ns: u64,
    /// What one measured execution costs to measure, in nanoseconds; zero if not calibrated.
    pair_cost_ns: u64,
    resolution_ns: u64,
    /// Bars finished, readable from another thread while the run goes on.
    progress: Arc<AtomicU64>,
}

impl ProfileRuntime {
    /// A recorder on the operating system's clock, with the cost of measuring calibrated on this
    /// machine now. That takes a few milliseconds.
    pub fn new(table: Arc<RegionTable>) -> Self {
        let mut runtime = Self::with_clock(table, Box::new(MonotonicClock::new()));
        runtime.pair_cost_ns = calibrate_pair_cost();
        runtime.resolution_ns = resolution_ns(&mut MonotonicClock::new());
        runtime
    }

    /// A recorder on `clock`, not calibrated: its overhead is reported as zero.
    pub fn with_clock(table: Arc<RegionTable>, clock: Box<dyn Clock>) -> Self {
        Self {
            counters: vec![Counters::default(); table.len()],
            table,
            clock,
            call_sites: FastMap::default(),
            stack: Vec::with_capacity(64),
            bar_started: 0,
            bar_frames: 0,
            bars: 0,
            realtime_bars: 0,
            total_ns: 0,
            pair_cost_ns: 0,
            resolution_ns: 1,
            progress: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Reports the number of bars finished to `progress`, which another thread can read while the
    /// run goes on.
    pub fn report_progress_to(mut self, progress: Arc<AtomicU64>) -> Self {
        self.progress = progress;
        self
    }

    pub fn table(&self) -> &Arc<RegionTable> {
        &self.table
    }

    // MARK: Bars

    pub fn begin_bar(&mut self) {
        self.stack.clear();
        self.bar_frames = 0;
        self.bar_started = self.clock.now_ns();
    }

    /// Ends the bar. `counted` is false when the interpreter refused the bar before running it. The
    /// returned time is what measuring this bar is estimated to have added to it, which the caller
    /// gives back to the run's deadline.
    pub fn end_bar(&mut self, counted: bool, realtime: bool) -> Duration {
        let now = self.clock.now_ns();
        // A region left open (the run was cut short, or an error unwound past it) ends with the bar.
        while !self.stack.is_empty() {
            self.pop(now);
        }
        if !counted {
            return Duration::ZERO;
        }
        self.total_ns += now.saturating_sub(self.bar_started);
        self.bars += 1;
        self.realtime_bars += u64::from(realtime);
        self.progress.store(self.bars, Ordering::Relaxed);
        Duration::from_nanos(self.bar_frames.saturating_mul(self.pair_cost_ns))
    }

    // MARK: Regions

    /// A statement starts. Returns whether it is a measured region; if so, [`exit`](Self::exit) must
    /// follow when it ends.
    #[inline]
    pub fn enter_statement(&mut self, offset: usize) -> bool {
        let Some(region) = self.table.at_offset(offset) else {
            return false;
        };
        self.push(region, None);
        true
    }

    /// A user-defined function's body starts, called from the call at `call_offset`. `first_statement`
    /// is the offset of the first statement of the body, which names the function.
    #[inline]
    pub fn enter_function(&mut self, first_statement: usize, call_offset: usize) -> bool {
        let Some(region) = self.table.function_with_body(first_statement) else {
            return false;
        };
        self.push(region, Some(call_offset));
        true
    }

    #[inline]
    fn push(&mut self, region: u32, call: Option<usize>) {
        let counters = &mut self.counters[region as usize];
        let outermost = counters.active == 0;
        counters.active += 1;
        let outermost_call = match call {
            Some(offset) => {
                let site = self.call_sites.entry((offset, region)).or_insert(CallSite {
                    function: region,
                    calls: 0,
                    inclusive: 0,
                    exclusive: 0,
                    active: 0,
                });
                let first = site.active == 0;
                site.active += 1;
                first
            }
            None => false,
        };
        self.bar_frames += 1;
        // The clock is read last, so the work above is not charged to the region.
        let start = self.clock.now_ns();
        self.stack.push(Frame {
            region,
            call,
            start,
            children: 0,
            outermost,
            outermost_call,
        });
    }

    /// The region entered last ends.
    #[inline]
    pub fn exit(&mut self) {
        let now = self.clock.now_ns();
        self.pop(now);
    }

    #[inline]
    fn pop(&mut self, now: u64) {
        let Some(frame) = self.stack.pop() else {
            return;
        };
        let elapsed = now.saturating_sub(frame.start);
        let exclusive = elapsed.saturating_sub(frame.children);
        let counters = &mut self.counters[frame.region as usize];
        counters.executions += 1;
        counters.exclusive += exclusive;
        counters.slowest = counters.slowest.max(elapsed);
        counters.active -= 1;
        if frame.outermost {
            counters.inclusive += elapsed;
        }
        if let Some(offset) = frame.call
            && let Some(site) = self.call_sites.get_mut(&(offset, frame.region))
        {
            site.calls += 1;
            site.exclusive += exclusive;
            site.active -= 1;
            if frame.outermost_call {
                site.inclusive += elapsed;
            }
        }
        if let Some(parent) = self.stack.last_mut() {
            parent.children += elapsed;
        }
    }

    // MARK: Result

    /// Ends the recording and hands over what it measured.
    pub fn finish(mut self, completeness: Completeness) -> ProfileReport {
        let now = self.clock.now_ns();
        while !self.stack.is_empty() {
            self.pop(now);
        }
        let stats: Vec<RegionStats> = self
            .counters
            .iter()
            .map(|counters| RegionStats {
                executions: counters.executions,
                inclusive_ns: counters.inclusive,
                exclusive_ns: counters.exclusive,
                slowest_ns: counters.slowest,
            })
            .collect();
        let measured: u64 = stats.iter().map(|stats| stats.executions).sum();
        let attributed: u64 = stats.iter().map(|stats| stats.exclusive_ns).sum();
        let mut call_sites: Vec<CallSiteStats> = self
            .call_sites
            .iter()
            .map(|((offset, _), site)| CallSiteStats {
                offset: *offset,
                function: site.function,
                calls: site.calls,
                inclusive_ns: site.inclusive,
                exclusive_ns: site.exclusive,
            })
            .collect();
        call_sites.sort_by_key(|site| (site.offset, site.function));
        ProfileReport {
            table: self.table,
            stats,
            call_sites,
            summary: Summary {
                bars: self.bars,
                realtime_bars: self.realtime_bars,
                total_ns: self.total_ns,
                attributed_ns: attributed.min(self.total_ns),
                measured_executions: measured,
                overhead_ns: measured.saturating_mul(self.pair_cost_ns),
                pair_cost_ns: self.pair_cost_ns,
                resolution_ns: self.resolution_ns,
            },
            completeness,
        }
    }
}

/// What it costs, in nanoseconds, to measure one execution of a region: the median over several
/// rounds of starting and ending an empty region through the recorder itself.
fn calibrate_pair_cost() -> u64 {
    const ROUNDS: usize = 7;
    const PAIRS: u32 = 20_000;
    let mut runtime = ProfileRuntime::with_clock(
        Arc::new(RegionTable::single()),
        Box::new(MonotonicClock::new()),
    );
    runtime.begin_bar();
    let mut rounds = [0u64; ROUNDS];
    for round in &mut rounds {
        let started = Instant::now();
        for _ in 0..PAIRS {
            runtime.enter_statement(0);
            runtime.exit();
        }
        *round = started.elapsed().as_nanos() as u64 / u64::from(PAIRS);
    }
    rounds.sort_unstable();
    rounds[ROUNDS / 2]
}
