//! Select portion, driven by real pointer events over a workspace's grid.

use super::select::{Input, Selecting, Step};
use super::*;
use crate::host::paper::PaperHost;
use crate::model::Kline;
use crate::model::Tool;
use crate::model::chart::snapshot::studio::{Area, Style};
use crate::ui::ChartCard;
use crate::ui::snapshot_studio::capture::{Capture, ChartCapture, LayoutCapture};
use crate::workspace::test_support::empty_workspace;
use crate::workspace::test_support::rectangle;
use eframe::egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2, pos2, vec2};

pub(super) const HOUR: i64 = 3_600_000;
pub(super) const SIZE: Vec2 = Vec2::new(1200.0, 800.0);

pub(super) fn series(count: usize) -> Vec<Kline> {
    (0..count)
        .map(|index| {
            let close = 100.0 + (index as f64 * 0.31).sin() * 20.0 + index as f64 * 0.2;
            Kline {
                open_time: index as i64 * HOUR,
                close_time: (index as i64 + 1) * HOUR - 1,
                open: close - 1.0,
                high: close + 2.0,
                low: close - 3.0,
                close,
                volume: 10.0,
            }
        })
        .collect()
}

/// How long a test waits for a snapshot job before it calls it hung. A picture takes seconds in a
/// debug build with the machine to itself, and these tests share one work queue with every other
/// test running, so the wait is a time, not a number of frames: a loaded machine runs frames
/// quickly while the job it waits for is still queued.
pub(super) const JOB_DEADLINE: std::time::Duration = std::time::Duration::from_secs(180);

/// Calls `step` until it says it is done, pausing a moment between calls, and fails the test with
/// `what` if the job is still not done after [`JOB_DEADLINE`].
pub(super) fn wait_until(what: &str, mut step: impl FnMut() -> bool) {
    let started = std::time::Instant::now();
    while !step() {
        assert!(started.elapsed() < JOB_DEADLINE, "{what}");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// A workspace drawn on a window that takes pointer events, one chart per symbol.
pub(super) struct Rig {
    pub(super) ctx: egui::Context,
    pub(super) workspace: ChartWorkspace,
    pub(super) paper: PaperHost,
    pub(super) ids: Vec<u64>,
    pub(super) time: f64,
    /// The window has the keyboard focus.
    pub(super) focused: bool,
    /// Draws the Studio over the grid, and gives the workspace the keyboard, as a frame does.
    pub(super) studio: bool,
    pub(super) cursor: egui::CursorIcon,
    /// The textures the frames asked to free.
    pub(super) freed: Vec<egui::TextureId>,
    /// Where the widgets a screen reader is told of were last drawn, by name (Studio frames).
    pub(super) labels: std::collections::HashMap<String, Rect>,
    /// Every word of the last frame.
    pub(super) words: Vec<String>,
}

impl Rig {
    pub(super) fn new(symbols: &[&str]) -> Self {
        let mut workspace = empty_workspace(1);
        let ctx = egui::Context::default();
        let mut ids = Vec::new();
        for symbol in symbols {
            let id = workspace
                .add_chart_for(&InstrumentId::new(DataSource::Binance, *symbol), &ctx)
                .chart_id;
            let index = workspace.chart_index(id).expect("the chart");
            workspace.charts[index].candles = series(300);
            workspace.charts[index].loading = false;
            ids.push(id);
        }
        let store: SharedStore = Arc::new(Mutex::new(
            crate::store::Store::open_in_memory().expect("in-memory store"),
        ));
        let mut rig = Self {
            ctx,
            workspace,
            paper: PaperHost::load(store),
            ids,
            time: 0.0,
            focused: true,
            studio: false,
            cursor: egui::CursorIcon::Default,
            freed: Vec::new(),
            labels: std::collections::HashMap::new(),
            words: Vec::new(),
        };
        rig.settle();
        rig.workspace.mark_clean();
        rig
    }

    pub(super) fn frame(&mut self, events: Vec<Event>) {
        self.time += 0.016;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SIZE)),
            time: Some(self.time),
            events,
            focused: self.focused,
            ..Default::default()
        };
        let ctx = self.ctx.clone();
        let mut output = ctx.run_ui(input, |ui| self.paint(ui));
        self.freed
            .extend(output.textures_delta.free.iter().copied());
        if let Some(update) = &output.platform_output.accesskit_update {
            for (_, node) in &update.nodes {
                if let (Some(label), Some(bounds)) = (node.label(), node.bounds()) {
                    self.labels.insert(
                        label.to_owned(),
                        Rect::from_min_max(
                            pos2(bounds.x0 as f32, bounds.y0 as f32),
                            pos2(bounds.x1 as f32, bounds.y1 as f32),
                        ),
                    );
                }
            }
        }
        self.words = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        output.textures_delta.clear();
        self.cursor = output.platform_output.cursor_icon;
    }

    /// One frame of the grid, in the central panel the app puts it in.
    pub(super) fn paint(&mut self, ui: &mut egui::Ui) {
        let (workspace, paper) = (&mut self.workspace, &mut self.paper);
        if self.studio {
            workspace.handle_keyboard(ui);
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(egui::Margin::same(10)))
            .show(ui, |ui| {
                let window = ui.max_rect();
                workspace.render_grid(ui, paper, window)
            });
        if self.studio {
            let ctx = ui.ctx().clone();
            workspace.show_studio(&ctx);
            workspace.show_context_ui(&ctx, paper);
        }
    }

    pub(super) fn settle(&mut self) {
        for _ in 0..4 {
            self.frame(Vec::new());
        }
    }

    pub(super) fn button(&self, at: Pos2, button: PointerButton, pressed: bool) -> Event {
        Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    pub(super) fn press(&mut self, at: Pos2) {
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![self.button(at, PointerButton::Primary, true)]);
    }

    /// Moves the held pointer to `to` over a few frames, as a hand does.
    pub(super) fn drag_to(&mut self, from: Pos2, to: Pos2) {
        for step in 1..=6 {
            let at = from + (to - from) * (step as f32 / 6.0);
            self.frame(vec![Event::PointerMoved(at)]);
        }
    }

    pub(super) fn release(&mut self, at: Pos2) {
        self.frame(vec![self.button(at, PointerButton::Primary, false)]);
        self.frame(Vec::new());
    }

    pub(super) fn drag(&mut self, from: Pos2, to: Pos2) {
        self.press(from);
        self.drag_to(from, to);
        self.release(to);
    }

    pub(super) fn escape(&mut self) {
        let key = |pressed| Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        self.frame(vec![key(true), key(false)]);
    }

    pub(super) fn card(&self, chart: usize) -> &ChartCard {
        let index = self.workspace.chart_index(self.ids[chart]).expect("chart");
        &self.workspace.charts[index].chart
    }

    /// What a snapshot of chart `chart` shows, on the window.
    pub(super) fn frame_of(&self, chart: usize) -> Rect {
        self.card(chart).snapshot_frame()
    }

    /// Opens the workspace's frames to the Studio: drawn over the grid, the keyboard its own.
    pub(super) fn with_studio(mut self) -> Self {
        self.studio = true;
        self.ctx.enable_accesskit();
        self
    }

    /// The centre of the widget a screen reader calls `label`.
    pub(super) fn label_at(&self, label: &str) -> Pos2 {
        self.labels
            .get(label)
            .unwrap_or_else(|| panic!("no widget called {label:?} in {:?}", self.labels.keys()))
            .center()
    }

    pub(super) fn selecting(&self) -> bool {
        self.workspace.is_selecting_portion()
    }

    pub(super) fn captured(&self) -> Option<&Capture> {
        self.workspace
            .snapshot
            .studio
            .as_ref()
            .map(|session| &session.capture)
    }

    pub(super) fn view_of(&self, chart: usize) -> crate::ui::ViewStep {
        self.card(chart).view_step()
    }

    pub(super) fn drawings_of(&self, chart: usize) -> usize {
        let index = self.workspace.chart_index(self.ids[chart]).expect("chart");
        self.workspace.charts[index].drawings.len()
    }

    /// A point `fraction` of the way across and down chart `chart`'s snapshot frame.
    pub(super) fn at(&self, chart: usize, fraction: (f32, f32)) -> Pos2 {
        let frame = self.frame_of(chart);
        frame.min + frame.size() * vec2(fraction.0, fraction.1)
    }

    pub(super) fn begin(&mut self) {
        self.workspace.begin_portion_selection();
        self.settle();
    }
}

fn single(rig: &Rig) -> &ChartCapture {
    match rig.captured() {
        Some(Capture::Single(chart)) => chart,
        _ => panic!("a capture of one chart"),
    }
}

#[test]
fn the_charts_are_laid_out_side_by_side_with_room_for_a_selection() {
    let rig = Rig::new(&["BTCUSDT", "ETHUSDT"]);
    let (a, b) = (rig.frame_of(0), rig.frame_of(1));
    assert!(a.width() > 200.0 && a.height() > 200.0, "{a:?}");
    assert!(b.width() > 200.0 && b.height() > 200.0, "{b:?}");
    assert!(!a.intersects(b));
}

fn layout(rig: &Rig) -> &LayoutCapture {
    match rig.captured() {
        Some(Capture::Layout(layout)) => layout,
        _ => panic!("a capture of a layout"),
    }
}

fn input() -> Input {
    Input {
        frame: 10,
        focused: true,
        escape: false,
        secondary: false,
        pressed: None,
        released: false,
        down: false,
        pointer: None,
        over_grid: true,
    }
}

fn corners(from: Pos2, to: Pos2) -> [(Pos2, Pos2); 4] {
    [
        (from, to),
        (pos2(to.x, from.y), pos2(from.x, to.y)),
        (pos2(from.x, to.y), pos2(to.x, from.y)),
        (to, from),
    ]
}

// MARK: The pointer, frame by frame

#[test]
fn a_press_in_a_body_starts_a_rectangle_that_ends_at_the_release() {
    let body = Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 300.0));
    let area = body.shrink(20.0);
    let mut selecting = Selecting::default();
    let press = Input {
        pressed: Some(pos2(100.0, 100.0)),
        pointer: Some(pos2(100.0, 100.0)),
        down: true,
        ..input()
    };
    assert_eq!(
        selecting.step(&press, &[body], area),
        Step::Dragging(Rect::from_min_max(pos2(100.0, 100.0), pos2(100.0, 100.0)))
    );
    let moved = Input {
        frame: 11,
        pointer: Some(pos2(250.0, 220.0)),
        down: true,
        ..input()
    };
    assert_eq!(
        selecting.step(&moved, &[body], area),
        Step::Dragging(Rect::from_min_max(pos2(100.0, 100.0), pos2(250.0, 220.0)))
    );
    let release = Input {
        frame: 12,
        pointer: Some(pos2(250.0, 220.0)),
        released: true,
        ..input()
    };
    assert_eq!(
        selecting.step(&release, &[body], area),
        Step::Done(Rect::from_min_max(pos2(100.0, 100.0), pos2(250.0, 220.0)))
    );
}

#[test]
fn every_drag_direction_gives_the_same_rectangle() {
    let body = Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 300.0));
    let (from, to) = (pos2(60.0, 50.0), pos2(310.0, 240.0));
    for (start, end) in corners(from, to) {
        let mut selecting = Selecting::default();
        let press = Input {
            pressed: Some(start),
            pointer: Some(start),
            down: true,
            ..input()
        };
        selecting.step(&press, &[body], body);
        let release = Input {
            frame: 11,
            pointer: Some(end),
            released: true,
            ..input()
        };
        assert_eq!(
            selecting.step(&release, &[body], body),
            Step::Done(Rect::from_two_pos(from, to)),
            "from {start:?} to {end:?}"
        );
    }
}

#[test]
fn a_press_outside_every_body_or_under_a_menu_starts_nothing() {
    let body = Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 300.0));
    let mut selecting = Selecting::default();
    let outside = Input {
        pressed: Some(pos2(500.0, 100.0)),
        pointer: Some(pos2(500.0, 100.0)),
        down: true,
        ..input()
    };
    assert_eq!(selecting.step(&outside, &[body], body), Step::Waiting);
    let covered = Input {
        frame: 11,
        pressed: Some(pos2(100.0, 100.0)),
        pointer: Some(pos2(100.0, 100.0)),
        down: true,
        over_grid: false,
        ..input()
    };
    assert_eq!(selecting.step(&covered, &[body], body), Step::Waiting);
}

#[test]
fn escape_a_second_button_a_lost_window_and_a_skipped_frame_each_end_it() {
    let body = Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 300.0));
    let press = Input {
        pressed: Some(pos2(100.0, 100.0)),
        pointer: Some(pos2(100.0, 100.0)),
        down: true,
        ..input()
    };
    let cases: [(&str, Input); 4] = [
        (
            "escape",
            Input {
                escape: true,
                ..input()
            },
        ),
        (
            "second button",
            Input {
                secondary: true,
                ..input()
            },
        ),
        (
            "focus",
            Input {
                focused: false,
                ..input()
            },
        ),
        (
            "skipped frame",
            Input {
                frame: 20,
                ..input()
            },
        ),
    ];
    for (name, ending) in cases {
        // Both idle and mid-drag.
        let mut idle = Selecting::default();
        idle.step(&input(), &[body], body);
        assert_eq!(idle.step(&ending, &[body], body), Step::Cancelled, "{name}");
        let mut dragging = Selecting::default();
        dragging.step(&press, &[body], body);
        assert_eq!(
            dragging.step(&ending, &[body], body),
            Step::Cancelled,
            "{name} while dragging"
        );
    }
}

#[test]
fn a_release_that_was_never_reported_ends_the_drag_as_a_release_does() {
    let body = Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 300.0));
    let mut selecting = Selecting::default();
    let press = Input {
        pressed: Some(pos2(100.0, 100.0)),
        pointer: Some(pos2(100.0, 100.0)),
        down: true,
        ..input()
    };
    selecting.step(&press, &[body], body);
    let gone = Input {
        frame: 11,
        pointer: Some(pos2(300.0, 250.0)),
        down: false,
        ..input()
    };
    assert_eq!(
        selecting.step(&gone, &[body], body),
        Step::Done(Rect::from_min_max(pos2(100.0, 100.0), pos2(300.0, 250.0)))
    );
}

// MARK: Entering and leaving

#[test]
fn picking_select_portion_takes_the_pointer_from_the_tools_and_zoom() {
    let mut rig = Rig::new(&["BTCUSDT"]);
    rig.workspace.toggle_tool(Tool::TrendLine);
    rig.workspace.begin_portion_selection();
    assert!(rig.selecting());
    assert_eq!(rig.workspace.tools.active_tool(), Tool::Cursor);

    let mut rig = Rig::new(&["BTCUSDT"]);
    rig.workspace.toggle_zoom_in();
    rig.workspace.begin_portion_selection();
    assert!(rig.selecting());
    assert!(!rig.workspace.tools.is_zoom_armed());
    rig.settle();
    assert!(rig.selecting(), "and a frame does not put it down");
}

#[test]
fn arming_a_tool_or_zoom_in_puts_the_selection_down() {
    let mut rig = Rig::new(&["BTCUSDT"]);
    rig.begin();
    assert!(rig.selecting());
    rig.workspace.toggle_tool(Tool::TrendLine);
    rig.frame(Vec::new());
    assert!(!rig.selecting(), "a drawing tool");

    let mut rig = Rig::new(&["BTCUSDT"]);
    rig.begin();
    rig.workspace.toggle_zoom_in();
    rig.frame(Vec::new());
    assert!(!rig.selecting(), "Zoom In");
    assert!(
        rig.workspace.tools.is_zoom_armed(),
        "which keeps the pointer"
    );
}

#[test]
fn there_is_nothing_to_select_without_a_market_chart() {
    let mut workspace = empty_workspace(1);
    workspace.begin_portion_selection();
    assert!(!workspace.is_selecting_portion());
    let notice = workspace.context.notice.as_ref().expect("it says why");
    assert!(notice.error);
}

#[test]
fn the_cursor_is_a_crosshair_over_the_grid() {
    let mut rig = Rig::new(&["BTCUSDT"]);
    rig.begin();
    let at = rig.at(0, (0.5, 0.5));
    rig.frame(vec![Event::PointerMoved(at)]);
    rig.frame(vec![Event::PointerMoved(at + vec2(2.0, 2.0))]);
    assert_eq!(rig.cursor, egui::CursorIcon::Crosshair);
}

// MARK: One chart

#[test]
fn a_drag_over_one_chart_captures_that_charts_portion() {
    let mut rig = Rig::new(&["BTCUSDT", "ETHUSDT"]);
    rig.begin();
    let (from, to) = (rig.at(1, (0.2, 0.2)), rig.at(1, (0.7, 0.6)));
    rig.drag(from, to);
    assert!(!rig.selecting(), "one drag is the whole mode");
    let capture = single(&rig);
    assert_eq!(capture.chart_id(), rig.ids[1]);
    let wanted = Rect::from_two_pos(from, to).size();
    assert!((capture.size() - wanted).abs().max_elem() < 0.01);
    let session = rig.workspace.snapshot.studio.as_ref().expect("a session");
    assert_eq!(session.config.area, Area::Portion);
    assert_eq!(session.config.style, Style::Styled);
}

#[test]
fn the_portion_shows_the_bars_and_prices_under_the_selection() {
    let mut rig = Rig::new(&["BTCUSDT"]);
    let frame = rig.frame_of(0);
    let whole = {
        let index = rig.workspace.chart_index(rig.ids[0]).expect("chart");
        let parts = rig.workspace.chart_parts(&rig.ctx, index, false);
        ChartCapture::new(rig.card(0), parts, 2.0, None).expect("the whole chart")
    };
    rig.begin();
    let (from, to) = (rig.at(0, (0.25, 0.15)), rig.at(0, (0.65, 0.5)));
    rig.drag(from, to);
    let portion = single(&rig);
    // A bar of the whole chart is this wide; the selection spans this many of them.
    let slot = whole.base().plot.x / whole.base().bars_in_view as f32;
    let across = f64::from((to.x - from.x) / slot);
    assert!(
        (portion.base().bars_in_view - across).abs() <= 3.0,
        "{} bars against {across}",
        portion.base().bars_in_view
    );
    // Its prices lie inside the chart's and are fewer, about the share of the plot's height
    // it covers.
    let (all, part) = (whole.base().price, portion.base().price);
    assert!(part.minimum >= all.minimum - 1e-6 && part.maximum <= all.maximum + 1e-6);
    let share = (part.maximum - part.minimum) / (all.maximum - all.minimum);
    let height = f64::from((to.y - from.y) / frame.height());
    assert!(
        share < height * 1.6 && share > height * 0.4,
        "{share} vs {height}"
    );
}

#[test]
fn every_drag_direction_captures_the_same_portion() {
    let mut sizes = Vec::new();
    let mut bases = Vec::new();
    for flip in 0..4 {
        let mut rig = Rig::new(&["BTCUSDT"]);
        rig.begin();
        let (from, to) = (rig.at(0, (0.2, 0.25)), rig.at(0, (0.6, 0.7)));
        let (start, end) = corners(from, to)[flip];
        rig.drag(start, end);
        let capture = single(&rig);
        sizes.push(capture.size());
        bases.push(*capture.base());
    }
    assert!(sizes.iter().all(|size| *size == sizes[0]), "{sizes:?}");
    assert!(bases.iter().all(|base| *base == bases[0]));
}

#[test]
fn a_tiny_drag_or_a_click_ends_the_mode_and_captures_nothing() {
    for delta in [vec2(10.0, 10.0), vec2(0.0, 0.0), vec2(200.0, 12.0)] {
        let mut rig = Rig::new(&["BTCUSDT"]);
        rig.begin();
        let from = rig.at(0, (0.4, 0.4));
        rig.drag(from, from + delta);
        assert!(!rig.selecting(), "{delta:?}");
        assert!(rig.captured().is_none(), "{delta:?}");
        assert!(rig.workspace.context.notice.is_none(), "it is not an error");
    }
}

#[test]
fn a_drag_past_the_grid_is_kept_inside_the_charts() {
    let mut rig = Rig::new(&["BTCUSDT"]);
    let frame = rig.frame_of(0);
    rig.begin();
    let from = rig.at(0, (0.3, 0.3));
    rig.drag(from, pos2(5000.0, 5000.0));
    let size = single(&rig).size();
    assert!(
        (size - (frame.max - from)).abs().max_elem() < 0.01,
        "{size:?}"
    );

    let mut rig = Rig::new(&["BTCUSDT"]);
    rig.begin();
    rig.drag(from, pos2(-300.0, -300.0));
    let size = single(&rig).size();
    assert!(
        (size - (from - frame.min)).abs().max_elem() < 0.01,
        "{size:?}"
    );
}

#[test]
fn the_replays_cut_candles_are_what_the_capture_shows() {
    let mut rig = Rig::new(&["BTCUSDT"]);
    let candles = series(300);
    let timeline: Vec<i64> = candles.iter().map(|candle| candle.open_time).collect();
    rig.workspace
        .replay
        .start(candles[149].open_time, "BTCUSDT", timeline);
    rig.settle();
    rig.begin();
    rig.drag(rig.at(0, (0.2, 0.2)), rig.at(0, (0.8, 0.8)));
    assert_eq!(single(&rig).base().total_bars, 150);
}

#[test]
fn hidden_drawings_are_not_in_the_capture() {
    let mut rig = Rig::new(&["BTCUSDT"]);
    let index = rig.workspace.chart_index(rig.ids[0]).expect("chart");
    rig.workspace.charts[index].drawings = vec![rectangle(1, 100 * HOUR)];
    rig.settle();
    rig.begin();
    rig.drag(rig.at(0, (0.2, 0.2)), rig.at(0, (0.8, 0.8)));
    assert!(single(&rig).has_drawings(), "shown, it counts");

    rig.workspace.tools.toggle_hide_all();
    rig.settle();
    rig.begin();
    rig.drag(rig.at(0, (0.2, 0.2)), rig.at(0, (0.8, 0.8)));
    assert!(!single(&rig).has_drawings(), "hidden, it does not");
}

// MARK: Across charts

#[test]
fn a_drag_across_charts_captures_the_layout_cropped_to_the_selection() {
    let mut rig = Rig::new(&["BTCUSDT", "ETHUSDT", "SOLUSDT"]);
    rig.workspace.focus_chart(rig.ids[1]);
    rig.settle();
    rig.begin();
    let (from, to) = (rig.at(0, (0.5, 0.3)), rig.at(1, (0.5, 0.7)));
    rig.drag(from, to);
    let layout = layout(&rig);
    // Only the charts the selection reaches are frozen, in the layout of all three.
    assert_eq!(layout.charts.len(), 2);
    let all = rig
        .frame_of(0)
        .union(rig.frame_of(1))
        .union(rig.frame_of(2));
    assert!((layout.size - all.size()).abs().max_elem() < 0.01);
    let origin = all.min.to_vec2();
    let crop = layout.crop.expect("a crop");
    let wanted = Rect::from_two_pos(from, to).translate(-origin);
    assert!((crop.min - wanted.min).abs().max_elem() < 0.01);
    assert!((crop.max - wanted.max).abs().max_elem() < 0.01);
    for (chart, place) in &layout.charts {
        let index = rig
            .ids
            .iter()
            .position(|id| *id == chart.chart_id())
            .unwrap();
        let live = rig.frame_of(index).translate(-origin);
        assert!((place.min - live.min).abs().max_elem() < 0.01);
        assert!((place.size() - live.size()).abs().max_elem() < 0.01);
    }
    assert_eq!(
        layout.focused().unwrap().chart_id(),
        rig.ids[1],
        "the focused chart"
    );
    assert_eq!(layout.view(), crop);
}

#[test]
fn the_focused_chart_is_the_first_reached_when_it_is_not_among_them() {
    let mut rig = Rig::new(&["BTCUSDT", "ETHUSDT", "SOLUSDT"]);
    rig.workspace.focus_chart(rig.ids[2]);
    rig.settle();
    rig.begin();
    rig.drag(rig.at(0, (0.5, 0.3)), rig.at(1, (0.5, 0.7)));
    assert_eq!(layout(&rig).focused().unwrap().chart_id(), rig.ids[0]);
}

#[test]
fn a_selection_over_no_chart_says_so() {
    let rig = Rig::new(&["BTCUSDT"]);
    let regions = rig.workspace.portion_regions();
    let header = Rect::from_min_max(
        pos2(regions[0].body.left(), regions[0].body.top()),
        pos2(regions[0].body.right(), regions[0].frame.top() - 0.5),
    );
    let error = rig
        .workspace
        .capture_portion(&rig.ctx, header, &regions)
        .err()
        .expect("nothing there");
    assert!(matches!(error, CaptureError::NothingSelected), "{error}");
    // A sliver of a chart is not a selection of it either.
    let sliver = Rect::from_min_max(
        pos2(
            regions[0].frame.left() + 20.0,
            regions[0].frame.top() - 40.0,
        ),
        pos2(
            regions[0].frame.left() + 300.0,
            regions[0].frame.top() + 5.0,
        ),
    );
    assert!(matches!(
        rig.workspace
            .capture_portion(&rig.ctx, sliver, &regions)
            .err(),
        Some(CaptureError::NothingSelected)
    ));
}

// MARK: Ending it

#[test]
fn escape_right_click_and_a_lost_window_cancel_mid_drag_and_the_release_does_nothing() {
    for how in 0..3 {
        let mut rig = Rig::new(&["BTCUSDT"]);
        rig.begin();
        let (from, to) = (rig.at(0, (0.2, 0.2)), rig.at(0, (0.7, 0.7)));
        rig.press(from);
        rig.drag_to(from, to);
        match how {
            0 => rig.escape(),
            1 => rig.frame(vec![rig.button(to, PointerButton::Secondary, true)]),
            _ => {
                rig.focused = false;
                rig.frame(Vec::new());
                rig.focused = true;
            }
        }
        assert!(!rig.selecting(), "case {how}");
        rig.release(to);
        assert!(rig.captured().is_none(), "case {how}");
        assert!(!rig.selecting(), "case {how}");
    }
}

#[test]
fn a_skipped_frame_cancels() {
    let mut rig = Rig::new(&["BTCUSDT"]);
    rig.begin();
    let from = rig.at(0, (0.2, 0.2));
    rig.press(from);
    // The workspace goes unseen for a frame (another tab), then comes back.
    for _ in 0..2 {
        rig.time += 0.016;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SIZE)),
            time: Some(rig.time),
            ..Default::default()
        };
        let _ = rig.ctx.run_ui(input, |_| {});
    }
    rig.frame(Vec::new());
    assert!(!rig.selecting());
}

#[test]
fn escape_is_spent_on_the_selection_and_the_next_one_is_the_workspaces() {
    let mut rig = Rig::new(&["BTCUSDT"]);
    let index = rig.workspace.chart_index(rig.ids[0]).expect("chart");
    rig.workspace.charts[index].drawings = vec![rectangle(1, 100 * HOUR)];
    rig.workspace.tools.select(rig.ids[0], vec![1]);
    rig.begin();
    rig.escape();
    assert!(!rig.selecting());
    assert!(
        rig.workspace.tools.selection().is_some(),
        "the drawing is still selected"
    );
    rig.escape();
    assert!(
        rig.workspace.tools.selection().is_none(),
        "the second Escape is the workspace's"
    );
}

// MARK: Nothing is written

#[test]
fn the_charts_stand_back_while_it_runs() {
    let mut rig = Rig::new(&["BTCUSDT"]);
    let index = rig.workspace.chart_index(rig.ids[0]).expect("chart");
    rig.workspace.charts[index].drawings = vec![rectangle(1, 100 * HOUR)];
    rig.workspace.toggle_tool(Tool::TrendLine);
    rig.settle();
    rig.begin();
    let (view, drawings) = (rig.view_of(0), rig.drawings_of(0));
    let from = rig.at(0, (0.3, 0.3));
    rig.press(from);
    rig.drag_to(from, rig.at(0, (0.8, 0.8)));
    // Mid-drag the chart has not panned, drawn or been told of a pointer over it.
    assert_eq!(rig.view_of(0), view);
    assert_eq!(rig.drawings_of(0), drawings);
    assert!(rig.workspace.crosshair.is_none());
    rig.frame(vec![Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: vec2(0.0, -5.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    }]);
    assert_eq!(rig.view_of(0), view, "the wheel does not zoom");
    rig.release(rig.at(0, (0.8, 0.8)));
    assert_eq!(rig.view_of(0), view);
    assert_eq!(rig.drawings_of(0), drawings);
    assert!(rig.captured().is_some());
}

#[test]
fn without_the_selection_the_same_hand_does_pan_and_hover() {
    let mut rig = Rig::new(&["BTCUSDT"]);
    let view = rig.view_of(0);
    let from = rig.at(0, (0.5, 0.5));
    rig.press(from);
    rig.drag_to(from, from + vec2(150.0, 0.0));
    assert!(
        rig.workspace.crosshair.is_some(),
        "the crosshair follows the pointer"
    );
    rig.release(from + vec2(150.0, 0.0));
    assert_ne!(rig.view_of(0), view, "and the chart pans");
}

#[test]
fn nothing_is_written_not_an_undo_step_not_a_save_not_a_drawing() {
    let mut rig = Rig::new(&["BTCUSDT", "ETHUSDT"]);
    assert!(!rig.workspace.is_dirty());
    let (can_undo, views) = (
        rig.workspace.history.can_undo(),
        (rig.view_of(0), rig.view_of(1)),
    );
    for (from, to) in [
        (rig.at(0, (0.2, 0.2)), rig.at(0, (0.7, 0.7))),
        (rig.at(0, (0.5, 0.5)), rig.at(1, (0.5, 0.5))),
        (rig.at(1, (0.5, 0.5)), rig.at(1, (0.52, 0.52))),
    ] {
        rig.begin();
        rig.drag(from, to);
        rig.begin();
        rig.press(from);
        rig.escape();
        rig.release(to);
    }
    assert_eq!(rig.workspace.history.can_undo(), can_undo);
    assert!(!rig.workspace.is_dirty(), "the layout was not touched");
    assert_eq!((rig.view_of(0), rig.view_of(1)), views);
    assert_eq!((rig.drawings_of(0), rig.drawings_of(1)), (0, 0));
}

#[test]
fn freezing_leaves_the_live_chart_as_it_was() {
    let mut rig = Rig::new(&["BTCUSDT"]);
    let index = rig.workspace.chart_index(rig.ids[0]).expect("chart");
    rig.workspace.charts[index].drawings = vec![rectangle(1, 100 * HOUR)];
    rig.settle();
    let (view, drawings, candles) = (
        rig.view_of(0),
        rig.workspace.charts[index].drawings.clone(),
        rig.workspace.charts[index].candles.clone(),
    );
    rig.begin();
    rig.drag(rig.at(0, (0.1, 0.1)), rig.at(0, (0.9, 0.9)));
    assert!(rig.captured().is_some());
    assert_eq!(rig.view_of(0), view);
    assert_eq!(rig.workspace.charts[index].drawings, drawings);
    assert_eq!(rig.workspace.charts[index].candles, candles);
    rig.settle();
    assert_eq!(rig.view_of(0), view);
}

// MARK: Pictures

/// The selection over the grid, written as PNGs in both themes to look at. Run by hand:
/// `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib gallery_of_the_portion_selection -- --ignored`.
#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_portion_selection() {
    use crate::ui::render_tests::{Screen, gallery_dir};
    use crate::ui::theme::{self, AppTheme};
    if gallery_dir().is_none() {
        return;
    }
    let _theme = theme::RestoreDefaultTheme;
    for (look, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
        let mut rig = Rig::new(&["BTCUSDT", "ETHUSDT"]);
        let mut screen = Screen::new(SIZE);
        theme::apply(&screen.ctx, app_theme);
        let shoot = |rig: &mut Rig, screen: &mut Screen, name: &str, events: Vec<Event>| {
            let mut last = None;
            for step in 0..3 {
                let events = if step == 0 {
                    events.clone()
                } else {
                    Vec::new()
                };
                last = Some(screen.run(events, |ui| rig.paint(ui)));
            }
            screen.save(&format!("selection-{look}-{name}"), last.expect("a frame"));
        };
        let move_to = |at: Pos2| Event::PointerMoved(at);
        let button = |at: Pos2, pressed: bool| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        for _ in 0..3 {
            screen.run(Vec::new(), |ui| rig.paint(ui));
        }
        rig.workspace.begin_portion_selection();
        shoot(&mut rig, &mut screen, "1-before", Vec::new());

        let (from, to) = (rig.at(0, (0.2, 0.25)), rig.at(0, (0.65, 0.7)));
        screen.run(vec![move_to(from)], |ui| rig.paint(ui));
        screen.run(vec![button(from, true)], |ui| rig.paint(ui));
        shoot(&mut rig, &mut screen, "2-over-one-chart", vec![move_to(to)]);
        screen.run(vec![button(to, false)], |ui| rig.paint(ui));

        rig.workspace.begin_portion_selection();
        screen.run(Vec::new(), |ui| rig.paint(ui));
        let (from, to) = (rig.at(0, (0.45, 0.35)), rig.at(1, (0.6, 0.75)));
        screen.run(vec![move_to(from)], |ui| rig.paint(ui));
        screen.run(vec![button(from, true)], |ui| rig.paint(ui));
        shoot(&mut rig, &mut screen, "3-across-two", vec![move_to(to)]);
        screen.run(vec![button(to, false)], |ui| rig.paint(ui));
    }
}
