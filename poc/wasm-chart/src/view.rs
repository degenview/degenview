//! What the user is looking at and how the pointer changes it. Pure: the painter reports where it
//! put things ([`Layout`]) and this turns pointer events into edits of the app's own viewport.

use degenview_market::viewport::zoom::Slots;
use degenview_market::viewport::{ChartViewport, PanGesture, PriceRange, Window};

/// Free slots kept past the newest bar, as the app's right margin does.
pub const RESERVED: usize = 6;

/// Wheel travel that changes the bar count by one e-fold; a notch is ~100.
const WHEEL_SENSITIVITY: f64 = 0.0015;
/// A wheel event is capped so a fast flick cannot jump across the whole series.
const WHEEL_LIMIT: f64 = 120.0;

/// The plot rectangle in CSS pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn right(&self) -> f64 {
        self.left + self.width
    }

    pub fn bottom(&self) -> f64 {
        self.top + self.height
    }

    fn contains(&self, x: f32, y: f32) -> bool {
        (self.left..=self.right()).contains(&f64::from(x))
            && (self.top..=self.bottom()).contains(&f64::from(y))
    }
}

/// Where the last frame put the plot and its bars: what a pointer position is read against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub plot: Rect,
    /// The prices the plot's top and bottom show.
    pub shown: PriceRange,
    pub slots: Slots,
    pub window: Window,
}

impl Layout {
    /// The index of the bar under `x`, if there is one.
    pub fn bar_at(&self, x: f32, total: usize) -> Option<usize> {
        let slot = self.slots.at(x).floor();
        (slot >= 0.0 && (slot as usize) < total).then_some(slot as usize)
    }
}

struct Drag {
    gesture: PanGesture,
    from: (f32, f32),
}

pub struct View {
    viewport: ChartViewport,
    total: usize,
    layout: Option<Layout>,
    drag: Option<Drag>,
    pointer: Option<(f32, f32)>,
}

impl View {
    /// The newest bars at the natural scale, over `total` bars.
    pub fn new(total: usize) -> Self {
        Self {
            viewport: ChartViewport::default(),
            total,
            layout: None,
            drag: None,
            pointer: None,
        }
    }

    pub fn viewport(&self) -> &ChartViewport {
        &self.viewport
    }

    #[cfg(test)]
    pub fn layout(&self) -> Option<&Layout> {
        self.layout.as_ref()
    }

    /// The pointer, while it is over the chart.
    pub fn pointer(&self) -> Option<(f32, f32)> {
        self.pointer
    }

    pub fn set_layout(&mut self, layout: Layout) {
        self.layout = Some(layout);
    }

    /// A press inside the plot starts a drag.
    pub fn press(&mut self, x: f32, y: f32) {
        let Some(layout) = self.layout.filter(|layout| layout.plot.contains(x, y)) else {
            return;
        };
        self.drag = Some(Drag {
            gesture: PanGesture::begin(&self.viewport, layout.shown),
            from: (x, y),
        });
    }

    /// The pointer moved: drag the chart if a drag is under way. The gesture follows the pointer
    /// in time and, once it strays far enough vertically, in price.
    pub fn move_to(&mut self, x: f32, y: f32) {
        self.pointer = Some((x, y));
        let (Some(drag), Some(layout)) = (self.drag.as_mut(), self.layout) else {
            return;
        };
        drag.gesture.update(
            &mut self.viewport,
            (x - drag.from.0, y - drag.from.1),
            layout.slots.width as f32,
            layout.plot.height as f32,
            self.total,
            false,
        );
    }

    /// The series is now `total` bars long: `added` of them arrived after the newest. A drag under
    /// way measures from the bars it began over, so it counts them.
    pub fn bars_changed(&mut self, total: usize, added: usize) {
        self.total = total;
        if let Some(drag) = self.drag.as_mut() {
            drag.gesture.arrived(added as f64);
        }
    }

    pub fn release(&mut self) {
        self.drag = None;
    }

    pub fn leave(&mut self) {
        self.pointer = None;
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Zooms the time axis about `x`, so the bar under the pointer stays under it. A negative
    /// `delta_y` (wheel up) shows fewer bars.
    pub fn wheel(&mut self, x: f32, delta_y: f64) {
        let Some(layout) = self.layout else {
            return;
        };
        let visible = self.viewport.horizontal.visible;
        let factor = (delta_y.clamp(-WHEEL_LIMIT, WHEEL_LIMIT) * WHEEL_SENSITIVITY).exp();
        let mut wanted = (visible as f64 * factor).round() as usize;
        if wanted == visible && delta_y != 0.0 {
            wanted = if delta_y > 0.0 {
                visible + 1
            } else {
                visible.saturating_sub(1)
            };
        }
        let anchor = (f64::from(x) - layout.plot.left) / layout.plot.width;
        self.viewport
            .horizontal
            .zoom_at(wanted, anchor, RESERVED, self.total);
    }

    /// Back to the newest bars at the natural scale.
    pub fn reset(&mut self) {
        self.viewport.reset();
        self.drag = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use degenview_market::viewport::{DEFAULT_VISIBLE, MIN_VISIBLE, visible_window};

    const TOTAL: usize = 300;

    /// A view over `TOTAL` bars, laid out over a 700 x 400 plot at (10, 10).
    fn laid_out() -> View {
        let mut view = View::new(TOTAL);
        view.set_layout(layout_of(&view));
        view
    }

    fn layout_of(view: &View) -> Layout {
        let horizontal = view.viewport.horizontal;
        let window = visible_window(TOTAL, horizontal.visible, horizontal.right_offset, RESERVED);
        Layout {
            plot: Rect {
                left: 10.0,
                top: 10.0,
                width: 700.0,
                height: 400.0,
            },
            shown: PriceRange::new(100.0, 200.0),
            slots: Slots {
                first: window.start as f64 - f64::from(window.lead),
                left: 10.0,
                width: 700.0 / window.slots as f64,
            },
            window,
        }
    }

    #[test]
    fn dragging_right_shows_older_bars_and_release_ends_the_drag() {
        let mut view = laid_out();
        view.press(300.0, 200.0);
        assert!(view.is_dragging());
        view.move_to(
            300.0 + 5.0 * view.layout().unwrap().slots.width as f32,
            200.0,
        );
        assert!((view.viewport().horizontal.right_offset - 5.0).abs() < 1e-3);
        view.release();
        assert!(!view.is_dragging());
        view.move_to(100.0, 200.0);
        assert!((view.viewport().horizontal.right_offset - 5.0).abs() < 1e-3);
    }

    #[test]
    fn a_press_outside_the_plot_does_not_start_a_drag() {
        let mut view = laid_out();
        view.press(5.0, 200.0);
        view.press(300.0, 500.0);
        assert!(!view.is_dragging());
    }

    #[test]
    fn a_vertical_drag_takes_the_price_range_with_it() {
        let mut view = laid_out();
        view.press(300.0, 200.0);
        view.move_to(300.0, 200.0 + 40.0);
        assert!(!view.viewport().vertical.is_auto());
        view.reset();
        assert!(view.viewport().vertical.is_auto());
    }

    #[test]
    fn the_wheel_zooms_about_the_pointer_and_stays_in_bounds() {
        let mut view = laid_out();
        view.wheel(400.0, -100.0);
        let closer = view.viewport().horizontal.visible;
        assert!(closer < DEFAULT_VISIBLE, "{closer}");
        view.wheel(400.0, 100.0);
        view.wheel(400.0, 100.0);
        assert!(view.viewport().horizontal.visible > DEFAULT_VISIBLE);
        for _ in 0..200 {
            view.wheel(400.0, -120.0);
        }
        assert_eq!(view.viewport().horizontal.visible, MIN_VISIBLE);
        for _ in 0..200 {
            view.wheel(400.0, 120.0);
        }
        assert_eq!(view.viewport().horizontal.visible, TOTAL);
    }

    #[test]
    fn a_small_wheel_step_still_moves_by_a_bar() {
        let mut view = laid_out();
        view.wheel(400.0, -1.0);
        assert_eq!(view.viewport().horizontal.visible, DEFAULT_VISIBLE - 1);
    }

    #[test]
    fn bars_are_found_by_slot() {
        let view = laid_out();
        let layout = view.layout().unwrap();
        let newest = layout.slots.x(TOTAL as f64 - 0.5);
        assert_eq!(layout.bar_at(newest, TOTAL), Some(TOTAL - 1));
        assert_eq!(
            layout.bar_at(layout.slots.x(TOTAL as f64 + 1.0), TOTAL),
            None
        );
        assert_eq!(layout.bar_at(layout.slots.x(-1.0), TOTAL), None);
    }
}
