//! Which graphics backends the app starts with, and what happens when a start dies there.
//!
//! The app used to leave the choice to egui-wgpu's default, which probes Vulkan, DirectX 12 and GL
//! alike. A driver (or a layer an overlay injects) that faults during that probe kills the process
//! without a panic, a log line or a window, and the next launch does the same. So the choice is
//! ours, per platform (`ladder::LADDER`), and it heals itself:
//!
//! - `record` is the memory of past starts, a pure state machine; `session` holds it for this
//!   launch and `persist` keeps it in `graphics.json` between launches.
//! - `start` runs the whole start. A try that fails cleanly before the renderer exists (no adapter,
//!   no surface) moves on to the next rung at once; a try that never returns (a crash, a hang) is
//!   seen by the next launch, which skips that rung for this build.
//! - `presentation` decides when a start counts as working: frames that reached the screen.
//!
//! `WGPU_BACKEND` still wins and bypasses all of it (nothing is read or written), so a reporter can
//! try each backend by hand. Deleting `graphics.json` starts the list over.

mod ladder;
mod persist;
mod presentation;
mod record;
mod session;

use std::sync::OnceLock;

use eframe::egui;
use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup};
use eframe::wgpu::Backends;

use crate::crash_report::note;
use presentation::Milestone;
use record::{Attempt, Choice};
use session::Session;

/// The backends one try may use, and the line `startup.log` gets about the choice.
pub struct Selection {
    backends: Backends,
    note: String,
    /// The try's identity; none when `WGPU_BACKEND` chose, since nothing is remembered then.
    attempt: Option<Attempt>,
}

impl Selection {
    fn from_env(backends: Backends) -> Self {
        Self {
            backends,
            note: format!("graphics backends: {backends:?} (WGPU_BACKEND)"),
            attempt: None,
        }
    }

    fn from_choice(choice: Choice<'_>) -> Self {
        Self {
            backends: choice.rung.backends,
            note: choice.note(),
            attempt: Some(choice.attempt),
        }
    }

    /// `config` with the instance limited to the chosen backends; everything else in it (surface
    /// settings, power preference, device limits) stays as eframe set it.
    pub fn configure(&self, mut config: WgpuConfiguration) -> WgpuConfiguration {
        if let WgpuSetup::CreateNew(setup) = &mut config.wgpu_setup {
            setup.instance_descriptor.backends = self.backends;
        }
        config
    }

    pub fn note(&self) -> &str {
        &self.note
    }
}

/// How one try at starting the renderer and the app ended.
pub enum Outcome<T, E> {
    /// The app ran and finished.
    Started(T),
    /// It failed before the renderer was ready (no adapter, no surface): the next backend may work.
    NoRenderer(E),
    /// It failed with the renderer ready: another backend would not help.
    Failed(E),
}

/// Where this launch's backends come from.
enum Source {
    /// `WGPU_BACKEND` chose them: nothing is remembered, nothing is retried.
    Environment(Backends),
    /// The ladder, and what earlier starts taught it.
    Remembered(Box<Session>),
}

impl Source {
    fn choose() -> Self {
        Backends::from_env().map_or_else(
            || {
                Self::Remembered(Box::new(Session::open(
                    crate::paths::data_dir(),
                    ladder::LADDER,
                    crate::build_info::BUILD_ID,
                    session::new_launch_id(),
                )))
            },
            Self::Environment,
        )
    }

    fn select(&self) -> Selection {
        match self {
            Self::Environment(backends) => Selection::from_env(*backends),
            Self::Remembered(session) => Selection::from_choice(session.select()),
        }
    }

    /// The try's rung did not start. Whether another rung is left to try.
    fn fail(&self, selection: &Selection, tries: usize) -> bool {
        match (self, &selection.attempt) {
            (Self::Remembered(session), Some(attempt)) => {
                session.fail(attempt);
                tries < session.rungs()
            }
            _ => false,
        }
    }

    /// The try's renderer was ready: its rung works.
    fn confirm(&self, selection: &Selection) {
        if let (Self::Remembered(session), Some(attempt)) = (self, &selection.attempt) {
            session.confirm(attempt);
        }
    }
}

/// This launch's source, chosen on the first start.
static SOURCE: OnceLock<Source> = OnceLock::new();

/// Runs `run` with the backends for this start, and again with the next rung while it fails before
/// the renderer is ready.
pub fn start<T, E>(run: impl FnMut(&Selection) -> Outcome<T, E>) -> Result<T, E> {
    drive(SOURCE.get_or_init(Source::choose), run)
}

/// Called by every window while it builds its UI: logs the first frames to `startup.log`, and tells
/// the session its start works once frames have really reached the screen.
pub fn observe_frame(ctx: &egui::Context) {
    presentation::observe(ctx, |milestone| match milestone {
        Milestone::Drawn => note("first tab window drawn"),
        Milestone::Presented => note("first frame presented"),
        Milestone::Healthy => {
            if let Some(Source::Remembered(session)) = SOURCE.get() {
                session.confirm_current();
            }
        }
    });
}

fn drive<T, E>(source: &Source, mut run: impl FnMut(&Selection) -> Outcome<T, E>) -> Result<T, E> {
    let mut tries = 0;
    loop {
        let selection = source.select();
        tries += 1;
        match run(&selection) {
            Outcome::NoRenderer(error) => {
                if !source.fail(&selection, tries) {
                    return Err(error);
                }
            }
            // The renderer was ready: the rung works, whatever else went wrong.
            Outcome::Started(value) => {
                source.confirm(&selection);
                return Ok(value);
            }
            Outcome::Failed(error) => {
                source.confirm(&selection);
                return Err(error);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ladder::Rung;

    const TWO: &[Rung] = &[
        Rung {
            name: "first",
            backends: Backends::DX12,
        },
        Rung {
            name: "second",
            backends: Backends::GL,
        },
    ];

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dv-drive-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn remembered(dir: &std::path::Path, launch: &str) -> Source {
        Source::Remembered(Box::new(session(dir, launch)))
    }

    fn session(dir: &std::path::Path, launch: &str) -> Session {
        Session::open(Some(dir.to_owned()), TWO, "build", launch.to_owned())
    }

    /// Drives `source` with a closure that answers from `outcomes`, and says which backends it saw.
    fn driven(
        source: &Source,
        outcomes: Vec<Outcome<&'static str, &'static str>>,
    ) -> (Result<&'static str, &'static str>, Vec<Backends>) {
        let mut outcomes = outcomes.into_iter();
        let mut seen = Vec::new();
        let result = drive(source, |selection| {
            seen.push(selection.backends);
            outcomes.next().expect("no more tries than answers")
        });
        (result, seen)
    }

    #[test]
    fn a_start_without_a_renderer_tries_the_next_rung_at_once() {
        let dir = scratch("no-renderer");
        let source = remembered(&dir, "one");
        let (result, seen) = driven(
            &source,
            vec![Outcome::NoRenderer("no adapter"), Outcome::Started("ran")],
        );
        assert_eq!(result, Ok("ran"));
        assert_eq!(seen, [Backends::DX12, Backends::GL]);
        let remembered = persist::load(&dir);
        assert_eq!(remembered.failed, ["first"]);
        assert_eq!(remembered.attempting, None, "the working rung is confirmed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_fallback_does_not_need_the_file_to_be_writable() {
        let dir = scratch("unwritable");
        std::fs::create_dir_all(dir.join("graphics.json")).unwrap();
        let source = remembered(&dir, "one");
        let (result, seen) = driven(
            &source,
            vec![Outcome::NoRenderer("no adapter"), Outcome::Started("ran")],
        );
        assert_eq!(result, Ok("ran"));
        assert_eq!(seen, [Backends::DX12, Backends::GL]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn when_every_rung_has_no_renderer_the_last_error_is_returned() {
        let dir = scratch("none-works");
        let source = remembered(&dir, "one");
        let (result, seen) = driven(
            &source,
            vec![
                Outcome::NoRenderer("first error"),
                Outcome::NoRenderer("second error"),
            ],
        );
        assert_eq!(result, Err("second error"));
        assert_eq!(seen.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failure_with_the_renderer_ready_is_not_retried_and_keeps_the_rung() {
        let dir = scratch("failed-ready");
        let source = remembered(&dir, "one");
        let (result, seen) = driven(&source, vec![Outcome::Failed("app error")]);
        assert_eq!(result, Err("app error"));
        assert_eq!(seen, [Backends::DX12]);
        assert_eq!(persist::load(&dir).attempting, None);
        assert_eq!(session(&dir, "two").select().rung.name, "first");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_environment_choice_is_tried_once_and_touches_no_memory() {
        let (result, seen) = driven(
            &Source::Environment(Backends::VULKAN),
            vec![Outcome::NoRenderer("no adapter")],
        );
        assert_eq!(result, Err("no adapter"));
        assert_eq!(seen, [Backends::VULKAN]);
    }

    #[test]
    fn the_selection_limits_the_instance_and_keeps_the_rest_of_the_configuration() {
        let selection = Selection::from_env(Backends::DX12);
        let base = eframe::NativeOptions::default().wgpu_options;
        let low_latency = format!("{:?}", base.surface);
        let configured = selection.configure(base);
        match &configured.wgpu_setup {
            WgpuSetup::CreateNew(setup) => {
                assert_eq!(setup.instance_descriptor.backends, Backends::DX12);
            }
            _ => panic!("the default setup creates its own instance"),
        }
        assert_eq!(format!("{:?}", configured.surface), low_latency);
    }
}
