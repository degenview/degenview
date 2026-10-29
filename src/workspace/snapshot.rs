//! Chart snapshots, from the click to the effect.
//!
//! The chart is frozen where the click is handled: the card paints itself onto a scratch context
//! and the result is tessellated, all on the UI thread within one frame, from the very data the
//! chart on screen is drawn from. Everything slow (the save panel, rasterizing, the PNG, the
//! upload) then runs on a worker (`service::snapshot_job`) holding no workspace lock, and the
//! outcome comes back through a channel polled each frame. The clipboard and the browser are the
//! window's to drive, so they run here, when the outcome arrives, and a message only says what
//! has happened.

use super::content::snapshot_images;
use super::*;
use crate::host::snapshot_host::{Hosting, SnapshotHost};
use crate::model::snapshot::{
    self, SnapshotAction, SnapshotDimensions, SnapshotHost as HostAddress, SnapshotRequest,
};
use crate::service::snapshot_job::{self, Outcome, Retry, SystemBackend};
use crate::service::{keychain_snapshot_token, timezone, work_queue};
use crate::ui::SnapshotInput;
use crate::ui::icons::Icon;
use crate::ui::kit;
use eframe::egui::{Context, Id};

/// A snapshot being made on a worker.
struct Running {
    action: SnapshotAction,
    receiver: Receiver<Outcome>,
    /// What the post says when the snapshot is shared on X.
    title: String,
    timeframe: String,
    host: Option<HostAddress>,
}

/// A hosted action that waits for the user to agree to the upload.
struct AwaitingConsent {
    request: SnapshotRequest,
    host: HostAddress,
}

/// An upload that failed in a way that may pass, waiting for the user's answer.
struct AwaitingRetry {
    retry: Retry,
    message: String,
    title: String,
    timeframe: String,
    host: HostAddress,
}

/// What a workspace keeps of its snapshots between frames.
#[derive(Default)]
pub(super) struct SnapshotState {
    running: Option<Running>,
    consent: Option<AwaitingConsent>,
    retry: Option<AwaitingRetry>,
    /// Stands in for the save panel, the file system and the network under test.
    backend: Option<Arc<dyn snapshot_job::Backend>>,
}

impl ChartWorkspace {
    /// Gives the workspace the snapshot settings every window shares. Cheap to call every frame.
    pub fn set_snapshots(&mut self, host: &SnapshotHost) {
        if self.snapshots.is_none() {
            self.snapshots = Some(host.clone());
        }
    }

    fn snapshot_prefs(&self) -> crate::model::snapshot::SnapshotPrefs {
        self.snapshots
            .as_ref()
            .map(SnapshotHost::prefs)
            .unwrap_or_default()
    }

    /// Whether a snapshot is being made: the menu offers nothing meanwhile.
    pub(super) fn snapshot_busy(&self) -> bool {
        self.snapshot.running.is_some()
            || self.snapshot.consent.is_some()
            || self.snapshot.retry.is_some()
    }

    /// What the camera menu needs to know about hosting.
    pub(super) fn snapshot_hosting(&self) -> Hosting {
        self.snapshots
            .as_ref()
            .map_or(Hosting::NotConfigured, SnapshotHost::hosting)
    }

    /// Takes the snapshot `action` asks for of chart `chart_id`.
    pub(super) fn begin_snapshot(&mut self, ctx: &Context, chart_id: u64, action: SnapshotAction) {
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
        let Some(host) = prefs.host() else {
            self.say(
                "Set a snapshot host in Settings ▸ Snapshots to share.",
                true,
            );
            return;
        };
        if prefs.has_consented(&host) {
            self.start_snapshot(ctx, request, Some(host));
        } else {
            self.snapshot.consent = Some(AwaitingConsent { request, host });
        }
    }

    fn start_snapshot(
        &mut self,
        ctx: &Context,
        request: SnapshotRequest,
        host: Option<HostAddress>,
    ) {
        let SnapshotRequest {
            chart_id,
            action,
            options,
        } = request;
        let Some(index) = self.chart_index(chart_id) else {
            return;
        };
        let replay_ms = if self.replay.is_active() {
            self.replay.current_ms()
        } else {
            None
        };
        let chart = &self.charts[index];
        let size = chart.chart.snapshot_size();
        let Some(dimensions) = SnapshotDimensions::fit(size.x, size.y, options.scale) else {
            self.say("There is no chart to take a picture of yet.", true);
            return;
        };
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
            hide_all_drawings: self.hide_all_drawings,
            replay_active: replay_ms.is_some(),
            include_trading: options.include_trading,
        };
        let scene = match chart
            .chart
            .snapshot_scene(&input, dimensions, ui::theme::surface())
        {
            Ok(scene) => scene,
            Err(error) => {
                self.say(error.message(), true);
                return;
            }
        };
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
        let (worker_ctx, worker_host) = (ctx.clone(), host.clone());
        let stand_in = self.snapshot.backend.clone();
        work_queue::SNAPSHOTS.submit(move || {
            let backend = stand_in.unwrap_or_else(|| system_backend(worker_host));
            let outcome = snapshot_job::run(action, &scene, &name, &*backend);
            let _ = sender.send(outcome);
            worker_ctx.request_repaint();
        });
        self.snapshot.running = Some(Running {
            action,
            receiver,
            title,
            timeframe,
            host,
        });
    }

    /// Uploads a kept image again.
    fn retry_snapshot(&mut self, ctx: &Context, waiting: AwaitingRetry) {
        let (sender, receiver) = mpsc::channel();
        let (worker_ctx, host, retry) = (ctx.clone(), waiting.host.clone(), waiting.retry);
        let action = retry.action;
        let stand_in = self.snapshot.backend.clone();
        work_queue::SNAPSHOTS.submit(move || {
            let backend = stand_in.unwrap_or_else(|| system_backend(Some(host)));
            let _ = sender.send(snapshot_job::retry(retry, &*backend));
            worker_ctx.request_repaint();
        });
        self.snapshot.running = Some(Running {
            action,
            receiver,
            title: waiting.title,
            timeframe: waiting.timeframe,
            host: Some(waiting.host),
        });
    }

    /// Carries out what a finished snapshot asks of the window.
    pub(super) fn poll_snapshot(&mut self, ctx: &Context) {
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
            } if running.host.is_some() => {
                self.snapshot.retry = running.host.map(|host| AwaitingRetry {
                    retry,
                    message,
                    title: running.title,
                    timeframe: running.timeframe,
                    host,
                });
            }
            Outcome::Failed { message, .. } => self.say(message, true),
        }
    }

    /// The upload's progress, and the questions it may stop to ask.
    pub(super) fn show_snapshot_sheets(&mut self, ctx: &Context) {
        if let Some(running) = &self.snapshot.running
            && running.action.is_hosted()
        {
            ui::chart_notice::show(
                ctx,
                Id::new(("snapshot-progress", self.id)),
                "Uploading snapshot…",
                false,
            );
        }
        if let Some(waiting) = &self.snapshot.consent {
            let message = format!(
                "To share this chart, DegenView uploads its image to {}. Anyone who has the link \
                 can open it. Saving or copying an image never uploads anything.",
                waiting.host.authority()
            );
            let outcome = kit::alert(
                ctx,
                Id::new(("snapshot-consent", self.id)),
                &kit::Alert {
                    icon: Icon::ShareUp,
                    tint: kit::ACCENT,
                    title: "Upload this chart image?",
                    message: &message,
                    accept: "Upload",
                    dismiss: "Cancel",
                },
            );
            match outcome {
                Some(kit::ConfirmOutcome::Confirmed) => {
                    if let Some(waiting) = self.snapshot.consent.take() {
                        if let Some(host) = &self.snapshots
                            && let Err(error) = host.consent(None, &waiting.host)
                        {
                            self.say(error, true);
                        }
                        self.start_snapshot(ctx, waiting.request, Some(waiting.host));
                    }
                }
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

/// The real backend for a job, with the token read here, on the worker: a Keychain prompt must
/// never stop the window.
fn system_backend(host: Option<HostAddress>) -> Arc<dyn snapshot_job::Backend> {
    Arc::new(SystemBackend::new(host, keychain_snapshot_token::token()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Kline;
    use crate::model::snapshot::{SharedSnapshot, UploadError};
    use crate::service::snapshot_export::ExportError;
    use crate::store::SharedStore;
    use std::path::{Path, PathBuf};

    #[derive(Default)]
    struct Fake {
        destination: Mutex<Vec<Result<Option<PathBuf>, ExportError>>>,
        written: Mutex<Vec<PathBuf>>,
        answers: Mutex<Vec<Result<SharedSnapshot, UploadError>>>,
        uploads: Mutex<usize>,
    }

    impl snapshot_job::Backend for Fake {
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
        let host = SnapshotHost::load(store);
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
            self.host.set_host(None, address).unwrap();
            if consented {
                self.host
                    .consent(None, &self.host.prefs().host().expect("a host"))
                    .unwrap();
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
            .set_scale(None, crate::model::snapshot::SnapshotScale::X1)
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
        assert!(url.contains("BTC%2FUSDT%201H%20chart"), "{url}");
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
        assert!(error && text.contains("refused the token"), "{text}");
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
        use crate::model::portfolio_card::{CardKind, PortfolioCard};
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
        let before = replaying.workspace.replay.current_ms();
        replaying.workspace.begin_snapshot(
            &replaying.ctx,
            replaying.chart_id,
            SnapshotAction::CopyImage,
        );
        let replayed = copied(replaying.finish());
        assert_eq!(
            replaying.workspace.replay.current_ms(),
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
        let (focused, hidden) = (rig.workspace.focused, rig.workspace.hide_all_drawings);
        for action in [SnapshotAction::CopyImage, SnapshotAction::CopyImage] {
            rig.workspace.begin_snapshot(&rig.ctx, rig.chart_id, action);
            rig.finish();
        }
        assert_eq!(rig.workspace.charts[index].candles, candles);
        assert_eq!(
            (rig.workspace.focused, rig.workspace.hide_all_drawings),
            (focused, hidden)
        );
    }
}
