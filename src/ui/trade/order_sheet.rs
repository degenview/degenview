//! The order sheet: where an order is typed, checked with the venue, and sent, for any venue.
//!
//! It is a right-hand panel beside the chart. Everything it shows comes from the venue's
//! [`TradingBackend`]: which order types exist, how a size is counted, what the account can spend,
//! what the venue says an order would do. The sheet holds only what is typed and what the user is
//! in the middle of (reviewing, sending).

use std::time::{Duration, Instant};

use degenview_brokers::{Decimal, MarketRef, Side};
use eframe::egui::{self, Id, vec2};

use super::fields::{self, Edit};
use super::venue_pill::{self, PillAction};
use crate::model::broker_ticket::{PreviewGate, parse_decimal};
use crate::model::trading_venue::TradingVenue;
use crate::trading::backend::TradingBackend;
use crate::trading::confirm;
use crate::trading::draft::{
    CancelResult, DraftCheck, DraftField, SubmitResult, TradeDraft, floor_to_increment,
};
use crate::trading::model::{
    BackendState, MarketAccess, MarketRules, OrderClass, OrderRow, Unavailable, UnavailableKind,
    VenueInfo,
};
use crate::trading::preview::PreviewSlot;
use crate::trading::task::Task;
use crate::ui::icons::{self, Icon};
use crate::ui::{broker_badge, dialog, kit, paper_form, theme};

/// How long what is typed must stay the same before the venue is asked about it.
const DEBOUNCE: Duration = Duration::from_millis(450);
/// How long a message stays under the button.
const NOTICE_FOR: Duration = Duration::from_secs(10);

enum Stage {
    Editing,
    /// The review dialog is open.
    Confirming,
    Sending(Task<SubmitResult>),
}

pub struct SheetState {
    pub draft: TradeDraft,
    /// The market the typed prices were for; a different chart starts the prices over.
    market: Option<MarketRef>,
    venue: Option<TradingVenue>,
    prefilled: bool,
    preview: PreviewSlot,
    stage: Stage,
    notice: Option<(String, bool, Instant)>,
    cancels: Vec<Task<CancelResult>>,
    /// Where the Buy/Sell button was drawn last frame, for the tests that press it.
    #[cfg(test)]
    pub(crate) button: Option<egui::Rect>,
    #[cfg(test)]
    button_enabled: bool,
}

impl Default for SheetState {
    fn default() -> Self {
        Self {
            draft: TradeDraft::new(Side::Buy),
            market: None,
            venue: None,
            prefilled: false,
            preview: PreviewSlot::default(),
            stage: Stage::Editing,
            notice: None,
            cancels: Vec::new(),
            #[cfg(test)]
            button: None,
            #[cfg(test)]
            button_enabled: false,
        }
    }
}

impl SheetState {
    /// Starts an order from somewhere else (a header pill, a click on the chart): the side, and a
    /// price where there is one. What was typed for another order is replaced.
    pub fn start(&mut self, side: Side, class: Option<OrderClass>, price: Option<Decimal>) {
        let class = class.unwrap_or(self.draft.class);
        self.draft = match price {
            Some(price) => TradeDraft::at_price(side, class, price),
            None => {
                let mut draft = TradeDraft::new(side);
                draft.class = class;
                draft
            }
        };
        self.prefilled = price.is_some();
        self.stage = Stage::Editing;
    }

    /// Whether the button was pressable when it was last drawn.
    #[cfg(test)]
    pub(crate) fn button_enabled_for_test(&self) -> bool {
        self.button_enabled
    }

    /// The prices are typed, so the sheet need not fill one in from the market.
    #[cfg(test)]
    pub(crate) fn mark_prefilled_for_test(&mut self) {
        self.prefilled = true;
    }

    /// Whether the venue's preview failed (it could not be asked, or it errored).
    #[cfg(test)]
    pub(crate) fn preview_failed(&self) -> bool {
        matches!(self.preview.gate(Instant::now()), PreviewGate::Failed(_))
    }

    /// Asks the venue again (the "Try again" button).
    #[cfg(test)]
    pub(crate) fn retry_preview(&mut self) {
        self.preview.retry(Instant::now());
    }

    /// Whether the review dialog is open.
    #[cfg(test)]
    pub(crate) fn is_reviewing(&self) -> bool {
        matches!(self.stage, Stage::Confirming)
    }

    /// Whether an order is on its way to the venue.
    #[cfg(test)]
    pub(crate) fn is_sending(&self) -> bool {
        matches!(self.stage, Stage::Sending(_))
    }

    #[cfg(test)]
    pub(crate) fn notice_text(&self) -> Option<&str> {
        self.notice.as_ref().map(|(text, _, _)| text.as_str())
    }

    /// Closes the review dialog without sending (a replay began). True when one was open.
    pub fn close_review(&mut self) -> bool {
        let open = matches!(self.stage, Stage::Confirming);
        if open {
            self.stage = Stage::Editing;
        }
        open
    }

    fn say(&mut self, text: impl Into<String>, error: bool) {
        self.notice = Some((text.into(), error, Instant::now()));
    }
}

pub struct SheetInput<'a> {
    /// The venues the user enabled, for the pill's menu.
    pub enabled: &'a [VenueInfo],
    /// Every venue, for the list shown when none is enabled.
    pub all: &'a [VenueInfo],
    /// The tab's venue and its backend; none when nothing is enabled.
    pub current: Option<(&'a VenueInfo, &'a dyn TradingBackend)>,
    /// The focused chart's market, if it is one.
    pub market: Option<MarketRef>,
    /// The user asked to confirm resting (limit and stop) orders too.
    pub confirm_resting: bool,
    /// A replay is running: no order may be started from its prices.
    pub replay: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SheetAction {
    PickVenue(TradingVenue),
    /// "Add broker…": show the list of venues in Settings.
    AddBroker,
    /// A venue was chosen from the first-run list.
    Connect(TradingVenue),
    /// A venue needs the user in Settings (a rejected key).
    OpenSettings,
    /// The sheet's ✕.
    Close,
}

pub fn show(ui: &mut egui::Ui, state: &mut SheetState, input: &SheetInput) -> Vec<SheetAction> {
    let mut actions = Vec::new();
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    #[cfg(test)]
    {
        state.button = None;
        state.button_enabled = false;
    }
    let Some((info, backend)) = input.current else {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(0.0);
                let width = ui.available_width();
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(14, 0))
                    .show(ui, |ui| {
                        ui.set_width(width - 28.0);
                        if let Some(venue) = super::connect_list::show(ui, input.all) {
                            actions.push(SheetAction::Connect(venue));
                        }
                    });
            });
        return actions;
    };
    if info.kind.is_real_money() {
        broker_badge::rule(ui);
    }
    let bstate = backend.state();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let width = ui.available_width();
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(14, 12))
                .show(ui, |ui| {
                    ui.set_width(width - 28.0);
                    header(ui, info, &bstate, input, &mut actions);
                    ui.add_space(12.0);
                    body(ui, state, info, backend, &bstate, input, &mut actions);
                });
        });
    // Something is in flight, or a price is moving: look again soon.
    ui.ctx().request_repaint_after(Duration::from_secs(1));
    actions
}

fn header(
    ui: &mut egui::Ui,
    info: &VenueInfo,
    bstate: &BackendState,
    input: &SheetInput,
    actions: &mut Vec<SheetAction>,
) {
    ui.horizontal(|ui| {
        match venue_pill::show(ui, info, bstate, input.enabled, venue_pill::PillSize::Sheet) {
            Some(PillAction::Pick(venue)) => actions.push(SheetAction::PickVenue(venue)),
            Some(PillAction::AddBroker) => actions.push(SheetAction::AddBroker),
            None => {}
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icons::button(
                ui,
                Icon::Xmark,
                22.0,
                theme::text_secondary(),
                "Close the order sheet",
            )
            .clicked()
            {
                actions.push(SheetAction::Close);
            }
            if let Some(market) = &input.market {
                ui.add_space(4.0);
                ui.label(kit::text(
                    &market.symbol,
                    kit::size::BODY,
                    kit::SEMIBOLD,
                    theme::text_primary(),
                ));
            }
        });
    });
}

/// A message between the sheet's fields: a glyph and wrapped text on a faint tint with a hairline
/// edge. Red for what went wrong, orange for a warning, quiet for what is only worth knowing.
fn note(ui: &mut egui::Ui, text: &str, error: bool) {
    let tint = if error {
        kit::RED
    } else {
        theme::text_secondary()
    };
    egui::Frame::new()
        .fill(theme::with_alpha(tint, 0.08))
        .stroke(egui::Stroke::new(1.0, theme::with_alpha(tint, 0.22)))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                icons::inline(ui, Icon::InfoCircle, 13.0, tint);
                ui.add(
                    egui::Label::new(kit::text(
                        text,
                        kit::size::CAPTION + 1.0,
                        kit::REGULAR,
                        if error {
                            theme::text_primary()
                        } else {
                            theme::text_secondary()
                        },
                    ))
                    .wrap(),
                );
            });
        });
}

/// Why the chart's market cannot be traded here, as a card: a short title for the kind of reason
/// and the venue's own sentence beneath it.
fn unavailable_card(ui: &mut egui::Ui, kind: UnavailableKind, text: &str) {
    let (title, tint) = match kind {
        UnavailableKind::NotThisVenue => ("Can't trade this market here", theme::text_secondary()),
        UnavailableKind::Waiting => ("Getting ready", theme::text_secondary()),
        UnavailableKind::Blocked => ("Orders unavailable", kit::ORANGE),
    };
    kit::notice_card(ui, Icon::InfoCircle, tint, title, Some(text), None);
}

fn body(
    ui: &mut egui::Ui,
    state: &mut SheetState,
    info: &VenueInfo,
    backend: &dyn TradingBackend,
    bstate: &BackendState,
    input: &SheetInput,
    actions: &mut Vec<SheetAction>,
) {
    match bstate {
        BackendState::NotConnected => {
            if kit::notice_card(
                ui,
                Icon::ArrowLeftRight,
                theme::text_secondary(),
                &format!("{} is not connected", info.name),
                Some("Connect it in Settings to trade here."),
                Some("Open Settings"),
            ) {
                actions.push(SheetAction::OpenSettings);
            }
            return;
        }
        BackendState::Connecting => {
            kit::notice_card(
                ui,
                Icon::InfoCircle,
                theme::text_secondary(),
                &format!("Connecting to {}…", info.name),
                Some("Orders are available as soon as it is connected."),
                None,
            );
            return;
        }
        BackendState::NeedsAttention(text) => {
            if kit::notice_card(
                ui,
                Icon::ArrowLeftRight,
                kit::RED,
                &format!("{} needs attention", info.name),
                Some(text),
                Some("Open Settings"),
            ) {
                actions.push(SheetAction::OpenSettings);
            }
            return;
        }
        BackendState::Paused(_) | BackendState::Ready => {}
    }
    let Some(market) = input.market.clone() else {
        kit::notice_card(
            ui,
            Icon::InfoCircle,
            theme::text_secondary(),
            "No market in focus",
            Some("Focus a market chart: an order is for the chart's market."),
            None,
        );
        return;
    };
    if state.market.as_ref() != Some(&market) || state.venue != Some(info.venue) {
        // Another chart or another venue: prices typed for the old one mean nothing here.
        if state.market.is_some() {
            state.draft.price_text.clear();
            state.draft.stop_text.clear();
            state.draft.size_text.clear();
            state.draft.take_profit_text.clear();
            state.draft.stop_loss_text.clear();
        }
        state.market = Some(market.clone());
        state.venue = Some(info.venue);
        state.prefilled = false;
        state.stage = Stage::Editing;
    }
    let rules = match backend.market(&market) {
        MarketAccess::Ready(rules) => rules,
        MarketAccess::Unavailable(Unavailable { kind, text }) => {
            unavailable_card(ui, kind, &text);
            return;
        }
    };
    form(ui, state, info, backend, bstate, &market, &rules, input);
}

/// Text for a price or size typed back into a field: no trailing zeros.
fn plain(value: Decimal) -> String {
    value.normalize().to_string()
}

#[allow(clippy::too_many_arguments)]
fn form(
    ui: &mut egui::Ui,
    state: &mut SheetState,
    info: &VenueInfo,
    backend: &dyn TradingBackend,
    bstate: &BackendState,
    market: &MarketRef,
    rules: &MarketRules,
    input: &SheetInput,
) {
    let now = Instant::now();
    state.draft.fit_to(&rules.caps);
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
        note(ui, text, false);
        ui.add_space(8.0);
    }

    // Buy or sell, then what kind of order.
    let before_side = state.draft.side;
    fields::side_tabs(ui, &mut state.draft.side);
    if state.draft.side != before_side {
        state.stage = Stage::Editing;
    }
    ui.add_space(10.0);
    let classes: Vec<(OrderClass, &str)> = rules
        .caps
        .order_classes
        .iter()
        .map(|class| (*class, class.label()))
        .collect();
    let width = ui.available_width();
    kit::segmented_filling(
        ui,
        Id::new("trade-class"),
        &classes,
        &mut state.draft.class,
        width,
    );

    let check = backend.check(market, &state.draft);
    ui.add_space(10.0);
    if let Some(quote) = &quote {
        quote_strip(ui, quote, rules);
        ui.add_space(8.0);
    }

    let draft_class = state.draft.class;
    let side = state.draft.side;
    let quote_unit = rules.quote_asset.as_str();
    let start = quote.as_ref().and_then(|quote| quote.reference(side));

    if state.draft.uses_stop_price() {
        paper_form::field_label(ui, &format!("Stop price ({quote_unit})"));
        number(
            ui,
            "stop",
            &mut state.draft.stop_text,
            "0.00",
            quote_unit,
            &check,
            DraftField::StopPrice,
            Some(rules.price_increment),
            start,
        );
        ui.add_space(10.0);
    }
    if state.draft.uses_limit_price() {
        let label = if draft_class == OrderClass::StopLimit {
            format!("Limit price ({quote_unit})")
        } else {
            format!("Price ({quote_unit})")
        };
        paper_form::field_label(ui, &label);
        number(
            ui,
            "price",
            &mut state.draft.price_text,
            "0.00",
            quote_unit,
            &check,
            DraftField::Price,
            Some(rules.price_increment),
            start,
        );
        ui.add_space(10.0);
    }

    let spends = state.draft.spend_in_quote;
    let (size_label, size_unit) = if spends {
        (format!("Spend ({quote_unit})"), quote_unit)
    } else {
        (
            format!("Amount ({})", rules.base_asset),
            rules.base_asset.as_str(),
        )
    };
    paper_form::field_label(ui, &size_label);
    let step = if spends {
        None
    } else {
        Some(rules.size_increment)
    };
    let edit = number(
        ui,
        "size",
        &mut state.draft.size_text,
        "0.00",
        size_unit,
        &check,
        DraftField::Size,
        step,
        Some(rules.min_size),
    );
    if edit != Edit::None {
        state.notice = None;
    }
    ui.add_space(8.0);
    presets(ui, state, backend, market, rules);
    ui.add_space(10.0);

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
        ui.add_space(10.0);
    }

    paper_form::field_label(ui, &format!("Total ({quote_unit})"));
    let total = check
        .estimate
        .notional
        .map_or_else(|| "—".to_owned(), crate::model::broker_ticket::display);
    fields::figure_field(ui, &total, quote_unit);
    cost_lines(ui, &check, state.preview.result());
    ui.add_space(10.0);

    options(ui, state, rules, &check);

    // Ask the venue what it would do, once the order is complete and has settled.
    let key = format!("{:?}|{}|{}", info.venue, market.symbol, state.draft.key());
    let debounce = if info.kind == crate::trading::model::VenueKind::Simulated {
        Duration::ZERO
    } else {
        DEBOUNCE
    };
    let ready = bstate.is_ready();
    let waiting = {
        let draft = &state.draft;
        state
            .preview
            .sync(&key, check.is_complete() && ready, now, debounce, || {
                backend.preview(market, draft)
            })
    };
    if waiting {
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
    let gate = state.preview.gate(now);
    status_line(ui, state, &check, &gate, info, now);

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
        && !input.replay
        && matches!(state.stage, Stage::Editing);
    let width = ui.available_width();
    let response = kit::tinted_button(ui, &label, tint, can_send, sending, width);
    #[cfg(test)]
    {
        state.button = Some(response.rect);
        state.button_enabled = can_send;
    }
    if let Some(reason) = why_not(&check, &gate, paused, input.replay)
        && !can_send
        && !sending
    {
        response.clone().on_hover_text(reason);
    }
    if response.clicked() && can_send {
        if confirm::needs_review(info.kind, state.draft.class, input.confirm_resting) {
            state.stage = Stage::Confirming;
        } else {
            state.stage = Stage::Sending(backend.submit(market, &state.draft));
        }
    }

    available_line(ui, backend, market, side);
    notice(ui, state);

    review_dialog(ui, state, info, backend, market, rules, &check);
    sending_result(state, now);
    poll_cancels(state);

    ui.add_space(16.0);
    open_orders(ui, state, backend);
}

#[allow(clippy::too_many_arguments)]
fn number(
    ui: &mut egui::Ui,
    salt: &str,
    text: &mut String,
    placeholder: &str,
    unit: &str,
    check: &DraftCheck,
    field: DraftField,
    step: Option<Decimal>,
    start: Option<Decimal>,
) -> Edit {
    let error = check.issue_for(field).map(str::to_owned);
    let edit = fields::number_field(
        ui,
        salt,
        text,
        placeholder,
        unit,
        error.is_some(),
        step,
        start,
    );
    paper_form::field_footer(ui, error.as_ref(), None);
    edit
}

fn quote_strip(ui: &mut egui::Ui, quote: &crate::trading::model::QuoteView, rules: &MarketRules) {
    let text = match (quote.bid, quote.ask, quote.last) {
        (Some(bid), Some(ask), _) => format!(
            "Bid {}   ·   Ask {}",
            confirm::price_text(bid, rules.price_increment),
            confirm::price_text(ask, rules.price_increment)
        ),
        (_, _, Some(last)) => format!("Last {}", confirm::price_text(last, rules.price_increment)),
        _ => return,
    };
    ui.label(kit::text(
        &text,
        kit::size::CAPTION + 1.0,
        kit::REGULAR,
        theme::text_secondary(),
    ));
}

/// 10%, 25%, 50%, 75% and 100% of what the account can afford.
fn presets(
    ui: &mut egui::Ui,
    state: &mut SheetState,
    backend: &dyn TradingBackend,
    market: &MarketRef,
    rules: &MarketRules,
) {
    let mut chosen = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for percent in [10u32, 25, 50, 75, 100] {
            if kit::value_chip(ui, &format!("{percent}%"), false).clicked() {
                chosen = Some(percent);
            }
        }
    });
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

fn cost_lines(
    ui: &mut egui::Ui,
    check: &DraftCheck,
    preview: Option<&crate::trading::draft::PreviewResult>,
) {
    let display = crate::model::broker_ticket::display;
    let mut rows: Vec<(&str, String)> = Vec::new();
    if let Some(fee) = preview.and_then(|p| p.fee).or(check.estimate.fee) {
        rows.push(("Fee ≈", display(fee)));
    }
    // Margin only says something when it is not simply the order's value (leverage).
    if let Some(margin) = check.estimate.margin
        && check.estimate.notional != Some(margin)
    {
        rows.push(("Margin", display(margin)));
    }
    if rows.is_empty() {
        return;
    }
    ui.add_space(6.0);
    for (label, value) in rows {
        ui.horizontal(|ui| {
            ui.label(kit::text(
                label,
                kit::size::CAPTION + 1.0,
                kit::REGULAR,
                theme::text_secondary(),
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(kit::text(
                    &value,
                    kit::size::CAPTION + 1.0,
                    kit::REGULAR,
                    theme::text_secondary(),
                ));
            });
        });
    }
}

/// The venue-specific switches (post only, reduce only, take profit and stop loss) and the time in
/// force: only what this venue can do.
fn options(ui: &mut egui::Ui, state: &mut SheetState, rules: &MarketRules, check: &DraftCheck) {
    let caps = &rules.caps;
    if caps.time_in_force.len() > 1 && state.draft.uses_limit_price() {
        paper_form::field_label(ui, "Time in force");
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
        if let Some(index) = paper_form::dropdown_field(ui, &labels, selected, true) {
            state.draft.tif = caps.time_in_force[index];
        }
        ui.add_space(10.0);
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
        ui.add_space(8.0);
    }
    if caps.brackets && state.draft.bracket {
        let unit = rules.quote_asset.as_str();
        paper_form::field_label(ui, &format!("Take profit ({unit})"));
        number(
            ui,
            "take-profit",
            &mut state.draft.take_profit_text,
            "Optional",
            unit,
            check,
            DraftField::TakeProfit,
            Some(rules.price_increment),
            None,
        );
        ui.add_space(8.0);
        paper_form::field_label(ui, &format!("Stop loss ({unit})"));
        number(
            ui,
            "stop-loss",
            &mut state.draft.stop_loss_text,
            "Optional",
            unit,
            check,
            DraftField::StopLoss,
            Some(rules.price_increment),
            None,
        );
        ui.add_space(8.0);
    }
}

fn status_line(
    ui: &mut egui::Ui,
    state: &mut SheetState,
    check: &DraftCheck,
    gate: &PreviewGate,
    info: &VenueInfo,
    now: Instant,
) {
    if let Some(order_issue) = check.issue_for(DraftField::Order) {
        note(ui, order_issue, true);
        return;
    }
    if !check.is_complete() {
        return;
    }
    match gate {
        PreviewGate::Waiting | PreviewGate::Stale => {
            ui.label(kit::text(
                format!("Checking with {}…", info.name),
                kit::size::CAPTION + 1.0,
                kit::REGULAR,
                theme::text_tertiary(),
            ));
        }
        PreviewGate::Failed(text) => {
            note(
                ui,
                &format!("{} could not check this order: {text}", info.name),
                true,
            );
            ui.add_space(6.0);
            if kit::sheet_button(ui, "Try again", false, true).clicked() {
                state.preview.retry(now);
            }
        }
        PreviewGate::Blocked => {
            if let Some(preview) = state.preview.result() {
                for problem in &preview.problems {
                    note(ui, problem, true);
                    ui.add_space(4.0);
                }
            }
        }
        PreviewGate::Ready => {
            if let Some(preview) = state.preview.result() {
                for warning in &preview.warnings {
                    note(ui, warning, false);
                    ui.add_space(4.0);
                }
            }
        }
    }
}

/// Why the button is not pressable, in a sentence.
fn why_not(
    check: &DraftCheck,
    gate: &PreviewGate,
    paused: Option<&str>,
    replay: bool,
) -> Option<String> {
    if replay {
        return Some("Not available during replay.".to_owned());
    }
    if let Some(text) = paused {
        return Some(text.to_owned());
    }
    if let Some(issue) = check.issues.first() {
        return Some(issue.text.clone());
    }
    if let Some(field) = check.missing.first() {
        return Some(
            match field {
                DraftField::Size => "Enter an amount.",
                DraftField::Price => "Enter a price.",
                DraftField::StopPrice => "Enter a stop price.",
                _ => "The order is not complete.",
            }
            .to_owned(),
        );
    }
    match gate {
        PreviewGate::Ready => None,
        PreviewGate::Waiting | PreviewGate::Stale => Some("Checking the order first.".to_owned()),
        PreviewGate::Failed(_) => Some("The order could not be checked.".to_owned()),
        PreviewGate::Blocked => Some("The venue would refuse this order.".to_owned()),
    }
}

fn available_line(ui: &mut egui::Ui, backend: &dyn TradingBackend, market: &MarketRef, side: Side) {
    let Some(funds) = backend.available(market, side) else {
        return;
    };
    ui.add_space(8.0);
    ui.vertical_centered(|ui| {
        ui.label(kit::text(
            format!(
                "Available {} {}",
                crate::model::broker_ticket::display(funds.value),
                funds.asset
            ),
            kit::size::CAPTION + 1.0,
            kit::REGULAR,
            theme::text_secondary(),
        ));
    });
}

fn notice(ui: &mut egui::Ui, state: &mut SheetState) {
    let Some((text, error, at)) = &state.notice else {
        return;
    };
    if at.elapsed() > NOTICE_FOR {
        state.notice = None;
        return;
    }
    ui.add_space(8.0);
    note(ui, text, *error);
}

fn review_dialog(
    ui: &mut egui::Ui,
    state: &mut SheetState,
    info: &VenueInfo,
    backend: &dyn TradingBackend,
    market: &MarketRef,
    rules: &MarketRules,
    check: &DraftCheck,
) {
    if !matches!(state.stage, Stage::Confirming) {
        return;
    }
    let account = backend.account().map(|account| account.label);
    let lines = confirm::lines(
        &state.draft,
        rules,
        account.as_deref(),
        &check.estimate,
        state.preview.result(),
    );
    let message = format!(
        "This sends a real order to {}. Check every line.",
        info.name
    );
    let mut spec = dialog::Dialog::new(
        Icon::ArrowLeftRight,
        broker_badge::LIVE,
        "Place live order?",
        &message,
        "Place order",
    );
    spec.dismiss = "Back";
    spec.cautious = true;
    let outcome = dialog::show(ui.ctx(), Id::new("trade-confirm"), &spec, |ui| {
        ui.add_space(14.0);
        for (name, value) in &lines {
            ui.horizontal(|ui| {
                ui.set_width(ui.available_width());
                ui.label(kit::text(
                    name,
                    kit::size::BODY,
                    kit::REGULAR,
                    theme::text_secondary(),
                ));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(kit::text(
                            value,
                            kit::size::BODY,
                            kit::MEDIUM,
                            theme::text_primary(),
                        ))
                        .wrap(),
                    );
                });
            });
            ui.add_space(4.0);
        }
    });
    match outcome {
        Some(dialog::Outcome::Accepted) => {
            state.stage = Stage::Sending(backend.submit(market, &state.draft));
        }
        Some(_) => state.stage = Stage::Editing,
        None => {}
    }
}

/// Takes the venue's answer to a send, when it has one.
fn sending_result(state: &mut SheetState, _now: Instant) {
    let Stage::Sending(task) = &mut state.stage else {
        return;
    };
    let Some(result) = task.poll().cloned() else {
        return;
    };
    state.stage = Stage::Editing;
    match result {
        Ok(_) => {
            // The order is on its way: what was typed for it is done with, the side and prices stay.
            state.draft.size_text.clear();
            state.draft.take_profit_text.clear();
            state.draft.stop_loss_text.clear();
            state.say("Order placed.", false);
        }
        Err(text) => state.say(text, true),
    }
}

fn poll_cancels(state: &mut SheetState) {
    let mut said = Vec::new();
    state.cancels.retain_mut(|task| match task.poll() {
        Some(Err(text)) => {
            said.push(text.clone());
            false
        }
        Some(Ok(())) => false,
        None => true,
    });
    for text in said {
        state.say(text, true);
    }
}

/// The working orders, compact: the full lists are in the orders panel.
fn open_orders(ui: &mut egui::Ui, state: &mut SheetState, backend: &dyn TradingBackend) {
    let rows: Vec<OrderRow> = backend
        .orders()
        .into_iter()
        .filter(|row| row.working)
        .collect();
    ui.horizontal(|ui| {
        ui.label(kit::text(
            "Open orders",
            kit::size::BODY,
            kit::SEMIBOLD,
            theme::text_primary(),
        ));
        ui.label(kit::text(
            format!(" {}", rows.len()),
            kit::size::BODY,
            kit::REGULAR,
            theme::text_secondary(),
        ));
    });
    ui.add_space(6.0);
    if rows.is_empty() {
        ui.label(kit::text(
            "Working orders rest here until they fill or are cancelled.",
            kit::size::CAPTION + 1.0,
            kit::REGULAR,
            theme::text_tertiary(),
        ));
        return;
    }
    const SHOWN: usize = 6;
    for row in rows.iter().take(SHOWN) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let color = match row.side {
                Side::Buy => theme::UP,
                Side::Sell => theme::DOWN,
            };
            ui.label(kit::text(
                match row.side {
                    Side::Buy => "Buy",
                    Side::Sell => "Sell",
                },
                kit::size::CAPTION + 1.0,
                kit::SEMIBOLD,
                color,
            ));
            let price = row
                .price
                .map_or_else(|| "market".to_owned(), crate::model::broker_ticket::display);
            let size = row
                .size
                .map_or_else(|| "—".to_owned(), crate::model::broker_ticket::display);
            ui.add(
                egui::Label::new(kit::text(
                    format!("{size} @ {price}"),
                    kit::size::CAPTION + 1.0,
                    kit::REGULAR,
                    theme::text_primary(),
                ))
                .truncate(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(row.can_cancel, |ui| {
                    if icons::button(
                        ui,
                        Icon::Xmark,
                        20.0,
                        theme::text_secondary(),
                        "Cancel this order",
                    )
                    .clicked()
                    {
                        state.cancels.push(backend.cancel(&row.id));
                    }
                });
                kit::status_badge(ui, row.status.label, tone(row.status.tone));
            });
        });
        ui.add_space(6.0);
    }
    if rows.len() > SHOWN {
        ui.label(kit::text(
            format!("{} more in the orders panel", rows.len() - SHOWN),
            kit::size::CAPTION + 1.0,
            kit::REGULAR,
            theme::text_tertiary(),
        ));
    }
}

fn tone(tone: crate::trading::model::Tone) -> kit::Tone {
    use crate::trading::model::Tone;
    match tone {
        Tone::Good => kit::Tone::Good,
        Tone::Warning => kit::Tone::Warning,
        Tone::Bad => kit::Tone::Bad,
        Tone::Neutral => kit::Tone::Neutral,
    }
}
