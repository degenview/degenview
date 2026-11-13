//! The sheet type scale and one-line text helpers the shared pieces draw with.

use eframe::egui::{self, Color32, FontId, Galley, RichText, Ui, text::LayoutJob};
use std::sync::Arc;

pub(in crate::ui) use crate::ui::kit::{BOLD, MEDIUM, SEMIBOLD};

/// The macOS text styles the Swift sheet uses, in points.
pub(in crate::ui) mod size {
    pub const TITLE2: f32 = 17.0;
    pub const BODY: f32 = 13.0;
    pub const CALLOUT: f32 = 12.0;
    pub const SUBHEADLINE: f32 = 11.0;
    pub const CAPTION: f32 = 10.0;
}

pub(in crate::ui) fn text(
    value: impl Into<String>,
    size: f32,
    weight: f32,
    color: Color32,
) -> RichText {
    RichText::new(value)
        .size(size)
        .variation("wght", weight)
        .color(color)
}

/// A one-line galley at `weight`, cut with an ellipsis past `max_width`.
pub(in crate::ui) fn galley(
    ui: &Ui,
    value: &str,
    size: f32,
    weight: f32,
    color: Color32,
    max_width: f32,
) -> Arc<Galley> {
    galley_rows(ui, value, size, weight, color, max_width, 1)
}

/// Like [`galley`], wrapping onto up to `max_rows` lines before the ellipsis.
pub(in crate::ui) fn galley_rows(
    ui: &Ui,
    value: &str,
    size: f32,
    weight: f32,
    color: Color32,
    max_width: f32,
    max_rows: usize,
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
    job.wrap.max_width = max_width.max(1.0);
    job.wrap.max_rows = max_rows;
    job.wrap.overflow_character = Some('…');
    ui.painter().layout_job(job)
}
