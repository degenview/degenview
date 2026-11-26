//! The small picture on each card, drawn from what the item holds: a layout's grid, a watchlist's
//! markets, a script's lines.

use crate::model::tradingview::library::Layout;
use crate::ui::theme;
use eframe::egui::{self, Rect, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

const RADIUS: f32 = 6.0;
const CELL_GAP: f32 = 2.0;
/// The room a candle takes across a cell.
const CANDLE_STEP: f32 = 5.0;
const LOGO: f32 = 26.0;
/// The logos a watchlist's picture stacks.
const LOGOS: usize = 4;
const LINE_STEP: f32 = 4.5;

/// The framed field a picture sits on.
fn well(ui: &Ui, rect: Rect) {
    ui.painter()
        .rect_filled(rect, RADIUS, theme::quaternary(0.5));
    ui.painter().rect_stroke(
        rect,
        RADIUS,
        Stroke::new(1.0, theme::card_border(false)),
        StrokeKind::Inside,
    );
}

/// The layout's own arrangement of charts, each with a few candles.
pub(super) fn grid(ui: &Ui, rect: Rect, layout: &Layout) {
    well(ui, rect);
    let inner = rect.shrink(4.0);
    let columns = &layout.grid.columns;
    let across = columns.len().max(1) as f32;
    let width = (inner.width() - CELL_GAP * (across - 1.0)) / across;
    let mut chart = 0;
    for (column, cells) in columns.iter().enumerate() {
        let down = cells.len().max(1) as f32;
        let height = (inner.height() - CELL_GAP * (down - 1.0)) / down;
        for row in 0..cells.len() {
            let cell = Rect::from_min_size(
                inner.min
                    + vec2(
                        column as f32 * (width + CELL_GAP),
                        row as f32 * (height + CELL_GAP),
                    ),
                vec2(width, height),
            );
            // Each chart a darker pane, so the arrangement reads.
            ui.painter().rect_filled(cell, 2.0, theme::chrome());
            candles(ui, cell.shrink(1.0), chart);
            chart += 1;
        }
    }
}

/// A run of candles filling `cell`; `seed` tells one chart's run from the next.
fn candles(ui: &Ui, cell: Rect, seed: usize) {
    const LEVELS: [f32; 9] = [0.70, 0.55, 0.63, 0.40, 0.50, 0.30, 0.42, 0.22, 0.34];
    let count = (cell.width() / CANDLE_STEP).floor() as usize;
    if count == 0 || cell.height() < 4.0 {
        return;
    }
    let step = cell.width() / count as f32;
    let level = |index: usize| LEVELS[(index + seed * 3) % LEVELS.len()];
    for index in 0..count {
        let (open, close) = (level(index), level(index + 1));
        let color = if close <= open {
            theme::UP
        } else {
            theme::DOWN
        };
        let x = cell.left() + step * (index as f32 + 0.5);
        let y = |level: f32| cell.top() + cell.height() * level;
        let (top, bottom) = (
            y(open.min(close)),
            y(open.max(close).max(open.min(close) + 0.08)),
        );
        ui.painter().vline(
            x,
            (top - cell.height() * 0.1)..=(bottom + cell.height() * 0.1),
            Stroke::new(0.75, color),
        );
        ui.painter().rect_filled(
            Rect::from_min_max(pos2(x - 1.25, top), pos2(x + 1.25, bottom)),
            0.5,
            color,
        );
    }
}

/// The first of `count` markets' logos, overlapping, the first on top.
pub(super) fn logos(
    ui: &mut Ui,
    rect: Rect,
    count: usize,
    mut logo: impl FnMut(&mut Ui, Rect, usize),
) {
    let shown = count.min(LOGOS);
    let step = (rect.width() - LOGO) / (LOGOS - 1) as f32;
    let across = LOGO + step * shown.saturating_sub(1) as f32;
    let left = rect.center().x - across / 2.0;
    for index in (0..shown).rev() {
        let at = Rect::from_min_size(
            pos2(left + step * index as f32, rect.center().y - LOGO / 2.0),
            Vec2::splat(LOGO),
        );
        // A ring of the sheet's colour sets it off the logo under it.
        ui.painter()
            .circle_filled(at.center(), LOGO / 2.0 + 1.5, theme::chrome());
        logo(ui, at, index);
    }
}

/// The script's first lines in outline: where each starts and how long it runs.
pub(super) fn source(ui: &Ui, rect: Rect, source: &str) {
    well(ui, rect);
    let inner = rect.shrink2(vec2(7.0, 6.0));
    let rows = (inner.height() / LINE_STEP).floor() as usize;
    let lines = source.lines().filter(|line| !line.trim().is_empty());
    for (row, line) in lines.take(rows).enumerate() {
        let text = line.trim_start();
        let indent = (line.len() - text.len()) as f32 * 1.5;
        let left = inner.left() + indent.min(inner.width() / 2.0);
        // Comments are faint, as an editor shows them.
        let color = if text.starts_with("//") {
            theme::text_tertiary()
        } else {
            theme::text_secondary()
        };
        let length = (text.chars().count() as f32 * 1.2).min(inner.right() - left);
        ui.painter().rect_filled(
            Rect::from_min_size(
                pos2(left, inner.top() + row as f32 * LINE_STEP + 1.0),
                egui::vec2(length, 2.0),
            ),
            1.0,
            color,
        );
    }
}
