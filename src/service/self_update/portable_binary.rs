//! The portable strategy (Windows and Linux): replace the running executable in place.
//!
//! Neither OS lets a running program be overwritten in the usual way, but both let it be renamed:
//! the swap moves the running binary into the staging folder and puts the new one where it was,
//! and the old process keeps running from its renamed file until it quits.
//!
//! There is no shell helper to reopen the app: `relaunch` (called once exit is certain) starts the new
//! binary with `--relaunch-after <pid>`, and `wait_for_predecessor` (the first thing the new process
//! does) holds it back until that pid has exited, so two instances never share the database.
//!
//! Unlike the macOS bundle, nothing here is checked against a code signature: neither the Windows
//! nor the Linux build is signed. Trust comes from the signed release tag, the immutable release and
//! the SHA-256 GitHub published for the archive (`docs/agent-guides/releases-updates.md`). Only the
//! unpacking differs per OS: a zip holding `degenview.exe`, or a tarball holding `degenview`.
//!
//! The logic is compiled on every platform so its tests run everywhere; only the process calls
//! are per-OS.
#![cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]

// `forwarded_env` is only called on Windows (Linux inherits its environment); tests use it everywhere.
#[cfg_attr(not(windows), allow(unused_imports))]
use super::forwarded_env;
use super::{Progress, STAGING_NAME, Stage, is_writable, verify};
use crate::service::update_check::{Release, ReleaseAsset};
use crate::service::{tag_signature, update_archive, update_download};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Passed to the relaunched binary, followed by the pid of the process it replaces.
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
            exe: PathBuf::from("/home/me/.local/bin/degenview"),
            staging: PathBuf::from("/home/me/.local/bin/.DegenView-update"),
            new_exe: PathBuf::from("/home/me/.local/bin/.DegenView-update/new/degenview"),
        }
    }
}

/// The running executable, when it can be replaced in place.
pub fn install_target() -> Result<PathBuf, String> {
    // A Flatpak installed from a downloaded bundle has no remote, so `flatpak update` finds
    // nothing: the release page is where the next one comes from.
    if crate::service::sandbox::is_flatpak() {
        return Err(
            "This Flatpak cannot update itself. Download the new .flatpak from the release page."
                .to_owned(),
        );
    }
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
pub(super) fn plain(path: &Path) -> PathBuf {
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
            "{} is not writable. {}",
            parent.display(),
            if cfg!(windows) {
                "Move DegenView to a folder you own, such as Documents."
            } else {
                "Install DegenView in a folder you own, such as ~/.local/bin."
            }
        ));
    }
    Ok(exe.to_path_buf())
}

/// Blocking: checks the release tag's signature, then downloads, verifies and unpacks the
/// release's archive next to `exe`. Nothing is replaced yet, and nothing is downloaded for a tag that
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
    let new_exe = unpack(&bytes, &staging.join("new"))?;
    drop(bytes);
    Ok(Prepared {
        exe: exe.to_path_buf(),
        staging: staging.to_path_buf(),
        new_exe,
    })
}

/// Unpacks the downloaded archive's one binary into `dest_dir` and checks it is an executable for
/// this CPU: a zip holding `degenview.exe` on Windows, a tarball holding `degenview` elsewhere.
/// Two definitions rather than a `cfg!` branch: each calls functions that only exist on its own OS.
#[cfg(windows)]
fn unpack(bytes: &[u8], dest_dir: &Path) -> Result<PathBuf, String> {
    update_archive::precheck(bytes)?;
    let exe = update_archive::extract_exe(bytes, dest_dir)?;
    update_archive::check_exe(&exe)?;
    Ok(exe)
}

#[cfg(not(windows))]
fn unpack(bytes: &[u8], dest_dir: &Path) -> Result<PathBuf, String> {
    let binary = update_archive::extract_tar_gz_binary(bytes, dest_dir)?;
    update_archive::check_elf(&binary)?;
    Ok(binary)
}

/// Swaps the prepared exe in. Nothing is started: the app is still running and may yet decide not
/// to quit (an unsaved-script question); `relaunch` runs once exit is certain.
pub fn apply(prepared: &Prepared) -> Result<(), String> {
    swap(prepared)
}

/// Starts the new exe, which waits for this process to exit before it opens anything. Call it when
/// exit is certain; the exe is already in place, so a failure only means nobody reopens the app.
pub fn relaunch(prepared: &Prepared) -> Result<(), String> {
    spawn_relaunch(prepared)
        .map_err(|error| format!("Updated, but could not restart: {error}. Reopen DegenView."))
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
#[cfg_attr(not(windows), allow(dead_code))]
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

/// What environment the relaunched process gets.
#[cfg_attr(not(windows), allow(dead_code))]
enum Environment {
    /// Only these variables (Windows): started clean, so nothing it was not meant to see is passed.
    Only(Vec<(OsString, OsString)>),
    /// Everything this process has (Linux). A child's environment is not visible to other users, it
    /// is the user's own session, and it must carry `DISPLAY`/`WAYLAND_DISPLAY`/`XDG_RUNTIME_DIR`/
    /// `DBUS_SESSION_BUS_ADDRESS` (the Secret Service) or the new process would start broken; an
    /// allowlist of those would be the riskier choice. Nothing is put on the command line.
    Inherit,
}

/// The new process: the exe and `--relaunch-after <pid>`, with the given environment.
fn relaunch_command(exe: &Path, pid: u32, environment: Environment) -> std::process::Command {
    let mut command = std::process::Command::new(exe);
    command.arg(RELAUNCH_FLAG).arg(pid.to_string());
    if let Environment::Only(variables) = environment {
        command.env_clear().envs(variables);
    }
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
        Environment::Only(forwarded_env(std::env::vars_os(), FORWARDED_ENV)),
    )
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
    .spawn()
    .map(drop)
}

#[cfg(unix)]
fn spawn_relaunch(prepared: &Prepared) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;

    // Its own process group, so closing the terminal this one was started from does not take it
    // down with the old process.
    relaunch_command(&prepared.exe, std::process::id(), Environment::Inherit)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map(drop)
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

/// What the new process says when the one it replaces never exited.
const PREDECESSOR_RUNNING: &str =
    "The previous DegenView is still running. Close it, then open DegenView again.";

/// Run first thing at startup: when an update started this process, waits (at most
/// `RELAUNCH_WAIT_MS`) for the one it replaces to exit, so the database is not opened twice. Fails
/// when that process is still there: the caller must not go on to open the database.
#[cfg(windows)]
pub fn wait_for_predecessor() -> Result<(), String> {
    match predecessor_pid(std::env::args_os()) {
        Some(pid) => wait_for_pid(pid, RELAUNCH_WAIT_MS),
        None => Ok(()),
    }
}

#[cfg(windows)]
fn wait_for_pid(pid: u32, millis: u32) -> Result<(), String> {
    use std::ffi::c_void;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn WaitForSingleObject(handle: *mut c_void, millis: u32) -> u32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const WAIT_OBJECT_0: u32 = 0;
    const WAIT_TIMEOUT: u32 = 0x102;

    // No handle means the process is already gone (or is not ours to wait on).
    let handle = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        return Ok(());
    }
    let result = unsafe { WaitForSingleObject(handle, millis) };
    unsafe { CloseHandle(handle) };
    match result {
        WAIT_OBJECT_0 => Ok(()),
        WAIT_TIMEOUT => Err(PREDECESSOR_RUNNING.to_owned()),
        other => Err(format!(
            "Could not wait for the previous DegenView to exit (wait returned {other:#x})."
        )),
    }
}

/// Run first thing at startup: when an update started this process, waits (at most
/// `RELAUNCH_WAIT_MS`) for the one it replaces to exit, so the database is not opened twice.
#[cfg(unix)]
pub fn wait_for_predecessor() -> Result<(), String> {
    if let Some(pid) = predecessor_pid(std::env::args_os()) {
        wait_until_exited(
            pid,
            std::time::Duration::from_millis(RELAUNCH_WAIT_MS.into()),
        );
    }
    Ok(())
}

/// Polls until `pid` is gone or `timeout` passes. Reads `/proc`, so on a system without one the
/// process counts as already gone and nothing waits.
#[cfg(unix)]
fn wait_until_exited(pid: u32, timeout: std::time::Duration) {
    let started = std::time::Instant::now();
    while process_is_running(pid) && started.elapsed() < timeout {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[cfg(unix)]
fn process_is_running(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| !is_dead(&stat))
}

/// Whether a `/proc/<pid>/stat` line says the process has exited but not yet been reaped (`Z`), or
/// is gone (`X`). The state is the field after the command name, which is in parentheses and may
/// itself contain spaces and parentheses, so it is found from the last `)`.
fn is_dead(stat: &str) -> bool {
    stat.rsplit_once(')')
        .and_then(|(_, rest)| rest.trim_start().chars().next())
        .is_some_and(|state| matches!(state, 'Z' | 'X'))
}

/// Run once the app has started: removes the staging folder (and the previous binary in it) a
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

    /// Runs `start` again while Linux says "Text file busy": another test thread forked while the
    /// file this one just wrote was still open for writing, and the kernel refuses to execute it
    /// until that copy of the descriptor closes. Harmless and brief.
    fn retry_busy<T>(mut start: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
        const ETXTBSY: i32 = 26;
        for _ in 0..50 {
            match start() {
                Err(error) if error.raw_os_error() == Some(ETXTBSY) => {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                other => return other,
            }
        }
        start()
    }

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

    /// A running binary can be swapped out on Linux as well: the old process keeps its open inode.
    #[cfg(unix)]
    #[test]
    fn a_running_binary_can_be_swapped_out() {
        let root = scratch("bin-running");
        let prepared = prepared_in(&root, "", "new");
        fs::copy("/bin/sleep", &prepared.exe).unwrap();
        let mut child =
            retry_busy(|| std::process::Command::new(&prepared.exe).arg("30").spawn()).unwrap();
        let swapped = swap(&prepared);
        let still_running = child.try_wait().unwrap().is_none();
        let _ = child.kill();
        let _ = child.wait();
        swapped.unwrap();
        assert!(still_running, "the old process must outlive the swap");
        assert_eq!(fs::read_to_string(&prepared.exe).unwrap(), "new");
        assert!(prepared.staging.join("old").join("degenview.exe").is_file());
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
            Environment::Only(env(&[("LOCALAPPDATA", r"C:\Users\me\AppData\Local")])),
        );
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, ["--relaunch-after", "4242"]);
        let envs: Vec<_> = command.get_envs().collect();
        assert_eq!(envs.len(), 1, "{envs:?}");
        assert_eq!(command.get_program(), r"C:\Apps\degenview.exe");
    }

    /// The two environments, observed: a script prints `HOME`, which this process has (and which `sh`,
    /// unlike `PATH`, does not invent when it is missing).
    #[cfg(unix)]
    #[test]
    fn the_relaunch_inherits_the_environment_only_where_asked() {
        use std::os::unix::fs::PermissionsExt;

        let root = scratch("relaunch-env");
        let script = root.join("degenview");
        fs::write(&script, "#!/bin/sh\nprintf %s \"${HOME-unset}\"\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        let run = |inherit: bool| {
            let output = retry_busy(|| {
                let environment = if inherit {
                    Environment::Inherit
                } else {
                    Environment::Only(Vec::new())
                };
                relaunch_command(&script, 1, environment).output()
            })
            .unwrap();
            String::from_utf8_lossy(&output.stdout).into_owned()
        };
        let home = std::env::var("HOME").unwrap();
        assert_eq!(run(true), home);
        assert_eq!(run(false), "unset");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_process_that_has_exited_but_not_been_reaped_counts_as_gone() {
        assert!(!is_dead("4242 (degenview) S 1 4242 4242 0 -1 4194560"));
        assert!(!is_dead("4242 (degenview) R 1 4242 4242 0 -1 4194560"));
        assert!(is_dead("4242 (degenview) Z 1 4242 4242 0 -1 4194560"));
        assert!(is_dead("4242 (degenview) X 1 4242 4242 0 -1 4194560"));
        // The command name may hold spaces and parentheses.
        assert!(is_dead("4242 (a b) c) Z 1 4242"));
        assert!(!is_dead("4242 (a b) c) S 1 4242"));
        assert!(!is_dead(""));
        assert!(!is_dead("no parenthesis here"));
    }

    /// Needs `/proc`, so Linux only.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_new_process_waits_for_the_old_one_to_exit() {
        use std::time::{Duration, Instant};

        let mut child = std::process::Command::new("sleep")
            .arg("0.5")
            .spawn()
            .unwrap();
        let started = Instant::now();
        wait_until_exited(child.id(), Duration::from_secs(20));
        let waited = started.elapsed();
        assert!(
            waited >= Duration::from_millis(300),
            "returned after {waited:?}"
        );
        assert!(
            waited < Duration::from_secs(10),
            "returned after {waited:?}"
        );
        child.wait().unwrap();

        // A process that stays gives up at the timeout instead of hanging the start.
        let mut stays = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let started = Instant::now();
        wait_until_exited(stays.id(), Duration::from_millis(300));
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(process_is_running(stays.id()));
        let _ = stays.kill();
        let _ = stays.wait();
        // And one that never existed is gone at once.
        assert!(!process_is_running(u32::MAX));
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

    /// Windows only: a process that outlives the wait is an error, one that has exited is not.
    #[cfg(windows)]
    #[test]
    fn the_new_process_does_not_go_on_while_the_old_one_lives() {
        // `ping` is a stock Windows exe that stays up for as long as asked.
        let mut stays = std::process::Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let waited = wait_for_pid(stays.id(), 300);
        let _ = stays.kill();
        let _ = stays.wait();
        assert_eq!(waited, Err(PREDECESSOR_RUNNING.to_owned()));
        // A pid that is gone (the child just killed and reaped) is fine.
        assert_eq!(wait_for_pid(stays.id(), 300), Ok(()));
    }
}
