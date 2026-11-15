//! Select portion: choosing which part of the charts a snapshot shows.
//!
//! A mode of the workspace, modelled on Zoom In but owned by the snapshot state, since the
//! rectangle belongs to the grid and not to a chart: it may start on any chart and cross into the
//! next. While it is on a layer over the grid (`ui::workspace::portion_overlay`) takes the pointer
//! from everything under it, and the cards stand back (`ChartCard::set_snapshot_select`: no
//! pan, wheel, drawing hover or pick).
//!
//! One press and release is the whole mode. Nothing is written while it runs and nothing when it
//! ends: no undo step, no layout save, no drawing, no change of view. A selection that ends well
//! freezes what it covers (read-only, on the UI thread) into a [`StudioSession`].

use super::studio::StudioSession;
use super::*;
use crate::model::Tool;
use crate::model::chart::snapshot::selection::{self, MIN_SIDE};
use crate::ui::snapshot_studio::capture::{
    Capture, ChartCapture, LayoutCapture, cached_logo_image,
};
use crate::ui::workspace::portion_overlay;
use eframe::egui::{Pos2, Rect};

/// The mode: waiting for a press, then dragging.
#[derive(Default)]
pub(super) struct Selecting {
    drag: Option<Drag>,
    /// The frame the mode was last driven in, to notice it went unseen (another tab).
    seen: Option<u64>,
}

/// The rectangle being dragged, in window points.
#[derive(Clone, Copy, Debug)]
struct Drag {
    origin: Pos2,
    to: Pos2,
}

/// What a frame's input says to the selection.
pub(super) struct Input {
    /// The frame's number, to notice one that was skipped.
    pub(super) frame: u64,
    pub(super) focused: bool,
    pub(super) escape: bool,
    /// A second button is, or was just, pressed.
    pub(super) secondary: bool,
    /// Where the primary button went down this frame.
    pub(super) pressed: Option<Pos2>,
    pub(super) released: bool,
    pub(super) down: bool,
    pub(super) pointer: Option<Pos2>,
    /// The pointer is over the grid and nothing is above it.
    pub(super) over_grid: bool,
}

impl Input {
    fn read(ctx: &Context, over_grid: bool) -> Self {
        let frame = ctx.cumulative_frame_nr();
        ctx.input(|input| Self {
            frame,
            focused: input.focused,
            escape: input.key_pressed(egui::Key::Escape),
            secondary: input.pointer.secondary_down(),
            pressed: input
                .pointer
                .button_pressed(egui::PointerButton::Primary)
                .then(|| input.pointer.press_origin().or(input.pointer.latest_pos()))
                .flatten(),
            released: input.pointer.button_released(egui::PointerButton::Primary),
            down: input.pointer.primary_down(),
            pointer: input.pointer.latest_pos(),
            over_grid,
        })
    }
}

/// What a frame did to the selection.
#[derive(Debug, PartialEq)]
pub(super) enum Step {
    /// Nothing is being dragged yet.
    Waiting,
    /// The rectangle so far, inside the area.
    Dragging(Rect),
    /// Escape, a second button, a lost window, a skipped frame or a selection too small ended it.
    Cancelled,
    /// The pointer was released on a selection large enough to capture.
    Done(Rect),
}

impl Selecting {
    /// One frame of the pointer. `bodies` are the charts' bodies, where a press may begin the
    /// rectangle, and `area` is the union of their snapshot frames, which it is kept inside.
    pub(super) fn step(&mut self, input: &Input, bodies: &[Rect], area: Rect) -> Step {
        let unseen = self.seen.is_some_and(|seen| input.frame > seen + 1);
        self.seen = Some(input.frame);
        if unseen || !input.focused || input.escape || input.secondary {
            return Step::Cancelled;
        }
        if self.drag.is_none() {
            let Some(origin) = input.pressed.filter(|origin| {
                input.over_grid && bodies.iter().any(|body| body.contains(*origin))
            }) else {
                return Step::Waiting;
            };
            self.drag = Some(Drag { origin, to: origin });
        }
        let Some(drag) = self.drag.as_mut() else {
            return Step::Waiting;
        };
        if let Some(pointer) = input.pointer {
            drag.to = pointer;
        }
        // A button that went up where egui did not report it ends the drag as a release does.
        if input.released || !input.down {
            return selection::released(drag.origin, drag.to, area)
                .map_or(Step::Cancelled, Step::Done);
        }
        Step::Dragging(selection::dragged(drag.origin, drag.to, area))
    }
}

/// A market chart as the selection sees it.
pub(super) struct Region {
    /// Its place in `ChartWorkspace::charts`.
    pub(super) index: usize,
    /// The whole body, header strip included: where a press may begin.
    pub(super) body: Rect,
    /// What a snapshot of the chart shows, on the window.
    pub(super) frame: Rect,
}

/// The smallest rectangle holding every rectangle given.
fn union(rects: impl IntoIterator<Item = Rect>) -> Rect {
    rects.into_iter().reduce(Rect::union).unwrap_or(Rect::ZERO)
}

/// Whether the rectangles share any area.
fn overlaps(a: Rect, b: Rect) -> bool {
    a.intersect(b).is_positive()
}

impl ChartWorkspace {
    /// Turns Select portion on. It takes the pointer from the drawing tools and Zoom In, as they
    /// take it from each other.
    pub(in crate::workspace) fn begin_portion_selection(&mut self) {
        if self.snapshot_busy() {
            self.say("A snapshot is already being made.", true);
            return;
        }
        if self.market_charts() == 0 {
            self.say("There is no chart to take a picture of yet.", true);
            return;
        }
        self.tools.disarm();
        self.tools.disarm_zoom();
        self.snapshot.selecting = Some(Selecting::default());
    }

    /// Whether Select portion is on.
    pub(in crate::workspace) fn is_selecting_portion(&self) -> bool {
        self.snapshot.selecting.is_some()
    }

    /// Select portion and the tools are one choice: whichever was picked last holds the pointer,
    /// so an armed drawing tool or Zoom In puts the selection down.
    pub(in crate::workspace) fn give_pointer_to_tool_over_selection(&mut self) {
        if self.tools.active_tool() != Tool::Cursor || self.tools.is_zoom_armed() {
            self.snapshot.selecting = None;
        }
    }

    /// The market charts that were drawn at the last frame.
    pub(super) fn portion_regions(&self) -> Vec<Region> {
        self.charts
            .iter()
            .enumerate()
            .filter(|(_, chart)| chart.is_market() && chart.chart.body_rect().is_positive())
            .map(|(index, chart)| Region {
                index,
                body: chart.chart.body_rect(),
                frame: chart.chart.snapshot_frame(),
            })
            .collect()
    }

    /// One frame of Select portion over `grid`, the area the charts are drawn in: lays the layer
    /// over it, follows the pointer, paints the rectangle, and on release freezes what it covers.
    /// Returns whether an Escape was spent ending the mode, so the workspace's own Escape does
    /// not also act.
    pub(in crate::workspace) fn drive_portion_selection(
        &mut self,
        ctx: &Context,
        grid: Rect,
    ) -> bool {
        if self.snapshot.selecting.is_none() {
            return false;
        }
        let regions = self.portion_regions();
        if regions.is_empty() {
            self.snapshot.selecting = None;
            return false;
        }
        let layer = portion_overlay::shield(ctx, Id::new(("snapshot-select", self.id)), grid);
        let input = Input::read(ctx, layer.hovered());
        let bodies: Vec<Rect> = regions.iter().map(|region| region.body).collect();
        let area = union(regions.iter().map(|region| region.frame));
        let Some(step) = self
            .snapshot
            .selecting
            .as_mut()
            .map(|selecting| selecting.step(&input, &bodies, area))
        else {
            return false;
        };
        match step {
            Step::Waiting => {
                portion_overlay::paint(&ctx.layer_painter(layer.layer_id), grid, area, None);
            }
            Step::Dragging(rect) => {
                portion_overlay::paint(&ctx.layer_painter(layer.layer_id), grid, area, Some(rect));
            }
            Step::Cancelled => self.snapshot.selecting = None,
            Step::Done(rect) => {
                self.snapshot.selecting = None;
                self.finish_portion(ctx, rect, &regions);
            }
        }
        if layer.hovered()
            || self
                .snapshot
                .selecting
                .as_ref()
                .is_some_and(|s| s.drag.is_some())
        {
            ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        input.escape
    }

    /// Freezes what `selection` covers and keeps it for the Studio; says why when it cannot.
    fn finish_portion(&mut self, ctx: &Context, selection: Rect, regions: &[Region]) {
        match self.capture_portion(ctx, selection, regions) {
            Ok(capture) => self.snapshot.studio = Some(StudioSession::portion(capture)),
            Err(error) => self.say(error.to_string(), true),
        }
    }

    /// Freezes the part of the charts `selection` (in window points) covers. Within one chart it
    /// is that chart's portion: the selection counted from the snapshot frame's corner, which the
    /// capture clamps to the plot (the axes are always drawn, so a selection that takes in an
    /// axis keeps it and one that does not still shows it, inside the same size). Across charts
    /// it is the layout, with the selection as its crop. Runs on the UI thread and only reads the
    /// workspace.
    pub(super) fn capture_portion(
        &self,
        ctx: &Context,
        selection: Rect,
        regions: &[Region],
    ) -> Result<Capture, CaptureError> {
        let options = self.snapshot_prefs().options();
        let pixels_per_point = options.scale.factor();
        let freeze = |region: &Region, portion: Option<Rect>| {
            let mut parts = self.chart_parts(ctx, region.index, options.include_trading);
            parts.logo = cached_logo_image(ctx, &parts.symbol, parts.source);
            ChartCapture::new(
                &self.charts[region.index].chart,
                parts,
                pixels_per_point,
                portion,
            )
            .map_err(CaptureError::Scene)
        };
        let hit: Vec<&Region> = regions
            .iter()
            .filter(|region| overlaps(region.frame, selection))
            .collect();
        match hit.as_slice() {
            [] => Err(CaptureError::NothingSelected),
            [only] => {
                let covered = selection.intersect(only.frame);
                if covered.width() < MIN_SIDE || covered.height() < MIN_SIDE {
                    return Err(CaptureError::NothingSelected);
                }
                let portion = covered.translate(-only.frame.min.to_vec2());
                freeze(only, Some(portion)).map(Capture::Single)
            }
            several => {
                // The layout is every chart as the window arranges them, from the corner of the
                // union of their frames; only the charts the selection reaches are frozen.
                let origin = union(regions.iter().map(|region| region.frame));
                let shift = -origin.min.to_vec2();
                let charts = several
                    .iter()
                    .map(|region| Ok((freeze(region, None)?, region.frame.translate(shift))))
                    .collect::<Result<Vec<_>, CaptureError>>()?;
                let focused = self.resolved_focused_id();
                Ok(Capture::Layout(LayoutCapture {
                    charts,
                    size: origin.size(),
                    crop: Some(selection.translate(shift)),
                    focused: several
                        .iter()
                        .position(|region| Some(self.charts[region.index].id) == focused)
                        .unwrap_or(0),
                }))
            }
        }
    }
}
