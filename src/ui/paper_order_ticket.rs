//! The paper order ticket: opened from a chart's BUY / SELL buttons. A pure view over a
//! `TicketDraft` (all the parsing, validation and figures live in `model::paper_ticket`) that
//! returns `PaperTicketAction`s; the host owns the store, refreshes the market figures each frame
//! and reports back whether the submit worked.

// Transient: the paper-trading host opens the ticket once it is wired.
#![allow(dead_code)]

use crate::model::paper::{PaperCurrency, PaperOrderRequest, PaperOrderSide, PaperOrderType};
use crate::model::paper_format as fmt;
use crate::model::paper_ticket::{self as ticket, TicketDraft, TicketField, decimal_text};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, TabItem};
use crate::ui::paper_form::{field_footer, field_label, text_field};
use crate::ui::paper_header;
use crate::ui::paper_tables::side_tint;
use crate::ui::sheet;
use crate::ui::theme;
use eframe::egui::{self, Color32, Id, Vec2};

pub const WIDTH: f32 = 460.0;

/// The quick-fill shortcuts under the quantity: a share of the available funds.
pub const QUICK_FILLS: [(f64, &str); 4] =
    [(0.25, "25%"), (0.5, "50%"), (0.75, "75%"), (1.0, "Max")];

pub struct PaperTicketState {
    pub draft: TicketDraft,
    pub shows_protection: bool,
    /// True between pressing Submit and the host calling `finish`.
    pub submitting: bool,
}

impl PaperTicketState {
    pub fn new(draft: TicketDraft) -> Self {
        Self {
            draft,
            shows_protection: false,
            submitting: false,
        }
    }

    /// The host's answer to a `Submit`: a success closes the sheet (the host drops the state), a
    /// failure re-enables the button and the host shows the error through `PaperTicketInput`.
    pub fn finish(&mut self) {
        self.submitting = false;
    }

    /// Fresh market figures for the open ticket; typed text is untouched.
    pub fn refresh_market(
        &mut self,
        bid: Option<f64>,
        ask: Option<f64>,
        last: Option<f64>,
        available_funds: Option<f64>,
        existing_signed_quantity: f64,
    ) {
        self.draft.bid = bid;
        self.draft.ask = ask;
        if last.is_some() {
            self.draft.last = last;
        }
        self.draft.available_funds = available_funds;
        self.draft.existing_signed_quantity = existing_signed_quantity;
    }
}

pub struct PaperTicketInput<'a> {
    pub account_id: Option<u64>,
    pub currency: PaperCurrency,
    /// The store's last error, shown under the form as "Order not placed".
    pub error: Option<&'a str>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PaperTicketAction {
    Submit(Box<PaperOrderRequest>),
    Cancel,
}

// MARK: Pure figures

/// One cell of the quote strip across the top.
#[derive(Clone, Debug, PartialEq)]
pub struct QuoteCell {
    pub title: &'static str,
    pub value: String,
    /// The side whose colour the cell takes (the price that side would trade at).
    pub highlight: Option<PaperOrderSide>,
}

const NO_VALUE: &str = "—";

/// Last is always shown; Bid, Ask and Spread only for markets whose stream supplies them.
pub fn quote_cells(draft: &TicketDraft) -> Vec<QuoteCell> {
    let price = |value: Option<f64>| {
        value.map_or_else(
            || NO_VALUE.to_owned(),
            |value| fmt::price(value, &draft.instrument),
        )
    };
    let mut cells = Vec::new();
    if draft.has_book() {
        cells.push(QuoteCell {
            title: "Bid",
            value: price(draft.bid),
            highlight: (draft.side == PaperOrderSide::Sell).then_some(PaperOrderSide::Sell),
        });
    }
    cells.push(QuoteCell {
        title: "Last",
        value: price(draft.last),
        highlight: None,
    });
    if draft.has_book() {
        cells.push(QuoteCell {
            title: "Ask",
            value: price(draft.ask),
            highlight: (draft.side == PaperOrderSide::Buy).then_some(PaperOrderSide::Buy),
        });
        if let Some(spread) = draft.spread() {
            cells.push(QuoteCell {
                title: "Spread",
                value: price(Some(spread)),
                highlight: None,
            });
        }
    }
    cells
}

pub const NO_BOOK_NOTE: &str =
    "Best bid and ask aren't available for this market, so orders fill at the last price.";

/// One line of the summary card.
#[derive(Clone, Debug, PartialEq)]
pub struct SummaryRow {
    pub title: &'static str,
    pub value: String,
    /// Drawn red: the order needs more than the account has free.
    pub alert: bool,
}

pub fn summary_rows(draft: &TicketDraft, currency: PaperCurrency) -> Vec<SummaryRow> {
    let money = |value: Option<f64>| {
        value.map_or_else(|| NO_VALUE.to_owned(), |value| fmt::money(value, currency))
    };
    let exceeds = draft.exceeds_funds();
    let mut rows = vec![
        SummaryRow {
            title: "Order value",
            value: money(draft.notional()),
            alert: false,
        },
        SummaryRow {
            title: "Required margin",
            value: money(draft.required_margin()),
            alert: exceeds,
        },
    ];
    if let Some(fee) = draft.estimated_fee() {
        rows.push(SummaryRow {
            title: "Estimated fee",
            value: money(Some(fee)),
            alert: false,
        });
    }
    rows.push(SummaryRow {
        title: "Available funds",
        value: money(draft.available_funds),
        alert: exceeds,
    });
    rows
}

pub fn shortfall_text(draft: &TicketDraft, currency: PaperCurrency) -> Option<String> {
    draft.shortfall().map(|shortfall| {
        format!(
            "Short by {}. Lower the quantity or raise leverage.",
            fmt::money(shortfall, currency)
        )
    })
}

/// The label for an order type's tab.
pub fn type_title(kind: PaperOrderType) -> &'static str {
    kind.label()
}

pub fn stop_label(kind: PaperOrderType) -> &'static str {
    if kind == PaperOrderType::StopLimit {
        "Stop trigger"
    } else {
        "Stop price"
    }
}

pub const MARKET_HINT: &str = "Fills immediately at the best available price.";

/// Sets the quantity to `fraction` of the funds, when that is possible. Returns whether it did.
pub fn apply_quick_fill(draft: &mut TicketDraft, fraction: f64) -> bool {
    match draft.quantity_for_funds_fraction(fraction) {
        Some(quantity) => {
            draft.quantity_text = decimal_text(quantity, draft.comma_locale);
            true
        }
        None => false,
    }
}

/// Fills a price field with the current market price. Returns whether there was one.
pub fn fill_market_price(draft: &mut TicketDraft, field: TicketField) -> bool {
    let Some(price) = draft.market_price() else {
        return false;
    };
    let text = decimal_text(price, draft.comma_locale);
    match field {
        TicketField::LimitPrice => draft.limit_price_text = text,
        TicketField::StopPrice => draft.stop_price_text = text,
        _ => return false,
    }
    true
}

// MARK: View

/// Space between the ticket's sections.
const SECTION_GAP: f32 = 16.0;

pub fn show(
    ctx: &egui::Context,
    state: &mut PaperTicketState,
    input: &PaperTicketInput,
) -> Vec<PaperTicketAction> {
    let mut actions = Vec::new();
    sheet::show(ctx, Id::new("paper-order-ticket"), WIDTH, |ui| {
        egui::ScrollArea::vertical()
            .max_height((ctx.content_rect().height() - 96.0).clamp(240.0, 760.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.add_space(24.0);
                sheet::inset(ui, |ui| form(ui, state, input));
                ui.add_space(24.0);
            });
        footer(ui, state, input, &mut actions);
    });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        actions.push(PaperTicketAction::Cancel);
    }
    actions
}

fn form(ui: &mut egui::Ui, state: &mut PaperTicketState, input: &PaperTicketInput) {
    ui.spacing_mut().item_spacing.y = 0.0;
    let instrument = state.draft.instrument.clone();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let (icon, _) = ui.allocate_exact_size(Vec2::splat(46.0), egui::Sense::hover());
        let artwork = egui::Rect::from_center_size(icon.center(), Vec2::splat(40.0));
        kit::chart_icon(ui, artwork, &instrument.symbol, instrument.source, true);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.add_space(3.0);
            ui.label(kit::text(
                &instrument.display_name,
                kit::size::TITLE2 + 1.0,
                kit::SEMIBOLD,
                theme::text_primary(),
            ));
            ui.label(kit::text(
                format!("{} · Paper order", instrument.source.display_name()),
                kit::size::SUBHEADLINE + 1.0,
                kit::REGULAR,
                theme::text_secondary(),
            ));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            paper_header::badge(ui)
        });
    });
    ui.add_space(SECTION_GAP);

    quote_strip(ui, &state.draft);
    ui.add_space(SECTION_GAP);
    side_selector(ui, &mut state.draft.side);
    ui.add_space(SECTION_GAP);
    kit::tab_bar_compact(
        ui,
        Id::new("paper-order-type-tabs"),
        &[
            type_tab(PaperOrderType::Market, Icon::Bolt),
            type_tab(PaperOrderType::Limit, Icon::ArrowDown),
            type_tab(PaperOrderType::Stop, Icon::XmarkOctagonFill),
            type_tab(PaperOrderType::StopLimit, Icon::Diamond),
        ],
        &mut state.draft.kind,
    );
    ui.add_space(SECTION_GAP);

    let issues = state.draft.issues();
    quantity_field(ui, &mut state.draft, &issues);
    if state.draft.kind != PaperOrderType::Market {
        ui.add_space(SECTION_GAP);
        price_fields(ui, &mut state.draft, &issues);
    }
    ui.add_space(SECTION_GAP);
    protection(ui, state, &issues);
    // No time-in-force control: the engine does not expire a Day order, so offering one would
    // promise what it does not do. Every ticket places a good-til-canceled order.
    ui.add_space(SECTION_GAP);
    summary_card(ui, &state.draft, input.currency);
    if let Some(error) = input.error {
        ui.add_space(SECTION_GAP);
        kit::notice_card(
            ui,
            Icon::TriangleFill,
            theme::DOWN,
            "Order not placed",
            Some(error),
            None,
        );
    }
}

fn type_tab(kind: PaperOrderType, icon: Icon) -> TabItem<PaperOrderType> {
    TabItem {
        value: kind,
        title: type_title(kind),
        icon,
        count: 0,
    }
}

/// The raised card the quote strip and the summary sit in.
fn card(ui: &mut egui::Ui, margin: egui::Margin, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(theme::quaternary(0.4))
        .stroke(egui::Stroke::new(
            1.0,
            theme::with_alpha(theme::separator(), 0.4),
        ))
        .corner_radius(10.0)
        .inner_margin(margin)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui);
        });
}

/// `PaperQuoteStrip`: Last always, Bid / Ask / Spread when the market streams a book, the side
/// the order would trade at lit in its colour.
fn quote_strip(ui: &mut egui::Ui, draft: &TicketDraft) {
    let cells = quote_cells(draft);
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 50.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 10.0, theme::quaternary(0.4));
    painter.rect_stroke(
        rect,
        10.0,
        egui::Stroke::new(1.0, theme::with_alpha(theme::separator(), 0.4)),
        egui::StrokeKind::Inside,
    );
    let width = rect.width() / cells.len().max(1) as f32;
    for (index, cell) in cells.iter().enumerate() {
        let centre_x = rect.left() + width * (index as f32 + 0.5);
        if index > 0 {
            painter.vline(
                rect.left() + width * index as f32,
                egui::Rangef::new(rect.center().y - 12.0, rect.center().y + 12.0),
                egui::Stroke::new(1.0, theme::separator()),
            );
        }
        let color = match cell.highlight {
            Some(side) => side_tint(side),
            None if cell.value == NO_VALUE => theme::text_secondary(),
            None => theme::text_primary(),
        };
        let title = kit::galley(
            ui,
            cell.title,
            kit::size::CAPTION,
            kit::REGULAR,
            theme::text_secondary(),
            width,
        );
        let value = kit::galley(ui, &cell.value, 13.0, kit::MEDIUM, color, width - 8.0);
        let gap = 3.0;
        let top = rect.center().y - (title.size().y + gap + value.size().y) / 2.0;
        painter.galley(
            egui::pos2(centre_x - title.size().x / 2.0, top),
            title.clone(),
            theme::text_secondary(),
        );
        painter.galley(
            egui::pos2(centre_x - value.size().x / 2.0, top + title.size().y + gap),
            value,
            color,
        );
    }
    if !draft.has_book() {
        ui.add_space(6.0);
        ui.add(
            egui::Label::new(kit::text(
                NO_BOOK_NOTE,
                kit::size::CAPTION + 0.5,
                kit::REGULAR,
                theme::text_tertiary(),
            ))
            .wrap(),
        );
    }
}

/// `PaperSideSelector`: Buy and Sell as two wide buttons, the chosen one filled with its colour.
fn side_selector(ui: &mut egui::Ui, side: &mut PaperOrderSide) {
    let (track, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(track, 11.0, theme::quaternary(0.6));
    ui.painter().rect_stroke(
        track,
        11.0,
        egui::Stroke::new(1.0, kit::hairline()),
        egui::StrokeKind::Inside,
    );
    let inner = track.shrink(3.0);
    for (index, option) in [PaperOrderSide::Buy, PaperOrderSide::Sell]
        .into_iter()
        .enumerate()
    {
        let half = (inner.width() - 2.0) / 2.0;
        let rect = egui::Rect::from_min_size(
            egui::pos2(inner.left() + index as f32 * (half + 2.0), inner.top()),
            Vec2::new(half, inner.height()),
        );
        let response = ui
            .interact(
                rect,
                ui.make_persistent_id(("paper-side", index)),
                egui::Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let response = crate::ui::accessibility::label(response, option.label());
        if response.clicked() {
            *side = option;
        }
        let selected = *side == option;
        if selected {
            ui.painter()
                .rect_filled(rect, 8.0, side_tint(option).gamma_multiply(0.92));
        } else if response.hovered() {
            ui.painter().rect_filled(rect, 8.0, kit::primary(0.05));
        }
        let color = if selected {
            Color32::WHITE
        } else {
            theme::text_secondary()
        };
        let label = kit::galley(
            ui,
            option.label(),
            14.0,
            kit::SEMIBOLD,
            color,
            f32::INFINITY,
        );
        ui.painter()
            .galley(rect.center() - label.size() / 2.0, label, color);
    }
}

fn quantity_field(
    ui: &mut egui::Ui,
    draft: &mut TicketDraft,
    issues: &std::collections::HashMap<TicketField, String>,
) {
    field_label(ui, "Quantity");
    let error = issues.get(&TicketField::Quantity);
    let symbol = ticket::base_symbol(&draft.instrument);
    text_field(
        ui,
        "quantity",
        &mut draft.quantity_text,
        "0",
        &symbol,
        error.is_some(),
        None,
    );
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for (fraction, title) in QUICK_FILLS {
            let possible = draft.quantity_for_funds_fraction(fraction).is_some();
            if quick_fill(ui, title, possible) {
                apply_quick_fill(draft, fraction);
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(kit::text(
                "of available funds",
                kit::size::CAPTION + 1.0,
                kit::REGULAR,
                theme::text_tertiary(),
            ));
        });
    });
    let hint = (draft.kind == PaperOrderType::Market).then_some(MARKET_HINT);
    field_footer(ui, error, hint);
}

/// A "25%" capsule; true when it was clicked.
fn quick_fill(ui: &mut egui::Ui, title: &str, enabled: bool) -> bool {
    let color = if enabled {
        theme::text_secondary()
    } else {
        theme::with_alpha(theme::text_secondary(), 0.5)
    };
    let galley = kit::galley(
        ui,
        title,
        kit::size::CAPTION + 1.0,
        kit::SEMIBOLD,
        color,
        f32::INFINITY,
    );
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(galley.size().x + 18.0, galley.size().y + 9.0),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let response = crate::ui::accessibility::label(response, title);
    let fill = if response.is_pointer_button_down_on() && enabled {
        kit::primary(0.16)
    } else if response.hovered() && enabled {
        kit::primary(0.1)
    } else {
        kit::primary(0.06)
    };
    ui.painter().rect_filled(rect, rect.height() / 2.0, fill);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, color);
    enabled && response.clicked()
}

fn price_fields(
    ui: &mut egui::Ui,
    draft: &mut TicketDraft,
    issues: &std::collections::HashMap<TicketField, String>,
) {
    let currency = draft.instrument.quote_currency.code();
    let accessory = draft.market_price().is_some().then_some("Market");
    let both = draft.uses_stop_price() && draft.uses_limit_price();
    ui.columns(if both { 2 } else { 1 }, |columns| {
        let mut index = 0;
        if draft.uses_stop_price() {
            let column = &mut columns[index];
            index += 1;
            field_label(column, stop_label(draft.kind));
            let error = issues.get(&TicketField::StopPrice);
            if text_field(
                column,
                "stop",
                &mut draft.stop_price_text,
                "0",
                currency,
                error.is_some(),
                accessory,
            ) {
                fill_market_price(draft, TicketField::StopPrice);
            }
            field_footer(column, error, None);
        }
        if draft.uses_limit_price() {
            let column = &mut columns[index];
            field_label(column, "Limit price");
            let error = issues.get(&TicketField::LimitPrice);
            if text_field(
                column,
                "limit",
                &mut draft.limit_price_text,
                "0",
                currency,
                error.is_some(),
                accessory,
            ) {
                fill_market_price(draft, TicketField::LimitPrice);
            }
            field_footer(column, error, None);
        }
    });
}

fn protection(
    ui: &mut egui::Ui,
    state: &mut PaperTicketState,
    issues: &std::collections::HashMap<TicketField, String>,
) {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 20.0), egui::Sense::click());
    let response = crate::ui::accessibility::label(response, "Take profit and stop loss")
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        state.shows_protection = !state.shows_protection;
    }
    let centre = rect.center().y;
    icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(egui::pos2(rect.left() + 6.0, centre), Vec2::splat(10.0)),
        if state.shows_protection {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        },
        theme::text_secondary(),
    );
    let title = kit::galley(
        ui,
        "Take profit & stop loss",
        12.5,
        kit::MEDIUM,
        theme::text_primary(),
        f32::INFINITY,
    );
    let title_width = title.size().x;
    ui.painter().galley(
        egui::pos2(rect.left() + 20.0, centre - title.size().y / 2.0),
        title,
        theme::text_primary(),
    );
    let optional = kit::galley(
        ui,
        "Optional",
        kit::size::CAPTION + 1.0,
        kit::REGULAR,
        theme::text_tertiary(),
        f32::INFINITY,
    );
    ui.painter().galley(
        egui::pos2(
            rect.left() + 20.0 + title_width + 8.0,
            centre - optional.size().y / 2.0,
        ),
        optional,
        theme::text_tertiary(),
    );
    if !state.shows_protection {
        return;
    }
    ui.add_space(10.0);
    let currency = state.draft.instrument.quote_currency.code();
    ui.columns(2, |columns| {
        field_label(&mut columns[0], "Take profit");
        let error = issues.get(&TicketField::TakeProfit);
        text_field(
            &mut columns[0],
            "take-profit",
            &mut state.draft.take_profit_text,
            "Price",
            currency,
            error.is_some(),
            None,
        );
        field_footer(&mut columns[0], error, None);
        field_label(&mut columns[1], "Stop loss");
        let error = issues.get(&TicketField::StopLoss);
        text_field(
            &mut columns[1],
            "stop-loss",
            &mut state.draft.stop_loss_text,
            "Price",
            currency,
            error.is_some(),
            None,
        );
        field_footer(&mut columns[1], error, None);
    });
}

/// `PaperOrderSummaryCard`: what the order costs, and how it changes the open position.
fn summary_card(ui: &mut egui::Ui, draft: &TicketDraft, currency: PaperCurrency) {
    card(ui, egui::Margin::same(14), |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        for row in summary_rows(draft, currency) {
            summary_row(
                ui,
                row.title,
                &row.value,
                if row.alert {
                    theme::DOWN
                } else {
                    theme::text_primary()
                },
            );
        }
        if let Some(text) = shortfall_text(draft, currency) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                icons::inline(ui, Icon::TriangleFill, 11.0, theme::DOWN);
                ui.add(
                    egui::Label::new(kit::text(
                        text,
                        kit::size::CAPTION + 1.0,
                        kit::REGULAR,
                        theme::DOWN,
                    ))
                    .wrap(),
                );
            });
        }
        if let Some(note) = draft.position_note() {
            let (line, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), egui::Sense::hover());
            ui.painter()
                .rect_filled(line, 0.0, theme::with_alpha(theme::separator(), 0.6));
            summary_row(ui, "Position", &note, theme::text_primary());
        }
    });
}

fn summary_row(ui: &mut egui::Ui, title: &str, value: &str, color: Color32) {
    ui.horizontal(|ui| {
        ui.label(kit::text(
            title,
            12.5,
            kit::REGULAR,
            theme::text_secondary(),
        ));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(kit::text(value, 12.5, kit::MEDIUM, color)).truncate());
        });
    });
}

fn footer(
    ui: &mut egui::Ui,
    state: &mut PaperTicketState,
    input: &PaperTicketInput,
    actions: &mut Vec<PaperTicketAction>,
) {
    let (line, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(line, 0.0, theme::separator());
    sheet::inset(ui, |ui| {
        ui.add_space(14.0);
        let enabled = state.draft.can_submit() && !state.submitting && input.account_id.is_some();
        ui.horizontal(|ui| {
            if kit::large_button(ui, "Cancel").clicked() {
                actions.push(PaperTicketAction::Cancel);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let title = state.draft.submit_title();
                let clicked = kit::tinted_button(
                    ui,
                    &title,
                    side_tint(state.draft.side),
                    enabled,
                    state.submitting,
                    150.0,
                )
                .clicked();
                let entered = enabled && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (clicked || entered)
                    && let Some(account_id) = input.account_id
                    && let Some(request) = state.draft.request(account_id)
                {
                    state.submitting = true;
                    actions.push(PaperTicketAction::Submit(Box::new(request)));
                }
            });
        });
        ui.add_space(14.0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;

    fn draft() -> TicketDraft {
        let mut instrument = PaperInstrument::chart("BTCUSDT", "BTC/USDT", DataSource::Binance);
        instrument.tick_size = 0.01;
        instrument.minimum_quantity = 0.001;
        instrument.quantity_increment = 0.001;
        let mut draft = TicketDraft::new(instrument);
        draft.bid = Some(99.0);
        draft.ask = Some(101.0);
        draft.last = Some(100.0);
        draft.available_funds = Some(10_000.0);
        draft
    }

    use crate::model::paper::PaperInstrument;

    #[test]
    fn the_strip_shows_the_book_only_when_there_is_one() {
        let mut ticket = draft();
        let titles = |cells: Vec<QuoteCell>| cells.iter().map(|c| c.title).collect::<Vec<_>>();
        assert_eq!(
            titles(quote_cells(&ticket)),
            ["Bid", "Last", "Ask", "Spread"]
        );
        ticket.bid = None;
        assert_eq!(titles(quote_cells(&ticket)), ["Last"]);
        ticket.last = None;
        assert_eq!(quote_cells(&ticket)[0].value, "—");
    }

    #[test]
    fn the_price_the_side_trades_at_takes_its_colour() {
        let mut ticket = draft();
        let highlighted = |ticket: &TicketDraft| {
            quote_cells(ticket)
                .iter()
                .filter(|cell| cell.highlight.is_some())
                .map(|cell| cell.title)
                .collect::<Vec<_>>()
        };
        assert_eq!(highlighted(&ticket), ["Ask"]);
        ticket.side = PaperOrderSide::Sell;
        assert_eq!(highlighted(&ticket), ["Bid"]);
        let cells = quote_cells(&ticket);
        assert_eq!(cells[3].value, "2", "spread");
    }

    #[test]
    fn the_summary_turns_red_and_names_the_shortfall() {
        let mut ticket = draft();
        ticket.quantity_text = "1".into();
        let rows = summary_rows(&ticket, PaperCurrency::Usd);
        let titles: Vec<_> = rows.iter().map(|row| row.title).collect();
        assert_eq!(
            titles,
            ["Order value", "Required margin", "Available funds"]
        );
        assert_eq!(rows[0].value, "$101.00");
        assert!(rows.iter().all(|row| !row.alert));
        assert_eq!(shortfall_text(&ticket, PaperCurrency::Usd), None);

        ticket.available_funds = Some(60.0);
        let rows = summary_rows(&ticket, PaperCurrency::Usd);
        assert!(rows[1].alert && rows[2].alert && !rows[0].alert);
        assert_eq!(
            shortfall_text(&ticket, PaperCurrency::Usd).as_deref(),
            Some("Short by $41.00. Lower the quantity or raise leverage.")
        );

        ticket.quantity_text.clear();
        assert_eq!(summary_rows(&ticket, PaperCurrency::Usd)[0].value, "—");
    }

    #[test]
    fn a_fee_row_appears_only_with_a_commission() {
        use crate::model::paper::PaperCommissionConfiguration;
        let mut ticket = draft();
        ticket.quantity_text = "1".into();
        assert_eq!(summary_rows(&ticket, PaperCurrency::Usd).len(), 3);
        ticket.commission = PaperCommissionConfiguration::FixedPerOrder { value: 2.0 };
        let rows = summary_rows(&ticket, PaperCurrency::Usd);
        assert_eq!(rows[2].title, "Estimated fee");
        assert_eq!(rows[2].value, "$2.00");
    }

    #[test]
    fn quick_fills_and_market_buttons_write_the_fields() {
        let mut ticket = draft();
        assert!(apply_quick_fill(&mut ticket, 0.5));
        assert_eq!(ticket.quantity_text, "49.504");
        ticket.available_funds = None;
        assert!(!apply_quick_fill(&mut ticket, 1.0));
        assert_eq!(
            ticket.quantity_text, "49.504",
            "an impossible fill leaves the field"
        );

        assert!(fill_market_price(&mut ticket, TicketField::LimitPrice));
        assert_eq!(ticket.limit_price_text, "101");
        ticket.side = PaperOrderSide::Sell;
        assert!(fill_market_price(&mut ticket, TicketField::StopPrice));
        assert_eq!(ticket.stop_price_text, "99");
        assert!(!fill_market_price(&mut ticket, TicketField::Quantity));
        ticket.bid = None;
        ticket.ask = None;
        ticket.last = None;
        assert!(!fill_market_price(&mut ticket, TicketField::LimitPrice));
    }

    #[test]
    fn the_stop_field_is_named_for_its_role() {
        assert_eq!(stop_label(PaperOrderType::Stop), "Stop price");
        assert_eq!(stop_label(PaperOrderType::StopLimit), "Stop trigger");
    }

    #[test]
    fn refreshing_market_data_keeps_what_was_typed() {
        let mut state = PaperTicketState::new(draft());
        state.draft.quantity_text = "0.5".into();
        state.refresh_market(Some(98.0), Some(100.0), Some(99.0), Some(500.0), -1.0);
        assert_eq!(state.draft.quantity_text, "0.5");
        assert_eq!(state.draft.bid, Some(98.0));
        assert_eq!(state.draft.existing_signed_quantity, -1.0);
        state.refresh_market(None, None, None, None, 0.0);
        assert_eq!(
            state.draft.last,
            Some(99.0),
            "a missing tick keeps the last price"
        );
        state.submitting = true;
        state.finish();
        assert!(!state.submitting);
    }
}
