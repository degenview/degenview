//! What earlier starts left behind, as a pure state machine. A start moves through it in memory
//! (`Session` owns the value and persists it); nothing here reads or writes a file.
//!
//! - `open` turns a leftover in-progress attempt into a failure: a start that never confirmed died
//!   in the driver (a crash or a hang no panic hook sees), or was killed.
//! - `select` picks the first rung not known to fail, and records the attempt under this launch's name.
//! - `fail` and `confirm` settle an attempt, and only that attempt: another launch's marker is never
//!   cleared by this one.

use serde::{Deserialize, Serialize};

use super::ladder::Rung;

/// One try at starting the renderer: the rung, and the launch of the app that made it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    pub rung: String,
    pub launch: String,
}

/// What `graphics.json` holds: the build it is about, the rungs that did not start on it, and the
/// attempt in progress.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    #[serde(default)]
    pub build: String,
    #[serde(default)]
    pub failed: Vec<String>,
    #[serde(default)]
    pub attempting: Option<Attempt>,
}

/// The rung a start takes, and why it is not the first one.
pub struct Choice<'a> {
    pub rung: &'a Rung,
    pub attempt: Attempt,
    /// Rungs known not to start on this build.
    pub skipped: Vec<String>,
    /// Every rung had failed, so the list started over.
    pub wrapped: bool,
}

impl Choice<'_> {
    /// The line `startup.log` gets.
    pub fn note(&self) -> String {
        let mut note = format!("graphics backends: {}", self.rung.name);
        if self.wrapped {
            note.push_str(" (every backend failed to start before; trying them again)");
        } else if !self.skipped.is_empty() {
            note.push_str(&format!(
                " (did not start on this build: {})",
                self.skipped.join(", ")
            ));
        }
        note
    }
}

impl Record {
    /// What a launch starts from. State about another build is dropped, so an update tries the
    /// first rung again. An attempt left in progress by an earlier launch counts as a failure.
    pub fn open(previous: Self, build: &str) -> Self {
        let mut record = if previous.build == build {
            previous
        } else {
            Self {
                build: build.to_owned(),
                ..Self::default()
            }
        };
        if let Some(stale) = record.attempting.take() {
            record.mark_failed(stale.rung);
        }
        record
    }

    /// Takes the first rung not known to fail. When all have, the list starts over. `ladder` is
    /// never empty.
    pub fn select<'a>(&mut self, ladder: &'a [Rung], launch: &str) -> Choice<'a> {
        let skipped = self.failed.clone();
        let wrapped = ladder.iter().all(|rung| self.has_failed(rung.name));
        if wrapped {
            self.failed.clear();
        }
        let rung = ladder
            .iter()
            .find(|rung| !self.has_failed(rung.name))
            .unwrap_or(&ladder[0]);
        let attempt = Attempt {
            rung: rung.name.to_owned(),
            launch: launch.to_owned(),
        };
        self.attempting = Some(attempt.clone());
        Choice {
            rung,
            attempt,
            skipped,
            wrapped,
        }
    }

    /// The attempt did not start. Whether or not it is still the one in progress, its rung is out.
    pub fn fail(&mut self, attempt: &Attempt) {
        self.confirm(attempt);
        self.mark_failed(attempt.rung.clone());
    }

    /// The attempt started. Clears the in-progress mark only if it is this attempt's own.
    pub fn confirm(&mut self, attempt: &Attempt) -> bool {
        let own = self.attempting.as_ref() == Some(attempt);
        if own {
            self.attempting = None;
        }
        own
    }

    fn mark_failed(&mut self, rung: String) {
        if !self.failed.contains(&rung) {
            self.failed.push(rung);
        }
    }

    fn has_failed(&self, rung: &str) -> bool {
        self.failed.iter().any(|name| name == rung)
    }
}

#[cfg(test)]
mod tests {
    use eframe::wgpu::Backends;

    use super::*;

    const THREE: &[Rung] = &[
        Rung {
            name: "a",
            backends: Backends::DX12,
        },
        Rung {
            name: "b",
            backends: Backends::GL,
        },
        Rung {
            name: "c",
            backends: Backends::VULKAN,
        },
    ];

    fn record(failed: &[&str], attempting: Option<(&str, &str)>) -> Record {
        Record {
            build: "build".to_owned(),
            failed: failed.iter().map(|name| (*name).to_owned()).collect(),
            attempting: attempting.map(|(rung, launch)| Attempt {
                rung: rung.to_owned(),
                launch: launch.to_owned(),
            }),
        }
    }

    #[test]
    fn the_first_start_takes_the_first_rung_and_marks_it_in_progress() {
        let mut record = Record::open(Record::default(), "build");
        let choice = record.select(THREE, "me");
        assert_eq!(choice.rung.name, "a");
        assert_eq!(choice.note(), "graphics backends: a");
        assert_eq!(record.attempting.as_ref(), Some(&choice.attempt));
        assert!(record.failed.is_empty());
    }

    #[test]
    fn an_attempt_left_by_an_earlier_launch_is_a_failure() {
        let mut record = Record::open(record(&[], Some(("a", "earlier"))), "build");
        assert_eq!(record.failed, ["a"]);
        assert_eq!(record.attempting, None);
        let choice = record.select(THREE, "me");
        assert_eq!(choice.rung.name, "b");
        assert_eq!(
            choice.note(),
            "graphics backends: b (did not start on this build: a)"
        );
    }

    #[test]
    fn a_new_build_forgets_what_failed_on_the_old_one() {
        let mut record = Record::open(record(&["a"], Some(("b", "earlier"))), "newer");
        assert_eq!(record.build, "newer");
        assert!(record.failed.is_empty() && record.attempting.is_none());
        assert_eq!(record.select(THREE, "me").rung.name, "a");
    }

    #[test]
    fn a_confirmed_rung_is_kept_by_the_next_launch() {
        let mut record = Record::open(record(&["a"], None), "build");
        assert_eq!(record.select(THREE, "me").rung.name, "b");
    }

    #[test]
    fn failing_the_attempts_in_turn_walks_down_the_ladder() {
        let mut record = Record::open(Record::default(), "build");
        let first = record.select(THREE, "me").attempt;
        record.fail(&first);
        let second = record.select(THREE, "me");
        assert_eq!(second.rung.name, "b");
        record.fail(&second.attempt);
        assert_eq!(record.select(THREE, "me").rung.name, "c");
        assert_eq!(record.failed, ["a", "b"]);
    }

    #[test]
    fn when_every_rung_has_failed_the_list_starts_over() {
        let mut record = Record::open(record(&["a", "b", "c"], None), "build");
        let choice = record.select(THREE, "me");
        assert!(choice.wrapped);
        assert_eq!(choice.rung.name, "a");
        assert!(record.failed.is_empty());
        assert!(choice.note().contains("trying them again"));
    }

    #[test]
    fn a_single_rung_ladder_stays_on_its_rung() {
        let one = &THREE[..1];
        let mut record = Record::open(record(&["a"], None), "build");
        let choice = record.select(one, "me");
        assert_eq!((choice.rung.name, choice.wrapped), ("a", true));
    }

    #[test]
    fn a_failure_is_recorded_once() {
        let mut record = Record::open(Record::default(), "build");
        let attempt = record.select(THREE, "me").attempt;
        record.fail(&attempt);
        record.fail(&attempt);
        assert_eq!(record.failed, ["a"]);
    }

    #[test]
    fn only_the_attempt_that_made_the_mark_can_confirm_it() {
        let mut record = record(&[], Some(("a", "other")));
        let mine = Attempt {
            rung: "a".to_owned(),
            launch: "me".to_owned(),
        };
        assert!(!record.confirm(&mine));
        assert!(record.attempting.is_some(), "the other launch's mark stays");
        let theirs = record.attempting.clone().unwrap();
        assert!(record.confirm(&theirs));
        assert!(record.attempting.is_none());
    }

    #[test]
    fn a_rung_the_ladder_no_longer_has_is_ignored() {
        let mut record = Record::open(record(&["removed"], None), "build");
        assert_eq!(record.select(THREE, "me").rung.name, "a");
    }
}
