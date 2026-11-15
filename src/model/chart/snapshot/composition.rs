//! The Studio's image as a layout: how large it is, where the chart sits in it and where the
//! title block and the DegenView mark go. Everything is in points, so the preview and the export
//! lay out identically and only the scale they are drawn at differs; pixels appear in
//! [`dimensions`], through the same limits every snapshot has.
//!
//! The chart is never stretched. A different proportion gives the chart a different shape (a
//! wider plot shows more bars, a taller one more prices, see [`framing`](super::framing)) around
//! the area the capture had, and a backdrop adds an even margin.

use super::studio::{Area, StudioConfig, Style};
use super::{SnapshotDimensions, SnapshotScale};
use eframe::egui::{Rect, Vec2, pos2, vec2};

/// The margin around the chart on a backdrop, as a share of the image's shorter side (a decision).
pub const PADDING_SHARE: f64 = 0.06;
/// The least margin, in points, however small the image.
pub const MIN_PADDING: f64 = 24.0;
/// The title block's height as a share of the chart's, within [`HEADER_HEIGHT`] (a decision).
const HEADER_SHARE: f32 = 0.15;
/// The title block's least and most height, in points.
pub const HEADER_HEIGHT: (f32, f32) = (44.0, 96.0);
/// The DegenView mark's height as a share of the chart's, within [`BRANDING_HEIGHT`].
const BRANDING_SHARE: f32 = 0.05;
/// The mark's least and most height, in points.
pub const BRANDING_HEIGHT: (f32, f32) = (18.0, 28.0);
/// The title block's proportions, as shares of the strip's height: the margin at its left and at
/// the mark's right, the logo's diameter and the gap after it. The composer sets the title in them.
pub const HEADER_PAD_SHARE: f32 = 0.34;
pub const HEADER_LOGO_SHARE: f32 = 0.52;
pub const HEADER_GAP_SHARE: f32 = 0.22;
/// The mark's width over its height: the app icon and the word beside it.
const BRANDING_ASPECT: f32 = 6.5;

/// Where everything goes in the image, in points from its top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Composition {
    /// The whole image.
    pub size: Vec2,
    /// The chart, header included. Its plot is what the framing's dials act on.
    pub chart: Rect,
    /// The margin on every side of the chart; zero without a backdrop.
    pub padding: f32,
    /// A Styled image's title block: the strip across the top of the chart, its content at the
    /// left.
    pub header: Option<Rect>,
    /// A Styled image's DegenView mark, at the right end of the title strip.
    pub branding: Option<Rect>,
}

impl Composition {
    /// The rest of the chart once the header has taken its strip: where the candles, the axes and
    /// the panes are drawn.
    pub fn body(&self) -> Rect {
        match self.header {
            Some(header) => {
                Rect::from_min_max(pos2(self.chart.min.x, header.max.y), self.chart.max)
            }
            None => self.chart,
        }
    }
}

/// A side of the capture the layout can work with.
fn usable(side: f32) -> f64 {
    if side.is_finite() {
        f64::from(side).max(1.0)
    } else {
        1.0
    }
}

/// The margin around a chart whose shorter side is `short`, when that margin is
/// [`PADDING_SHARE`] of the image's shorter side, which holds the margin twice.
fn margin_around(short: f64) -> f64 {
    (PADDING_SHARE * short / (1.0 - 2.0 * PADDING_SHARE)).max(MIN_PADDING)
}

/// The width and height of an image of `terms` proportions whose chart (inside the margin
/// `framed` asks for) has about `area` square points: both sides are whole multiples of the
/// proportions' terms, so the ratio is exact at whole scales.
fn preset_size((across, down): (u32, u32), area: f64, framed: bool) -> (f64, f64) {
    let (a, b) = (f64::from(across), f64::from(down));
    let ratio = a / b;
    let shorter = ratio.min(1.0);
    let s = PADDING_SHARE;
    // The image's height H, in each case from "chart sides multiply to `area`".
    let height = if framed {
        // The margin at its least: (r·H - m)(H - m) = area, m = twice the margin.
        let m = 2.0 * MIN_PADDING;
        let b2 = m * (ratio + 1.0);
        let at_least = (b2 + (b2 * b2 - 4.0 * ratio * (m * m - area)).sqrt()) / (2.0 * ratio);
        if s * at_least * shorter <= MIN_PADDING {
            at_least
        } else {
            // The margin as a share: H²·(r - 2s·min(r, 1))(1 - 2s·min(r, 1)) = area.
            (area / ((ratio - 2.0 * s * shorter) * (1.0 - 2.0 * s * shorter))).sqrt()
        }
    } else {
        (area / ratio).sqrt()
    };
    let units = |k: f64| (a * k, b * k);
    let mut k = (height / b).round().max(1.0);
    // A tiny capture on a backdrop still leaves the chart a side to stand on.
    let leaves_room = |k: f64| {
        let (w, h) = units(k);
        let margin = (PADDING_SHARE * w.min(h)).max(MIN_PADDING);
        w - 2.0 * margin >= 1.0 && h - 2.0 * margin >= 1.0
    };
    while framed && !leaves_room(k) {
        k += 1.0;
    }
    units(k)
}

/// The image for a capture of `capture` points (its chart, axes included) in `config`'s style,
/// size and background.
///
/// Chart style, or Styled at Original: the chart is the capture, as it was. A preset size makes
/// the image exactly those proportions, the chart about the capture's area in the shape that is
/// left once the margin is taken: one side grows, the other shrinks. A backdrop adds the margin,
/// [`PADDING_SHARE`] of the image's shorter side (at least [`MIN_PADDING`]) on every side.
/// Styled reserves a title strip at the top of the chart and, unless switched off, the mark's
/// place at its bottom-left.
pub fn layout(capture: Vec2, config: &StudioConfig) -> Composition {
    let (width, height) = (usable(capture.x), usable(capture.y));
    let framed = config.background.has_frame();

    let (size, padding) = match config.effective_size().terms() {
        Some(terms) => {
            let (w, h) = preset_size(terms, width * height, framed);
            let margin = if framed {
                (PADDING_SHARE * w.min(h)).max(MIN_PADDING)
            } else {
                0.0
            };
            ((w, h), margin)
        }
        None => {
            let margin = if framed {
                margin_around(width.min(height))
            } else {
                0.0
            };
            ((width + 2.0 * margin, height + 2.0 * margin), margin)
        }
    };

    let padding = padding as f32;
    let size = vec2(size.0 as f32, size.1 as f32);
    let chart = Rect::from_min_max(
        pos2(padding, padding),
        pos2(size.x - padding, size.y - padding),
    );
    let styled = config.style == Style::Styled;
    let header = styled.then(|| {
        let height = (chart.height() * HEADER_SHARE)
            .clamp(HEADER_HEIGHT.0, HEADER_HEIGHT.1)
            .min(chart.height() / 2.0);
        Rect::from_min_size(chart.min, vec2(chart.width(), height))
    });
    let branding = config
        .shows_branding()
        .then(|| header.and_then(|header| branding_rect(header, chart)))
        .flatten();
    Composition {
        size,
        chart,
        padding,
        header,
        branding,
    }
}

/// The mark's place in the title strip: at its right end, centred on the strip, so it never lies
/// over market data. This is the room the full mark may take, right-aligned; `None` where it would
/// reach into the logo's place. How much of it is drawn (in full, its icon alone or none) depends on
/// how wide the market's name is, which the composer measures: the name is never cut for the mark.
fn branding_rect(header: Rect, chart: Rect) -> Option<Rect> {
    let height = (chart.height() * BRANDING_SHARE)
        .clamp(BRANDING_HEIGHT.0, BRANDING_HEIGHT.1)
        .min(header.height() / 2.0);
    let width = height * BRANDING_ASPECT;
    let pad = header.height() * HEADER_PAD_SHARE;
    let rect = Rect::from_min_size(
        pos2(header.max.x - pad - width, header.center().y - height / 2.0),
        vec2(width, height),
    );
    // What the title block takes before its text: the margin, the logo and the gap after it.
    let before_title = header.height() * (HEADER_PAD_SHARE + HEADER_LOGO_SHARE + HEADER_GAP_SHARE);
    (rect.min.x - header.min.x >= before_title).then_some(rect)
}

/// The image's size in pixels at `scale`, or at a smaller one where the image would pass the
/// limits every snapshot has; `None` where even a quarter of the scale cannot hold it.
pub fn dimensions(composition: &Composition, scale: SnapshotScale) -> Option<SnapshotDimensions> {
    SnapshotDimensions::fit(composition.size.x, composition.size.y, scale)
}

/// `1234567` as `1,234,567`.
fn grouped(number: u32) -> String {
    let digits = number.to_string();
    let mut text = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }
    text
}

/// The footer's line: `Selected portion · Styled · 3,884 × 2,557 · PNG`.
pub fn summary(area: Area, style: Style, dimensions: &SnapshotDimensions) -> String {
    format!(
        "{} · {} · {} × {} · PNG",
        area.label(),
        style.label(),
        grouped(dimensions.width),
        grouped(dimensions.height),
    )
}
