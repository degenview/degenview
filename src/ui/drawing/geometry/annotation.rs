//! The shapes of the text and note tools, as pure functions on screen points: the bubble of a
//! comment or a callout with its tail, the pin, the flag and where a pin's note opens. Nothing
//! here knows about drawings, fonts or the chart, so every shape is tested for the awkward
//! placements (a tail that must reach round a corner, a note near the edge of the plot).

use eframe::egui::{Pos2, Rect, Vec2, pos2, vec2};

/// Points per quarter-circle of a rounded corner.
const ARC_STEPS: usize = 6;

/// A box of `text` plus `pad` on every side, never smaller than `min`. The text's size is rounded
/// up so the text always fits the box it is laid out in.
pub fn box_size(text: Vec2, pad: f32, min: Vec2) -> Vec2 {
    vec2(
        (text.x.ceil() + 2.0 * pad).max(min.x),
        (text.y.ceil() + 2.0 * pad).max(min.y),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

/// The edge of `rect` a tail to `tip` leaves from: the one `tip` is furthest beyond. `None` when
/// the tip is on or inside the box.
fn tail_side(rect: Rect, tip: Pos2) -> Option<Side> {
    [
        (Side::Top, rect.top() - tip.y),
        (Side::Right, tip.x - rect.right()),
        (Side::Bottom, tip.y - rect.bottom()),
        (Side::Left, rect.left() - tip.x),
    ]
    .into_iter()
    .filter(|(_, beyond)| *beyond > 0.5)
    .max_by(|a, b| a.1.total_cmp(&b.1))
    .map(|(side, _)| side)
}

/// The closed outline of `rect` with rounded corners of `radius`, and a tail to `tip` when there
/// is one: a triangle of `tail_width` on the edge nearest the tip, slid along it as far as the
/// corners allow, so the outline never crosses itself however the tip is placed. Clockwise on the
/// screen, starting at the top edge.
pub fn bubble(rect: Rect, radius: f32, tip: Option<Pos2>, tail_width: f32) -> Vec<Pos2> {
    let radius = radius.clamp(0.0, rect.width().min(rect.height()) / 2.0);
    let side = tip.and_then(|tip| tail_side(rect, tip));
    // How wide the tail may be on its edge, and where along it the tail is centred.
    let along = |low: f32, high: f32, wanted: f32| {
        let room = (high - low - 2.0 * radius).max(0.0);
        let width = tail_width.min(room);
        let center = if room > 0.0 {
            wanted.clamp(low + radius + width / 2.0, high - radius - width / 2.0)
        } else {
            (low + high) / 2.0
        };
        (center, width)
    };
    let corner = |points: &mut Vec<Pos2>, center: Pos2, from: f32| {
        for step in 0..=ARC_STEPS {
            let angle = (from + 90.0 * step as f32 / ARC_STEPS as f32).to_radians();
            points.push(center + vec2(angle.cos(), angle.sin()) * radius);
        }
    };
    let (left, top, right, bottom) = (rect.left(), rect.top(), rect.right(), rect.bottom());
    let mut points = Vec::new();
    corner(&mut points, pos2(left + radius, top + radius), 180.0);
    if let (Some(Side::Top), Some(tip)) = (side, tip) {
        let (center, width) = along(left, right, tip.x);
        if width >= 1.0 {
            points.extend([
                pos2(center - width / 2.0, top),
                tip,
                pos2(center + width / 2.0, top),
            ]);
        }
    }
    corner(&mut points, pos2(right - radius, top + radius), 270.0);
    if let (Some(Side::Right), Some(tip)) = (side, tip) {
        let (center, width) = along(top, bottom, tip.y);
        if width >= 1.0 {
            points.extend([
                pos2(right, center - width / 2.0),
                tip,
                pos2(right, center + width / 2.0),
            ]);
        }
    }
    corner(&mut points, pos2(right - radius, bottom - radius), 0.0);
    if let (Some(Side::Bottom), Some(tip)) = (side, tip) {
        let (center, width) = along(left, right, tip.x);
        if width >= 1.0 {
            points.extend([
                pos2(center + width / 2.0, bottom),
                tip,
                pos2(center - width / 2.0, bottom),
            ]);
        }
    }
    corner(&mut points, pos2(left + radius, bottom - radius), 90.0);
    if let (Some(Side::Left), Some(tip)) = (side, tip) {
        let (center, width) = along(top, bottom, tip.y);
        if width >= 1.0 {
            points.extend([
                pos2(left, center + width / 2.0),
                tip,
                pos2(left, center - width / 2.0),
            ]);
        }
    }
    points
}

/// The box of a comment: above its anchor, the tail leaving from near the left end.
pub fn comment_rect(tip: Pos2, size: Vec2) -> Rect {
    Rect::from_min_size(
        pos2(tip.x - COMMENT_TAIL_INSET, tip.y - COMMENT_GAP - size.y),
        size,
    )
}

/// The width of the small tail joining a pin's note to its pin.
pub const PIN_NOTE_TAIL: f32 = 10.0;

/// How far (px) the comment's box sits above its anchor, and how far its left edge is left of it.
pub const COMMENT_GAP: f32 = 12.0;
pub const COMMENT_TAIL_INSET: f32 = 16.0;
pub const COMMENT_TAIL: f32 = 12.0;

/// The box of a price label: to the right of its anchor, centred on the price.
pub fn price_label_rect(tip: Pos2, size: Vec2) -> Rect {
    Rect::from_min_size(pos2(tip.x + PRICE_LABEL_GAP, tip.y - size.y / 2.0), size)
}

/// How far (px) the label's box is right of its anchor: the length of its pointer.
pub const PRICE_LABEL_GAP: f32 = 9.0;
pub const PRICE_LABEL_TAIL: f32 = 12.0;

/// How tall the pole of a flag is, and the flag's size, in px.
pub const FLAG_POLE: f32 = 26.0;
pub const FLAG_SIZE: Vec2 = vec2(17.0, 12.0);

/// A flag mark planted at `base`: its pole, and the flag as a swallow-tailed polygon at the pole's
/// top, flying to the right.
pub fn flag(base: Pos2) -> ([Pos2; 2], Vec<Pos2>) {
    let top = pos2(base.x, base.y - FLAG_POLE);
    let (width, height) = (FLAG_SIZE.x, FLAG_SIZE.y);
    let flag = vec![
        top,
        top + vec2(width, 0.0),
        top + vec2(width * 0.78, height / 2.0),
        top + vec2(width, height),
        top + vec2(0.0, height),
    ];
    ([base, top], flag)
}

/// The radius of a pin's head and the distance from its point to the head's centre, in px.
pub const PIN_RADIUS: f32 = 8.0;
pub const PIN_HEIGHT: f32 = 21.0;

/// A pin planted at `tip`: a drop with its point on the anchor, and the centre of the head (where
/// the pin's stud sits).
pub fn pin(tip: Pos2) -> (Vec<Pos2>, Pos2) {
    let center = pos2(tip.x, tip.y - PIN_HEIGHT);
    // The tangents from the tip meet the head this far round from the direction of the tip.
    let spread = (PIN_RADIUS / PIN_HEIGHT).acos().to_degrees();
    let (from, to) = (90.0 + spread, 450.0 - spread);
    let steps = 28;
    let mut points = vec![tip];
    for step in 0..=steps {
        let angle = (from + (to - from) * step as f32 / steps as f32).to_radians();
        points.push(center + vec2(angle.cos(), angle.sin()) * PIN_RADIUS);
    }
    (points, center)
}

/// Where a pin's note opens: centred above the pin, or below its point when there is no room
/// above, and slid sideways to stay on the plot. Never over the pin itself unless it is too tall
/// for either side, in which case it is held on the plot on the side with more room. A note larger
/// than the plot is cut to it; the text inside is clipped to what is left.
pub fn pin_note_rect(tip: Pos2, size: Vec2, plot: Rect) -> Rect {
    let gap = 6.0;
    let margin = 4.0;
    let size = vec2(
        size.x.min((plot.width() - 2.0 * margin).max(0.0)),
        size.y.min((plot.height() - 2.0 * margin).max(0.0)),
    );
    let above = tip.y - PIN_HEIGHT - PIN_RADIUS - gap - size.y;
    let below = tip.y + gap;
    let top = if above >= plot.top() + margin {
        above
    } else if below + size.y <= plot.bottom() - margin {
        below
    } else {
        let room_above = tip.y - PIN_HEIGHT - PIN_RADIUS - gap - plot.top();
        let room_below = plot.bottom() - below;
        let wanted = if room_below >= room_above {
            below
        } else {
            above
        };
        wanted.clamp(
            plot.top() + margin,
            (plot.bottom() - margin - size.y).max(plot.top() + margin),
        )
    };
    let low = plot.left() + margin;
    let high = (plot.right() - margin - size.x).max(low);
    let left = (tip.x - size.x / 2.0).clamp(low, high);
    Rect::from_min_size(pos2(left, top), size)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether the open segments `a`-`b` and `c`-`d` cross (touching ends do not count).
    fn cross(a: Pos2, b: Pos2, c: Pos2, d: Pos2) -> bool {
        let side = |p: Pos2, q: Pos2, r: Pos2| (q - p).x * (r - p).y - (q - p).y * (r - p).x;
        let (d1, d2) = (side(c, d, a), side(c, d, b));
        let (d3, d4) = (side(a, b, c), side(a, b, d));
        d1 * d2 < -1e-3 && d3 * d4 < -1e-3
    }

    fn is_simple(points: &[Pos2]) -> bool {
        let count = points.len();
        for i in 0..count {
            let (a, b) = (points[i], points[(i + 1) % count]);
            for j in i + 2..count {
                if i == 0 && j == count - 1 {
                    continue;
                }
                if cross(a, b, points[j], points[(j + 1) % count]) {
                    return false;
                }
            }
        }
        true
    }

    #[test]
    fn a_box_fits_its_text_and_padding_and_never_shrinks_below_its_minimum() {
        let size = box_size(vec2(40.2, 14.1), 5.0, vec2(0.0, 0.0));
        assert_eq!(size, vec2(51.0, 25.0));
        assert_eq!(
            box_size(vec2(1.0, 1.0), 5.0, vec2(30.0, 20.0)),
            vec2(30.0, 20.0)
        );
    }

    #[test]
    fn a_bubble_without_a_tip_is_a_rounded_rectangle_inside_its_rect() {
        let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(80.0, 30.0));
        let outline = bubble(rect, 6.0, None, 12.0);
        assert_eq!(outline.len(), 4 * (ARC_STEPS + 1));
        assert!(outline.iter().all(|p| rect.expand(0.01).contains(*p)));
        assert!(is_simple(&outline));
        // A tip inside the box makes no tail either.
        assert_eq!(bubble(rect, 6.0, Some(rect.center()), 12.0), outline);
    }

    #[test]
    fn a_tail_reaches_its_tip_from_the_nearest_edge_without_crossing_the_outline() {
        let rect = Rect::from_min_size(pos2(100.0, 100.0), vec2(90.0, 34.0));
        let mut count = 0;
        // Tips on a grid all round the box, near and far, including past every corner.
        for xi in -6..=14 {
            for yi in -6..=10 {
                let tip = pos2(
                    100.0 + xi as f32 * 20.0 - 40.0,
                    100.0 + yi as f32 * 17.0 - 40.0,
                );
                if rect.expand(0.5).contains(tip) {
                    continue;
                }
                let outline = bubble(rect, 6.0, Some(tip), 12.0);
                assert!(outline.contains(&tip), "{tip:?}");
                assert!(is_simple(&outline), "{tip:?} makes a crossing outline");
                // Only the tip is outside the rectangle.
                let outside = outline
                    .iter()
                    .filter(|p| !rect.expand(0.01).contains(**p))
                    .count();
                assert_eq!(outside, 1, "{tip:?}");
                count += 1;
            }
        }
        assert!(count > 200);
    }

    #[test]
    fn a_tail_slides_along_its_edge_but_stays_clear_of_the_corners() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 40.0));
        // Far to the left of a tip above: the tail is clamped to the first straight stretch.
        let outline = bubble(rect, 8.0, Some(pos2(-500.0, -30.0)), 12.0);
        let base: Vec<&Pos2> = outline
            .iter()
            .filter(|p| p.y == 0.0 && p.x >= 0.0)
            .collect();
        assert!(
            base.iter().all(|p| p.x >= 8.0 - 0.01 && p.x <= 92.0 + 0.01),
            "{base:?}"
        );
        assert!(is_simple(&outline));
        // A box too narrow for a tail between its corners has none rather than a broken one.
        let narrow = Rect::from_min_size(pos2(0.0, 0.0), vec2(14.0, 40.0));
        let outline = bubble(narrow, 8.0, Some(pos2(7.0, -20.0)), 12.0);
        assert!(is_simple(&outline));
    }

    #[test]
    fn the_comment_and_the_price_label_point_at_their_anchor() {
        let tip = pos2(300.0, 200.0);
        let size = vec2(80.0, 30.0);
        let comment = comment_rect(tip, size);
        assert!(comment.bottom() < tip.y && comment.left() < tip.x && tip.x < comment.right());
        let outline = bubble(comment, 6.0, Some(tip), COMMENT_TAIL);
        assert!(outline.contains(&tip) && is_simple(&outline));
        let label = price_label_rect(tip, size);
        assert_eq!(label.center().y, tip.y);
        assert_eq!(label.left() - tip.x, PRICE_LABEL_GAP);
        let outline = bubble(label, 4.0, Some(tip), PRICE_LABEL_TAIL);
        assert!(outline.contains(&tip) && is_simple(&outline));
        // The pointer leaves from the middle of the left edge.
        assert!(outline.iter().any(|p| p.x == label.left() && p.y < tip.y));
        assert!(outline.iter().any(|p| p.x == label.left() && p.y > tip.y));
    }

    #[test]
    fn a_flag_stands_on_its_anchor_and_flies_to_the_right() {
        let base = pos2(50.0, 80.0);
        let ([foot, top], flag) = flag(base);
        assert_eq!(foot, base);
        assert_eq!(top, pos2(50.0, 80.0 - FLAG_POLE));
        assert_eq!(flag[0], top);
        assert!(
            flag.iter()
                .all(|p| p.x >= base.x && p.y >= top.y && p.y <= top.y + FLAG_SIZE.y)
        );
        assert!(is_simple(&flag));
    }

    #[test]
    fn a_pin_is_a_drop_with_its_point_on_the_anchor() {
        let tip = pos2(120.0, 200.0);
        let (drop, head) = pin(tip);
        assert_eq!(drop[0], tip);
        assert_eq!(head, pos2(120.0, 200.0 - PIN_HEIGHT));
        // Every other point lies on the head's circle, none below the point.
        for point in &drop[1..] {
            assert!(((*point - head).length() - PIN_RADIUS).abs() < 0.01);
            assert!(point.y <= tip.y);
        }
        assert!(is_simple(&drop));
    }

    #[test]
    fn a_pins_note_opens_above_it_and_stays_on_the_plot_and_off_the_pin() {
        let plot = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 500.0));
        let size = vec2(220.0, 60.0);
        let tip = pos2(400.0, 300.0);
        let note = pin_note_rect(tip, size, plot);
        assert!(note.bottom() < tip.y - PIN_HEIGHT - PIN_RADIUS);
        assert_eq!(note.center().x, 400.0);
        // Near the top there is no room above: it opens below the point.
        let low = pin_note_rect(pos2(400.0, 40.0), size, plot);
        assert!(low.top() > 40.0);
        // Near either side it is slid back onto the plot.
        for x in [-50.0, 5.0, 795.0, 900.0] {
            let note = pin_note_rect(pos2(x, 300.0), size, plot);
            assert!(
                note.left() >= plot.left() && note.right() <= plot.right(),
                "{x}"
            );
        }
        // A note wider than the plot is pinned to its left edge, not thrown off it.
        let wide = pin_note_rect(tip, vec2(900.0, 40.0), plot);
        assert_eq!(wide.left(), plot.left() + 4.0);
    }

    #[test]
    fn a_pins_note_taller_than_either_side_is_held_on_the_plot() {
        let plot = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 500.0));
        for (tip, height) in [
            (pos2(400.0, 250.0), 460.0),
            (pos2(400.0, 100.0), 700.0),
            (pos2(400.0, 450.0), 700.0),
            (pos2(400.0, 250.0), 300.0),
        ] {
            let note = pin_note_rect(tip, vec2(220.0, height), plot);
            assert!(plot.contains_rect(note), "{tip:?} {height}: {note:?}");
        }
    }
}
