//! The Allocation card: a donut of the portfolio's split with its legend. The donut sweeps in,
//! hovering a slice (or its legend row) lifts it and dims the rest, and the centre reads the
//! total until a slice is hovered. The geometry is pure and tested; the painting follows it.

use super::{fitted, loading_pill, reveal};
use crate::model::portfolio::card::CardData;
use crate::ui::icons::Icon;
use crate::ui::kit;
use crate::ui::portfolio::charts::{Slice, arc, empty_message, slice_at, slices};
use crate::ui::theme;
use eframe::egui::{self, Align2, Color32, Id, Pos2, Rect, Shape, Stroke, Ui, Vec2};
use std::f32::consts::{PI, TAU};

const LEGEND_ROW: f32 = 26.0;
const MIN_DONUT: f32 = 76.0;
const MAX_DONUT: f32 = 230.0;
/// A gap between slices, in radians.
const SLICE_GAP: f32 = 0.028;

/// Where the donut and the legend go in a card.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Layout {
    pub donut: Rect,
    pub legend: Rect,
}

/// Donut beside its legend unless the card is taller than it is wide, then above it.
pub(super) fn layout(area: Rect) -> Layout {
    let pad = 12.0;
    let inner = area.shrink(pad);
    if inner.width() >= 210.0 && inner.width() >= inner.height() * 0.95 {
        let share = if inner.width() >= 330.0 { 0.5 } else { 0.42 };
        let size = inner
            .height()
            .clamp(MIN_DONUT, MAX_DONUT)
            .min(inner.width() * share);
        let gap = if inner.width() >= 330.0 { 28.0 } else { 16.0 };
        let legend_room = (inner.width() - size - gap).clamp(96.0, 280.0);
        let used = size + gap + legend_room;
        let left = inner.left() + (inner.width() - used).max(0.0) / 2.0;
        let donut = Rect::from_min_size(
            Pos2::new(left, inner.center().y - size / 2.0),
            Vec2::splat(size),
        );
        let legend = Rect::from_min_max(
            Pos2::new(donut.right() + gap, inner.top()),
            Pos2::new((left + used).min(inner.right()), inner.bottom()),
        );
        Layout { donut, legend }
    } else {
        let size = (inner.height() * 0.5)
            .clamp(MIN_DONUT, MAX_DONUT)
            .min(inner.width());
        let donut = Rect::from_min_size(
            Pos2::new(inner.center().x - size / 2.0, inner.top() + 4.0),
            Vec2::splat(size),
        );
        let legend = Rect::from_min_max(
            Pos2::new(inner.left(), donut.bottom() + 14.0),
            inner.right_bottom(),
        );
        Layout { donut, legend }
    }
}

/// How many legend rows fit `height`, keeping one row for "+N more" when some are cut.
pub(super) fn visible_rows(height: f32, total: usize) -> usize {
    let fit = (height / LEGEND_ROW).floor().max(0.0) as usize;
    if total <= fit {
        total
    } else {
        fit.saturating_sub(1)
    }
}

pub(super) fn paint(ui: &Ui, area: Rect, id: Id, data: &CardData, pointer: Option<Pos2>) {
    if data.loading {
        loading_pill(ui, area, "Loading market data…");
        return;
    }
    let slices = slices(&data.holdings);
    if slices.is_empty() {
        empty_message(
            ui,
            area,
            Icon::PieChart,
            "No Allocations",
            "Priced holdings appear here.",
        );
        return;
    }
    let ctx = ui.ctx();
    let layout = layout(area);
    let centre = layout.donut.center();
    let ring_width = (layout.donut.width() * 0.1).clamp(8.0, 20.0);
    let radius = layout.donut.width() / 2.0 - ring_width / 2.0 - 3.0;

    // Legend rows are placed first so a pointer over one lifts its slice too.
    let rows = visible_rows(layout.legend.height(), slices.len());
    let block = rows as f32 * LEGEND_ROW;
    // Beside the donut the legend centres on it; under it the legend starts right below.
    let beside = layout.legend.left() >= layout.donut.right();
    let used = block + if rows < slices.len() { LEGEND_ROW } else { 0.0 };
    let offset = if beside {
        ((layout.legend.height() - used) / 2.0).max(0.0)
    } else {
        0.0
    };
    let row_rect = |index: usize| {
        Rect::from_min_size(
            Pos2::new(
                layout.legend.left(),
                layout.legend.top() + offset + index as f32 * LEGEND_ROW,
            ),
            Vec2::new(layout.legend.width(), LEGEND_ROW),
        )
    };
    let hovered = pointer.and_then(|pointer| {
        slice_at(&slices, pointer - centre, radius, ring_width / 2.0 + 6.0)
            .or_else(|| (0..rows).find(|row| row_rect(*row).contains(pointer)))
    });

    let progress = reveal(ctx, id.with("sweep"), 0.7);
    let painter = ui.painter_at(area);
    let mut start = -PI / 2.0;
    let sweep_end = -PI / 2.0 + TAU * progress;
    for (index, slice) in slices.iter().enumerate() {
        let end = start + slice.share as f32 * TAU;
        let focus = ctx.animate_value_with_time(
            id.with(("focus", index)),
            if hovered == Some(index) { 1.0 } else { 0.0 },
            0.12,
        );
        let dim = ctx.animate_value_with_time(
            id.with(("dim", index)),
            if hovered.is_some() && hovered != Some(index) {
                0.3
            } else {
                1.0
            },
            0.12,
        );
        let gap = if slices.len() > 1 { SLICE_GAP } else { 0.0 };
        let visible_end = (end - gap).min(sweep_end);
        if visible_end > start {
            let points = arc(centre, radius, start + gap / 2.0, visible_end);
            if points.len() >= 2 {
                painter.add(Shape::line(
                    points,
                    Stroke::new(ring_width + 4.0 * focus, slice.color.gamma_multiply(dim)),
                ));
            }
        }
        start = end;
    }

    paint_centre(
        ui,
        &painter,
        layout.donut,
        ring_width,
        data,
        &slices,
        hovered,
    );

    // Legend: a dot and a name, a bar on a shared track when the legend is wide enough, and the
    // share in a column of its own so every track and every number lines up.
    let widest = slices
        .iter()
        .map(|slice| slice.share)
        .fold(f64::EPSILON, f64::max);
    let percent_width = kit::galley(
        ui,
        "100.0%",
        11.5,
        kit::MEDIUM,
        Color32::WHITE,
        f32::INFINITY,
    )
    .size()
    .x;
    let name_width = slices
        .iter()
        .take(rows)
        .map(|slice| {
            kit::galley(
                ui,
                &slice.label,
                12.0,
                kit::MEDIUM,
                Color32::WHITE,
                f32::INFINITY,
            )
            .size()
            .x
        })
        .fold(0.0, f32::max)
        .clamp(30.0, 92.0);
    for (index, slice) in slices.iter().take(rows).enumerate() {
        let row = row_rect(index);
        let active = hovered == Some(index);
        if active {
            painter.rect_filled(row.shrink2(Vec2::new(0.0, 1.0)), 6.0, theme::ink(10));
        }
        let fade = if hovered.is_none() || active {
            1.0
        } else {
            0.45
        };
        painter.circle_filled(
            Pos2::new(row.left() + 11.0, row.center().y),
            3.5,
            slice.color.gamma_multiply(fade),
        );
        let percent_right = row.right() - 8.0;
        let percent = kit::galley(
            ui,
            &data.formatter.percent(slice.share, 1),
            11.5,
            kit::MEDIUM,
            if active {
                theme::text_primary()
            } else {
                theme::text_secondary()
            }
            .gamma_multiply(fade),
            percent_width + 4.0,
        );
        painter.galley(
            Pos2::new(
                percent_right - percent.size().x,
                row.center().y - percent.size().y / 2.0,
            ),
            percent,
            Color32::PLACEHOLDER,
        );
        let label_left = row.left() + 24.0;
        let track_left = label_left + name_width + 12.0;
        let track_right = percent_right - percent_width - 12.0;
        let bar = track_right - track_left >= 36.0;
        let label = kit::galley(
            ui,
            &slice.label,
            12.0,
            kit::MEDIUM,
            theme::text_primary().gamma_multiply(fade),
            if bar {
                name_width + 2.0
            } else {
                (percent_right - percent_width - 8.0 - label_left).max(20.0)
            },
        );
        painter.galley(
            Pos2::new(label_left, row.center().y - label.size().y / 2.0),
            label,
            Color32::PLACEHOLDER,
        );
        if bar {
            let track = Rect::from_min_max(
                Pos2::new(track_left, row.center().y - 2.0),
                Pos2::new(track_right, row.center().y + 2.0),
            );
            painter.rect_filled(track, 2.0, theme::ink(9));
            let fill = Rect::from_min_size(
                track.min,
                Vec2::new(
                    (track.width() * (slice.share / widest) as f32 * progress).max(4.0),
                    4.0,
                ),
            );
            painter.rect_filled(fill, 2.0, slice.color.gamma_multiply(fade));
        }
    }
    if rows < slices.len() {
        let hidden = slices.len() - rows;
        let galley = kit::galley(
            ui,
            &format!("+{hidden} more"),
            11.0,
            kit::REGULAR,
            theme::text_tertiary(),
            layout.legend.width(),
        );
        painter.galley(
            Pos2::new(
                layout.legend.left() + 24.0,
                row_rect(rows).center().y - galley.size().y / 2.0,
            ),
            galley,
            Color32::PLACEHOLDER,
        );
    }
}

/// The donut's middle: the total and the asset count, or the hovered slice.
fn paint_centre(
    ui: &Ui,
    painter: &egui::Painter,
    donut: Rect,
    ring_width: f32,
    data: &CardData,
    slices: &[Slice],
    hovered: Option<usize>,
) {
    let inner = donut.width() - ring_width * 2.0 - 22.0;
    if inner < 40.0 {
        return;
    }
    let centre = donut.center();
    let lines: Vec<(String, f32, f32, Color32)> = match hovered.and_then(|index| slices.get(index))
    {
        Some(slice) => vec![
            (slice.label.clone(), 11.0, kit::SEMIBOLD, slice.color),
            (
                data.formatter.percent(slice.share, 1),
                18.0,
                kit::SEMIBOLD,
                theme::text_primary(),
            ),
            (
                slice
                    .value
                    .map_or_else(|| "—".to_owned(), |value| data.formatter.money(value)),
                10.5,
                kit::REGULAR,
                theme::text_secondary(),
            ),
        ],
        None => vec![
            (
                data.formatter.money(data.total_value),
                16.0,
                kit::SEMIBOLD,
                theme::text_primary(),
            ),
            (
                format!(
                    "{} {}",
                    data.holdings.len(),
                    if data.holdings.len() == 1 {
                        "asset"
                    } else {
                        "assets"
                    }
                ),
                10.5,
                kit::REGULAR,
                theme::text_secondary(),
            ),
        ],
    };
    let galleys: Vec<_> = lines
        .iter()
        .map(|(text, size, weight, color)| fitted(ui, text, *size, 9.0, *weight, *color, inner))
        .collect();
    let total: f32 =
        galleys.iter().map(|g| g.size().y).sum::<f32>() + 2.0 * (galleys.len() - 1) as f32;
    let mut y = centre.y - total / 2.0;
    for galley in galleys {
        let height = galley.size().y;
        painter.galley(
            Pos2::new(centre.x - galley.size().x / 2.0, y),
            galley,
            Color32::PLACEHOLDER,
        );
        y += height + 2.0;
    }
    let _ = Align2::CENTER_CENTER;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(width: f32, height: f32) -> Rect {
        Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(width, height))
    }

    #[test]
    fn a_wide_card_puts_the_legend_beside_the_donut() {
        let layout = layout(area(520.0, 260.0));
        assert!(layout.legend.left() >= layout.donut.right());
        assert!(layout.donut.height() <= MAX_DONUT);
        assert!(area(520.0, 260.0).contains_rect(layout.donut));
    }

    #[test]
    fn a_small_card_keeps_its_legend_beside_a_smaller_donut() {
        let layout = layout(area(270.0, 160.0));
        assert!(layout.legend.left() >= layout.donut.right());
        assert!(layout.legend.width() >= 96.0);
        assert!(area(270.0, 160.0).contains_rect(layout.donut));
    }

    #[test]
    fn a_tall_card_puts_the_legend_under_the_donut() {
        let layout = layout(area(240.0, 420.0));
        assert!(layout.legend.top() >= layout.donut.bottom());
        assert!(area(240.0, 420.0).contains_rect(layout.donut));
        assert!(layout.donut.width() <= 240.0);
    }

    #[test]
    fn a_squat_card_keeps_a_readable_donut() {
        let layout = layout(area(400.0, 120.0));
        assert!(layout.donut.width() >= MIN_DONUT);
    }

    #[test]
    fn cut_legends_keep_a_row_for_the_remainder() {
        assert_eq!(visible_rows(240.0, 9), 9);
        assert_eq!(visible_rows(80.0, 9), 2);
        assert_eq!(visible_rows(0.0, 9), 0);
    }
}
