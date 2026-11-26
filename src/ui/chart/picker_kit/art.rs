//! The picture at the head of the portfolio's asset picker and the portfolio sheets: one small
//! scene per kind of market, drawn like the Settings pages' pictures (`settings_art`). Painter
//! code only.

use crate::ui::icons::Icon;
use crate::ui::kit::art::{Art, ghost};
use crate::ui::{kit, theme};
use eframe::egui::{Color32, Pos2, Rect, Shape, Stroke, Ui, pos2, vec2};

pub use crate::ui::kit::art::SIZE;

/// What the sheet is for right now.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scene {
    Crypto,
    Stocks,
    Predictions,
    CoinMarketCap,
    Portfolio,
}

impl Scene {
    #[cfg(test)]
    pub const ALL: [Self; 5] = [
        Self::Crypto,
        Self::Stocks,
        Self::Predictions,
        Self::CoinMarketCap,
        Self::Portfolio,
    ];

    fn tint(self) -> Color32 {
        match self {
            Self::Crypto => Color32::from_rgb(247, 147, 26),
            Self::Stocks => Color32::from_rgb(10, 132, 255),
            Self::Predictions => Color32::from_rgb(191, 90, 242),
            Self::CoinMarketCap => Color32::from_rgb(64, 156, 255),
            Self::Portfolio => Color32::from_rgb(48, 209, 88),
        }
    }
}

/// Draws `scene` in `rect` (a [`SIZE`] box).
pub fn paint(ui: &Ui, rect: Rect, scene: Scene) {
    let art = Art {
        ui,
        painter: ui.painter(),
        tint: scene.tint(),
        centre: rect.center(),
        flat: false,
    };
    match scene {
        Scene::Crypto => art.crypto(),
        Scene::Stocks => art.stocks(),
        Scene::Predictions => art.predictions(),
        Scene::CoinMarketCap => art.index(),
        Scene::Portfolio => art.portfolio(),
    }
}

impl Art<'_> {
    fn sparkline(&self, area: Rect, levels: &[f32], color: Color32) {
        let curve: Vec<Pos2> = levels
            .iter()
            .enumerate()
            .map(|(index, level)| {
                pos2(
                    area.left() + area.width() * index as f32 / (levels.len() - 1) as f32,
                    area.top() + area.height() * level,
                )
            })
            .collect();
        let mut fill = curve.clone();
        fill.push(pos2(area.right(), area.bottom()));
        fill.push(pos2(area.left(), area.bottom()));
        self.painter.add(Shape::convex_polygon(
            fill,
            color.gamma_multiply(0.15),
            Stroke::NONE,
        ));
        self.painter
            .add(Shape::line(curve, Stroke::new(2.0, color)));
    }

    /// A coin over a rising line.
    fn crypto(&self) {
        let card = self.scene();
        self.sparkline(
            card.shrink2(vec2(9.0, 11.0)),
            &[0.9, 0.75, 0.82, 0.55, 0.62, 0.35, 0.2],
            kit::GREEN,
        );
        let coin = card.left_top() + vec2(2.0, 1.0);
        self.painter.circle_filled(coin, 10.0, self.tint);
        self.painter.circle_stroke(
            coin,
            7.4,
            Stroke::new(1.2, Color32::WHITE.gamma_multiply(0.6)),
        );
        self.glyph(Icon::BitcoinCircle, coin, 11.0, Color32::WHITE);
    }

    /// Columns of bars, as a stock's volume under its price.
    fn stocks(&self) {
        let card = self.scene();
        self.sparkline(
            card.shrink2(vec2(9.0, 9.0))
                .with_max_y(card.bottom() - 20.0),
            &[0.8, 0.7, 0.75, 0.5, 0.55, 0.3, 0.38],
            self.tint,
        );
        for (index, (height, up)) in [
            (6.0, true),
            (9.0, false),
            (5.0, true),
            (12.0, true),
            (7.0, false),
            (10.0, true),
            (14.0, true),
        ]
        .into_iter()
        .enumerate()
        {
            let x = card.left() + 10.0 + index as f32 * 7.4;
            let color = if up { kit::GREEN } else { kit::RED };
            self.painter.rect_filled(
                Rect::from_min_max(
                    pos2(x, card.bottom() - 7.0 - height),
                    pos2(x + 4.6, card.bottom() - 7.0),
                ),
                1.2,
                color.gamma_multiply(0.8),
            );
        }
        let chip = Rect::from_center_size(card.right_top() + vec2(-3.0, 1.0), vec2(26.0, 16.0));
        self.chip(chip);
        self.glyph(Icon::BuildingColumns, chip.center(), 10.0, Color32::WHITE);
    }

    /// A market's odds as a split bar.
    fn predictions(&self) {
        let card = self.scene();
        for (index, (share, color)) in [(0.64, kit::GREEN), (0.36, kit::RED)]
            .into_iter()
            .enumerate()
        {
            let row = Rect::from_min_size(
                card.left_top() + vec2(8.0, 12.0 + index as f32 * 17.0),
                vec2(card.width() - 16.0, 10.0),
            );
            self.painter.rect_filled(row, 5.0, theme::well_fill());
            self.painter.rect_filled(
                Rect::from_min_size(row.min, vec2(row.width() * share, row.height())),
                5.0,
                color.gamma_multiply(0.85),
            );
            self.line(
                row.left_center() + vec2(5.0, 0.0),
                8.0,
                Color32::WHITE.gamma_multiply(0.8),
            );
        }
        let chip = Rect::from_center_size(card.right_bottom() + vec2(-3.0, -1.0), vec2(26.0, 16.0));
        self.chip(chip);
        self.glyph(Icon::Percent, chip.center(), 9.5, Color32::WHITE);
    }

    /// An index between its bounds.
    fn index(&self) {
        let card = self.scene();
        let area = card.shrink2(vec2(9.0, 10.0));
        for level in [0.2, 0.8] {
            self.painter.extend(Shape::dashed_line(
                &[
                    pos2(area.left(), area.top() + area.height() * level),
                    pos2(area.right(), area.top() + area.height() * level),
                ],
                Stroke::new(1.0, ghost()),
                2.5,
                2.5,
            ));
        }
        self.sparkline(area, &[0.7, 0.55, 0.62, 0.4, 0.5, 0.28, 0.35], self.tint);
        let end = pos2(area.right(), area.top() + area.height() * 0.35);
        self.painter.circle_filled(end, 3.2, self.tint);
        self.painter
            .circle_stroke(end, 3.2, Stroke::new(1.4, theme::chrome()));
        let chip = Rect::from_center_size(card.left_top() + vec2(8.0, 2.0), vec2(28.0, 14.0));
        self.chip(chip);
        self.glyph(Icon::Gauge, chip.center(), 9.5, Color32::WHITE);
    }

    /// Holdings as a ring.
    fn portfolio(&self) {
        let card = self.scene();
        let centre = card.center() + vec2(-3.0, 0.0);
        let tau = std::f32::consts::TAU;
        let mut at = -std::f32::consts::FRAC_PI_2;
        for (share, color) in [
            (0.46, self.tint),
            (0.28, kit::ORANGE),
            (0.18, Color32::from_rgb(10, 132, 255)),
            (0.08, theme::text_tertiary()),
        ] {
            let sweep = tau * share;
            self.arc(
                centre,
                16.0,
                at + 0.05,
                at + sweep - 0.05,
                Stroke::new(7.0, color),
            );
            at += sweep;
        }
        let chip = Rect::from_center_size(card.right_bottom() + vec2(-3.0, -1.0), vec2(26.0, 16.0));
        self.chip(chip);
        self.glyph(Icon::Briefcase, chip.center(), 10.0, Color32::WHITE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::kit::art::test_support;

    #[test]
    fn every_scene_draws_a_picture_inside_its_box() {
        test_support::assert_all_inside(&Scene::ALL, paint);
    }

    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_the_picker_kit_pictures() {
        test_support::gallery("picker-kit-art", &Scene::ALL, paint);
    }
}
