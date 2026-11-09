//! "Link Tabs": the section at the bottom of a chart tab's right-click menu where the tab gets a
//! colour tag and chooses which parts of its chart the tabs of that colour keep in step. The tab
//! strip owns the menu; this draws the section.
//!
//! A title, a Link Color card with nine swatches and, once a colour is chosen, the four things that
//! can be synced as a grid of cards (Symbol, Interval, Time, Date range) and a picture of two
//! linked tabs. Choosing the colour that is already chosen unlinks, and the cards go away with it.
//! The switches belong to the colour: every tab of that colour shares them.
//!
//! Unlike the mock-up this follows, nothing here is blue: a card that is on is marked by a lighter
//! fill and a neutral outline.

use crate::model::tab_link::{LinkColor, LinkKind, LinkToggles};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, REGULAR, SEMIBOLD, size};
use crate::ui::theme;
use eframe::egui::{
    self, Color32, CornerRadius, Id, Rect, Sense, Stroke, StrokeKind, Ui, WidgetInfo, WidgetType,
    pos2, vec2,
};

/// The room the section needs inside the menu; the menu is made at least this wide.
pub const WIDTH: f32 = 332.0;
/// Space between the section's edge and its cards.
const PAD: f32 = 12.0;
const HEADER: f32 = 40.0;
/// The Link Color card, the option cards and the gap between neighbours.
const COLOR_CARD: f32 = 40.0;
const OPTION: f32 = 40.0;
const GAP: f32 = 8.0;
/// Distance between the centres of two swatches, and their drawn size.
const PITCH: f32 = 21.0;
const SWATCH: f32 = 14.0;
const SWITCH: egui::Vec2 = vec2(32.0, 18.0);
const PICTURE: f32 = 78.0;

/// What the user changed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TabLinkEdit {
    /// Tag the tab with a colour, or `None` to unlink it.
    Color(Option<LinkColor>),
    /// The switches of the tab's colour.
    Toggles(LinkToggles),
}

/// A colour's swatch. The nine colours were sampled from TradingView's menu.
pub fn swatch(color: LinkColor) -> Color32 {
    match color {
        LinkColor::Red => Color32::from_rgb(0xde, 0x43, 0x41),
        LinkColor::Green => Color32::from_rgb(0x5a, 0xa1, 0x4d),
        LinkColor::Blue => Color32::from_rgb(0x36, 0x70, 0xed),
        LinkColor::Yellow => Color32::from_rgb(0xf8, 0xd2, 0x45),
        LinkColor::Orange => Color32::from_rgb(0xec, 0x86, 0x26),
        LinkColor::Cyan => Color32::from_rgb(0x44, 0xb0, 0xcc),
        LinkColor::Purple => Color32::from_rgb(0x86, 0x33, 0xa3),
        LinkColor::Pink => Color32::from_rgb(0xef, 0x7e, 0xa1),
        LinkColor::Violet => Color32::from_rgb(0x50, 0x32, 0xf8),
    }
}

/// The tick on a swatch: white, except on the light colours where it would vanish.
fn check_color(color: LinkColor) -> Color32 {
    match color {
        LinkColor::Yellow | LinkColor::Pink => Color32::from_rgb(0x1c, 0x1c, 0x1c),
        _ => Color32::WHITE,
    }
}

/// What a row's info icon says.
pub fn explanation(kind: Row) -> &'static str {
    match kind {
        Row::Symbol => "Keep the same symbol across linked tabs.",
        Row::Interval => "Keep the same chart interval across linked tabs.",
        Row::Time => "Keep linked charts aligned to the same point in time.",
        Row::DateRange => "Keep linked charts displaying the same date range.",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Row {
    Symbol,
    Interval,
    Time,
    DateRange,
}

impl Row {
    const ALL: [Self; 4] = [Self::Symbol, Self::Interval, Self::Time, Self::DateRange];

    fn title(self) -> &'static str {
        match self {
            Self::Symbol => "Symbol",
            Self::Interval => "Interval",
            Self::Time => "Time",
            Self::DateRange => "Date range",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Symbol => Icon::ChartUptrend,
            Self::Interval => Icon::BarChart,
            Self::Time => Icon::Clock,
            Self::DateRange => Icon::Calendar,
        }
    }

    fn get(self, toggles: &LinkToggles) -> bool {
        match self {
            Self::Symbol => toggles.symbol,
            Self::Interval => toggles.interval,
            Self::Time => toggles.time,
            Self::DateRange => toggles.date_range,
        }
    }

    fn set(self, toggles: &mut LinkToggles, on: bool) {
        match self {
            Self::Symbol => toggles.symbol = on,
            Self::Interval => toggles.interval = on,
            Self::Time => toggles.time = on,
            Self::DateRange => toggles.date_range = on,
        }
    }
}

/// Draws the section for a tab tagged `color`, whose group has `toggles`. Returns the change when
/// the user made one. Done closes the menu it is in.
pub fn show(ui: &mut Ui, color: Option<LinkColor>, toggles: LinkToggles) -> Option<TabLinkEdit> {
    let mut edit = None;
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    let width = ui.available_width();
    ui.add_space(8.0);
    header(ui, width);
    ui.add_space(10.0);
    if let Some(picked) = color_card(ui, width, color) {
        edit = Some(TabLinkEdit::Color(picked));
    }
    // What to sync only makes sense for a tab that has a colour: with none chosen (or after the
    // chosen one is chosen again) the rest of the section is not there.
    if let Some(color) = color {
        ui.add_space(10.0);
        caption(ui, "Sync what");
        let mut next = toggles;
        for pair in Row::ALL.chunks(2) {
            let (rect, _) = ui.allocate_exact_size(vec2(width, OPTION + GAP), Sense::hover());
            let card = (width - 2.0 * PAD - GAP) / 2.0;
            for (column, row) in pair.iter().enumerate() {
                let at = Rect::from_min_size(
                    pos2(rect.left() + PAD + column as f32 * (card + GAP), rect.top()),
                    vec2(card, OPTION),
                );
                let mut on = row.get(&next);
                if option_card(ui, at, *row, &mut on) {
                    row.set(&mut next, on);
                }
            }
        }
        if next != toggles {
            edit = Some(TabLinkEdit::Toggles(next));
        }
        ui.add_space(2.0);
        picture(ui, width, color);
    }
    ui.add_space(10.0);
    edit
}

/// The title and what the section is for.
fn header(ui: &mut Ui, width: f32) {
    let (rect, response) = ui.allocate_exact_size(vec2(width, HEADER), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, "Link Tabs"));
    let left = rect.left() + PAD + 2.0;
    let room = rect.right() - PAD - left;
    let title = kit::galley(
        ui,
        "Link Tabs",
        size::HEADLINE + 2.0,
        SEMIBOLD,
        theme::text_primary(),
        room,
    );
    let subtitle = kit::paragraph(
        ui,
        "Sync this tab\u{2019}s charts with other tabs.",
        size::SUBHEADLINE,
        REGULAR,
        theme::text_secondary(),
        room,
    );
    let block = title.size().y + 2.0 + subtitle.size().y;
    let top = rect.center().y - block / 2.0;
    let title_height = title.size().y;
    let painter = ui.painter();
    painter.galley(pos2(left, top), title, theme::text_primary());
    painter.galley(
        pos2(left, top + title_height + 2.0),
        subtitle,
        theme::text_secondary(),
    );
}

/// A rounded card like the app's settings cards: a faint fill, an edge and a lit top line.
fn paint_card(ui: &Ui, rect: Rect, fill: Color32, edge: Stroke) {
    let painter = ui.painter();
    painter.rect_filled(rect, 10.0, fill);
    painter.rect_stroke(rect, 10.0, edge, StrokeKind::Inside);
}

/// A small muted heading over a group ("SYNC WHAT"), in the menu headings' style.
fn caption(ui: &mut Ui, text: &str) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
    let color = theme::text_tertiary();
    let label = kit::galley(
        ui,
        &text.to_uppercase(),
        size::CAPTION,
        SEMIBOLD,
        color,
        rect.width() - 2.0 * PAD,
    );
    ui.painter().galley(
        pos2(
            rect.left() + PAD + 2.0,
            rect.center().y - label.size().y / 2.0 + 1.0,
        ),
        label,
        color,
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, text));
}

/// "Link Color" and nine swatches. `Some(Some(c))` picks a colour, `Some(None)` unlinks.
fn color_card(ui: &mut Ui, width: f32, current: Option<LinkColor>) -> Option<Option<LinkColor>> {
    let (outer, _) = ui.allocate_exact_size(vec2(width, COLOR_CARD), Sense::hover());
    let card = outer.shrink2(vec2(PAD, 0.0));
    paint_card(
        ui,
        card,
        theme::card_fill(false),
        Stroke::new(1.0, theme::card_border(false)),
    );
    let label = kit::galley(
        ui,
        "Link Color",
        size::BODY,
        REGULAR,
        theme::text_primary(),
        card.width() / 2.0,
    );
    ui.painter().galley(
        pos2(card.left() + 12.0, card.center().y - label.size().y / 2.0),
        label,
        theme::text_primary(),
    );
    let mut picked = None;
    let run = PITCH * 8.0 + SWATCH;
    let first = card.right() - 12.0 - run + SWATCH / 2.0;
    for (index, color) in LinkColor::ALL.into_iter().enumerate() {
        let center = pos2(first + index as f32 * PITCH, card.center().y);
        let hit = Rect::from_center_size(center, vec2(PITCH - 1.0, PITCH - 1.0));
        let selected = current == Some(color);
        let response = ui.interact(
            hit,
            Id::new(("tab-link-swatch", color.key())),
            Sense::click(),
        );
        let name = if selected {
            format!("{}, selected. Choose again to unlink", color.title())
        } else {
            color.title().to_owned()
        };
        response.widget_info(move || {
            WidgetInfo::selected(WidgetType::RadioButton, true, selected, name.clone())
        });
        let response = response.on_hover_text(if selected {
            format!("{} (click to unlink)", color.title())
        } else {
            color.title().to_owned()
        });
        if response.clicked() {
            picked = Some(if selected { None } else { Some(color) });
        }
        let painter = ui.painter();
        let radius = SWATCH / 2.0;
        if selected {
            // Ringed as well as ticked, so the choice is not told by hue alone.
            painter.circle_stroke(
                center,
                radius + 2.5,
                Stroke::new(1.5, theme::text_primary()),
            );
        } else if response.hovered() {
            painter.circle_filled(center, radius + 3.0, theme::selected_tint(true));
        }
        painter.circle_filled(center, radius, swatch(color));
        if response.has_focus() {
            painter.circle_stroke(
                center,
                radius + 4.5,
                Stroke::new(2.0, theme::text_secondary()),
            );
        }
        if selected {
            icons::paint(
                painter,
                Rect::from_center_size(center, vec2(9.0, 9.0)),
                Icon::Check,
                check_color(color),
            );
        }
    }
    picked
}

/// One of the four things to sync: a glyph, its name and a switch, on a card that is lit while
/// it is on. The whole card is the control. Returns whether it was flipped.
fn option_card(ui: &mut Ui, rect: Rect, row: Row, on: &mut bool) -> bool {
    let response = ui.interact(
        rect,
        Id::new(("tab-link-option", row.title())),
        Sense::click(),
    );
    let tip = explanation(row);
    let (name, state) = (row.title().to_owned(), *on);
    response
        .widget_info(move || WidgetInfo::selected(WidgetType::Checkbox, true, state, name.clone()));
    let response = response.on_hover_text(tip);
    let mut changed = false;
    if response.clicked() {
        *on = !*on;
        changed = true;
    }
    let lit = *on;
    let hovered = response.hovered();
    let (fill, edge) = if lit {
        (
            theme::card_fill(true),
            Stroke::new(1.5, theme::text_secondary()),
        )
    } else {
        (
            theme::card_fill(hovered),
            Stroke::new(1.0, theme::card_border(hovered)),
        )
    };
    paint_card(ui, rect, fill, edge);
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            12.0,
            Stroke::new(2.0, theme::text_secondary()),
            StrokeKind::Outside,
        );
    }
    let color = theme::text_primary();
    let icon_color = if lit {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(14.0, 14.0)),
        row.icon(),
        icon_color,
    );
    let label = kit::galley(
        ui,
        row.title(),
        size::BODY,
        REGULAR,
        color,
        rect.width() - 34.0 - SWITCH.x - 14.0,
    );
    ui.painter().galley(
        pos2(rect.left() + 36.0, rect.center().y - label.size().y / 2.0),
        label,
        color,
    );
    let progress = ui
        .ctx()
        .animate_bool(Id::new(("tab-link-switch", row.title())), *on);
    let switch = Rect::from_center_size(
        pos2(rect.right() - 10.0 - SWITCH.x / 2.0, rect.center().y),
        SWITCH,
    );
    kit::paint_switch(ui.painter(), switch, progress, false);
    changed
}

/// Two small tabs joined by an arrow, in the group's colour, and what linking does.
fn picture(ui: &mut Ui, width: f32, color: LinkColor) {
    let (outer, _) = ui.allocate_exact_size(vec2(width, PICTURE), Sense::hover());
    let card = outer.shrink2(vec2(PAD, 0.0));
    paint_card(
        ui,
        card,
        theme::card_fill(false),
        Stroke::new(1.0, theme::card_border(false)),
    );
    let accent = swatch(color);
    let window = vec2(52.0, 44.0);
    let top = card.center().y - window.y / 2.0;
    let first = Rect::from_min_size(pos2(card.left() + 12.0, top), window);
    let second = Rect::from_min_size(pos2(first.right() + 30.0, top), window);
    for tab in [first, second] {
        mini_tab(ui, tab, accent);
    }
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            pos2((first.right() + second.left()) / 2.0, card.center().y),
            vec2(13.0, 13.0),
        ),
        Icon::ArrowLeftRight,
        theme::text_secondary(),
    );
    let divider = second.right() + 10.0;
    ui.painter().vline(
        divider,
        (card.top() + 14.0)..=(card.bottom() - 14.0),
        Stroke::new(1.0, theme::separator()),
    );
    let text = kit::paragraph(
        ui,
        "Changes you make (e.g. symbol) will be synced to other linked tabs with the same link color.",
        size::SUBHEADLINE,
        REGULAR,
        theme::text_secondary(),
        card.right() - 10.0 - (divider + 10.0),
    );
    ui.painter().galley(
        pos2(divider + 10.0, card.center().y - text.size().y / 2.0),
        text,
        theme::text_secondary(),
    );
}

/// A tab window in miniature: three dots, the colour line, a chain, a rising line.
fn mini_tab(ui: &Ui, rect: Rect, accent: Color32) {
    let painter = ui.painter();
    painter.rect_filled(rect, 6.0, theme::well_fill());
    painter.rect_stroke(
        rect,
        6.0,
        Stroke::new(1.0, theme::card_border(true)),
        StrokeKind::Inside,
    );
    for dot in 0..3 {
        painter.circle_filled(
            pos2(rect.left() + 7.0 + dot as f32 * 5.0, rect.top() + 7.0),
            1.4,
            theme::text_tertiary(),
        );
    }
    icons::paint(
        painter,
        Rect::from_center_size(pos2(rect.right() - 9.0, rect.top() + 7.0), vec2(9.0, 9.0)),
        Icon::ChainLinked,
        accent,
    );
    let bar = Rect::from_min_max(
        pos2(rect.left() + 6.0, rect.top() + 12.0),
        pos2(rect.right() - 6.0, rect.top() + 14.5),
    );
    painter.rect_filled(bar, CornerRadius::same(1), accent);
    let base = rect.bottom() - 7.0;
    let points: Vec<egui::Pos2> = [0.0, 0.22, 0.12, 0.45, 0.35, 0.7, 1.0]
        .iter()
        .enumerate()
        .map(|(index, rise)| {
            pos2(
                rect.left() + 7.0 + index as f32 * (rect.width() - 14.0) / 6.0,
                base - rise * 14.0,
            )
        })
        .collect();
    painter.add(egui::Shape::line(
        points,
        Stroke::new(1.2, theme::text_secondary()),
    ));
}

/// Which kinds a set of switches turns on, for a tab's tooltip.
pub fn describe(toggles: LinkToggles) -> String {
    let mut parts = Vec::new();
    if toggles.enables(LinkKind::Symbol) {
        parts.push("symbol");
    }
    if toggles.interval {
        parts.push("interval");
    }
    if toggles.time {
        parts.push("time");
    }
    if toggles.date_range {
        parts.push("date range");
    }
    if parts.is_empty() {
        "nothing synced".to_owned()
    } else {
        parts.join(", ")
    }
}

#[cfg(test)]
mod tests;
