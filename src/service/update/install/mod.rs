//! Replaces the running app with a newer release: the `.app` bundle on macOS (`app_bundle`), the
//! one executable on Windows and Linux (`portable_binary`). The shared parts live here.
//!
//! What follows describes the macOS bundle strategy; the portable one has its own header.
//!
//! Two steps so nothing on disk changes until the user says so: `prepare` downloads the release
//! zip, checks its SHA-256 against the digest GitHub published for it, unpacks it, sanity-checks
//! the bundle and verifies its Developer ID signature against the running app's Team ID;
//! `apply` swaps the bundle in place when the user asks to restart, and `relaunch` starts a detached
//! shell that waits for this process to exit and reopens the app (called once exit is certain, so a
//! quit the user calls off leaves no second app behind). The previous bundle is kept in the staging folder until the
//! new version starts and calls `cleanup_stale`, so a build that never launches can be restored by
//! hand.
//!
//! Everything lives in a hidden folder next to the app (same volume, so the swap is two atomic
//! renames); an app in a folder the user cannot write to simply is not installable.

use crate::service::update::check::ReleaseAsset;
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::path::Path;

// The macOS strategy: replace the `.app` bundle. Compiled on the other Unixes too, where its
// install target is refused, so the rest of the crate has one set of functions to call.
#[cfg(not(any(windows, target_os = "linux")))]
mod app_bundle;
#[cfg(not(any(windows, target_os = "linux")))]
pub use app_bundle::{Prepared, apply, cleanup_stale, install_target, prepare, relaunch};

// The portable strategy: replace the one executable. Compiled everywhere under test so its logic is
// tested on every platform; it is the strategy only on Windows and Linux.
#[cfg(any(windows, target_os = "linux", test))]
mod portable_binary;
#[cfg(any(windows, target_os = "linux"))]
pub use portable_binary::{
    Prepared, apply, cleanup_stale, install_target, prepare, relaunch, wait_for_predecessor,
};

#[cfg(any(windows, test))]
mod installed_version;
#[cfg(windows)]
pub use installed_version::sync_installed_version;

/// Nothing to wait for on macOS: its helper waits for the old process itself, and starts the new
/// app only once it is gone.
#[cfg(not(any(windows, target_os = "linux")))]
pub fn wait_for_predecessor() -> Result<(), String> {
    Ok(())
}

/// Nothing to keep in step outside Windows: only its installer writes an Installed apps entry.
#[cfg(not(windows))]
pub fn sync_installed_version() {}

/// The folder next to the app where an update is downloaded, unpacked and kept until the new
/// version has started.
const STAGING_NAME: &str = ".DegenView-update";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    Verifying,
    Downloading,
    Unpacking,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Verifying => "Checking the release signature…",
            Self::Downloading => "Downloading the new version…",
            Self::Unpacking => "Unpacking and checking it…",
        }
    }
}

/// Where `prepare` has got to: the stage, and how far through the whole job that is (0 to 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress {
    pub stage: Stage,
    pub fraction: f32,
}

/// Share of the overall bar each stage covers: the signature check, the download (by bytes),
/// then unpacking, whose own progress is not measurable.
const VERIFY_END: f32 = 0.05;
const DOWNLOAD_END: f32 = 0.90;

impl Progress {
    pub fn new(stage: Stage) -> Self {
        let fraction = match stage {
            Stage::Verifying => 0.0,
            Stage::Downloading => VERIFY_END,
            Stage::Unpacking => DOWNLOAD_END,
        };
        Self { stage, fraction }
    }

    /// The download after `received` of `total` bytes.
    fn downloading(received: u64, total: u64) -> Self {
        let done = if total == 0 {
            0.0
        } else {
            (received as f64 / total as f64).clamp(0.0, 1.0) as f32
        };
        Self {
            stage: Stage::Downloading,
            fraction: VERIFY_END + (DOWNLOAD_END - VERIFY_END) * done,
        }
    }
}

/// Whether a folder can be created in `dir`. `create_dir` fails if the name exists, and the probe is
/// removed only when this call created it, so a folder that was already there is never touched.
fn is_writable(dir: &Path) -> bool {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let probe = dir.join(format!(
        ".DegenView-write-test-{}-{nanos}",
        std::process::id()
    ));
    if std::fs::create_dir(&probe).is_err() {
        return false;
    }
    let _ = std::fs::remove_dir(&probe);
    true
}

fn verify(bytes: &[u8], asset: &ReleaseAsset) -> Result<(), String> {
    if bytes.len() as u64 != asset.size {
        return Err("The download is incomplete.".to_owned());
    }
    let hash = Sha256::digest(bytes);
    let hex: String = hash.iter().map(|byte| format!("{byte:02x}")).collect();
    if hex != asset.sha256 {
        return Err("The download does not match its published checksum.".to_owned());
    }
    Ok(())
}

/// Moves the installation at `current` to `old`, then `new` into its place, with `rename` doing the
/// moves. If the second move fails the old one is put back; if that fails too, nothing else will
/// restore the app, so the error says where the old copy is and what to do. Both moves are within
/// one folder tree on one volume, so each is atomic, but there is a moment between them with
/// nothing at `current` (a crash there leaves the old copy in the staging folder).
fn swap_paths(
    current: &Path,
    new: &Path,
    old: &Path,
    rename: &dyn Fn(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), String> {
    rename(current, old)
        .map_err(|error| format!("Could not move the current app aside: {error}"))?;
    let Err(error) = rename(new, current) else {
        return Ok(());
    };
    match rename(old, current) {
        Ok(()) => Err(format!("Could not install the new app: {error}")),
        Err(restore) => Err(format!(
            "Could not install the new app: {error}. Putting the old one back failed too \
             ({restore}). It is at {}: move it to {} to get it back.",
            old.display(),
            current.display()
        )),
    }
}

/// Refuses to clear `staging` when it holds the only copy of the app: a swap that failed and could
/// not put the old one back leaves `staging/old` behind with nothing at `target`, and preparing the
/// next attempt starts by deleting the staging folder.
fn keep_only_copy(staging: &Path, target: &Path) -> Result<(), String> {
    let old = staging.join("old");
    if std::fs::symlink_metadata(target).is_err() && std::fs::symlink_metadata(&old).is_ok() {
        return Err(format!(
            "DegenView is missing from {} and the previous version is in {}. Move it back, then \
             try the update again.",
            target.display(),
            old.display()
        ));
    }
    Ok(())
}

// Called by the macOS and Windows strategies; Linux inherits its environment (tests use it everywhere).
#[cfg_attr(target_os = "linux", allow(dead_code))]
fn forwarded_env(
    vars: impl Iterator<Item = (OsString, OsString)>,
    allowed: &[&str],
) -> Vec<(OsString, OsString)> {
    vars.filter(|(name, _)| {
        name.to_str().is_some_and(|name| {
            allowed.iter().any(|allowed| {
                if cfg!(windows) {
                    allowed.eq_ignore_ascii_case(name)
                } else {
                    *allowed == name
                }
            })
        })
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    pub(super) fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "degenview-self-update-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_checksum_mismatch_or_short_body_is_rejected() {
        let asset = |size, sha256: &str| ReleaseAsset {
            name: "a.zip".to_owned(),
            url: String::new(),
            size,
            sha256: sha256.to_owned(),
        };
        // sha256("abc")
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(verify(b"abc", &asset(3, abc)).is_ok());
        assert!(verify(b"abd", &asset(3, abc)).is_err());
        assert!(verify(b"ab", &asset(3, abc)).is_err());
    }

    use std::io;

    /// A `current` app holding "old" and a `new` one holding "new", under a scratch folder.
    fn swap_fixture(name: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
        let root = scratch(name);
        let (current, new) = (root.join("app"), root.join("staging-new"));
        let old = root.join("staging").join("old").join("app");
        fs::write(&current, "old").unwrap();
        fs::write(&new, "new").unwrap();
        fs::create_dir_all(old.parent().unwrap()).unwrap();
        (root, current, new, old)
    }

    #[test]
    fn a_swap_installs_the_new_and_keeps_the_old() {
        let (root, current, new, old) = swap_fixture("swap-ok");
        swap_paths(&current, &new, &old, &|from, to| fs::rename(from, to)).unwrap();
        assert_eq!(fs::read_to_string(&current).unwrap(), "new");
        assert_eq!(fs::read_to_string(&old).unwrap(), "old");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_swap_that_cannot_start_changes_nothing() {
        let (root, current, new, old) = swap_fixture("swap-first");
        let refuse = |_: &Path, _: &Path| Err(io::Error::other("busy"));
        let error = swap_paths(&current, &new, &old, &refuse).unwrap_err();
        assert!(error.contains("move the current app aside"), "{error}");
        assert_eq!(fs::read_to_string(&current).unwrap(), "old");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_failed_second_move_puts_the_old_one_back() {
        let (root, current, new, old) = swap_fixture("swap-second");
        // The new copy has gone missing: only the move into place fails.
        fs::remove_file(&new).unwrap();
        let error = swap_paths(&current, &new, &old, &|from, to| fs::rename(from, to)).unwrap_err();
        assert!(
            error.starts_with("Could not install the new app"),
            "{error}"
        );
        assert!(!error.contains("Putting the old one back"), "{error}");
        assert_eq!(fs::read_to_string(&current).unwrap(), "old");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_failed_put_back_says_where_the_old_copy_is() {
        let (root, current, new, old) = swap_fixture("swap-both");
        let rename = |from: &Path, to: &Path| {
            if from == new {
                Err(io::Error::other("no space"))
            } else if to == current && from == old {
                Err(io::Error::other("denied"))
            } else {
                fs::rename(from, to)
            }
        };
        let error = swap_paths(&current, &new, &old, &rename).unwrap_err();
        assert!(
            error.contains("no space") && error.contains("denied"),
            "{error}"
        );
        assert!(error.contains(&old.display().to_string()), "{error}");
        assert!(error.contains(&current.display().to_string()), "{error}");
        assert!(!current.exists());
        assert_eq!(fs::read_to_string(&old).unwrap(), "old");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_only_copy_in_the_staging_folder_is_never_cleared() {
        let (root, current, new, old) = swap_fixture("keep-only");
        let staging = root.join("staging");
        // Healthy: the app is where it belongs, so staging is disposable.
        assert!(keep_only_copy(&staging, &current).is_ok());
        // A swap that lost the app: it is missing and the old copy sits in staging.
        let _ = new;
        fs::rename(&current, &old).unwrap();
        let error = keep_only_copy(&staging, &current).unwrap_err();
        assert!(error.contains("previous version"), "{error}");
        // No staging folder at all: nothing to protect.
        assert!(keep_only_copy(&root.join("nothing"), &current).is_ok());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_read_only_folder_is_not_writable() {
        let root = scratch("writable");
        assert!(is_writable(&root));
        assert!(!is_writable(&root.join("missing")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_write_probe_leaves_no_trace_and_touches_nothing_else() {
        let root = scratch("probe");
        let bystander = root.join(".DegenView-write-test-keep");
        fs::create_dir(&bystander).unwrap();
        assert!(is_writable(&root));
        assert!(bystander.is_dir());
        let leftovers = fs::read_dir(&root).unwrap().count();
        assert_eq!(leftovers, 1, "only the bystander remains");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_bar_only_moves_forward_across_the_stages() {
        let start = Progress::new(Stage::Verifying).fraction;
        let begin = Progress::new(Stage::Downloading).fraction;
        let half = Progress::downloading(50, 100).fraction;
        let end = Progress::downloading(100, 100).fraction;
        let unpack = Progress::new(Stage::Unpacking).fraction;
        assert!(start < begin && begin < half && half < end);
        assert_eq!(end, unpack);
        assert_eq!(Progress::downloading(500, 100).fraction, end);
        assert_eq!(Progress::downloading(0, 0).fraction, begin);
    }
}
