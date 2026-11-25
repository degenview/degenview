//! The orders table and the bar under it, drawn from [`OrderRow`]s so a venue's orders look the
//! same wherever they come from. The table asks for things (cancel, edit, look up); it changes
//! nothing itself. Which orders it is given, and what an action does, is the venue's business.

use std::collections::BTreeSet;

use degenview_brokers::{Decimal, Side};
use eframe::egui;

use crate::i18n::{tr, tr_c};
use crate::trading::model::{OrderRow, RowStatus, Tone};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::Tone as BadgeTone;
use crate::ui::kit::time_text::clock_time_seconds;
use crate::ui::paper::grid::{self as grid, Col, col};
use crate::ui::paper::tables::empty;
use crate::ui::{kit, theme};

/// What the user asked of an order, by the id the venue gave it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OrderAction {
    Cancel(String),
    Edit(String),
    /// An order sent whose answer was lost: look it up.
    Check(String),
}

/// What the bar under the table was used for.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BarAction {
    pub cancel_selected: bool,
    pub cancel_all: bool,
}

pub fn badge_tone(tone: Tone) -> BadgeTone {
    match tone {
        Tone::Good => BadgeTone::Good,
        Tone::Warning => BadgeTone::Warning,
        Tone::Bad => BadgeTone::Bad,
        Tone::Neutral => BadgeTone::Neutral,
    }
}

fn side_color(side: Side) -> egui::Color32 {
    match side {
        Side::Buy => theme::UP,
        Side::Sell => theme::DOWN,
    }
}

/// `1,234.50`: thousands grouped, the digits as the venue wrote them (no trailing zero is cut, so
/// a price keeps the places its market has).
pub fn figure(value: Decimal) -> String {
    let text = value.to_string();
    let (sign, rest) = text
        .strip_prefix('-')
        .map_or(("", text.as_str()), |rest| ("-", rest));
    let (whole, fraction) = rest
        .split_once('.')
        .map_or((rest, None), |(whole, fraction)| (whole, Some(fraction)));
    let mut grouped = String::new();
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    match fraction {
        Some(fraction) => format!("{sign}{grouped}.{fraction}"),
        None => format!("{sign}{grouped}"),
    }
}

/// The price cell: the limit (or stop), or the word for an order that has none.
pub fn price_text(row: &OrderRow) -> String {
    match row.price {
        Some(price) => figure(price),
        None if row.kind == "Market" => tr_c!("order type", "Market").to_owned(),
        None => "—".to_owned(),
    }
}

/// "0.0004 / 0.001", or "0 / 25 (spend)" for a market buy sized by an amount to spend.
pub fn filled_text(row: &OrderRow) -> String {
    let size = match (row.size, row.spend) {
        (Some(size), _) => figure(size),
        (None, Some(spend)) => tr!("{amount} (spend)", amount = figure(spend)),
        (None, None) => "—".to_owned(),
    };
    format!("{} / {size}", figure(row.filled))
}

/// The Side column of a table of orders or fills: 64 points, or what the BUY and SELL tags need in the
/// language in use, whichever is more. The words are the ones `broker::chart_trading::side_tag` writes.
pub fn side_column_width(ui: &egui::Ui) -> f32 {
    grid::tags_width(ui, 64.0, [tr!("BUY"), tr!("SELL")])
}

/// The Status column: `min`, or what the widest badge among `rows` needs.
pub fn status_column_width(ui: &egui::Ui, min: f32, rows: &[OrderRow]) -> f32 {
    grid::badges_width(ui, min, rows.iter().map(|row| row.status.label))
}

/// The columns of a table with a trigger column.
fn with_trigger(side: f32, status: f32) -> [Col; 10] {
    [
        col("", 30.0, 0.0, false),
        col(tr!("Market"), 130.0, 1.2, false),
        col(tr!("Side"), side, 0.0, false),
        col(tr!("Type"), 120.0, 1.0, false),
        col(tr!("Price"), 110.0, 1.0, true),
        col(tr!("Trigger"), 100.0, 0.8, true),
        col(tr!("Filled / Size"), 130.0, 1.0, true),
        col(tr!("Status"), status, 0.0, false),
        col(tr!("Time"), 80.0, 0.0, true),
        col("", 76.0, 0.0, true),
    ]
}

/// The columns of a table with no trigger column.
fn without_trigger(side: f32, status: f32) -> [Col; 9] {
    [
        col("", 30.0, 0.0, false),
        col(tr!("Market"), 130.0, 1.2, false),
        col(tr!("Side"), side, 0.0, false),
        col(tr!("Type"), 120.0, 1.0, false),
        col(tr!("Price"), 110.0, 1.0, true),
        col(tr!("Filled / Size"), 130.0, 1.0, true),
        col(tr!("Status"), status, 0.0, false),
        col(tr!("Time"), 80.0, 0.0, true),
        col("", 76.0, 0.0, true),
    ]
}

/// What a venue's orders panel shows when it has no orders.
pub fn empty_orders(ui: &mut egui::Ui) {
    empty(
        ui,
        Icon::List,
        tr!("No orders"),
        tr!("Working orders rest here until they fill or are cancelled."),
    );
}

/// The table of `rows`. `selected` holds the ids ticked for a bulk cancel; ids that are no longer
/// working are dropped from it. Editing is offered only when `allow_edit`.
pub fn orders(
    ui: &mut egui::Ui,
    scope: &str,
    rows: &[OrderRow],
    selected: &mut BTreeSet<String>,
    allow_edit: bool,
) -> Vec<OrderAction> {
    selected.retain(|id| rows.iter().any(|row| &row.id == id && row.can_cancel));
    if rows.is_empty() {
        empty_orders(ui);
        return Vec::new();
    }
    let trigger = rows.iter().any(|row| row.trigger.is_some());
    let side = side_column_width(ui);
    let status = status_column_width(ui, 120.0, rows);
    let with_trigger = with_trigger(side, status);
    let without_trigger = without_trigger(side, status);
    let cols: &[Col] = if trigger {
        &with_trigger
    } else {
        &without_trigger
    };
    let mut actions = Vec::new();
    let mut toggled = None;
    grid::show(ui, scope, cols, rows.len(), |ui, index, column| {
        let row = &rows[index];
        let weak = theme::text_secondary();
        let primary = theme::text_primary();
        let dim = if row.working || row.unconfirmed {
            primary
        } else {
            weak
        };
        // The trigger column is only there for some tables: line the rest up behind it.
        let column = if !trigger && column >= 5 {
            column + 1
        } else {
            column
        };
        match column {
            0 if row.can_cancel => {
                let mut on = selected.contains(&row.id);
                if ui
                    .add(egui::Checkbox::without_text(&mut on))
                    .on_hover_text(tr!("Select to cancel several orders at once"))
                    .changed()
                {
                    toggled = Some(row.id.clone());
                }
            }
            1 => {
                ui.spacing_mut().item_spacing.x = 8.0;
                if let Some(source) = row.source {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::Vec2::splat(20.0), egui::Sense::hover());
                    kit::chart_icon(ui, rect, &row.market, source, false);
                }
                ui.add(
                    egui::Label::new(kit::text(&row.market, 12.5, kit::SEMIBOLD, dim)).truncate(),
                );
            }
            2 => grid::tag(
                ui,
                if row.side == Side::Buy {
                    tr!("BUY")
                } else {
                    tr!("SELL")
                },
                side_color(row.side),
            ),
            3 => {
                grid::text_cell(ui, &row.kind, weak);
                if let Some(tag) = &row.tag {
                    grid::tag(
                        ui,
                        tag.label,
                        if tag.tone == Tone::Good {
                            theme::UP
                        } else {
                            theme::DOWN
                        },
                    );
                }
            }
            4 => grid::figure(ui, &price_text(row), dim),
            5 => grid::figure(
                ui,
                &row.trigger.map_or_else(|| "—".to_owned(), figure),
                if row.trigger.is_some() { dim } else { weak },
            ),
            6 => grid::figure(ui, &filled_text(row), dim),
            7 => {
                let RowStatus { label, tone } = &row.status;
                kit::status_badge(ui, label, badge_tone(*tone));
            }
            8 => grid::figure(ui, &clock_time_seconds(row.created_ms), weak),
            9 if row.unconfirmed => {
                let clicked = kit::sheet_button(ui, tr!("Check"), false, true)
                    .on_hover_text(tr!(
                        "Look the order up at the venue; sends it again only if it is not there"
                    ))
                    .clicked();
                if clicked {
                    actions.push(OrderAction::Check(row.id.clone()));
                }
            }
            9 if row.working => {
                if allow_edit {
                    let hint = row.edit_hint.as_deref();
                    let pencil = ui
                        .add_enabled_ui(row.can_edit, |ui| {
                            icons::button(
                                ui,
                                Icon::Pencil,
                                20.0,
                                theme::text_secondary(),
                                hint.unwrap_or(tr!("Edit price or size")),
                            )
                        })
                        .inner;
                    if pencil.clicked() {
                        actions.push(OrderAction::Edit(row.id.clone()));
                    }
                }
                let cross = ui
                    .add_enabled_ui(row.can_cancel, |ui| {
                        icons::button(ui, Icon::Xmark, 20.0, kit::RED, tr!("Cancel order"))
                    })
                    .inner;
                if cross.clicked() {
                    actions.push(OrderAction::Cancel(row.id.clone()));
                }
            }
            _ => {}
        }
    });
    if let Some(id) = toggled
        && !selected.remove(&id)
    {
        selected.insert(id);
    }
    actions
}

/// The bar under the orders table: how many are working, and the two ways to cancel several.
pub fn orders_bar(ui: &mut egui::Ui, open: usize, selected: usize) -> BarAction {
    let mut action = BarAction::default();
    let (bar, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), grid::FOOTER_HEIGHT),
        egui::Sense::hover(),
    );
    ui.painter().rect_filled(bar, 0.0, kit::window());
    ui.painter().hline(
        bar.x_range(),
        bar.top(),
        egui::Stroke::new(1.0, theme::separator()),
    );
    let mut row = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(bar.shrink2(egui::vec2(20.0, 0.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    row.spacing_mut().item_spacing.x = 10.0;
    if kit::sheet_button(
        &mut row,
        &tr!("Cancel all ({open})", open = open),
        false,
        open > 0,
    )
    .clicked()
    {
        action.cancel_all = true;
    }
    if kit::sheet_button(
        &mut row,
        &tr!("Cancel selected ({selected})", selected = selected),
        false,
        selected > 0,
    )
    .clicked()
    {
        action.cancel_selected = true;
    }
    row.label(kit::text(
        tr!("{open} working", open = open),
        kit::size::CAPTION + 0.5,
        kit::REGULAR,
        theme::text_secondary(),
    ));
    action
}

/// An open limit order, for the tests of the tables that show one.
#[cfg(test)]
pub(crate) fn sample_row() -> OrderRow {
    use crate::trading::model::RowStatus;
    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }
    OrderRow {
        id: "1".to_owned(),
        market: "BTC-USD".to_owned(),
        side: Side::Buy,
        kind: "Limit".to_owned(),
        price: Some(d("89000.00")),
        trigger: None,
        filled: d("0.0004"),
        size: Some(d("0.001")),
        spend: None,
        status: RowStatus {
            label: "Open",
            tone: Tone::Good,
        },
        created_ms: None,
        working: true,
        can_cancel: true,
        can_edit: true,
        note: None,
        edit_hint: None,
        source: None,
        tag: None,
        pending: false,
        fees: Decimal::ZERO,
        unconfirmed: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> OrderRow {
        sample_row()
    }

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    #[test]
    fn figures_group_thousands_and_keep_the_places_the_venue_gave() {
        assert_eq!(figure(d("89000.00")), "89,000.00");
        assert_eq!(figure(d("1234567")), "1,234,567");
        assert_eq!(figure(d("-0.0015")), "-0.0015");
        assert_eq!(figure(d("999")), "999");
    }

    #[test]
    fn a_market_order_has_the_word_where_a_limit_has_its_price() {
        let mut market = row();
        market.kind = "Market".to_owned();
        market.price = None;
        assert_eq!(price_text(&market), "Market");
        assert_eq!(price_text(&row()), "89,000.00");
        let mut other = row();
        other.price = None;
        other.kind = "Stop".to_owned();
        assert_eq!(price_text(&other), "—");
    }

    #[test]
    fn size_is_said_in_the_base_asset_or_as_an_amount_to_spend() {
        assert_eq!(filled_text(&row()), "0.0004 / 0.001");
        let mut spend = row();
        spend.size = None;
        spend.spend = Some(d("25"));
        spend.filled = d("0");
        assert_eq!(filled_text(&spend), "0 / 25 (spend)");
    }

    /// What `width` measures in `locale`.
    fn in_locale(locale: crate::i18n::Locale, width: impl Fn(&egui::Ui) -> f32) -> f32 {
        let ctx = egui::Context::default();
        let mut measured = 0.0;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            measured = crate::i18n::with_locale(locale, || width(ui));
        });
        output.textures_delta.clear();
        measured
    }

    #[test]
    fn the_side_column_holds_its_tags_in_every_language() {
        use crate::i18n::Locale;
        assert_eq!(
            in_locale(Locale::ENGLISH, side_column_width),
            64.0,
            "English keeps its width"
        );
        assert!(
            in_locale(Locale::PSEUDO, side_column_width) > 64.0,
            "the longer tags widen the column"
        );
    }

    #[test]
    fn the_status_column_holds_the_widest_badge_in_every_language() {
        use crate::i18n::Locale;
        // Every status a venue's order can have: the longest are "Partly filled" and "Cancelling".
        let labels = [
            "Pending",
            "Open",
            "Partly filled",
            "Filled",
            "Cancelling",
            "Cancelled",
            "Rejected",
            "Expired",
        ];
        let rows = |locale: Locale| {
            crate::i18n::with_locale(locale, || {
                labels
                    .map(|label| {
                        let mut row = row();
                        row.status.label = crate::i18n::translate("order status", label);
                        row
                    })
                    .to_vec()
            })
        };
        let english = rows(Locale::ENGLISH);
        let pseudo = rows(Locale::PSEUDO);
        assert_eq!(
            in_locale(Locale::ENGLISH, |ui| status_column_width(
                ui, 120.0, &english
            )),
            120.0,
            "English keeps its width"
        );
        let widened = in_locale(Locale::PSEUDO, |ui| status_column_width(ui, 120.0, &pseudo));
        assert!(
            widened > 140.0,
            "{widened}: \"Partly filled\" fits its column"
        );
    }
}
