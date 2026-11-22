//! The graphics backends a start may use, in the order they are tried, and the rule that moves down
//! the list after a start that never drew a frame. Pure: `state.rs` keeps the memory between starts.

use eframe::wgpu::Backends;
use serde::{Deserialize, Serialize};

/// One way to start the renderer: the wgpu backends the instance may probe.
pub struct Rung {
    pub name: &'static str,
    pub backends: Backends,
}

/// Windows: DirectX 12 is on every Windows 10 and later, and the Vulkan driver is the usual place a
/// start dies without a trace (drivers and injected layers that fault while the instance probes it),
/// so it is the last resort, not part of the default.
#[cfg(windows)]
pub const LADDER: &[Rung] = &[
    Rung {
        name: "dx12",
        backends: Backends::DX12,
    },
    Rung {
        name: "gl",
        backends: Backends::GL,
    },
    Rung {
        name: "vulkan",
        backends: Backends::VULKAN,
    },
];

/// macOS has one renderer, Metal.
#[cfg(target_os = "macos")]
pub const LADDER: &[Rung] = &[Rung {
    name: "metal",
    backends: Backends::METAL,
}];

/// Elsewhere: Vulkan with GL as its companion, then each alone.
#[cfg(not(any(windows, target_os = "macos")))]
pub const LADDER: &[Rung] = &[
    Rung {
        name: "vulkan+gl",
        backends: Backends::VULKAN.union(Backends::GL),
    },
    Rung {
        name: "gl",
        backends: Backends::GL,
    },
    Rung {
        name: "vulkan",
        backends: Backends::VULKAN,
    },
];

/// What `graphics.json` remembers: which build it is about, the rungs that did not start on it, and
/// the rung the last start was trying (cleared once that start drew its first frame).
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub build: String,
    #[serde(default)]
    pub failed: Vec<String>,
    #[serde(default)]
    pub attempting: Option<String>,
}

/// The rung this start uses, the state to remember, and what the log should say about the choice.
pub struct Plan<'a> {
    pub rung: &'a Rung,
    pub state: State,
    /// Rungs that did not start on this build, so the log says why this one is not the first.
    pub skipped: Vec<String>,
    /// Every rung had failed, so the list starts over.
    pub wrapped: bool,
}

impl Plan<'_> {
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

/// Picks the rung for this start. A start that left `attempting` behind never drew a frame, so that
/// rung joins `failed` (a crash or hang in the driver, which no panic hook sees); state about another
/// build is dropped, so an update tries the first rung again. `ladder` is never empty.
pub fn plan<'a>(ladder: &'a [Rung], previous: State, build: &str) -> Plan<'a> {
    let mut state = if previous.build == build {
        previous
    } else {
        State {
            build: build.to_owned(),
            ..State::default()
        }
    };
    if let Some(name) = state.attempting.take()
        && !state.failed.contains(&name)
    {
        state.failed.push(name);
    }
    let skipped = state.failed.clone();
    let wrapped = ladder
        .iter()
        .all(|rung| state.failed.iter().any(|name| name == rung.name));
    if wrapped {
        state.failed.clear();
    }
    let rung = ladder
        .iter()
        .find(|rung| !state.failed.iter().any(|name| name == rung.name))
        .unwrap_or(&ladder[0]);
    state.attempting = Some(rung.name.to_owned());
    Plan {
        rung,
        state,
        skipped,
        wrapped,
    }
}

#[cfg(test)]
mod tests {
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

    fn after(attempting: &str, failed: &[&str]) -> State {
        State {
            build: "build".to_owned(),
            failed: failed.iter().map(|name| (*name).to_owned()).collect(),
            attempting: Some(attempting.to_owned()),
        }
    }

    #[test]
    fn the_first_start_uses_the_first_rung_and_marks_it_in_progress() {
        let plan = plan(THREE, State::default(), "build");
        assert_eq!(plan.rung.name, "a");
        assert_eq!(plan.state.build, "build");
        assert_eq!(plan.state.attempting.as_deref(), Some("a"));
        assert!(plan.state.failed.is_empty());
        assert_eq!(plan.note(), "graphics backends: a");
    }

    #[test]
    fn a_start_that_never_drew_a_frame_moves_to_the_next_rung() {
        let plan = plan(THREE, after("a", &[]), "build");
        assert_eq!(plan.rung.name, "b");
        assert_eq!(plan.state.failed, ["a"]);
        assert_eq!(plan.state.attempting.as_deref(), Some("b"));
        assert_eq!(
            plan.note(),
            "graphics backends: b (did not start on this build: a)"
        );
    }

    #[test]
    fn two_failed_starts_move_two_rungs() {
        let plan = plan(THREE, after("b", &["a"]), "build");
        assert_eq!(plan.rung.name, "c");
        assert_eq!(plan.state.failed, ["a", "b"]);
    }

    #[test]
    fn a_confirmed_start_keeps_its_rung() {
        let confirmed = State {
            attempting: None,
            ..after("b", &["a"])
        };
        let plan = plan(THREE, confirmed, "build");
        assert_eq!(plan.rung.name, "b");
        assert_eq!(plan.state.failed, ["a"]);
    }

    #[test]
    fn a_new_build_forgets_what_failed_on_the_old_one() {
        let plan = plan(THREE, after("b", &["a"]), "newer build");
        assert_eq!(plan.rung.name, "a");
        assert!(plan.state.failed.is_empty());
        assert_eq!(plan.state.build, "newer build");
    }

    #[test]
    fn when_every_rung_has_failed_the_list_starts_over() {
        let plan = plan(THREE, after("c", &["a", "b"]), "build");
        assert!(plan.wrapped);
        assert_eq!(plan.rung.name, "a");
        assert!(plan.state.failed.is_empty());
        assert!(plan.note().contains("trying them again"));
    }

    #[test]
    fn a_single_rung_ladder_stays_on_its_rung() {
        let one = &THREE[..1];
        let plan = plan(one, after("a", &[]), "build");
        assert_eq!(plan.rung.name, "a");
        assert!(plan.wrapped);
    }

    #[test]
    fn a_failure_is_recorded_once() {
        let plan = plan(THREE, after("a", &["a"]), "build");
        assert_eq!(plan.state.failed, ["a"]);
    }

    #[test]
    fn a_name_the_ladder_no_longer_has_is_ignored() {
        let plan = plan(THREE, after("a", &["removed"]), "build");
        assert_eq!(plan.rung.name, "b");
    }

    #[test]
    fn every_platform_has_a_rung_and_unique_names() {
        assert!(!LADDER.is_empty());
        for (at, rung) in LADDER.iter().enumerate() {
            assert!(!rung.backends.is_empty(), "{} probes nothing", rung.name);
            assert!(LADDER[..at].iter().all(|other| other.name != rung.name));
        }
    }
}
