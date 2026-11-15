//! Several charts painted as the workspace arranged them: each in the rectangle it had, scaled
//! into the composition, with a hairline between neighbours.
//!
//! The layout's rectangles are mapped onto the composition's body, so a different proportion
//! stretches the arrangement (and each chart is refitted to the plot it gets, as a single chart
//! is, rather than stretched). A part of the layout (a selection across charts) is the same
//! picture cut: the whole arrangement is mapped as if it were all shown, and the painter clips
//! what lies outside.

use super::super::capture::LayoutCapture;
use super::{Cards, chart, styled};
use crate::model::chart::snapshot::framing::Framing;
use crate::model::chart::snapshot::studio::StudioConfig;
use crate::ui::chart::canvas::{CanvasStyle, Fill, paint_background};
use crate::ui::chart::card::SnapshotError;
use eframe::egui::{Painter, Pos2, Rect, Stroke, Vec2};

/// How close, in points, two edges are to be the same edge.
const SAME_EDGE: f32 = 0.5;

/// Where the part of the layout `view` shows, in layout points, lands in `body`.
fn mapper(view: Rect, body: Rect) -> impl Fn(Rect) -> Rect {
    let scale = Vec2::new(
        body.width() / view.width().max(1.0),
        body.height() / view.height().max(1.0),
    );
    move |rect: Rect| {
        let place = |point: Pos2| body.min + (point - view.min) * scale;
        Rect::from_min_max(place(rect.min), place(rect.max))
    }
}

/// Paints the layout's charts into `body` and their dividers.
pub fn paint<'c>(
    painter: &Painter,
    body: Rect,
    layout: &LayoutCapture,
    cards: &Cards<'c>,
    config: &StudioConfig,
    style: &CanvasStyle,
) -> Result<(), SnapshotError> {
    let place = mapper(layout.view(), body);
    let clipped = painter.with_clip_rect(body);
    // The gutters between the charts take the headline chart's canvas, as they do on a plain
    // image, rather than letting the backdrop through as a stripe.
    paint_background(&clipped, body, Fill::Solid(styled::top_color(style.fill)));
    let mut shown = Vec::new();
    for (capture, rect) in &layout.charts {
        let rect = place(*rect);
        if !rect.intersects(body) {
            continue;
        }
        let card = cards(capture.chart_id()).ok_or(SnapshotError::Empty)?;
        // A label that reaches past its chart's edge is cut there, as it is on screen.
        chart::paint(
            &clipped.with_clip_rect(rect),
            rect,
            capture,
            card,
            config,
            &Framing::default(),
        )?;
        shown.push(rect);
    }
    dividers(&clipped, &shown, style);
    Ok(())
}

/// A hairline along every edge two charts share: the edges that are not on the arrangement's
/// outline.
fn dividers(painter: &Painter, rects: &[Rect], style: &CanvasStyle) {
    let Some(outline) = rects.iter().copied().reduce(Rect::union) else {
        return;
    };
    let stroke = Stroke::new(1.0, style.scale_line);
    let inside =
        |edge: f32, bounds: [f32; 2]| bounds.iter().all(|bound| (edge - bound).abs() > SAME_EDGE);
    for rect in rects {
        let across = [outline.min.x, outline.max.x];
        let down = [outline.min.y, outline.max.y];
        if inside(rect.min.x, across) {
            painter.line_segment([rect.left_top(), rect.left_bottom()], stroke);
        }
        if inside(rect.max.x, across) {
            painter.line_segment([rect.right_top(), rect.right_bottom()], stroke);
        }
        if inside(rect.min.y, down) {
            painter.line_segment([rect.left_top(), rect.right_top()], stroke);
        }
        if inside(rect.max.y, down) {
            painter.line_segment([rect.left_bottom(), rect.right_bottom()], stroke);
        }
    }
}
