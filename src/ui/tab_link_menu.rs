//! The Tab Linking section at the bottom of a tab's right-click menu: a colour tag for the tab and
//! the four things its group keeps in step. The tab strip owns the menu; this draws the section.
//!
//! Laid out after TradingView Desktop's menu (the proportions and the nine colours were taken from
//! a screenshot of it): a muted heading, a row of round swatches with a check on the chosen one,
//! then Symbol, Interval, Time and Date range, each with an info icon and a switch.
//!
//! Choosing the colour that is already chosen unlinks the tab. The switches belong to the colour
//! (every tab of that colour shares them), so with no colour they are shown dimmed and do nothing.

use crate::model::tab_link::{LinkColor, LinkKind, LinkToggles};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM};
use crate::ui::theme;
use eframe::egui::{
    self, Color32, Id, Rect, Sense, Stroke, StrokeKind, Ui, WidgetInfo, WidgetType, pos2, vec2,
};

/// The room the section needs inside the menu; the menu is made at least this wide.
pub const WIDTH: f32 = 288.0;
const PADDING: f32 = 14.0;
/// Distance between the centres of two swatches, their drawn size and their hit target.
const PITCH: f32 = 30.0;
const SWATCH: f32 = 18.0;
const HIT: f32 = 24.0;
const SWATCH_ROW: f32 = 40.0;
const ROW: f32 = 33.0;
const SWITCH: egui::Vec2 = vec2(38.0, 20.0);
const LABEL_SIZE: f32 = 15.0;
const INFO: f32 = 16.0;

/// What the user changed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TabLinkEdit {
    /// Tag the tab with a colour, or `None` to unlink it.
    Color(Option<LinkColor>),
    /// The switches of the tab's colour.
    Toggles(LinkToggles),
}

/// A colour's swatch. Taken from TradingView's own menu.
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

/// The check on a swatch: white, except on the light colours where it would vanish.
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

/// Draws the section for a tab tagged `color`, whose group has `toggles`. Returns the
/// change when the user made one.
pub fn show(ui: &mut Ui, color: Option<LinkColor>, toggles: LinkToggles) -> Option<TabLinkEdit> {
    let mut edit = None;
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    kit::menu_heading(ui, "Tab linking");
    if let Some(picked) = swatches(ui, color) {
        edit = Some(TabLinkEdit::Color(picked));
    }
    let enabled = color.is_some();
    let mut next = toggles;
    for row in Row::ALL {
        let mut on = row.get(&next);
        if switch_row(ui, row, &mut on, enabled) {
            row.set(&mut next, on);
        }
    }
    if enabled && next != toggles {
        edit = Some(TabLinkEdit::Toggles(next));
    }
    edit
}

/// The nine round swatches. `Some(Some(c))` picks a colour, `Some(None)` unlinks.
fn swatches(ui: &mut Ui, current: Option<LinkColor>) -> Option<Option<LinkColor>> {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, SWATCH_ROW), Sense::hover());
    let mut picked = None;
    for (index, color) in LinkColor::ALL.into_iter().enumerate() {
        let center = pos2(
            rect.left() + PADDING + SWATCH / 2.0 + index as f32 * PITCH,
            rect.center().y,
        );
        let hit = Rect::from_center_size(center, vec2(HIT, HIT));
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
        painter.circle_filled(center, radius, swatch(color));
        if response.hovered() && !selected {
            painter.circle_stroke(center, radius + 2.0, Stroke::new(1.5, kit::primary(0.28)));
        }
        if response.has_focus() {
            painter.circle_stroke(
                center,
                radius + 3.5,
                Stroke::new(2.0, theme::text_secondary()),
            );
        }
        if selected {
            icons::paint(
                painter,
                Rect::from_center_size(center, vec2(12.0, 12.0)),
                Icon::Check,
                check_color(color),
            );
        }
    }
    picked
}

/// One row: the label, an info icon, and the switch at the right. The row is the control.
fn switch_row(ui: &mut Ui, row: Row, on: &mut bool, enabled: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), ROW),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let (name, state) = (row.title().to_owned(), *on);
    response.widget_info(move || {
        WidgetInfo::selected(WidgetType::Checkbox, enabled, state, name.clone())
    });
    let mut changed = false;
    if enabled && response.clicked() {
        *on = !*on;
        changed = true;
    }
    let painter = ui.painter();
    let band = rect.shrink2(vec2(4.0, 1.0));
    if enabled && response.hovered() {
        painter.rect_filled(band, 6.0, kit::primary(0.06));
    }
    if enabled && response.has_focus() {
        painter.rect_stroke(
            band,
            6.0,
            Stroke::new(2.0, theme::text_secondary()),
            StrokeKind::Inside,
        );
    }
    let dim = if enabled { 1.0 } else { 0.45 };
    let label = kit::galley(
        ui,
        row.title(),
        LABEL_SIZE,
        MEDIUM,
        theme::text_primary().gamma_multiply(dim),
        rect.width() / 2.0,
    );
    let label_pos = pos2(
        rect.left() + PADDING,
        rect.center().y - label.size().y / 2.0,
    );
    let info_center = pos2(
        label_pos.x + label.size().x + 8.0 + INFO / 2.0,
        rect.center().y,
    );
    ui.painter().galley(label_pos, label, theme::text_primary());

    // Not part of the row's click: pointing at it explains, pressing it does nothing.
    let info_rect = Rect::from_center_size(info_center, vec2(INFO + 4.0, INFO + 4.0));
    let info = ui.interact(
        info_rect,
        Id::new(("tab-link-info", row.title())),
        Sense::click(),
    );
    let tip = explanation(row);
    let info =
        crate::ui::accessibility::label(info.on_hover_text(tip), format!("{}: {tip}", row.title()));
    icons::paint(
        ui.painter(),
        Rect::from_center_size(info_center, vec2(INFO, INFO)),
        Icon::InfoCircle,
        if info.hovered() {
            theme::text_secondary()
        } else {
            theme::text_tertiary()
        },
    );

    let progress = ui
        .ctx()
        .animate_bool(Id::new(("tab-link-switch", row.title())), *on);
    let switch = Rect::from_center_size(
        pos2(rect.right() - PADDING - SWITCH.x / 2.0, rect.center().y),
        SWITCH,
    );
    kit::paint_switch(ui.painter(), switch, progress, !enabled);
    changed
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
