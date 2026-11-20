//! The picture at the head of the update sheet: a release-notes card, each change led by a small
//! mark and followed by faint strokes for its text, with the orange up-arrow chip of the tab bar's
//! Update mark over its corner. Drawn like the Settings pages' pictures (`settings_art`), flat, with
//! painter code only, so it follows the theme.

use crate::ui::icons::Icon;
use crate::ui::kit::art::{Art, ghost};
use crate::ui::{kit, theme};
use eframe::egui::{Color32, Pos2, Rect, Shape, Stroke, Ui, pos2, vec2};

pub use crate::ui::kit::art::SIZE;

/// A new thing, drawn as a sparkle.
const SPARK: Color32 = Color32::from_rgb(255, 214, 10);

/// Draws the picture in `rect` (a [`SIZE`] box).
pub fn paint(ui: &Ui, rect: Rect) {
    let art = Art {
        ui,
        painter: ui.painter(),
        tint: kit::ORANGE,
        centre: rect.center(),
        flat: true,
    };
    art.release_notes();
}

impl Art<'_> {
    /// Two new things and a fix on one card; the chip says an update carries them.
    fn release_notes(&self) {
        let card = self.scene();
        let rows: [(f32, f32, f32); 3] = [(26.0, 18.0, 0.0), (34.0, 14.0, 1.0), (23.0, 20.0, 2.0)];
        for (length, detail, index) in rows {
            let y = card.top() + 13.0 + index * 14.0;
            let mark = pos2(card.left() + 10.0, y);
            match index as u32 {
                0 => self.star(mark, 4.4, SPARK),
                1 => self.plus(mark, 3.0, kit::GREEN),
                _ => self.tick(mark, 3.2, kit::GREEN),
            }
            let start = pos2(card.left() + 18.0, y - 2.4);
            self.line(start, length, theme::text_secondary().gamma_multiply(0.8));
            self.line(start + vec2(0.0, 5.4), detail, ghost());
        }
        let chip = Rect::from_center_size(card.right_top() + vec2(-2.0, 1.0), vec2(26.0, 16.0));
        self.chip(chip);
        self.glyph(Icon::ArrowUp, chip.center(), 10.0, Color32::WHITE);
        self.star(card.left_bottom() + vec2(-3.0, -1.0), 4.0, SPARK);
    }

    fn plus(&self, centre: Pos2, reach: f32, color: Color32) {
        let stroke = Stroke::new(1.8, color);
        self.painter
            .hline(centre.x - reach..=centre.x + reach, centre.y, stroke);
        self.painter
            .vline(centre.x, centre.y - reach..=centre.y + reach, stroke);
    }

    fn tick(&self, centre: Pos2, reach: f32, color: Color32) {
        self.painter.add(Shape::line(
            vec![
                centre + vec2(-reach, 0.2),
                centre + vec2(-reach * 0.3, reach * 0.8),
                centre + vec2(reach, -reach * 0.7),
            ],
            Stroke::new(1.8, color),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::kit::art::test_support;

    #[test]
    fn the_picture_stays_inside_its_box() {
        test_support::assert_all_inside(&[()], |ui, rect, ()| paint(ui, rect));
    }

    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_the_update_picture() {
        test_support::gallery("update-art", &[()], |ui, rect, ()| paint(ui, rect));
    }
}
