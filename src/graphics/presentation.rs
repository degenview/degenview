//! Evidence that frames reach the screen, which is what tells a graphics start from a blank window.
//!
//! Building the UI proves nothing: egui-wgpu skips a frame when the surface cannot be acquired
//! (lost, outdated, occluded) and eframe keeps running the UI regardless. Two things do prove it:
//!
//! - a **paint callback** (`Painted`) runs inside the render pass, which only exists once the
//!   surface texture was acquired. Frames painted continuously for `HEALTHY_AFTER`, and still
//!   painting, mean the renderer works. This needs no read-back, so it holds on virtual GPUs.
//! - the **screenshot** the first window asks for is answered once a frame was painted and the GPU
//!   finished with it, which is the strongest signal and the fastest where the driver supports it.
//!
//! `observe` reports each milestone once; what a milestone is worth is for the caller to decide.

use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use eframe::egui;
use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait};

/// How long frames must keep reaching the surface before the renderer counts as working.
const HEALTHY_AFTER: Duration = Duration::from_secs(10);
/// The latest painted frame must be this recent: frames that stopped are no evidence of now.
const FRESH: Duration = Duration::from_secs(2);
/// While nothing is confirmed, a window asks to be drawn again this often, so frames keep coming.
const REPAINT_EVERY: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Milestone {
    /// A window built its UI for the first time.
    Drawn,
    /// The screenshot of the first frame came back: it was painted and the GPU finished with it.
    Presented,
    /// Enough evidence that the renderer works.
    Healthy,
}

/// What one pass of a window shows.
struct Pass {
    /// Time since the process started.
    now: Duration,
    /// The screenshot asked for by the first window has come back.
    answered: bool,
    /// When the first and the latest painted frame happened.
    painted: Option<(Duration, Duration)>,
}

/// Which milestones were reached. Pure: `Watch` feeds it what the window shows.
struct Tracker {
    drawn: bool,
    presented: bool,
    healthy: bool,
}

impl Tracker {
    const fn new() -> Self {
        Self {
            drawn: false,
            presented: false,
            healthy: false,
        }
    }

    /// Nothing more to learn: later passes add no probe and ask for no repaint.
    fn finished(&self) -> bool {
        self.healthy
    }

    fn on_pass(&mut self, pass: &Pass) -> Vec<Milestone> {
        let mut reached = Vec::new();
        if !self.drawn {
            self.drawn = true;
            reached.push(Milestone::Drawn);
        }
        if pass.answered && !self.presented {
            self.presented = true;
            reached.push(Milestone::Presented);
        }
        let painted_long_enough = pass.painted.is_some_and(|(first, last)| {
            last.saturating_sub(first) >= HEALTHY_AFTER && pass.now.saturating_sub(last) <= FRESH
        });
        if !self.healthy && (self.presented || painted_long_enough) {
            self.healthy = true;
            reached.push(Milestone::Healthy);
        }
        reached
    }
}

/// When frames were painted, written from the render pass (no lock there) and read by `observe`.
struct Paints {
    /// Milliseconds since the process started, plus one; zero means none yet.
    first: AtomicU64,
    last: AtomicU64,
}

impl Paints {
    const fn new() -> Self {
        Self {
            first: AtomicU64::new(0),
            last: AtomicU64::new(0),
        }
    }

    fn record(&self, now: Duration) {
        let stamp = now.as_millis() as u64 + 1;
        let _ = self
            .first
            .compare_exchange(0, stamp, Ordering::Relaxed, Ordering::Relaxed);
        self.last.store(stamp, Ordering::Relaxed);
    }

    fn snapshot(&self) -> Option<(Duration, Duration)> {
        let first = self.first.load(Ordering::Relaxed);
        let last = self.last.load(Ordering::Relaxed);
        (first != 0).then(|| {
            (
                Duration::from_millis(first - 1),
                Duration::from_millis(last.saturating_sub(1)),
            )
        })
    }
}

static PAINTS: Paints = Paints::new();

fn since_start() -> Duration {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed()
}

/// Draws nothing; its `paint` running means a frame reached an acquired surface.
struct Painted;

impl CallbackTrait for Painted {
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        _render_pass: &mut egui_wgpu::wgpu::RenderPass<'static>,
        _resources: &CallbackResources,
    ) {
        PAINTS.record(since_start());
    }
}

/// Tags the screenshot the first window asks for, so no other screenshot is taken for it.
struct FrameProbe;

/// The first window to build its UI is the one that is watched.
struct Watch {
    tracker: Tracker,
    viewport: Option<egui::ViewportId>,
}

static WATCH: Mutex<Watch> = Mutex::new(Watch {
    tracker: Tracker::new(),
    viewport: None,
});

/// Called by every window while it builds its UI. Passes of windows other than the first are
/// ignored. Until the renderer is confirmed, the watched window adds the paint probe, asks for the
/// screenshot once, and keeps itself drawing; `reached` is called for each milestone, once.
pub fn observe(ctx: &egui::Context, mut reached: impl FnMut(Milestone)) {
    let Ok(mut watch) = WATCH.lock() else { return };
    if watch.tracker.finished() {
        return;
    }
    let first_pass = watch.viewport.is_none();
    if *watch.viewport.get_or_insert(ctx.viewport_id()) != ctx.viewport_id() {
        return;
    }
    let pass = Pass {
        now: since_start(),
        answered: !first_pass && answered(ctx),
        painted: PAINTS.snapshot(),
    };
    let milestones = watch.tracker.on_pass(&pass);
    let finished = watch.tracker.finished();
    drop(watch);

    if first_pass {
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
            FrameProbe,
        )));
        ctx.request_repaint();
    }
    if !finished {
        add_probe(ctx);
        // The screenshot is read back a frame or more later; an idle window would not ask for one.
        ctx.request_repaint_after(REPAINT_EVERY);
    }
    for milestone in milestones {
        reached(milestone);
    }
}

fn answered(ctx: &egui::Context) -> bool {
    ctx.input(|input| {
        input.events.iter().any(|event| match event {
            egui::Event::Screenshot { user_data, .. } => user_data
                .data
                .as_ref()
                .is_some_and(|data| data.is::<FrameProbe>()),
            _ => false,
        })
    })
}

fn add_probe(ctx: &egui::Context) {
    let layer = egui::LayerId::new(egui::Order::Background, egui::Id::new("graphics-probe"));
    // A few points, inside the window: a callback with no area is not painted.
    let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(4.0, 4.0));
    ctx.layer_painter(layer)
        .add(egui_wgpu::Callback::new_paint_callback(area, Painted));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    fn pass(now: u64, answered: bool, painted: Option<(u64, u64)>) -> Pass {
        Pass {
            now: secs(now),
            answered,
            painted: painted.map(|(first, last)| (secs(first), secs(last))),
        }
    }

    #[test]
    fn the_first_pass_is_the_first_window_drawn() {
        let mut tracker = Tracker::new();
        assert_eq!(tracker.on_pass(&pass(1, false, None)), [Milestone::Drawn]);
        assert!(tracker.on_pass(&pass(2, false, None)).is_empty());
    }

    #[test]
    fn ui_passes_without_painted_frames_never_make_the_renderer_healthy() {
        let mut tracker = Tracker::new();
        for now in 1..120 {
            tracker.on_pass(&pass(now, false, None));
        }
        assert!(!tracker.finished());
    }

    #[test]
    fn frames_painted_for_long_enough_and_still_painting_are_healthy() {
        let mut tracker = Tracker::new();
        tracker.on_pass(&pass(1, false, Some((1, 1))));
        assert!(tracker.on_pass(&pass(9, false, Some((1, 9)))).is_empty());
        assert_eq!(
            tracker.on_pass(&pass(11, false, Some((1, 11)))),
            [Milestone::Healthy]
        );
        assert!(tracker.finished());
    }

    #[test]
    fn frames_that_stopped_are_no_evidence_of_now() {
        let mut tracker = Tracker::new();
        tracker.on_pass(&pass(1, false, Some((1, 1))));
        // Painted from 1 s to 12 s, then nothing for 5 s while the UI kept running.
        assert!(tracker.on_pass(&pass(17, false, Some((1, 12)))).is_empty());
        assert!(!tracker.finished());
    }

    #[test]
    fn the_screenshot_reply_is_presented_and_healthy_at_once() {
        let mut tracker = Tracker::new();
        assert_eq!(
            tracker.on_pass(&pass(1, true, None)),
            [Milestone::Drawn, Milestone::Presented, Milestone::Healthy]
        );
        assert!(tracker.finished());
    }

    #[test]
    fn each_milestone_is_reached_once() {
        let mut tracker = Tracker::new();
        tracker.on_pass(&pass(1, false, None));
        assert_eq!(
            tracker.on_pass(&pass(2, true, None)),
            [Milestone::Presented, Milestone::Healthy]
        );
        assert!(tracker.on_pass(&pass(3, true, Some((1, 3)))).is_empty());
    }

    #[test]
    fn paints_remember_the_first_and_the_latest_frame() {
        let paints = Paints::new();
        assert_eq!(paints.snapshot(), None);
        paints.record(Duration::from_millis(0));
        paints.record(Duration::from_millis(4_000));
        paints.record(Duration::from_millis(9_500));
        assert_eq!(
            paints.snapshot(),
            Some((Duration::from_millis(0), Duration::from_millis(9_500)))
        );
    }
}
