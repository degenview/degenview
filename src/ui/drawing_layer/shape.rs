//! A drawing projected onto the screen: the polylines to stroke, the polygon to fill, the solid
//! glyphs, the handles and the bounds. It is built once from a drawing and the projection and
//! reused until either changes, so panning the pointer never re-flattens a curve.

use super::Projection;
use super::edit::{BoxHandle, Part};
use super::line_shapes;
use super::note_shapes;
use super::table_shapes;
use crate::model::chart_type::{LineDash, Rgba};
use crate::model::drawing_style::{DrawingStyle, LineEnd, MiddleLine};
use crate::model::{ArrowDirection, Drawing, Geometry};
use crate::ui::drawing_geometry as geo;
use crate::ui::series_render;
use eframe::egui::{self, Mesh, Pos2, Rect, Vec2, pos2};
use std::sync::{Arc, OnceLock};

/// A grab point on a shape.
#[derive(Clone, Copy, Debug)]
pub(super) struct HandleSpot {
    pub part: Part,
    pub pos: Pos2,
    /// Drawn as a hollow ring: a control point rather than a vertex.
    pub control: bool,
}

/// Where a shape's text sits.
#[derive(Clone, Copy, Debug)]
pub(super) enum TextPlace {
    /// Aligned within a box, by the text's own alignment.
    Inside(Rect),
    /// Centred on a point, above or below it.
    At { point: Pos2, above: bool },
    /// Along the visible piece of a line from `from` to `to`: the text's horizontal alignment
    /// says where along it, its vertical alignment on which side, and it is turned to follow the
    /// line unless its orientation says otherwise.
    Line { from: Pos2, to: Pos2 },
    /// Beside a vertical line at `x` that runs from `top` to `bottom`: the horizontal alignment
    /// says which side of the line, the vertical alignment where along it.
    Column { x: f32, top: f32, bottom: f32 },
}

/// A note that opens from its marker (a pin's): where it sits and its outline. Its background,
/// border and text are shown only while it is open.
#[derive(Clone, Debug)]
pub(super) struct Popup {
    pub rect: Rect,
    pub outline: Vec<Pos2>,
}

/// A picture to paint: which stored picture, where, and how opaque.
#[derive(Clone, Debug)]
pub(super) struct PictureDraw {
    pub rect: Rect,
    pub asset: String,
    pub name: String,
    pub opacity: f32,
}

/// A line of a table that is dragged to resize the row or column beside it.
#[derive(Clone, Copy, Debug)]
pub(super) struct Divider {
    pub part: Part,
    pub a: Pos2,
    pub b: Pos2,
}

/// Where a table's cells are: the x of each column's left edge and then the last column's right
/// edge, and the y of each row's top and then the last row's bottom.
#[derive(Clone, Debug)]
pub(super) struct Grid {
    pub xs: Vec<f32>,
    pub ys: Vec<f32>,
}

impl Grid {
    /// The cell under `point`, as `(row, column)`.
    pub(super) fn cell_at(&self, point: Pos2) -> Option<(usize, usize)> {
        let find = |edges: &[f32], at: f32| {
            let last = edges.len().checked_sub(2)?;
            (0..=last).find(|index| {
                at >= edges[*index]
                    && (at < edges[index + 1] || *index == last && at <= edges[index + 1])
            })
        };
        Some((find(&self.ys, point.y)?, find(&self.xs, point.x)?))
    }

    pub(super) fn cell_rect(&self, row: usize, column: usize) -> Option<Rect> {
        Some(Rect::from_min_max(
            pos2(*self.xs.get(column)?, *self.ys.get(row)?),
            pos2(*self.xs.get(column + 1)?, *self.ys.get(row + 1)?),
        ))
    }
}

/// A line with a look of its own: a level of a channel or a pitchfork, a regression band.
#[derive(Clone, Debug)]
pub(super) struct Styled {
    pub path: Vec<Pos2>,
    pub color: Rgba,
    pub width: f32,
    pub dash: LineDash,
}

impl Styled {
    pub(super) fn new(path: Vec<Pos2>, line: &MiddleLine) -> Self {
        Self {
            path,
            color: line.color,
            width: line.width,
            dash: line.dash,
        }
    }
}

/// A filled region with its own colour (one band of a pitchfork or a regression), already cut
/// to the plot.
#[derive(Clone, Debug)]
pub(super) struct Band {
    pub polygon: Vec<Pos2>,
    pub color: Rgba,
}

/// A tag the chart puts on an axis for a line: the price of a horizontal line, the time of a
/// vertical one. The drawing painter is clipped to the plot, so these are painted by the frame's
/// label pass instead.
#[derive(Clone, Copy, Debug)]
pub(super) enum Tag {
    Price { y: f32, price: f64, color: Rgba },
    Time { x: f32, time_ms: i64, color: Rgba },
}

/// A price printed beside the end of a line, pushed away from it along `outward`.
#[derive(Clone, Copy, Debug)]
pub(super) struct EndLabel {
    pub at: Pos2,
    pub price: f64,
    pub outward: Vec2,
}

/// The angle a trend angle shows: from the horizontal at `origin` to the line towards `toward`.
#[derive(Clone, Copy, Debug)]
pub(super) struct AngleMark {
    pub origin: Pos2,
    pub toward: Pos2,
}

/// A line of text a shape prints at a point (Pearson's R).
#[derive(Clone, Debug)]
pub(super) struct Readout {
    pub at: Pos2,
    pub text: String,
}

pub(super) struct Screen {
    /// Open polylines to stroke.
    pub lines: Vec<Vec<Pos2>>,
    /// Lines with a look of their own, drawn under the main ones.
    pub styled: Vec<Styled>,
    /// Filled regions with their own colour.
    pub bands: Vec<Band>,
    /// The bands as meshes, made when the shape is built.
    pub band_meshes: Vec<Arc<Mesh>>,
    /// Axis tags (see [`Tag`]).
    pub tags: Vec<Tag>,
    pub end_labels: Vec<EndLabel>,
    pub angle: Option<AngleMark>,
    pub readout: Option<Readout>,
    /// Closed outlines to stroke.
    pub outlines: Vec<Vec<Pos2>>,
    /// The containers of text (a note's box, a callout's bubble), stroked in the drawing's
    /// border rather than its stroke. The background is `fill`.
    pub frames: Vec<Vec<Pos2>>,
    /// Regions that are on the shape for a click though nothing is painted there (the inside of
    /// a text box with no background).
    pub hit: Vec<Rect>,
    /// A picture, painted in its rectangle.
    pub picture: Option<PictureDraw>,
    /// A table's inner grid lines, stroked in the drawing's border.
    pub rules: Vec<[Pos2; 2]>,
    /// A table's cell texts, each with the cell it sits in.
    pub cell_texts: Vec<(Rect, String)>,
    /// The borders a table's rows and columns are resized by.
    pub dividers: Vec<Divider>,
    /// Where a table's cells are.
    pub grid: Option<Grid>,
    /// Discs painted white on the shape (the hole of a pin).
    pub studs: Vec<(Pos2, f32)>,
    /// Solid discs in a colour of their own: the dot where a note's leader meets its anchor, the
    /// round marker of a signpost.
    pub discs: Vec<(Pos2, f32, Rgba)>,
    /// An emoji painted at a point, at a size.
    pub glyph: Option<(Pos2, &'static str, f32)>,
    /// Text the tool writes itself (a price), shown above the drawing's own.
    pub heading: Option<String>,
    /// A note shown only while it is open.
    pub popup: Option<Popup>,
    /// The popup's background as a mesh, made when the shape is built.
    pub popup_mesh: Option<Arc<Mesh>>,
    /// The region to fill (even-odd); empty for none.
    pub fill: Vec<Pos2>,
    /// Solid polygons painted in the stroke colour: arrowheads, the arrow marker, the marks.
    pub solids: Vec<Vec<Pos2>>,
    /// Construction lines shown only while selected (a curve's control arms).
    pub guides: Vec<[Pos2; 2]>,
    /// A rectangle's middle line.
    pub middle: Option<[Pos2; 2]>,
    /// A dot at the middle of an arrow.
    pub middle_dot: Option<Pos2>,
    /// A freehand stroke as one antialiased mesh.
    pub stroke: Option<Arc<Mesh>>,
    /// A translucent freehand stroke as a coverage map painted as one image.
    pub mask: Option<geo::StrokeMask>,
    /// The mask's texture, made the first time it is painted.
    pub mask_texture: OnceLock<egui::TextureHandle>,
    /// The fill as a mesh in the drawing's fill colour; `None` when it has no fill.
    pub fill_mesh: Option<Arc<Mesh>>,
    /// The two ends of an arrow, for its statistics.
    pub arrow: Option<(Pos2, Pos2)>,
    pub text: Option<TextPlace>,
    pub handles: Vec<HandleSpot>,
    pub bounds: Rect,
}

impl Screen {
    pub(super) fn empty() -> Self {
        Self {
            lines: Vec::new(),
            styled: Vec::new(),
            bands: Vec::new(),
            band_meshes: Vec::new(),
            tags: Vec::new(),
            end_labels: Vec::new(),
            angle: None,
            readout: None,
            outlines: Vec::new(),
            frames: Vec::new(),
            hit: Vec::new(),
            picture: None,
            rules: Vec::new(),
            cell_texts: Vec::new(),
            dividers: Vec::new(),
            grid: None,
            studs: Vec::new(),
            discs: Vec::new(),
            glyph: None,
            heading: None,
            popup: None,
            popup_mesh: None,
            fill: Vec::new(),
            solids: Vec::new(),
            guides: Vec::new(),
            middle: None,
            middle_dot: None,
            stroke: None,
            mask: None,
            mask_texture: OnceLock::new(),
            fill_mesh: None,
            arrow: None,
            text: None,
            handles: Vec::new(),
            bounds: Rect::NOTHING,
        }
    }

    /// Computes the bounds from everything the shape touches, grown by `margin`.
    pub(super) fn finish(mut self, margin: f32) -> Option<Self> {
        let mut all: Vec<Pos2> = Vec::new();
        for path in self
            .lines
            .iter()
            .chain(&self.outlines)
            .chain(&self.frames)
            .chain(&self.solids)
        {
            all.extend(path);
        }
        for rect in &self.hit {
            all.extend([rect.left_top(), rect.right_bottom()]);
        }
        for styled in &self.styled {
            all.extend(&styled.path);
        }
        for band in &self.bands {
            all.extend(&band.polygon);
        }
        for guide in &self.guides {
            all.extend(guide);
        }
        all.extend(self.handles.iter().map(|handle| handle.pos));
        all.extend(self.middle.iter().flatten());
        all.extend(self.fill.iter());
        if !geo::all_finite(&all) {
            return None;
        }
        self.bounds = geo::bounds(&all)?.expand(margin);
        Some(self)
    }

    /// Whether `point` is on the shape's body: near a stroke, inside a solid, or inside the
    /// fill when it has one.
    pub(super) fn hits_body(&self, point: Pos2, tolerance: f32, filled: bool, open: bool) -> bool {
        let in_popup = open
            && self
                .popup
                .as_ref()
                .is_some_and(|popup| popup.rect.expand(tolerance).contains(point));
        if in_popup {
            return true;
        }
        if !self.bounds.expand(tolerance).contains(point) {
            return false;
        }
        let near = |path: &Vec<Pos2>, closed: bool| {
            geo::distance_to_polyline(point, path, closed) <= tolerance
        };
        self.lines.iter().any(|path| near(path, false))
            || self.styled.iter().any(|styled| near(&styled.path, false))
            || (filled
                && self
                    .bands
                    .iter()
                    .any(|band| geo::polygon_contains(&band.polygon, point)))
            || self.outlines.iter().any(|path| near(path, true))
            || self.frames.iter().any(|path| near(path, true))
            || self
                .hit
                .iter()
                .any(|rect| rect.expand(tolerance).contains(point))
            || self
                .middle
                .is_some_and(|line| geo::distance_to_segment(point, line[0], line[1]) <= tolerance)
            || self
                .solids
                .iter()
                .any(|solid| geo::polygon_contains(solid, point) || near(solid, true))
            || (filled && !self.fill.is_empty() && geo::polygon_contains(&self.fill, point))
    }

    /// The row or column border within `reach` of `point`, the nearest when two meet.
    pub(super) fn divider_at(&self, point: Pos2, reach: f32) -> Option<Part> {
        self.dividers
            .iter()
            .map(|divider| {
                (
                    divider,
                    geo::distance_to_segment(point, divider.a, divider.b),
                )
            })
            .filter(|(_, distance)| *distance <= reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(divider, _)| divider.part)
    }

    /// The handle under `point`, the nearest when several overlap.
    pub(super) fn handle_at(&self, point: Pos2, reach: f32) -> Option<Part> {
        self.handles
            .iter()
            .map(|handle| (handle, handle.pos.distance(point)))
            .filter(|(_, distance)| *distance <= reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(handle, _)| handle.part)
    }
}

/// The arrowhead size for a line of `width`.
fn head_size(width: f32) -> (f32, f32) {
    let length = 8.0 + width * 2.5;
    (length, length * 0.5)
}

/// `points` with arrowheads at the chosen ends: the line is pulled back so it does not poke
/// through a head, and each head is a solid triangle.
pub(super) fn with_heads(
    mut points: Vec<Pos2>,
    left: LineEnd,
    right: LineEnd,
    width: f32,
) -> (Vec<Pos2>, Vec<Vec<Pos2>>) {
    let mut heads = Vec::new();
    let (length, half) = head_size(width);
    if points.len() >= 2 && right == LineEnd::Arrow {
        let (from, tip) = (points[points.len() - 2], points[points.len() - 1]);
        let reach = length.min(from.distance(tip) * 0.9);
        if let Some(head) = geo::arrowhead(from, tip, reach, half * reach / length) {
            heads.push(head.to_vec());
            let direction = (tip - from) / from.distance(tip).max(1e-3);
            let last = points.len() - 1;
            points[last] = tip - direction * (reach * 0.8);
        }
    }
    if points.len() >= 2 && left == LineEnd::Arrow {
        let (from, tip) = (points[1], points[0]);
        let reach = length.min(from.distance(tip) * 0.9);
        if let Some(head) = geo::arrowhead(from, tip, reach, half * reach / length) {
            heads.push(head.to_vec());
            let direction = (tip - from) / from.distance(tip).max(1e-3);
            points[0] = tip - direction * (reach * 0.8);
        }
    }
    (points, heads)
}

fn vertex_handles(points: &[Pos2]) -> Vec<HandleSpot> {
    points
        .iter()
        .enumerate()
        .map(|(index, pos)| HandleSpot {
            part: Part::Anchor(index),
            pos: *pos,
            control: false,
        })
        .collect()
}

pub(super) fn box_handles(rect: Rect) -> Vec<HandleSpot> {
    BoxHandle::ALL
        .iter()
        .map(|handle| HandleSpot {
            part: Part::Box(*handle),
            pos: handle.position(rect),
            control: false,
        })
        .collect()
}

/// The shape of a drawing that has its own screen geometry (the brush and every new tool);
/// `None` for the original line drawings (which paint themselves) and for unusable geometry.
pub(super) fn build(drawing: &Drawing, proj: &Projection) -> Option<Screen> {
    match drawing {
        Drawing::Brush { points, style, .. } => freehand(points, style, proj),
        Drawing::Shape {
            geometry: Geometry::Highlighter { points },
            style,
            ..
        } => freehand(points, style, proj),
        Drawing::Shape {
            geometry,
            style,
            text,
            ..
        } => finished(shape(geometry, style, text, proj)?, style),
        _ => None,
    }
}

/// `screen` with the meshes of its fill and its bands made.
fn finished(mut screen: Screen, style: &DrawingStyle) -> Option<Screen> {
    if style.fill.enabled && !screen.fill.is_empty() {
        let mesh = geo::fill_mesh(&screen.fill, series_render::color(style.fill.color));
        screen.fill_mesh = Some(Arc::new(mesh));
    }
    if style.fill.enabled
        && let Some(popup) = &screen.popup
    {
        let mesh = geo::fill_mesh(&popup.outline, series_render::color(style.fill.color));
        screen.popup_mesh = Some(Arc::new(mesh));
    }
    if style.fill.enabled {
        screen.band_meshes = screen
            .bands
            .iter()
            .filter(|band| band.polygon.len() >= 3)
            .map(|band| {
                Arc::new(geo::fill_mesh(
                    &band.polygon,
                    series_render::color(band.color),
                ))
            })
            .collect();
    }
    Some(screen)
}

fn freehand(
    points: &[crate::model::Anchor],
    style: &DrawingStyle,
    proj: &Projection,
) -> Option<Screen> {
    let raw: Vec<Pos2> = points.iter().map(|point| proj.point(*point)).collect();
    if !geo::all_finite(&raw) {
        return None;
    }
    let smoothed = geo::smooth(&raw, 2, 40_000);
    // Smoothing doubles the points; most are now on straight runs, so drop them again.
    let smoothed: Vec<Pos2> = geo::simplify_indices(&smoothed, 0.15)
        .into_iter()
        .map(|index| smoothed[index])
        .collect();
    let width = style.stroke.width;
    let mut screen = Screen::empty();
    // A stroke wholly off the plot is neither painted nor picked, so nothing is built for it;
    // panning it into view builds it then.
    let on_plot =
        geo::bounds(&smoothed).is_some_and(|bounds| bounds.expand(width).intersects(proj.plot));
    // Translucent strokes are painted from a coverage map so a stroke that crosses itself
    // stays one even alpha; opaque ones are a plain mesh.
    if on_plot && style.stroke.color[3] < 255 {
        screen.mask = geo::rasterize_stroke(&smoothed, width, proj.pixels_per_point);
    }
    if on_plot && screen.mask.is_none() {
        screen.stroke = geo::stroke_mesh(
            &smoothed,
            width,
            series_render::color(style.stroke.color),
            1.0,
        )
        .map(Arc::new);
    }
    screen.lines = vec![if smoothed.len() == 1 {
        // A dot: give it a hair of length so distance tests treat it as a segment.
        vec![smoothed[0], smoothed[0] + eframe::egui::vec2(0.01, 0.0)]
    } else {
        smoothed
    }];
    screen.finish(width / 2.0 + 2.0)
}

fn shape(
    geometry: &Geometry,
    style: &DrawingStyle,
    text: &str,
    proj: &Projection,
) -> Option<Screen> {
    let anchors: Vec<Pos2> = geometry.anchors().iter().map(|a| proj.point(*a)).collect();
    if !geo::all_finite(&anchors) {
        return None;
    }
    let width = style.stroke.width;
    let mut screen = Screen::empty();
    match geometry {
        Geometry::Highlighter { .. } => return None,
        Geometry::ArrowMarker { .. } => {
            let (a, b) = (anchors[0], anchors[1]);
            if let Some(polygon) = geo::arrow_marker(a, b, width) {
                screen.solids.push(polygon);
            } else {
                screen.lines.push(vec![a, b]);
            }
            screen.handles = vertex_handles(&anchors);
            screen.text = Some(TextPlace::At {
                point: a.lerp(b, 0.5),
                above: true,
            });
        }
        Geometry::Arrow { .. } => {
            let (a, b) = (anchors[0], anchors[1]);
            let (line, heads) = with_heads(vec![a, b], style.head_start, style.head_end, width);
            screen.lines.push(line);
            screen.solids = heads;
            screen.arrow = Some((a, b));
            screen.middle_dot = style.middle_point.then(|| a.lerp(b, 0.5));
            screen.handles = vertex_handles(&anchors);
            screen.text = Some(TextPlace::At {
                point: a.lerp(b, 0.5),
                above: true,
            });
        }
        Geometry::ArrowMark { direction, .. } => {
            let up = *direction == ArrowDirection::Up;
            let glyph = geo::arrow_mark(anchors[0], up);
            let far = glyph.iter().map(|point| point.y).fold(
                if up { f32::MIN } else { f32::MAX },
                |acc, y| {
                    if up { acc.max(y) } else { acc.min(y) }
                },
            );
            screen.text = Some(TextPlace::At {
                point: pos2(anchors[0].x, far),
                above: !up,
            });
            screen.solids.push(glyph);
            screen.handles = vec![HandleSpot {
                part: Part::Anchor(0),
                pos: anchors[0],
                control: false,
            }];
        }
        Geometry::Rectangle { .. } => {
            let rect = Rect::from_two_pos(anchors[0], anchors[1]);
            let left = if style.extend_left {
                proj.plot.left().min(rect.left())
            } else {
                rect.left()
            };
            let right = if style.extend_right {
                proj.plot.right().max(rect.right())
            } else {
                rect.right()
            };
            let (top, bottom) = (rect.top(), rect.bottom());
            let polygon = vec![
                pos2(left, top),
                pos2(right, top),
                pos2(right, bottom),
                pos2(left, bottom),
            ];
            if style.extend_left || style.extend_right {
                screen.lines.push(vec![pos2(left, top), pos2(right, top)]);
                screen
                    .lines
                    .push(vec![pos2(left, bottom), pos2(right, bottom)]);
                if !style.extend_left {
                    screen.lines.push(vec![pos2(left, top), pos2(left, bottom)]);
                }
                if !style.extend_right {
                    screen
                        .lines
                        .push(vec![pos2(right, top), pos2(right, bottom)]);
                }
            } else {
                screen.outlines.push(polygon.clone());
            }
            if style.middle_line.enabled {
                let y = rect.center().y;
                screen.middle = Some([pos2(left, y), pos2(right, y)]);
            }
            screen.fill = polygon;
            screen.handles = box_handles(rect);
            screen.text = Some(TextPlace::Inside(rect));
        }
        Geometry::RotatedRectangle { .. } => {
            let (a, b, c) = (anchors[0], anchors[1], anchors[2]);
            match geo::rotated_rect(a, b, c) {
                Some(corners) => {
                    screen.outlines.push(corners.to_vec());
                    screen.fill = corners.to_vec();
                    screen.handles = vec![
                        HandleSpot {
                            part: Part::Anchor(0),
                            pos: a,
                            control: false,
                        },
                        HandleSpot {
                            part: Part::Anchor(1),
                            pos: b,
                            control: false,
                        },
                        HandleSpot {
                            part: Part::Anchor(2),
                            pos: corners[2].lerp(corners[3], 0.5),
                            control: false,
                        },
                    ];
                }
                None => {
                    screen.lines.push(vec![a, b]);
                    screen.handles = vertex_handles(&[a, b, c]);
                }
            }
        }
        Geometry::Path { .. } => {
            let (line, heads) =
                with_heads(anchors.clone(), style.head_start, style.head_end, width);
            screen.lines.push(line);
            screen.solids = heads;
            screen.handles = vertex_handles(&anchors);
        }
        Geometry::Polyline { closed, .. } => {
            if *closed && anchors.len() >= 3 {
                screen.outlines.push(anchors.clone());
                screen.fill = anchors.clone();
            } else {
                screen.lines.push(anchors.clone());
            }
            screen.handles = vertex_handles(&anchors);
        }
        Geometry::Circle { .. } => {
            let (center, edge) = (anchors[0], anchors[1]);
            let radius = center.distance(edge);
            let polygon = geo::ellipse_polygon(center, radius, radius);
            if polygon.is_empty() {
                return None;
            }
            screen.outlines.push(polygon.clone());
            screen.fill = polygon;
            screen.handles = vec![
                HandleSpot {
                    part: Part::Anchor(1),
                    pos: edge,
                    control: false,
                },
                HandleSpot {
                    part: Part::Body,
                    pos: center,
                    control: true,
                },
            ];
            screen.text = Some(TextPlace::Inside(Rect::from_center_size(
                center,
                eframe::egui::vec2(radius * 2.0, radius * 2.0),
            )));
        }
        Geometry::Ellipse { .. } => {
            let rect = Rect::from_two_pos(anchors[0], anchors[1]);
            let polygon =
                geo::ellipse_polygon(rect.center(), rect.width() / 2.0, rect.height() / 2.0);
            if polygon.is_empty() {
                return None;
            }
            screen.outlines.push(polygon.clone());
            screen.fill = polygon;
            screen.handles = box_handles(rect);
            screen.text = Some(TextPlace::Inside(rect));
        }
        Geometry::Triangle { .. } => {
            screen.outlines.push(anchors.clone());
            screen.fill = anchors.clone();
            screen.handles = vertex_handles(&anchors);
        }
        Geometry::Arc { .. } => {
            let (start, end, through) = (anchors[0], anchors[1], anchors[2]);
            match geo::arc_through(start, end, through) {
                Some(arc) => {
                    let curve = arc.flatten();
                    screen.fill = curve.clone();
                    screen.lines.push(curve);
                }
                None => screen.lines.push(vec![start, end]),
            }
            screen.handles = vertex_handles(&anchors);
        }
        Geometry::Curve { .. } => {
            let (a, b, control) = (anchors[0], anchors[1], anchors[2]);
            let curve = geo::flatten_quad(a, control, b);
            curved(
                &mut screen,
                curve,
                style,
                proj,
                [a - control, b - control],
                [a, b],
            );
            screen.guides = vec![[a, control], [b, control]];
            screen.handles = curve_handles(&anchors);
        }
        Geometry::DoubleCurve { .. } => {
            let (a, b, c1, c2) = (anchors[0], anchors[1], anchors[2], anchors[3]);
            let curve = geo::flatten_cubic(a, c1, c2, b);
            curved(&mut screen, curve, style, proj, [a - c1, b - c2], [a, b]);
            screen.guides = vec![[a, c1], [b, c2]];
            screen.handles = curve_handles(&anchors);
        }
        Geometry::Table(table) => return table_shapes::project(table, style, proj),
        Geometry::Image(image) => return note_shapes::picture(image, &anchors),
        Geometry::Annotation { .. } | Geometry::Leader { .. } | Geometry::Signpost { .. } => {
            return note_shapes::project(geometry, style, text, proj, &anchors);
        }
        _ => return line_shapes::project(geometry, style, proj, anchors),
    }
    screen.finish(width / 2.0 + 8.0)
}

/// A curve's handles: the two ends solid, the controls hollow.
fn curve_handles(anchors: &[Pos2]) -> Vec<HandleSpot> {
    anchors
        .iter()
        .enumerate()
        .map(|(index, pos)| HandleSpot {
            part: Part::Anchor(index),
            pos: *pos,
            control: index >= 2,
        })
        .collect()
}

/// Fills in a curve's line, heads, extensions and fill. `away` is the direction each end leaves
/// the curve (from its control towards the end), `ends` the two end points.
fn curved(
    screen: &mut Screen,
    mut curve: Vec<Pos2>,
    style: &DrawingStyle,
    proj: &Projection,
    away: [eframe::egui::Vec2; 2],
    ends: [Pos2; 2],
) {
    screen.fill = curve.clone();
    // "Left" and "right" are the ends as they sit on screen, whichever was placed first.
    let left = usize::from(ends[0].x > ends[1].x);
    for (wanted, end) in [(style.extend_left, left), (style.extend_right, 1 - left)] {
        if wanted && away[end].length() > 1e-3 {
            let (_, reach) =
                geo::extend_to_rect(ends[end], ends[end] + away[end], proj.plot, false, true);
            screen.lines.push(vec![ends[end], reach]);
        }
    }
    let (line, heads) = {
        let moved = std::mem::take(&mut curve);
        with_heads(moved, style.head_start, style.head_end, style.stroke.width)
    };
    screen.lines.insert(0, line);
    screen.solids.extend(heads);
}
