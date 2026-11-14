//! The macOS type scale with real SF weights, and the galleys every kit widget sets its words in:
//! one line cut with an ellipsis, figures, and wrapped paragraphs.

use eframe::egui::{self, Color32, FontId, Galley, RichText, Ui, text::LayoutJob};
use std::sync::Arc;

/// SF Pro weights on its `wght` axis.
pub const REGULAR: f32 = 400.0;
pub const MEDIUM: f32 = 510.0;
pub const SEMIBOLD: f32 = 590.0;
pub const BOLD: f32 = 700.0;

/// The macOS text styles, in points.
pub mod size {
    pub const TITLE2: f32 = 17.0;
    pub const HEADLINE: f32 = 13.0;
    pub const BODY: f32 = 13.0;
    pub const SUBHEADLINE: f32 = 11.0;
    pub const CAPTION: f32 = 10.0;
}

pub fn text(value: impl Into<String>, size: f32, weight: f32, color: Color32) -> RichText {
    RichText::new(value)
        .size(size)
        .variation("wght", weight)
        .color(color)
}

/// A one-line galley at `weight`, cut with an ellipsis past `max_width`.
pub fn galley(
    ui: &Ui,
    value: &str,
    size: f32,
    weight: f32,
    color: Color32,
    max_width: f32,
) -> Arc<Galley> {
    galley_font(
        ui,
        value,
        FontId::proportional(size),
        weight,
        color,
        max_width,
    )
}

/// A price or time. SF's tabular figures are not reachable from egui, and the monospaced face
/// reads heavier than Swift's `.monospacedDigit()`; these columns are right-aligned, so the
/// proportional face is what matches.
pub fn digits(
    ui: &Ui,
    value: &str,
    size: f32,
    weight: f32,
    color: Color32,
    max_width: f32,
) -> Arc<Galley> {
    galley_font(
        ui,
        value,
        FontId::proportional(size),
        weight,
        color,
        max_width,
    )
}

/// Text at `weight` that wraps at words inside `width` and shows every line.
pub fn paragraph(
    ui: &Ui,
    value: &str,
    size: f32,
    weight: f32,
    color: Color32,
    width: f32,
) -> Arc<Galley> {
    let mut job = LayoutJob::default();
    job.append(
        value,
        0.0,
        egui::TextFormat {
            font_id: FontId::proportional(size),
            color,
            coords: egui::epaint::text::VariationCoords::new([("wght", weight)]),
            ..Default::default()
        },
    );
    job.wrap.max_width = width.max(1.0);
    ui.painter().layout_job(job)
}

fn galley_font(
    ui: &Ui,
    value: &str,
    font_id: FontId,
    weight: f32,
    color: Color32,
    max_width: f32,
) -> Arc<Galley> {
    let mut job = LayoutJob::default();
    job.append(
        value,
        0.0,
        egui::TextFormat {
            font_id,
            color,
            coords: egui::epaint::text::VariationCoords::new([("wght", weight)]),
            ..Default::default()
        },
    );
    job.wrap.max_width = max_width.max(1.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    ui.painter().layout_job(job)
}
