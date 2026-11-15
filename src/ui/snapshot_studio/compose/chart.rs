//! One frozen chart painted into a rectangle of the composition, through the card's own
//! `paint_snapshot`: the composer never draws a chart itself.

use super::super::capture::ChartCapture;
use super::styled;
use crate::model::chart::snapshot::framing::Framing;
use crate::model::chart::snapshot::studio::{StudioConfig, Style};
use crate::ui::chart::canvas::Restyle;
use crate::ui::chart::card::{ChartCard, SnapshotError, SnapshotFrame, SnapshotGeometry};
use eframe::egui::{Painter, Rect, Vec2};

/// How many times the view is refitted to the plot it is drawn on. The plot depends on the
/// axis' width, which depends on the prices it labels, which depend on the plot: two rounds
/// settle it to well within a pixel.
const FIT_ROUNDS: usize = 3;
/// A plot this close (in points) to the one the view was made for is the plot it was made for.
const FIT_TOLERANCE: f32 = 0.25;

/// How the canvas is restyled for `style`.
pub fn restyle(style: Style) -> Restyle {
    match style {
        Style::Chart => Restyle::default(),
        Style::Styled => styled::RESTYLE,
    }
}

/// Paints `chart` into `rect` of `painter` (which clips to what shows) with `config`'s look,
/// and says where the plot landed. The view is `framing` applied to the capture's own, fitted to
/// the plot the chart has in `rect`: a plot of another shape shows other bars and prices at the
/// scale the capture had.
pub fn paint(
    painter: &Painter,
    rect: Rect,
    chart: &ChartCapture,
    card: &ChartCard,
    config: &StudioConfig,
    framing: &Framing,
) -> Result<SnapshotGeometry, SnapshotError> {
    let restyle = restyle(config.style);
    let input = |viewport| {
        let mut input = chart.input(SnapshotFrame {
            viewport: Some(viewport),
            outputs: Some(chart.outputs()),
            include_indicators: config.include.indicators,
            restyle,
        });
        input.hide_all_drawings = !config.include.drawings;
        input
    };
    let base = chart.base();
    let mut plot: Vec2 = base.plot;
    let mut viewport = framing.viewport(base, plot);
    let mut geometry = card.snapshot_geometry_in(painter, rect, &input(viewport))?;
    for _ in 1..FIT_ROUNDS {
        let fitted = geometry.view.plot.size();
        if (fitted - plot).abs().max_elem() <= FIT_TOLERANCE {
            break;
        }
        plot = fitted;
        viewport = framing.viewport(base, plot);
        geometry = card.snapshot_geometry_in(painter, rect, &input(viewport))?;
    }
    card.paint_snapshot(painter, rect, &input(viewport))?;
    Ok(geometry)
}
