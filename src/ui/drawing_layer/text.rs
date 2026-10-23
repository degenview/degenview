//! Laying out a drawing's text. One layout job serves painting, measuring and the in-place
//! editor, so the box a note is sized to, the glyphs painted in it and the field the text is typed
//! in all agree to the pixel.

use crate::model::drawing_style::{HorizontalAlign, TextStyle};
use crate::ui::series_render;
use eframe::egui::epaint::text::VariationCoords;
use eframe::egui::{self, FontId, TextFormat, Vec2, text::LayoutJob};

/// The narrowest a text is wrapped, in logical pixels.
pub(super) const MIN_WRAP: f32 = 20.0;

/// `text` in `style`, wrapped at `wrap` (or not at all for an infinite width).
pub(super) fn layout_job(text: &str, style: &TextStyle, wrap: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.append(
        text,
        0.0,
        TextFormat {
            font_id: FontId::proportional(style.size),
            color: series_render::color(style.color),
            italics: style.italic,
            coords: VariationCoords::new([("wght", if style.bold { 700.0 } else { 400.0 })]),
            ..Default::default()
        },
    );
    job.wrap.max_width = wrap.max(MIN_WRAP);
    job.halign = match style.align {
        HorizontalAlign::Left => egui::Align::LEFT,
        HorizontalAlign::Center => egui::Align::Center,
        HorizontalAlign::Right => egui::Align::RIGHT,
    };
    job
}

/// Sizes text with the context that paints it, so a snapshot (which paints in a context of its
/// own) measures with its own fonts and matches the live chart. egui caches the layouts, so
/// measuring a note that has not changed costs a lookup.
#[derive(Clone)]
pub(in crate::ui) struct TextMeasure {
    ctx: egui::Context,
}

impl TextMeasure {
    pub(in crate::ui) fn new(ctx: &egui::Context) -> Self {
        Self { ctx: ctx.clone() }
    }

    /// A measure over an offscreen context with the app's fonts, shared by the tests that
    /// project shapes without a window.
    #[cfg(test)]
    pub(in crate::ui) fn for_tests() -> Self {
        static CONTEXT: std::sync::OnceLock<egui::Context> = std::sync::OnceLock::new();
        let ctx = CONTEXT.get_or_init(|| {
            let ctx = egui::Context::default();
            crate::ui::theme::install_offscreen(&ctx);
            ctx.set_pixels_per_point(2.0);
            // Fonts take effect when a pass begins.
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(100.0, 100.0),
                    )),
                    ..Default::default()
                },
                |_| {},
            );
            output.textures_delta.clear();
            ctx
        });
        Self::new(ctx)
    }

    /// The size `text` takes in `style` when wrapped at `wrap`.
    pub(super) fn size(&self, text: &str, style: &TextStyle, wrap: f32) -> Vec2 {
        let job = layout_job(text, style, wrap);
        self.ctx.fonts_mut(|fonts| fonts.layout_job(job)).size()
    }
}
