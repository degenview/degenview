//! A macOS-style popover: a rounded panel with a pointed edge aimed at the control that opened
//! it. It sits below its anchor (above, when there is no room), keeps inside the window and
//! dismisses on Esc or a press anywhere outside it.

use crate::ui::kit;
use eframe::egui::{
    self, Align2, Area, Color32, Context, Frame, Id, Margin, Order, Pos2, Rect, Shadow, Shape,
    Stroke, Ui, pos2, vec2,
};

/// The pointed edge: how wide its base is and how far it stands out of the panel.
const ARROW_WIDTH: f32 = 20.0;
const ARROW_HEIGHT: f32 = 9.0;
/// Space between the anchor and the arrow's tip.
const GAP: f32 = 2.0;
/// What the panel keeps clear of the window's edge.
const EDGE: f32 = 8.0;
/// How near the arrow may come to the panel's rounded corner.
const CORNER_CLEARANCE: f32 = 22.0;
const RADIUS: f32 = 12.0;

/// Which side of the anchor the panel sits on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Side {
    Below,
    Above,
}

/// Where the panel sits along its anchor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Placement {
    /// Centred on the anchor, the usual pointed popover.
    Centered,
    /// Its left edge at the anchor's left edge: for a wide anchor at the left of the window, where
    /// centring would push the panel out over what lies to the left of it.
    Start,
}

/// Shows `body` in a panel `width` wide, pointing at `anchor`. `padding` is the panel's inner
/// margin. Returns whether it was dismissed: Esc, or a press outside the panel and its anchor.
/// `consume_escape` takes the Esc away from whatever draws after it (a sheet that would close too).
pub fn show(
    ctx: &Context,
    id: Id,
    anchor: Rect,
    width: f32,
    padding: i8,
    consume_escape: bool,
    body: impl FnOnce(&mut Ui),
) -> bool {
    show_placed(
        ctx,
        id,
        anchor,
        Placement::Centered,
        width,
        padding,
        consume_escape,
        body,
    )
}

/// [`show`], with the panel placed along its anchor as `placement` says.
#[allow(clippy::too_many_arguments)]
pub fn show_placed(
    ctx: &Context,
    id: Id,
    anchor: Rect,
    placement: Placement,
    width: f32,
    padding: i8,
    consume_escape: bool,
    body: impl FnOnce(&mut Ui),
) -> bool {
    let window = ctx.content_rect();
    let last_height = ctx
        .data(|data| data.get_temp::<f32>(id.with("height")))
        .unwrap_or(320.0);
    let side = side_for(window, anchor, last_height + ARROW_HEIGHT + GAP);
    let left = panel_left(window, anchor, width, placement);
    let (pivot, y) = match side {
        Side::Below => (Align2::LEFT_TOP, anchor.bottom() + GAP),
        Side::Above => (Align2::LEFT_BOTTOM, anchor.top() - GAP),
    };
    let arrow_x = anchor
        .center()
        .x
        .clamp(left + CORNER_CLEARANCE, left + width - CORNER_CLEARANCE);

    // A menu or colour picker opened from inside is not "outside": its clicks and its Esc are its
    // own.
    let nested_open = egui::Popup::is_any_open(ctx);
    let fill = kit::menu_fill();
    let border = Stroke::new(1.0, kit::primary(0.12));

    let output = Area::new(id)
        .order(Order::Foreground)
        .fixed_pos(pos2(left, y))
        .pivot(pivot)
        .constrain(false)
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            if side == Side::Below {
                ui.add_space(ARROW_HEIGHT);
            }
            let panel = Frame::new()
                .fill(fill)
                .corner_radius(RADIUS)
                .stroke(border)
                .inner_margin(Margin::same(padding))
                .shadow(Shadow {
                    offset: [0, 10],
                    blur: 32,
                    spread: 0,
                    color: Color32::from_black_alpha(120),
                })
                .show(ui, |ui| {
                    ui.set_width(width - 2.0 * f32::from(padding) - 2.0);
                    body(ui);
                });
            if side == Side::Above {
                ui.add_space(ARROW_HEIGHT);
            }
            paint_arrow(ui, panel.response.rect, side, arrow_x, fill, border);
        });

    let rect = output.response.rect;
    ctx.data_mut(|data| {
        data.insert_temp(id.with("height"), rect.height() - ARROW_HEIGHT);
    });

    let pressed_outside = !nested_open
        && ctx.input(|input| {
            input.pointer.any_pressed()
                && input
                    .pointer
                    .interact_pos()
                    .is_some_and(|pos| !rect.contains(pos) && !anchor.contains(pos))
        });
    let escaped = !nested_open
        && if consume_escape {
            ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        } else {
            ctx.input(|input| input.key_pressed(egui::Key::Escape))
        };
    pressed_outside || escaped
}

/// Below the anchor when the panel fits there, else above it — whichever has more room if it
/// fits neither.
fn side_for(window: Rect, anchor: Rect, needed: f32) -> Side {
    let below = window.bottom() - EDGE - anchor.bottom();
    let above = anchor.top() - (window.top() + EDGE);
    if below >= needed || below >= above {
        Side::Below
    } else {
        Side::Above
    }
}

/// The panel's left edge: centred on the anchor or starting at it, kept inside the window.
fn panel_left(window: Rect, anchor: Rect, width: f32, placement: Placement) -> f32 {
    let lowest = window.left() + EDGE;
    let highest = (window.right() - EDGE - width).max(lowest);
    let wanted = match placement {
        Placement::Centered => anchor.center().x - width / 2.0,
        Placement::Start => anchor.left(),
    };
    wanted.clamp(lowest, highest)
}

/// The pointed edge, drawn over the panel's border so the two read as one outline.
fn paint_arrow(ui: &Ui, panel: Rect, side: Side, x: f32, fill: Color32, border: Stroke) {
    let (base_y, tip_y) = match side {
        Side::Below => (panel.top(), panel.top() - ARROW_HEIGHT),
        Side::Above => (panel.bottom(), panel.bottom() + ARROW_HEIGHT),
    };
    let half = ARROW_WIDTH / 2.0;
    let left: Pos2 = pos2(x - half, base_y);
    let tip: Pos2 = pos2(x, tip_y);
    let right: Pos2 = pos2(x + half, base_y);
    let painter = ui.painter();
    painter.add(Shape::convex_polygon(
        vec![left, tip, right],
        fill,
        Stroke::NONE,
    ));
    painter.add(Shape::line(vec![left, tip, right], border));
    // Cover the panel's border where the arrow joins it.
    painter.line_segment(
        [pos2(left.x + 0.75, base_y), pos2(right.x - 0.75, base_y)],
        Stroke::new(border.width + 1.0, fill),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 700.0))
    }

    #[test]
    fn the_panel_opens_below_unless_it_only_fits_above() {
        let high = Rect::from_min_size(pos2(400.0, 40.0), vec2(80.0, 20.0));
        let low = Rect::from_min_size(pos2(400.0, 640.0), vec2(80.0, 20.0));
        assert_eq!(side_for(window(), high, 400.0), Side::Below);
        assert_eq!(side_for(window(), low, 400.0), Side::Above);
        // Too tall for either side: the roomier one wins.
        assert_eq!(side_for(window(), high, 900.0), Side::Below);
    }

    #[test]
    fn the_panel_centres_on_its_anchor_and_stays_in_the_window() {
        let anchor = |x: f32| Rect::from_min_size(pos2(x, 40.0), vec2(40.0, 20.0));
        assert_eq!(
            panel_left(window(), anchor(480.0), 440.0, Placement::Centered),
            280.0
        );
        assert_eq!(
            panel_left(window(), anchor(0.0), 440.0, Placement::Centered),
            EDGE
        );
        assert_eq!(
            panel_left(window(), anchor(980.0), 440.0, Placement::Centered),
            1000.0 - EDGE - 440.0
        );
        // A window narrower than the panel pins it to the left edge instead of panicking.
        let narrow = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 700.0));
        assert_eq!(
            panel_left(narrow, anchor(100.0), 440.0, Placement::Centered),
            EDGE
        );
    }

    #[test]
    fn a_start_placed_panel_begins_at_its_anchor_and_stays_in_the_window() {
        let anchor = |x: f32| Rect::from_min_size(pos2(x, 40.0), vec2(200.0, 20.0));
        assert_eq!(
            panel_left(window(), anchor(150.0), 360.0, Placement::Start),
            150.0
        );
        assert_eq!(
            panel_left(window(), anchor(0.0), 360.0, Placement::Start),
            EDGE
        );
        assert_eq!(
            panel_left(window(), anchor(900.0), 360.0, Placement::Start),
            1000.0 - EDGE - 360.0
        );
    }

    #[test]
    fn the_popover_draws_and_stays_up_until_dismissed() {
        let ctx = Context::default();
        let anchor = Rect::from_min_size(pos2(500.0, 100.0), vec2(60.0, 20.0));
        let (mut ran, mut dismissed) = (0, true);
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                dismissed = show(
                    ui.ctx(),
                    Id::new("test-popover"),
                    anchor,
                    300.0,
                    16,
                    true,
                    |ui| {
                        ran += 1;
                        ui.label("Hello");
                    },
                );
            });
            output.textures_delta.clear();
        }
        assert_eq!(ran, 2);
        assert!(!dismissed);
    }
}
