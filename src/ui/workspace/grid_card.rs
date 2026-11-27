//! The chart cards of the grid: square, flat panes that fill it edge to edge, told apart only
//! by hairline seams.

use crate::ui::theme;
use eframe::egui::{self, Color32, Margin, Rect, Stroke, StrokeKind, Ui, pos2};

/// Space between a card's edge and its text and controls. A plot takes none of it: it runs to
/// the card's edges, and what is drawn over it keeps this distance itself.
pub const PADDING: i8 = 7;

/// How strongly an outline's colour shows: enough to find the card, not to compete with it.
const OUTLINE_STRENGTH: f32 = 0.45;

/// Where a card sits among the columns, to know which of its edges another card or an empty
/// stretch of the grid lies across.
#[derive(Clone, Copy)]
pub struct Slot<'a> {
    /// Cards in each column, left to right, a previewed new column included.
    pub column_lengths: &'a [usize],
    pub column: usize,
    pub row: usize,
}

#[derive(Debug, Default, Eq, PartialEq)]
struct Seams {
    left: bool,
    right: bool,
    bottom: bool,
}

impl Slot<'_> {
    /// Each card draws the seam to its right and below it, so none is drawn twice and none
    /// lies on the grid's own edge. A card beside a shorter column draws that seam too, as
    /// there is no card over there to do it.
    fn seams(self) -> Seams {
        let rows = self.column_lengths.iter().copied().max().unwrap_or(0);
        Seams {
            left: self.column > 0 && self.column_lengths[self.column - 1] <= self.row,
            right: self.column + 1 < self.column_lengths.len(),
            bottom: self.row + 1 < rows,
        }
    }
}

/// Draws one card around `contents`, which gets the card's whole area. `outline` marks the card
/// out (focus, replay).
pub fn show<R>(
    ui: &mut Ui,
    slot: Slot<'_>,
    outline: Option<Color32>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    let drawn = egui::Frame::new().fill(theme::surface()).show(ui, contents);
    let card = drawn.response.rect;
    let painter = ui.painter();
    let seams = slot.seams();
    let seam = |from, to| painter.rect_filled(Rect::from_min_max(from, to), 0.0, theme::border());
    if seams.left {
        seam(card.left_top(), pos2(card.left() + 1.0, card.bottom()));
    }
    if seams.right {
        seam(pos2(card.right() - 1.0, card.top()), card.right_bottom());
    }
    if seams.bottom {
        seam(pos2(card.left(), card.bottom() - 1.0), card.right_bottom());
    }
    if let Some(color) = outline {
        let stroke = Stroke::new(1.0, color.gamma_multiply(OUTLINE_STRENGTH));
        painter.rect_stroke(card, 0.0, stroke, StrokeKind::Inside);
    }
    drawn.inner
}

/// Keeps a card's text and controls [`PADDING`] away from its edges.
pub fn padded<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .inner_margin(Margin::same(PADDING))
        .show(ui, contents)
        .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seams(column_lengths: &[usize], column: usize, row: usize) -> Seams {
        Slot {
            column_lengths,
            column,
            row,
        }
        .seams()
    }

    #[test]
    fn a_lone_card_has_no_seam_on_the_grid_edge() {
        assert_eq!(seams(&[1], 0, 0), Seams::default());
    }

    #[test]
    fn neighbours_share_one_seam() {
        let first = seams(&[2, 2], 0, 0);
        assert!(first.right && first.bottom && !first.left);
        let last = seams(&[2, 2], 1, 1);
        assert_eq!(last, Seams::default());
    }

    #[test]
    fn a_card_next_to_empty_space_closes_its_own_edges() {
        // The short column's only card ends above the grid's bottom.
        assert!(seams(&[2, 1], 1, 0).bottom);
        // The second card of a tall right column has nothing on its left.
        assert!(seams(&[1, 2], 1, 1).left);
        assert!(!seams(&[1, 2], 1, 0).left);
    }
}
