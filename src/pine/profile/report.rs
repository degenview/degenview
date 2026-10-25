//! What a profile measured, and the questions the editor asks of it.
//!
//! Everything here is derived from the recorder's aggregates; nothing is measured again. A report
//! owns its region table, so it keeps describing the program it was made from however the text
//! changes afterwards.

use std::sync::Arc;

use crate::pine::Diagnostic;

use super::regions::{RegionKind, RegionTable};

/// What one region came to over the whole run.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RegionStats {
    /// Times the region ran.
    pub executions: u64,
    /// Time from start to end of each execution, nested regions included, summed. A recursive
    /// region counts only its outermost executions.
    pub inclusive_ns: u64,
    /// Inclusive time less the inclusive time of the regions that ran inside it.
    pub exclusive_ns: u64,
    /// The longest single execution, nested regions included.
    pub slowest_ns: u64,
}

/// One place a user function was called from, with the time its calls took.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CallSiteStats {
    /// The offset of the call.
    pub offset: usize,
    /// The function's region.
    pub function: u32,
    pub calls: u64,
    pub inclusive_ns: u64,
    pub exclusive_ns: u64,
}

/// Why a profile does not cover the whole run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Interruption {
    /// The person ended it.
    Cancelled,
    /// The script ran out of time.
    TimedOut,
    /// The script failed; its bars up to the failure are in the profile.
    Failed(Diagnostic),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Completeness {
    Complete,
    Partial(Interruption),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Summary {
    /// Bars the script ran on. Each is counted once, however many regions ran in it.
    pub bars: u64,
    /// Of those, bars run as a realtime tick or close.
    pub realtime_bars: u64,
    /// The time of all the bars, from the start of each to its end: the denominator of every share.
    pub total_ns: u64,
    /// The time spent in regions, each moment counted once: the sum of all line times.
    pub attributed_ns: u64,
    /// Executions that were timed (statements and function calls).
    pub measured_executions: u64,
    /// What measuring them is estimated to have added to `total_ns`: the number of timed executions
    /// times the cost of timing one. Reported, never subtracted from anything.
    pub overhead_ns: u64,
    /// The cost of timing one execution on this machine.
    pub pair_cost_ns: u64,
    /// The smallest difference the clock reports.
    pub resolution_ns: u64,
}

impl Summary {
    /// Time inside the bars that no region covers: setting a bar up, committing it, the strategy
    /// broker, the script's declaration.
    pub fn unattributed_ns(&self) -> u64 {
        self.total_ns.saturating_sub(self.attributed_ns)
    }
}

/// The measurements of one line, from the regions that start on it.
#[derive(Clone, Debug, PartialEq)]
pub struct LineStats {
    /// One-based.
    pub line: usize,
    /// Time of the outermost regions starting here, nested regions included.
    pub time_ns: u64,
    /// Time of the line itself: the sum of the line times of every region starting here.
    pub line_time_ns: u64,
    /// Executions of the regions starting here, summed (two statements on a line add up).
    pub executions: u64,
    pub slowest_ns: u64,
    /// The outermost region starting here, which names the line's kind and block.
    pub region: u32,
    pub kind: RegionKind,
    /// The lines the code block that starts here covers, if the line starts one.
    pub block: Option<(usize, usize)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProfileReport {
    pub table: Arc<RegionTable>,
    /// Parallel to the table's regions.
    pub stats: Vec<RegionStats>,
    /// Sorted by offset.
    pub call_sites: Vec<CallSiteStats>,
    pub summary: Summary,
    pub completeness: Completeness,
}

impl ProfileReport {
    pub fn is_complete(&self) -> bool {
        self.completeness == Completeness::Complete
    }

    pub fn stats_of(&self, region: u32) -> RegionStats {
        self.stats.get(region as usize).copied().unwrap_or_default()
    }

    /// `ns` as a fraction of the script's total time. Zero when nothing was timed.
    pub fn share(&self, ns: u64) -> f64 {
        if self.summary.total_ns == 0 {
            0.0
        } else {
            ns as f64 / self.summary.total_ns as f64
        }
    }

    /// How many times a loop's body ran: the executions of the body's first statement, which runs
    /// once per iteration including those that `continue`.
    pub fn iterations(&self, region: u32) -> Option<u64> {
        let region = self.table.get(region)?;
        if region.kind != RegionKind::Loop {
            return None;
        }
        let first = (*region.blocks.first()?)?;
        Some(self.stats_of(first).executions)
    }

    /// For a conditional, how many times each branch ran (`None` for an empty one).
    pub fn branch_executions(&self, region: u32) -> Vec<Option<u64>> {
        let Some(region) = self.table.get(region) else {
            return Vec::new();
        };
        if region.kind != RegionKind::Branch {
            return Vec::new();
        }
        region
            .blocks
            .iter()
            .map(|first| first.map(|first| self.stats_of(first).executions))
            .collect()
    }

    /// The measurements of each line that has a region on it, in line order.
    pub fn lines(&self) -> Vec<LineStats> {
        let regions = self.table.regions();
        let mut lines: Vec<LineStats> = Vec::new();
        let mut order: Vec<usize> = (0..regions.len()).collect();
        // The sort is stable and regions come outermost first, so the first region of a line is its
        // outermost.
        order.sort_by_key(|index| regions[*index].start.line);
        for index in order {
            let region = &regions[index];
            let stats = self.stats[index];
            if stats.executions == 0 {
                continue;
            }
            let line = region.start.line;
            let covered = region
                .parent
                .is_some_and(|parent| regions[parent as usize].start.line == line);
            match lines.last_mut().filter(|last| last.line == line) {
                Some(last) => {
                    last.line_time_ns += stats.exclusive_ns;
                    // A region written inside another on the same line (`x = if …`) is part of that
                    // statement: it adds its own time but is neither another execution nor more time.
                    if !covered {
                        last.time_ns += stats.inclusive_ns;
                        last.executions += stats.executions;
                        last.slowest_ns = last.slowest_ns.max(stats.slowest_ns);
                        if last.block.is_none() && region.is_block() {
                            last.block = Some((region.start.line, region.end.line));
                        }
                    }
                }
                None => lines.push(LineStats {
                    line,
                    time_ns: stats.inclusive_ns,
                    line_time_ns: stats.exclusive_ns,
                    executions: stats.executions,
                    slowest_ns: stats.slowest_ns,
                    region: index as u32,
                    kind: region.kind,
                    block: region
                        .is_block()
                        .then_some((region.start.line, region.end.line)),
                }),
            }
        }
        lines
    }

    /// The `limit` lines that spend the most time themselves, hottest first. A line must have taken
    /// a measurable share to be listed.
    pub fn hottest_lines(&self, limit: usize) -> Vec<LineStats> {
        let mut lines: Vec<LineStats> = self
            .lines()
            .into_iter()
            .filter(|line| line.line_time_ns > 0)
            .collect();
        lines.sort_by(|a, b| {
            b.line_time_ns
                .cmp(&a.line_time_ns)
                .then(a.line.cmp(&b.line))
        });
        lines.truncate(limit);
        lines
    }

    /// The function with the most inclusive time, if any ran.
    pub fn hottest_function(&self) -> Option<(u32, RegionStats)> {
        self.table
            .regions()
            .iter()
            .enumerate()
            .filter(|(_, region)| region.kind == RegionKind::Function)
            .map(|(index, _)| (index as u32, self.stats[index]))
            .filter(|(_, stats)| stats.executions > 0)
            .max_by_key(|(index, stats)| (stats.inclusive_ns, std::cmp::Reverse(*index)))
    }

    /// The code block (a region with regions inside it) with the most inclusive time.
    pub fn hottest_block(&self) -> Option<(u32, RegionStats)> {
        self.table
            .regions()
            .iter()
            .enumerate()
            .filter(|(_, region)| region.is_block())
            .map(|(index, _)| (index as u32, self.stats[index]))
            .filter(|(_, stats)| stats.executions > 0)
            .max_by_key(|(index, stats)| (stats.inclusive_ns, std::cmp::Reverse(*index)))
    }

    /// The call sites of `function`, hottest first.
    pub fn call_sites_of(&self, function: u32) -> Vec<CallSiteStats> {
        let mut sites: Vec<CallSiteStats> = self
            .call_sites
            .iter()
            .filter(|site| site.function == function)
            .copied()
            .collect();
        sites.sort_by_key(|site| std::cmp::Reverse(site.inclusive_ns));
        sites
    }
}
