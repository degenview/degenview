//! The app's one view of "is there a newer DegenView": checks GitHub on a schedule, downloads and
//! verifies a release when asked, and hands the result to the tab bar and Settings ▸ About.
//!
//! The network and install work is `service::update_check` / `service::self_update`; this is the
//! stateful bridge that runs them on `work_queue::UPDATES`, remembers the outcome, and exposes it
//! as a plain `UpdateView` both UIs draw. Nothing here is persisted: a launch starts unchecked.

use crate::service::sandbox;
use crate::service::self_update::{self, Prepared, Progress, Stage};
use crate::service::update_check::{self, Channel, Release, UpdateOutcome};
use crate::service::work_queue;
use crate::store::{SharedStore, Store};
use eframe::egui;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The commit this build came from (`-dirty` when it had uncommitted changes), set by `build.rs`.
/// Shown in Settings ▸ About for bug reports; nothing reads it.
pub const BUILD_ID: &str = env!("DEGENVIEW_BUILD_ID");
/// The `setting` key holding the chosen update channel. Local to this Mac: not in `sync::policy`.
pub const CHANNEL_KEY: &str = "update.channel";

/// How long after launch the first check waits, so it never competes with startup.
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(15);
/// Between checks while the app stays open.
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// Between checks after one failed (offline, rate limited).
const RETRY_INTERVAL: Duration = Duration::from_secs(30 * 60);
/// What an available update says about itself until `install_target` has answered. On macOS that
/// runs `codesign`, which must not hold up the first window.
const PENDING_TARGET: &str = "Checking whether this copy can update itself…";

/// Where installing the available release has got to.
#[derive(Clone)]
enum Install {
    Idle,
    Working(Progress),
    /// `applied`: the new version is already swapped in on disk (the user asked to restart), which
    /// must happen once even if the quit that follows is called off and asked for again.
    Ready {
        version: String,
        prepared: Prepared,
        applied: bool,
    },
    /// Installing `version` failed; the error is shown only while that release is still the one on
    /// offer (a newer release makes it moot).
    Failed {
        version: String,
        message: String,
    },
}

struct State {
    /// Which releases checks offer.
    channel: Channel,
    /// The newest answer GitHub gave, and when it came back.
    last: Option<(UpdateOutcome, Instant)>,
    checking: bool,
    /// When the last check started; the schedule counts from it.
    last_started: Option<Instant>,
    /// The latest check could not reach GitHub, even when `last` still holds an earlier answer
    /// (a failed refresh keeps a known update). The retry interval follows this, not `last`.
    last_failed: bool,
    install: Install,
    /// The app bundle an update can replace, or why there is none (a `cargo run` build). Starts as
    /// `PENDING_TARGET` and is resolved on the update worker (`tick`), never while the app starts.
    target: Result<PathBuf, String>,
    /// The lookup has not been started yet.
    target_pending: bool,
    /// `flatpak update` owns updates (installed from a remote): nothing here checks or draws them.
    managed: bool,
    /// Set when an installed update should ask for the restart (cleared by "Later").
    restart_prompt: bool,
    /// The user chose "Restart Now" and the quit that follows has not been called off: when the app
    /// exits, the new version is started. Cleared when the quit is called off.
    relaunch_on_exit: bool,
    born: Instant,
    /// Viewports that draw this state; a worker's answer repaints exactly these.
    viewers: HashSet<egui::ViewportId>,
}

/// What the UIs draw: the state of the update flow, nothing about how it is run.
#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    /// No check has finished or started yet.
    Unchecked,
    Checking,
    UpToDate,
    Available {
        version: String,
        url: String,
        /// This build can download and swap the release in itself.
        installable: bool,
        /// Why it cannot, when it cannot.
        note: Option<String>,
    },
    Installing(Progress),
    Ready {
        version: String,
    },
    InstallFailed {
        version: String,
        url: String,
        message: String,
    },
    CheckFailed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct UpdateView {
    pub phase: Phase,
    /// A check is running, even when the phase still shows an earlier answer.
    pub checking: bool,
    /// How long ago the last answer came back.
    pub checked_ago: Option<Duration>,
}

/// A cheap handle: every window and the Settings page hold a clone of the app's one host.
#[derive(Clone)]
pub struct UpdateHost {
    shared: Arc<Mutex<State>>,
}

impl Default for UpdateHost {
    fn default() -> Self {
        Self::new()
    }
}

impl UpdateHost {
    /// Starts on the running version's own channel (a release candidate keeps getting release
    /// candidates until the user picks one).
    pub fn new() -> Self {
        let host = Self::with_target(Err(PENDING_TARGET.to_owned()), Channel::of_version(VERSION));
        {
            // One small file read at start-up, and only in a Flatpak.
            let mut state = host.lock();
            state.target_pending = true;
            state.managed = !cfg!(test) && sandbox::updated_by_flatpak();
        }
        host
    }

    /// `new`, with the channel the user saved. A value this version cannot read is ignored.
    pub fn load(store: &SharedStore) -> Self {
        let host = Self::new();
        let saved = store
            .lock()
            .ok()
            .and_then(|store| store.setting_strict::<String>(CHANNEL_KEY).ok())
            .flatten()
            .and_then(|key| Channel::from_key(&key));
        if let Some(channel) = saved {
            host.lock().channel = channel;
        }
        host
    }

    fn with_target(target: Result<PathBuf, String>, channel: Channel) -> Self {
        Self {
            shared: Arc::new(Mutex::new(State {
                channel,
                last: None,
                checking: false,
                last_started: None,
                last_failed: false,
                install: Install::Idle,
                target,
                target_pending: false,
                managed: false,
                restart_prompt: false,
                relaunch_on_exit: false,
                born: Instant::now(),
                viewers: HashSet::new(),
            })),
        }
    }

    /// A host for a Flatpak installed from a remote, for tests of the UIs that read it.
    #[cfg(test)]
    pub(crate) fn managed_for_test() -> Self {
        let host = Self::with_target(Err(PENDING_TARGET.to_owned()), Channel::Stable);
        host.lock().managed = true;
        host
    }

    /// A host standing at `phase`, for drawing tests of the UIs that read it.
    #[cfg(test)]
    pub(crate) fn at_phase(phase: &Phase) -> Self {
        let host = Self::with_target(
            Ok(PathBuf::from("/Applications/DegenView.app")),
            Channel::Stable,
        );
        let release = |installable: bool| Release {
            tag: "v9.9.9".to_owned(),
            version: "9.9.9".to_owned(),
            url: "https://example.test/release".to_owned(),
            asset: installable.then(|| update_check::ReleaseAsset {
                name: "DegenView-9.9.9-macos-arm64.zip".to_owned(),
                url: "https://example.test/a.zip".to_owned(),
                size: 10,
                sha256: "00".repeat(32),
            }),
            manual_only: None,
        };
        {
            let mut state = host.lock();
            match phase {
                Phase::Unchecked => {}
                Phase::Checking => state.checking = true,
                Phase::UpToDate => state.last = Some((UpdateOutcome::UpToDate, Instant::now())),
                Phase::CheckFailed(message) => {
                    state.last = Some((UpdateOutcome::Failed(message.clone()), Instant::now()));
                }
                Phase::Available { installable, .. } => {
                    state.last = Some((
                        UpdateOutcome::Available(release(*installable)),
                        Instant::now(),
                    ));
                }
                Phase::Installing(progress) => {
                    state.last = Some((UpdateOutcome::Available(release(true)), Instant::now()));
                    state.install = Install::Working(*progress);
                }
                Phase::Ready { version } => {
                    state.install = Install::Ready {
                        version: version.clone(),
                        prepared: Prepared::for_test(),
                        applied: false,
                    };
                }
                Phase::InstallFailed { message, .. } => {
                    state.last = Some((UpdateOutcome::Available(release(true)), Instant::now()));
                    state.install = Install::Failed {
                        version: "9.9.9".to_owned(),
                        message: message.clone(),
                    };
                }
            }
        }
        host
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The view for a window that draws it this frame: remembers the window, so a change that
    /// arrives from a worker thread repaints it.
    pub fn view_for(&self, ctx: &egui::Context) -> UpdateView {
        self.lock().viewers.insert(ctx.viewport_id());
        self.view()
    }

    fn repaint(&self, ctx: &egui::Context) {
        let viewers: Vec<_> = self.lock().viewers.iter().copied().collect();
        for viewer in viewers {
            ctx.request_repaint_of(viewer);
        }
        ctx.request_repaint();
    }

    pub fn view(&self) -> UpdateView {
        let state = self.lock();
        UpdateView {
            phase: phase(&state),
            checking: state.checking,
            checked_ago: state.last.as_ref().map(|(_, at)| at.elapsed()),
        }
    }

    pub fn channel(&self) -> Channel {
        self.lock().channel
    }

    /// Switches channel, saves the choice and looks again. `store` is the one the caller already
    /// holds (Settings draws under its lock). An update only the old channel offers is dropped; the
    /// new check brings back whatever the new one offers. Never downgrades: a release older than
    /// the running build is not an update on any channel.
    pub fn set_channel(&self, store: &Store, ctx: &egui::Context, channel: Channel) {
        {
            let mut state = self.lock();
            if state.channel == channel {
                return;
            }
            state.channel = channel;
            let stale = matches!(&state.last, Some((UpdateOutcome::Available(_), _)));
            if stale && !matches!(state.install, Install::Working(_) | Install::Ready { .. }) {
                state.last = None;
                state.install = Install::Idle;
            }
        }
        let _ = store.set_setting(CHANNEL_KEY, &channel.key());
        self.check_now(ctx);
    }

    /// Installed from a Flatpak remote, which updates it: the app neither checks nor offers updates.
    pub fn managed_by_flatpak(&self) -> bool {
        self.lock().managed
    }

    /// Called every frame: starts the launch check and the periodic ones.
    pub fn tick(&self, ctx: &egui::Context) {
        if self.managed_by_flatpak() {
            return;
        }
        self.resolve_target(ctx);
        let due = {
            let state = self.lock();
            check_due(&state, Instant::now())
        };
        if due {
            self.start_check(ctx, false);
        }
    }

    /// Looks up what this copy can replace, once, on the update worker. It runs before any check
    /// (one worker, in order), so an answer that arrives finds the target settled.
    fn resolve_target(&self, ctx: &egui::Context) {
        if !std::mem::take(&mut self.lock().target_pending) || cfg!(test) {
            return;
        }
        let host = self.clone();
        let ctx = ctx.clone();
        work_queue::UPDATES.submit(move || {
            let target = self_update::install_target();
            host.lock().target = target;
            host.repaint(&ctx);
        });
    }

    /// Asks GitHub for the latest release on a worker thread; the UIs repaint when it answers. A
    /// person asked for this, so a failed install's error is cleared: the answer is a fresh start.
    pub fn check_now(&self, ctx: &egui::Context) {
        self.start_check(ctx, true);
    }

    /// `manual`: a check someone asked for. A scheduled one leaves a failed install's error in
    /// place until it finds a different release (`finish_check`), so it is not wiped unseen.
    fn start_check(&self, ctx: &egui::Context, manual: bool) {
        let channel = {
            let mut state = self.lock();
            if state.managed || state.checking || matches!(state.install, Install::Working(_)) {
                return;
            }
            state.checking = true;
            state.last_started = Some(Instant::now());
            if manual && matches!(state.install, Install::Failed { .. }) {
                state.install = Install::Idle;
            }
            state.channel
        };
        if cfg!(test) {
            return;
        }
        let host = self.clone();
        let ctx = ctx.clone();
        work_queue::UPDATES.submit(move || {
            host.finish_check(update_check::check(VERSION, channel), channel);
            host.repaint(&ctx);
        });
    }

    /// Takes a check's answer. One asked of a channel the user has since left is dropped and the
    /// next tick asks again, so a switch never shows the old channel's update.
    fn finish_check(&self, outcome: UpdateOutcome, channel: Channel) {
        let mut state = self.lock();
        state.checking = false;
        if channel != state.channel {
            state.last_started = None;
            return;
        }
        state.last_failed = matches!(outcome, UpdateOutcome::Failed(_));
        // A check that could not reach GitHub must not take a known update away.
        let keeps_known_update =
            state.last_failed && matches!(&state.last, Some((UpdateOutcome::Available(_), _)));
        if keeps_known_update {
            return;
        }
        // The error of a failed install belongs to the release that failed: once a different one
        // is on offer (or none), it no longer applies and a retry starts clean.
        if let Install::Failed { version, .. } = &state.install {
            let still_offered = matches!(
                &outcome,
                UpdateOutcome::Available(release) if release.version == *version
            );
            if !still_offered {
                state.install = Install::Idle;
            }
        }
        state.last = Some((outcome, Instant::now()));
    }

    /// Downloads and verifies the available release on a worker thread; nothing is replaced until
    /// `restart`.
    pub fn install(&self, ctx: &egui::Context) {
        let (app, release) = {
            let mut state = self.lock();
            let Ok(app) = state.target.clone() else {
                return;
            };
            let Some((UpdateOutcome::Available(release), _)) = &state.last else {
                return;
            };
            if release.asset.is_none() || matches!(state.install, Install::Working(_)) {
                return;
            }
            let release = release.clone();
            state.install = Install::Working(Progress::new(Stage::Verifying));
            (app, release)
        };
        if cfg!(test) {
            return;
        }
        let host = self.clone();
        let ctx = ctx.clone();
        work_queue::UPDATES.submit(move || {
            let progress = |progress| {
                host.lock().install = Install::Working(progress);
                host.repaint(&ctx);
            };
            let result = self_update::prepare(&app, &release, &progress);
            host.finish_install(release.version, result);
            host.repaint(&ctx);
        });
    }

    fn finish_install(&self, version: String, result: Result<Prepared, String>) {
        let mut state = self.lock();
        match result {
            Ok(prepared) => {
                state.install = Install::Ready {
                    version,
                    prepared,
                    applied: false,
                };
                state.restart_prompt = true;
            }
            Err(message) => state.install = Install::Failed { version, message },
        }
    }

    /// What clicking the tab bar's update mark does in the current phase: install, retry, open the
    /// release page for a build that cannot install itself, or ask about the restart.
    pub fn activate(&self, ctx: &egui::Context) {
        match self.view().phase {
            Phase::Available {
                installable: true, ..
            }
            | Phase::InstallFailed { .. } => self.install(ctx),
            Phase::Available { url, .. } => ctx.open_url(egui::OpenUrl::new_tab(url)),
            Phase::Ready { .. } => self.prompt_restart(),
            _ => {}
        }
    }

    /// Swaps the prepared update in and schedules the reopen. On `Ok` the caller must quit now.
    pub fn restart(&self) -> Result<(), String> {
        self.restart_with(self_update::apply)
    }

    /// "Restart Now": swaps the new version in (once) and arms the relaunch, which happens when the
    /// app actually exits (`relaunch_if_armed`). Nothing is started yet: the quit that follows may
    /// still be called off by an unsaved-script question, and a new process started now would sit
    /// beside the old one instead of replacing it.
    fn restart_with(&self, apply: impl Fn(&Prepared) -> Result<(), String>) -> Result<(), String> {
        let (prepared, applied, version) = match &self.lock().install {
            Install::Ready {
                prepared,
                applied,
                version,
            } => (prepared.clone(), *applied, version.clone()),
            _ => return Err("There is no update ready to install.".to_owned()),
        };
        // Outside the lock: this renames the bundle on disk. Not again after a quit that was
        // called off and is now asked for a second time: the new version is already in place, and
        // swapping once more would move it over the copy kept for a manual restore.
        let result = if applied { Ok(()) } else { apply(&prepared) };
        let mut state = self.lock();
        state.restart_prompt = false;
        match &result {
            Ok(()) => {
                if let Install::Ready { applied, .. } = &mut state.install {
                    *applied = true;
                }
                state.relaunch_on_exit = true;
            }
            Err(message) => {
                state.install = Install::Failed {
                    version,
                    message: message.clone(),
                };
            }
        }
        result
    }

    /// The quit a restart asked for was called off (an unsaved script, Cancel): the app keeps
    /// running, so nothing may start the new version when it exits some other day.
    pub fn disarm_relaunch(&self) {
        self.lock().relaunch_on_exit = false;
    }

    /// Run when the app is certain to exit: starts the new version if "Restart Now" was chosen and
    /// the quit was not called off. A failure cannot be shown (the app is going); the new version is
    /// already in place, so the caller logs it and the user reopens the app.
    pub fn relaunch_if_armed(&self) -> Result<(), String> {
        self.relaunch_with(self_update::relaunch)
    }

    fn relaunch_with(
        &self,
        relaunch: impl Fn(&Prepared) -> Result<(), String>,
    ) -> Result<(), String> {
        let prepared = {
            let state = self.lock();
            match &state.install {
                Install::Ready { prepared, .. } if state.relaunch_on_exit => prepared.clone(),
                _ => return Ok(()),
            }
        };
        relaunch(&prepared)
    }

    /// Whether the "restart to finish updating" question is waiting for an answer.
    pub fn restart_prompt_pending(&self) -> bool {
        let state = self.lock();
        state.restart_prompt && matches!(state.install, Install::Ready { .. })
    }

    pub fn prompt_restart(&self) {
        self.lock().restart_prompt = true;
    }

    /// "Later": the tab bar keeps offering the restart.
    pub fn dismiss_restart_prompt(&self) {
        self.lock().restart_prompt = false;
    }
}

fn phase(state: &State) -> Phase {
    let available = match &state.last {
        Some((UpdateOutcome::Available(release), _)) => Some(release),
        _ => None,
    };
    match (&state.install, available) {
        (Install::Working(progress), _) => Phase::Installing(*progress),
        (Install::Ready { version, .. }, _) => Phase::Ready {
            version: version.clone(),
        },
        (Install::Failed { version, message }, Some(release)) if release.version == *version => {
            Phase::InstallFailed {
                version: release.version.clone(),
                url: release.url.clone(),
                message: message.clone(),
            }
        }
        (_, Some(release)) => available_phase(release, &state.target),
        (_, None) => match &state.last {
            Some((UpdateOutcome::Failed(message), _)) => Phase::CheckFailed(message.clone()),
            Some(_) => Phase::UpToDate,
            None if state.checking => Phase::Checking,
            None => Phase::Unchecked,
        },
    }
}

fn available_phase(release: &Release, target: &Result<PathBuf, String>) -> Phase {
    let installable = release.asset.is_some() && target.is_ok();
    let note = if installable {
        None
    } else if let Some(reason) = release.manual_only {
        Some(reason.to_owned())
    } else if let Err(reason) = target {
        Some(reason.clone())
    } else {
        Some("This release has no download for this computer.".to_owned())
    };
    Phase::Available {
        version: release.version.clone(),
        url: release.url.clone(),
        installable,
        note,
    }
}

/// The schedule: once shortly after launch, then every six hours (half an hour after a failure),
/// and never while an install is under way or waiting for its restart. A failed install does not
/// stop it: a corrected release published since must still be found.
fn check_due(state: &State, now: Instant) -> bool {
    if state.checking || matches!(state.install, Install::Working(_) | Install::Ready { .. }) {
        return false;
    }
    match state.last_started {
        None => now.saturating_duration_since(state.born) >= FIRST_CHECK_DELAY,
        Some(started) => {
            let wait = if state.last_failed {
                RETRY_INTERVAL
            } else {
                CHECK_INTERVAL
            };
            now.saturating_duration_since(started) >= wait
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::update_check::ReleaseAsset;

    fn release(manual_only: Option<&'static str>, with_asset: bool) -> Release {
        Release {
            tag: "v9.9.9".to_owned(),
            version: "9.9.9".to_owned(),
            url: "https://example.test/release".to_owned(),
            asset: with_asset.then(|| ReleaseAsset {
                name: "DegenView-9.9.9-macos-arm64.zip".to_owned(),
                url: "https://example.test/a.zip".to_owned(),
                size: 10,
                sha256: "00".repeat(32),
            }),
            manual_only,
        }
    }

    fn host() -> UpdateHost {
        UpdateHost::with_target(
            Ok(PathBuf::from("/Applications/DegenView.app")),
            Channel::Stable,
        )
    }

    fn ctx() -> egui::Context {
        egui::Context::default()
    }

    #[test]
    fn a_fresh_host_has_not_checked() {
        let view = host().view();
        assert_eq!(view.phase, Phase::Unchecked);
        assert_eq!(view.checked_ago, None);
    }

    #[test]
    fn checking_then_each_outcome() {
        let host = host();
        host.check_now(&ctx());
        assert_eq!(host.view().phase, Phase::Checking);
        assert!(host.view().checking);

        host.finish_check(UpdateOutcome::UpToDate, Channel::Stable);
        assert_eq!(host.view().phase, Phase::UpToDate);
        assert!(!host.view().checking);

        host.finish_check(UpdateOutcome::Failed("offline".to_owned()), Channel::Stable);
        assert_eq!(host.view().phase, Phase::CheckFailed("offline".to_owned()));

        host.finish_check(
            UpdateOutcome::Available(release(None, true)),
            Channel::Stable,
        );
        let view = host.view();
        assert_eq!(
            view.phase,
            Phase::Available {
                version: "9.9.9".to_owned(),
                url: "https://example.test/release".to_owned(),
                installable: true,
                note: None,
            }
        );
    }

    #[test]
    fn a_failed_recheck_keeps_a_known_update() {
        let host = host();
        host.finish_check(
            UpdateOutcome::Available(release(None, true)),
            Channel::Stable,
        );
        host.check_now(&ctx());
        // While rechecking, the earlier answer still shows.
        assert!(matches!(host.view().phase, Phase::Available { .. }));
        assert!(host.view().checking);
        host.finish_check(UpdateOutcome::Failed("offline".to_owned()), Channel::Stable);
        assert!(matches!(host.view().phase, Phase::Available { .. }));
        host.finish_check(UpdateOutcome::UpToDate, Channel::Stable);
        assert_eq!(host.view().phase, Phase::UpToDate);
    }

    #[test]
    fn a_release_that_cannot_be_installed_says_why() {
        let host = host();
        host.finish_check(
            UpdateOutcome::Available(release(Some("Download it from the release page."), false)),
            Channel::Stable,
        );
        let Phase::Available {
            installable, note, ..
        } = host.view().phase
        else {
            panic!("available");
        };
        assert!(!installable);
        assert_eq!(note.as_deref(), Some("Download it from the release page."));

        let blocked = UpdateHost::with_target(
            Err("Move DegenView to Applications.".to_owned()),
            Channel::Stable,
        );
        blocked.finish_check(
            UpdateOutcome::Available(release(None, true)),
            Channel::Stable,
        );
        let Phase::Available {
            installable, note, ..
        } = blocked.view().phase
        else {
            panic!("available");
        };
        assert!(!installable);
        assert_eq!(note.as_deref(), Some("Move DegenView to Applications."));
        // Nothing starts for a build that cannot install.
        blocked.install(&ctx());
        assert!(matches!(blocked.view().phase, Phase::Available { .. }));
    }

    #[test]
    fn installing_goes_through_working_to_ready_and_asks_to_restart() {
        let host = host();
        host.finish_check(
            UpdateOutcome::Available(release(None, true)),
            Channel::Stable,
        );
        host.install(&ctx());
        assert_eq!(
            host.view().phase,
            Phase::Installing(Progress::new(Stage::Verifying))
        );
        // A second click while it runs does nothing.
        host.install(&ctx());
        host.check_now(&ctx());
        assert!(!host.view().checking);

        host.finish_install("9.9.9".to_owned(), Err("checksum".to_owned()));
        assert!(matches!(
            host.view().phase,
            Phase::InstallFailed { ref message, .. } if message == "checksum"
        ));
        assert!(!host.restart_prompt_pending());
        // A new check clears the failure.
        host.check_now(&ctx());
        assert!(matches!(host.view().phase, Phase::Available { .. }));
    }

    #[test]
    fn a_ready_update_prompts_once_until_asked_again() {
        let host = host();
        host.finish_check(
            UpdateOutcome::Available(release(None, true)),
            Channel::Stable,
        );
        let prepared = Prepared::for_test();
        host.finish_install("9.9.9".to_owned(), Ok(prepared));
        assert_eq!(
            host.view().phase,
            Phase::Ready {
                version: "9.9.9".to_owned()
            }
        );
        assert!(host.restart_prompt_pending());
        host.dismiss_restart_prompt();
        assert!(!host.restart_prompt_pending());
        host.prompt_restart();
        assert!(host.restart_prompt_pending());
    }

    fn ready_host() -> UpdateHost {
        UpdateHost::at_phase(&Phase::Ready {
            version: "9.9.9".to_owned(),
        })
    }

    #[test]
    fn restart_swaps_once_and_relaunches_only_when_the_app_exits() {
        let (swaps, relaunches) = (std::cell::Cell::new(0), std::cell::Cell::new(0));
        let apply = |_: &Prepared| {
            swaps.set(swaps.get() + 1);
            Ok(())
        };
        let relaunch = |_: &Prepared| {
            relaunches.set(relaunches.get() + 1);
            Ok(())
        };
        let host = ready_host();
        // Nothing starts at "Restart Now": the quit is not decided yet.
        host.restart_with(apply).unwrap();
        assert_eq!((swaps.get(), relaunches.get()), (1, 0));
        // The quit is called off, then asked for again: one swap in all, and the exit relaunches.
        host.disarm_relaunch();
        host.restart_with(apply).unwrap();
        assert_eq!(swaps.get(), 1);
        host.relaunch_with(relaunch).unwrap();
        assert_eq!(relaunches.get(), 1);
    }

    #[test]
    fn a_called_off_quit_never_relaunches() {
        let relaunches = std::cell::Cell::new(0);
        let relaunch = |_: &Prepared| {
            relaunches.set(relaunches.get() + 1);
            Ok(())
        };
        let host = ready_host();
        host.restart_with(|_| Ok(())).unwrap();
        host.disarm_relaunch();
        // The user keeps working and quits another day: nothing starts a second copy.
        host.relaunch_with(relaunch).unwrap();
        // And an app that never chose "Restart Now" relaunches nothing either.
        ready_host().relaunch_with(relaunch).unwrap();
        assert_eq!(relaunches.get(), 0);
    }

    #[test]
    fn a_failed_swap_stays_visible_and_arms_nothing() {
        let relaunches = std::cell::Cell::new(0);
        let host = ready_host();
        let result = host.restart_with(|_| Err("Could not install the new app: busy".to_owned()));
        assert!(result.is_err());
        // The failure is what the tab bar shows (a red mark with this reason).
        assert!(
            matches!(&host.lock().install, Install::Failed { message, .. } if message.contains("busy"))
        );
        host.relaunch_with(|_| {
            relaunches.set(relaunches.get() + 1);
            Ok(())
        })
        .unwrap();
        assert_eq!(relaunches.get(), 0);
    }

    #[test]
    fn restart_without_a_ready_update_is_an_error() {
        assert!(host().restart().is_err());
    }

    #[test]
    fn checks_follow_the_schedule() {
        let host = host();
        let mut state = host.lock();
        let born = state.born;
        // Not before the launch delay.
        assert!(!check_due(
            &state,
            born + FIRST_CHECK_DELAY - Duration::from_secs(1)
        ));
        assert!(check_due(&state, born + FIRST_CHECK_DELAY));

        state.last_started = Some(born);
        state.last = Some((UpdateOutcome::UpToDate, born));
        assert!(!check_due(
            &state,
            born + CHECK_INTERVAL - Duration::from_secs(1)
        ));
        assert!(check_due(&state, born + CHECK_INTERVAL));

        // A failed check is retried sooner.
        state.last = Some((UpdateOutcome::Failed("x".to_owned()), born));
        state.last_failed = true;
        assert!(check_due(&state, born + RETRY_INTERVAL));
        state.last_failed = false;

        // Never while one runs, an install runs, or a restart waits.
        state.checking = true;
        assert!(!check_due(&state, born + CHECK_INTERVAL * 2));
        state.checking = false;
        state.install = Install::Working(Progress::new(Stage::Downloading));
        assert!(!check_due(&state, born + CHECK_INTERVAL * 2));
        // A failed install does not stop the schedule: a corrected release must still be found.
        state.install = Install::Failed {
            version: "9.9.9".to_owned(),
            message: "x".to_owned(),
        };
        assert!(check_due(&state, born + CHECK_INTERVAL * 2));
    }

    #[test]
    fn a_flatpak_installed_from_a_remote_never_checks() {
        let host = UpdateHost::managed_for_test();
        assert!(host.managed_by_flatpak());
        // Neither the schedule nor a person asking starts a check, and the target is not looked up.
        host.tick(&ctx());
        host.check_now(&ctx());
        let view = host.view();
        assert!(!view.checking);
        assert_eq!(view.phase, Phase::Unchecked);
        assert!(host.lock().last_started.is_none());
        // Any other host is unaffected.
        assert!(!UpdateHost::new().managed_by_flatpak());
        let other = UpdateHost::new();
        other.check_now(&ctx());
        assert!(other.view().checking);
    }

    #[test]
    fn construction_does_not_look_up_the_install_target() {
        let host = UpdateHost::new();
        assert!(host.lock().target_pending);
        let channel = host.channel();
        host.finish_check(UpdateOutcome::Available(release(None, true)), channel);
        // Until the worker answers, the update is on offer but not yet installable.
        assert!(matches!(
            host.view().phase,
            Phase::Available { installable: false, note: Some(ref note), .. } if note == PENDING_TARGET
        ));
        // The lookup is started once, by the first tick.
        host.tick(&ctx());
        assert!(!host.lock().target_pending);
    }

    #[test]
    fn a_failed_refresh_of_a_known_update_retries_after_half_an_hour() {
        let host = host();
        host.finish_check(
            UpdateOutcome::Available(release(None, true)),
            Channel::Stable,
        );
        let born = host.lock().born;
        host.lock().last_started = Some(born);
        assert!(!check_due(&host.lock(), born + RETRY_INTERVAL));
        // GitHub cannot be reached: the update stays on offer, but the next try is the short one.
        host.finish_check(UpdateOutcome::Failed("offline".to_owned()), Channel::Stable);
        assert!(matches!(host.view().phase, Phase::Available { .. }));
        assert!(check_due(&host.lock(), born + RETRY_INTERVAL));
        // An answer clears it.
        host.finish_check(
            UpdateOutcome::Available(release(None, true)),
            Channel::Stable,
        );
        assert!(!check_due(&host.lock(), born + RETRY_INTERVAL));
    }

    fn newer(version: &str) -> UpdateOutcome {
        let mut newer = release(None, true);
        newer.version = version.to_owned();
        newer.tag = format!("v{version}");
        UpdateOutcome::Available(newer)
    }

    #[test]
    fn a_scheduled_check_finds_the_release_that_replaces_a_failed_one() {
        let host = host();
        host.finish_check(newer("9.9.9"), Channel::Stable);
        host.finish_install("9.9.9".to_owned(), Err("checksum".to_owned()));
        assert!(matches!(host.view().phase, Phase::InstallFailed { .. }));
        let born = host.lock().born;
        host.lock().last_started = Some(born);
        // The periodic check is still due after the failure...
        assert!(check_due(&host.lock(), born + CHECK_INTERVAL));
        // ...and when 9.9.10 is out, it is offered without the old error attached.
        host.start_check(&ctx(), false);
        host.finish_check(newer("9.9.10"), Channel::Stable);
        match host.view().phase {
            Phase::Available { version, .. } => assert_eq!(version, "9.9.10"),
            other => panic!("expected the new release, got {other:?}"),
        }
    }

    #[test]
    fn a_scheduled_check_keeps_the_error_of_the_release_that_failed() {
        let host = host();
        host.finish_check(newer("9.9.9"), Channel::Stable);
        host.finish_install("9.9.9".to_owned(), Err("checksum".to_owned()));
        // Nothing new: the scheduled check does not wipe the error unseen.
        host.start_check(&ctx(), false);
        host.finish_check(newer("9.9.9"), Channel::Stable);
        assert!(matches!(
            host.view().phase,
            Phase::InstallFailed { ref message, .. } if message == "checksum"
        ));
        // Nor does a check that could not reach GitHub.
        host.start_check(&ctx(), false);
        host.finish_check(UpdateOutcome::Failed("offline".to_owned()), Channel::Stable);
        assert!(matches!(host.view().phase, Phase::InstallFailed { .. }));
        // A person asking again starts clean.
        host.check_now(&ctx());
        assert!(matches!(host.view().phase, Phase::Available { .. }));
    }

    #[test]
    fn tick_starts_nothing_before_the_launch_delay() {
        let host = host();
        host.tick(&ctx());
        assert_eq!(host.view().phase, Phase::Unchecked);
    }

    fn store() -> SharedStore {
        Arc::new(Mutex::new(Store::open_in_memory().unwrap()))
    }

    #[test]
    fn the_channel_defaults_to_the_running_builds_and_saves_the_choice() {
        let shared = store();
        // Nothing saved: the running build's own channel.
        assert_eq!(
            UpdateHost::load(&shared).channel(),
            Channel::of_version(VERSION)
        );
        // A value this version cannot read is ignored.
        shared
            .lock()
            .unwrap()
            .set_setting(CHANNEL_KEY, &"nightly")
            .unwrap();
        assert_eq!(
            UpdateHost::load(&shared).channel(),
            Channel::of_version(VERSION)
        );

        let host = UpdateHost::load(&shared);
        let other = if host.channel() == Channel::Stable {
            Channel::ReleaseCandidate
        } else {
            Channel::Stable
        };
        host.set_channel(&shared.lock().unwrap(), &ctx(), other);
        assert_eq!(host.channel(), other);
        assert_eq!(UpdateHost::load(&shared).channel(), other);
    }

    #[test]
    fn switching_channel_drops_the_old_updates_and_checks_again() {
        let shared = store();
        let host = host();
        host.finish_check(
            UpdateOutcome::Available(release(None, true)),
            Channel::Stable,
        );
        host.set_channel(&shared.lock().unwrap(), &ctx(), Channel::ReleaseCandidate);
        // The stable answer is gone; the new channel is being asked.
        assert!(host.view().checking);
        assert_eq!(host.view().phase, Phase::Checking);
        // Choosing the same channel again does nothing.
        host.finish_check(UpdateOutcome::UpToDate, Channel::ReleaseCandidate);
        host.set_channel(&shared.lock().unwrap(), &ctx(), Channel::ReleaseCandidate);
        assert!(!host.view().checking);
        assert_eq!(host.view().phase, Phase::UpToDate);
    }

    #[test]
    fn an_answer_for_a_channel_the_user_left_is_dropped() {
        let shared = store();
        let host = host();
        host.check_now(&ctx());
        host.set_channel(&shared.lock().unwrap(), &ctx(), Channel::ReleaseCandidate);
        // The stable check lands after the switch.
        host.finish_check(
            UpdateOutcome::Available(release(None, true)),
            Channel::Stable,
        );
        assert!(!host.view().checking);
        assert_eq!(host.view().phase, Phase::Unchecked);
        assert!(host.lock().last_started.is_none());
    }

    #[test]
    fn the_build_id_is_one_word() {
        assert!(!BUILD_ID.is_empty());
        assert!(!BUILD_ID.contains(char::is_whitespace));
    }
}
