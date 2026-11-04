//! What the editor and the drawer show of a profile, worked out once when the profile arrives.
//!
//! The editor knows nothing of profiles. It is handed a [`ProfileMarks`] (a mark per measured line,
//! with the words of its tooltip already made) and draws it; painting never measures, sorts or
//! formats. Lines are one-based, as in the source.
//!
//! A mark's bar is the line's *time* (itself and everything it runs) on one scale for the whole
//! profile: the longest bar is the line that took the longest, and every label is that line's true
//! share of the script's time. Its heat is the line's *own* time (line time), so a loop's header is
//! long and cool when its body does the work, and the lines that do it are the warm ones.

use std::sync::Arc;

use crate::pine::compiler::normalize_line_endings;
use crate::pine::profile::{Completeness, Interruption, LineStats, ProfileReport, RegionKind};

/// A line shows a mark only when it took at least this share of the script's time. Below it the
/// line stays quiet, though hovering it still names what it did.
pub const QUIET_SHARE: f64 = 0.001;
/// Lines that get a flame: the hottest by line time, if there are enough marked lines to rank.
const FLAMES: usize = 3;
const FLAME_MIN_LINES: usize = 4;
/// A line must have spent this share of the script itself to be called hot.
const FLAME_MIN_SHARE: f64 = 0.01;
const SNIPPET_CHARS: usize = 72;
const CALL_SITES_SHOWN: usize = 4;

/// One line's words, ready to show.
#[derive(Clone, Debug, PartialEq)]
pub struct Tooltip {
    /// "Line 224", or "Code block: lines 221–255".
    pub title: String,
    /// A label and its value, in the order they are shown.
    pub rows: Vec<(&'static str, String)>,
    /// Sentences under the rows: what is not known, or what to do about it.
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LineMark {
    pub line: usize,
    /// The share of the script's time, as text: "22.3%".
    pub label: String,
    /// The bar's length as a fraction of the longest bar, 0 to 1. Never zero for a marked line.
    pub bar: f32,
    /// How much of the script's time the line spent itself relative to the line that spent the most,
    /// 0 to 1.
    pub heat: f32,
    pub flame: bool,
    /// The lines of the code block starting here.
    pub block: Option<(usize, usize)>,
    /// Whether the line is quiet: it took too small a share to draw, but can still be hovered.
    pub quiet: bool,
    pub tooltip: Tooltip,
}

/// A row of the ranked list.
#[derive(Clone, Debug, PartialEq)]
pub struct HotRow {
    pub line: usize,
    pub kind: RegionKind,
    /// The line's source, trimmed.
    pub snippet: String,
    pub time: String,
    pub line_time: String,
    pub executions: String,
    /// Share of the script's time spent in the line itself, 0 to 1, for the row's bar.
    pub line_share: f32,
}

/// Which card of the details a row belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SummaryGroup {
    /// Where the time goes.
    Hotspots,
    /// How it was measured.
    Measurement,
}

impl SummaryGroup {
    pub fn title(self) -> &'static str {
        match self {
            Self::Hotspots => "Where the time goes",
            Self::Measurement => "How it was measured",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SummaryRow {
    pub group: SummaryGroup,
    pub label: &'static str,
    /// What goes under the label, saying what the value is of.
    pub note: Option<String>,
    pub value: String,
    /// The one-based line a click on the row goes to.
    pub line: Option<usize>,
}

/// The three figures at the top of a result.
#[derive(Clone, Debug, PartialEq)]
pub struct Totals {
    pub time: String,
    pub bars: String,
    pub executions: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProfileMarks {
    lines: Vec<LineMark>,
    pub hot: Vec<HotRow>,
    pub totals: Totals,
    /// The rest of the run's figures.
    pub summary: Vec<SummaryRow>,
    /// "Partial: cancelled after 120 of 500 bars", when the profile does not cover the run.
    pub caveat: Option<String>,
}

impl ProfileMarks {
    pub fn build(report: &ProfileReport, source: &str, compile_ns: u64) -> Self {
        let stats = report.lines();
        let total = report.summary.total_ns;
        let longest = stats.iter().map(|line| line.time_ns).max().unwrap_or(0);
        let hottest = stats
            .iter()
            .map(|line| line.line_time_ns)
            .max()
            .unwrap_or(0);
        let marked = stats
            .iter()
            .filter(|line| report.share(line.time_ns) >= QUIET_SHARE)
            .count();
        let flames: Vec<usize> = if marked >= FLAME_MIN_LINES {
            report
                .hottest_lines(FLAMES)
                .into_iter()
                .filter(|line| report.share(line.line_time_ns) >= FLAME_MIN_SHARE)
                .map(|line| line.line)
                .collect()
        } else {
            Vec::new()
        };
        let lines = stats
            .iter()
            .map(|line| {
                let share = report.share(line.time_ns);
                LineMark {
                    line: line.line,
                    label: percent(share),
                    bar: if longest == 0 {
                        0.0
                    } else {
                        (line.time_ns as f64 / longest as f64) as f32
                    },
                    heat: if hottest == 0 {
                        0.0
                    } else {
                        (line.line_time_ns as f64 / hottest as f64) as f32
                    },
                    flame: flames.contains(&line.line),
                    block: line.block,
                    quiet: share < QUIET_SHARE,
                    tooltip: tooltip(report, line, total),
                }
            })
            .collect();
        let text = normalize_line_endings(source);
        let rows = report
            .hottest_lines(10)
            .into_iter()
            .map(|line| HotRow {
                line: line.line,
                kind: line.kind,
                snippet: snippet(&text, line.line),
                time: duration(line.time_ns),
                line_time: duration(line.line_time_ns),
                executions: count(line.executions),
                line_share: report.share(line.line_time_ns) as f32,
            })
            .collect();
        Self {
            lines,
            hot: rows,
            totals: totals(report),
            summary: summary(report, &text, compile_ns),
            caveat: caveat(report),
        }
    }

    /// The mark of a one-based line, if it ran.
    pub fn mark(&self, line: usize) -> Option<&LineMark> {
        self.lines
            .binary_search_by_key(&line, |mark| mark.line)
            .ok()
            .map(|index| &self.lines[index])
    }

    pub fn lines(&self) -> &[LineMark] {
        &self.lines
    }

    /// The marks in the one-based line range `first..=last`, for the lines on screen.
    pub fn in_view(&self, first: usize, last: usize) -> &[LineMark] {
        let start = self.lines.partition_point(|mark| mark.line < first);
        let end = self.lines.partition_point(|mark| mark.line <= last);
        &self.lines[start..end.max(start)]
    }

    /// The flamed lines, hottest first is not promised: they are in line order.
    pub fn flames(&self) -> Vec<usize> {
        self.lines
            .iter()
            .filter(|mark| mark.flame)
            .map(|mark| mark.line)
            .collect()
    }
}

/// What the editor needs to draw the profile column.
#[derive(Clone, Debug)]
pub enum ProfileColumn {
    /// The profile is current: draw its marks.
    Marks(Arc<ProfileMarks>),
    /// The script's text is what was profiled but its inputs or data have changed since: its lines
    /// still match, so the marks stay, faded, until the profile is run again.
    Dated(Arc<ProfileMarks>),
    /// There is nothing to draw yet, or what there is describes other text. The column stays, so
    /// the text does not move, and says why when hovered.
    Empty(&'static str),
}

// MARK: Words

fn tooltip(report: &ProfileReport, line: &LineStats, total: u64) -> Tooltip {
    let table = &report.table;
    let stats = report.stats_of(line.region);
    let title = match line.block {
        Some((first, last)) if last > first => {
            format!("Code block: lines {first}\u{2013}{last}")
        }
        _ => format!("Line {}", line.line),
    };
    let mut rows: Vec<(&'static str, String)> = Vec::new();
    let of_total = |ns: u64| {
        format!(
            "{} ({} of {})",
            percent(report.share(ns)),
            duration(ns),
            duration(total)
        )
    };
    rows.push(("Time", of_total(line.time_ns)));
    if line.line_time_ns != line.time_ns {
        rows.push(("Line time", of_total(line.line_time_ns)));
    }
    rows.push(("Executions", count(line.executions)));
    if line.kind == RegionKind::Loop
        && let Some(iterations) = report.iterations(line.region)
        && stats.executions > 0
    {
        rows.push((
            "Iterations",
            format!(
                "{} (about {} per entry)",
                count(iterations),
                average_count(iterations, stats.executions)
            ),
        ));
    }
    if line.kind == RegionKind::Branch {
        let taken: Vec<String> = report
            .branch_executions(line.region)
            .into_iter()
            .map(|branch| branch.map_or_else(|| "–".to_owned(), count))
            .collect();
        if taken.len() > 1 {
            rows.push(("Branches taken", taken.join(" · ")));
        }
    }
    let mut notes = Vec::new();
    if line.executions > 0 {
        let average = stats_average(line);
        if average < report.summary.resolution_ns {
            notes.push(format!(
                "Each run is shorter than the clock can tell apart ({}), so its share is an estimate.",
                duration(report.summary.resolution_ns)
            ));
        } else {
            rows.push(("Average", format!("{} per execution", duration(average))));
        }
        rows.push(("Slowest", duration(line.slowest_ns)));
    }
    if line.kind == RegionKind::Function {
        let sites = report.call_sites_of(line.region);
        for site in sites.iter().take(CALL_SITES_SHOWN) {
            let at = table
                .enclosing(site.offset)
                .and_then(|index| table.get(index))
                .map_or(0, |region| region.start.line);
            rows.push((
                "Called from",
                format!(
                    "line {at}: {} calls, {}",
                    count(site.calls),
                    duration(site.inclusive_ns)
                ),
            ));
        }
        if sites.len() > CALL_SITES_SHOWN {
            notes.push(format!(
                "and {} more call sites.",
                sites.len() - CALL_SITES_SHOWN
            ));
        }
    }
    Tooltip { title, rows, notes }
}

/// Inclusive time per execution of the line's outermost region, in nanoseconds.
fn stats_average(line: &LineStats) -> u64 {
    line.time_ns.checked_div(line.executions).unwrap_or(0)
}

fn average_count(iterations: u64, entries: u64) -> String {
    let average = iterations as f64 / entries as f64;
    if average >= 10.0 {
        format!("{average:.0}")
    } else {
        format!("{average:.1}")
    }
}

fn summary(report: &ProfileReport, text: &str, compile_ns: u64) -> Vec<SummaryRow> {
    let s = report.summary;
    let row = |group: SummaryGroup,
               label: &'static str,
               note: Option<String>,
               value: String,
               line: Option<usize>| SummaryRow {
        group,
        label,
        note,
        value,
        line,
    };
    let (hot, measured) = (SummaryGroup::Hotspots, SummaryGroup::Measurement);
    let mut rows = Vec::new();
    if let Some(line) = report.hottest_lines(1).first() {
        rows.push(row(
            hot,
            "Hottest line",
            Some(format!("Line {}: {}", line.line, snippet(text, line.line))),
            percent(report.share(line.line_time_ns)),
            Some(line.line),
        ));
    }
    if let Some((index, stats)) = report.hottest_block()
        && let Some(region) = report.table.get(index)
    {
        rows.push(row(
            hot,
            "Hottest block",
            Some(format!(
                "Lines {}\u{2013}{}",
                region.start.line,
                region.end.line.max(region.start.line)
            )),
            percent(report.share(stats.inclusive_ns)),
            Some(region.start.line),
        ));
    }
    if let Some((index, stats)) = report.hottest_function()
        && let Some(region) = report.table.get(index)
    {
        rows.push(row(
            hot,
            "Hottest function",
            Some(format!(
                "{}() \u{b7} {} calls",
                region.name.as_deref().unwrap_or("?"),
                count(stats.executions)
            )),
            percent(report.share(stats.inclusive_ns)),
            Some(region.start.line),
        ));
    }
    rows.push(row(
        measured,
        "Not in any line",
        Some(format!(
            "{} of the total: setting bars up, committing them, orders",
            percent(report.share(s.unattributed_ns()))
        )),
        duration(s.unattributed_ns()),
        None,
    ));
    if s.waited_ns > 0 {
        rows.push(row(
            measured,
            "Waiting for data",
            Some("Fetching other symbols for request.security(); in no time here".to_owned()),
            duration(s.waited_ns),
            None,
        ));
    }
    rows.push(row(
        measured,
        "Profiler overhead",
        Some(format!(
            "About {} per measured execution; inside every time, not subtracted",
            duration(s.pair_cost_ns)
        )),
        format!("\u{2248} {}", duration(s.overhead_ns)),
        None,
    ));
    rows.push(row(
        measured,
        "Clock resolution",
        Some("Times shorter than this are estimates".to_owned()),
        duration(s.resolution_ns),
        None,
    ));
    rows.push(row(
        measured,
        "Compiled in",
        Some("Not part of the total".to_owned()),
        duration(compile_ns),
        None,
    ));
    rows
}

fn totals(report: &ProfileReport) -> Totals {
    let s = report.summary;
    Totals {
        time: duration(s.total_ns),
        bars: count(s.bars),
        executions: count(s.measured_executions),
    }
}

fn caveat(report: &ProfileReport) -> Option<String> {
    let Completeness::Partial(why) = &report.completeness else {
        return None;
    };
    let bars = count(report.summary.bars);
    Some(match why {
        Interruption::Cancelled => format!("Partial: cancelled after {bars} bars."),
        Interruption::TimedOut => format!("Partial: the script ran out of time after {bars} bars."),
        Interruption::Failed(error) => {
            format!(
                "Partial: the script failed after {bars} bars: {}",
                error.message
            )
        }
    })
}

fn snippet(text: &str, line: usize) -> String {
    let source = text.split('\n').nth(line.saturating_sub(1)).unwrap_or("");
    let trimmed = source.trim();
    if trimmed.chars().count() <= SNIPPET_CHARS {
        return trimmed.to_owned();
    }
    let mut cut: String = trimmed.chars().take(SNIPPET_CHARS - 1).collect();
    cut.push('\u{2026}');
    cut
}

/// "22.3%", "0.4%", "<0.1%", "100%". One decimal under 10%, none from 100%.
pub fn percent(share: f64) -> String {
    let percent = share * 100.0;
    if percent <= 0.0 {
        "0%".to_owned()
    } else if percent < 0.1 {
        "<0.1%".to_owned()
    } else if percent >= 99.95 {
        "100%".to_owned()
    } else {
        format!("{percent:.1}%")
    }
}

/// A time with three significant figures at most: "850 ns", "12.3 \u{b5}s", "329 ms", "1.50 s".
pub fn duration(ns: u64) -> String {
    let (value, unit) = if ns < 1_000 {
        return format!("{ns} ns");
    } else if ns < 1_000_000 {
        (ns as f64 / 1e3, "\u{b5}s")
    } else if ns < 1_000_000_000 {
        (ns as f64 / 1e6, "ms")
    } else {
        (ns as f64 / 1e9, "s")
    };
    if value >= 100.0 {
        format!("{value:.0} {unit}")
    } else if value >= 10.0 {
        format!("{value:.1} {unit}")
    } else {
        format!("{value:.2} {unit}")
    }
}

/// "5,460".
pub fn count(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::profile::support::report;

    const STEP: u64 = 1_000;

    fn profiled(body: &str, bars: usize) -> (ProfileReport, ProfileMarks) {
        let report = report(body, bars, STEP);
        let source = format!("//@version=6\nindicator(\"T\")\n{body}");
        let marks = ProfileMarks::build(&report, &source, 1_234_000);
        (report, marks)
    }

    #[test]
    fn durations_keep_three_figures_and_a_sensible_unit() {
        assert_eq!(duration(0), "0 ns");
        assert_eq!(duration(850), "850 ns");
        assert_eq!(duration(12_345), "12.3 \u{b5}s");
        assert_eq!(duration(329_000_000), "329 ms");
        assert_eq!(duration(1_500_000_000), "1.50 s");
        assert_eq!(duration(999_999), "1000 \u{b5}s");
    }

    #[test]
    fn percentages_name_the_very_small_and_do_not_round_up_to_certainty() {
        assert_eq!(percent(0.0), "0%");
        assert_eq!(percent(0.00004), "<0.1%");
        assert_eq!(percent(0.001), "0.1%");
        assert_eq!(percent(0.223), "22.3%");
        assert_eq!(percent(0.567), "56.7%");
        assert_eq!(percent(0.9996), "100%");
        assert_eq!(percent(0.99), "99.0%");
    }

    #[test]
    fn counts_have_thousands_separators() {
        assert_eq!(count(0), "0");
        assert_eq!(count(999), "999");
        assert_eq!(count(5460), "5,460");
        assert_eq!(count(1_234_567), "1,234,567");
    }

    #[test]
    fn a_line_that_runs_has_a_mark_and_one_that_does_not_has_none() {
        let (_, marks) = profiled("x = close\nif close < 0\n    y = 1\nplot(x)\n", 20);
        assert!(marks.mark(3).is_some());
        assert!(marks.mark(4).is_some());
        assert!(marks.mark(5).is_none(), "the branch never ran");
        assert!(
            marks.mark(1).is_none(),
            "the script's header is not a region"
        );
        assert_eq!(marks.in_view(3, 4).len(), 2);
        assert!(marks.in_view(9, 12).is_empty());
    }

    #[test]
    fn the_longest_bar_is_the_longest_line_and_labels_are_true_shares() {
        let (report, marks) = profiled("t = 0.0\nfor i = 0 to 9\n    t += i\nplot(t)\n", 15);
        let header = marks.mark(4).unwrap();
        let body = marks.mark(5).unwrap();
        assert_eq!(header.bar, 1.0, "the loop includes its body");
        assert!(body.bar < 1.0 && body.bar > 0.0);
        let line = report.lines().into_iter().find(|l| l.line == 4).unwrap();
        assert_eq!(header.label, percent(report.share(line.time_ns)));
        assert_eq!(header.block, Some((4, 5)));
        assert!(body.block.is_none());
    }

    #[test]
    fn heat_follows_what_a_line_spends_itself_not_what_it_runs() {
        let (report, marks) = profiled("t = 0.0\nfor i = 0 to 9\n    t += i\nplot(t)\n", 15);
        let lines = report.lines();
        let hottest = lines.iter().map(|line| line.line_time_ns).max().unwrap();
        for line in &lines {
            let mark = marks.mark(line.line).unwrap();
            let expected = (line.line_time_ns as f64 / hottest as f64) as f32;
            assert_eq!(mark.heat, expected, "line {}", line.line);
        }
        // The loop's bar includes its body, so it is the longest bar whatever its own heat.
        assert_eq!(marks.mark(4).unwrap().bar, 1.0);
        assert!(marks.mark(4).unwrap().bar >= marks.mark(5).unwrap().bar);
    }

    #[test]
    fn flames_mark_the_three_hottest_lines_when_there_are_enough_to_rank() {
        let body = "a = close\nb = a + 1\nfor i = 0 to 20\n    c = b * i\n    d = c + i\n    e = d * 2\nplot(e)\n";
        let (report, marks) = profiled(body, 30);
        let mut expected: Vec<usize> = report
            .hottest_lines(3)
            .into_iter()
            .map(|line| line.line)
            .collect();
        expected.sort_unstable();
        assert_eq!(marks.flames(), expected);
        assert_eq!(expected.len(), 3);
        // Too few lines to rank: none.
        let (_, few) = profiled("a = close\nplot(a)\n", 30);
        assert!(few.flames().is_empty());
    }

    #[test]
    fn a_tooltip_for_a_loop_gives_time_line_time_executions_and_iterations() {
        let (_, marks) = profiled("t = 0.0\nfor i = 0 to 9\n    t += i\nplot(t)\n", 10);
        let tip = &marks.mark(4).unwrap().tooltip;
        assert_eq!(tip.title, "Code block: lines 4\u{2013}5");
        let labels: Vec<&str> = tip.rows.iter().map(|(label, _)| *label).collect();
        assert_eq!(labels[..3], ["Time", "Line time", "Executions"]);
        assert!(labels.contains(&"Iterations"), "{labels:?}");
        let value = |wanted: &str| {
            tip.rows
                .iter()
                .find(|(label, _)| *label == wanted)
                .map(|(_, value)| value.clone())
                .unwrap()
        };
        assert_eq!(value("Executions"), "10");
        assert_eq!(value("Iterations"), "100 (about 10 per entry)");
        assert!(value("Time").contains("of"), "{}", value("Time"));
        let leaf = &marks.mark(5).unwrap().tooltip;
        assert_eq!(leaf.title, "Line 5");
        assert!(leaf.rows.iter().all(|(label, _)| *label != "Line time"));
    }

    #[test]
    fn a_tooltip_for_a_conditional_gives_each_branch() {
        let body = "x = 0.0\nif bar_index % 4 == 0\n    x := 1.0\nelse\n    x := 2.0\nplot(x)\n";
        let (_, marks) = profiled(body, 20);
        let tip = &marks.mark(4).unwrap().tooltip;
        let taken = tip
            .rows
            .iter()
            .find(|(label, _)| *label == "Branches taken")
            .map(|(_, value)| value.as_str());
        assert_eq!(taken, Some("5 \u{b7} 15"));
    }

    #[test]
    fn a_tooltip_for_a_function_lists_where_it_is_called_from() {
        let body = "f(x) =>\n    x * 2\na = f(close)\nb = f(open)\nplot(a + b)\n";
        let (_, marks) = profiled(body, 10);
        let tip = &marks.mark(3).unwrap().tooltip;
        let sites: Vec<&String> = tip
            .rows
            .iter()
            .filter(|(label, _)| *label == "Called from")
            .map(|(_, value)| value)
            .collect();
        assert_eq!(sites.len(), 2);
        assert!(sites[0].starts_with("line 5") || sites[0].starts_with("line 6"));
        assert!(sites[0].contains("10 calls"), "{}", sites[0]);
    }

    #[test]
    fn times_below_the_clocks_resolution_are_flagged_not_averaged() {
        let report = report("x = close\nplot(x)\n", 10, 1);
        let mut s = report.clone();
        s.summary.resolution_ns = 1_000_000;
        let marks = ProfileMarks::build(
            &s,
            "//@version=6\nindicator(\"T\")\nx = close\nplot(x)\n",
            0,
        );
        let tip = &marks.mark(3).unwrap().tooltip;
        assert!(tip.rows.iter().all(|(label, _)| *label != "Average"));
        assert!(
            tip.notes
                .iter()
                .any(|n| n.contains("shorter than the clock"))
        );
    }

    #[test]
    fn the_ranked_list_is_by_line_time_with_the_source_of_each_line() {
        let body = "a = close\nt = 0.0\nfor i = 0 to 9\n    t += i * 2\nplot(t + a)\n";
        let (report, marks) = profiled(body, 10);
        let expected: Vec<usize> = report
            .hottest_lines(10)
            .into_iter()
            .map(|line| line.line)
            .collect();
        let listed: Vec<usize> = marks.hot.iter().map(|row| row.line).collect();
        assert_eq!(listed, expected);
        let row = marks.hot.iter().find(|row| row.line == 6).unwrap();
        assert_eq!(row.snippet, "t += i * 2");
        assert!(
            marks
                .hot
                .windows(2)
                .all(|pair| pair[0].line_share >= pair[1].line_share)
        );
        assert!(marks.hot.len() <= 10);
    }

    #[test]
    fn a_long_line_is_cut_with_an_ellipsis_and_unicode_is_not_split() {
        let long = format!("x = {}", "\u{3c0} + ".repeat(60));
        let text = format!("//@version=6\nindicator(\"T\")\n{long}1\n");
        let cut = snippet(&text, 3);
        assert_eq!(cut.chars().count(), SNIPPET_CHARS);
        assert!(cut.ends_with('\u{2026}'));
        assert_eq!(snippet(&text, 99), "");
    }

    #[test]
    fn the_summary_names_the_run_and_what_is_not_in_any_line() {
        let (report, marks) = profiled("x = close\nplot(x)\n", 25);
        let labels: Vec<&str> = marks.summary.iter().map(|row| row.label).collect();
        for wanted in [
            "Hottest line",
            "Not in any line",
            "Profiler overhead",
            "Clock resolution",
            "Compiled in",
        ] {
            assert!(labels.contains(&wanted), "{wanted} in {labels:?}");
        }
        assert_eq!(marks.totals.time, duration(report.summary.total_ns));
        assert_eq!(marks.totals.bars, "25");
        assert_eq!(
            marks.totals.executions,
            count(report.summary.measured_executions)
        );
        let hottest = marks.summary.iter().find(|row| row.label == "Hottest line");
        assert!(hottest.is_some_and(|row| row.line.is_some()));
        assert!(
            marks
                .summary
                .iter()
                .filter(|row| row.group == SummaryGroup::Hotspots)
                .all(|row| row.line.is_some()),
            "every hotspot goes to its line"
        );
        assert!(
            marks
                .summary
                .iter()
                .filter(|row| row.group == SummaryGroup::Measurement)
                .all(|row| row.line.is_none())
        );
        assert!(marks.caveat.is_none());
    }

    #[test]
    fn a_partial_profile_says_how_far_it_got_and_why() {
        let (mut report, _) = profiled("x = close\nplot(x)\n", 25);
        report.completeness = Completeness::Partial(Interruption::Cancelled);
        let caveat = ProfileMarks::build(&report, "", 0).caveat.unwrap();
        assert_eq!(caveat, "Partial: cancelled after 25 bars.");
        report.completeness = Completeness::Partial(Interruption::TimedOut);
        assert!(
            ProfileMarks::build(&report, "", 0)
                .caveat
                .unwrap()
                .contains("ran out of time")
        );
    }

    #[test]
    fn lines_too_small_to_draw_are_quiet_but_still_have_a_tooltip() {
        let (report, _) = profiled("x = close\nplot(x)\n", 25);
        let mut tiny = report.clone();
        tiny.summary.total_ns *= 10_000;
        let marks = ProfileMarks::build(&tiny, "", 0);
        let mark = marks.mark(3).unwrap();
        assert!(mark.quiet);
        assert!(!mark.tooltip.rows.is_empty());
        assert!(mark.bar > 0.0);
    }
}
