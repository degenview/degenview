//! The order sheet: where an order is typed, checked with the venue, and sent, for any venue.
//!
//! It is a right-hand panel beside the chart. Everything it shows comes from the venue's
//! [`TradingBackend`]: which order types exist, how a size is counted, what the account can spend,
//! what the venue says an order would do. The sheet holds only what is typed and what the user is
//! in the middle of (reviewing, sending).

mod form;
mod open_orders;
mod review;
mod state;
mod status;

use std::time::Duration;

use degenview_brokers::{Decimal, MarketRef};
use eframe::egui::{self, vec2};

use super::connect_list;
use super::fields::{self, Hero};
use super::figures;
use super::venue_pill::{self, PillAction};
use crate::model::DataSource;
use crate::model::broker::venue::TradingVenue;
use crate::trading::backend::TradingBackend;
use crate::trading::model::{
    BackendState, MarketAccess, MarketRules, Unavailable, UnavailableKind, VenueInfo,
};
use crate::ui::icons::{self, Icon};
use crate::ui::{broker::badge, kit, theme};
use form::FormContext;
pub use state::SheetState;

/// What the focused chart knows of its market, for the top of the sheet: where its logo comes
/// from, the last price, and the move the chart's own header shows (price and percent over the
/// candles on screen: not a 24-hour figure).
#[derive(Clone, Debug)]
pub struct MarketFace {
    pub source: DataSource,
    pub last: Option<f64>,
    pub change: Option<(f64, f64)>,
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
    /// The same chart's logo source, last price and move.
    pub face: Option<MarketFace>,
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
                        if let Some(venue) = connect_list::show(ui, input.all) {
                            actions.push(SheetAction::Connect(venue));
                        }
                    });
            });
        return actions;
    };
    if info.kind.is_real_money() {
        badge::rule(ui);
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
            if icons::button(
                ui,
                Icon::Gear,
                22.0,
                theme::text_secondary(),
                "Trading settings",
            )
            .clicked()
            {
                actions.push(SheetAction::OpenSettings);
            }
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
    if !matches!(bstate, BackendState::Paused(_) | BackendState::Ready) {
        // A review of an order for a connection that is gone cannot be answered with "Place".
        state.drop_review();
    }
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
    state.follow(&market, info.venue);
    let access = backend.market(&market);
    let ready_rules = match &access {
        MarketAccess::Ready(rules) => Some(rules.as_ref()),
        MarketAccess::Unavailable(_) => None,
    };
    hero(ui, &market, input.face.as_ref(), ready_rules, backend);
    ui.add_space(14.0);
    let rules = match access {
        MarketAccess::Ready(rules) => rules,
        MarketAccess::Unavailable(Unavailable { kind, text }) => {
            unavailable_card(ui, kind, &text);
            return;
        }
    };
    let context = FormContext {
        info,
        backend,
        bstate,
        market: &market,
        rules: &rules,
        replay: input.replay,
        confirm_resting: input.confirm_resting,
    };
    form::show(ui, state, &context);
}

/// The market in focus: logo, symbol and pair, last price and its move.
fn hero(
    ui: &mut egui::Ui,
    market: &MarketRef,
    face: Option<&MarketFace>,
    rules: Option<&MarketRules>,
    backend: &dyn TradingBackend,
) {
    let increment = rules.map(|rules| rules.price_increment);
    let last = face
        .and_then(|face| face.last)
        .and_then(|last| Decimal::try_from(last).ok())
        .or_else(|| backend.quote(market).and_then(|quote| quote.last));
    fields::market_hero(
        ui,
        &Hero {
            symbol: &market.symbol,
            source: face.map(|face| face.source),
            subtitle: rules.map(|rules| format!("{} / {}", rules.base_asset, rules.quote_asset)),
            price: last.map(|last| figures::price_label(last, increment)),
            change: face.and_then(|face| face.change).map(|(delta, percent)| {
                (
                    figures::price_move_text(delta, percent, increment),
                    delta >= 0.0,
                )
            }),
        },
    );
}
