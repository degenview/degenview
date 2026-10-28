//! The assistant's `profile` tool: runs the draft on a frozen copy of the preview chart's bars and
//! tells the model, in words, where the time goes.
//!
//! This is the engine's profiler (`pine::profile`, see `docs/pine-profiler.md`), the one behind the
//! editor's Profile tab, on its own session and thread, so it shares nothing with the preview, a
//! chart, an alert or an order. It is *not* the UI's `Profiler`: that one belongs to the text in the
//! editor, and this one profiles the assistant's draft, which is somewhere else. The words come from
//! `ProfileMarks`, so the model reads the same figures the user would.
//!
//! The bars are never sent anywhere: only the words made from the profile are.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::ai_tools::Outcome;
use super::debug::DebugFeed;
use super::profile_marks::{ProfileMarks, count, duration, percent};
use super::registry::LibraryRegistry;
use crate::host::pine_host::KlineSecurityDataProvider;
use crate::pine::profile::{
    Completeness, Interruption, ProfileConfig, ProfileOutcome, ProfileReport, ProfileSession,
};
use std::sync::atomic::{AtomicBool, Ordering};

const POLL: Duration = Duration::from_millis(15);
/// The engine's own deadline is shorter; this only keeps a stuck run from holding the request.
const MAX_WAIT: Duration = Duration::from_secs(60);
/// How long a run asked to stop is given to say so.
const STOP_GRACE: Duration = Duration::from_secs(3);
/// Lines listed. The editor's list has ten as well.
const HOT_LINES: usize = 10;

/// What the last complete profile of a request measured, to compare the next one with.
#[derive(Clone, Copy)]
struct Previous {
    total_ns: u64,
    executions: u64,
}

pub struct ProfileTool {
    /// The preview chart's bars as they were when the request was sent; `None` if it had none.
    feed: Option<DebugFeed>,
    previous: Mutex<Option<Previous>>,
}

impl ProfileTool {
    pub fn new(feed: Option<DebugFeed>) -> Self {
        Self {
            feed,
            previous: Mutex::new(None),
        }
    }

    /// Profiles `source`. Anything that stops it is an error answer the model can read.
    pub fn run(&self, source: &str, cancel: &AtomicBool) -> Outcome {
        let Some(feed) = &self.feed else {
            return Outcome::error(
                "There are no bars to run the script on: the preview chart has not loaded any. Tell the user.",
            );
        };
        if source
            .lines()
            .any(|line| line.trim_start().starts_with("library("))
        {
            return Outcome::error("A library exports functions; there is nothing to run.");
        }
        let mut config = ProfileConfig::new(
            source.to_owned(),
            feed.symbol.clone(),
            feed.bars.clone(),
            feed.now_ms,
        );
        config.inputs = feed.inputs.clone();
        config.theme = Some(feed.theme);
        config.libraries = Box::new(LibraryRegistry);
        config.security_data = Some(Box::new(KlineSecurityDataProvider));
        let mut session = match ProfileSession::start(config, std::sync::Arc::new(|| {})) {
            Ok(session) => session,
            Err(error) => return Outcome::error(error.to_string()),
        };
        let outcome = match wait(&mut session, cancel) {
            Some(outcome) => outcome,
            None => return Outcome::error("The profile run did not stop when asked to."),
        };
        self.answer(outcome, source, &feed.label, cancel.load(Ordering::Relaxed))
    }

    fn answer(
        &self,
        outcome: ProfileOutcome,
        source: &str,
        label: &str,
        cancelled: bool,
    ) -> Outcome {
        let report = match outcome.result {
            Ok(report) => report,
            Err(error) => {
                return Outcome::error(format!("The script could not be run: {}", error.message));
            }
        };
        // What it measured leaves out the failing bar and all after it, so a failure has no profile.
        if let Completeness::Partial(Interruption::Failed(error)) = &report.completeness {
            return Outcome::error(format!(
                "The script failed after {} bars, so there is no profile: {}",
                count(report.summary.bars),
                error.message
            ));
        }
        if cancelled {
            return Outcome::error("Stopped.");
        }
        let marks = ProfileMarks::build(&report, source, outcome.compile_ns);
        let previous = self.take_previous(&report);
        let content = describe(&report, &marks, label, previous);
        let summary = match marks.hot.first() {
            Some(hot) => format!(
                "Profiled the draft \u{b7} {} bars \u{b7} {} \u{b7} hottest line {} ({})",
                marks.totals.bars,
                marks.totals.time,
                hot.line,
                percent(f64::from(hot.line_share))
            ),
            None => format!(
                "Profiled the draft \u{b7} {} bars \u{b7} {}",
                marks.totals.bars, marks.totals.time
            ),
        };
        Outcome {
            content,
            is_error: false,
            summary,
        }
    }

    /// The profile before this one, if both ran to the end; this one becomes the next's.
    fn take_previous(&self, report: &ProfileReport) -> Option<Previous> {
        let mut slot = self.previous.lock().ok()?;
        let before = *slot;
        if report.is_complete() {
            *slot = Some(Previous {
                total_ns: report.summary.total_ns,
                executions: report.summary.measured_executions,
            });
        }
        before
    }
}

/// Polls the session until it ends. A request that is stopped asks the run to stop, and gets what it
/// measured so far (or `None` if it will not stop).
fn wait(session: &mut ProfileSession, cancel: &AtomicBool) -> Option<ProfileOutcome> {
    let started = Instant::now();
    let mut asked = None;
    loop {
        if let Some(outcome) = session.poll() {
            return Some(outcome);
        }
        if (cancel.load(Ordering::Relaxed) || started.elapsed() > MAX_WAIT) && asked.is_none() {
            session.cancel();
            asked = Some(Instant::now());
        }
        if asked.is_some_and(|at| at.elapsed() > STOP_GRACE) {
            return None;
        }
        std::thread::sleep(POLL);
    }
}

/// A signed change as text: "\u{2212}44%", "+3%", "no change".
fn change(before: u64, after: u64) -> String {
    if before == 0 {
        return "n/a".to_owned();
    }
    let delta = (after as f64 - before as f64) / before as f64 * 100.0;
    if delta.abs() < 0.5 {
        "no change".to_owned()
    } else if delta < 0.0 {
        format!("\u{2212}{:.0}%", -delta)
    } else {
        format!("+{delta:.0}%")
    }
}

fn describe(
    report: &ProfileReport,
    marks: &ProfileMarks,
    label: &str,
    previous: Option<Previous>,
) -> String {
    let mut out = format!(
        "Profile of the draft on {label}, {} bars.",
        marks.totals.bars
    );
    if let Some(caveat) = &marks.caveat {
        out.push_str(&format!(" {caveat}"));
    }
    out.push_str(&format!(
        "\nTotal time {} over {} measured executions.\n",
        marks.totals.time, marks.totals.executions
    ));
    out.push_str(
        "\nHottest lines by the time spent in the line itself (line: share of the total \u{b7} own time \u{b7} time with everything it runs \u{b7} executions \u{b7} source):\n",
    );
    for row in marks.hot.iter().take(HOT_LINES) {
        out.push_str(&format!(
            "{}: {} \u{b7} {} \u{b7} {} \u{b7} \u{d7}{} \u{b7} {}\n",
            row.line,
            percent(f64::from(row.line_share)),
            row.line_time,
            row.time,
            row.executions,
            row.snippet
        ));
    }
    let wanted = [
        "Hottest block",
        "Hottest function",
        "Not in any line",
        "Waiting for data",
    ];
    let mut extra = String::new();
    for row in &marks.summary {
        if wanted.contains(&row.label) {
            let note = row
                .note
                .as_deref()
                .map_or(String::new(), |n| format!(" ({n})"));
            extra.push_str(&format!("{}: {}{note}\n", row.label, row.value));
        }
    }
    if !extra.is_empty() {
        out.push('\n');
        out.push_str(&extra);
    }
    if let Some(before) = previous
        && report.is_complete()
    {
        out.push_str(&format!(
            "\nCompared with the previous profile in this request: total time {} \u{2192} {} ({}); measured executions {} \u{2192} {} ({}). Executions are exact; times vary a little between runs, so do not read a small difference in time as a result.\n",
            duration(before.total_ns),
            marks.totals.time,
            change(before.total_ns, report.summary.total_ns),
            count(before.executions),
            marks.totals.executions,
            change(before.executions, report.summary.measured_executions),
        ));
    }
    out.push_str(&format!(
        "\nHow to read it: one profile covers at most 500 bars, so the shares are meaningful and the absolute times are of this run only. It is measured per statement, not per expression: split a long line to learn which part costs. A time under {} is below what the clock can tell apart. The profiler's own cost (about {} per measured execution) is inside every time.\n",
        duration(report.summary.resolution_ns),
        duration(report.summary.pair_cost_ns)
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Kline;
    use crate::pine::program::{ChartTheme, SymbolInfo};
    use std::collections::HashMap;

    const LOOP: &str = "//@version=6\nindicator(\"T\")\ntotal = 0.0\nfor i = 0 to 49\n    total += math.sin(i)\nplot(total)\n";

    /// Bars that wander up and down, a minute apart.
    fn bars(count: usize, spacing_ms: i64) -> Vec<Kline> {
        (0..count)
            .map(|index| {
                let base = 100.0 + (index as f64 * 0.7).sin() * 5.0;
                Kline {
                    open_time: 1_700_000_000_000 + index as i64 * spacing_ms,
                    close_time: 1_700_000_000_000 + (index as i64 + 1) * spacing_ms - 1,
                    open: base,
                    high: base + 1.0,
                    low: base - 1.0,
                    close: base + 0.5,
                    volume: 10.0,
                }
            })
            .collect()
    }

    fn feed(count: usize) -> DebugFeed {
        let data: Vec<Kline> = bars(count, 60_000);
        DebugFeed {
            now_ms: data.last().map_or(0, |bar| bar.open_time + 60_000),
            bars: data,
            symbol: SymbolInfo::default(),
            inputs: HashMap::new(),
            theme: ChartTheme::DARK,
            label: "TEST \u{b7} 1m".to_owned(),
        }
    }

    fn tool(count: usize) -> ProfileTool {
        ProfileTool::new(Some(feed(count)))
    }

    #[test]
    fn a_profile_names_the_hot_lines_with_their_exact_counts() {
        let outcome = tool(30).run(LOOP, &AtomicBool::new(false));
        assert!(!outcome.is_error, "{}", outcome.content);
        let text = &outcome.content;
        assert!(text.starts_with("Profile of the draft on TEST \u{b7} 1m, 30 bars."));
        // Executions are exact: the loop is entered once a bar, its body 50 times.
        assert!(
            text.contains("\u{d7}1,500 \u{b7} total += math.sin(i)"),
            "{text}"
        );
        assert!(text.contains("\u{d7}30 \u{b7} for i = 0 to 49"), "{text}");
        assert!(text.contains("Hottest block"));
        assert!(text.contains("How to read it"));
        assert!(
            outcome
                .summary
                .starts_with("Profiled the draft \u{b7} 30 bars")
        );
    }

    #[test]
    fn the_second_profile_is_compared_with_the_first() {
        let tool = tool(30);
        let first = tool.run(LOOP, &AtomicBool::new(false));
        assert!(!first.content.contains("Compared with the previous"));
        let lighter = LOOP.replace("0 to 49", "0 to 24");
        let second = tool.run(&lighter, &AtomicBool::new(false));
        assert!(!second.is_error, "{}", second.content);
        let text = &second.content;
        assert!(
            text.contains("Compared with the previous profile in this request"),
            "{text}"
        );
        // 30 bars x 50 body runs became 30 x 25: executions, which do not vary, fell by about half.
        assert!(text.contains("\u{2212}"), "{text}");
        assert!(text.contains("Executions are exact"));
    }

    #[test]
    fn a_script_that_fails_has_no_profile_only_the_failure() {
        let failing =
            "//@version=6\nindicator(\"T\")\nx = array.get(array.new_float(0), 3)\nplot(x)\n";
        let outcome = tool(10).run(failing, &AtomicBool::new(false));
        assert!(outcome.is_error);
        assert!(
            outcome.content.contains("so there is no profile"),
            "{}",
            outcome.content
        );
    }

    #[test]
    fn nothing_to_run_on_is_an_answer_not_a_failure_of_the_request() {
        let none = ProfileTool::new(None).run(LOOP, &AtomicBool::new(false));
        assert!(none.is_error);
        assert!(none.content.contains("no bars"));
        let library = "//@version=6\nlibrary(\"L\")\nexport f(x) => x\n";
        let outcome = tool(5).run(library, &AtomicBool::new(false));
        assert!(outcome.is_error);
        assert!(outcome.content.contains("library"));
    }

    #[test]
    fn a_stopped_request_stops_the_run() {
        let slow = "//@version=6\nindicator(\"T\")\ntotal = 0.0\nfor i = 0 to 2000000\n    total += math.sin(i)\nplot(total)\n";
        let cancel = AtomicBool::new(true);
        let started = Instant::now();
        let outcome = tool(200).run(slow, &cancel);
        assert!(outcome.is_error);
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn changes_read_as_signed_percentages() {
        assert_eq!(change(100, 56), "\u{2212}44%");
        assert_eq!(change(100, 103), "+3%");
        assert_eq!(change(100, 100), "no change");
        assert_eq!(change(0, 5), "n/a");
    }
}
