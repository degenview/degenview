//! A draggable two-pane split, ported from `SplitContainer` / `SplitLayout` / `SplitMetrics`.
//!
//! One pane is the *secondary* (the sidebar, the preview chart, the drawer): it owns the stored
//! length and can be collapsed. The other is the *primary* and takes the rest. A stored length of
//! zero means "use the default"; a double-click on the divider resets to it.

use crate::ui::theme;
use eframe::egui::{CursorIcon, Id, Rect, Sense, Stroke, Ui, UiBuilder, pos2, vec2};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Axis {
    /// Panes sit side by side; the divider is vertical.
    Horizontal,
    /// Panes are stacked; the divider is horizontal.
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DefaultLength {
    Points(f32),
    /// A fraction of the total length.
    Fraction(f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitSpec {
    pub axis: Axis,
    /// The secondary pane is the leading one (left / top).
    pub secondary_first: bool,
    pub min_primary: f32,
    pub min_secondary: f32,
    pub max_secondary: f32,
    pub default: DefaultLength,
}

/// What the owner persists: the secondary pane's length (0 = default) and whether it is shown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitState {
    pub length: f32,
    pub visible: bool,
}

impl Default for SplitState {
    fn default() -> Self {
        Self {
            length: 0.0,
            visible: true,
        }
    }
}

/// Width of the draggable strip centred on the boundary.
pub const HIT_THICKNESS: f32 = 9.0;
const REVEAL_SECONDS: f32 = 0.2;

impl SplitSpec {
    /// The secondary length for `total` points of space: the stored length (or the default when
    /// unset) clamped to what both panes' minimums allow.
    pub fn resolved(&self, stored: f32, total: f32) -> f32 {
        let wanted = if stored > 0.0 {
            stored
        } else {
            match self.default {
                DefaultLength::Points(points) => points,
                DefaultLength::Fraction(fraction) => total * fraction,
            }
        };
        self.clamped(wanted, total)
    }

    pub fn clamped(&self, length: f32, total: f32) -> f32 {
        let upper = (self.max_secondary.min(total - self.min_primary)).max(0.0);
        let lower = self.min_secondary.min(upper);
        length.clamp(lower, upper)
    }
}

/// Lays out both panes. `primary` and `secondary` draw into child UIs sized by the split.
pub fn show(
    ui: &mut Ui,
    id: Id,
    spec: SplitSpec,
    state: &mut SplitState,
    primary: impl FnOnce(&mut Ui),
    secondary: impl FnOnce(&mut Ui),
) {
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    let horizontal = spec.axis == Axis::Horizontal;
    let total = if horizontal {
        full.width()
    } else {
        full.height()
    };

    let reveal = ui
        .ctx()
        .animate_bool_with_time(id.with("reveal"), state.visible, REVEAL_SECONDS);
    let length = spec.resolved(state.length, total);
    let shown = length * reveal;

    // Rects along the split axis: [leading pane][divider][trailing pane].
    let along = |rect: Rect| {
        if horizontal {
            (rect.min.x, rect.max.x)
        } else {
            (rect.min.y, rect.max.y)
        }
    };
    let (start, end) = along(full);
    let make = |a: f32, b: f32| {
        if horizontal {
            Rect::from_min_max(pos2(a, full.min.y), pos2(b, full.max.y))
        } else {
            Rect::from_min_max(pos2(full.min.x, a), pos2(full.max.x, b))
        }
    };
    let (primary_rect, secondary_rect, secondary_visible, boundary) = if spec.secondary_first {
        let boundary = start + shown;
        // The hidden pane keeps its full size and slides out beneath the clip.
        let secondary = make(boundary - length, boundary);
        (make(boundary, end), secondary, shown > 0.5, boundary)
    } else {
        let boundary = end - shown;
        let secondary = make(boundary, boundary + length);
        (make(start, boundary), secondary, shown > 0.5, boundary)
    };

    let child = |ui: &mut Ui, rect: Rect, clip: Rect, content: Box<dyn FnOnce(&mut Ui) + '_>| {
        let mut pane = ui.new_child(UiBuilder::new().max_rect(rect));
        pane.set_clip_rect(clip.intersect(ui.clip_rect()));
        content(&mut pane);
    };
    child(ui, primary_rect, primary_rect, Box::new(primary));
    if secondary_visible {
        let clip = secondary_rect.intersect(make(start, end));
        child(ui, secondary_rect, clip, Box::new(secondary));
    }

    if !secondary_visible || reveal < 1.0 {
        if reveal > 0.0 && reveal < 1.0 {
            ui.ctx().request_repaint();
        }
        return;
    }

    // The divider, drawn over both panes.
    let hit = if horizontal {
        Rect::from_center_size(
            pos2(boundary, full.center().y),
            vec2(HIT_THICKNESS, full.height()),
        )
    } else {
        Rect::from_center_size(
            pos2(full.center().x, boundary),
            vec2(full.width(), HIT_THICKNESS),
        )
    };
    let response = ui.interact(hit, id.with("divider"), Sense::click_and_drag());
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(if horizontal {
            CursorIcon::ResizeHorizontal
        } else {
            CursorIcon::ResizeVertical
        });
    }
    if response.double_clicked() {
        state.length = 0.0;
    } else if response.dragged() {
        let delta = if horizontal {
            response.drag_delta().x
        } else {
            response.drag_delta().y
        };
        let grow = if spec.secondary_first { delta } else { -delta };
        state.length = spec.clamped(length + grow, total);
    }
    let active = response.hovered() || response.dragged();
    let (width, color) = if active {
        (2.0, theme::with_alpha(theme::ACCENT, 0.6))
    } else {
        (1.0, theme::separator())
    };
    let painter = ui.painter();
    if horizontal {
        painter.line_segment(
            [pos2(boundary, full.min.y), pos2(boundary, full.max.y)],
            Stroke::new(width, color),
        );
    } else {
        painter.line_segment(
            [pos2(full.min.x, boundary), pos2(full.max.x, boundary)],
            Stroke::new(width, color),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui;

    const SIDEBAR: SplitSpec = SplitSpec {
        axis: Axis::Horizontal,
        secondary_first: true,
        min_primary: 420.0,
        min_secondary: 200.0,
        max_secondary: 320.0,
        default: DefaultLength::Points(240.0),
    };

    #[test]
    fn unset_length_uses_the_default() {
        assert_eq!(SIDEBAR.resolved(0.0, 1000.0), 240.0);
        let half = SplitSpec {
            default: DefaultLength::Fraction(0.5),
            max_secondary: f32::MAX,
            min_secondary: 100.0,
            min_primary: 100.0,
            ..SIDEBAR
        };
        assert_eq!(half.resolved(0.0, 800.0), 400.0);
    }

    #[test]
    fn length_clamps_to_both_panes_minimums() {
        assert_eq!(SIDEBAR.resolved(500.0, 1000.0), 320.0, "max secondary");
        assert_eq!(SIDEBAR.resolved(50.0, 1000.0), 200.0, "min secondary");
        // 600 wide leaves only 180 for the sidebar once the primary keeps 420.
        assert_eq!(SIDEBAR.resolved(300.0, 600.0), 180.0);
        // Too small for anything: the secondary gives way entirely.
        assert_eq!(SIDEBAR.resolved(300.0, 300.0), 0.0);
    }

    #[test]
    fn splits_draw_headlessly_in_both_orders() {
        for secondary_first in [true, false] {
            let spec = SplitSpec {
                secondary_first,
                axis: Axis::Vertical,
                ..SIDEBAR
            };
            let mut state = SplitState::default();
            let ctx = egui::Context::default();
            for visible in [true, true, true, false] {
                state.visible = visible;
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    show(
                        ui,
                        Id::new("t"),
                        spec,
                        &mut state,
                        |ui| {
                            ui.label("p");
                        },
                        |ui| {
                            ui.label("s");
                        },
                    );
                });
                output.textures_delta.clear();
            }
        }
    }
}
