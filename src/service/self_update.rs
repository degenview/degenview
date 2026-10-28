//! Replaces the running macOS app bundle with a newer release.
//!
//! Two steps so nothing on disk changes until the user says so: `prepare` downloads the release
//! zip, checks its SHA-256 against the digest GitHub published for it, unpacks it, sanity-checks
//! the bundle and verifies its Developer ID signature against the running app's Team ID;
//! `apply_and_relaunch` swaps the bundle in place, then a detached shell waits for this
//! process to exit and reopens the app. The previous bundle is kept in the staging folder until the
//! new version starts and calls `cleanup_stale`, so a build that never launches can be restored by
//! hand.
//!
//! Everything lives in a hidden folder next to the app (same volume, so the swap is two atomic
//! renames); an app in a folder the user cannot write to simply is not installable.

use crate::service::update_check::{Release, ReleaseAsset};
use crate::service::{code_signature, tag_signature, update_archive, update_download};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

const STAGING_NAME: &str = ".DegenView-update";
const APP_NAME: &str = "DegenView.app";
/// How long the relaunch helper waits for this process to exit before giving up.
const RELAUNCH_WAIT_TICKS: u32 = 300;

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

/// A downloaded, verified update waiting for the user's restart.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Prepared {
    app: PathBuf,
    staging: PathBuf,
    new_app: PathBuf,
}

#[cfg(test)]
impl Prepared {
    /// A stand-in for tests of code that only passes a `Prepared` along.
    pub(crate) fn for_test() -> Self {
        Self {
            app: PathBuf::from("/Applications/DegenView.app"),
            staging: PathBuf::from("/Applications/.DegenView-update"),
            new_app: PathBuf::from("/Applications/.DegenView-update/new/DegenView.app"),
        }
    }
}

/// The `.app` bundle this process runs from, when it can be replaced in place.
pub fn install_target() -> Result<PathBuf, String> {
    if !cfg!(target_os = "macos") {
        // A Flatpak installed from a downloaded bundle has no remote, so `flatpak update` finds
        // nothing: the release page is where the next one comes from.
        return Err(if crate::service::sandbox::is_flatpak() {
            "This Flatpak cannot update itself. Download the new .flatpak from the release page."
        } else {
            "Automatic updates are only available on macOS."
        }
        .to_owned());
    }
    let exe = std::env::current_exe()
        .and_then(|exe| exe.canonicalize())
        .map_err(|error| error.to_string())?;
    let app = bundle_for_executable(&exe)
        .ok_or_else(|| "DegenView is not running from an app bundle.".to_owned())?;
    if app
        .components()
        .any(|part| part.as_os_str() == "AppTranslocation")
    {
        return Err("Move DegenView to the Applications folder first.".to_owned());
    }
    let parent = app.parent().ok_or("The app has no parent folder.")?;
    if !is_writable(parent) {
        return Err(format!("{} is not writable.", parent.display()));
    }
    // An update must be signed by the team that signed this app; without one there is nothing to
    // compare against.
    code_signature::team_id(&app)?;
    Ok(app)
}

/// `/…/DegenView.app` for `/…/DegenView.app/Contents/MacOS/DegenView`.
fn bundle_for_executable(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    let is_bundle = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && app.extension()? == "app";
    is_bundle.then(|| app.to_path_buf())
}

fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".DegenView-write-test-{}", std::process::id()));
    let ok = std::fs::create_dir(&probe).is_ok();
    let _ = std::fs::remove_dir(&probe);
    ok
}

/// Blocking: checks the release tag's signature, then downloads, verifies and unpacks the
/// release's zip next to `app`, whose Developer ID Team ID the new bundle must be signed with. Nothing is replaced yet, and nothing is downloaded for a tag that
/// is not signed by a pinned key.
pub fn prepare(
    app: &Path,
    release: &Release,
    progress: &dyn Fn(Progress),
) -> Result<Prepared, String> {
    let asset = release
        .asset
        .as_ref()
        .ok_or("This release has no download for this Mac.")?;
    progress(Progress::new(Stage::Verifying));
    tag_signature::verify_release_tag(&release.tag).map_err(|error| error.to_string())?;
    let team_id = code_signature::team_id(app)?;
    let parent = app.parent().ok_or("The app has no parent folder.")?;
    let staging = parent.join(STAGING_NAME);
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir(&staging)
        .map_err(|error| format!("Could not create {STAGING_NAME}: {error}"))?;
    prepare_in(&staging, app, asset, &team_id, progress).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&staging);
    })
}

fn prepare_in(
    staging: &Path,
    app: &Path,
    asset: &ReleaseAsset,
    team_id: &str,
    progress: &dyn Fn(Progress),
) -> Result<Prepared, String> {
    progress(Progress::new(Stage::Downloading));
    // One report per percent: the callback takes a lock and wakes the UI.
    let reported = std::cell::Cell::new(u64::MAX);
    let bytes = update_download::get(&asset.url, asset.size, &|received| {
        let percent = received.saturating_mul(100) / asset.size.max(1);
        if reported.replace(percent) != percent {
            progress(Progress::downloading(received, asset.size));
        }
    })?;
    verify(&bytes, asset)?;

    progress(Progress::new(Stage::Unpacking));
    update_archive::precheck(&bytes)?;
    let zip = staging.join("update.zip");
    std::fs::write(&zip, &bytes)
        .map_err(|error| format!("Could not save the download: {error}"))?;
    drop(bytes);
    let extracted = staging.join("new");
    update_archive::extract(&zip, &extracted)?;
    let _ = std::fs::remove_file(&zip);
    let new_app = extracted.join(APP_NAME);
    update_archive::check_bundle(&new_app)?;
    code_signature::verify_bundle(&new_app, &new_app.join(update_archive::HELPER_APP), team_id)?;
    Ok(Prepared {
        app: app.to_path_buf(),
        staging: staging.to_path_buf(),
        new_app,
    })
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

/// Swaps the prepared bundle in and schedules the reopen. On `Ok` the caller must quit now.
pub fn apply_and_relaunch(prepared: &Prepared) -> Result<(), String> {
    swap(prepared)?;
    if let Err(error) = spawn_relaunch(prepared) {
        // Without the helper nobody would reopen the app; the new bundle is in place regardless,
        // so tell the user rather than quitting into nothing.
        return Err(format!(
            "Updated, but could not restart: {error}. Reopen DegenView."
        ));
    }
    Ok(())
}

fn swap(prepared: &Prepared) -> Result<(), String> {
    let old = prepared.staging.join("old").join(APP_NAME);
    std::fs::create_dir_all(old.parent().expect("old has a parent"))
        .map_err(|error| error.to_string())?;
    std::fs::rename(&prepared.app, &old)
        .map_err(|error| format!("Could not move the current app aside: {error}"))?;
    if let Err(error) = std::fs::rename(&prepared.new_app, &prepared.app) {
        let _ = std::fs::rename(&old, &prepared.app);
        return Err(format!("Could not install the new app: {error}"));
    }
    Ok(())
}

/// Environment the relaunched app gets: `open` otherwise starts it clean, which would drop a dev
/// run's data/cache directories. Only names that are safe to show in a process listing belong
/// here; credentials (`DEGENVIEW_ALPACA_*`, `DEGENVIEW_CMC_API_KEY`, …) must never be added: the
/// relaunched app reads them from the Keychain again.
const FORWARDED_ENV: &[&str] = &[
    "HOME",
    "DEGENVIEW_CACHE_DIR",
    "DEGENVIEW_NO_KEYCHAIN",
    "DEGENVIEW_NO_NOTIFICATIONS",
];

/// What the helper needs itself (it runs `sh`, `kill`, `sleep`, `open`).
const HELPER_ENV: &[&str] = &["HOME", "PATH", "USER", "TMPDIR"];

fn forwarded_env(
    vars: impl Iterator<Item = (OsString, OsString)>,
    allowed: &[&str],
) -> Vec<(OsString, OsString)> {
    vars.filter(|(name, _)| name.to_str().is_some_and(|name| allowed.contains(&name)))
        .collect()
}

/// The detached helper: waits for `pid` to exit (at most `RELAUNCH_WAIT_TICKS` × 0.2 s), then
/// reopens `app`. Its arguments are visible to every user on the Mac, and so are only `forwarded`
/// names; its own environment is cleared down to `helper_env`.
#[cfg(unix)]
fn relaunch_command(
    app: &Path,
    pid: u32,
    forwarded: &[(OsString, OsString)],
    helper_env: Vec<(OsString, OsString)>,
) -> std::process::Command {
    let script = format!(
        "pid=$1; shift; i=0; \
         while kill -0 \"$pid\" 2>/dev/null && [ $i -lt {RELAUNCH_WAIT_TICKS} ]; do sleep 0.2; i=$((i+1)); done; \
         exec \"$@\""
    );
    let mut command = std::process::Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(script)
        .arg("sh")
        .arg(pid.to_string())
        .args(["/usr/bin/open", "-n"]);
    for (name, value) in forwarded {
        let mut assignment = name.clone();
        assignment.push("=");
        assignment.push(value);
        command.arg("--env").arg(assignment);
    }
    command.arg(app).env_clear().envs(helper_env);
    command
}

#[cfg(unix)]
fn spawn_relaunch(prepared: &Prepared) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;

    relaunch_command(
        &prepared.app,
        std::process::id(),
        &forwarded_env(std::env::vars_os(), FORWARDED_ENV),
        forwarded_env(std::env::vars_os(), HELPER_ENV),
    )
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .process_group(0)
    .spawn()
    .map(drop)
}

#[cfg(not(unix))]
fn spawn_relaunch(_: &Prepared) -> std::io::Result<()> {
    Err(std::io::Error::other("unsupported platform"))
}

/// Run once the app has started: removes the staging folder (and the previous bundle in it) a
/// finished or abandoned update left next to the app.
pub fn cleanup_stale() {
    let Ok(exe) = std::env::current_exe().and_then(|exe| exe.canonicalize()) else {
        return;
    };
    if let Some(parent) = bundle_for_executable(&exe)
        .as_deref()
        .and_then(Path::parent)
    {
        let _ = std::fs::remove_dir_all(parent.join(STAGING_NAME));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::update_archive::testing::fake_app;
    use std::fs;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "degenview-self-update-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_bundle_is_found_from_its_executable() {
        assert_eq!(
            bundle_for_executable(Path::new(
                "/Applications/DegenView.app/Contents/MacOS/DegenView"
            )),
            Some(PathBuf::from("/Applications/DegenView.app"))
        );
        assert_eq!(
            bundle_for_executable(Path::new("/repo/target/release/degenview")),
            None
        );
        assert_eq!(
            bundle_for_executable(Path::new("/x/Tools/Contents/MacOS/degenview")),
            None
        );
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
    fn the_swap_installs_the_new_bundle_and_keeps_the_old_one() {
        let root = scratch("swap");
        let app = fake_app(&root, "old");
        let staging = root.join(STAGING_NAME);
        let new_dir = staging.join("new");
        fs::create_dir_all(&new_dir).unwrap();
        let new_app = fake_app(&new_dir, "new");
        let prepared = Prepared {
            app: app.clone(),
            staging: staging.clone(),
            new_app,
        };
        swap(&prepared).unwrap();
        assert_eq!(
            fs::read_to_string(app.join("Contents/MacOS/DegenView")).unwrap(),
            "new"
        );
        assert_eq!(
            fs::read_to_string(
                staging
                    .join("old")
                    .join(APP_NAME)
                    .join("Contents/MacOS/DegenView")
            )
            .unwrap(),
            "old"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_failed_swap_puts_the_old_bundle_back() {
        let root = scratch("rollback");
        let app = fake_app(&root, "old");
        let staging = root.join(STAGING_NAME);
        fs::create_dir_all(&staging).unwrap();
        let prepared = Prepared {
            app: app.clone(),
            staging,
            new_app: root.join("missing.app"),
        };
        assert!(swap(&prepared).is_err());
        assert_eq!(
            fs::read_to_string(app.join("Contents/MacOS/DegenView")).unwrap(),
            "old"
        );
        let _ = fs::remove_dir_all(root);
    }

    /// Hits GitHub: `cargo test -- --ignored the_latest_release_downloads_and_unpacks`.
    #[test]
    #[ignore = "downloads the latest release"]
    #[cfg(target_os = "macos")]
    fn the_latest_release_downloads_and_unpacks() {
        use crate::service::update_check::{Channel, UpdateOutcome, check};
        let UpdateOutcome::Available(release) = check("0.0.1", Channel::Stable) else {
            panic!("no release found");
        };
        if let Some(reason) = release.manual_only {
            // Releases published before immutability was on (v0.1.2 and older) stay manual.
            eprintln!("skipped: the latest release is not installable: {reason}");
            return;
        }
        assert!(
            release.asset.is_some(),
            "a macOS asset for this architecture"
        );
        // The download must be signed by the installed app's team, so this needs a signed install.
        let installed = Path::new("/Applications/DegenView.app");
        let Ok(team_id) = code_signature::team_id(installed) else {
            eprintln!("skipped: no Developer ID signed DegenView in /Applications");
            return;
        };
        let root = scratch("live");
        let app = fake_app(&root, "old");
        let staging = root.join(STAGING_NAME);
        fs::create_dir(&staging).unwrap();
        let asset = release.asset.as_ref().unwrap();
        let prepared = prepare_in(&staging, &app, asset, &team_id, &|_| {}).unwrap();
        assert!(update_archive::check_bundle(&prepared.new_app).is_ok());
        swap(&prepared).unwrap();
        assert_ne!(
            fs::read(app.join("Contents/MacOS/DegenView")).unwrap(),
            b"old"
        );
        let _ = fs::remove_dir_all(root);
    }

    fn env(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs
            .iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect()
    }

    #[test]
    fn credentials_are_never_forwarded() {
        let vars = env(&[
            ("HOME", "/Users/me"),
            ("DEGENVIEW_CACHE_DIR", "/tmp/cache"),
            ("DEGENVIEW_NO_KEYCHAIN", "1"),
            ("DEGENVIEW_NO_NOTIFICATIONS", "1"),
            ("DEGENVIEW_ALPACA_KEY_ID", "canary-id"),
            ("DEGENVIEW_ALPACA_SECRET_KEY", "canary-secret"),
            ("DEGENVIEW_CMC_API_KEY", "canary-cmc"),
            ("DEGENVIEW_CODESIGN_IDENTITY", "canary-identity"),
            ("AWS_SECRET_ACCESS_KEY", "canary-aws"),
        ]);
        let forwarded = forwarded_env(vars.clone().into_iter(), FORWARDED_ENV);
        let names: Vec<_> = forwarded
            .iter()
            .map(|(name, _)| name.to_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "HOME",
                "DEGENVIEW_CACHE_DIR",
                "DEGENVIEW_NO_KEYCHAIN",
                "DEGENVIEW_NO_NOTIFICATIONS"
            ]
        );
        let helper = forwarded_env(vars.into_iter(), HELPER_ENV);
        assert_eq!(helper, env(&[("HOME", "/Users/me")]));
    }

    #[cfg(unix)]
    #[test]
    fn the_helpers_command_line_and_environment_carry_no_secrets() {
        let vars = env(&[
            ("HOME", "/Users/me"),
            ("DEGENVIEW_CACHE_DIR", "/tmp/cache"),
            ("DEGENVIEW_ALPACA_SECRET_KEY", "canary-secret"),
            ("DEGENVIEW_CMC_API_KEY", "canary-cmc"),
        ]);
        let command = relaunch_command(
            Path::new("/Applications/DegenView.app"),
            4242,
            &forwarded_env(vars.clone().into_iter(), FORWARDED_ENV),
            forwarded_env(vars.into_iter(), HELPER_ENV),
        );
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let line = args.join(" ");
        assert!(!line.contains("canary"), "{line}");
        assert!(line.contains("--env HOME=/Users/me"), "{line}");
        assert!(
            line.contains("--env DEGENVIEW_CACHE_DIR=/tmp/cache"),
            "{line}"
        );
        assert!(line.ends_with("/Applications/DegenView.app"), "{line}");
        let envs: Vec<_> = command.get_envs().collect();
        assert!(envs.iter().all(|(name, _)| *name == "HOME"), "{envs:?}");
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
