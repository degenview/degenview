//! The order form: side, type, prices, size, options, the summary and the button.

use std::time::{Duration, Instant};

use degenview_brokers::{Decimal, MarketRef, Side};
use eframe::egui::{self, vec2};

use super::state::{SheetState, Stage};
use super::{open_orders, review, status};
use crate::model::broker::ticket::{self, PREVIEW_DEBOUNCE, parse_decimal};
use crate::trading::backend::TradingBackend;
use crate::trading::confirm;
use crate::trading::draft::{DraftCheck, DraftField, floor_to_increment};
use crate::trading::model::{BackendState, MarketRules, OrderClass, VenueInfo, VenueKind};
use crate::ui::icons::{self, Icon};
use crate::ui::trade::fields::{self, Edit, NumberInput};
use crate::ui::trade::{figures, notice};
use crate::ui::{kit, paper, theme};

/// Text for a price or size typed back into a field: no trailing zeros.
fn plain(value: Decimal) -> String {
    value.normalize().to_string()
}

/// What the form is drawn for: the venue and its backend, the market and what the venue says
/// about trading it, and the two things the sheet's caller decides.
#[derive(Clone, Copy)]
pub(super) struct FormContext<'a> {
    pub info: &'a VenueInfo,
    pub backend: &'a dyn TradingBackend,
    pub bstate: &'a BackendState,
    pub market: &'a MarketRef,
    pub rules: &'a MarketRules,
    /// A replay is running: no order may be started from its prices.
    pub replay: bool,
    /// The user asked to confirm resting (limit and stop) orders too.
    pub confirm_resting: bool,
}

/// Draws the form for the order in `state`, and moves the sheet on: the venue is asked about the
/// order, the button sends or opens the review, and answers to sends and cancels are taken.
pub(super) fn show(ui: &mut egui::Ui, state: &mut SheetState, context: &FormContext) {
    let FormContext {
        info,
        backend,
        bstate,
        market,
        rules,
        replay,
        confirm_resting,
    } = *context;
    let now = Instant::now();
    state.draft.fit_to(&rules.caps);
    if state.snap_prices {
        state.snap_prices = false;
        for text in [&mut state.draft.price_text, &mut state.draft.stop_text] {
            if let Some(price) = parse_decimal(text).ok().flatten() {
                *text = plain(figures::snap_price(price, rules.price_increment));
            }
        }
    }
    let quote = backend.quote(market);
    if !state.prefilled
        && let Some(reference) = quote
            .as_ref()
            .and_then(|quote| quote.reference(state.draft.side))
    {
        if state.draft.price_text.is_empty() {
            state.draft.price_text = plain(floor_to_increment(reference, rules.price_increment));
        }
        state.prefilled = true;
    }
    let paused = match bstate {
        BackendState::Paused(text) => Some(text.as_str()),
        _ => None,
    };
    if let Some(text) = paused {
        status::note(ui, text, false);
        ui.add_space(8.0);
    }

    // Buy or sell, then what kind of order.
    let before_side = state.draft.side;
    fields::side_tabs(ui, &mut state.draft.side);
    if state.draft.side != before_side {
        state.stage = Stage::Editing;
    }
    ui.add_space(12.0);
    let side_tint = match state.draft.side {
        Side::Buy => theme::UP,
        Side::Sell => theme::DOWN,
    };
    let classes: Vec<(OrderClass, &str)> = rules
        .caps
        .order_classes
        .iter()
        .map(|class| (*class, class.label()))
        .collect();
    fields::class_tiles(ui, &classes, &mut state.draft.class, side_tint);

    let check = backend.check(market, &state.draft);
    ui.add_space(14.0);

    let draft_class = state.draft.class;
    let side = state.draft.side;
    let quote_unit = rules.quote_asset.as_str();
    let start = quote.as_ref().and_then(|quote| quote.reference(side));
    // The first price row says where the market is: the last price, else what an order meets.
    let mut price_hint = quote.as_ref().and_then(|quote| {
        let increment = Some(rules.price_increment);
        match (quote.last, quote.reference(side)) {
            (Some(last), _) => Some(format!("Last {}", figures::price_label(last, increment))),
            (None, Some(reference)) => Some(format!(
                "{} {}",
                if side == Side::Buy { "Ask" } else { "Bid" },
                figures::price_label(reference, increment)
            )),
            _ => None,
        }
    });

    if state.draft.uses_stop_price() {
        fields::label_row(ui, "Stop price", price_hint.take().as_deref());
        number(
            ui,
            &mut state.draft.stop_text,
            &NumberInput {
                scope: "stop",
                placeholder: "0.00",
                unit: quote_unit,
                step: Some(rules.price_increment),
                start,
            },
            &check,
            DraftField::StopPrice,
        );
        ui.add_space(12.0);
    }
    if state.draft.uses_limit_price() {
        fields::label_row(ui, "Limit price", price_hint.take().as_deref());
        number(
            ui,
            &mut state.draft.price_text,
            &NumberInput {
                scope: "price",
                placeholder: "0.00",
                unit: quote_unit,
                step: Some(rules.price_increment),
                start,
            },
            &check,
            DraftField::Price,
        );
        ui.add_space(12.0);
    }

    let spends = state.draft.spend_in_quote;
    let (size_label, size_unit) = if spends {
        ("Spend", quote_unit)
    } else {
        ("Amount", rules.base_asset.as_str())
    };
    // The most the account could put into this order, as far as it is known.
    let most = if spends {
        None
    } else {
        let at = if state.draft.uses_limit_price() {
            parse_decimal(&state.draft.price_text).ok().flatten()
        } else {
            None
        };
        backend.max_size(market, side, at).map(|most| {
            format!(
                "≈ {} {}",
                ticket::display(floor_to_increment(most, rules.size_increment)),
                rules.base_asset
            )
        })
    };
    fields::label_row(ui, size_label, most.as_deref());
    let step = if spends {
        None
    } else {
        Some(rules.size_increment)
    };
    let edit = number(
        ui,
        &mut state.draft.size_text,
        &NumberInput {
            scope: "size",
            placeholder: "0.00",
            unit: size_unit,
            step,
            start: Some(rules.min_size),
        },
        &check,
        DraftField::Size,
    );
    if edit != Edit::None {
        state.notice = None;
    }
    ui.add_space(8.0);
    presets(ui, state, context);
    ui.add_space(14.0);

    if rules.caps.quote_sized_market && side == Side::Buy && draft_class == OrderClass::Market {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            kit::switch(ui, &mut state.draft.spend_in_quote, "Spend an amount");
            ui.label(kit::text(
                format!("Spend an amount of {quote_unit}"),
                kit::size::BODY,
                kit::REGULAR,
                theme::text_primary(),
            ));
        });
        ui.add_space(14.0);
    }

    let (venue_fee, venue_total) = {
        let preview = state.preview.result();
        (
            preview.and_then(|preview| preview.fee),
            preview.and_then(|preview| preview.total),
        )
    };
    summary(ui, side, quote_unit, &check, venue_fee, venue_total);
    ui.add_space(14.0);

    options(ui, state, rules, &check);

    // Ask the venue what it would do, once the order is complete and has settled.
    // The account and connection this order would go to, read once and used for the question, the
    // key and the review: a preview is for one account, and a different one starts over.
    let binding = backend.binding();
    let key = format!(
        "{:?}|{:?}|{}|{}",
        info.venue,
        binding,
        market.symbol,
        state.draft.key()
    );
    let debounce = if info.kind == VenueKind::Simulated {
        Duration::ZERO
    } else {
        PREVIEW_DEBOUNCE
    };
    let ready = bstate.is_ready();
    let waiting = {
        let draft = &state.draft;
        state
            .preview
            .sync(&key, check.is_complete() && ready, now, debounce, || {
                backend.preview(market, draft, &binding)
            })
    };
    if waiting {
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
    let gate = state.preview.gate(now);
    status::line(ui, state, &check, &gate, info, now);

    // The button.
    ui.add_space(12.0);
    let side_word = state.draft.side_label();
    let label = format!("{side_word} {}", rules.base_asset);
    let tint = match side {
        Side::Buy => theme::UP,
        Side::Sell => theme::DOWN,
    };
    let sending = matches!(state.stage, Stage::Sending(_));
    let can_send = check.is_complete()
        && gate.allows_review()
        && ready
        && paused.is_none()
        && !replay
        && matches!(state.stage, Stage::Editing);
    let width = ui.available_width();
    let response = kit::cta_button(
        ui,
        &label,
        fields::muted(tint),
        fields::on_tint(tint),
        can_send,
        sending,
        width,
    );
    #[cfg(test)]
    {
        state.button = Some(response.rect);
        state.button_enabled = can_send;
    }
    if let Some(reason) = status::why_not(&check, &gate, paused, replay)
        && !can_send
        && !sending
    {
        response.clone().on_hover_text(reason);
    }
    if response.clicked() && can_send {
        if confirm::needs_review(info.kind, state.draft.class, confirm_resting) {
            state.stage = Stage::Confirming(binding.clone());
        } else {
            state.stage = Stage::Sending(backend.submit(market, &state.draft, &binding));
        }
    }

    available_line(ui, backend, market, side);
    status::notice(ui, state);

    state.expire_review(backend, &gate);
    review::show(ui, state, context, &check);
    state.take_send_result();
    state.poll_cancels();

    ui.add_space(16.0);
    notice::divider(ui);
    ui.add_space(8.0);
    open_orders::show(ui, state, backend);
}

/// A number field with the check's complaint about `field` under it.
fn number(
    ui: &mut egui::Ui,
    text: &mut String,
    input: &NumberInput,
    check: &DraftCheck,
    field: DraftField,
) -> Edit {
    let error = check.issue_for(field).map(str::to_owned);
    let edit = fields::number_field(ui, text, input, error.is_some());
    paper::form::field_footer(ui, error.as_ref(), None);
    edit
}

/// 10%, 25%, 50%, 75% and 100% of what the account can afford.
fn presets(ui: &mut egui::Ui, state: &mut SheetState, context: &FormContext) {
    let FormContext {
        backend,
        market,
        rules,
        ..
    } = *context;
    let chosen = fields::percent_row(ui);
    let Some(percent) = chosen else { return };
    let side = state.draft.side;
    let fraction = Decimal::from(percent) / Decimal::from(100);
    if state.draft.spend_in_quote {
        match backend.available(market, side) {
            Some(funds) if funds.value > Decimal::ZERO => {
                state.draft.size_text = plain((funds.value * fraction).round_dp(2));
            }
            _ => state.say("There is nothing to spend.", true),
        }
        return;
    }
    let price = if state.draft.uses_limit_price() {
        parse_decimal(&state.draft.price_text).ok().flatten()
    } else {
        None
    };
    match backend.max_size(market, side, price) {
        Some(most) => {
            let size = floor_to_increment(most * fraction, rules.size_increment);
            if size >= rules.min_size && size > Decimal::ZERO {
                state.draft.size_text = plain(size);
            } else {
                state.say("That is below the smallest order this market takes.", true);
            }
        }
        None => state.say("The account cannot afford the smallest order here.", true),
    }
}

/// What the order comes to: its value, the fee (with its share), the total.
fn summary(
    ui: &mut egui::Ui,
    side: Side,
    unit: &str,
    check: &DraftCheck,
    venue_fee: Option<Decimal>,
    venue_total: Option<Decimal>,
) {
    let display = ticket::display;
    let amount = |value: Option<Decimal>| {
        format!("{} {unit}", value.map_or_else(|| "—".to_owned(), display))
    };
    let value = check.estimate.notional;
    let fee = venue_fee.or(check.estimate.fee);
    let mut rows = vec![fields::SummaryRow {
        label: "Order value".to_owned(),
        value: amount(value),
        strong: false,
        info: None,
    }];
    rows.push(fields::SummaryRow {
        label: match value.zip(fee).and_then(|(value, fee)| figures::fee_percent(fee, value)) {
            Some(percent) => format!("Trading fee ({percent}%)"),
            None => "Trading fee".to_owned(),
        },
        value: amount(fee),
        strong: false,
        info: Some(
            "What trading this order is expected to cost. The venue's own check replaces the estimate once it has run.",
        ),
    });
    // Margin only says something when it is not simply the order's value (leverage).
    if let Some(margin) = check.estimate.margin
        && value != Some(margin)
    {
        rows.push(fields::SummaryRow {
            label: "Margin".to_owned(),
            value: amount(Some(margin)),
            strong: false,
            info: None,
        });
    }
    rows.push(fields::SummaryRow {
        label: "Total".to_owned(),
        value: amount(figures::order_total(side, value, fee, venue_total)),
        strong: true,
        info: None,
    });
    fields::summary_card(ui, &rows);
}

/// The venue-specific switches (post only, reduce only, take profit and stop loss) and the time in
/// force: only what this venue can do.
fn options(ui: &mut egui::Ui, state: &mut SheetState, rules: &MarketRules, check: &DraftCheck) {
    let caps = &rules.caps;
    if caps.time_in_force.len() > 1 && state.draft.uses_limit_price() {
        fields::label_row(ui, "Time in force", None);
        let labels: Vec<String> = caps
            .time_in_force
            .iter()
            .map(|tif| tif.label().to_owned())
            .collect();
        let selected = caps
            .time_in_force
            .iter()
            .position(|tif| *tif == state.draft.tif)
            .unwrap_or(0);
        let look = paper::form::DropdownLook {
            leading: Some(Icon::Clock),
            height: fields::FIELD_HEIGHT,
            chevron: Icon::ChevronDown,
        };
        if let Some(index) = paper::form::dropdown_field_with(ui, &labels, selected, true, look) {
            state.draft.tif = caps.time_in_force[index];
        }
        ui.add_space(14.0);
    }
    let mut switches: Vec<(&str, &mut bool)> = Vec::new();
    if caps.post_only && state.draft.class == OrderClass::Limit {
        switches.push(("Post only", &mut state.draft.post_only));
    }
    if caps.reduce_only {
        switches.push(("Reduce only", &mut state.draft.reduce_only));
    }
    if caps.brackets {
        switches.push(("Take profit / stop loss", &mut state.draft.bracket));
    }
    for (label, value) in switches {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            kit::switch(ui, value, label);
            ui.label(kit::text(
                label,
                kit::size::BODY,
                kit::REGULAR,
                theme::text_primary(),
            ));
        });
        ui.add_space(10.0);
    }
    if caps.brackets && state.draft.bracket {
        let unit = rules.quote_asset.as_str();
        ui.add_space(2.0);
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.columns(2, |columns| {
                fields::labelled_icon(&mut columns[0], Icon::Target, theme::UP, "Take profit");
                number(
                    &mut columns[0],
                    &mut state.draft.take_profit_text,
                    &NumberInput {
                        scope: "take-profit",
                        placeholder: "Optional",
                        unit,
                        step: None,
                        start: None,
                    },
                    check,
                    DraftField::TakeProfit,
                );
                fields::labelled_icon(&mut columns[1], Icon::Shield, theme::DOWN, "Stop loss");
                number(
                    &mut columns[1],
                    &mut state.draft.stop_loss_text,
                    &NumberInput {
                        scope: "stop-loss",
                        placeholder: "Optional",
                        unit,
                        step: None,
                        start: None,
                    },
                    check,
                    DraftField::StopLoss,
                );
            });
        });
        ui.add_space(8.0);
    }
}

fn available_line(ui: &mut egui::Ui, backend: &dyn TradingBackend, market: &MarketRef, side: Side) {
    let Some(funds) = backend.available(market, side) else {
        return;
    };
    ui.add_space(12.0);
    let label = kit::galley(
        ui,
        "Available",
        kit::size::CAPTION + 1.0,
        kit::REGULAR,
        theme::text_secondary(),
        f32::INFINITY,
    );
    let value = kit::galley(
        ui,
        &format!("{} {}", ticket::display(funds.value), funds.asset),
        kit::size::CAPTION + 1.0,
        kit::SEMIBOLD,
        theme::text_primary(),
        f32::INFINITY,
    );
    const ICON: f32 = 16.0;
    let total = ICON + 8.0 + label.size().x + 6.0 + value.size().x;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), egui::Sense::hover());
    let mut x = rect.center().x - total / 2.0;
    icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(
            egui::pos2(x + ICON / 2.0, rect.center().y),
            vec2(ICON, ICON),
        ),
        Icon::Banknote,
        theme::text_secondary(),
    );
    x += ICON + 8.0;
    let label_width = label.size().x;
    ui.painter().galley(
        egui::pos2(x, rect.center().y - label.size().y / 2.0),
        label,
        theme::text_secondary(),
    );
    x += label_width + 6.0;
    ui.painter().galley(
        egui::pos2(x, rect.center().y - value.size().y / 2.0),
        value,
        theme::text_primary(),
    );
}
