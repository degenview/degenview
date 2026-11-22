//! Which graphics backends the app starts with, and what happens when a start dies there.
//!
//! The app used to leave the choice to egui-wgpu's default, which probes Vulkan, DirectX 12 and GL
//! alike. A driver (or a layer an overlay injects) that faults during that probe kills the process
//! without a panic, a log line or a window, and the next launch does the same. So the choice is
//! ours, per platform (`ladder::LADDER`), and it heals itself:
//!
//! - `begin` runs before the renderer starts. It reads `graphics.json`; a start that left its rung
//!   marked as in progress never drew a frame, so that rung is skipped from now on (for this build).
//! - `confirm_started` runs when the first frame has been presented and clears the mark.
//!
//! `WGPU_BACKEND` still wins and bypasses all of it, so a reporter can try each backend by hand.
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

/// Chooses the backends for this start and marks the choice as in progress.
pub fn begin() -> Selection {
    begin_in(
        crate::paths::data_dir().as_deref(),
        Backends::from_env(),
        ladder::LADDER,
        crate::build_info::BUILD_ID,
    )
}

/// Clears the in-progress mark: this start presented a frame, so its rung works.
pub fn confirm_started() {
    if let Some(dir) = crate::paths::data_dir() {
        confirm_in(&dir);
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
        };
    }
    let plan = ladder::plan(ladder, dir.map(state::load).unwrap_or_default(), build);
    if let Some(dir) = dir {
        state::save(dir, &plan.state);
    }
    Selection {
        backends: plan.rung.backends,
        note: plan.note(),
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
