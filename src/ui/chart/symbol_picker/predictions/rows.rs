//! How prediction events are drawn in the list: a header per event, its choices under it with a
//! checkbox each, and the topic chips offered while the box is empty.

use super::{EventHead, Tick};
use crate::service::SearchResult;
use crate::ui::chart::picker_kit::artwork::prediction_icon;
use crate::ui::chart::picker_kit::suggestions::PREDICTION_TOPICS;
use crate::ui::chart::symbol_picker::list::{
    PAD, RowOutput, RowState, SUBTITLE_SIZE, TITLE_SIZE, chance_label, content_right, marks,
    paint_background, venue_block,
};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD};
use crate::ui::theme;
use eframe::egui::{
    self, Color32, Galley, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use std::sync::Arc;

const HEADER_HEIGHT: f32 = 56.0;
const CHOICE_HEIGHT: f32 = 36.0;
const CHOICE_PAD: f32 = 8.0;
/// Right-hand room a choice keeps free for its chance and the plus.
const CHOICE_RESERVED: f32 = 120.0;
const CHEVRON: f32 = 12.0;
const CHECKBOX: f32 = 14.0;
const HEADER_ART: f32 = 36.0;
/// Where the checkbox column starts, shared by headers and choices so the boxes line up.
const CHECK_LEFT: f32 = PAD + CHEVRON + 8.0;
/// How far a lone choice's artwork is pushed in to stand where an event's artwork does.
pub const ART_INDENT: f32 = ART_LEFT - PAD;
/// Where an event's artwork starts, and where its title (and its choices' labels) start.
const ART_LEFT: f32 = CHECK_LEFT + CHECKBOX + 10.0;
const TEXT_LEFT: f32 = ART_LEFT + HEADER_ART + 12.0;

/// An event's header: chevron, checkbox, artwork, title over its summary, and the exchange.
pub fn event_header(
    ui: &mut Ui,
    head: &EventHead,
    highlighted: bool,
    width: f32,
) -> (Response, RowOutput) {
    let (rect, response) = ui.allocate_exact_size(vec2(width, HEADER_HEIGHT), Sense::click());
    let label = format!("{}, {} choices", head.title, head.choices);
    let response = crate::ui::accessibility::label(response, label);
    let checkbox = Rect::from_center_size(
        pos2(rect.left() + CHECK_LEFT + CHECKBOX / 2.0, rect.center().y),
        Vec2::splat(CHECKBOX),
    );
    let tick = ui
        .interact(
            checkbox.expand(7.0),
            response.id.with("tick"),
            Sense::click(),
        )
        .on_hover_text("Chart every choice together")
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let out = RowOutput {
        hovered: response.hovered() || tick.hovered(),
        tick: tick.clicked(),
        fold: response.clicked() && !tick.clicked(),
        ..RowOutput::default()
    };
    if !ui.is_rect_visible(rect) {
        return (response, out);
    }
    paint_background(ui, rect, highlighted, response.hovered());
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            pos2(rect.left() + PAD + CHEVRON / 2.0, rect.center().y),
            Vec2::splat(CHEVRON),
        ),
        if head.open {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        },
        theme::text_secondary(),
    );
    paint_checkbox(ui.painter(), checkbox, head.tick);

    let art = Rect::from_center_size(
        pos2(rect.left() + ART_LEFT + HEADER_ART / 2.0, rect.center().y),
        Vec2::splat(HEADER_ART),
    );
    prediction_icon(ui, art, &head.title, head.image_url.as_deref());
    let venue_left = venue_block(ui, rect, content_right(rect), head.source());
    let text_left = rect.left() + TEXT_LEFT;
    let text_width = (venue_left - 16.0 - text_left).max(40.0);
    let title = kit::galley(
        ui,
        &head.title,
        TITLE_SIZE,
        SEMIBOLD,
        theme::text_primary(),
        text_width,
    );
    let summary = kit::galley(
        ui,
        &head.summary(),
        SUBTITLE_SIZE,
        REGULAR,
        theme::text_secondary(),
        text_width,
    );
    let top = rect.center().y - (title.size().y + 2.0 + summary.size().y) / 2.0;
    let summary_top = top + title.size().y + 2.0;
    ui.painter()
        .galley(pos2(text_left, top), title, theme::text_primary());
    ui.painter().galley(
        pos2(text_left, summary_top),
        summary,
        theme::text_secondary(),
    );
    (response, out)
}

/// One choice of an open event: its checkbox under the event's artwork, its label over up to two
/// lines under the event's title, and its chance. It has no artwork of its own.
pub fn choice(
    ui: &mut Ui,
    result: &SearchResult,
    ticked: bool,
    state: RowState,
    width: f32,
) -> (Response, RowOutput) {
    let text_left = TEXT_LEFT;
    // Room kept at the right for the chance and the plus.
    let label = kit::galley_rows(
        ui,
        &result.symbol,
        14.0,
        MEDIUM,
        theme::text_primary(),
        (width - text_left - CHOICE_RESERVED).max(40.0),
        2,
    );
    let height = (label.size().y + 2.0 * CHOICE_PAD).max(CHOICE_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let response = crate::ui::accessibility::label(response, result.symbol.clone());
    let checkbox = Rect::from_center_size(
        pos2(rect.left() + ART_LEFT + HEADER_ART / 2.0, rect.center().y),
        Vec2::splat(CHECKBOX),
    );
    let tick = ui
        .interact(
            checkbox.expand(7.0),
            response.id.with("tick"),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let mut out = RowOutput {
        hovered: response.hovered() || tick.hovered(),
        tick: tick.clicked(),
        opened_menu: response.secondary_clicked(),
        ..RowOutput::default()
    };
    if !ui.is_rect_visible(rect) {
        return (response, out);
    }
    paint_background(ui, rect, state.highlighted, response.hovered());
    paint_checkbox(
        ui.painter(),
        checkbox,
        if ticked { Tick::All } else { Tick::None },
    );
    let content_right = marks(ui, rect, result, state, &mut out);
    out.pick |= response.clicked() && !out.add && !tick.clicked();
    if let Some(chance) = result.price {
        chance_label(ui, rect, content_right, chance);
    }
    ui.painter().galley(
        pos2(
            rect.left() + text_left,
            rect.center().y - label.size().y / 2.0,
        ),
        label,
        theme::text_primary(),
    );
    (response, out)
}

/// A checkbox: an outline, a grey square with a tick, or one with a dash for some.
fn paint_checkbox(painter: &egui::Painter, rect: Rect, tick: Tick) {
    if tick == Tick::None {
        painter.rect_stroke(
            rect,
            3.0,
            Stroke::new(1.2, theme::with_alpha(theme::text_secondary(), 0.7)),
            StrokeKind::Inside,
        );
        return;
    }
    painter.rect_filled(rect, 3.0, checkbox_fill());
    let at = |x: f32, y: f32| rect.min + vec2(rect.width() * x, rect.height() * y);
    let stroke = Stroke::new(1.6, Color32::WHITE);
    if tick == Tick::All {
        painter.line_segment([at(0.24, 0.52), at(0.43, 0.72)], stroke);
        painter.line_segment([at(0.43, 0.72), at(0.78, 0.30)], stroke);
    } else {
        painter.line_segment([at(0.25, 0.5), at(0.75, 0.5)], stroke);
    }
}

/// The grey of a ticked checkbox, dark enough to keep its white tick legible in both appearances.
fn checkbox_fill() -> Color32 {
    if theme::is_light() {
        Color32::from_gray(118)
    } else {
        Color32::from_rgb(88, 98, 104)
    }
}

const CHIP_HEIGHT: f32 = 30.0;
const CHIP_GAP: f32 = 8.0;
/// The most lines of chips: more than that scroll sideways.
const CHIP_LINES: usize = 2;

/// The starting points offered while the box is empty, as pills on at most two lines. What does not
/// fit scrolls sideways. Returns the search the clicked one stands for.
pub fn topic_chips(ui: &mut Ui, width: f32) -> Option<&'static str> {
    let labels: Vec<_> = PREDICTION_TOPICS
        .iter()
        .map(|(title, _, _)| kit::galley(ui, title, 13.0, MEDIUM, theme::text_primary(), 200.0))
        .collect();
    let widths: Vec<f32> = labels
        .iter()
        .map(|label| 12.0 + 14.0 + 6.0 + label.size().x + 14.0)
        .collect();
    let lines = chip_lines(&widths, width);
    let content = lines
        .iter()
        .map(|line| line_width(&widths, line))
        .fold(0.0, f32::max);
    let height = lines.len() as f32 * CHIP_HEIGHT + lines.len().saturating_sub(1) as f32 * CHIP_GAP;
    let mut chosen = None;
    let mut draw = |ui: &mut Ui| {
        let (area, _) = ui.allocate_exact_size(vec2(content.max(width), height), Sense::hover());
        for (line, members) in lines.iter().enumerate() {
            let mut x = area.left();
            let y = area.top() + line as f32 * (CHIP_HEIGHT + CHIP_GAP);
            for &index in members {
                let rect = Rect::from_min_size(pos2(x, y), vec2(widths[index], CHIP_HEIGHT));
                if chip(ui, rect, index, &labels[index]) {
                    chosen = Some(PREDICTION_TOPICS[index].1);
                }
                x += widths[index] + CHIP_GAP;
            }
        }
    };
    if content > width {
        egui::ScrollArea::horizontal()
            .id_salt("prediction_topics")
            .auto_shrink([false, true])
            .show(ui, |ui| draw(ui));
    } else {
        draw(ui);
    }
    chosen
}

/// One pill: its icon and title. Returns whether it was clicked.
fn chip(ui: &mut Ui, rect: Rect, index: usize, label: &Arc<Galley>) -> bool {
    let (title, query, icon) = PREDICTION_TOPICS[index];
    let response = ui
        .interact(rect, Id::new(("prediction_topic", query)), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let response = crate::ui::accessibility::label(response, title);
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        CHIP_HEIGHT / 2.0,
        if response.hovered() {
            theme::ink(18)
        } else {
            theme::card_fill(false)
        },
    );
    painter.rect_stroke(
        rect,
        CHIP_HEIGHT / 2.0,
        Stroke::new(1.0, theme::card_border(false)),
        StrokeKind::Inside,
    );
    icons::paint(
        painter,
        Rect::from_center_size(
            pos2(rect.left() + 12.0 + 7.0, rect.center().y),
            Vec2::splat(14.0),
        ),
        icon,
        theme::text_secondary(),
    );
    painter.galley(
        pos2(
            rect.left() + 12.0 + 14.0 + 6.0,
            rect.center().y - label.size().y / 2.0,
        ),
        label.clone(),
        theme::text_primary(),
    );
    response.clicked()
}

/// How wide a line of the chips `line` is.
fn line_width(widths: &[f32], line: &[usize]) -> f32 {
    line.iter().map(|index| widths[*index]).sum::<f32>()
        + line.len().saturating_sub(1) as f32 * CHIP_GAP
}

/// The chips of each line, for chips `widths` wide in a line `width` wide: filled in order while
/// that takes at most [`CHIP_LINES`] lines, else split into [`CHIP_LINES`] lines of about equal
/// length.
fn chip_lines(widths: &[f32], width: f32) -> Vec<Vec<usize>> {
    let mut lines: Vec<Vec<usize>> = vec![Vec::new()];
    for (index, chip) in widths.iter().enumerate() {
        let used = lines.last().map_or(0.0, |line| line_width(widths, line));
        let needed = if used == 0.0 {
            *chip
        } else {
            used + CHIP_GAP + chip
        };
        if needed > width && used > 0.0 {
            lines.push(Vec::new());
        }
        if let Some(line) = lines.last_mut() {
            line.push(index);
        }
    }
    if lines.len() <= CHIP_LINES {
        return lines;
    }
    // Cut in order at the middle of the total width, so each line still reads left to right and
    // the sideways scroll is as short as it can be.
    let half = (widths.iter().sum::<f32>() + widths.len() as f32 * CHIP_GAP) / CHIP_LINES as f32;
    let mut split = vec![Vec::new(); CHIP_LINES];
    let mut used = 0.0;
    for (index, chip) in widths.iter().enumerate() {
        let line = ((used / half) as usize).min(CHIP_LINES - 1);
        split[line].push(index);
        used += chip + CHIP_GAP;
    }
    split
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips_wrap_in_order_while_two_lines_are_enough() {
        let widths = [100.0, 100.0, 100.0, 100.0];
        assert_eq!(chip_lines(&widths, 1000.0), vec![vec![0, 1, 2, 3]]);
        assert_eq!(chip_lines(&widths, 220.0), vec![vec![0, 1], vec![2, 3]]);
        // A chip wider than the line still gets a line of its own.
        assert_eq!(chip_lines(&[300.0, 50.0], 120.0), vec![vec![0], vec![1]]);
    }

    #[test]
    fn more_than_two_lines_become_two_balanced_lines_that_scroll() {
        let widths = [100.0; 6];
        let lines = chip_lines(&widths, 220.0);
        assert_eq!(lines, vec![vec![0, 1, 2], vec![3, 4, 5]]);
        assert!(line_width(&widths, &lines[0]) > 220.0, "so it scrolls");
        // Reading order is kept on every line.
        let uneven = [60.0, 200.0, 60.0, 60.0, 60.0, 60.0];
        let lines = chip_lines(&uneven, 200.0);
        assert_eq!(lines.concat(), (0..6).collect::<Vec<_>>());
        assert_eq!(lines.len(), 2);
    }
}
