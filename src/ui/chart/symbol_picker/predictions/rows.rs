//! How prediction events are drawn in the list: a header per event, its choices under it with a
//! checkbox each, and the topic chips offered while the box is empty.

use super::{EventHead, Tick};
use crate::service::SearchResult;
use crate::ui::chart::picker_kit::artwork::prediction_icon;
use crate::ui::chart::picker_kit::suggestions::PREDICTION_TOPICS;
use crate::ui::chart::symbol_picker::list::{
    PAD, RowOutput, RowState, SUBTITLE_SIZE, TITLE_SIZE, chance_label, marks, paint_background,
    venue_block,
};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD};
use crate::ui::theme;
use eframe::egui::{
    self, Color32, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};

const HEADER_HEIGHT: f32 = 56.0;
const CHOICE_HEIGHT: f32 = 46.0;
const CHEVRON: f32 = 12.0;
const CHECKBOX: f32 = 14.0;
const HEADER_ART: f32 = 36.0;
const CHOICE_ART: f32 = 28.0;
/// Where the checkbox column starts, shared by headers and choices so the boxes line up.
const CHECK_LEFT: f32 = PAD + CHEVRON + 8.0;
/// How far a lone choice's artwork is pushed in to stand where an event's artwork does.
pub const ART_INDENT: f32 = CHECK_LEFT + CHECKBOX + 10.0 - PAD;

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
        pos2(checkbox.right() + 10.0 + HEADER_ART / 2.0, rect.center().y),
        Vec2::splat(HEADER_ART),
    );
    prediction_icon(ui, art, &head.title, head.image_url.as_deref());
    let venue_left = venue_block(ui, rect, rect.right() - PAD, head.source());
    let text_left = art.right() + 12.0;
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

/// One choice of an open event: checkbox, artwork, its label over up to two lines, and its chance.
pub fn choice(
    ui: &mut Ui,
    result: &SearchResult,
    ticked: bool,
    state: RowState,
    width: f32,
) -> (Response, RowOutput) {
    let (rect, response) = ui.allocate_exact_size(vec2(width, CHOICE_HEIGHT), Sense::click());
    let response = crate::ui::accessibility::label(response, result.symbol.clone());
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

    let art = Rect::from_center_size(
        pos2(checkbox.right() + 10.0 + CHOICE_ART / 2.0, rect.center().y),
        Vec2::splat(CHOICE_ART),
    );
    let image = result
        .event
        .as_ref()
        .and_then(|event| event.image_url.as_deref());
    prediction_icon(ui, art, &result.symbol, image);
    let mut right = content_right;
    if let Some(chance) = result.price {
        right = chance_label(ui, rect, right, chance) - 14.0;
    }
    let text_left = art.right() + 10.0;
    let label = crate::ui::chart::picker_kit::text::galley_rows(
        ui,
        &result.symbol,
        14.0,
        MEDIUM,
        theme::text_primary(),
        (right - text_left).max(40.0),
        2,
    );
    ui.painter().galley(
        pos2(text_left, rect.center().y - label.size().y / 2.0),
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

/// The starting points offered while the box is empty, as pills that wrap onto as many lines as
/// the width needs. Returns the search the clicked one stands for.
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
    let height = lines.len() as f32 * CHIP_HEIGHT + lines.len().saturating_sub(1) as f32 * CHIP_GAP;
    let (area, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let mut chosen = None;
    for (line, members) in lines.iter().enumerate() {
        let y = area.top() + line as f32 * (CHIP_HEIGHT + CHIP_GAP);
        let mut x = area.left();
        for &index in members {
            let rect = Rect::from_min_size(pos2(x, y), vec2(widths[index], CHIP_HEIGHT));
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
            let label = labels[index].clone();
            painter.galley(
                pos2(
                    rect.left() + 12.0 + 14.0 + 6.0,
                    rect.center().y - label.size().y / 2.0,
                ),
                label,
                theme::text_primary(),
            );
            if response.clicked() {
                chosen = Some(query);
            }
            x += widths[index] + CHIP_GAP;
        }
    }
    chosen
}

/// The chips of each line, first-fit in order, for chips `widths` wide in a line `width` wide.
fn chip_lines(widths: &[f32], width: f32) -> Vec<Vec<usize>> {
    let mut lines: Vec<Vec<usize>> = vec![Vec::new()];
    let mut used = 0.0;
    for (index, chip) in widths.iter().enumerate() {
        let needed = if used == 0.0 {
            *chip
        } else {
            used + CHIP_GAP + chip
        };
        if needed > width && used > 0.0 {
            lines.push(Vec::new());
            used = 0.0;
        }
        used = if used == 0.0 {
            *chip
        } else {
            used + CHIP_GAP + chip
        };
        if let Some(line) = lines.last_mut() {
            line.push(index);
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips_wrap_first_fit_and_never_leave_a_line_empty() {
        let widths = [100.0, 100.0, 100.0, 100.0];
        assert_eq!(chip_lines(&widths, 1000.0), vec![vec![0, 1, 2, 3]]);
        assert_eq!(chip_lines(&widths, 220.0), vec![vec![0, 1], vec![2, 3]]);
        // A chip wider than the line still gets a line of its own.
        assert_eq!(chip_lines(&[300.0, 50.0], 120.0), vec![vec![0], vec![1]]);
    }
}
