//! What the Styled look adds to a chart: the title block over its top (the market's logo, its
//! name and where it trades), and the DegenView mark.
//!
//! Neither follows the app's theme: they sit on the chart's own canvas, so their colours are
//! chosen against the canvas' fill (dark text on a light chart, light on a dark one) and the
//! subline uses the canvas' axis text colour.

use super::super::capture::ChartCapture;
use crate::ui::chart::canvas::{CanvasStyle, Fill, Restyle};
use crate::ui::kit::{self, BOLD, MEDIUM, SEMIBOLD};
use eframe::egui::{
    self, Color32, ColorImage, CornerRadius, Painter, Pos2, Rect, Stroke, TextureOptions, epaint,
    pos2, vec2,
};
use std::sync::{Arc, OnceLock};

/// How much larger the axes' text is drawn in the Styled look.
const TEXT_SCALE: f32 = 1.35;

/// The Styled look's changes to the chart's canvas: no grid or watermark, larger axis text.
pub const RESTYLE: Restyle = Restyle {
    hide_grid: true,
    hide_watermark: true,
    text_scale: TEXT_SCALE,
};

/// The mark's picture: the application's icon.
const APP_ICON: &[u8] = include_bytes!("../../../../assets/brand/app_icon.png");

/// The title block's proportions, as shares of its height.
const PAD_SHARE: f32 = 0.34;
const LOGO_SHARE: f32 = 0.52;
const GAP_SHARE: f32 = 0.22;
const TITLE_SHARE: f32 = 0.31;
const SUBLINE_SHARE: f32 = 0.18;
const LINE_GAP_SHARE: f32 = 0.06;
/// The mark: the wordmark's size and the gap after the icon, as shares of the mark's height; its
/// opacity.
const WORDMARK_SHARE: f32 = 0.62;
const MARK_GAP_SHARE: f32 = 0.4;
const MARK_OPACITY: f32 = 0.55;

/// The icon, decoded once.
fn app_icon() -> &'static Arc<ColorImage> {
    static ICON: OnceLock<Arc<ColorImage>> = OnceLock::new();
    ICON.get_or_init(|| {
        let decoded = image::load_from_memory_with_format(APP_ICON, image::ImageFormat::Png)
            .expect("the application icon is a PNG")
            .to_rgba8();
        let size = [decoded.width() as usize, decoded.height() as usize];
        Arc::new(ColorImage::from_rgba_unmultiplied(size, decoded.as_raw()))
    })
}

/// The colour the canvas starts out with at its top: what the title block's strip is filled with,
/// so it continues the chart under it.
pub fn top_color(fill: Fill) -> Color32 {
    match fill {
        Fill::Solid(color) | Fill::Gradient { top: color, .. } => color,
    }
}

/// Text that reads on `fill`: near black on a light canvas, near white on a dark one.
fn ink_on(fill: Fill) -> Color32 {
    let color = top_color(fill);
    let luminance = (0.2126 * f32::from(color.r())
        + 0.7152 * f32::from(color.g())
        + 0.0722 * f32::from(color.b()))
        / 255.0;
    if luminance > 0.55 {
        Color32::from_rgb(0x14, 0x17, 0x1c)
    } else {
        Color32::from_rgb(0xf4, 0xf6, 0xf8)
    }
}

/// A picture in a circle: `image` loaded as a texture of the painter's context.
fn paint_round_image(
    painter: &Painter,
    rect: Rect,
    name: &str,
    image: &Arc<ColorImage>,
    tint: Color32,
) {
    let texture = painter.ctx().load_texture(
        name,
        egui::ImageData::Color(Arc::clone(image)),
        TextureOptions::LINEAR,
    );
    painter.add(
        epaint::RectShape::filled(rect, CornerRadius::same((rect.width() / 2.0) as u8), tint)
            .with_texture(texture.id(), Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0))),
    );
}

/// The title block, in `strip`, the chart's top: the market's logo (a monogram until it has one),
/// its name in bold and `EXCHANGE · timeframe` under it.
pub fn paint_header(painter: &Painter, strip: Rect, chart: &ChartCapture, style: &CanvasStyle) {
    let height = strip.height();
    let pad = height * PAD_SHARE;
    let diameter = height * LOGO_SHARE;
    let logo = Rect::from_min_size(
        pos2(strip.min.x + pad, strip.center().y - diameter / 2.0),
        vec2(diameter, diameter),
    );
    match chart.logo() {
        Some(image) => {
            paint_round_image(
                painter,
                logo,
                &format!("studio-logo-{}", chart.chart_id()),
                image,
                Color32::WHITE,
            );
            painter.circle_stroke(
                logo.center(),
                diameter / 2.0,
                Stroke::new(1.0, style.scale_line),
            );
        }
        None => kit::monogram_on(
            painter,
            logo,
            &chart.source().base_asset(chart.symbol()),
            0.4,
            Some(BOLD),
        ),
    }

    let left = logo.max.x + height * GAP_SHARE;
    let room = (strip.max.x - pad - left).max(1.0);
    let title = kit::galley_on(
        painter,
        chart.title(),
        (height * TITLE_SHARE).max(12.0),
        BOLD,
        ink_on(style.fill),
        room,
    );
    let subline = kit::galley_on(
        painter,
        &format!(
            "{} · {}",
            chart.source().display_name().to_uppercase(),
            chart.timeframe().label()
        ),
        (height * SUBLINE_SHARE).max(9.0),
        MEDIUM,
        style.text_color,
        room,
    );
    let block = title.size().y + height * LINE_GAP_SHARE + subline.size().y;
    let top = strip.center().y - block / 2.0;
    painter.galley(pos2(left, top), Arc::clone(&title), Color32::WHITE);
    painter.galley(
        pos2(left, top + title.size().y + height * LINE_GAP_SHARE),
        subline,
        Color32::WHITE,
    );
}

/// The DegenView mark in `rect`: the icon in a circle and the name beside it, faint.
pub fn paint_mark(painter: &Painter, rect: Rect, style: &CanvasStyle) {
    let diameter = rect.height();
    let icon = Rect::from_min_size(rect.min, vec2(diameter, diameter));
    paint_round_image(
        painter,
        icon,
        "studio-mark",
        app_icon(),
        Color32::WHITE.gamma_multiply(MARK_OPACITY),
    );
    let name = kit::galley_on(
        painter,
        "DegenView",
        diameter * WORDMARK_SHARE,
        SEMIBOLD,
        ink_on(style.fill).gamma_multiply(MARK_OPACITY),
        f32::INFINITY,
    );
    painter.galley(
        pos2(
            icon.max.x + diameter * MARK_GAP_SHARE,
            icon.center().y - name.size().y / 2.0,
        ),
        name,
        Color32::WHITE,
    );
}
