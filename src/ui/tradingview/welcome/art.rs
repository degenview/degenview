//! The moving picture at the head of the offer: a chart with its drawings in a TradingView
//! window, the things to bring over travelling across, and the same chart building up in a
//! DegenView window. Painter code and the app's own icon, so it follows the theme.

use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM, size};
use crate::ui::theme;
use eframe::egui::{
    self, Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Ui, Vec2, pos2,
    vec2,
};

pub const HEIGHT: f32 = 172.0;
/// The sheet's corner radius, inside its hairline.
const SHEET_RADIUS: u8 = 11;
const WINDOW: Vec2 = vec2(172.0, 112.0);
const TITLE_BAR: f32 = 24.0;
/// How far a window's middle is from the picture's.
const APART: f32 = 150.0;
/// One round: the things cross, the chart builds up and holds.
const ROUND: f64 = 4.8;
/// Between a window's edge and the ends of the parcels' path: more than half a parcel.
const CLEAR: f32 = 16.0;
const AMBER: Color32 = Color32::from_rgb(255, 159, 10);
/// The level across the chart, dark enough to be seen on a light window.
fn yellow() -> Color32 {
    theme::pick(
        Color32::from_rgb(255, 214, 10),
        Color32::from_rgb(196, 148, 0),
    )
}

/// Each candle as `(open, close, low, high)`, from the top of the chart (0) to its foot (1).
const CANDLES: [[f32; 4]; 11] = [
    [0.78, 0.66, 0.86, 0.60],
    [0.66, 0.72, 0.80, 0.58],
    [0.72, 0.56, 0.78, 0.50],
    [0.56, 0.62, 0.70, 0.48],
    [0.62, 0.44, 0.66, 0.38],
    [0.44, 0.52, 0.60, 0.40],
    [0.52, 0.36, 0.56, 0.30],
    [0.36, 0.42, 0.50, 0.28],
    [0.42, 0.26, 0.46, 0.18],
    [0.26, 0.32, 0.40, 0.20],
    [0.32, 0.14, 0.36, 0.08],
];

/// How much of the DegenView chart there is at `phase` (0 to 1) of a round: its candles, then
/// its drawings.
fn built(phase: f32) -> (f32, f32) {
    let ramp = |from: f32, to: f32| ((phase - from) / (to - from)).clamp(0.0, 1.0);
    (ramp(0.18, 0.55), ramp(0.52, 0.74))
}

/// Draws the picture across `rect`, which is [`HEIGHT`] tall, as it is `time` seconds in.
pub fn paint(ui: &Ui, rect: Rect, time: f64) {
    let painter = ui.painter().with_clip_rect(rect);
    let phase = (time.rem_euclid(ROUND) / ROUND) as f32;
    grid(&painter, rect);

    let middle = rect.center() + vec2(0.0, 6.0);
    let from = Rect::from_center_size(middle - vec2(APART, 0.0), WINDOW);
    let to = Rect::from_center_size(middle + vec2(APART, 0.0), WINDOW);

    path(&painter, from, to);

    window(ui, &painter, from, "TradingView", None);
    chart(&painter, body(from), 1.0, 1.0);

    window(ui, &painter, to, "DegenView", Some(app_icon()));
    let (candles, drawings) = built(phase);
    chart(&painter, body(to), candles, drawings);

    for (index, icon) in [Icon::BarChart, Icon::List, Icon::Code]
        .into_iter()
        .enumerate()
    {
        // One after the other, apart enough not to touch, all across before the chart is whole.
        let travel = ((phase - index as f32 * 0.18) / 0.36).clamp(0.0, 1.0);
        if travel > 0.0 && travel < 1.0 {
            parcel(&painter, along(from, to, travel), icon, fade(travel));
        }
    }
}

fn app_icon() -> egui::ImageSource<'static> {
    egui::include_image!("../../../../assets/brand/app_icon.png")
}

/// Faint chart paper behind everything, its top corners those of the sheet it heads.
fn grid(painter: &Painter, rect: Rect) {
    let corners = CornerRadius {
        nw: SHEET_RADIUS,
        ne: SHEET_RADIUS,
        sw: 0,
        se: 0,
    };
    painter.rect_filled(rect, corners, theme::card_fill(false));
    let stroke = Stroke::new(1.0, theme::ink(5));
    let mut x = rect.center().x % 34.0 + rect.left();
    while x < rect.right() {
        painter.vline(x, rect.y_range(), stroke);
        x += 34.0;
    }
    let mut y = rect.top() + 18.0;
    while y < rect.bottom() {
        painter.hline(rect.x_range(), y, stroke);
        y += 34.0;
    }
    painter.hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, theme::ink(18)),
    );
}

/// Where a parcel is `travel` (0 to 1) of the way: an arc from one window to the other.
fn along(from: Rect, to: Rect, travel: f32) -> Pos2 {
    let start = pos2(from.right() + CLEAR, from.center().y);
    let end = pos2(to.left() - CLEAR, to.center().y);
    let point = start + (end - start) * travel;
    point - vec2(0.0, (travel * std::f32::consts::PI).sin() * 34.0)
}

fn path(painter: &Painter, from: Rect, to: Rect) {
    let points: Vec<Pos2> = (0..=32)
        .map(|step| along(from, to, step as f32 / 32.0))
        .collect();
    painter.extend(Shape::dashed_line(
        &points,
        Stroke::new(1.4, theme::text_tertiary()),
        3.0,
        4.0,
    ));
}

/// Parcels appear as they leave and go as they land.
fn fade(travel: f32) -> f32 {
    (travel.min(1.0 - travel) / 0.12).min(1.0)
}

fn parcel(painter: &Painter, centre: Pos2, icon: Icon, opacity: f32) {
    let rect = Rect::from_center_size(centre, Vec2::splat(28.0));
    painter.rect_filled(rect, 8.0, theme::chrome().gamma_multiply(opacity));
    painter.rect_stroke(
        rect,
        8.0,
        Stroke::new(1.4, AMBER.gamma_multiply(opacity)),
        StrokeKind::Inside,
    );
    icons::paint(
        painter,
        Rect::from_center_size(centre, Vec2::splat(15.0)),
        icon,
        AMBER.gamma_multiply(opacity),
    );
}

/// The chart's room inside a window.
fn body(window: Rect) -> Rect {
    window
        .with_min_y(window.top() + TITLE_BAR)
        .shrink2(vec2(10.0, 9.0))
}

fn window(
    ui: &Ui,
    painter: &Painter,
    rect: Rect,
    name: &str,
    mark: Option<egui::ImageSource<'static>>,
) {
    painter.rect_filled(rect, 10.0, theme::chrome());
    painter.rect_stroke(
        rect,
        10.0,
        Stroke::new(1.0, theme::card_border(false)),
        StrokeKind::Inside,
    );
    let bar = rect.with_max_y(rect.top() + TITLE_BAR);
    painter.hline(
        rect.x_range().shrink(1.0),
        bar.bottom(),
        Stroke::new(1.0, theme::ink(16)),
    );
    let mut x = bar.left() + 9.0;
    match mark {
        Some(source) => {
            let icon = Rect::from_center_size(pos2(x + 7.0, bar.center().y), Vec2::splat(15.0));
            egui::Image::new(source)
                .fit_to_exact_size(icon.size())
                .corner_radius(3.5)
                .paint_at(ui, icon);
            x += 20.0;
        }
        None => {
            for dot in 0..3 {
                painter.circle_filled(
                    pos2(x + 3.0 + dot as f32 * 8.0, bar.center().y),
                    2.6,
                    theme::text_tertiary(),
                );
            }
            x += 28.0;
        }
    }
    // A product's name, the same in every language.
    let text = kit::galley_on(
        painter,
        name,
        size::CAPTION,
        MEDIUM,
        theme::text_secondary(),
        bar.right() - x,
    );
    painter.galley(
        pos2(x, bar.center().y - text.size().y / 2.0),
        text,
        theme::text_secondary(),
    );
}

/// The share `candles` of the candles, and the drawings as far as `drawings`.
fn chart(painter: &Painter, area: Rect, candles: f32, drawings: f32) {
    let step = area.width() / CANDLES.len() as f32;
    let at = |index: usize, level: f32| {
        pos2(
            area.left() + step * (index as f32 + 0.5),
            area.top() + area.height() * level,
        )
    };
    for (index, [open, close, low, high]) in CANDLES.into_iter().enumerate() {
        let shown = (candles * CANDLES.len() as f32 - index as f32).clamp(0.0, 1.0);
        if shown <= 0.0 {
            continue;
        }
        let color = if close < open { kit::GREEN } else { kit::RED }.gamma_multiply(shown);
        painter.line_segment([at(index, high), at(index, low)], Stroke::new(1.0, color));
        let x = at(index, open).x;
        painter.rect_filled(
            Rect::from_min_max(
                pos2(x - step * 0.32, at(index, open.min(close)).y),
                pos2(x + step * 0.32, at(index, open.max(close)).y),
            ),
            1.0,
            color,
        );
    }
    if drawings <= 0.0 {
        return;
    }
    // A trend line growing under the lows, and a level across.
    let start = at(0, 0.92);
    let end = start + (at(10, 0.40) - start) * drawings;
    painter.line_segment([start, end], Stroke::new(1.6, AMBER));
    for point in [start, end] {
        painter.circle_filled(point, 2.4, theme::chrome());
        painter.circle_stroke(point, 2.4, Stroke::new(1.2, AMBER));
    }
    let level = area.top() + area.height() * 0.28;
    painter.extend(Shape::dashed_line(
        &[
            pos2(area.left(), level),
            pos2(area.left() + area.width() * drawings, level),
        ],
        Stroke::new(1.2, yellow()),
        4.0,
        3.0,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_round_starts_empty_and_ends_whole() {
        assert_eq!(built(0.0), (0.0, 0.0));
        assert_eq!(built(1.0), (1.0, 1.0));
        let (candles, drawings) = built(0.5);
        assert!(candles > 0.5 && drawings == 0.0, "candles come first");
    }

    #[test]
    fn parcels_cross_from_one_window_to_the_other() {
        let from = Rect::from_center_size(pos2(0.0, 0.0), WINDOW);
        let to = Rect::from_center_size(pos2(2.0 * APART, 0.0), WINDOW);
        assert!(along(from, to, 0.0).x > from.right());
        assert!(along(from, to, 1.0).x < to.left());
        assert!(along(from, to, 0.5).y < from.center().y, "over an arc");
        assert_eq!((fade(0.0), fade(0.5), fade(1.0)), (0.0, 1.0, 0.0));
    }
}
