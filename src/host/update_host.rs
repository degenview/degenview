//! The app's one view of "is there a newer DegenView": checks GitHub on a schedule, downloads and
//! verifies a release when asked, and hands the result to the tab bar and Settings ▸ About.
//!
//! The network and install work is `service::update_check` / `service::self_update`; this is the
//! stateful bridge that runs them on `work_queue::UPDATES`, remembers the outcome, and exposes it
//! as a plain `UpdateView` both UIs draw. Nothing here is persisted: a launch starts unchecked.

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

/// Where installing the available release has got to.
#[derive(Clone)]
enum Install {
    Idle,
    Working(Progress),
    Ready { version: String, prepared: Prepared },
    Failed(String),
}

struct State {
    /// Which releases checks offer.
    channel: Channel,
    /// The newest answer GitHub gave, and when it came back.
    last: Option<(UpdateOutcome, Instant)>,
    checking: bool,
    /// When the last check started; the schedule counts from it.
    last_started: Option<Instant>,
    install: Install,
    /// The app bundle an update can replace, or why there is none (a `cargo run` build).
    target: Result<PathBuf, String>,
    /// Set when an installed update should ask for the restart (cleared by "Later").
    restart_prompt: bool,
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
        Self::with_target(self_update::install_target(), Channel::of_version(VERSION))
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
                install: Install::Idle,
                target,
                restart_prompt: false,
                born: Instant::now(),
                viewers: HashSet::new(),
            })),
        }
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
                    };
                }
                Phase::InstallFailed { message, .. } => {
                    state.last = Some((UpdateOutcome::Available(release(true)), Instant::now()));
                    state.install = Install::Failed(message.clone());
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

    /// Called every frame: starts the launch check and the periodic ones.
    pub fn tick(&self, ctx: &egui::Context) {
        let due = {
            let state = self.lock();
            check_due(&state, Instant::now())
        };
        if due {
            self.check_now(ctx);
        }
    }

    /// Asks GitHub for the latest release on a worker thread; the UIs repaint when it answers.
    pub fn check_now(&self, ctx: &egui::Context) {
        let channel = {
            let mut state = self.lock();
            if state.checking || matches!(state.install, Install::Working(_)) {
                return;
            }
            state.checking = true;
            state.last_started = Some(Instant::now());
            if matches!(state.install, Install::Failed(_)) {
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
        // A check that could not reach GitHub must not take a known update away.
        let keeps_known_update = matches!(outcome, UpdateOutcome::Failed(_))
            && matches!(&state.last, Some((UpdateOutcome::Available(_), _)));
        if !keeps_known_update {
            state.last = Some((outcome, Instant::now()));
        }
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
                state.install = Install::Ready { version, prepared };
                state.restart_prompt = true;
            }
            Err(message) => state.install = Install::Failed(message),
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
        let prepared = match &self.lock().install {
            Install::Ready { prepared, .. } => prepared.clone(),
            _ => return Err("There is no update ready to install.".to_owned()),
        };
        // Outside the lock: this renames the bundle on disk.
        let result = self_update::apply_and_relaunch(&prepared);
        let mut state = self.lock();
        state.restart_prompt = false;
        if let Err(message) = &result {
            state.install = Install::Failed(message.clone());
        }
        result
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
        (Install::Failed(message), Some(release)) => Phase::InstallFailed {
            version: release.version.clone(),
            url: release.url.clone(),
            message: message.clone(),
        },
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
        Some("This release has no download for this Mac.".to_owned())
    };
    Phase::Available {
        version: release.version.clone(),
        url: release.url.clone(),
        installable,
        note,
    }
}

/// The schedule: once shortly after launch, then every six hours (half an hour after a failure),
/// and never while an install is under way or waiting for its restart.
fn check_due(state: &State, now: Instant) -> bool {
    if state.checking || !matches!(state.install, Install::Idle) {
        return false;
    }
    match state.last_started {
        None => now.saturating_duration_since(state.born) >= FIRST_CHECK_DELAY,
        Some(started) => {
            let wait = match &state.last {
                Some((UpdateOutcome::Failed(_), _)) => RETRY_INTERVAL,
                _ => CHECK_INTERVAL,
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
        assert!(check_due(&state, born + RETRY_INTERVAL));

        // Never while one runs, an install runs, or a restart waits.
        state.checking = true;
        assert!(!check_due(&state, born + CHECK_INTERVAL * 2));
        state.checking = false;
        state.install = Install::Working(Progress::new(Stage::Downloading));
        assert!(!check_due(&state, born + CHECK_INTERVAL * 2));
        state.install = Install::Failed("x".to_owned());
        assert!(!check_due(&state, born + CHECK_INTERVAL * 2));
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
