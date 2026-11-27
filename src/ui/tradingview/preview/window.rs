//! A layout as it will open: its columns of charts, each with its market and how much is drawn on
//! it.

use crate::model::PersistedChart;
use crate::model::tradingview::plan::PlannedLayout;
use crate::ui::kit::{self, ticker_icon};
use crate::ui::theme;
use crate::ui::tradingview::{pictures, words};
use eframe::egui::{
    self, Color32, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2,
};

const PADDING: f32 = 6.0;
const GAP: f32 = 4.0;
const LOGO: f32 = 16.0;
/// Wider than a card's thumbnail draws them: there is room.
const CANDLE_STEP: f32 = 9.0;
/// A narrower chart shows its logo alone.
const NAMED_FROM: f32 = 96.0;
/// The drawings' line, as the welcome's picture draws one.
const AMBER: Color32 = Color32::from_rgb(255, 159, 10);

/// The height of one chart where the fullest column holds `rows` of them.
fn cell_height(rows: usize) -> f32 {
    match rows {
        0 | 1 => 108.0,
        2 => 68.0,
        _ => 44.0,
    }
}

pub(super) fn show(ui: &mut Ui, layout: &PlannedLayout) {
    let rows = layout
        .columns
        .iter()
        .map(|column| column.chart_ids.len())
        .max()
        .unwrap_or(1)
        .max(1);
    let height = cell_height(rows) * rows as f32 + GAP * (rows - 1) as f32 + PADDING * 2.0;
    let (frame, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    ui.painter().rect_filled(frame, 8.0, theme::quaternary(0.5));
    ui.painter().rect_stroke(
        frame,
        8.0,
        Stroke::new(1.0, theme::card_border(false)),
        StrokeKind::Inside,
    );
    let inner = frame.shrink(PADDING);
    let across = layout.columns.len().max(1) as f32;
    let width = (inner.width() - GAP * (across - 1.0)) / across;
    for (column, ids) in layout.columns.iter().enumerate() {
        let down = ids.chart_ids.len().max(1) as f32;
        let tall = (inner.height() - GAP * (down - 1.0)) / down;
        for (row, id) in ids.chart_ids.iter().enumerate() {
            let Some(chart) = layout.charts.iter().find(|chart| chart.id == *id) else {
                continue;
            };
            let cell = Rect::from_min_size(
                inner.min + vec2(column as f32 * (width + GAP), row as f32 * (tall + GAP)),
                vec2(width, tall),
            );
            pane(ui, cell, chart);
        }
    }
}

fn pane(ui: &mut Ui, cell: Rect, chart: &PersistedChart) {
    let name = chart
        .title
        .clone()
        .unwrap_or_else(|| chart.source.display_symbol(&chart.symbol));
    ui.painter().rect_filled(cell, 5.0, theme::chrome());
    let mut inside = ui.new_child(UiBuilder::new().max_rect(cell));
    inside.set_clip_rect(inside.clip_rect().intersect(cell));
    let response = ui.interact(
        cell,
        egui::Id::new(("tradingview-pane", chart.source, &chart.symbol, chart.id)),
        Sense::hover(),
    );
    response.on_hover_text(chart.source.display_name());

    if cell.width() < NAMED_FROM {
        ticker_icon(
            &inside,
            Rect::from_center_size(cell.center(), Vec2::splat(LOGO)),
            &chart.symbol,
            chart.source,
        );
        return;
    }
    let drawn = chart.drawings.count();
    // The chart itself, under its name.
    let plot = Rect::from_min_max(
        pos2(cell.left() + 10.0, cell.top() + 26.0),
        pos2(cell.right() - 10.0, cell.bottom() - 6.0),
    );
    if plot.height() >= 12.0 {
        let mut faint = inside.new_child(UiBuilder::new().max_rect(plot));
        faint.multiply_opacity(0.55);
        pictures::candles(&faint, plot, CANDLE_STEP, chart.id as usize);
        if drawn > 0 {
            faint.painter().line_segment(
                [
                    pos2(plot.left(), plot.bottom() - plot.height() * 0.1),
                    pos2(plot.right(), plot.top() + plot.height() * 0.25),
                ],
                Stroke::new(1.25, AMBER),
            );
        }
    }
    ticker_icon(
        &inside,
        Rect::from_center_size(
            pos2(cell.left() + 16.0, cell.top() + 14.0),
            Vec2::splat(LOGO),
        ),
        &chart.symbol,
        chart.source,
    );
    let count = (drawn > 0).then(|| {
        kit::galley(
            &inside,
            &words::drawings(drawn),
            kit::size::CAPTION,
            kit::REGULAR,
            theme::text_secondary(),
            cell.width() / 2.0,
        )
    });
    let taken = count.as_ref().map_or(0.0, |count| count.size().x + 12.0);
    let title = kit::galley(
        &inside,
        &name,
        kit::size::CAPTION + 0.5,
        kit::SEMIBOLD,
        theme::text_primary(),
        cell.width() - 36.0 - taken,
    );
    inside.painter().galley(
        pos2(cell.left() + 29.0, cell.top() + 14.0 - title.size().y / 2.0),
        title,
        Color32::PLACEHOLDER,
    );
    if let Some(count) = count {
        inside.painter().galley(
            pos2(
                cell.right() - 8.0 - count.size().x,
                cell.top() + 14.0 - count.size().y / 2.0,
            ),
            count,
            Color32::PLACEHOLDER,
        );
    }
}
