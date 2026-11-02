//! Replaces the running macOS app bundle with a newer release.
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

use crate::service::update_check::ReleaseAsset;
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::path::Path;

#[cfg(not(windows))]
mod app_bundle;
#[cfg(not(windows))]
pub use app_bundle::{Prepared, apply, cleanup_stale, install_target, prepare, relaunch};

// Compiled everywhere under test so its logic is tested on every platform; it is the strategy only
// on Windows.
#[cfg(any(windows, test))]
mod portable_exe;
#[cfg(windows)]
pub use portable_exe::{
    Prepared, apply, cleanup_stale, install_target, prepare, relaunch, wait_for_predecessor,
};

/// Nothing to wait for outside Windows: the macOS helper waits for the old process itself, and
/// starts the new app only once it is gone.
#[cfg(not(windows))]
pub fn wait_for_predecessor() -> Result<(), String> {
    Ok(())
}

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

fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".DegenView-write-test-{}", std::process::id()));
    let ok = std::fs::create_dir(&probe).is_ok();
    let _ = std::fs::remove_dir(&probe);
    ok
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

    #[test]
    fn a_read_only_folder_is_not_writable() {
        let root = scratch("writable");
        assert!(is_writable(&root));
        assert!(!is_writable(&root.join("missing")));
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
