//! The boxes of the order sheet: a number with a unit and −/+ steppers, a read-only figure, and
//! the segmented Buy/Sell tabs. Built on the Paper form's controls so a field looks the same
//! wherever an amount is typed.

use degenview_brokers::{Decimal, Side};
use eframe::egui::{self, Id, Vec2};

use crate::model::broker_ticket::parse_decimal;
use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::paper_form::FIELD_HEIGHT;
use crate::ui::theme;

/// What a stepper press did, so the caller can note that the user edited the field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Edit {
    None,
    Typed,
    Stepped,
}

/// A number the user types, with the asset it is counted in and −/+ steppers. `step` is how far
/// one press moves it; from an empty field a press starts at `start` (the market's price, say).
/// A red outline marks a value the sheet will not take.
#[allow(clippy::too_many_arguments)]
pub fn number_field(
    ui: &mut egui::Ui,
    salt: &str,
    text: &mut String,
    placeholder: &str,
    unit: &str,
    is_error: bool,
    step: Option<Decimal>,
    start: Option<Decimal>,
) -> Edit {
    let id = Id::new(("trade-number-field", salt));
    let focused = ui.memory(|memory| memory.has_focus(id));
    let (rect, background) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), FIELD_HEIGHT),
        egui::Sense::click(),
    );
    let (border, width) = if is_error {
        (theme::with_alpha(theme::DOWN, 0.8), 1.5)
    } else if focused {
        (theme::with_alpha(theme::text_secondary(), 0.8), 1.5)
    } else {
        (theme::with_alpha(theme::separator(), 0.6), 1.0)
    };
    ui.painter().rect_filled(rect, 8.0, theme::quaternary(0.5));
    ui.painter().rect_stroke(
        rect,
        8.0,
        egui::Stroke::new(width, border),
        egui::StrokeKind::Inside,
    );
    if background.clicked() {
        ui.memory_mut(|memory| memory.request_focus(id));
    }

    let mut edit = Edit::None;
    let before = text.clone();
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(8.0, 0.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    inner.spacing_mut().item_spacing.x = 4.0;
    // Right to left: the unit, then + and −, then the text takes what is left.
    if !unit.is_empty() {
        inner.add_space(2.0);
        inner.label(kit::text(
            unit,
            kit::size::CAPTION + 1.0,
            kit::REGULAR,
            theme::text_secondary(),
        ));
    }
    if let Some(step) = step {
        for (icon, direction, hint) in [(Icon::Plus, 1, "Increase"), (Icon::Minus, -1, "Decrease")]
        {
            let clicked =
                icons::button(&mut inner, icon, 20.0, theme::text_secondary(), hint).clicked();
            if clicked {
                let current = parse_decimal(text).ok().flatten().or(start);
                if let Some(current) = current {
                    let next = (current + step * Decimal::from(direction)).max(Decimal::ZERO);
                    *text = next.normalize().to_string();
                    edit = Edit::Stepped;
                }
            }
        }
    }
    let room = inner.available_width().max(36.0);
    kit::plain_field(&mut inner, id, text, placeholder, 14.0, kit::REGULAR, room);
    if edit == Edit::None && *text != before {
        edit = Edit::Typed;
    }
    edit
}

/// A figure in a box that cannot be typed into (the total).
pub fn figure_field(ui: &mut egui::Ui, text: &str, unit: &str) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), FIELD_HEIGHT),
        egui::Sense::hover(),
    );
    ui.painter().rect_filled(rect, 8.0, theme::quaternary(0.3));
    ui.painter().rect_stroke(
        rect,
        8.0,
        egui::Stroke::new(1.0, theme::with_alpha(theme::separator(), 0.4)),
        egui::StrokeKind::Inside,
    );
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(10.0, 0.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    inner.spacing_mut().item_spacing.x = 6.0;
    if !unit.is_empty() {
        inner.label(kit::text(
            unit,
            kit::size::CAPTION + 1.0,
            kit::REGULAR,
            theme::text_secondary(),
        ));
    }
    inner.label(kit::text(
        text,
        14.0,
        kit::REGULAR,
        if text == "—" {
            theme::text_tertiary()
        } else {
            theme::text_primary()
        },
    ));
}

/// The Buy and Sell tabs: two halves, the chosen one in its side's colour. Colour here is the
/// trade's side, not a theme accent.
pub fn side_tabs(ui: &mut egui::Ui, side: &mut Side) {
    const HEIGHT: f32 = 38.0;
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), HEIGHT),
        egui::Sense::hover(),
    );
    let half = rect.width() / 2.0;
    ui.painter().rect_filled(rect, 10.0, theme::quaternary(0.4));
    for (index, (value, label, tint)) in [
        (Side::Buy, "Buy", theme::UP),
        (Side::Sell, "Sell", theme::DOWN),
    ]
    .into_iter()
    .enumerate()
    {
        let tab = egui::Rect::from_min_size(
            rect.min + Vec2::new(half * index as f32, 0.0),
            Vec2::new(half, HEIGHT),
        );
        let response = ui
            .interact(
                tab,
                Id::new(("trade-side-tab", label)),
                egui::Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let response = crate::ui::accessibility::label(response, label);
        let chosen = *side == value;
        if chosen {
            ui.painter()
                .rect_filled(tab.shrink(2.0), 8.0, theme::with_alpha(tint, 0.16));
            ui.painter().rect_stroke(
                tab.shrink(2.0),
                8.0,
                egui::Stroke::new(1.0, theme::with_alpha(tint, 0.5)),
                egui::StrokeKind::Inside,
            );
        } else if response.hovered() {
            ui.painter()
                .rect_filled(tab.shrink(2.0), 8.0, kit::primary(0.05));
        }
        let color = if chosen {
            tint
        } else {
            theme::text_secondary()
        };
        let galley = kit::galley(ui, label, 14.0, kit::SEMIBOLD, color, f32::INFINITY);
        ui.painter()
            .galley(tab.center() - galley.size() / 2.0, galley, color);
        if response.clicked() {
            *side = value;
        }
    }
}
