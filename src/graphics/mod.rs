//! Which graphics backends the app starts with, and what happens when a start dies there.
//!
//! The app used to leave the choice to egui-wgpu's default, which probes Vulkan, DirectX 12 and GL
//! alike. A driver (or a layer an overlay injects) that faults during that probe kills the process
//! without a panic, a log line or a window, and the next launch does the same. So the choice is
//! ours, per platform (`ladder::LADDER`), and it heals itself:
//!
//! - `start` runs the whole start. Each try reads `graphics.json`; a start that left its rung
//!   marked as in progress never drew a frame, so that rung is skipped from now on (for this build).
//! - A try that fails cleanly before the renderer exists (no adapter, no surface) moves on to the
//!   next rung at once, in the same launch, instead of showing an error.
//! - `confirm_started` runs when the first frame has been presented and clears the mark.
//!
//! `WGPU_BACKEND` still wins and bypasses all of it (the memory is neither read nor changed), so a
//! reporter can try each backend by hand.
//! Deleting `graphics.json` starts the list over.

mod ladder;
mod state;

use std::path::Path;

use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup};
use eframe::wgpu::Backends;

use ladder::Rung;

/// The backends this start allows, and the line `startup.log` gets about the choice.
pub struct Selection {
    backends: Backends,
    note: String,
    /// False when `WGPU_BACKEND` chose: another try would choose the same.
    can_fall_back: bool,
}

impl Selection {
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

/// Runs `run` with the backends for this start, and again with the next rung while it fails before
/// the renderer is ready. A try that never returns (a crash or a hang in the driver) is handled by
/// the next launch: see the module comment.
pub fn start<T, E>(run: impl FnMut(&Selection) -> Outcome<T, E>) -> Result<T, E> {
    start_in(
        crate::paths::data_dir().as_deref(),
        Backends::from_env(),
        ladder::LADDER,
        crate::build_info::BUILD_ID,
        run,
    )
}

/// Clears the in-progress mark: this start presented a frame, so its rung works.
pub fn confirm_started() {
    if Backends::from_env().is_some() {
        return;
    }
    if let Some(dir) = crate::paths::data_dir() {
        confirm_in(&dir);
    }
}

fn start_in<T, E>(
    dir: Option<&Path>,
    from_env: Option<Backends>,
    ladder: &[Rung],
    build: &str,
    mut run: impl FnMut(&Selection) -> Outcome<T, E>,
) -> Result<T, E> {
    let mut tries = 0;
    loop {
        let selection = begin_in(dir, from_env, ladder, build);
        tries += 1;
        match run(&selection) {
            Outcome::NoRenderer(_) if selection.can_fall_back && tries < ladder.len() => {}
            Outcome::NoRenderer(error) => return Err(error),
            Outcome::Started(value) => {
                finish_in(dir, &selection);
                return Ok(value);
            }
            Outcome::Failed(error) => {
                finish_in(dir, &selection);
                return Err(error);
            }
        }
    }
}

/// The renderer was ready, so the rung works whatever else went wrong. A run with `WGPU_BACKEND`
/// proves nothing about the rungs: it leaves the memory as it found it.
fn finish_in(dir: Option<&Path>, selection: &Selection) {
    if let (Some(dir), true) = (dir, selection.can_fall_back) {
        confirm_in(dir);
    }
}

fn begin_in(
    dir: Option<&Path>,
    from_env: Option<Backends>,
    ladder: &[Rung],
    build: &str,
) -> Selection {
    if let Some(backends) = from_env {
        return Selection {
            backends,
            note: format!("graphics backends: {backends:?} (WGPU_BACKEND)"),
            can_fall_back: false,
        };
    }
    let plan = ladder::plan(ladder, dir.map(state::load).unwrap_or_default(), build);
    if let Some(dir) = dir {
        state::save(dir, &plan.state);
    }
    Selection {
        backends: plan.rung.backends,
        note: plan.note(),
        can_fall_back: true,
    }
}

fn confirm_in(dir: &Path) {
    let mut remembered = state::load(dir);
    if remembered.attempting.take().is_some() {
        state::save(dir, &remembered);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let dir = std::env::temp_dir().join(format!("dv-begin-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_start_that_never_confirms_makes_the_next_one_fall_back() {
        let dir = scratch("fallback");
        let first = begin_in(Some(&dir), None, TWO, "build");
        assert_eq!(first.backends, Backends::DX12);
        // No `confirm_in`: the process died in the driver.
        let second = begin_in(Some(&dir), None, TWO, "build");
        assert_eq!(second.backends, Backends::GL);
        assert!(second.note().contains("did not start on this build: first"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_confirmed_start_keeps_its_backend_next_time() {
        let dir = scratch("confirmed");
        begin_in(Some(&dir), None, TWO, "build");
        confirm_in(&dir);
        assert_eq!(
            begin_in(Some(&dir), None, TWO, "build").backends,
            Backends::DX12
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_fallback_stays_after_it_is_confirmed() {
        let dir = scratch("sticks");
        begin_in(Some(&dir), None, TWO, "build");
        begin_in(Some(&dir), None, TWO, "build");
        confirm_in(&dir);
        assert_eq!(
            begin_in(Some(&dir), None, TWO, "build").backends,
            Backends::GL
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_environment_wins_and_leaves_the_memory_alone() {
        let dir = scratch("env");
        let selection = begin_in(Some(&dir), Some(Backends::VULKAN), TWO, "build");
        assert_eq!(selection.backends, Backends::VULKAN);
        assert!(selection.note().contains("WGPU_BACKEND"));
        assert!(!dir.exists());
    }

    #[test]
    fn without_a_data_directory_the_first_rung_is_used() {
        assert_eq!(begin_in(None, None, TWO, "build").backends, Backends::DX12);
    }

    /// Runs `start_in` with a closure that answers from `outcomes`, and says which backends it saw.
    fn started_with(
        dir: Option<&Path>,
        from_env: Option<Backends>,
        outcomes: Vec<Outcome<&'static str, &'static str>>,
    ) -> (Result<&'static str, &'static str>, Vec<Backends>) {
        let mut outcomes = outcomes.into_iter();
        let mut seen = Vec::new();
        let result = start_in(dir, from_env, TWO, "build", |selection| {
            seen.push(selection.backends);
            outcomes.next().expect("no more tries than answers")
        });
        (result, seen)
    }

    #[test]
    fn a_start_without_a_renderer_tries_the_next_rung_at_once() {
        let dir = scratch("no-renderer");
        let (result, seen) = started_with(
            Some(&dir),
            None,
            vec![Outcome::NoRenderer("no adapter"), Outcome::Started("ran")],
        );
        assert_eq!(result, Ok("ran"));
        assert_eq!(seen, [Backends::DX12, Backends::GL]);
        let remembered = state::load(&dir);
        assert_eq!(remembered.failed, ["first"]);
        assert_eq!(remembered.attempting, None, "the working rung is confirmed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn when_every_rung_has_no_renderer_the_last_error_is_returned() {
        let dir = scratch("none-works");
        let (result, seen) = started_with(
            Some(&dir),
            None,
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
        let (result, seen) = started_with(Some(&dir), None, vec![Outcome::Failed("app error")]);
        assert_eq!(result, Err("app error"));
        assert_eq!(seen, [Backends::DX12]);
        assert_eq!(state::load(&dir).attempting, None);
        assert_eq!(
            begin_in(Some(&dir), None, TWO, "build").backends,
            Backends::DX12
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_run_with_the_environment_choice_keeps_an_earlier_failure_on_record() {
        let dir = scratch("env-keeps-mark");
        // An earlier start died on the first rung and left its mark.
        begin_in(Some(&dir), None, TWO, "build");
        let (result, _) = started_with(
            Some(&dir),
            Some(Backends::GL),
            vec![Outcome::Started("ran")],
        );
        assert_eq!(result, Ok("ran"));
        assert_eq!(state::load(&dir).attempting.as_deref(), Some("first"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_environment_choice_is_tried_once() {
        let (result, seen) = started_with(
            None,
            Some(Backends::VULKAN),
            vec![Outcome::NoRenderer("no adapter")],
        );
        assert_eq!(result, Err("no adapter"));
        assert_eq!(seen, [Backends::VULKAN]);
    }

    #[test]
    fn the_selection_limits_the_instance_and_keeps_the_rest_of_the_configuration() {
        let selection = begin_in(None, None, TWO, "build");
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
