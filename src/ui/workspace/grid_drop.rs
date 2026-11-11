//! Drag-and-drop affordances for the chart grid, as the Swift app draws them. Nothing shows
//! until a chart is being dragged; then the cards, the gaps and the grid's right edge become
//! drop targets, each with its own cue: an accent line where the chart would land, and a strip
//! on the right that, held over for a moment, previews the new column.

use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, SEMIBOLD, size};
use crate::ui::theme;
use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, Shape, Stroke, Ui, pos2, vec2};
use std::time::{Duration, Instant};

/// Width of the right-edge strip while a chart is dragged (Swift `gridNewColumnDropWidth`).
pub const NEW_COLUMN_STRIP_WIDTH: f32 = 56.0;
/// How long the pointer must rest on the strip before the new column is previewed.
pub const PREVIEW_DELAY: Duration = Duration::from_millis(300);
/// How far a card's drop zone reaches into the gap around it.
const CARD_REACH: f32 = 4.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DropTarget {
    /// Insert before `chart` in `column`.
    Before {
        column: u64,
        chart: u64,
    },
    /// Append to the end of `column`.
    Append {
        column: u64,
    },
    NewColumn,
}

/// Where a drop would land for each place the pointer can be.
#[derive(Default)]
pub struct Zones {
    /// `(column, chart, card rect)` in grid order.
    pub cards: Vec<(u64, u64, Rect)>,
    /// `(column, column rect)`.
    pub columns: Vec<(u64, Rect)>,
    /// The right-edge strip (or, while previewed, the placeholder column).
    pub new_column: Option<Rect>,
}

impl Zones {
    /// The strip wins over the grid beneath it, a card over its column, and a column takes
    /// everything not on a card, the gaps around the cards included.
    pub fn target_at(&self, pos: Pos2) -> Option<DropTarget> {
        if self.new_column.is_some_and(|rect| rect.contains(pos)) {
            return Some(DropTarget::NewColumn);
        }
        if let Some((column, chart, _)) = self
            .cards
            .iter()
            .find(|(_, _, rect)| rect.expand2(vec2(0.0, CARD_REACH)).contains(pos))
        {
            return Some(DropTarget::Before {
                column: *column,
                chart: *chart,
            });
        }
        self.columns
            .iter()
            .find(|(_, rect)| rect.contains(pos))
            .map(|(column, _)| DropTarget::Append { column: *column })
    }
}

/// What the grid remembers across frames of one drag.
#[derive(Default)]
pub struct GridDrag {
    target: Option<DropTarget>,
    since: Option<Instant>,
    /// The chart whose new column is being previewed: the pointer has rested on the strip.
    previewed: Option<u64>,
}

impl GridDrag {
    pub fn previewed(&self) -> Option<u64> {
        self.previewed
    }

    /// Moves the hover to `target`. Resting on the new-column strip for [`PREVIEW_DELAY`]
    /// previews the column; leaving it ends the preview. Returns how long until the preview
    /// is due, to schedule a repaint.
    pub fn hover(
        &mut self,
        dragged: u64,
        target: Option<DropTarget>,
        now: Instant,
    ) -> Option<Duration> {
        if self.target != target {
            self.target = target;
            self.since = Some(now);
            if target != Some(DropTarget::NewColumn) {
                self.previewed = None;
            }
        }
        if target != Some(DropTarget::NewColumn) {
            return None;
        }
        let waited = self
            .since
            .map_or(Duration::ZERO, |since| now.duration_since(since));
        if waited >= PREVIEW_DELAY {
            self.previewed = Some(dragged);
            None
        } else {
            Some(PREVIEW_DELAY - waited)
        }
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// The 3pt accent line that marks where the chart would be inserted, inset 8pt from the
/// card's edges and centred on the seam.
pub fn insert_line_above(ui: &Ui, card: Rect) {
    let y = card.top() - 1.0;
    ui.painter().rect_filled(
        Rect::from_min_max(
            pos2(card.left() + 8.0, y - 1.5),
            pos2(card.right() - 8.0, y + 1.5),
        ),
        CornerRadius::same(2),
        kit::ACCENT,
    );
}

/// The same line along the bottom of a column, for an append.
pub fn insert_line_below(ui: &Ui, column: Rect) {
    let y = column.bottom() - 2.5;
    ui.painter().rect_filled(
        Rect::from_min_max(
            pos2(column.left() + 8.0, y - 1.5),
            pos2(column.right() - 8.0, y + 1.5),
        ),
        CornerRadius::same(2),
        kit::ACCENT,
    );
}

/// The right-edge strip: a faint accent wash with a leading rule, a plus and the columns glyph.
/// It deepens and its rule thickens while the pointer is over it.
pub fn new_column_strip(ui: &Ui, rect: Rect, active: bool) {
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        0.0,
        theme::with_alpha(kit::ACCENT, if active { 0.16 } else { 0.07 }),
    );
    let rule = if active { 3.0 } else { 1.0 };
    painter.rect_filled(
        Rect::from_min_size(rect.min, vec2(rule, rect.height())),
        0.0,
        theme::with_alpha(kit::ACCENT, 0.75),
    );
    let centre = rect.center();
    icons::paint(
        painter,
        Rect::from_center_size(pos2(centre.x, centre.y - 16.0), vec2(13.0, 13.0)),
        Icon::Plus,
        kit::ACCENT,
    );
    icons::paint(
        painter,
        Rect::from_center_size(pos2(centre.x, centre.y + 8.0), vec2(18.0, 18.0)),
        Icon::SplitColumns,
        kit::ACCENT,
    );
}

/// The new column's placeholder: a dashed accent card with "New column" in it.
pub fn new_column_placeholder(ui: &Ui, rect: Rect) {
    let painter = ui.painter();
    painter.rect_filled(rect, 10.0, theme::with_alpha(kit::ACCENT, 0.08));
    let stroke = Stroke::new(2.0, kit::ACCENT);
    painter.extend(Shape::dashed_line(
        &rounded_outline(rect.shrink(1.0), 9.0),
        stroke,
        7.0,
        5.0,
    ));
    // The line that stands for the card's header.
    painter.extend(Shape::dashed_line(
        &[
            pos2(rect.left() + 10.0, rect.top() + 5.0),
            pos2(
                (rect.right() - 10.0).max(rect.left() + 10.0),
                rect.top() + 5.0,
            ),
        ],
        stroke,
        7.0,
        5.0,
    ));
    let centre = rect.center();
    icons::paint(
        painter,
        Rect::from_center_size(pos2(centre.x, centre.y - 14.0), vec2(26.0, 26.0)),
        Icon::SplitColumns,
        kit::ACCENT,
    );
    let label = kit::galley(
        ui,
        "New column",
        size::HEADLINE,
        SEMIBOLD,
        kit::ACCENT,
        rect.width(),
    );
    painter.galley(
        pos2(centre.x - label.size().x / 2.0, centre.y + 12.0),
        label,
        kit::ACCENT,
    );
}

/// A closed path around `rect` with corners of `radius`, for dashing.
fn rounded_outline(rect: Rect, radius: f32) -> Vec<Pos2> {
    let radius = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let corners = [
        (pos2(rect.right() - radius, rect.top() + radius), -90.0_f32),
        (pos2(rect.right() - radius, rect.bottom() - radius), 0.0),
        (pos2(rect.left() + radius, rect.bottom() - radius), 90.0),
        (pos2(rect.left() + radius, rect.top() + radius), 180.0),
    ];
    let mut points = Vec::with_capacity(4 * 9 + 1);
    for (centre, start) in corners {
        for step in 0..=8 {
            let angle = (start + step as f32 * 90.0 / 8.0).to_radians();
            points.push(centre + vec2(angle.cos(), angle.sin()) * radius);
        }
    }
    points.push(points[0]);
    points
}

/// A small label that follows the pointer while a chart is dragged, so it is clear what is
/// being moved.
pub fn drag_label(ctx: &egui::Context, label: &str) {
    let Some(pos) = ctx.pointer_latest_pos() else {
        return;
    };
    egui::Area::new(egui::Id::new("chart-drag-label"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pos + vec2(14.0, 14.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(
                    kit::ACCENT.r(),
                    kit::ACCENT.g(),
                    kit::ACCENT.b(),
                    235,
                ))
                .corner_radius(10.0)
                .inner_margin(egui::Margin::symmetric(10, 4))
                .shadow(egui::Shadow {
                    offset: [0, 3],
                    blur: 10,
                    spread: 0,
                    color: Color32::from_black_alpha(80),
                })
                .show(ui, |ui| {
                    ui.label(kit::text(
                        label,
                        size::SUBHEADLINE + 1.0,
                        SEMIBOLD,
                        Color32::WHITE,
                    ));
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zones() -> Zones {
        Zones {
            cards: vec![
                (
                    1,
                    10,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 100.0)),
                ),
                (
                    1,
                    11,
                    Rect::from_min_max(pos2(0.0, 110.0), pos2(100.0, 210.0)),
                ),
            ],
            columns: vec![(1, Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 400.0)))],
            new_column: Some(Rect::from_min_max(pos2(300.0, 0.0), pos2(356.0, 400.0))),
        }
    }

    #[test]
    fn a_card_inserts_before_itself_and_the_rest_of_the_column_appends() {
        let zones = zones();
        assert_eq!(
            zones.target_at(pos2(50.0, 50.0)),
            Some(DropTarget::Before {
                column: 1,
                chart: 10
            })
        );
        assert_eq!(
            zones.target_at(pos2(50.0, 150.0)),
            Some(DropTarget::Before {
                column: 1,
                chart: 11
            })
        );
        // The gap just under a card still belongs to the card below it.
        assert_eq!(
            zones.target_at(pos2(50.0, 106.0)),
            Some(DropTarget::Before {
                column: 1,
                chart: 11
            })
        );
        assert_eq!(
            zones.target_at(pos2(50.0, 300.0)),
            Some(DropTarget::Append { column: 1 })
        );
        assert_eq!(zones.target_at(pos2(200.0, 50.0)), None);
    }

    #[test]
    fn the_strip_wins_over_the_grid_beneath_it() {
        let mut zones = zones();
        zones.columns[0].1 = Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 400.0));
        assert_eq!(
            zones.target_at(pos2(320.0, 100.0)),
            Some(DropTarget::NewColumn)
        );
        zones.new_column = None;
        assert_eq!(
            zones.target_at(pos2(320.0, 100.0)),
            Some(DropTarget::Append { column: 1 })
        );
    }

    #[test]
    fn resting_on_the_strip_previews_the_new_column_and_leaving_ends_it() {
        let mut drag = GridDrag::default();
        let start = Instant::now();
        assert_eq!(
            drag.hover(7, Some(DropTarget::NewColumn), start),
            Some(PREVIEW_DELAY)
        );
        assert_eq!(drag.previewed(), None);
        let later = start + Duration::from_millis(120);
        assert_eq!(
            drag.hover(7, Some(DropTarget::NewColumn), later),
            Some(PREVIEW_DELAY - Duration::from_millis(120))
        );
        assert_eq!(
            drag.hover(7, Some(DropTarget::NewColumn), start + PREVIEW_DELAY),
            None
        );
        assert_eq!(drag.previewed(), Some(7));
        // Moving to a card ends it, and coming back starts the wait over.
        drag.hover(
            7,
            Some(DropTarget::Append { column: 1 }),
            start + PREVIEW_DELAY,
        );
        assert_eq!(drag.previewed(), None);
        assert_eq!(
            drag.hover(7, Some(DropTarget::NewColumn), start + PREVIEW_DELAY),
            Some(PREVIEW_DELAY)
        );
        drag.reset();
        assert_eq!(drag.previewed(), None);
    }

    #[test]
    fn the_outline_is_closed_and_stays_inside_its_rect() {
        let rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(80.0, 60.0));
        let points = rounded_outline(rect, 9.0);
        assert_eq!(points.first(), points.last());
        assert!(
            points
                .iter()
                .all(|point| rect.expand(0.01).contains(*point))
        );
    }
}
