//! The pieces of the order sheet: the market hero, the Buy/Sell tabs, the order-type tiles, a
//! labelled number box with −/+ buttons, the percent pills and the summary card. Built on the
//! Paper form's controls so a field looks the same wherever an amount is typed.

use degenview_brokers::{Decimal, Side};
use eframe::egui::{self, Color32, Id, Rect, Sense, Ui, Vec2, pos2, vec2};

use crate::model::DataSource;
use crate::model::broker::ticket::parse_decimal;
use crate::trading::model::OrderClass;
use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::theme;

/// The height of a box the user types in or picks from.
pub const FIELD_HEIGHT: f32 = 40.0;
const FIELD_RADIUS: f32 = 9.0;

/// What a stepper press did, so the caller can note that the user edited the field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Edit {
    None,
    Typed,
    Stepped,
}

/// A side's colour toned down for a large solid fill, where the chart's full-strength green and
/// red are too loud.
pub fn muted(tint: Color32) -> Color32 {
    let scale = |channel: u8| (f32::from(channel) * 0.82).round() as u8;
    Color32::from_rgb(scale(tint.r()), scale(tint.g()), scale(tint.b()))
}

/// The label colour on a solid fill of `tint`: dark on a light green, white on red.
pub fn on_tint(tint: Color32) -> Color32 {
    let luminance =
        0.299 * f32::from(tint.r()) + 0.587 * f32::from(tint.g()) + 0.114 * f32::from(tint.b());
    if luminance > 130.0 {
        Color32::from_rgb(8, 22, 14)
    } else {
        Color32::WHITE
    }
}

/// What the top of the sheet says about the market in focus.
pub struct Hero<'a> {
    pub symbol: &'a str,
    /// Where the market's logo is looked up; none draws no logo.
    pub source: Option<DataSource>,
    /// `BASE / QUOTE`, once the venue has said what the market is.
    pub subtitle: Option<String>,
    pub price: Option<String>,
    /// The move on the chart's header, and whether it is up.
    pub change: Option<(String, bool)>,
}

/// Logo, symbol and pair on the left; last price and its move on the right.
pub fn market_hero(ui: &mut Ui, hero: &Hero) {
    const HEIGHT: f32 = 38.0;
    const LOGO: f32 = 32.0;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), Sense::hover());
    let mut left = rect.left();
    if let Some(source) = hero.source {
        let icon = Rect::from_min_size(pos2(left, rect.center().y - LOGO / 2.0), Vec2::splat(LOGO));
        kit::chart_icon(ui, icon, hero.symbol, source, false);
        left += LOGO + 10.0;
    }
    // The right column first: the symbol gives way to it, not the other way round.
    let mut right_width = 0.0;
    if hero.price.is_some() {
        let price = hero.price.as_deref().unwrap_or_default();
        let price = kit::galley(
            ui,
            price,
            kit::size::TITLE2,
            kit::SEMIBOLD,
            theme::text_primary(),
            f32::INFINITY,
        );
        let change = hero.change.as_ref().map(|(text, up)| {
            let tint = if *up { theme::UP } else { theme::DOWN };
            (
                kit::galley(
                    ui,
                    text,
                    kit::size::SUBHEADLINE,
                    kit::MEDIUM,
                    tint,
                    f32::INFINITY,
                ),
                tint,
            )
        });
        let block = price.size().y + change.as_ref().map_or(0.0, |(g, _)| g.size().y + 1.0);
        let mut y = rect.center().y - block / 2.0;
        right_width = price
            .size()
            .x
            .max(change.as_ref().map_or(0.0, |(g, _)| g.size().x));
        let price_height = price.size().y;
        ui.painter().galley(
            pos2(rect.right() - price.size().x, y),
            price,
            theme::text_primary(),
        );
        y += price_height + 1.0;
        if let Some((galley, tint)) = change {
            ui.painter()
                .galley(pos2(rect.right() - galley.size().x, y), galley, tint);
        }
    }
    let room = (rect.right() - left - right_width - 10.0).max(40.0);
    let symbol = kit::galley(
        ui,
        hero.symbol,
        kit::size::BODY + 1.0,
        kit::SEMIBOLD,
        theme::text_primary(),
        room,
    );
    let subtitle = hero.subtitle.as_ref().map(|text| {
        kit::galley(
            ui,
            text,
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_secondary(),
            room,
        )
    });
    let block = symbol.size().y + subtitle.as_ref().map_or(0.0, |g| g.size().y + 1.0);
    let mut y = rect.center().y - block / 2.0;
    let symbol_height = symbol.size().y;
    ui.painter()
        .galley(pos2(left, y), symbol, theme::text_primary());
    y += symbol_height + 1.0;
    if let Some(subtitle) = subtitle {
        ui.painter()
            .galley(pos2(left, y), subtitle, theme::text_secondary());
    }
}

/// A label with a quiet hint at the other end of its line: `Limit price … Last 0.15670`.
pub fn label_row(ui: &mut Ui, label: &str, hint: Option<&str>) {
    ui.horizontal(|ui| {
        ui.label(kit::text(
            label,
            kit::size::SUBHEADLINE,
            kit::MEDIUM,
            theme::text_secondary(),
        ));
        if let Some(hint) = hint {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(kit::text(
                    hint,
                    kit::size::SUBHEADLINE,
                    kit::REGULAR,
                    theme::text_tertiary(),
                ));
            });
        }
    });
    ui.add_space(6.0);
}

/// A −/+ button: a filled rounded square inside the box.
fn step_button(ui: &mut Ui, icon: Icon, hint: &str) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
    let response = crate::ui::accessibility::label(response, hint)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let alpha = if response.is_pointer_button_down_on() {
        0.2
    } else if response.hovered() {
        0.14
    } else {
        0.08
    };
    ui.painter().rect_filled(rect, 7.0, kit::primary(alpha));
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(12.0)),
        icon,
        theme::text_primary(),
    );
    response.clicked()
}

/// A number the user types, with the asset it is counted in beside it and −/+ buttons at the end.
/// `step` is how far one press moves it; from an empty field a press starts at `start` (the
/// market's price, say). A red outline marks a value the sheet will not take.
#[allow(clippy::too_many_arguments)]
pub fn number_field(
    ui: &mut Ui,
    scope: &str,
    text: &mut String,
    placeholder: &str,
    unit: &str,
    is_error: bool,
    step: Option<Decimal>,
    start: Option<Decimal>,
) -> Edit {
    const SIZE: f32 = 14.0;
    let id = Id::new(("trade-number-field", scope));
    let focused = ui.memory(|memory| memory.has_focus(id));
    let (rect, background) =
        ui.allocate_exact_size(vec2(ui.available_width(), FIELD_HEIGHT), Sense::click());
    let (border, width) = if is_error {
        (theme::with_alpha(theme::DOWN, 0.8), 1.5)
    } else if focused {
        (theme::with_alpha(theme::text_secondary(), 0.8), 1.5)
    } else {
        (theme::with_alpha(theme::separator(), 0.7), 1.0)
    };
    ui.painter()
        .rect_filled(rect, FIELD_RADIUS, theme::quaternary(0.5));
    ui.painter().rect_stroke(
        rect,
        FIELD_RADIUS,
        egui::Stroke::new(width, border),
        egui::StrokeKind::Inside,
    );
    if background.clicked() {
        ui.memory_mut(|memory| memory.request_focus(id));
    }

    let mut edit = Edit::None;
    let before = text.clone();
    let buttons = if step.is_some() {
        2.0 * 30.0 + 4.0 + 6.0
    } else {
        0.0
    };
    if let Some(step) = step {
        let mut inner = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink2(vec2(5.0, 0.0)))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        inner.spacing_mut().item_spacing.x = 4.0;
        // Right to left: + is the last button, − sits before it.
        for (icon, direction, hint) in [(Icon::Plus, 1, "Increase"), (Icon::Minus, -1, "Decrease")]
        {
            if step_button(&mut inner, icon, hint) {
                let current = parse_decimal(text).ok().flatten().or(start);
                if let Some(current) = current {
                    let next = (current + step * Decimal::from(direction)).max(Decimal::ZERO);
                    *text = next.normalize().to_string();
                    edit = Edit::Stepped;
                }
            }
        }
    }
    // The number, then its unit right behind it, as far as the buttons leave room.
    let room = (rect.width() - 12.0 - buttons).max(60.0);
    let unit_width = if unit.is_empty() {
        0.0
    } else {
        6.0 + kit::galley(
            ui,
            unit,
            kit::size::CAPTION + 1.0,
            kit::REGULAR,
            theme::text_secondary(),
            f32::INFINITY,
        )
        .size()
        .x
    };
    let shown = if text.is_empty() {
        placeholder
    } else {
        text.as_str()
    };
    let measured = kit::galley(
        ui,
        shown,
        SIZE,
        kit::REGULAR,
        theme::text_primary(),
        f32::INFINITY,
    )
    .size()
    .x;
    let field_width = (measured + 10.0).clamp(36.0, (room - unit_width).max(36.0));
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_size(
                rect.min + vec2(12.0, 0.0),
                vec2(room, FIELD_HEIGHT),
            ))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    inner.spacing_mut().item_spacing.x = 0.0;
    kit::plain_field(
        &mut inner,
        id,
        text,
        placeholder,
        SIZE,
        kit::REGULAR,
        field_width,
    );
    if !unit.is_empty() {
        inner.add_space(6.0);
        inner.label(kit::text(
            unit,
            kit::size::CAPTION + 1.0,
            kit::REGULAR,
            theme::text_secondary(),
        ));
    }
    if edit == Edit::None && *text != before {
        edit = Edit::Typed;
    }
    edit
}

/// A field's label with a glyph before it: the take-profit and stop-loss pair.
pub fn labelled_icon(ui: &mut Ui, icon: Icon, tint: Color32, label: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        icons::inline(ui, icon, 14.0, tint);
        ui.label(kit::text(
            label,
            kit::size::SUBHEADLINE,
            kit::MEDIUM,
            theme::text_secondary(),
        ));
    });
    ui.add_space(6.0);
}

/// The Buy and Sell tabs: two halves, the chosen one filled in its side's colour. Colour here is
/// the trade's side, not a theme accent.
pub fn side_tabs(ui: &mut Ui, side: &mut Side) {
    const HEIGHT: f32 = 40.0;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), Sense::hover());
    let half = rect.width() / 2.0;
    ui.painter()
        .rect_filled(rect, FIELD_RADIUS + 1.0, theme::quaternary(0.3));
    ui.painter().rect_stroke(
        rect,
        FIELD_RADIUS + 1.0,
        egui::Stroke::new(1.0, theme::with_alpha(theme::separator(), 0.8)),
        egui::StrokeKind::Inside,
    );
    for (index, (value, label, tint)) in [
        (Side::Buy, "Buy", theme::UP),
        (Side::Sell, "Sell", theme::DOWN),
    ]
    .into_iter()
    .enumerate()
    {
        let tab = Rect::from_min_size(
            rect.min + vec2(half * index as f32, 0.0),
            vec2(half, HEIGHT),
        );
        let response = ui
            .interact(tab, Id::new(("trade-side-tab", label)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let response = crate::ui::accessibility::label(response, label);
        let chosen = *side == value;
        if chosen {
            ui.painter()
                .rect_filled(tab, FIELD_RADIUS + 1.0, muted(tint));
        } else if response.hovered() {
            ui.painter()
                .rect_filled(tab.shrink(2.0), FIELD_RADIUS, kit::primary(0.05));
        }
        let color = if chosen {
            on_tint(tint)
        } else {
            theme::text_primary()
        };
        let galley = kit::galley(
            ui,
            label,
            kit::size::BODY + 1.0,
            kit::SEMIBOLD,
            color,
            f32::INFINITY,
        );
        ui.painter()
            .galley(tab.center() - galley.size() / 2.0, galley, color);
        if response.clicked() {
            *side = value;
        }
    }
}

/// The glyph of an order type.
pub fn class_icon(class: OrderClass) -> Icon {
    match class {
        OrderClass::Market => Icon::ChartUptrend,
        OrderClass::Limit => Icon::List,
        OrderClass::Stop => Icon::StopSign,
        OrderClass::StopLimit => Icon::Link,
    }
}

/// The order types as tiles side by side: glyph above name, the chosen one outlined in the
/// side's colour.
pub fn class_tiles(
    ui: &mut Ui,
    classes: &[(OrderClass, &str)],
    class: &mut OrderClass,
    tint: Color32,
) {
    const HEIGHT: f32 = 46.0;
    const GAP: f32 = 6.0;
    if classes.is_empty() {
        return;
    }
    let count = classes.len() as f32;
    let width = (ui.available_width() - GAP * (count - 1.0)) / count;
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), Sense::hover());
    for (index, (value, label)) in classes.iter().enumerate() {
        let tile = Rect::from_min_size(
            row.min + vec2((width + GAP) * index as f32, 0.0),
            vec2(width, HEIGHT),
        );
        let response = ui
            .interact(tile, Id::new(("trade-class-tile", *label)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let response = crate::ui::accessibility::label(response, *label);
        let chosen = *class == *value;
        ui.painter().rect_filled(
            tile,
            FIELD_RADIUS,
            if chosen {
                theme::with_alpha(tint, 0.1)
            } else {
                theme::quaternary(if response.hovered() { 0.8 } else { 0.5 })
            },
        );
        if chosen {
            ui.painter().rect_stroke(
                tile,
                FIELD_RADIUS,
                egui::Stroke::new(1.5, muted(tint)),
                egui::StrokeKind::Inside,
            );
        }
        let color = if chosen {
            theme::text_primary()
        } else {
            theme::text_secondary()
        };
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(tile.center().x, tile.top() + 15.0), Vec2::splat(16.0)),
            class_icon(*value),
            color,
        );
        let galley = kit::galley(
            ui,
            label,
            kit::size::CAPTION + 1.0,
            kit::MEDIUM,
            color,
            tile.width() - 6.0,
        );
        ui.painter().galley(
            pos2(tile.center().x - galley.size().x / 2.0, tile.top() + 26.0),
            galley,
            color,
        );
        if response.clicked() {
            *class = *value;
        }
    }
}

/// 10%, 25%, 50%, 75% and 100% as equal outlined pills; returns the one clicked.
pub fn percent_row(ui: &mut Ui) -> Option<u32> {
    const HEIGHT: f32 = 32.0;
    const GAP: f32 = 6.0;
    let steps = [10u32, 25, 50, 75, 100];
    let width = (ui.available_width() - GAP * 4.0) / 5.0;
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), Sense::hover());
    let mut chosen = None;
    for (index, percent) in steps.into_iter().enumerate() {
        let pill = Rect::from_min_size(
            row.min + vec2((width + GAP) * index as f32, 0.0),
            vec2(width, HEIGHT),
        );
        let label = format!("{percent}%");
        let response = ui
            .interact(pill, Id::new(("trade-percent", percent)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let response = crate::ui::accessibility::label(response, &label);
        ui.painter().rect_filled(
            pill,
            HEIGHT / 2.0,
            theme::quaternary(if response.hovered() { 0.8 } else { 0.4 }),
        );
        ui.painter().rect_stroke(
            pill,
            HEIGHT / 2.0,
            egui::Stroke::new(1.0, theme::with_alpha(theme::separator(), 0.9)),
            egui::StrokeKind::Inside,
        );
        let galley = kit::galley(
            ui,
            &label,
            kit::size::CAPTION + 1.0,
            kit::SEMIBOLD,
            theme::text_primary(),
            f32::INFINITY,
        );
        ui.painter().galley(
            pill.center() - galley.size() / 2.0,
            galley,
            theme::text_primary(),
        );
        if response.clicked() {
            chosen = Some(percent);
        }
    }
    chosen
}

/// One line of the summary card.
pub struct SummaryRow {
    pub label: String,
    pub value: String,
    /// The last line: the total, in the stronger type.
    pub strong: bool,
    /// What the ⓘ after the label says on hover.
    pub info: Option<&'static str>,
}

/// What the order comes to, as a card: `Order value`, `Trading fee (0.10%) ⓘ`, `Total`.
pub fn summary_card(ui: &mut Ui, rows: &[SummaryRow]) {
    egui::Frame::new()
        .fill(theme::quaternary(0.35))
        .stroke(egui::Stroke::new(
            1.0,
            theme::with_alpha(theme::separator(), 0.7),
        ))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(14, 11))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            for (index, row) in rows.iter().enumerate() {
                if index > 0 {
                    ui.add_space(8.0);
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let (size, weight, color) = if row.strong {
                        (kit::size::BODY, kit::SEMIBOLD, theme::text_primary())
                    } else {
                        (kit::size::BODY, kit::REGULAR, theme::text_secondary())
                    };
                    ui.label(kit::text(&row.label, size, weight, color));
                    if let Some(info) = row.info {
                        let response =
                            icons::inline(ui, Icon::InfoCircle, 13.0, theme::text_tertiary());
                        response.on_hover_text(info);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let value_color = if row.value.starts_with('—') {
                            theme::text_tertiary()
                        } else {
                            theme::text_primary()
                        };
                        ui.label(kit::text(
                            &row.value,
                            size,
                            if row.strong {
                                kit::SEMIBOLD
                            } else {
                                kit::MEDIUM
                            },
                            value_color,
                        ));
                    });
                });
            }
        });
}
