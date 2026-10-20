//! The app's one view of iCloud Sync: owns the sync worker, applies fetched changes on the app
//! thread, carries out the user's choices, and exposes a plain `SyncView` the interface draws.
//!
//! The pure logic is `sync::*`; this is the stateful bridge. The worker thread runs `SyncCore`
//! and never touches a store the UI holds a lock on for long. Everything that changes what the
//! user sees on screen (applying remote data, reading script files) happens in `tick`, on the app
//! thread, where those owners live.

use crate::script_manager::store::ScriptStore;
use crate::store::SharedStore;
use crate::store::sync::ConflictSummary;
use crate::sync::account::{self, AccountChoice};
use crate::sync::apply::{ApplyCtx, ApplyReport, apply_batch};
use crate::sync::core::{Effect, PauseReason, Phase, SyncCore};
use crate::sync::initial::{JoinMode, JoinSummary, finish_join, summarize};
use crate::sync::policy::Category;
use crate::sync::scripts::{NoScripts, ScriptCache, ScriptSink, StoreScriptSink, scan_scripts};
use crate::sync::transport::{SyncTransport, TransportEvent};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// How the worker hears from the native bridge.
pub type EventSink = Arc<dyn Fn(TransportEvent) + Send + Sync>;

/// Creates the connection to iCloud. The real one wraps CloudKit; builds without it give none.
pub trait TransportFactory: Send + Sync {
    /// `Production` or `Development`, or why iCloud cannot work in this build as signed.
    fn environment(&self) -> Result<String, String>;
    fn create(&self, events: EventSink) -> Box<dyn SyncTransport + Send>;
}

impl SyncTransport for Box<dyn SyncTransport + Send> {
    fn send(&mut self, command: crate::sync::transport::TransportCommand) {
        (**self).send(command);
    }
}

/// How often another Mac's changes are looked for even without a push.
const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// How often changed script files are looked for.
const SCRIPT_SCAN_INTERVAL: Duration = Duration::from_secs(60);
/// After a script is saved, wait this long for more edits before scanning.
const SCRIPT_SCAN_DEBOUNCE: Duration = Duration::from_secs(3);

/// What the interface asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum SyncAction {
    Enable,
    Disable,
    SyncNow,
    /// The user agreed to use the signed-in Apple Account for this database.
    ConfirmAccount,
    ChooseJoin(JoinMode),
    ResolveAccount(AccountChoice),
    SetCategory(Category, bool),
    /// Let a large delete go to iCloud.
    ConfirmHeldDeletes,
    /// Keep a large delete from reaching iCloud and the other Macs.
    KeepHeldDeletes,
    DismissConflict(i64),
    /// Delete every DegenView record in iCloud for the account this Mac syncs with.
    DeleteCloudData,
    /// Another Mac deleted the data in iCloud: upload this Mac's again, or stop.
    ResolveRemoteDeletion {
        reupload: bool,
    },
}

/// Something only the user can decide.
#[derive(Debug, Clone, PartialEq)]
pub enum SyncPrompt {
    /// An Apple Account is signed in; agree to use it before anything is read or sent.
    ConfirmAccount,
    /// iCloud has data (or is empty); choose how to combine it with this Mac's.
    Join(JoinSummary),
    /// A different Apple Account is signed in than the one this database syncs with.
    AccountChanged,
    /// Another Mac deleted DegenView's data in iCloud.
    RemoteDataDeleted,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SyncView {
    /// This build can sync at all; when not, the reason.
    pub unavailable: Option<String>,
    pub enabled: bool,
    pub phase: Phase,
    pub pending: usize,
    pub held: usize,
    pub blocked: usize,
    pub last_send_ms: Option<i64>,
    pub last_fetch_ms: Option<i64>,
    pub last_error: Option<String>,
    pub environment: Option<String>,
    pub device_name: Option<String>,
    pub prompt: Option<SyncPrompt>,
    pub categories: Vec<(Category, bool)>,
    pub conflicts: Vec<ConflictSummary>,
    /// iCloud confirmed that all DegenView data there was deleted at this Mac's request.
    pub cloud_deleted: bool,
}

impl SyncView {
    fn initial(unavailable: Option<String>) -> Self {
        Self {
            unavailable,
            enabled: false,
            phase: Phase::Off,
            pending: 0,
            held: 0,
            blocked: 0,
            last_send_ms: None,
            last_fetch_ms: None,
            last_error: None,
            environment: None,
            device_name: None,
            prompt: None,
            categories: Category::ALL.iter().map(|c| (*c, true)).collect(),
            conflicts: Vec::new(),
            cloud_deleted: false,
        }
    }
}

enum WorkerMsg {
    Event(TransportEvent),
    Applied(u64),
    Start,
    Stop,
    Restart,
    ClearPause,
    SyncNow,
    DeleteCloudData,
    Tick,
    /// Only tests end the worker early; the process exiting ends it otherwise.
    #[cfg_attr(not(test), allow(dead_code))]
    Shutdown,
}

struct Worker {
    tx: Sender<WorkerMsg>,
    #[cfg_attr(not(test), allow(dead_code))]
    handle: Option<std::thread::JoinHandle<()>>,
}

#[derive(Default)]
struct Shared {
    view: Option<SyncView>,
    effects: Vec<Effect>,
    actions: Vec<SyncAction>,
    reports: Vec<ApplyReport>,
    unsaved_scripts: HashSet<String>,
    pending_account: Option<String>,
    mismatched_account: Option<String>,
    scripts_changed_at: Option<Instant>,
    last_script_scan: Option<Instant>,
}

#[derive(Clone)]
pub struct SyncHost {
    store: SharedStore,
    factory: Option<Arc<dyn TransportFactory>>,
    shared: Arc<Mutex<Shared>>,
    worker: Arc<Mutex<Option<Worker>>>,
    script_cache: ScriptCache,
    scripts: Arc<Mutex<Option<ScriptStore>>>,
    repaint: Arc<dyn Fn() + Send + Sync>,
    backups_dir: PathBuf,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl SyncHost {
    /// `scripts` is the user's script library (a separate handle on the same files as the Script
    /// Manager's); `None` leaves scripts out of sync.
    pub fn new(
        store: SharedStore,
        factory: Option<Arc<dyn TransportFactory>>,
        scripts: Option<ScriptStore>,
        backups_dir: PathBuf,
        repaint: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let unavailable = match &factory {
            None => Some("iCloud Sync is not included in this build of DegenView.".to_string()),
            Some(factory) => factory.environment().err(),
        };
        let host = Self {
            store,
            factory,
            shared: Arc::new(Mutex::new(Shared {
                view: Some(SyncView::initial(unavailable.clone())),
                ..Default::default()
            })),
            worker: Arc::new(Mutex::new(None)),
            script_cache: ScriptCache::default(),
            scripts: Arc::new(Mutex::new(scripts)),
            repaint,
            backups_dir,
        };
        host.refresh_view();
        if unavailable.is_none() && host.store_enabled() {
            host.start_worker();
        }
        host
    }

    fn store_enabled(&self) -> bool {
        lock(&self.store).sync_enabled()
    }

    /// What the interface draws.
    pub fn view(&self) -> SyncView {
        lock(&self.shared)
            .view
            .clone()
            .unwrap_or_else(|| SyncView::initial(None))
    }

    /// Queues an action for the next `tick`; the interface never blocks on sync.
    pub fn act(&self, action: SyncAction) {
        lock(&self.shared).actions.push(action);
        (self.repaint)();
    }

    /// Text typed but not saved in the Script Manager, which sync must never overwrite.
    pub fn set_unsaved_scripts(&self, ids: HashSet<String>) {
        lock(&self.shared).unsaved_scripts = ids;
    }

    /// A script was saved, renamed or deleted: look for changes soon.
    pub fn note_scripts_changed(&self) {
        lock(&self.shared).scripts_changed_at = Some(Instant::now());
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn shutdown(&self) {
        if let Some(mut worker) = lock(&self.worker).take() {
            let _ = worker.tx.send(WorkerMsg::Shutdown);
            if let Some(handle) = worker.handle.take() {
                let _ = handle.join();
            }
        }
    }

    // MARK: Worker

    fn sender(&self) -> Option<Sender<WorkerMsg>> {
        lock(&self.worker).as_ref().map(|worker| worker.tx.clone())
    }

    fn start_worker(&self) {
        let Some(factory) = self.factory.clone() else {
            return;
        };
        let mut guard = lock(&self.worker);
        if let Some(worker) = guard.as_ref() {
            let _ = worker.tx.send(WorkerMsg::Start);
            return;
        }
        let (tx, rx) = mpsc::channel::<WorkerMsg>();
        let sink: EventSink = {
            let tx = tx.clone();
            Arc::new(move |event| {
                let _ = tx.send(WorkerMsg::Event(event));
            })
        };
        let transport = factory.create(sink);
        let mut core = SyncCore::new(self.store.clone(), transport, Box::new(now_ms));
        core.set_script_cache(self.script_cache.clone());
        let shared = self.shared.clone();
        let store = self.store.clone();
        let repaint = self.repaint.clone();
        let handle = std::thread::Builder::new()
            .name("degenview-sync".into())
            .spawn(move || run_worker(core, rx, shared, store, repaint))
            .ok();
        *guard = Some(Worker { tx, handle });
        drop(guard);
        if let Some(tx) = self.sender() {
            let _ = tx.send(WorkerMsg::Start);
        }
    }

    fn tell(&self, message: WorkerMsg) {
        if let Some(tx) = self.sender() {
            let _ = tx.send(message);
        }
    }

    // MARK: App thread

    /// Carries out queued actions, applies fetched changes, and refreshes what the interface
    /// shows. Returns what was applied so the app can reload the stores it holds in memory.
    pub fn tick(&self) -> Vec<ApplyReport> {
        let actions = std::mem::take(&mut lock(&self.shared).actions);
        for action in actions {
            self.run(action);
        }
        let effects = std::mem::take(&mut lock(&self.shared).effects);
        for effect in effects {
            match effect {
                Effect::RemoteChanges(batch) => {
                    let id = batch.id;
                    if let Some(report) = self.apply(&batch) {
                        lock(&self.shared).reports.push(report);
                    }
                    self.tell(WorkerMsg::Applied(id));
                }
                Effect::NeedsBinding { user_hash } => {
                    lock(&self.shared).pending_account = Some(user_hash);
                }
                Effect::AccountMismatch { user_hash } => {
                    lock(&self.shared).mismatched_account = Some(user_hash);
                }
            }
        }
        // iCloud confirmed the deletion this Mac asked for: forget the binding and stop.
        let confirmed = lock(&self.shared)
            .view
            .as_ref()
            .is_some_and(|view| view.cloud_deleted);
        if confirmed {
            let _ = account::after_cloud_delete(&lock(&self.store));
            self.tell(WorkerMsg::Stop);
            if let Some(view) = lock(&self.shared).view.as_mut() {
                view.cloud_deleted = false;
            }
        }
        self.scan_scripts_if_due();
        self.refresh_view();
        std::mem::take(&mut lock(&self.shared).reports)
    }

    fn apply_ctx(&self) -> Option<ApplyCtx> {
        let state = lock(&self.store).sync_state().ok().flatten()?;
        Some(ApplyCtx {
            device_id: state.device_id,
            device_name: state.device_name,
            now_ms: now_ms(),
            date: today(),
        })
    }

    /// Runs `f` with the script library as a sink (or none, when there is no library).
    fn with_scripts<R>(&self, f: impl FnOnce(&mut dyn ScriptSink) -> R) -> R {
        let unsaved = lock(&self.shared).unsaved_scripts.clone();
        let mut library = lock(&self.scripts);
        match library.as_mut() {
            Some(store) => f(&mut StoreScriptSink { store, unsaved }),
            None => f(&mut NoScripts),
        }
    }

    fn apply(&self, batch: &crate::sync::core::RemoteBatch) -> Option<ApplyReport> {
        let ctx = self.apply_ctx()?;
        let report = self.with_scripts(|sink| {
            let store = lock(&self.store);
            apply_batch(&store, sink, batch, &ctx)
        });
        match report {
            Ok(report) => {
                if !report.tables.is_empty() || !report.settings.is_empty() {
                    (self.repaint)();
                }
                Some(report)
            }
            // Not acknowledged as failed: the checkpoint did not move, so it is fetched again.
            Err(_) => None,
        }
    }

    fn scan_scripts_if_due(&self) {
        if self.sender().is_none() || !self.store_enabled() {
            return;
        }
        let due = {
            let mut shared = lock(&self.shared);
            let debounced = shared
                .scripts_changed_at
                .is_some_and(|at| at.elapsed() >= SCRIPT_SCAN_DEBOUNCE);
            let periodic = shared
                .last_script_scan
                .is_none_or(|at| at.elapsed() >= SCRIPT_SCAN_INTERVAL);
            if debounced || periodic {
                shared.scripts_changed_at = None;
                shared.last_script_scan = Some(Instant::now());
                true
            } else {
                false
            }
        };
        if !due {
            return;
        }
        // Until the first-time merge has run, scripts are not queued: it decides what goes where.
        let ready = lock(&self.store)
            .sync_state()
            .ok()
            .flatten()
            .is_some_and(|s| s.initial_sync_done);
        if !ready {
            return;
        }
        let queued = self.with_scripts(|sink| {
            let store = lock(&self.store);
            scan_scripts(&store, sink, &self.script_cache)
        });
        if queued.is_ok_and(|done| done.upserts + done.deletes > 0) {
            self.tell(WorkerMsg::Tick);
        }
    }

    fn run(&self, action: SyncAction) {
        match action {
            SyncAction::Enable => self.enable(),
            SyncAction::Disable => {
                let _ = account::disable(&lock(&self.store));
                self.tell(WorkerMsg::Stop);
            }
            SyncAction::SyncNow => {
                self.tell(WorkerMsg::SyncNow);
                self.note_scripts_changed();
            }
            SyncAction::ConfirmAccount => {
                let hash = lock(&self.shared).pending_account.take();
                if let Some(hash) = hash {
                    let _ = account::bind(&lock(&self.store), &hash, now_ms());
                    self.tell(WorkerMsg::Restart);
                }
            }
            SyncAction::ChooseJoin(mode) => self.join(mode),
            SyncAction::ResolveAccount(choice) => self.resolve_account(choice),
            SyncAction::SetCategory(category, on) => {
                let store = lock(&self.store);
                let _ = store.set_category_enabled(category, on);
                if on {
                    let _ = store.reconcile_stored();
                }
                drop(store);
                if on {
                    // Records held back while it was off, and script changes made meanwhile.
                    let empty = crate::sync::core::RemoteBatch {
                        id: 0,
                        records: Vec::new(),
                        deletions: Vec::new(),
                    };
                    if let Some(report) = self.apply(&empty) {
                        lock(&self.shared).reports.push(report);
                    }
                    self.note_scripts_changed();
                    self.tell(WorkerMsg::Tick);
                }
            }
            SyncAction::ConfirmHeldDeletes => {
                let _ = lock(&self.store).release_held();
                self.tell(WorkerMsg::Tick);
            }
            SyncAction::KeepHeldDeletes => {
                let _ = lock(&self.store).drop_held();
            }
            SyncAction::DismissConflict(id) => {
                let _ = lock(&self.store).dismiss_conflict(id, now_ms());
            }
            SyncAction::DeleteCloudData => self.tell(WorkerMsg::DeleteCloudData),
            SyncAction::ResolveRemoteDeletion { reupload } => {
                let _ = account::resolve_remote_deletion(&lock(&self.store), reupload);
                if reupload {
                    self.tell(WorkerMsg::ClearPause);
                    self.tell(WorkerMsg::Restart);
                } else {
                    self.tell(WorkerMsg::Stop);
                }
            }
        }
    }

    fn enable(&self) {
        let Some(factory) = &self.factory else { return };
        let Ok(environment) = factory.environment() else {
            return;
        };
        let _ = account::enable(
            &lock(&self.store),
            &device_name(),
            &hardware_hash(),
            &environment,
        );
        let _ = account::check_device_identity(&lock(&self.store), &hardware_hash());
        self.start_worker();
    }

    fn join(&self, mode: JoinMode) {
        let Some(ctx) = self.apply_ctx() else { return };
        let backups = self.backups_dir.clone();
        let cache = self.script_cache.clone();
        let outcome = self.with_scripts(|sink| {
            let store = lock(&self.store);
            finish_join(&store, sink, &cache, mode, &backups, &ctx)
        });
        if let Ok(outcome) = outcome {
            lock(&self.shared).reports.push(outcome.report);
        }
        if mode == JoinMode::KeepLocalOnly {
            self.tell(WorkerMsg::Stop);
        } else {
            self.tell(WorkerMsg::Tick);
        }
    }

    fn resolve_account(&self, choice: AccountChoice) {
        let Some(hash) = lock(&self.shared).mismatched_account.take() else {
            return;
        };
        let resolution = account::resolve_account_change(
            &lock(&self.store),
            choice,
            &hash,
            &self.backups_dir,
            now_ms(),
        );
        match resolution {
            Ok(resolution) if resolution.needs_join => {
                self.tell(WorkerMsg::ClearPause);
                self.tell(WorkerMsg::Restart);
            }
            _ => self.tell(WorkerMsg::Stop),
        }
    }

    // MARK: View

    fn refresh_view(&self) {
        let (state, conflicts, status) = {
            let store = lock(&self.store);
            (
                store.sync_state().ok().flatten(),
                store.open_conflicts().unwrap_or_default(),
                store.outbox().unwrap_or_default(),
            )
        };
        let mut shared = lock(&self.shared);
        let Some(mut view) = shared.view.take() else {
            return;
        };
        view.enabled = state.as_ref().is_some_and(|s| s.enabled);
        view.environment = state.as_ref().and_then(|s| s.environment.clone());
        view.device_name = state.as_ref().map(|s| s.device_name.clone());
        view.last_send_ms = state.as_ref().and_then(|s| s.last_send_ms);
        view.last_fetch_ms = state.as_ref().and_then(|s| s.last_fetch_ms);
        use crate::store::sync::OutboxState;
        view.pending = status
            .iter()
            .filter(|r| r.state == OutboxState::Pending)
            .count();
        view.held = status
            .iter()
            .filter(|r| r.state == OutboxState::Held)
            .count();
        view.blocked = status
            .iter()
            .filter(|r| r.state == OutboxState::Blocked)
            .count();
        view.conflicts = conflicts;
        let off = lock(&self.store).disabled_categories();
        view.categories = Category::ALL
            .iter()
            .map(|c| (*c, !off.contains(c)))
            .collect();
        if !view.enabled {
            view.phase = Phase::Off;
            view.prompt = None;
        }
        let pending_account = shared.pending_account.is_some();
        let mismatched = shared.mismatched_account.is_some();
        drop(shared);

        view.prompt = if !view.enabled {
            None
        } else if mismatched || matches!(view.phase, Phase::Paused(PauseReason::AccountChanged)) {
            Some(SyncPrompt::AccountChanged)
        } else if matches!(view.phase, Phase::Paused(PauseReason::RemoteDataDeleted)) {
            Some(SyncPrompt::RemoteDataDeleted)
        } else if pending_account {
            Some(SyncPrompt::ConfirmAccount)
        } else if view.phase == Phase::AwaitingJoin {
            let local_scripts = self.with_scripts(|sink| sink.all().map_or(0, |all| all.len()));
            let summary = summarize(&lock(&self.store), local_scripts).ok();
            summary.map(SyncPrompt::Join)
        } else {
            None
        };
        lock(&self.shared).view = Some(view);
    }
}

fn run_worker<T: SyncTransport>(
    mut core: SyncCore<T>,
    rx: mpsc::Receiver<WorkerMsg>,
    shared: Arc<Mutex<Shared>>,
    store: SharedStore,
    repaint: Arc<dyn Fn() + Send + Sync>,
) {
    // CloudKit pushes wake the engine, but a Developer ID app cannot rely on them alone: also look
    // for other Macs' changes every few minutes.
    let mut last_poll = Instant::now();
    loop {
        let message = rx.recv_timeout(Duration::from_secs(1));
        if last_poll.elapsed() >= POLL_INTERVAL {
            last_poll = Instant::now();
            core.request_sync();
        }
        let mut effects = Vec::new();
        match message {
            Ok(WorkerMsg::Event(event)) => effects = core.handle(event),
            Ok(WorkerMsg::Applied(id)) => core.remote_applied(id),
            Ok(WorkerMsg::Start) => core.start(),
            Ok(WorkerMsg::Stop) => core.stop(),
            Ok(WorkerMsg::Restart) => {
                core.stop();
                core.start();
            }
            Ok(WorkerMsg::ClearPause) => core.clear_pause(),
            Ok(WorkerMsg::SyncNow) => core.request_sync(),
            Ok(WorkerMsg::DeleteCloudData) => core.delete_cloud_data(),
            Ok(WorkerMsg::Tick) | Err(RecvTimeoutError::Timeout) => core.tick(),
            Ok(WorkerMsg::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                core.stop();
                return;
            }
        }
        effects.extend(core.take_effects());
        let status = core.status();
        {
            let mut shared = lock(&shared);
            let had_effects = !effects.is_empty();
            shared.effects.extend(effects);
            if let Some(view) = shared.view.as_mut() {
                view.phase = status.phase;
                view.last_error = status.last_error.clone();
                view.cloud_deleted = status.cloud_deleted;
            }
            if had_effects {
                drop(shared);
                repaint();
            }
        }
        let _ = &store;
    }
}

/// `YYYY-MM-DD` of today, UTC, for naming conflict copies.
fn today() -> String {
    let days = now_ms().div_euclid(86_400_000);
    // Civil-from-days (Howard Hinnant), so no date crate is needed.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}")
}

/// What this Mac is called in the interface and in conflict copies.
pub fn device_name() -> String {
    #[cfg(target_os = "macos")]
    if let Ok(output) = std::process::Command::new("scutil")
        .args(["--get", "ComputerName"])
        .output()
    {
        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !name.is_empty() {
            return name;
        }
    }
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "This Mac".to_string())
}

/// A stable hash of this machine's hardware identity, to notice a database copied to another Mac.
pub fn hardware_hash() -> String {
    #[cfg(target_os = "macos")]
    if let Ok(output) = std::process::Command::new("ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        if let Some(uuid) = text
            .lines()
            .find(|line| line.contains("IOPlatformUUID"))
            .and_then(|line| line.split('"').nth(3))
        {
            return crate::sync::record::sha256_hex(uuid.as_bytes());
        }
    }
    crate::sync::record::sha256_hex(device_name().as_bytes())
}

#[cfg(test)]
mod tests;
