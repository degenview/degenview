//! The picture at the head of the Alerts window: one small scene per tab, drawn like the Settings
//! pages' pictures (`settings_art`). Painter code only, so it follows the theme.

use crate::ui::alert::center::AlertTab;
use crate::ui::art::{Art, ghost};
use crate::ui::icons::Icon;
use crate::ui::{kit, theme};
use eframe::egui::{Color32, Pos2, Rect, Shape, Stroke, Ui, pos2, vec2};

pub use crate::ui::art::SIZE;

/// The colour a tab's picture is drawn in.
pub fn tint(tab: AlertTab) -> Color32 {
    match tab {
        AlertTab::Active => Color32::from_rgb(255, 69, 58),
        AlertTab::Triggered => Color32::from_rgb(48, 209, 88),
        AlertTab::Paused => Color32::from_rgb(255, 159, 10),
        AlertTab::All => Color32::from_rgb(10, 132, 255),
        AlertTab::History => Color32::from_rgb(191, 90, 242),
        AlertTab::Scripts => Color32::from_rgb(94, 92, 230),
    }
}

/// Draws `tab`'s picture in `rect` (a [`SIZE`] box).
pub fn paint(ui: &Ui, rect: Rect, tab: AlertTab) {
    let art = Art {
        ui,
        painter: ui.painter(),
        tint: tint(tab),
        centre: rect.center(),
        flat: false,
    };
    match tab {
        AlertTab::Active => art.watching(false, false),
        AlertTab::Triggered => art.watching(true, false),
        AlertTab::Paused => art.watching(false, true),
        AlertTab::All => art.all(),
        AlertTab::History => art.history(),
        AlertTab::Scripts => art.scripts(),
    }
}

impl Art<'_> {
    /// A price line with the level an alert waits on. Triggered: the line has crossed it and a
    /// check is set. Paused: the line is faded and the alert is a pause mark.
    fn watching(&self, crossed: bool, paused: bool) {
        let card = self.scene();
        let area = card.shrink2(vec2(9.0, 10.0));
        let level = area.top() + area.height() * 0.38;
        let path: &[f32] = if crossed {
            &[0.95, 0.8, 0.88, 0.62, 0.5, 0.2, 0.28]
        } else {
            &[0.95, 0.78, 0.86, 0.66, 0.74, 0.56, 0.62]
        };
        let curve: Vec<Pos2> = path
            .iter()
            .enumerate()
            .map(|(index, value)| {
                pos2(
                    area.left() + area.width() * index as f32 / (path.len() - 1) as f32,
                    area.top() + area.height() * value,
                )
            })
            .collect();
        let mut fill = curve.clone();
        fill.push(pos2(area.right(), area.bottom()));
        fill.push(pos2(area.left(), area.bottom()));
        let fade = if paused { 0.4 } else { 1.0 };
        self.painter.add(Shape::convex_polygon(
            fill,
            self.tint.gamma_multiply(0.12 * fade),
            Stroke::NONE,
        ));
        self.painter.extend(Shape::dashed_line(
            &[
                pos2(card.left() + 6.0, level),
                pos2(card.right() - 3.0, level),
            ],
            Stroke::new(
                1.5,
                self.tint.gamma_multiply(if paused { 0.5 } else { 0.95 }),
            ),
            3.0,
            2.5,
        ));
        self.painter.add(Shape::line(
            curve.clone(),
            Stroke::new(
                2.0,
                theme::text_secondary().gamma_multiply(if paused { 0.5 } else { 1.0 }),
            ),
        ));
        if crossed {
            // Where it crossed.
            let hit = curve[5];
            self.painter.circle_filled(hit, 3.6, self.tint);
            self.painter
                .circle_stroke(hit, 3.6, Stroke::new(1.4, theme::chrome()));
        }
        let badge = card.right_top() + vec2(-1.0, 1.0);
        self.painter.circle_filled(badge, 9.0, self.tint);
        let icon = if crossed {
            Icon::Check
        } else if paused {
            Icon::PauseFill
        } else {
            Icon::BellFill
        };
        self.glyph(icon, badge, 9.5, Color32::WHITE);
    }

    /// Alerts in a stack, each in the state it is in.
    fn all(&self) {
        let card = self.scene();
        for (index, (color, length)) in [
            (tint(AlertTab::Active), 20.0),
            (tint(AlertTab::Triggered), 15.0),
            (tint(AlertTab::Paused), 21.0),
        ]
        .into_iter()
        .enumerate()
        {
            let row = Rect::from_min_size(
                card.left_top() + vec2(7.0, 7.0 + index as f32 * 15.0),
                vec2(card.width() - 14.0, 12.0),
            );
            self.painter.rect_filled(row, 4.0, theme::well_fill());
            self.painter
                .circle_filled(row.left_center() + vec2(6.0, 0.0), 2.8, color);
            self.line(
                row.left_center() + vec2(14.0, -2.0),
                length,
                theme::text_secondary().gamma_multiply(0.8),
            );
            self.line(row.left_center() + vec2(14.0, 2.6), length * 0.6, ghost());
            let pill = Rect::from_center_size(row.right_center() - vec2(7.0, 0.0), vec2(9.0, 5.0));
            self.painter
                .rect_filled(pill, 2.5, color.gamma_multiply(0.85));
        }
        let chip = Rect::from_center_size(card.right_bottom() + vec2(-2.0, 0.0), vec2(22.0, 14.0));
        self.chip(chip);
        self.glyph(Icon::List, chip.center(), 9.0, Color32::WHITE);
    }

    /// A timeline of what fired, and a clock.
    fn history(&self) {
        let card = self.scene();
        let spine = card.left() + 15.0;
        self.painter.vline(
            spine,
            card.top() + 11.0..=card.bottom() - 11.0,
            Stroke::new(1.5, theme::ink(24)),
        );
        for (index, (color, length)) in [(kit::GREEN, 26.0), (self.tint, 20.0), (kit::GREEN, 30.0)]
            .into_iter()
            .enumerate()
        {
            let y = card.top() + 12.0 + index as f32 * 15.0;
            self.painter.circle_filled(pos2(spine, y), 3.4, color);
            self.painter
                .circle_stroke(pos2(spine, y), 3.4, Stroke::new(1.5, theme::chrome()));
            self.line(
                pos2(spine + 9.0, y - 2.2),
                length,
                theme::text_secondary().gamma_multiply(0.8),
            );
            self.line(pos2(spine + 9.0, y + 3.0), length * 0.55, ghost());
        }
        let clock = card.right_top() + vec2(-1.0, 1.0);
        self.painter.circle_filled(clock, 9.5, self.tint);
        self.glyph(Icon::ClockArrow, clock, 11.0, Color32::WHITE);
    }

    /// Code, and the bell it rings.
    fn scripts(&self) {
        let card = self.scene();
        let rows: [(f32, [(f32, Color32); 2]); 4] = [
            (
                0.0,
                [
                    (9.0, theme::syntax::keyword()),
                    (16.0, theme::syntax::function()),
                ],
            ),
            (
                1.0,
                [
                    (13.0, theme::syntax::variable()),
                    (8.0, theme::syntax::number()),
                ],
            ),
            (
                1.0,
                [
                    (10.0, theme::syntax::function()),
                    (14.0, theme::syntax::string()),
                ],
            ),
            (0.0, [(7.0, theme::syntax::keyword()), (12.0, ghost())]),
        ];
        for (index, (indent, parts)) in rows.into_iter().enumerate() {
            let y = card.top() + 11.0 + index as f32 * 10.0;
            let mut x = card.left() + 9.0 + indent * 6.0;
            for (length, color) in parts {
                self.line(pos2(x, y), length, color.gamma_multiply(0.9));
                x += length + 3.0;
            }
        }
        let chip = Rect::from_center_size(card.right_bottom() + vec2(-3.0, -1.0), vec2(26.0, 16.0));
        self.chip(chip);
        self.glyph(Icon::BellFill, chip.center(), 10.0, Color32::WHITE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::art::test_support;

    #[test]
    fn every_tab_draws_a_picture_inside_its_box() {
        test_support::assert_all_inside(&AlertTab::ALL, paint);
    }

    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_the_alert_pictures() {
        test_support::gallery("alert-art", &AlertTab::ALL, paint);
    }
}
