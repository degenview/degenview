//! The picture at the top of a blank tab: a chart waiting for its market, with the kinds of
//! market the search finds floating around it. Drawn in the style of the Settings pictures
//! (`settings_art`), larger because it is the page's one image. Painter code only.

use crate::ui::icons::Icon;
use crate::ui::kit::art::Art;
use crate::ui::{kit, theme};
use eframe::egui::{Color32, Rect, Shape, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

/// The room the picture takes.
pub const SIZE: Vec2 = vec2(176.0, 116.0);

const COIN: Color32 = Color32::from_rgb(247, 147, 26);
const STOCK: Color32 = Color32::from_rgb(10, 132, 255);
const ODDS: Color32 = Color32::from_rgb(191, 90, 242);

/// Draws the picture in `rect` (a [`SIZE`] box).
pub fn paint(ui: &Ui, rect: Rect) {
    let art = Art {
        ui,
        painter: ui.painter(),
        tint: theme::text_secondary(),
        centre: rect.center(),
        flat: false,
    };
    art.hero(rect);
}

impl Art<'_> {
    fn hero(&self, rect: Rect) {
        let painter = self.painter;
        let centre = rect.center() + vec2(0.0, -2.0);
        // A soft disc with a fainter ring around it.
        painter.circle_filled(centre, 54.0, theme::ink(7));
        painter.circle_stroke(centre, 54.0, Stroke::new(1.0, theme::ink(10)));
        painter.circle_stroke(centre, 59.0, Stroke::new(1.0, theme::ink(5)));

        // The chart.
        let card = Rect::from_center_size(centre, vec2(116.0, 76.0));
        self.card_at(card, theme::chrome());
        // Lifted a step off the page so the chart reads as a surface of its own.
        painter.rect_filled(card.shrink(1.5), 8.0, theme::ink(7));
        let area = card.shrink2(vec2(11.0, 10.0));
        for level in [0.25, 0.5, 0.75] {
            self.painter.extend(Shape::dashed_line(
                &[
                    pos2(area.left(), area.top() + area.height() * level),
                    pos2(area.right() - 18.0, area.top() + area.height() * level),
                ],
                Stroke::new(1.0, theme::ink(12)),
                2.5,
                3.0,
            ));
        }
        // Candles climbing to the right: (low, high of the body, wick above, wick below, up).
        let candles = [
            (0.18, 0.34, 0.06, 0.05, true),
            (0.26, 0.42, 0.05, 0.06, false),
            (0.34, 0.5, 0.06, 0.05, true),
            (0.46, 0.6, 0.07, 0.05, true),
            (0.42, 0.58, 0.05, 0.07, false),
            (0.56, 0.74, 0.07, 0.05, true),
            (0.68, 0.86, 0.06, 0.06, true),
        ];
        let step = (area.width() - 22.0) / (candles.len() - 1) as f32;
        let at = |level: f32| area.bottom() - area.height() * level;
        for (index, (low, high, above, below, up)) in candles.into_iter().enumerate() {
            let x = area.left() + 3.0 + index as f32 * step;
            let color = if up { kit::GREEN } else { kit::RED };
            painter.vline(
                x,
                at(high + above)..=at(low - below),
                Stroke::new(1.3, color.gamma_multiply(0.9)),
            );
            painter.rect_filled(
                Rect::from_min_max(pos2(x - 3.0, at(high)), pos2(x + 3.0, at(low))),
                1.6,
                color,
            );
        }
        // The last price, tagged at the right edge.
        let last = at(0.74);
        self.painter.extend(Shape::dashed_line(
            &[
                pos2(area.right() - 20.0, last),
                pos2(area.right() - 13.0, last),
            ],
            Stroke::new(1.2, kit::GREEN.gamma_multiply(0.8)),
            2.0,
            2.0,
        ));
        let tag = Rect::from_center_size(pos2(area.right() - 6.0, last), vec2(14.0, 8.0));
        painter.rect_filled(tag, 3.0, kit::GREEN);

        // What it can show, hovering around the card.
        self.tinted(COIN).market_chip(
            Rect::from_center_size(card.left_top() + vec2(-3.0, 3.0), vec2(30.0, 22.0)),
            Icon::BitcoinCircle,
        );
        self.tinted(STOCK).market_chip(
            Rect::from_center_size(card.right_top() + vec2(4.0, -3.0), vec2(30.0, 22.0)),
            Icon::BuildingColumns,
        );
        self.tinted(ODDS).market_chip(
            Rect::from_center_size(card.right_bottom() + vec2(-4.0, 2.0), vec2(30.0, 22.0)),
            Icon::Percent,
        );
        // The search that finds them.
        let lens = card.left_bottom() + vec2(3.0, 2.0);
        painter.circle_filled(lens, 13.0, theme::chrome());
        painter.circle_stroke(lens, 13.0, Stroke::new(1.5, theme::text_secondary()));
        self.glyph(Icon::Magnifier, lens, 13.0, theme::text_primary());
        self.star(
            rect.right_top() + vec2(-4.0, 44.0),
            5.5,
            theme::text_tertiary(),
        );
        self.star(
            rect.left_bottom() + vec2(14.0, -12.0),
            3.8,
            theme::text_tertiary(),
        );
    }

    /// A rounded chip in the picture's colour with a white glyph.
    fn market_chip(&self, rect: Rect, icon: Icon) {
        self.chip(rect);
        self.painter.rect_stroke(
            rect,
            rect.height() / 2.0,
            Stroke::new(1.5, theme::chrome()),
            StrokeKind::Outside,
        );
        self.glyph(icon, rect.center(), 13.0, Color32::WHITE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::kit::art::test_support;
    use crate::ui::theme::AppTheme;

    #[test]
    fn the_picture_draws_inside_its_box_in_both_themes() {
        let _theme = crate::ui::theme::RestoreDefaultTheme;
        for app_theme in [AppTheme::Dark, AppTheme::Light] {
            let (rect, bounds) = test_support::drawn_sized(app_theme, SIZE, paint);
            assert!(bounds.len() > 20, "{} shapes", bounds.len());
            for shape in bounds {
                assert!(
                    rect.expand(4.0).contains_rect(shape),
                    "{app_theme:?}: {shape:?} outside {rect:?}"
                );
            }
        }
    }
}
