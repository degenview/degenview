//! A price label with its zero count drawn as a real subscript.
//!
//! `PriceFormat` writes a tiny price as `0.0₅278` (Unicode subscript digits), which is what every
//! plain string and the clipboard get. The glyphs of that block are small and sit on the baseline,
//! so on the chart the count is laid out here instead: the same digits, larger, and lowered so
//! they hang a little under the line. A label without a count is one ordinary galley.

use eframe::egui::{Align2, Color32, FontId, Galley, Painter, Pos2, Rect, Vec2};
use std::sync::Arc;

/// The count's digits are this fraction of the label's font size.
const SUBSCRIPT_SCALE: f32 = 0.8;
/// How far the count hangs below the line, as a fraction of the font size.
const SUBSCRIPT_DROP: f32 = 0.24;

/// `text` with its run of subscript digits split out: what comes before, the digits as plain
/// ASCII, and what follows. `None` when the text has no such run.
fn split_count(text: &str) -> Option<(&str, String, &str)> {
    let is_subscript = |c: char| ('₀'..='₉').contains(&c);
    let start = text.find(is_subscript)?;
    let rest = &text[start..];
    let len = rest.find(|c| !is_subscript(c)).unwrap_or(rest.len());
    let digits = rest[..len]
        .chars()
        .filter_map(|c| char::from_digit(c as u32 - '₀' as u32, 10))
        .collect();
    Some((&text[..start], digits, &rest[len..]))
}

/// A laid-out label: measure it, then paint it anchored where a `Painter::text` call would.
pub struct PriceText {
    parts: Vec<Part>,
    size: Vec2,
}

struct Part {
    galley: Arc<Galley>,
    /// Left edge and top, from the label's own top-left.
    offset: Vec2,
}

impl PriceText {
    pub fn new(painter: &Painter, text: &str, font: FontId, color: Color32) -> Self {
        let Some((before, count, after)) = split_count(text) else {
            let galley = painter.layout_no_wrap(text.to_owned(), font, color);
            let size = galley.size();
            return Self {
                parts: vec![Part {
                    galley,
                    offset: Vec2::ZERO,
                }],
                size,
            };
        };
        let small = FontId::new(font.size * SUBSCRIPT_SCALE, font.family.clone());
        let lay = |text: &str, font: &FontId| {
            painter.layout_no_wrap(text.to_owned(), font.clone(), color)
        };
        let (before, count, after) = (lay(before, &font), lay(&count, &small), lay(after, &font));
        let height = before.size().y.max(after.size().y);
        let drop = font.size * SUBSCRIPT_DROP;
        let mut x = 0.0;
        let mut parts = Vec::with_capacity(3);
        for (index, galley) in [before, count, after].into_iter().enumerate() {
            // The count's bottom meets the line's, then it drops by `drop`.
            let y = if index == 1 {
                height - galley.size().y + drop
            } else {
                0.0
            };
            let width = galley.size().x;
            parts.push(Part {
                galley,
                offset: Vec2::new(x, y),
            });
            x += width;
        }
        Self {
            parts,
            size: Vec2::new(x, height),
        }
    }

    /// Width and height of the line, not counting what the count hangs below it.
    pub fn size(&self) -> Vec2 {
        self.size
    }

    /// Paints the label with `anchor` of its box at `pos`, like `Painter::text`.
    pub fn paint(&self, painter: &Painter, anchor: Align2, pos: Pos2) -> Rect {
        let rect = anchor.anchor_size(pos, self.size);
        for part in &self.parts {
            painter.galley(
                rect.min + part.offset,
                Arc::clone(&part.galley),
                Color32::WHITE,
            );
        }
        rect
    }
}

/// Paints `text` the way `Painter::text` would, with a zero count drawn as a subscript.
pub fn paint(
    painter: &Painter,
    pos: Pos2,
    anchor: Align2,
    text: &str,
    font: FontId,
    color: Color32,
) -> Rect {
    PriceText::new(painter, text, font, color).paint(painter, anchor, pos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui;

    #[test]
    fn the_count_is_split_out_of_the_text() {
        assert_eq!(split_count("0.0₅278"), Some(("0.0", "5".to_owned(), "278")));
        assert_eq!(
            split_count("-0.0₁₁₉150"),
            Some(("-0.0", "119".to_owned(), "150"))
        );
        assert_eq!(split_count("67,432.19"), None);
        assert_eq!(split_count(""), None);
    }

    /// Runs `body` with a painter inside a frame, so the fonts exist.
    fn with_painter(body: impl FnOnce(&Painter)) {
        let ctx = egui::Context::default();
        let mut body = Some(body);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            if let Some(body) = body.take() {
                body(ui.painter());
            }
        });
        output.textures_delta.clear();
    }

    #[test]
    fn a_label_with_a_count_is_one_line_with_the_count_hanging_below() {
        with_painter(|painter| {
            let font = FontId::monospace(12.0);
            let plain = PriceText::new(painter, "0.0x278", font.clone(), Color32::WHITE);
            let counted = PriceText::new(painter, "0.0₅278", font, Color32::WHITE);
            assert_eq!(counted.parts.len(), 3);
            assert_eq!(plain.parts.len(), 1);
            // The count is smaller than the line but ends lower than the line does.
            let count = &counted.parts[1];
            assert!(count.galley.size().y < counted.size().y);
            assert!(count.offset.y + count.galley.size().y > counted.size().y);
            // The pieces run on without gaps or overlaps.
            let widths: f32 = counted.parts.iter().map(|part| part.galley.size().x).sum();
            assert!((widths - counted.size().x).abs() < 0.01);
        });
    }

    #[test]
    fn a_plain_label_measures_like_the_painter_does() {
        with_painter(|painter| {
            let font = FontId::monospace(12.0);
            let label = PriceText::new(painter, "67,432", font.clone(), Color32::WHITE);
            let galley = painter.layout_no_wrap("67,432".to_owned(), font, Color32::WHITE);
            assert_eq!(label.size(), galley.size());
        });
    }
}
