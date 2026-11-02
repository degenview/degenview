//! The Windows strategy: replace the running `degenview.exe` in place.
//!
//! Windows will not let a running program be overwritten or deleted, but it does let it be
//! renamed. So the swap moves the running exe into the staging folder and puts the new one where
//! it was; the old process keeps running from its renamed file until it quits.
//!
//! There is no shell helper to reopen the app: `apply_and_relaunch` starts the new exe straight
//! away with `--relaunch-after <pid>`, and `wait_for_predecessor` (the first thing the new process
//! does) holds it back until that pid has exited, so two instances never share the database.
//!
//! Unlike the macOS bundle, nothing here is checked against a code signature: the Windows build
//! is not Authenticode-signed. Trust comes from the signed release tag, the immutable release and
//! the SHA-256 GitHub published for the zip (`docs/agent-guides/releases-updates.md`).
//!
//! The logic is compiled on every platform so its tests run everywhere; only the process calls
//! are Windows-only.
#![cfg_attr(not(windows), allow(dead_code))]

use super::{Progress, STAGING_NAME, Stage, forwarded_env, is_writable, verify};
use crate::service::update_check::{Release, ReleaseAsset};
use crate::service::{tag_signature, update_archive, update_download};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Passed to the relaunched exe, followed by the pid of the process it replaces.
const RELAUNCH_FLAG: &str = "--relaunch-after";
/// How long the new process waits for its predecessor to exit before carrying on regardless: as
/// long as the macOS helper, since the old app may still be asking about unsaved scripts.
const RELAUNCH_WAIT_MS: u32 = 60_000;

/// A downloaded, verified update waiting for the user's restart.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Prepared {
    exe: PathBuf,
    staging: PathBuf,
    new_exe: PathBuf,
}

#[cfg(test)]
impl Prepared {
    /// A stand-in for tests of code that only passes a `Prepared` along.
    pub(crate) fn for_test() -> Self {
        Self {
            exe: PathBuf::from(r"C:\Users\me\DegenView\degenview.exe"),
            staging: PathBuf::from(r"C:\Users\me\DegenView\.DegenView-update"),
            new_exe: PathBuf::from(r"C:\Users\me\DegenView\.DegenView-update\new\degenview.exe"),
        }
    }
}

/// The running executable, when it can be replaced in place.
pub fn install_target() -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .and_then(|exe| exe.canonicalize())
        .map_err(|error| error.to_string())?;
    // `canonicalize` gives a `\\?\C:\…` path on Windows, which never `starts_with` the plain
    // `C:\…` manifest dir: both are brought to the plain form before they are compared.
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source_root = source_root
        .canonicalize()
        .unwrap_or_else(|_| source_root.to_path_buf());
    target_for(&plain(&exe), &plain(&source_root))
}

/// `path` without the `\\?\` prefix `canonicalize` adds on Windows (a `\\?\UNC\` path keeps it).
fn plain(path: &Path) -> PathBuf {
    match path.to_str().and_then(|path| path.strip_prefix(r"\\?\")) {
        Some(rest) if !rest.starts_with(r"UNC\") => PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
}

/// `exe` itself, unless it sits in a folder an update cannot write to or is a development build
/// (`cargo run` under `source_root`), which must never replace itself with a release.
fn target_for(exe: &Path, source_root: &Path) -> Result<PathBuf, String> {
    if exe.starts_with(source_root) {
        return Err("A development build does not update itself.".to_owned());
    }
    let parent = exe.parent().ok_or("The app has no parent folder.")?;
    if !is_writable(parent) {
        return Err(format!(
            "{} is not writable. Move DegenView to a folder you own, such as Documents.",
            parent.display()
        ));
    }
    Ok(exe.to_path_buf())
}

/// Blocking: checks the release tag's signature, then downloads, verifies and unpacks the
/// release's zip next to `exe`. Nothing is replaced yet, and nothing is downloaded for a tag that
/// is not signed by a pinned key.
pub fn prepare(
    exe: &Path,
    release: &Release,
    progress: &dyn Fn(Progress),
) -> Result<Prepared, String> {
    let asset = release
        .asset
        .as_ref()
        .ok_or("This release has no download for this computer.")?;
    progress(Progress::new(Stage::Verifying));
    tag_signature::verify_release_tag(&release.tag).map_err(|error| error.to_string())?;
    let parent = exe.parent().ok_or("The app has no parent folder.")?;
    let staging = parent.join(STAGING_NAME);
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir(&staging)
        .map_err(|error| format!("Could not create {STAGING_NAME}: {error}"))?;
    prepare_in(&staging, exe, asset, progress).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&staging);
    })
}

fn prepare_in(
    staging: &Path,
    exe: &Path,
    asset: &ReleaseAsset,
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
    let new_exe = update_archive::extract_exe(&bytes, &staging.join("new"))?;
    drop(bytes);
    update_archive::check_exe(&new_exe)?;
    Ok(Prepared {
        exe: exe.to_path_buf(),
        staging: staging.to_path_buf(),
        new_exe,
    })
}

/// Swaps the prepared exe in and starts it. On `Ok` the caller must quit now.
pub fn apply_and_relaunch(prepared: &Prepared) -> Result<(), String> {
    swap(prepared)?;
    if let Err(error) = spawn_relaunch(prepared) {
        // Without the new process nobody would reopen the app; the new exe is in place
        // regardless, so tell the user rather than quitting into nothing.
        return Err(format!(
            "Updated, but could not restart: {error}. Reopen DegenView."
        ));
    }
    Ok(())
}

fn swap(prepared: &Prepared) -> Result<(), String> {
    let name = prepared
        .exe
        .file_name()
        .ok_or("The app has no file name.")?;
    let old = prepared.staging.join("old").join(name);
    std::fs::create_dir_all(old.parent().expect("old has a parent"))
        .map_err(|error| error.to_string())?;
    std::fs::rename(&prepared.exe, &old)
        .map_err(|error| format!("Could not move the current app aside: {error}"))?;
    if let Err(error) = std::fs::rename(&prepared.new_exe, &prepared.exe) {
        let _ = std::fs::rename(&old, &prepared.exe);
        return Err(format!("Could not install the new app: {error}"));
    }
    Ok(())
}

/// Environment the relaunched app gets (names compare case-insensitively on Windows). It is
/// started clean, so these are what it needs to find its data and cache folders, plus the
/// non-secret `DEGENVIEW_*` switches. Credentials (`DEGENVIEW_ALPACA_*`, `DEGENVIEW_CMC_API_KEY`, …)
/// must never be added: the relaunched app reads them from its own store again.
const FORWARDED_ENV: &[&str] = &[
    // Where its data and cache folders are.
    "APPDATA",
    "LOCALAPPDATA",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "TEMP",
    "TMP",
    // What anything it starts later (the browser, a file chooser) expects of a Windows process.
    "SystemRoot",
    "windir",
    "SystemDrive",
    "ComSpec",
    "PATH",
    "PATHEXT",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "USERNAME",
    "USERDOMAIN",
    "COMPUTERNAME",
    "OS",
    "PROCESSOR_ARCHITECTURE",
    "NUMBER_OF_PROCESSORS",
    // Non-secret switches of ours.
    "DEGENVIEW_CACHE_DIR",
    "DEGENVIEW_NO_KEYCHAIN",
    "DEGENVIEW_NO_NOTIFICATIONS",
];

/// The new process: the exe, `--relaunch-after <pid>`, and only the `forwarded` environment.
fn relaunch_command(
    exe: &Path,
    pid: u32,
    forwarded: Vec<(OsString, OsString)>,
) -> std::process::Command {
    let mut command = std::process::Command::new(exe);
    command
        .arg(RELAUNCH_FLAG)
        .arg(pid.to_string())
        .env_clear()
        .envs(forwarded);
    command
}

#[cfg(windows)]
fn spawn_relaunch(prepared: &Prepared) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;

    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

    relaunch_command(
        &prepared.exe,
        std::process::id(),
        forwarded_env(std::env::vars_os(), FORWARDED_ENV),
    )
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
    .spawn()
    .map(drop)
}

#[cfg(not(windows))]
fn spawn_relaunch(_: &Prepared) -> std::io::Result<()> {
    Err(std::io::Error::other("unsupported platform"))
}

/// The pid after `--relaunch-after`, when this process was started by an update.
fn predecessor_pid(args: impl Iterator<Item = OsString>) -> Option<u32> {
    let mut args = args.skip(1);
    while let Some(arg) = args.next() {
        if arg == RELAUNCH_FLAG {
            return args.next()?.to_str()?.parse().ok();
        }
    }
    None
}

/// Run first thing at startup: when an update started this process, waits (at most
/// `RELAUNCH_WAIT_MS`) for the one it replaces to exit, so the database is not opened twice.
#[cfg(windows)]
pub fn wait_for_predecessor() {
    use std::ffi::c_void;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn WaitForSingleObject(handle: *mut c_void, millis: u32) -> u32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    const SYNCHRONIZE: u32 = 0x0010_0000;

    let Some(pid) = predecessor_pid(std::env::args_os()) else {
        return;
    };
    // No handle means the process is already gone (or is not ours to wait on).
    let handle = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        return;
    }
    unsafe {
        WaitForSingleObject(handle, RELAUNCH_WAIT_MS);
        CloseHandle(handle);
    }
}

/// Run once the app has started: removes the staging folder (and the previous exe in it) a
/// finished or abandoned update left next to the app. A file still in use stays for next time.
pub fn cleanup_stale() {
    let Ok(exe) = std::env::current_exe().and_then(|exe| exe.canonicalize()) else {
        return;
    };
    if let Some(parent) = exe.parent() {
        let _ = std::fs::remove_dir_all(parent.join(STAGING_NAME));
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::scratch;
    use super::*;
    use std::fs;

    fn env(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs
            .iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect()
    }

    fn prepared_in(root: &Path, old: &str, new: &str) -> Prepared {
        let exe = root.join("degenview.exe");
        fs::write(&exe, old).unwrap();
        let staging = root.join(STAGING_NAME);
        fs::create_dir_all(staging.join("new")).unwrap();
        let new_exe = staging.join("new").join("degenview.exe");
        fs::write(&new_exe, new).unwrap();
        Prepared {
            exe,
            staging,
            new_exe,
        }
    }

    #[test]
    fn the_swap_installs_the_new_exe_and_keeps_the_old_one() {
        let root = scratch("exe-swap");
        let prepared = prepared_in(&root, "old", "new");
        swap(&prepared).unwrap();
        assert_eq!(fs::read_to_string(&prepared.exe).unwrap(), "new");
        assert_eq!(
            fs::read_to_string(prepared.staging.join("old").join("degenview.exe")).unwrap(),
            "old"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_failed_swap_puts_the_old_exe_back() {
        let root = scratch("exe-rollback");
        let mut prepared = prepared_in(&root, "old", "new");
        prepared.new_exe = root.join("missing.exe");
        assert!(swap(&prepared).is_err());
        assert_eq!(fs::read_to_string(&prepared.exe).unwrap(), "old");
        let _ = fs::remove_dir_all(root);
    }

    /// The whole point of the design: a running exe can be renamed. Only Windows has that
    /// behaviour to prove (Unix lets any file be replaced), so only Windows runs it.
    #[cfg(windows)]
    #[test]
    fn a_running_exe_can_be_swapped_out() {
        let root = scratch("exe-running");
        let prepared = prepared_in(&root, "", "new");
        // `ping` is a stock Windows exe that stays up for as long as asked.
        let ping = Path::new(&std::env::var_os("SystemRoot").unwrap()).join(r"System32\ping.exe");
        fs::copy(ping, &prepared.exe).unwrap();
        let mut child = std::process::Command::new(&prepared.exe)
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let swapped = swap(&prepared);
        let _ = child.kill();
        let _ = child.wait();
        swapped.unwrap();
        assert_eq!(fs::read_to_string(&prepared.exe).unwrap(), "new");
        assert!(prepared.staging.join("old").join("degenview.exe").is_file());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_development_build_or_unwritable_folder_is_not_installable() {
        let root = scratch("exe-target");
        let exe = root.join("degenview.exe");
        assert_eq!(target_for(&exe, Path::new("/elsewhere")), Ok(exe.clone()));
        // Under the source tree: `cargo run`.
        assert!(target_for(&exe, &root).is_err());
        assert!(target_for(&root.join("missing").join("degenview.exe"), Path::new("/x")).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_verbatim_prefix_is_stripped_so_paths_compare() {
        assert_eq!(
            plain(Path::new(r"\\?\C:\src\target\debug\degenview.exe")),
            Path::new(r"C:\src\target\debug\degenview.exe")
        );
        assert_eq!(
            plain(Path::new(r"\\?\UNC\server\share\a.exe")),
            Path::new(r"\\?\UNC\server\share\a.exe")
        );
        assert_eq!(plain(Path::new("/a/b")), Path::new("/a/b"));
    }

    /// The test binary lives under `target/` in the source tree, so it is a development build on
    /// every platform. On Windows this is the check that the `\\?\` handling really lines up.
    #[test]
    fn the_running_test_binary_is_a_development_build() {
        let error = install_target().unwrap_err();
        assert!(error.contains("development build"), "{error}");
    }

    #[test]
    fn credentials_are_never_forwarded_and_names_match_in_any_case() {
        let vars = env(&[
            ("AppData", r"C:\Users\me\AppData\Roaming"),
            ("LOCALAPPDATA", r"C:\Users\me\AppData\Local"),
            ("SystemRoot", r"C:\Windows"),
            ("DEGENVIEW_CACHE_DIR", r"C:\cache"),
            ("DEGENVIEW_ALPACA_KEY_ID", "canary-id"),
            ("DEGENVIEW_ALPACA_SECRET_KEY", "canary-secret"),
            ("DEGENVIEW_CMC_API_KEY", "canary-cmc"),
            ("AWS_SECRET_ACCESS_KEY", "canary-aws"),
            ("GITHUB_TOKEN", "canary-token"),
            ("PYTHONPATH", "x"),
        ]);
        let forwarded = forwarded_env(vars.into_iter(), FORWARDED_ENV);
        let names: Vec<_> = forwarded
            .iter()
            .map(|(name, _)| name.to_str().unwrap())
            .collect();
        // `AppData` only matches case-insensitively, which is how Windows names behave.
        let expected = if cfg!(windows) {
            vec![
                "AppData",
                "LOCALAPPDATA",
                "SystemRoot",
                "DEGENVIEW_CACHE_DIR",
            ]
        } else {
            vec!["LOCALAPPDATA", "SystemRoot", "DEGENVIEW_CACHE_DIR"]
        };
        assert_eq!(names, expected);
    }

    #[test]
    fn the_relaunch_carries_the_pid_and_no_secrets() {
        let command = relaunch_command(
            Path::new(r"C:\Apps\degenview.exe"),
            4242,
            env(&[("LOCALAPPDATA", r"C:\Users\me\AppData\Local")]),
        );
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, ["--relaunch-after", "4242"]);
        let envs: Vec<_> = command.get_envs().collect();
        assert_eq!(envs.len(), 1, "{envs:?}");
        assert_eq!(command.get_program(), r"C:\Apps\degenview.exe");
    }

    #[test]
    fn the_predecessor_pid_is_read_from_the_arguments() {
        let args = |list: &[&str]| list.iter().map(OsString::from).collect::<Vec<_>>();
        let pid = |list: &[&str]| predecessor_pid(args(list).into_iter());
        assert_eq!(
            pid(&["degenview.exe", "--relaunch-after", "4242"]),
            Some(4242)
        );
        assert_eq!(pid(&["degenview.exe"]), None);
        assert_eq!(pid(&["degenview.exe", "--relaunch-after"]), None);
        assert_eq!(pid(&["degenview.exe", "--relaunch-after", "soon"]), None);
        // The program name is never an argument.
        assert_eq!(pid(&["--relaunch-after", "4242"]), None);
    }
}
