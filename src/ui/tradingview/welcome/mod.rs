//! The welcome to the import: a picture of what it does, what was found on this computer, and the
//! way in. It is the import's first page, and the offer made once at launch.

pub(super) mod art;
#[cfg(test)]
mod tests;

use super::words;
use crate::i18n::tr;
use crate::model::tradingview::library::Contents;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, CARD_RADIUS, MEDIUM, REGULAR, SEMIBOLD, loading, sheet, size};
use crate::ui::theme;
use eframe::egui::{
    self, Id, Key, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use std::time::Duration;

pub const WIDTH: f32 = 560.0;
/// How often the picture moves on.
const FRAME: Duration = Duration::from_millis(33);
const CARD_GAP: f32 = 10.0;
const CARD_PADDING: f32 = 14.0;
/// One or two cards stay card-shaped, in the middle, instead of stretching across.
const CARD_MAX_WIDTH: f32 = 240.0;
const CARD_ICON: f32 = 18.0;
/// A card's icon, title, detail and hairline, with the room around them and its names.
const CARD_CHROME: f32 = CARD_PADDING * 2.0 + CARD_ICON + 10.0 + 22.0 + 18.0 + 17.0 - 4.0;
const NAME_ROW: f32 = 18.0;
/// The names a card lists before it counts the rest.
const NAMES: usize = 3;
const WAITING_SIZE: f32 = 40.0;
const PROMISE_ICON: f32 = 14.0;
const PROMISE_GAP: f32 = 20.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Choice {
    Import,
    Dismiss,
}

/// What brought the welcome up.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Occasion {
    /// Nobody asked: the offer made at launch.
    Offer,
    /// Someone opened the import.
    Opened,
}

/// One kind of thing there is to bring over.
#[derive(Debug, PartialEq)]
struct Card<'a> {
    icon: Icon,
    /// How many there are.
    title: String,
    /// What they hold.
    detail: String,
    names: &'a [String],
}

fn cards(found: &Contents) -> Vec<Card<'_>> {
    [
        (
            Icon::BarChart,
            words::layouts(found.layouts.len()),
            words::drawings(found.drawings),
            &found.layouts,
        ),
        (
            Icon::List,
            words::watchlists(found.watchlists.len()),
            words::markets(found.markets),
            &found.watchlists,
        ),
        (
            Icon::Code,
            words::scripts(found.scripts.len()),
            words::lines(found.lines),
            &found.scripts,
        ),
    ]
    .into_iter()
    .filter(|(.., names)| !names.is_empty())
    .map(|(icon, title, detail, names)| Card {
        icon,
        title,
        detail,
        names,
    })
    .collect()
}

/// The names a card shows, and the line for those it leaves out.
fn listed(names: &[String]) -> (&[String], Option<String>) {
    if names.len() <= NAMES {
        return (names, None);
    }
    // The count takes a row, so one name more makes way for it.
    let shown = NAMES - 1;
    let rest = names.len() - shown;
    (&names[..shown], Some(tr!("and {count} more", count = rest)))
}

/// Draws the sheet until it is answered; `found` is `None` while TradingView's data is still being
/// read. A click beside it is not an answer: the offer appears unasked, and it appears once.
pub fn show(ctx: &egui::Context, found: Option<&Contents>, occasion: Occasion) -> Option<Choice> {
    let id = Id::new(("tradingview-welcome", occasion == Occasion::Offer));
    // No button has the focus until the keyboard moves it there, so a key pressed for something
    // else answers nothing.
    let ring = kit::focus::start_on(ctx, id, None);
    let mut choice = None;
    sheet::show(ctx, id, WIDTH, |ui| {
        let time = ui.input(|input| input.time);
        body(ui, found, occasion, time, ring, &mut choice);
    });
    if choice.is_none() && ctx.input(|input| input.key_pressed(Key::Escape)) {
        choice = Some(Choice::Dismiss);
    }
    choice
}

/// The sheet's inside, also what the gallery draws; `time` is where its picture is.
fn body(
    ui: &mut Ui,
    found: Option<&Contents>,
    occasion: Occasion,
    time: f64,
    ring: bool,
    choice: &mut Option<Choice>,
) {
    ui.spacing_mut().item_spacing = Vec2::ZERO;
    let (picture, _) = ui.allocate_exact_size(vec2(WIDTH, art::HEIGHT), Sense::hover());
    // Inside the sheet's rounded corners and hairline.
    art::paint(ui, picture.shrink(1.0), time);
    ui.ctx().request_repaint_after(FRAME);

    ui.add_space(20.0);
    sheet::inset(ui, |ui| {
        headline(
            ui,
            tr!("Bring your TradingView setup with you"),
            size::TITLE2 + 4.0,
            SEMIBOLD,
            theme::text_primary(),
        );
        ui.add_space(6.0);
        headline(
            ui,
            match (occasion, found) {
                (Occasion::Offer, _) => {
                    tr!("DegenView found it on this computer. You choose what comes over.")
                }
                (Occasion::Opened, Some(_)) => {
                    tr!("Layouts, drawings, watchlists and scripts the TradingView app has loaded.")
                }
                (Occasion::Opened, None) => tr!("Reading TradingView\u{2019}s data\u{2026}"),
            },
            size::BODY,
            REGULAR,
            theme::text_secondary(),
        );
        ui.add_space(18.0);
        match found {
            Some(found) => card_row(ui, &cards(found)),
            None => waiting(ui),
        }
        ui.add_space(18.0);
        promises(ui);
        if occasion == Occasion::Offer {
            ui.add_space(14.0);
            headline(
                ui,
                tr!("Asked only once. Later, choose Import from TradingView in the layout menu."),
                size::CAPTION,
                REGULAR,
                theme::text_tertiary(),
            );
        }
    });
    ui.add_space(16.0);
    sheet::footer(ui, |ui| {
        let ready = found.is_some();
        if button(ui, tr!("Choose What to Import"), true, ready, ring).clicked() {
            *choice = Some(Choice::Import);
        }
        let leave = match occasion {
            Occasion::Offer => tr!("No Thanks"),
            Occasion::Opened => tr!("Cancel"),
        };
        if button(ui, leave, false, true, ring).clicked() {
            *choice = Some(Choice::Dismiss);
        }
    });
}

/// Where the cards will be, as tall as the fullest can get, so the sheet does not grow much.
fn waiting(ui: &mut Ui) {
    let height = card_height(NAMES);
    let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    loading::LoadingIndicator::new()
        .size(WAITING_SIZE)
        .paint_at(
            ui,
            Rect::from_center_size(area.center(), Vec2::splat(WAITING_SIZE)),
        );
}

fn card_height(names: usize) -> f32 {
    CARD_CHROME + names as f32 * NAME_ROW
}

fn headline(ui: &mut Ui, text: &str, size: f32, weight: f32, color: egui::Color32) {
    ui.vertical_centered(|ui| {
        ui.add(
            egui::Label::new(kit::text(text, size, weight, color))
                .wrap()
                .halign(egui::Align::Center),
        );
    });
}

fn button(ui: &mut Ui, label: &str, prominent: bool, enabled: bool, ring: bool) -> Response {
    let response = kit::neutral_button(ui, label, prominent, enabled, false, 120.0);
    if ring && response.has_focus() {
        kit::focus::paint_ring(ui.painter(), response.rect, 8.0);
    }
    response
}

/// The cards side by side in the middle, each as tall as the fullest.
fn card_row(ui: &mut Ui, cards: &[Card<'_>]) {
    let count = cards.len().max(1) as f32;
    let width = ((ui.available_width() - CARD_GAP * (count - 1.0)) / count).min(CARD_MAX_WIDTH);
    let rows = cards
        .iter()
        .map(|card| card.names.len().min(NAMES))
        .max()
        .unwrap_or(0);
    let height = card_height(rows);
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let across = width * count + CARD_GAP * (count - 1.0);
    for (index, card) in cards.iter().enumerate() {
        let left = row.center().x - across / 2.0 + index as f32 * (width + CARD_GAP);
        let rect = Rect::from_min_size(pos2(left, row.top()), vec2(width, height));
        paint_card(ui, rect, card);
    }
}

fn paint_card(ui: &Ui, rect: Rect, card: &Card<'_>) {
    let painter = ui.painter();
    painter.rect_filled(rect, CARD_RADIUS, theme::card_fill(false));
    painter.rect_stroke(
        rect,
        CARD_RADIUS,
        Stroke::new(1.0, theme::card_border(false)),
        StrokeKind::Inside,
    );
    let inner = rect.shrink(CARD_PADDING);
    icons::paint(
        painter,
        Rect::from_min_size(inner.min, Vec2::splat(CARD_ICON)),
        card.icon,
        theme::text_secondary(),
    );
    let mut y = inner.top() + CARD_ICON + 10.0;
    let line = |y: f32, text: &str, size: f32, weight: f32, color: egui::Color32| {
        let galley = kit::galley_words(ui, text, size, weight, color, inner.width());
        painter.galley(pos2(inner.left(), y), galley, color);
    };
    line(
        y,
        &card.title,
        size::TITLE2,
        SEMIBOLD,
        theme::text_primary(),
    );
    y += 22.0;
    line(
        y,
        &card.detail,
        size::CAPTION,
        REGULAR,
        theme::text_secondary(),
    );
    y += 18.0;
    painter.hline(
        inner.x_range(),
        y + 6.0,
        Stroke::new(1.0, theme::card_border(false)),
    );
    y += 17.0;
    let (names, rest) = listed(card.names);
    for name in names {
        line(y, name, size::CAPTION + 0.5, MEDIUM, theme::text_primary());
        y += NAME_ROW;
    }
    if let Some(rest) = rest {
        line(y, &rest, size::CAPTION, REGULAR, theme::text_tertiary());
    }
}

/// What the import will not do to anything: in a row, or one under the other where the words
/// need it.
fn promises(ui: &mut Ui) {
    let promises = [
        (Icon::Eye, tr!("Preview first")),
        (Icon::Shield, tr!("TradingView unchanged")),
        (Icon::CheckCircle, tr!("Nothing replaced")),
    ];
    let color = theme::text_secondary();
    let available = ui.available_width();
    let texts: Vec<_> = promises
        .iter()
        .map(|(_, text)| kit::galley(ui, text, size::CAPTION, REGULAR, color, available))
        .collect();
    let widths: Vec<f32> = texts
        .iter()
        .map(|text| PROMISE_ICON + 6.0 + text.size().x)
        .collect();
    let across = widths.iter().sum::<f32>() + PROMISE_GAP * (widths.len() - 1) as f32;
    let widest = widths.iter().copied().fold(0.0, f32::max);
    let one_row = across <= available;
    let rows = if one_row { 1 } else { promises.len() };
    let (area, _) = ui.allocate_exact_size(vec2(available, rows as f32 * NAME_ROW), Sense::hover());
    // Under one another they share a left edge, so the icons stand in a column.
    let wide = if one_row { across } else { widest };
    let mut at = pos2(area.center().x - wide / 2.0, area.top() + NAME_ROW / 2.0);
    for (index, (text, width)) in texts.into_iter().zip(widths).enumerate() {
        paint_promise(ui, at, promises[index].0, text);
        if one_row {
            at.x += width + PROMISE_GAP;
        } else {
            at.y += NAME_ROW;
        }
    }
}

fn paint_promise(ui: &Ui, left: Pos2, icon: Icon, text: std::sync::Arc<egui::Galley>) {
    let color = theme::text_secondary();
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            left + vec2(PROMISE_ICON / 2.0, 0.0),
            Vec2::splat(PROMISE_ICON),
        ),
        icon,
        color,
    );
    let height = text.size().y;
    ui.painter()
        .galley(left + vec2(PROMISE_ICON + 6.0, -height / 2.0), text, color);
}
