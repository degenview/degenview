//! Chart snapshots, from the click to the effect.
//!
//! The chart is frozen where the click is handled: the card paints itself onto a scratch context
//! and the result is tessellated, all on the UI thread within one frame, from the very data the
//! chart on screen is drawn from. Everything slow (the save panel, rasterizing, the PNG, the
//! upload) then runs on a worker (`service::snapshot::job`) holding no workspace lock, and the
//! outcome comes back through a channel polled each frame. The clipboard and the browser are the
//! window's to drive, so they run here, when the outcome arrives, and a message only says what
//! has happened.

use crate::host::snapshot::{Hosting, SnapshotHost, TURN_ON_A_HOST};
use crate::model::chart::snapshot::{self, SnapshotAction, SnapshotDimensions, SnapshotRequest};
use crate::service::snapshot::job::{self, Outcome, Retry, SystemBackend};
use crate::service::snapshot::providers::Destination;
use crate::service::snapshot::raster;
use crate::service::{system::timezone, work_queue};
use crate::ui::icons::Icon;
use crate::ui::kit;
use crate::ui::{SnapshotError, SnapshotInput};
use crate::workspace::drawing::content::snapshot_images;
use crate::workspace::*;
use eframe::egui::{Context, Id};

/// The PNG of a picture made for an agent, or why it could not be made.
pub type PngReceiver = Receiver<Result<Vec<u8>, String>>;

/// A snapshot being made on a worker.
struct Running {
    action: SnapshotAction,
    receiver: Receiver<Outcome>,
    /// What the post says when the snapshot is shared on X.
    title: String,
    timeframe: String,
    /// Where the image went: only a hosted action has one.
    destination: Option<Destination>,
    /// When the work began (or began again, for a retry): a slow upload says so.
    started: Instant,
}

/// After this long an upload says it is still going, so a slow connection is not taken for a
/// hang.
const SLOW_AFTER: Duration = Duration::from_secs(8);

/// What the progress notice says of an upload to `label` that began `elapsed` ago.
fn progress_text(label: &str, elapsed: Duration) -> String {
    if elapsed >= SLOW_AFTER {
        format!("Still uploading to {label}… a slow connection can take a while")
    } else {
        format!("Uploading to {label}…")
    }
}

/// A hosted action that waits for the user to agree to the upload.
struct AwaitingConsent {
    request: SnapshotRequest,
    destination: Destination,
}

/// An upload that failed in a way that may pass, waiting for the user's answer.
struct AwaitingRetry {
    retry: Retry,
    message: String,
    title: String,
    timeframe: String,
    destination: Destination,
}

/// What a workspace keeps of its snapshots between frames.
#[derive(Default)]
pub(in crate::workspace) struct SnapshotState {
    running: Option<Running>,
    consent: Option<AwaitingConsent>,
    retry: Option<AwaitingRetry>,
    /// Stands in for the save panel, the file system and the network under test.
    backend: Option<Arc<dyn job::Backend>>,
    /// The camera menu asked for the Snapshots page of Settings; the app takes it this frame.
    pub(in crate::workspace) settings_requested: bool,
}

/// Why a chart could not be frozen into a picture.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("There is no such chart.")]
    NoChart,
    /// An agent asked for a chart that is a portfolio card, not a market.
    #[error("There is no such market chart.")]
    NoMarketChart,
    /// The chart has not been laid out on screen yet.
    #[error("There is no chart to take a picture of yet.")]
    NotLaidOut,
    #[error("{}", .0.message())]
    Scene(SnapshotError),
}

impl ChartWorkspace {
    /// Gives the workspace the snapshot settings every window shares. Cheap to call every frame.
    pub fn set_snapshots(&mut self, host: &SnapshotHost) {
        if self.snapshots.is_none() {
            self.snapshots = Some(host.clone());
        }
    }

    fn snapshot_prefs(&self) -> crate::model::chart::snapshot::SnapshotPrefs {
        self.snapshots
            .as_ref()
            .map(SnapshotHost::prefs)
            .unwrap_or_default()
    }

    /// Whether a snapshot is being made: the menu offers nothing meanwhile.
    pub(in crate::workspace) fn snapshot_busy(&self) -> bool {
        self.snapshot.running.is_some()
            || self.snapshot.consent.is_some()
            || self.snapshot.retry.is_some()
    }

    /// Whether an image is being drawn, saved or uploaded right now (not waiting for the user's
    /// answer to a question): what the camera's spinner says.
    pub(in crate::workspace) fn snapshot_working(&self) -> bool {
        self.snapshot.running.is_some()
    }

    /// What the camera menu needs to know about hosting.
    pub(in crate::workspace) fn snapshot_hosting(&self) -> Hosting {
        self.snapshots.as_ref().map_or_else(
            || Hosting::Unavailable(TURN_ON_A_HOST.to_owned()),
            SnapshotHost::hosting,
        )
    }

    /// Takes the snapshot `action` asks for of chart `chart_id`.
    pub(in crate::workspace) fn begin_snapshot(
        &mut self,
        ctx: &Context,
        chart_id: u64,
        action: SnapshotAction,
    ) {
        if self.snapshot_busy() {
            self.say("A snapshot is already being made.", true);
            return;
        }
        let Some(index) = self.chart_index(chart_id) else {
            return;
        };
        if !self.charts[index].is_market() {
            self.say("This chart cannot be saved as an image.", true);
            return;
        }
        // What the user asked for is fixed now: a setting changed while the picture is made
        // (or while a question waits) does not change it.
        let prefs = self.snapshot_prefs();
        let request = SnapshotRequest {
            chart_id,
            action,
            options: prefs.options(),
        };
        if !action.is_hosted() {
            self.start_snapshot(ctx, request, None);
            return;
        }
        let destination = match self.snapshot_hosting() {
            Hosting::Ready(destination) => destination,
            Hosting::Unavailable(reason) => {
                self.say(reason, true);
                return;
            }
        };
        if prefs.has_consented(&destination.consent_key()) {
            self.start_snapshot(ctx, request, Some(destination));
        } else {
            self.snapshot.consent = Some(AwaitingConsent {
                request,
                destination,
            });
        }
    }

    /// Freezes chart `chart_id` into a scene, at the scale `options` asks for (smaller if the
    /// chart is too large for it), from the very data the chart on screen is drawn from. Runs on
    /// the UI thread and reads, never changes, the workspace.
    fn capture_scene(
        &self,
        ctx: &Context,
        chart_id: u64,
        options: snapshot::SnapshotOptions,
    ) -> Result<(raster::Scene, SnapshotDimensions), CaptureError> {
        let index = self.chart_index(chart_id).ok_or(CaptureError::NoChart)?;
        let replay_ms = self.replay.cut_ms();
        let chart = &self.charts[index];
        let size = chart.chart.snapshot_size();
        let dimensions = SnapshotDimensions::fit(size.x, size.y, options.scale)
            .ok_or(CaptureError::NotLaidOut)?;
        let open_notes = chart.chart.open_notes();
        let images = snapshot_images(ctx, &chart.drawings);
        let input = SnapshotInput {
            symbol: &chart.symbol,
            source: chart.source,
            candles: displayed_candles(&chart.candles, replay_ms),
            scale_mode: chart.scale_mode,
            indicators: &chart.indicators,
            drawings: &chart.drawings,
            open_notes: &open_notes,
            images: Some(&images),
            hide_all_drawings: self.tools.hides_all(),
            replay_active: replay_ms.is_some(),
            include_trading: options.include_trading,
        };
        chart
            .chart
            .snapshot_scene(&input, dimensions, ui::theme::surface())
            .map(|scene| (scene, dimensions))
            .map_err(CaptureError::Scene)
    }

    /// Starts a picture of chart `chart_id` for an agent: the scene is frozen now, on the UI
    /// thread, and the rasterizing and PNG run on the snapshot worker. The PNG comes back through
    /// the receiver; nothing is saved, copied or uploaded, and no notice is shown. It is drawn
    /// as the Settings say (orders and positions only if the user included them) at `scale`.
    pub fn snapshot_png_for_mcp(
        &self,
        ctx: &Context,
        chart_id: u64,
        scale: snapshot::SnapshotScale,
    ) -> Result<(PngReceiver, SnapshotDimensions), CaptureError> {
        if !self
            .chart_index(chart_id)
            .is_some_and(|index| self.charts[index].is_market())
        {
            return Err(CaptureError::NoMarketChart);
        }
        let options = snapshot::SnapshotOptions {
            scale,
            ..self.snapshot_prefs().options()
        };
        let (scene, dimensions) = self.capture_scene(ctx, chart_id, options)?;
        let (sender, receiver) = mpsc::channel();
        work_queue::SNAPSHOTS.submit(move || {
            let png = raster::rasterize(&scene)
                .map_err(|_| "Could not draw the image.".to_owned())
                .and_then(|image| {
                    crate::service::snapshot::export::encode_png(&image)
                        .map_err(|error| error.message().to_owned())
                });
            let _ = sender.send(png);
        });
        Ok((receiver, dimensions))
    }

    fn start_snapshot(
        &mut self,
        ctx: &Context,
        request: SnapshotRequest,
        destination: Option<Destination>,
    ) {
        let SnapshotRequest {
            chart_id,
            action,
            options,
        } = request;
        let (scene, _) = match self.capture_scene(ctx, chart_id, options) {
            Ok(captured) => captured,
            Err(error) => {
                // An unknown chart was always silent; the rest is worth saying.
                if self.chart_index(chart_id).is_some() {
                    self.say(error.to_string(), true);
                }
                return;
            }
        };
        let Some(index) = self.chart_index(chart_id) else {
            return;
        };
        let chart = &self.charts[index];
        let timeframe = self.timeframe.label().to_owned();
        let title = chart.display_title();
        let now = now_ms();
        let name = snapshot::file_name(
            &chart.symbol,
            &timeframe,
            now,
            timezone::local_utc_offset_secs(now),
        );
        let (sender, receiver) = mpsc::channel();
        let (worker_ctx, worker_destination) = (ctx.clone(), destination.clone());
        let stand_in = self.snapshot.backend.clone();
        work_queue::SNAPSHOTS.submit(move || {
            let backend = stand_in.unwrap_or_else(|| system_backend(worker_destination));
            let outcome = job::run(action, &scene, &name, &*backend);
            let _ = sender.send(outcome);
            worker_ctx.request_repaint();
        });
        if destination.is_some() {
            // The last result ("Link copied") would sit over the progress notice, at the same place.
            self.context.notice = None;
        }
        self.snapshot.running = Some(Running {
            action,
            receiver,
            title,
            timeframe,
            destination,
            started: Instant::now(),
        });
    }

    /// Uploads a kept image again.
    fn retry_snapshot(&mut self, ctx: &Context, waiting: AwaitingRetry) {
        let (sender, receiver) = mpsc::channel();
        let (worker_ctx, destination, retry) =
            (ctx.clone(), waiting.destination.clone(), waiting.retry);
        let action = retry.action;
        let stand_in = self.snapshot.backend.clone();
        work_queue::SNAPSHOTS.submit(move || {
            let backend = stand_in.unwrap_or_else(|| system_backend(Some(destination)));
            let _ = sender.send(job::retry(retry, &*backend));
            worker_ctx.request_repaint();
        });
        self.context.notice = None;
        self.snapshot.running = Some(Running {
            action,
            receiver,
            title: waiting.title,
            timeframe: waiting.timeframe,
            destination: Some(waiting.destination),
            started: Instant::now(),
        });
    }

    /// Carries out what a finished snapshot asks of the window.
    pub(in crate::workspace) fn poll_snapshot(&mut self, ctx: &Context) {
        let Some(running) = &self.snapshot.running else {
            return;
        };
        let outcome = match running.receiver.try_recv() {
            Ok(outcome) => outcome,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(100));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.snapshot.running = None;
                self.say("The snapshot stopped before it finished.", true);
                return;
            }
        };
        let Some(running) = self.snapshot.running.take() else {
            return;
        };
        match outcome {
            Outcome::Cancelled => {}
            Outcome::Saved(path) => {
                let name = path.file_name().map_or_else(
                    || path.display().to_string(),
                    |name| name.to_string_lossy().into_owned(),
                );
                self.say(format!("Saved {name}"), false);
                // Without a save panel the file went to the downloads folder: show where.
                if !crate::platform::has_file_dialogs() {
                    crate::platform::reveal_in_file_manager(&path);
                }
            }
            Outcome::CopyImage(image) => {
                ctx.copy_image(image);
                self.say("Image copied", false);
            }
            Outcome::Hosted { action, shared } => match action {
                SnapshotAction::CopyLink => {
                    ctx.copy_text(shared.url);
                    self.say("Link copied", false);
                }
                SnapshotAction::OpenInTab => {
                    ctx.open_url(egui::OpenUrl::new_tab(shared.url));
                    self.say("Snapshot opened in your browser", false);
                }
                SnapshotAction::ShareOnX => {
                    let text = snapshot::share_text(&running.title, &running.timeframe);
                    ctx.open_url(egui::OpenUrl::new_tab(snapshot::x_compose_url(
                        &text,
                        &shared.url,
                    )));
                    self.say("Opened X with a link to the snapshot", false);
                }
                SnapshotAction::Download | SnapshotAction::CopyImage => {}
            },
            Outcome::Failed {
                message,
                retry: Some(retry),
            } if running.destination.is_some() => {
                self.snapshot.retry = running.destination.map(|destination| AwaitingRetry {
                    retry,
                    message,
                    title: running.title,
                    timeframe: running.timeframe,
                    destination,
                });
            }
            Outcome::Failed { message, .. } => self.say(message, true),
        }
    }

    /// What the progress notice says while an image is being uploaded; nothing for the rest (a
    /// save or a copy has no network to wait for).
    fn progress_notice(&self) -> Option<String> {
        let running = self
            .snapshot
            .running
            .as_ref()
            .filter(|running| running.action.is_hosted())?;
        let label = running
            .destination
            .as_ref()
            .map_or_else(|| "the snapshot host".to_owned(), Destination::label);
        Some(progress_text(&label, running.started.elapsed()))
    }

    /// The user agreed to upload to the destination asked about: remember it, and go on.
    fn confirm_consent(&mut self, ctx: &Context) {
        let Some(waiting) = self.snapshot.consent.take() else {
            return;
        };
        let consented = self
            .snapshots
            .as_ref()
            .map(|host| host.consent(None, &waiting.destination.consent_key()));
        self.start_snapshot(ctx, waiting.request, Some(waiting.destination));
        // After the start, which clears the last result: this is news about the answer just given
        // (it is asked again after a restart), so it must show.
        if let Some(Err(error)) = consented {
            self.say(error, true);
        }
    }

    /// The upload's progress, and the questions it may stop to ask.
    pub(in crate::workspace) fn show_snapshot_sheets(&mut self, ctx: &Context) {
        if let Some(text) = self.progress_notice() {
            // A result still on screen (an error that came with the start, say) keeps its place;
            // the progress sits over it instead of under it.
            ui::chart::notice::show_progress(
                ctx,
                Id::new(("snapshot-progress", self.id)),
                &text,
                self.context.notice.is_some(),
            );
        }
        if let Some(waiting) = &self.snapshot.consent {
            let note = waiting
                .destination
                .note()
                .map_or_else(String::new, |note| format!(" {note}"));
            let message = format!(
                "To share this chart, DegenView uploads its image to {}. {}{note} Saving or \
                 copying an image never uploads anything.",
                waiting.destination.label(),
                waiting.destination.notice()
            );
            let outcome = kit::alert(
                ctx,
                Id::new(("snapshot-consent", self.id)),
                &kit::Alert {
                    icon: Icon::ShareUp,
                    tint: kit::ORANGE,
                    title: "Upload this chart image?",
                    message: &message,
                    accept: "Upload",
                    dismiss: "Cancel",
                    cautious: true,
                },
            );
            match outcome {
                Some(kit::ConfirmOutcome::Confirmed) => self.confirm_consent(ctx),
                Some(kit::ConfirmOutcome::Cancelled) => self.snapshot.consent = None,
                None => {}
            }
        }
        if let Some(waiting) = &self.snapshot.retry {
            let outcome = kit::alert(
                ctx,
                Id::new(("snapshot-retry", self.id)),
                &kit::Alert {
                    icon: Icon::TriangleFill,
                    tint: kit::ORANGE,
                    title: "Could not upload the snapshot",
                    message: &waiting.message,
                    accept: "Try again",
                    dismiss: "Cancel",
                    cautious: false,
                },
            );
            match outcome {
                Some(kit::ConfirmOutcome::Confirmed) => {
                    if let Some(waiting) = self.snapshot.retry.take() {
                        self.retry_snapshot(ctx, waiting);
                    }
                }
                Some(kit::ConfirmOutcome::Cancelled) => self.snapshot.retry = None,
                None => {}
            }
        }
    }
}

/// The real backend for a job. It reads the provider's secrets itself, on the worker: a
/// Keychain prompt must never stop the window.
fn system_backend(destination: Option<Destination>) -> Arc<dyn job::Backend> {
    Arc::new(SystemBackend::new(destination))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Kline;
    use crate::model::chart::snapshot::SELF_HOSTED;
    use crate::model::chart::snapshot::{SharedSnapshot, UploadError};
    use crate::service::keychain::api_keys::MemoryBackend;
    use crate::service::keychain::snapshot_secrets::SnapshotSecrets;
    use crate::service::snapshot::export::ExportError;
    use crate::store::SharedStore;
    use std::path::{Path, PathBuf};

    #[derive(Default)]
    struct Fake {
        destination: Mutex<Vec<Result<Option<PathBuf>, ExportError>>>,
        written: Mutex<Vec<PathBuf>>,
        answers: Mutex<Vec<Result<SharedSnapshot, UploadError>>>,
        uploads: Mutex<usize>,
    }

    impl job::Backend for Fake {
        fn choose_destination(&self, _: &str) -> Result<Option<PathBuf>, ExportError> {
            self.destination.lock().unwrap().remove(0)
        }
        fn write(&self, path: &Path, _: &[u8]) -> Result<(), ExportError> {
            self.written.lock().unwrap().push(path.to_owned());
            Ok(())
        }
        fn upload(&self, _: &[u8]) -> Result<SharedSnapshot, UploadError> {
            *self.uploads.lock().unwrap() += 1;
            self.answers.lock().unwrap().remove(0)
        }
    }

    fn link(url: &str) -> Result<SharedSnapshot, UploadError> {
        Ok(SharedSnapshot {
            url: url.to_owned(),
            delete_url: None,
            expires_at: None,
        })
    }

    struct Rig {
        ctx: egui::Context,
        workspace: ChartWorkspace,
        host: SnapshotHost,
        fake: Arc<Fake>,
        chart_id: u64,
    }

    fn rig() -> Rig {
        let store: SharedStore = Arc::new(Mutex::new(
            crate::store::Store::open_in_memory().expect("in-memory store"),
        ));
        let book = WatchlistQuoteBook::default();
        let mut workspace = ChartWorkspace::empty(
            1,
            Timeframe::Hour,
            Arc::new(Mutex::new(WatchlistStore::new(store.clone()))),
            book.clone(),
            WatchlistQuoteCoordinator::new(book, Arc::new(|| {})),
            store.clone(),
        );
        let ctx = egui::Context::default();
        let added =
            workspace.add_chart_for(&InstrumentId::new(DataSource::Binance, "BTCUSDT"), &ctx);
        let index = workspace.chart_index(added.chart_id).expect("the chart");
        workspace.charts[index].candles = (0..200)
            .map(|i| Kline {
                open_time: i * 3_600_000,
                close_time: (i + 1) * 3_600_000 - 1,
                open: 100.0 + i as f64,
                high: 103.0 + i as f64,
                low: 98.0 + i as f64,
                close: 101.0 + i as f64,
                volume: 5.0,
            })
            .collect();
        workspace.charts[index]
            .chart
            .set_snapshot_size_for_tests(egui::vec2(640.0, 400.0));
        // Secure storage of its own: tests do not share a key.
        let host = SnapshotHost::load_with(
            store,
            Arc::new(SnapshotSecrets::with(
                Box::new(MemoryBackend::default()),
                Box::new(|_| None),
            )),
        );
        workspace.set_snapshots(&host);
        let fake = Arc::new(Fake::default());
        workspace.snapshot.backend = Some(fake.clone());
        Rig {
            ctx,
            workspace,
            host,
            fake,
            chart_id: added.chart_id,
        }
    }

    impl Rig {
        fn share_to(&self, address: &str, consented: bool) {
            self.host.set_provider(None, Some(SELF_HOSTED)).unwrap();
            self.host
                .set_fields(
                    None,
                    SELF_HOSTED,
                    &std::collections::BTreeMap::from([("host".to_owned(), address.to_owned())]),
                )
                .unwrap();
            if consented {
                let Hosting::Ready(destination) = self.host.hosting() else {
                    panic!("a host was set");
                };
                self.host.consent(None, &destination.consent_key()).unwrap();
            }
        }

        /// Turns ImgBB on with a key saved.
        fn share_to_imgbb(&self, consented: bool) {
            self.host.set_provider(None, Some("imgbb")).unwrap();
            self.host.save_secret("imgbb", "key", "abc123").unwrap();
            if consented {
                self.host.consent(None, "api.imgbb.com").unwrap();
            }
        }

        /// Runs frames until the snapshot finishes, and returns what the window was asked to do.
        /// A snapshot takes about two seconds alone and the tests share one work queue, so a
        /// loaded machine running them all at once needs far longer than that; a hung one waits a
        /// minute and fails.
        fn finish(&mut self) -> Vec<egui::OutputCommand> {
            let mut commands = Vec::new();
            for _ in 0..6_000 {
                let mut out = self.ctx.run_ui(egui::RawInput::default(), |_| {
                    self.workspace.poll_snapshot(&self.ctx);
                });
                out.textures_delta.clear();
                commands.extend(std::mem::take(&mut out.platform_output.commands));
                if self.workspace.snapshot.running.is_none() {
                    return commands;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("the snapshot did not finish");
        }

        fn notice(&self) -> Option<(String, bool)> {
            self.workspace
                .context
                .notice
                .as_ref()
                .map(|notice| (notice.text.clone(), notice.error))
        }
    }

    #[test]
    fn copying_the_image_asks_the_window_for_it_and_then_says_so() {
        let mut rig = rig();
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::CopyImage);
        assert!(rig.workspace.snapshot_busy());
        assert_eq!(rig.notice(), None, "nothing is claimed yet");
        let commands = rig.finish();
        let copied = commands.iter().find_map(|command| match command {
            egui::OutputCommand::CopyImage(image) => Some(image.size),
            _ => None,
        });
        assert_eq!(copied, Some([1280, 800]), "the chart at the default 2×");
        assert_eq!(rig.notice(), Some(("Image copied".into(), false)));
        assert!(!rig.workspace.snapshot_busy());
    }

    #[test]
    fn the_scale_chosen_in_settings_is_the_scale_drawn() {
        let mut rig = rig();
        rig.host
            .set_scale(None, crate::model::chart::snapshot::SnapshotScale::X1)
            .unwrap();
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::CopyImage);
        let commands = rig.finish();
        assert!(commands.iter().any(|command| matches!(
            command,
            egui::OutputCommand::CopyImage(image) if image.size == [640, 400]
        )));
    }

    #[test]
    fn download_saves_where_the_user_chose_and_a_cancel_says_nothing() {
        let mut rig = rig();
        rig.fake
            .destination
            .lock()
            .unwrap()
            .push(Ok(Some(PathBuf::from("/tmp/btc.png"))));
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::Download);
        rig.finish();
        assert_eq!(
            *rig.fake.written.lock().unwrap(),
            [PathBuf::from("/tmp/btc.png")]
        );
        assert_eq!(
            rig.notice().map(|(text, _)| text).as_deref(),
            Some("Saved btc.png")
        );

        rig.workspace.context.notice = None;
        rig.fake.destination.lock().unwrap().push(Ok(None));
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::Download);
        rig.finish();
        assert_eq!(rig.notice(), None, "a cancelled panel is not an error");
        assert_eq!(rig.fake.written.lock().unwrap().len(), 1);
    }

    #[test]
    fn sharing_needs_a_host_and_uploads_nothing_without_one() {
        let mut rig = rig();
        for action in [
            SnapshotAction::CopyLink,
            SnapshotAction::OpenInTab,
            SnapshotAction::ShareOnX,
        ] {
            rig.workspace.begin_snapshot(&rig.ctx, rig.chart_id, action);
            assert!(!rig.workspace.snapshot_busy(), "{action:?}");
            let (text, error) = rig.notice().expect("a reason");
            assert!(error && text.contains("Settings"), "{text}");
        }
        assert_eq!(*rig.fake.uploads.lock().unwrap(), 0);
    }

    #[test]
    fn the_camera_spins_while_work_is_done_and_not_while_a_question_waits() {
        let mut rig = rig();
        rig.share_to("https://snap.example.com/up", false);
        assert!(!rig.workspace.snapshot_working());
        // The first upload asks first: the menu is busy (nothing else can start), but nothing is
        // being done yet, so nothing turns.
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::CopyLink);
        assert!(rig.workspace.snapshot_busy());
        assert!(
            !rig.workspace.snapshot_working(),
            "waiting for the user is not work"
        );
        rig.workspace.snapshot.consent = None;
        rig.host.consent(None, "snap.example.com").unwrap();
        rig.fake
            .answers
            .lock()
            .unwrap()
            .push(link("https://s.example/a.png"));
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::CopyLink);
        assert!(rig.workspace.snapshot_working());
        rig.finish();
        assert!(!rig.workspace.snapshot_working());
    }

    #[test]
    fn a_host_with_a_saved_token_names_it_in_the_question_asked_before_the_first_upload() {
        let mut rig = rig();
        rig.share_to("https://snap.example.com/up", false);
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::CopyLink);
        let without = rig.workspace.snapshot.consent.as_ref().expect("asks");
        assert_eq!(without.destination.note(), None);
        rig.workspace.snapshot.consent = None;
        rig.host
            .save_secret(SELF_HOSTED, "token", "tok123")
            .unwrap();
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::CopyLink);
        let with = rig.workspace.snapshot.consent.as_ref().expect("asks");
        assert!(
            with.destination
                .note()
                .is_some_and(|note| note.contains("access token"))
        );
    }

    #[test]
    fn a_consent_that_could_not_be_saved_is_still_said_when_the_upload_goes_ahead() {
        let mut rig = rig();
        rig.share_to("https://snap.example.com/up", false);
        rig.workspace.say("Link copied", false);
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::CopyLink);
        assert!(rig.workspace.snapshot.consent.is_some(), "asks first");
        // The settings database stops answering between the question and the answer.
        let store = rig.workspace.store.clone();
        let _ = std::thread::spawn(move || {
            let _held = store.lock().unwrap();
            panic!("poisons the lock, as a crash while it is held would");
        })
        .join();
        rig.fake
            .answers
            .lock()
            .unwrap()
            .push(link("https://s.example/a.png"));
        rig.workspace.confirm_consent(&rig.ctx);
        assert!(rig.workspace.snapshot_working(), "the upload goes ahead");
        let (text, error) = rig.notice().expect("what went wrong is said");
        assert!(error && text.contains("settings database"), "{text}");
        rig.finish();
    }

    #[test]
    fn a_slow_upload_says_it_is_still_going() {
        assert_eq!(
            progress_text("ImgBB", Duration::from_secs(1)),
            "Uploading to ImgBB…"
        );
        assert_eq!(
            progress_text("ImgBB", SLOW_AFTER - Duration::from_millis(1)),
            "Uploading to ImgBB…"
        );
        let slow = progress_text("snap.example.com", SLOW_AFTER);
        assert!(
            slow.starts_with("Still uploading to snap.example.com"),
            "{slow}"
        );
    }

    #[test]
    fn an_upload_in_progress_says_where_the_image_is_going_until_it_is_done() {
        let mut rig = rig();
        rig.share_to("https://snap.example.com/up", true);
        assert_eq!(rig.workspace.progress_notice(), None, "nothing yet");
        rig.fake
            .answers
            .lock()
            .unwrap()
            .push(link("https://s.example/a.png"));
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::CopyLink);
        assert_eq!(
            rig.workspace.progress_notice().as_deref(),
            Some("Uploading to snap.example.com…")
        );
        // Long enough, and it says it is still going; the clock is the upload's own.
        let running = rig.workspace.snapshot.running.as_mut().expect("running");
        running.started = Instant::now()
            .checked_sub(SLOW_AFTER + Duration::from_secs(1))
            .expect("a clock that has run for nine seconds");
        assert!(
            rig.workspace
                .progress_notice()
                .is_some_and(|text| text.starts_with("Still uploading to snap.example.com"))
        );
        rig.finish();
        assert_eq!(
            rig.workspace.progress_notice(),
            None,
            "gone with the upload"
        );
    }

    #[test]
    fn imgbb_is_named_in_the_progress_notice() {
        let mut rig = rig();
        rig.share_to_imgbb(true);
        rig.fake
            .answers
            .lock()
            .unwrap()
            .push(link("https://i.ibb.co/a/b.png"));
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::OpenInTab);
        assert_eq!(
            rig.workspace.progress_notice().as_deref(),
            Some("Uploading to ImgBB…")
        );
        rig.finish();
    }

    #[test]
    fn saving_and_copying_the_image_show_no_upload_progress() {
        let mut rig = rig();
        rig.fake
            .destination
            .lock()
            .unwrap()
            .push(Ok(Some(PathBuf::from("/tmp/btc.png"))));
        for action in [SnapshotAction::Download, SnapshotAction::CopyImage] {
            rig.workspace.begin_snapshot(&rig.ctx, rig.chart_id, action);
            assert!(rig.workspace.snapshot_busy());
            assert_eq!(rig.workspace.progress_notice(), None, "{action:?}");
            rig.finish();
        }
    }

    #[test]
    fn an_old_result_does_not_sit_over_the_progress_of_a_new_upload() {
        let mut rig = rig();
        rig.share_to("https://snap.example.com/up", true);
        rig.workspace.say("Link copied", false);
        rig.fake
            .answers
            .lock()
            .unwrap()
            .push(link("https://s.example/a.png"));
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::CopyLink);
        assert_eq!(
            rig.notice(),
            None,
            "the old notice made way for the progress"
        );
        rig.finish();
        assert_eq!(
            rig.notice(),
            Some(("Link copied".into(), false)),
            "and the result says its own"
        );

        // A save or a copy keeps what was said: it has no progress to show.
        rig.workspace.say("Something else", false);
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::CopyImage);
        assert_eq!(rig.notice(), Some(("Something else".into(), false)));
        rig.finish();
    }

    #[test]
    fn imgbb_turned_on_without_a_key_says_so_and_uploads_nothing() {
        let mut rig = rig();
        rig.host.set_provider(None, Some("imgbb")).unwrap();
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::CopyLink);
        assert!(!rig.workspace.snapshot_busy());
        let (text, error) = rig.notice().expect("a reason");
        assert!(error && text.contains("ImgBB API key"), "{text}");
        assert!(matches!(
            rig.workspace.snapshot_hosting(),
            Hosting::Unavailable(reason) if reason.contains("ImgBB")
        ));
        assert_eq!(*rig.fake.uploads.lock().unwrap(), 0);
    }

    #[test]
    fn the_first_upload_to_imgbb_asks_by_name_and_remembers_the_answer_for_the_service() {
        let mut rig = rig();
        rig.share_to_imgbb(false);
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::CopyLink);
        let waiting = rig.workspace.snapshot.consent.as_ref().expect("asks first");
        assert_eq!(waiting.destination.label(), "ImgBB");
        assert_eq!(waiting.destination.consent_key(), "api.imgbb.com");
        assert!(waiting.destination.notice().contains("ImgBB's terms"));
        assert_eq!(*rig.fake.uploads.lock().unwrap(), 0);
        // After the user agrees, the next one goes straight out.
        rig.workspace.snapshot.consent = None;
        rig.host.consent(None, "api.imgbb.com").unwrap();
        rig.fake
            .answers
            .lock()
            .unwrap()
            .push(link("https://i.ibb.co/a/b.png"));
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::CopyLink);
        assert!(rig.workspace.snapshot.consent.is_none());
        rig.finish();
        assert_eq!(*rig.fake.uploads.lock().unwrap(), 1);
    }

    #[test]
    fn agreeing_to_a_host_of_your_own_is_not_agreeing_to_imgbb() {
        let mut rig = rig();
        rig.share_to("https://snap.example.com/up", true);
        rig.share_to_imgbb(false);
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::CopyLink);
        assert!(rig.workspace.snapshot.consent.is_some());
    }

    #[test]
    fn the_first_upload_to_a_host_waits_for_the_user_and_sends_nothing_before() {
        let mut rig = rig();
        rig.share_to("https://snap.example.com/up", false);
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::CopyLink);
        assert!(rig.workspace.snapshot.consent.is_some());
        assert!(
            rig.workspace.snapshot.running.is_none(),
            "nothing is drawn or sent yet"
        );
        assert_eq!(*rig.fake.uploads.lock().unwrap(), 0);
        // While it waits, nothing else starts.
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::CopyImage);
        assert!(rig.workspace.snapshot.running.is_none());
    }

    #[test]
    fn an_approved_host_gets_the_image_and_the_link_reaches_the_clipboard() {
        let mut rig = rig();
        rig.share_to("https://snap.example.com/up", true);
        rig.fake
            .answers
            .lock()
            .unwrap()
            .push(link("https://cdn.example.com/s/abc.png"));
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::CopyLink);
        let commands = rig.finish();
        assert_eq!(*rig.fake.uploads.lock().unwrap(), 1);
        assert!(commands.iter().any(|command| matches!(
            command,
            egui::OutputCommand::CopyText(text) if text == "https://cdn.example.com/s/abc.png"
        )));
        assert_eq!(rig.notice(), Some(("Link copied".into(), false)));
    }

    #[test]
    fn open_in_new_tab_opens_the_hosted_image() {
        let mut rig = rig();
        rig.share_to("https://snap.example.com/up", true);
        rig.fake
            .answers
            .lock()
            .unwrap()
            .push(link("https://cdn.example.com/s/abc.png"));
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::OpenInTab);
        let commands = rig.finish();
        let opened: Vec<_> = commands
            .iter()
            .filter_map(|command| match command {
                egui::OutputCommand::OpenUrl(open) => Some((open.url.clone(), open.new_tab)),
                _ => None,
            })
            .collect();
        assert_eq!(
            opened,
            [("https://cdn.example.com/s/abc.png".to_owned(), true)]
        );
    }

    #[test]
    fn sharing_on_x_opens_the_composer_with_the_chart_and_the_link() {
        let mut rig = rig();
        rig.share_to("https://snap.example.com/up", true);
        rig.fake
            .answers
            .lock()
            .unwrap()
            .push(link("https://cdn.example.com/s/abc.png"));
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::ShareOnX);
        let commands = rig.finish();
        let url = commands
            .iter()
            .find_map(|command| match command {
                egui::OutputCommand::OpenUrl(open) => Some(open.url.clone()),
                _ => None,
            })
            .expect("the composer");
        assert!(url.starts_with("https://x.com/intent/post?text="), "{url}");
        assert!(url.contains("BTCUSDT%201H%20chart"), "{url}");
        assert!(
            url.ends_with("&url=https%3A%2F%2Fcdn.example.com%2Fs%2Fabc.png"),
            "{url}"
        );
    }

    #[test]
    fn a_failed_upload_opens_nothing_copies_nothing_and_offers_a_retry() {
        let mut rig = rig();
        rig.share_to("https://snap.example.com/up", true);
        rig.fake
            .answers
            .lock()
            .unwrap()
            .push(Err(UploadError::Offline));
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::OpenInTab);
        let commands = rig.finish();
        assert!(
            commands.is_empty(),
            "no link was copied and no page was opened: {commands:?}"
        );
        let waiting = rig
            .workspace
            .snapshot
            .retry
            .take()
            .expect("a retry is offered");
        assert!(waiting.message.contains("Could not reach"));
        // Trying again sends the kept image and, this time, opens the page.
        rig.fake
            .answers
            .lock()
            .unwrap()
            .push(link("https://cdn.example.com/s/again.png"));
        rig.workspace.retry_snapshot(&rig.ctx, waiting);
        let commands = rig.finish();
        assert_eq!(*rig.fake.uploads.lock().unwrap(), 2);
        assert!(commands.iter().any(|command| matches!(
            command,
            egui::OutputCommand::OpenUrl(open) if open.url == "https://cdn.example.com/s/again.png"
        )));
    }

    #[test]
    fn a_failure_that_will_not_pass_is_just_said() {
        let mut rig = rig();
        rig.share_to("https://snap.example.com/up", true);
        rig.fake
            .answers
            .lock()
            .unwrap()
            .push(Err(UploadError::Unauthorized));
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::CopyLink);
        let commands = rig.finish();
        assert!(commands.is_empty());
        assert!(rig.workspace.snapshot.retry.is_none());
        let (text, error) = rig.notice().expect("a message");
        assert!(error && text.contains("refused the credentials"), "{text}");
    }

    #[test]
    fn only_one_snapshot_is_made_at_a_time() {
        let mut rig = rig();
        rig.fake
            .destination
            .lock()
            .unwrap()
            .push(Ok(Some(PathBuf::from("/tmp/a.png"))));
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::Download);
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::CopyImage);
        assert_eq!(
            rig.notice(),
            Some(("A snapshot is already being made.".into(), true))
        );
        rig.finish();
        assert_eq!(rig.fake.written.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_chart_that_was_never_drawn_has_no_snapshot() {
        let mut rig = rig();
        let index = rig.workspace.chart_index(rig.chart_id).unwrap();
        rig.workspace.charts[index]
            .chart
            .set_snapshot_size_for_tests(egui::Vec2::ZERO);
        rig.workspace
            .begin_snapshot(&rig.ctx, rig.chart_id, SnapshotAction::CopyImage);
        assert!(!rig.workspace.snapshot_busy());
        assert!(rig.notice().is_some_and(|(_, error)| error));
    }

    #[test]
    fn a_portfolio_card_is_not_a_chart_to_photograph() {
        use crate::model::portfolio::card::{CardKind, PortfolioCard};
        let mut rig = rig();
        let card = rig
            .workspace
            .add_card(PortfolioCard::new(CardKind::TopPerformers, None))
            .chart_id;
        rig.workspace
            .begin_snapshot(&rig.ctx, card, SnapshotAction::CopyImage);
        assert!(!rig.workspace.snapshot_busy());
        assert!(rig.notice().is_some_and(|(_, error)| error));
    }

    #[test]
    fn the_snapshot_is_of_the_chart_asked_for_in_its_own_window() {
        // Two workspaces, as two windows hold: each answers for its own chart only.
        let mut first = rig();
        let mut second = rig();
        let index = second.workspace.chart_index(second.chart_id).unwrap();
        second.workspace.charts[index]
            .chart
            .set_snapshot_size_for_tests(egui::vec2(320.0, 200.0));
        first
            .workspace
            .begin_snapshot(&first.ctx, first.chart_id, SnapshotAction::CopyImage);
        second
            .workspace
            .begin_snapshot(&second.ctx, second.chart_id, SnapshotAction::CopyImage);
        let size = |commands: Vec<egui::OutputCommand>| {
            commands.into_iter().find_map(|command| match command {
                egui::OutputCommand::CopyImage(image) => Some(image.size),
                _ => None,
            })
        };
        assert_eq!(size(first.finish()), Some([1280, 800]));
        assert_eq!(size(second.finish()), Some([640, 400]));
        // A chart id from another workspace is nothing here.
        first
            .workspace
            .begin_snapshot(&first.ctx, 999, SnapshotAction::CopyImage);
        assert!(!first.workspace.snapshot_busy());
    }

    /// The picture the window was asked to copy.
    fn copied(commands: Vec<egui::OutputCommand>) -> egui::ColorImage {
        commands
            .into_iter()
            .find_map(|command| match command {
                egui::OutputCommand::CopyImage(image) => Some(image),
                _ => None,
            })
            .expect("an image to copy")
    }

    #[test]
    fn a_replay_snapshot_holds_only_the_revealed_candles_and_leaves_the_replay_alone() {
        let timeline = |rig: &Rig| -> Vec<i64> {
            let index = rig.workspace.chart_index(rig.chart_id).unwrap();
            rig.workspace.charts[index]
                .candles
                .iter()
                .map(|bar| bar.open_time)
                .collect()
        };
        // Replaying from candle 120 of 200 …
        let mut replaying = rig();
        let times = timeline(&replaying);
        let cut = times[120];
        replaying
            .workspace
            .replay
            .start(cut, "BTCUSDT", times.clone());
        assert!(replaying.workspace.replay.is_active());
        let before = replaying.workspace.replay.cut_ms();
        replaying.workspace.begin_snapshot(
            &replaying.ctx,
            replaying.chart_id,
            SnapshotAction::CopyImage,
        );
        let replayed = copied(replaying.finish());
        assert_eq!(
            replaying.workspace.replay.cut_ms(),
            before,
            "the replay did not move"
        );
        assert!(replaying.workspace.replay.is_active());

        // … is the picture of a chart that holds nothing after it …
        let mut truncated = rig();
        let index = truncated.workspace.chart_index(truncated.chart_id).unwrap();
        truncated.workspace.charts[index]
            .candles
            .retain(|bar| bar.open_time <= cut);
        truncated.workspace.begin_snapshot(
            &truncated.ctx,
            truncated.chart_id,
            SnapshotAction::CopyImage,
        );
        assert_eq!(replayed.pixels, copied(truncated.finish()).pixels);

        // … and not of the whole chart.
        let mut whole = rig();
        whole
            .workspace
            .begin_snapshot(&whole.ctx, whole.chart_id, SnapshotAction::CopyImage);
        assert_ne!(replayed.pixels, copied(whole.finish()).pixels);
    }

    #[test]
    fn a_snapshot_never_changes_what_the_workspace_holds() {
        let mut rig = rig();
        let index = rig.workspace.chart_index(rig.chart_id).unwrap();
        let candles = rig.workspace.charts[index].candles.clone();
        let (focused, hidden) = (rig.workspace.focused, rig.workspace.tools.hides_all());
        for action in [SnapshotAction::CopyImage, SnapshotAction::CopyImage] {
            rig.workspace.begin_snapshot(&rig.ctx, rig.chart_id, action);
            rig.finish();
        }
        assert_eq!(rig.workspace.charts[index].candles, candles);
        assert_eq!(
            (rig.workspace.focused, rig.workspace.tools.hides_all()),
            (focused, hidden)
        );
    }

    #[test]
    fn a_png_for_an_agent_is_a_real_picture_made_without_any_effect_on_the_window() {
        use crate::model::chart::snapshot::SnapshotScale;
        let rig = rig();
        let png_of = |scale| {
            let (receiver, dimensions) = rig
                .workspace
                .snapshot_png_for_mcp(&rig.ctx, rig.chart_id, scale)
                .expect("the chart can be pictured");
            let png = receiver
                .recv_timeout(Duration::from_secs(60))
                .expect("the worker answers")
                .expect("a picture");
            (png, dimensions)
        };
        let (small, small_size) = png_of(SnapshotScale::X1);
        let (large, large_size) = png_of(SnapshotScale::X2);
        assert_eq!(&small[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!((small_size.width, small_size.height), (640, 400));
        assert_eq!((large_size.width, large_size.height), (1280, 800));
        assert!(large.len() > small.len(), "a larger picture holds more");
        // IHDR says the same size as the dimensions.
        let width = u32::from_be_bytes(small[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(small[20..24].try_into().unwrap());
        assert_eq!((width, height), (640, 400));
        // Nothing is shown, saved, copied or left running.
        assert_eq!(rig.notice(), None);
        assert!(!rig.workspace.snapshot_busy());
        assert!(rig.fake.written.lock().unwrap().is_empty());
        // An unknown chart, and a chart that has not been drawn yet, say why instead.
        assert!(
            rig.workspace
                .snapshot_png_for_mcp(&rig.ctx, 999, SnapshotScale::X1)
                .is_err()
        );
    }
}
