//! The Screenshot Studio's image: one paint function ([`paint`]) that the preview and the export
//! both run, so they differ only in the scale they are drawn at.
//!
//! In order: the backdrop (when the image has one), the soft shadow under the chart, the chart (or
//! each chart of a layout) through the card's own `paint_snapshot`, the rounding of the chart's
//! corners, and for the Styled look the title block and the DegenView mark. The composer never
//! draws a chart: what a chart looks like is the card's alone, and the composer says only where
//! it goes, which bars and prices it shows and how its canvas is restyled.
//!
//! Layout is in points ([`composition::layout`]) and text is laid out by egui at the scene's
//! scale, so an image at 2× is drawn again at 2× and never enlarged. The live chart is only
//! read: the capture is a frozen copy and the card paints it without a change to itself.

pub mod background;
mod chart;
mod layout;
mod styled;
mod surround;

use super::capture::Capture;
use crate::model::chart::snapshot::composition::{self, Composition};
use crate::model::chart::snapshot::studio::StudioConfig;
use crate::model::chart::snapshot::{SnapshotDimensions, SnapshotScale};
use crate::service::snapshot::raster::Scene;
use crate::ui::chart::canvas::{CanvasStyle, Fill, paint_background};
use crate::ui::chart::card::{ChartCard, SnapshotError};
use crate::ui::chart::snapshot_scene;
use eframe::egui::{Color32, Painter, Rect, vec2};
use surround::Surround;

/// The largest scale a preview is drawn at: what an export at 2× would be.
const PREVIEW_MAX_SCALE: f32 = SnapshotScale::X2.factor();

/// How the composer finds the live card of a captured chart by its id.
pub type Cards<'c> = dyn Fn(u64) -> Option<&'c ChartCard> + 'c;

/// What a scene is drawn for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Target {
    /// The preview: scaled to fit within `max_px` (width, height) pixels, and no larger than an
    /// export at 2× would be.
    Preview { max_px: (u32, u32) },
    /// The image to save or copy, at `scale` (or at a smaller one where it would be too large).
    Export(SnapshotScale),
}

/// Why no image was made.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ComposeError {
    #[error("{}", .0.message())]
    Chart(SnapshotError),
    #[error("This image is too large to draw. Try a different size or background.")]
    TooLarge,
}

impl From<SnapshotError> for ComposeError {
    fn from(error: SnapshotError) -> Self {
        Self::Chart(error)
    }
}

/// How `composition` is drawn for `target`: the scale and the image's size in pixels.
fn dimensions(composition: &Composition, target: Target) -> Option<SnapshotDimensions> {
    match target {
        Target::Export(scale) => composition::dimensions(composition, scale),
        Target::Preview { max_px } => {
            let (width, height) = (composition.size.x, composition.size.y);
            if !(width >= 1.0 && height >= 1.0 && max_px.0 > 0 && max_px.1 > 0) {
                return None;
            }
            let scale = (max_px.0 as f32 / width)
                .min(max_px.1 as f32 / height)
                .min(PREVIEW_MAX_SCALE);
            // Rounded down to a thousandth, so the pixels never pass the box asked for.
            let pixels_per_point = (scale * 1000.0).floor() / 1000.0;
            (pixels_per_point > 0.0).then(|| SnapshotDimensions {
                pixels_per_point,
                width: ((width * pixels_per_point).round() as u32).clamp(1, max_px.0),
                height: ((height * pixels_per_point).round() as u32).clamp(1, max_px.1),
            })
        }
    }
}

/// The canvas of the chart the image is headed by, as it is drawn in `config`'s look.
fn canvas_of<'c>(
    capture: &Capture,
    cards: &Cards<'c>,
    config: &StudioConfig,
) -> Result<CanvasStyle, SnapshotError> {
    let card = capture
        .headline()
        .and_then(|chart| cards(chart.chart_id()))
        .ok_or(SnapshotError::Empty)?;
    Ok(CanvasStyle::resolve(&card.appearance().canvas).restyled(chart::restyle(config.style)))
}

/// Paints the Studio's image into `rect` of `painter`, whose top-left corner is the image's. Its
/// size is the composition's (`composition::layout` of the capture and `config`); `rect` only
/// says where it starts. `cards` gives the live card of a captured chart: the card paints, the
/// capture is what it paints.
pub fn paint<'c>(
    painter: &Painter,
    rect: Rect,
    capture: &Capture,
    cards: &Cards<'c>,
    config: &StudioConfig,
) -> Result<(), SnapshotError> {
    let composition = composition::layout(capture.size(), config);
    let style = canvas_of(capture, cards, config)?;
    let origin = rect.min.to_vec2();
    let surround = config.background.has_frame().then(|| {
        Surround::new(
            &composition,
            Rect::from_min_size(rect.min, composition.size),
            config.background,
        )
    });
    if let Some(surround) = &surround {
        surround.paint_behind(painter);
    }

    let body = composition.body().translate(origin);
    if let Some(header) = composition.header {
        paint_background(
            painter,
            header.translate(origin),
            Fill::Solid(styled::top_color(style.fill)),
        );
    }
    match capture {
        Capture::Single(single) => {
            let card = cards(single.chart_id()).ok_or(SnapshotError::Empty)?;
            chart::paint(
                &painter.with_clip_rect(body),
                body,
                single,
                card,
                config,
                &config.framing,
            )?;
        }
        Capture::Layout(arrangement) => {
            layout::paint(painter, body, arrangement, cards, config, &style)?;
        }
    }
    if let Some(surround) = &surround {
        surround.paint_over(painter);
    }
    if let (Some(header), Some(headline)) = (composition.header, capture.headline()) {
        let mark = composition.branding.map(|mark| mark.translate(origin));
        styled::paint_header(painter, header.translate(origin), mark, headline, &style);
    }
    Ok(())
}

/// The colour the image starts out as: what shows in a pixel the painting does not reach.
fn backdrop<'c>(capture: &Capture, cards: &Cards<'c>, config: &StudioConfig) -> Color32 {
    if config.background.has_frame() {
        return background::color_at(config.background, vec2(0.5, 0.5));
    }
    canvas_of(capture, cards, config).map_or(Color32::BLACK, |style| styled::top_color(style.fill))
}

/// The Studio's image for `target`, drawn by [`paint`] on a context of its own at the scale the
/// target asks for: what the preview shows and what the export saves, differing only in scale.
pub fn scene<'c>(
    capture: &Capture,
    cards: &Cards<'c>,
    config: &StudioConfig,
    target: Target,
) -> Result<(Scene, SnapshotDimensions), ComposeError> {
    let composition = composition::layout(capture.size(), config);
    let dimensions = dimensions(&composition, target).ok_or(ComposeError::TooLarge)?;
    let scene = snapshot_scene::render(
        (dimensions.width, dimensions.height),
        dimensions.pixels_per_point,
        backdrop(capture, cards, config),
        Some(&capture.images()),
        |painter, rect| paint(painter, rect, capture, cards, config),
    )?;
    Ok((scene, dimensions))
}

#[cfg(test)]
mod tests;
