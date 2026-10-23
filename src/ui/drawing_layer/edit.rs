//! Editing a placed drawing: which part the pointer grabbed, what dragging it does, and the
//! structural edits (inserting and removing a vertex, duplicating). All pure functions of a
//! drawing and a projection, so every tool is tested without a window.

use super::Projection;
use crate::model::{Anchor, Drawing, Geometry};
use crate::ui::drawing_render::{self, RulerCorner};
use crate::ui::line_geometry as lines;
use eframe::egui::{Pos2, Vec2};

/// How far a duplicate is offset from the original, in px (right and down).
pub(super) const DUPLICATE_OFFSET: Vec2 = Vec2::new(16.0, 16.0);

/// One of the eight handles of a box-shaped drawing (a rectangle or an ellipse).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BoxHandle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

impl BoxHandle {
    pub(super) const ALL: [Self; 8] = [
        Self::TopLeft,
        Self::Top,
        Self::TopRight,
        Self::Right,
        Self::BottomRight,
        Self::Bottom,
        Self::BottomLeft,
        Self::Left,
    ];

    /// `Some(true)` for a handle on the left edge, `Some(false)` on the right, `None` when it
    /// does not move the horizontal extent.
    fn horizontal(self) -> Option<bool> {
        match self {
            Self::TopLeft | Self::BottomLeft | Self::Left => Some(true),
            Self::TopRight | Self::BottomRight | Self::Right => Some(false),
            Self::Top | Self::Bottom => None,
        }
    }

    /// `Some(true)` for a handle on the top edge, `Some(false)` on the bottom.
    fn vertical(self) -> Option<bool> {
        match self {
            Self::TopLeft | Self::TopRight | Self::Top => Some(true),
            Self::BottomLeft | Self::BottomRight | Self::Bottom => Some(false),
            Self::Left | Self::Right => None,
        }
    }

    /// Where the handle sits on the box.
    pub(super) fn position(self, rect: eframe::egui::Rect) -> Pos2 {
        let x = match self.horizontal() {
            Some(true) => rect.left(),
            Some(false) => rect.right(),
            None => rect.center().x,
        };
        let y = match self.vertical() {
            Some(true) => rect.top(),
            Some(false) => rect.bottom(),
            None => rect.center().y,
        };
        Pos2::new(x, y)
    }
}

/// The part of a drawing the pointer is on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Part {
    /// The shape itself: dragging moves all of it.
    Body,
    /// The anchor at this index of `Drawing::anchors`.
    Anchor(usize),
    Box(BoxHandle),
    /// The right edge of a text box: dragging it sets the width the text wraps at.
    Width,
    /// The border to the right of this column of a table: dragging it resizes the column.
    Column(usize),
    /// The border under this row of a table: dragging it resizes the row.
    Row(usize),
    /// The label of a note or a signpost (the second anchor): dragging it moves the label by
    /// the pointer's travel, so a handle that is not on the anchor itself does not make it jump.
    Label,
    /// A ruler's corner.
    Corner(RulerCorner),
}

/// A box's two corner anchors with `handle` dragged to `to`, the opposite edges left in place.
/// Each axis is written to whichever stored anchor owns that edge, so the stored order survives
/// and dragging past the opposite edge flips the box without a special case.
pub(super) fn resize_box(a: Anchor, b: Anchor, handle: BoxHandle, to: Anchor) -> (Anchor, Anchor) {
    let (mut a, mut b) = (a, b);
    if let Some(left) = handle.horizontal() {
        if left == (a.time_ms <= b.time_ms) {
            a.time_ms = to.time_ms;
        } else {
            b.time_ms = to.time_ms;
        }
    }
    if let Some(top) = handle.vertical() {
        if top == (a.price >= b.price) {
            a.price = to.price;
        } else {
            b.price = to.price;
        }
    }
    (a, b)
}

/// `original` after dragging `part` from `press` to `now`. `target` is where an anchor handle
/// lands (the pointer, or the magnet's pick); a body drag moves every anchor by the pointer's
/// pixel delta, which keeps it right on a log scale too.
pub(super) fn edited(
    original: &Drawing,
    part: Part,
    press: Pos2,
    now: Pos2,
    target: Anchor,
    proj: &Projection,
) -> Drawing {
    let delta = now - press;
    let mut result = original.clone();
    match part {
        Part::Body => translate(&mut result, delta, proj),
        // A pinned table has no anchor to set: its corner moves with the pointer instead.
        Part::Anchor(_) if pinned(&result) => translate(&mut result, delta, proj),
        Part::Anchor(index) => {
            if let Some(anchor) = result.anchors_mut().into_iter().nth(index) {
                *anchor = target;
            }
        }
        Part::Column(column) => {
            if let Some(grid) = grid_of(original, proj)
                && let Some(Geometry::Table(table)) = result.geometry_mut()
                && let (Some(left), Some(right)) = (grid.xs.get(column), grid.xs.get(column + 1))
            {
                table.resize_column(column, (right - left) + delta.x);
            }
        }
        Part::Row(row) => {
            if let Some(grid) = grid_of(original, proj)
                && let Some(Geometry::Table(table)) = result.geometry_mut()
                && let (Some(top), Some(bottom)) = (grid.ys.get(row), grid.ys.get(row + 1))
            {
                table.resize_row(row, (bottom - top) + delta.y);
            }
        }
        Part::Box(handle) if matches!(original.geometry(), Some(Geometry::Table(_))) => {
            if handle == BoxHandle::BottomRight
                && let Some(grid) = grid_of(original, proj)
                && let (Some(width), Some(height)) = (
                    grid.xs.last().zip(grid.xs.first()).map(|(r, l)| r - l),
                    grid.ys.last().zip(grid.ys.first()).map(|(b, t)| b - t),
                )
                && let Some(Geometry::Table(table)) = result.geometry_mut()
            {
                table.scale_to(width + delta.x, height + delta.y);
            }
        }
        Part::Box(handle) => match &mut result {
            Drawing::Shape {
                geometry: Geometry::Rectangle { a, b } | Geometry::Ellipse { a, b },
                ..
            } => (*a, *b) = resize_box(*a, *b, handle, target),
            Drawing::Shape {
                geometry: Geometry::Image(image),
                ..
            } => (image.a, image.b) = resize_image(image, handle, now, proj),
            Drawing::Shape {
                geometry: Geometry::Card { card, .. },
                ..
            } if handle == BoxHandle::BottomRight => card.resize_by(delta.x, delta.y),
            _ => {}
        },
        Part::Label => {
            if let Some(label) = result.anchors_mut().into_iter().nth(1) {
                *label = proj.anchor(proj.point(*label) + delta);
            }
        }
        Part::Width => {
            if let Some(width) = dragged_width(original, proj, press, now)
                && let Drawing::Shape { style, .. } = &mut result
            {
                style.wrap = true;
                style.box_width = width;
            }
        }
        Part::Corner(corner) => {
            if let Drawing::Ruler { a, b, .. } = &mut result {
                (*a, *b) = drawing_render::resize_ruler(*a, *b, corner, target);
            }
        }
    }
    keep_channel_width(original, &mut result, part, press, now, proj);
    if let (Some(edited), Some(before)) = (result.geometry_mut(), original.geometry()) {
        edited.constrain(before);
    }
    result
}

/// The outer width a text box has after its right edge is dragged from `press` to `now`: its
/// width now, moved by the pointer's travel. `None` for a drawing with no text box.
fn dragged_width(original: &Drawing, proj: &Projection, press: Pos2, now: Pos2) -> Option<f32> {
    let screen = super::shape::build(original, proj)?;
    let Some(super::shape::TextPlace::Inside(rect)) = screen.text else {
        return None;
    };
    Some((rect.width() + (now.x - press.x)).clamp(
        crate::model::drawing_style::MIN_BOX_WIDTH,
        crate::model::drawing_style::MAX_BOX_WIDTH,
    ))
}

/// Moves a drawing by `delta` px on screen. A drawing at chart points moves every anchor, which
/// keeps it right on a log scale too; a table pinned to the screen moves its pinned place by the
/// same fraction of the plot.
pub(super) fn translate(drawing: &mut Drawing, delta: Vec2, proj: &Projection) {
    if let Some(Geometry::Table(table)) = drawing.geometry_mut()
        && let Some(place) = &mut table.screen
    {
        place.x += delta.x / proj.plot.width().max(1.0);
        place.y += delta.y / proj.plot.height().max(1.0);
        *place = place.clamped();
        return;
    }
    for anchor in drawing.anchors_mut() {
        *anchor = proj.anchor(proj.point(*anchor) + delta);
    }
}

/// Whether the drawing is a table pinned to the screen.
fn pinned(drawing: &Drawing) -> bool {
    matches!(drawing.geometry(), Some(Geometry::Table(table)) if table.screen.is_some())
}

/// Where the cells of a table drawing are on screen now.
fn grid_of(drawing: &Drawing, proj: &Projection) -> Option<super::shape::Grid> {
    super::shape::build(drawing, proj)?.grid
}

/// The smallest a picture is resized to, on each side, in px.
const MIN_PICTURE: f32 = 16.0;

/// The corners of a picture with `handle` dragged to `pointer`. The opposite edges stay where
/// they are. When the picture keeps its shape, a corner takes the larger of the two moves and the
/// other side follows from the shape, and an edge takes its own move and the other side follows
/// about its middle. Worked in screen space, so it is right on a log scale too, and written back
/// to whichever stored corner each end is nearest, so the order they were stored in survives.
pub(super) fn resize_image(
    image: &crate::model::image_content::ImageData,
    handle: BoxHandle,
    pointer: Pos2,
    proj: &Projection,
) -> (Anchor, Anchor) {
    let (pa, pb) = (proj.point(image.a), proj.point(image.b));
    let rect = eframe::egui::Rect::from_two_pos(pa, pb);
    let aspect = image.aspect();
    let (left, right) = (
        handle.horizontal() == Some(true),
        handle.horizontal() == Some(false),
    );
    let (top, bottom) = (
        handle.vertical() == Some(true),
        handle.vertical() == Some(false),
    );
    // The edges that stay put.
    let fixed_x = if left { rect.right() } else { rect.left() };
    let fixed_y = if top { rect.bottom() } else { rect.top() };
    let mut min = rect.min;
    let mut max = rect.max;
    if image.keep_aspect {
        if (left || right) && (top || bottom) {
            let across = (pointer.x - fixed_x).abs().max(MIN_PICTURE);
            let down = (pointer.y - fixed_y).abs().max(MIN_PICTURE);
            let width = across.max(down * aspect);
            let height = width / aspect;
            let x = if left {
                fixed_x - width
            } else {
                fixed_x + width
            };
            let y = if top {
                fixed_y - height
            } else {
                fixed_y + height
            };
            min = Pos2::new(x.min(fixed_x), y.min(fixed_y));
            max = Pos2::new(x.max(fixed_x), y.max(fixed_y));
        } else if left || right {
            let width = (pointer.x - fixed_x).abs().max(MIN_PICTURE);
            let height = width / aspect;
            let x = if left {
                fixed_x - width
            } else {
                fixed_x + width
            };
            min = Pos2::new(x.min(fixed_x), rect.center().y - height / 2.0);
            max = Pos2::new(x.max(fixed_x), rect.center().y + height / 2.0);
        } else {
            let height = (pointer.y - fixed_y).abs().max(MIN_PICTURE);
            let width = height * aspect;
            let y = if top {
                fixed_y - height
            } else {
                fixed_y + height
            };
            min = Pos2::new(rect.center().x - width / 2.0, y.min(fixed_y));
            max = Pos2::new(rect.center().x + width / 2.0, y.max(fixed_y));
        }
    } else {
        if left || right {
            let x = pointer.x;
            min.x = x.min(fixed_x);
            max.x = x.max(fixed_x);
        }
        if top || bottom {
            let y = pointer.y;
            min.y = y.min(fixed_y);
            max.y = y.max(fixed_y);
        }
    }
    corners_for(image, eframe::egui::Rect::from_min_max(min, max), proj)
}

/// The two corner anchors of `image` once it fills `rect` on screen. Each end goes back to the
/// corner it was at, so the order they were stored in survives.
pub(super) fn corners_for(
    image: &crate::model::image_content::ImageData,
    rect: eframe::egui::Rect,
    proj: &Projection,
) -> (Anchor, Anchor) {
    let (pa, pb) = (proj.point(image.a), proj.point(image.b));
    let before = eframe::egui::Rect::from_two_pos(pa, pb);
    let corner = |original: Pos2| {
        Pos2::new(
            if original.x <= before.center().x {
                rect.left()
            } else {
                rect.right()
            },
            if original.y <= before.center().y {
                rect.top()
            } else {
                rect.bottom()
            },
        )
    };
    (proj.anchor(corner(pa)), proj.anchor(corner(pb)))
}

/// A parallel channel keeps the price distance between its edges when an end of the first edge
/// is dragged, and the handle on the second edge moves that distance by the pointer's vertical
/// travel alone. Both are worked in screen space, where the distance is a vertical offset, so
/// they are right on a log scale too.
fn keep_channel_width(
    original: &Drawing,
    result: &mut Drawing,
    part: Part,
    press: Pos2,
    now: Pos2,
    proj: &Projection,
) {
    let (
        Some(Geometry::ParallelChannel { a, b, c }),
        Some(Geometry::ParallelChannel {
            a: new_a,
            b: new_b,
            c: new_c,
        }),
    ) = (original.geometry(), result.geometry_mut())
    else {
        return;
    };
    let (pa, pb, pc) = (proj.point(*a), proj.point(*b), proj.point(*c));
    if (pb.x - pa.x).abs() < 1e-3 {
        // A vertical first edge has no price distance; the generic edit stands.
        return;
    }
    let offset = lines::channel_offset(pa, pb, pc).y;
    let (edge_a, edge_b) = (proj.point(*new_a), proj.point(*new_b));
    let offset = match part {
        Part::Anchor(2) => offset + (now.y - press.y),
        Part::Anchor(0 | 1) => offset,
        _ => return,
    };
    if let Some(y) = lines::y_at(edge_a, edge_b, pc.x) {
        *new_c = proj.anchor(Pos2::new(pc.x, y + offset));
    }
}

/// The vertices of an editable polyline-like shape.
fn vertices_mut(drawing: &mut Drawing) -> Option<(&mut Vec<Anchor>, usize)> {
    match drawing {
        Drawing::Shape {
            geometry: Geometry::Path { points },
            ..
        } => Some((points, 2)),
        Drawing::Shape {
            geometry: Geometry::Polyline { points, closed },
            ..
        } => {
            let minimum = if *closed { 3 } else { 2 };
            Some((points, minimum))
        }
        _ => None,
    }
}

/// `drawing` with a vertex added after vertex `segment` (the segment's start), at `at`.
/// `None` when the drawing has no vertices to add to, or is at its size limit.
pub(super) fn with_inserted_vertex(
    drawing: &Drawing,
    segment: usize,
    at: Anchor,
) -> Option<Drawing> {
    let mut result = drawing.clone();
    let (points, _) = vertices_mut(&mut result)?;
    if points.len() >= crate::model::MAX_POINTS || segment >= points.len() {
        return None;
    }
    points.insert(segment + 1, at);
    Some(result)
}

/// `drawing` without vertex `index`; `None` when that would leave too few for the shape.
pub(super) fn without_vertex(drawing: &Drawing, index: usize) -> Option<Drawing> {
    let mut result = drawing.clone();
    let (points, minimum) = vertices_mut(&mut result)?;
    if points.len() <= minimum || index >= points.len() {
        return None;
    }
    points.remove(index);
    Some(result)
}

/// A copy of `drawing` under `id`, moved by [`DUPLICATE_OFFSET`] on screen so it does not sit
/// exactly on the original, and unlocked.
pub(super) fn duplicate(drawing: &Drawing, id: u64, proj: &Projection) -> Drawing {
    let mut copy = drawing.clone().with_id(id);
    translate(&mut copy, DUPLICATE_OFFSET, proj);
    copy.set_locked(false);
    copy
}
