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

use crate::pine::runtime::fast_hash::FastMap;

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

/// How a bar ended, for [`ProfileRuntime::end_bar`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BarOutcome {
    /// The interpreter refused the bar before running it, or never started it: nothing ran, so
    /// nothing is counted.
    Skipped,
    /// The bar ran to its end.
    Completed,
    /// The bar started and stopped early (cancelled, out of time, or the script failed in it). What
    /// ran is kept and its time is in the total, but it is not a finished bar.
    Cut,
}

/// Marks a frame that is not a region: code the profile does not time, kept on the stack so that
/// every start has an end to match.
const UNTIMED: u32 = u32::MAX;

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
    /// A bar has begun and not ended: a panic that unwinds past the interpreter leaves it so.
    bar_open: bool,
    /// Statement and call frames started over the whole run; the difference to the values at the
    /// start of a bar is that bar's, which the deadline is given back.
    statement_frames: u64,
    call_frames: u64,
    bar_statement_base: u64,
    bar_call_base: u64,
    bars: u64,
    realtime_bars: u64,
    /// The last counted bar was cut short.
    cut_bar: bool,
    total_ns: u64,
    /// What one measured statement costs to measure, in nanoseconds; zero if not calibrated.
    statement_cost_ns: u64,
    /// The same for a user-function call, which also finds its call site.
    call_cost_ns: u64,
    resolution_ns: u64,
    /// Bars finished, readable from another thread while the run goes on.
    progress: Arc<AtomicU64>,
    /// Nanoseconds the run has spent waiting for data it asked for, which no region is charged: the
    /// clock the recorder reads is the real one less this.
    waited: Arc<AtomicU64>,
    last_read: u64,
}

impl ProfileRuntime {
    /// A recorder on the operating system's clock, with the cost of measuring calibrated on this
    /// machine now. That takes a few milliseconds.
    pub fn new(table: Arc<RegionTable>) -> Self {
        let mut runtime = Self::with_clock(table, Box::new(MonotonicClock::new()));
        runtime.statement_cost_ns = calibrate(RegionTable::single(), |runtime| {
            runtime.enter_statement(0);
        });
        runtime.call_cost_ns = calibrate(RegionTable::single_function(), |runtime| {
            runtime.enter_function(0, 1);
        });
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
            bar_open: false,
            statement_frames: 0,
            call_frames: 0,
            bar_statement_base: 0,
            bar_call_base: 0,
            bars: 0,
            realtime_bars: 0,
            cut_bar: false,
            total_ns: 0,
            statement_cost_ns: 0,
            call_cost_ns: 0,
            resolution_ns: 1,
            progress: Arc::new(AtomicU64::new(0)),
            waited: Arc::new(AtomicU64::new(0)),
            last_read: 0,
        }
    }

    /// Takes the time the run spends waiting for data out of every time it measures. The counter is
    /// added to by whoever fetches (see [`WaitMeter`](super::WaitMeter)), on the thread that runs
    /// the script.
    pub fn exclude_waiting(mut self, waited: Arc<AtomicU64>) -> Self {
        self.waited = waited;
        self
    }

    /// The clock, less the time spent waiting for data, never going backwards.
    #[inline]
    fn now(&mut self) -> u64 {
        let raw = self.clock.now_ns();
        let now = raw
            .saturating_sub(self.waited.load(Ordering::Relaxed))
            .max(self.last_read);
        self.last_read = now;
        now
    }

    /// Reports the number of bars finished to `progress`, which another thread can read while the
    /// run goes on.
    pub fn report_progress_to(mut self, progress: Arc<AtomicU64>) -> Self {
        self.progress = progress;
        self
    }

    // MARK: Bars

    pub fn begin_bar(&mut self) {
        self.stack.clear();
        self.bar_statement_base = self.statement_frames;
        self.bar_call_base = self.call_frames;
        self.bar_open = true;
        self.bar_started = self.now();
    }

    /// Ends the bar. The returned time is what measuring this bar is estimated to have added to it,
    /// which the caller gives back to the run's deadline.
    pub fn end_bar(&mut self, outcome: BarOutcome, realtime: bool) -> Duration {
        let now = self.now();
        // A region left open (the run was cut short, or an error unwound past it) ends with the bar.
        while !self.stack.is_empty() {
            self.pop(now);
        }
        self.bar_open = false;
        if outcome == BarOutcome::Skipped {
            return Duration::ZERO;
        }
        self.total_ns += now.saturating_sub(self.bar_started);
        self.bars += 1;
        self.realtime_bars += u64::from(realtime);
        if outcome == BarOutcome::Completed {
            self.progress.store(self.bars, Ordering::Relaxed);
        } else {
            self.cut_bar = true;
        }
        let statements = self.statement_frames - self.bar_statement_base;
        let calls = self.call_frames - self.bar_call_base;
        Duration::from_nanos(
            statements
                .saturating_mul(self.statement_cost_ns)
                .saturating_add(calls.saturating_mul(self.call_cost_ns)),
        )
    }

    // MARK: Regions

    /// A statement starts. Returns whether it is a measured region. Either way a frame is open until
    /// [`exit`](Self::exit): every start has one end.
    #[inline]
    pub fn enter_statement(&mut self, offset: usize) -> bool {
        match self.table.at_offset(offset) {
            Some(region) => {
                self.push(region, None);
                true
            }
            None => {
                self.enter_untimed();
                false
            }
        }
    }

    /// A user-defined function's body starts, called from the call at `call_offset`. `first_statement`
    /// is the offset of the first statement of the body, which names the function. Returns whether
    /// it is a measured region; either way a frame is open until [`exit`](Self::exit).
    #[inline]
    pub fn enter_function(&mut self, first_statement: usize, call_offset: usize) -> bool {
        match self.table.function_with_body(first_statement) {
            Some(region) => {
                self.push(region, Some(call_offset));
                true
            }
            None => {
                self.enter_untimed();
                false
            }
        }
    }

    /// Something starts that the profile does not time (library code, a function's declaration). Its
    /// end is matched like any other, and whatever ran inside it is charged to what is around it.
    #[inline]
    pub fn enter_untimed(&mut self) {
        self.stack.push(Frame {
            region: UNTIMED,
            call: None,
            start: 0,
            children: 0,
            outermost: false,
            outermost_call: false,
        });
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
        if call.is_some() {
            self.call_frames += 1;
        } else {
            self.statement_frames += 1;
        }
        // The clock is read last, so the work above is not charged to the region.
        let start = self.now();
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
        // An untimed frame has no end to read.
        let now = match self.stack.last() {
            Some(frame) if frame.region == UNTIMED => 0,
            _ => self.now(),
        };
        self.pop(now);
    }

    #[inline]
    fn pop(&mut self, now: u64) {
        let Some(frame) = self.stack.pop() else {
            return;
        };
        if frame.region == UNTIMED {
            // What ran inside it was timed on its own; the region around it spent that time in
            // children all the same.
            if let Some(parent) = self.stack.last_mut() {
                parent.children += frame.children;
            }
            return;
        }
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
        let now = self.now();
        while !self.stack.is_empty() {
            self.pop(now);
        }
        if self.bar_open {
            // A bar that never ended: the interpreter unwound past it (a panic). Its time belongs to
            // the total, or its regions would add up to more than the run took. It is not a
            // finished bar, so it is not counted among the bars.
            self.total_ns += now.saturating_sub(self.bar_started);
            self.cut_bar = true;
            self.bar_open = false;
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
        debug_assert!(
            attributed <= self.total_ns,
            "regions took {attributed} ns of a total of {}",
            self.total_ns
        );
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
                cut_bar: self.cut_bar,
                attributed_ns: attributed,
                measured_executions: measured,
                overhead_ns: self
                    .statement_frames
                    .saturating_mul(self.statement_cost_ns)
                    .saturating_add(self.call_frames.saturating_mul(self.call_cost_ns)),
                waited_ns: self.waited.load(Ordering::Relaxed),
                pair_cost_ns: self.statement_cost_ns,
                call_pair_cost_ns: self.call_cost_ns,
                resolution_ns: self.resolution_ns,
            },
            completeness,
        }
    }
}

/// What it costs, in nanoseconds, to measure one execution of the kind of region `table` holds: the
/// median over several rounds of starting (`enter`) and ending an empty region through the recorder
/// itself.
fn calibrate(table: RegionTable, enter: impl Fn(&mut ProfileRuntime)) -> u64 {
    const ROUNDS: usize = 7;
    const PAIRS: u32 = 10_000;
    let mut runtime = ProfileRuntime::with_clock(Arc::new(table), Box::new(MonotonicClock::new()));
    runtime.begin_bar();
    let mut rounds = [0u64; ROUNDS];
    for round in &mut rounds {
        let started = Instant::now();
        for _ in 0..PAIRS {
            enter(&mut runtime);
            runtime.exit();
        }
        *round = started.elapsed().as_nanos() as u64 / u64::from(PAIRS);
    }
    rounds.sort_unstable();
    rounds[ROUNDS / 2]
}
