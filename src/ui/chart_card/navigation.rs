//! Pointer and button navigation of one chart.
//!
//! Every gesture edits `ChartCard::viewport` (see [`crate::model::viewport`]) and nothing else, so
//! the series, grid, indicators, Pine visuals, drawings and crosshair all follow from the same
//! numbers. Which gesture a press starts is decided by where it lands: the plot pans the chart,
//! the price axis stretches the price range, the time axis stretches the bars, and a script pane
//! pans through time with the main chart.

use super::ChartCard;
use crate::model::Kline;
use crate::model::viewport::{
    ChartViewport, MIN_VISIBLE, PanGesture, PriceRange, VerticalViewport,
};
use crate::ui::chart_buttons::NavAction;
use eframe::egui::{self, CursorIcon, Pos2, Rect, Response};

/// How the price axis drag maps points to a stretch: this many points double or halve the range.
const AXIS_POINTS_PER_DOUBLING: f32 = 120.0;
/// The same for the time axis.
const TIME_POINTS_PER_DOUBLING: f32 = 160.0;

/// Where a pointer is on the chart.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Region {
    /// The candles' plot.
    Plot,
    /// The price labels right of the plot.
    PriceAxis,
    /// The time labels under the plot.
    TimeAxis,
    /// A script pane under the main chart: it shares the time axis, not the price scale.
    Pane,
    /// The corner between the axes and anywhere else.
    Elsewhere,
}

/// The rects that decide a press's [`Region`].
#[derive(Clone, Copy, Debug)]
pub(super) struct Regions {
    pub plot: Rect,
    /// The main chart down to the bottom of its time axis.
    pub main: Rect,
    /// The whole chart, panes included.
    pub chart: Rect,
}

impl Regions {
    pub fn of(&self, point: Pos2) -> Region {
        if self.plot.contains(point) {
            Region::Plot
        } else if !self.chart.contains(point) {
            Region::Elsewhere
        } else if point.y >= self.main.bottom() {
            Region::Pane
        } else if point.x > self.plot.right() && point.y <= self.plot.bottom() {
            Region::PriceAxis
        } else if point.y > self.plot.bottom() && point.x <= self.plot.right() {
            Region::TimeAxis
        } else {
            Region::Elsewhere
        }
    }
}

/// What a press that became a drag is doing.
pub(super) enum Kind {
    /// Dragging the chart; `vertical` is false in a pane, which has no price scale to move.
    Pan { drag: PanGesture, vertical: bool },
    /// Dragging the price axis.
    PriceAxis { origin: VerticalViewport },
    /// Dragging the time axis.
    TimeAxis { origin_visible: usize },
}

/// The drag in progress, with the view to return to if it is cancelled.
pub(super) struct ActiveGesture {
    kind: Kind,
    start: ChartViewport,
    /// Escape was pressed: the drag is ignored until the button comes up.
    cancelled: bool,
}

impl ActiveGesture {
    /// `bars` arrived after the newest one: what the drag measures from moves with the view.
    fn arrived(&mut self, bars: f64) {
        self.start.horizontal.right_offset += bars;
        if let Kind::Pan { drag, .. } = &mut self.kind {
            drag.arrived(bars);
        }
    }
}

/// What the card gives a frame of input handling beyond the response.
pub(super) struct Navigation<'a> {
    pub regions: Regions,
    /// The price range the axis shows now, if the main chart has one.
    pub shown: Option<PriceRange>,
    pub total: usize,
    pub slot_width: f32,
    pub logarithmic: bool,
    /// Whether a drawing gesture owns the drag.
    pub drawing_claims: bool,
    pub response: &'a Response,
}

impl ChartCard {
    /// Applies one frame of wheel, trackpad and drag input to the viewport. Returns whether an
    /// Escape press was spent cancelling a drag, so nothing else answers it too.
    pub(super) fn handle_input(&mut self, ui: &egui::Ui, nav: &Navigation<'_>) -> bool {
        let response = nav.response;
        let primary = egui::PointerButton::Primary;

        // A drag that lost its button (focus left the window, the pointer was released outside
        // it) must not stay half-alive.
        if self.gesture.is_some() && !ui.input(|input| input.pointer.primary_down()) {
            self.gesture = None;
        }

        if response.hovered() {
            self.scroll_and_zoom(ui, nav);
        }

        if response.drag_started_by(primary)
            && !nav.drawing_claims
            && let Some(origin) = ui.input(|input| input.pointer.press_origin())
        {
            self.begin_gesture(nav, nav.regions.of(origin));
        }

        let escaped = self
            .gesture
            .as_ref()
            .is_some_and(|active| !active.cancelled)
            && ui.input(|input| input.key_pressed(egui::Key::Escape));
        if escaped {
            self.cancel_gesture();
        }

        if response.dragged_by(primary)
            && let Some(total) = response.total_drag_delta()
        {
            self.update_gesture(nav, (total.x, total.y));
        }
        if response.drag_stopped_by(primary) {
            self.gesture = None;
        }

        let total = nav.total;
        self.viewport.horizontal.clamp(total);
        self.set_cursor(ui, nav);
        escaped
    }

    /// TradingView's rule (Lightweight Charts' `shiftVisibleRangeOnNewBar`): a bar that arrives
    /// while the previous newest bar is on screen keeps the offset, so the chart slides left by a
    /// slot and the new bar appears where the newest one was, wherever the user has scrolled to.
    /// Only a chart whose newest bar is already out of view is held on its bars: the offset counts
    /// from the newest bar, so it grows by what arrived. Older bars loading need no such care: the
    /// offset never counted them.
    pub(super) fn keep_view_on_its_bars(&mut self, bars: &[Kline]) {
        let newest = bars.last().map(|bar| bar.open_time);
        if let (Some(previous), Some(_)) = (self.newest_bar, newest)
            && !self.viewport.horizontal.shows_newest(self.reserved_slots())
            && let Ok(index) = bars.binary_search_by_key(&previous, |bar| bar.open_time)
        {
            let arrived = (bars.len() - 1 - index) as f64;
            if arrived > 0.0 {
                self.viewport.horizontal.right_offset += arrived;
                if let Some(active) = self.gesture.as_mut() {
                    active.arrived(arrived);
                }
            }
        }
        self.newest_bar = newest;
    }

    /// The wheel zooms; a sideways swipe scrolls through time.
    fn scroll_and_zoom(&mut self, ui: &egui::Ui, nav: &Navigation<'_>) {
        let delta = ui.input(|input| input.smooth_scroll_delta);
        if delta.x.abs() > delta.y.abs() {
            // Swiping right moves the chart right, as dragging does.
            let bars = f64::from(delta.x) / f64::from(nav.slot_width.max(1.0));
            self.viewport.horizontal.scroll_by(bars, nav.total);
        } else if delta.y != 0.0 {
            let factor = (1.0 - delta.y * 0.004).clamp(0.5, 2.0);
            let next = (self.viewport.horizontal.visible as f32 * factor).round() as usize;
            self.viewport.horizontal.set_visible(next, nav.total);
        }
    }

    fn begin_gesture(&mut self, nav: &Navigation<'_>, region: Region) {
        let start = self.viewport;
        let kind = match region {
            Region::Plot | Region::Pane => {
                let shown = nav.shown.unwrap_or_else(|| PriceRange::new(0.0, 1.0));
                Kind::Pan {
                    drag: PanGesture::begin(&self.viewport, shown),
                    vertical: region == Region::Plot && nav.shown.is_some(),
                }
            }
            Region::PriceAxis if nav.shown.is_some() => Kind::PriceAxis {
                origin: self.viewport.vertical,
            },
            Region::TimeAxis => Kind::TimeAxis {
                origin_visible: self.viewport.horizontal.visible,
            },
            _ => return,
        };
        self.gesture = Some(ActiveGesture {
            kind,
            start,
            cancelled: false,
        });
    }

    fn update_gesture(&mut self, nav: &Navigation<'_>, (dx, dy): (f32, f32)) {
        let Some(active) = self.gesture.as_mut().filter(|active| !active.cancelled) else {
            return;
        };
        match &mut active.kind {
            Kind::Pan { drag, vertical } => {
                let dy = if *vertical { dy } else { 0.0 };
                drag.update(
                    &mut self.viewport,
                    (dx, dy),
                    nav.slot_width,
                    nav.regions.plot.height(),
                    nav.total,
                    nav.logarithmic,
                );
            }
            Kind::PriceAxis { origin } => {
                // Dragging up stretches the candles taller: a smaller span is shown.
                let factor = 2_f64.powf(f64::from(dy) / f64::from(AXIS_POINTS_PER_DOUBLING));
                self.viewport
                    .vertical
                    .stretch(origin, factor, nav.logarithmic);
            }
            Kind::TimeAxis { origin_visible } => {
                // Dragging right spreads the bars apart, so fewer fit.
                let factor = 2_f64.powf(-f64::from(dx) / f64::from(TIME_POINTS_PER_DOUBLING));
                self.viewport
                    .horizontal
                    .stretch(*origin_visible, factor, nav.total);
            }
        }
    }

    /// Escape: the view goes back to where the press found it.
    fn cancel_gesture(&mut self) {
        if let Some(active) = self.gesture.as_mut() {
            self.viewport = active.start;
            active.cancelled = true;
        }
    }

    fn set_cursor(&self, ui: &egui::Ui, nav: &Navigation<'_>) {
        let icon = match &self.gesture {
            Some(active) if active.cancelled => None,
            Some(ActiveGesture {
                kind: Kind::Pan { .. },
                ..
            }) => Some(CursorIcon::Grabbing),
            Some(ActiveGesture {
                kind: Kind::PriceAxis { .. },
                ..
            }) => Some(CursorIcon::ResizeVertical),
            Some(ActiveGesture {
                kind: Kind::TimeAxis { .. },
                ..
            }) => Some(CursorIcon::ResizeHorizontal),
            None if nav.drawing_claims => None,
            None => nav
                .response
                .hover_pos()
                .map(|point| nav.regions.of(point))
                .and_then(|region| match region {
                    // A drawing or its tool chose a cursor already: it keeps it.
                    Region::Plot | Region::Pane
                        if ui.ctx().output(|output| output.cursor_icon) == CursorIcon::Default =>
                    {
                        Some(CursorIcon::Crosshair)
                    }
                    Region::PriceAxis => Some(CursorIcon::ResizeVertical),
                    Region::TimeAxis => Some(CursorIcon::ResizeHorizontal),
                    _ => None,
                }),
        };
        if let Some(icon) = icon {
            ui.ctx().set_cursor_icon(icon);
        }
    }

    /// A double click on an axis puts it back: the price axis to autoscale at its natural scale,
    /// the time axis to the newest bars at the natural spacing.
    pub(super) fn handle_axis_double_click(&mut self, response: &Response, regions: &Regions) {
        if !response.double_clicked_by(egui::PointerButton::Primary) {
            return;
        }
        let Some(point) = response.interact_pointer_pos() else {
            return;
        };
        match regions.of(point) {
            Region::PriceAxis => self.viewport.vertical.reset(),
            Region::TimeAxis => self.viewport.horizontal = Default::default(),
            _ => {}
        }
        self.gesture = None;
    }

    /// Applies a navigation button to the one viewport the wheel and the drag also drive.
    pub(super) fn apply_navigation(&mut self, action: NavAction, total_candles: usize) {
        let visible = self.viewport.horizontal.visible;
        let offset = self.viewport.horizontal.right_offset;
        let step = (visible / 5).max(1) as f64;
        let maximum = total_candles.saturating_sub(MIN_VISIBLE) as f64;
        match action {
            NavAction::ZoomIn => self.set_zoom(
                (visible as f32 * 0.8).round() as usize,
                visible.saturating_sub(1),
                total_candles,
            ),
            NavAction::ZoomOut => self.set_zoom(
                (visible as f32 * 1.25).round() as usize,
                visible + 1,
                total_candles,
            ),
            // The chart's right edge moves back in time: older bars come into view.
            NavAction::ScrollLeft => {
                self.viewport.horizontal.right_offset = (offset + step).min(maximum);
            }
            // The buttons stop at the newest bar; only a drag goes on into empty space.
            NavAction::ScrollRight => {
                if offset > 0.0 {
                    self.viewport.horizontal.right_offset = (offset - step).max(0.0);
                }
            }
            NavAction::Reset => self.reset_view(),
        }
        self.viewport.horizontal.clamp(total_candles);
    }

    /// Sets how many candles are in view, at least one away from the current count when `forced`
    /// says where a rounded factor would stand still.
    fn set_zoom(&mut self, wanted: usize, forced: usize, total_candles: usize) {
        let visible = self.viewport.horizontal.visible;
        let next = if wanted == visible { forced } else { wanted };
        self.viewport.horizontal.visible = next.clamp(
            MIN_VISIBLE,
            crate::model::viewport::HorizontalViewport::max_visible(total_candles),
        );
    }
}
