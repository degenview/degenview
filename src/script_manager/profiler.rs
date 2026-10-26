//! The Script Manager's profiler: whether it is on, the run in flight, the profile it produced and
//! whether that profile still describes what is on screen.
//!
//! A profile belongs to the exact script text, inputs, symbol, timeframe and bars it was produced
//! for (a [`ProfileKey`]). Whatever the key no longer matches, the profile is *stale*: it is kept,
//! labelled, and never drawn on the editor, because its line numbers describe other text.
//!
//! The panel holds no egui types. The manager starts runs and asks it what to show.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::pine::profile::{ProfileConfig, ProfileOutcome, ProfileReport, ProfileSession};
use crate::pine::program::InputValue;

use super::profile_marks::{ProfileColumn, ProfileMarks};

/// How often a running profile asks for a frame, to show its progress.
const PROGRESS_FRAME: Duration = Duration::from_millis(100);

/// What a profile was produced for. Two keys are equal exactly when running the script again would
/// measure the same thing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileKey {
    pub script_id: String,
    pub source: u64,
    pub inputs: u64,
    /// The symbol and timeframe, as the preview names them.
    pub chart: String,
    /// Bars, first bar's open time and last bar's open time: a new bar is different data, a tick on
    /// the last one is not.
    pub bars: Option<(usize, i64, i64)>,
}

impl ProfileKey {
    /// Whether `other` is the same script with the same text, so that line numbers mean the same.
    pub fn same_text(&self, other: &Self) -> bool {
        self.script_id == other.script_id && self.source == other.source
    }

    pub fn new(
        script_id: &str,
        source: &str,
        inputs: &HashMap<String, InputValue>,
        chart: &str,
        bars: Option<(usize, i64, i64)>,
    ) -> Self {
        Self {
            script_id: script_id.to_owned(),
            source: fingerprint(source.as_bytes()),
            inputs: fingerprint(canonical_inputs(inputs).as_bytes()),
            chart: chart.to_owned(),
            bars,
        }
    }
}

/// The inputs in a fixed order, so the same values always make the same text.
fn canonical_inputs(inputs: &HashMap<String, InputValue>) -> String {
    let mut entries: Vec<(&String, &InputValue)> = inputs.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    format!("{entries:?}")
}

fn fingerprint(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// A finished profile with the words made from it.
pub struct Profile {
    pub key: ProfileKey,
    /// The text that was profiled: what the line numbers in the profile refer to.
    pub source: Arc<str>,
    /// The symbol and timeframe it ran on, for the summary.
    pub label: String,
    pub report: Arc<ProfileReport>,
    pub marks: Arc<ProfileMarks>,
}

struct Run {
    session: ProfileSession,
    key: ProfileKey,
    source: Arc<str>,
    label: String,
}

/// Why the last attempt did not produce a profile.
struct Failure {
    key: ProfileKey,
    message: String,
    /// The script could not be run at all (it does not compile): the attempt is worth repeating when
    /// its text changes.
    not_runnable: bool,
}

/// What the toolbar and the drawer say.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Status {
    Off,
    /// On, and nothing has run yet for the script shown.
    Idle,
    /// Waiting for something the run needs (the preview chart's bars).
    Waiting(&'static str),
    Running {
        done: u64,
        total: u64,
        cancelling: bool,
    },
    Ready,
    /// A profile of part of the run: cancelled, failed or out of time.
    Partial,
    /// A profile of other text, inputs or data than what is on screen.
    Stale,
    Failed(String),
}

impl Status {
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running { .. })
    }
}

#[derive(Default)]
pub struct Profiler {
    on: bool,
    run: Option<Run>,
    profile: Option<Profile>,
    failure: Option<Failure>,
    /// Something the run needs has not arrived (the preview chart's bars); asked again each frame.
    waiting: Option<&'static str>,
    /// A run may start by itself: just turned on, or switched to another script. A run that was
    /// started, or a profile gone stale by editing, clears it; rerunning is then the person's call.
    auto: bool,
}

impl Profiler {
    pub fn is_on(&self) -> bool {
        self.on
    }

    pub fn profile(&self) -> Option<&Profile> {
        self.profile.as_ref()
    }

    /// Turns profiling on for the open script; the next frame starts a run.
    pub fn enable(&mut self) {
        self.on = true;
        self.auto = true;
    }

    /// Turns profiling off and forgets everything it produced. A run in flight ends.
    pub fn disable(&mut self) {
        self.on = false;
        self.auto = false;
        self.run = None;
        self.profile = None;
        self.failure = None;
        self.waiting = None;
    }

    /// The open script changed: what was measured is another script's.
    pub fn script_changed(&mut self) {
        self.run = None;
        self.profile = None;
        self.failure = None;
        self.waiting = None;
        self.auto = self.on;
    }

    /// The profile belongs to a script that no longer exists.
    pub fn script_deleted(&mut self, id: &str) {
        let concerned = self.profile.as_ref().is_some_and(|p| p.key.script_id == id)
            || self.run.as_ref().is_some_and(|r| r.key.script_id == id)
            || self.failure.as_ref().is_some_and(|f| f.key.script_id == id);
        if concerned {
            self.script_changed();
        }
    }

    /// Whether a run should start now, by itself.
    pub fn wants_start(&self) -> bool {
        self.on && self.auto && self.run.is_none()
    }

    pub fn is_running(&self) -> bool {
        self.run.is_some()
    }

    /// Notes that a run cannot start until `why` is there; the manager tries again next frame, a
    /// run the person asked for as much as one that was to start by itself.
    pub fn wait(&mut self, why: &'static str) {
        self.waiting = Some(why);
        self.auto = self.on;
    }

    /// Notes a refusal without running anything.
    pub fn refuse(&mut self, key: ProfileKey, message: &str, not_runnable: bool) {
        self.waiting = None;
        self.auto = false;
        self.failure = Some(Failure {
            key,
            message: message.to_owned(),
            not_runnable,
        });
    }

    /// Starts a run, ending the one in flight if there is one.
    pub fn begin(
        &mut self,
        key: ProfileKey,
        source: &str,
        label: String,
        config: ProfileConfig,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<(), String> {
        self.run = None;
        self.failure = None;
        self.waiting = None;
        self.auto = false;
        match ProfileSession::start(config, wake) {
            Ok(session) => {
                self.run = Some(Run {
                    session,
                    key,
                    source: source.into(),
                    label,
                });
                Ok(())
            }
            Err(error) => {
                let message = error.to_string();
                self.failure = Some(Failure {
                    key,
                    message: message.clone(),
                    not_runnable: false,
                });
                Err(message)
            }
        }
    }

    /// Asks the run to stop; what it measured so far still arrives, as a partial profile.
    pub fn cancel(&mut self) {
        if let Some(run) = &self.run {
            run.session.cancel();
        }
    }

    /// Reads the outcome of a finished run. Returns whether anything changed.
    pub fn poll(&mut self) -> bool {
        let Some(run) = self.run.as_mut() else {
            return false;
        };
        let Some(outcome) = run.session.poll() else {
            return false;
        };
        let run = self.run.take().expect("a run was polled");
        self.finish(run, outcome);
        true
    }

    fn finish(&mut self, run: Run, outcome: ProfileOutcome) {
        match outcome.result {
            Ok(report) => {
                let marks = ProfileMarks::build(&report, &run.source, outcome.compile_ns);
                self.profile = Some(Profile {
                    key: run.key,
                    source: run.source,
                    label: run.label,
                    report: Arc::new(report),
                    marks: Arc::new(marks),
                });
            }
            Err(error) => {
                self.failure = Some(Failure {
                    key: run.key,
                    message: error.message,
                    not_runnable: false,
                });
            }
        }
    }

    /// A handle that outlives the run, to watch its thread end.
    pub fn liveness(&self) -> Option<std::sync::Weak<()>> {
        self.run.as_ref().map(|run| run.session.liveness())
    }

    /// The repaint a run in flight wants, to show its progress.
    pub fn wants_frames(&self) -> Option<Duration> {
        self.run.as_ref().map(|_| PROGRESS_FRAME)
    }

    /// Forgets a refusal that the script's new text may have cured, and lets a run start again.
    pub fn text_changed(&mut self, current: &ProfileKey) {
        if let Some(failure) = &self.failure
            && failure.not_runnable
            && failure.key != *current
        {
            self.failure = None;
            self.auto = self.on;
        }
    }

    /// What the profiler is doing, for the script described by `current`.
    pub fn status(&self, current: Option<&ProfileKey>) -> Status {
        if !self.on {
            return Status::Off;
        }
        if let Some(run) = &self.run {
            let (done, total) = run.session.progress();
            return Status::Running {
                done,
                total,
                cancelling: run.session.is_cancelled(),
            };
        }
        if let Some(profile) = &self.profile {
            return if current == Some(&profile.key) {
                if profile.report.is_complete() {
                    Status::Ready
                } else {
                    Status::Partial
                }
            } else {
                Status::Stale
            };
        }
        if let Some(failure) = &self.failure
            && current == Some(&failure.key)
        {
            return Status::Failed(failure.message.clone());
        }
        match self.waiting {
            Some(why) => Status::Waiting(why),
            None => Status::Idle,
        }
    }

    /// Whether the profile's line numbers still mean what they do in the editor: it is of the text
    /// on screen, whatever has changed besides.
    pub fn lines_match(&self, current: Option<&ProfileKey>) -> bool {
        match (&self.profile, current) {
            (Some(profile), Some(current)) => profile.key.same_text(current),
            _ => false,
        }
    }

    /// What the editor's profile column shows.
    pub fn column(&self, current: Option<&ProfileKey>) -> Option<ProfileColumn> {
        let status = self.status(current);
        Some(match status {
            Status::Off => return None,
            Status::Ready | Status::Partial => {
                ProfileColumn::Marks(Arc::clone(&self.profile.as_ref()?.marks))
            }
            Status::Stale if self.lines_match(current) => {
                ProfileColumn::Dated(Arc::clone(&self.profile.as_ref()?.marks))
            }
            Status::Running { .. } => ProfileColumn::Empty("Profiling\u{2026}"),
            Status::Stale => ProfileColumn::Empty(
                "This profile is of earlier text. Run it again to see where the time goes now.",
            ),
            Status::Failed(_) => ProfileColumn::Empty("The script could not be profiled."),
            Status::Waiting(why) => ProfileColumn::Empty(why),
            Status::Idle => ProfileColumn::Empty("Run the profiler to see where the time goes."),
        })
    }
}
