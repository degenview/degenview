//! The order ticket for a live broker, and the confirmation that follows it.
//!
//! Everything a user can do here is checked twice: by the pure rules in
//! [`crate::model::broker_ticket`] while they type, and by the broker's own preview (fees, warnings
//! and refusals) once the order is well formed. Nothing is sent until the user has seen exactly what
//! will be sent. The broker's rules and prices are read from the connection, never from the chart:
//! the price a chart shows is not necessarily the price an order trades at.
//!
//! The view never touches the network: it asks the broker's runtime for the market's rules, the
//! best prices, the preview and the placement, and polls the answers on later frames.

use std::time::{Duration, Instant};

use degenview_brokers::model::orders::{OrderPreview, PlacedOrder};
use degenview_brokers::validate::floor_to_increment;
use degenview_brokers::{
    BrokerId, BrokerSnapshot, Decimal, OrderDraft, Pending, ProductId, ProductSpec, Quote, Side,
};
use eframe::egui::{self, Id};

use crate::host::broker_host::BrokerHost;
use crate::model::broker_ticket::{
    self as rules, TicketContext, TicketDraft, TicketField, TicketIssue, TicketKind, TicketTif,
};
use crate::ui::icons::Icon;
use crate::ui::kit;
use crate::ui::settings_api_keys::field;
use crate::ui::{broker_badge, dialog, sheet, theme};

const WIDTH: f32 = 460.0;
/// The best prices are asked for this often while the ticket is open.
const QUOTE_EVERY: Duration = Duration::from_secs(2);
/// A preview is asked for this long after the last change, not on every keystroke.
const PREVIEW_AFTER: Duration = Duration::from_millis(450);

enum Stage {
    Editing,
    /// The user has pressed Review: the order is shown for approval.
    Confirming(Box<OrderDraft>),
    Sending(Pending<PlacedOrder>),
}

/// What happened to the ticket this frame.
#[derive(Debug, Eq, PartialEq)]
pub enum TicketEvent {
    Open,
    Closed,
    /// The broker accepted the order.
    Placed,
    /// The order was sent and the broker has not said whether it took it: it is shown as
    /// unconfirmed until it is looked up.
    Unconfirmed(String),
}

pub struct TicketState {
    pub broker: BrokerId,
    pub product: ProductId,
    pub draft: TicketDraft,
    spec: Option<ProductSpec>,
    spec_pending: Option<Pending<ProductSpec>>,
    quote: Option<Quote>,
    quote_pending: Option<Pending<Quote>>,
    quote_asked: Option<Instant>,
    /// The order the last preview was asked for, and the answer.
    preview_for: Option<String>,
    preview: Option<OrderPreview>,
    preview_error: Option<String>,
    preview_pending: Option<(String, Pending<OrderPreview>)>,
    seen: String,
    changed_at: Instant,
    stage: Stage,
    error: Option<String>,
    /// A test presses Review without a pointer.
    #[cfg(test)]
    force_review: bool,
}

impl TicketState {
    /// Whether the market's rules and the broker's preview have arrived.
    #[cfg(test)]
    pub(crate) fn ready_to_review(&self) -> bool {
        self.spec.is_some()
            && self.preview_for.is_some()
            && self.preview_for == Some(self.seen.clone())
    }

    #[cfg(test)]
    pub(crate) fn confirming(&self) -> bool {
        matches!(self.stage, Stage::Confirming(_))
    }

    #[cfg(test)]
    pub(crate) fn press_review(&mut self) {
        self.force_review = true;
    }

    pub fn new(broker: BrokerId, product: ProductId, draft: TicketDraft) -> Self {
        Self {
            broker,
            product,
            draft,
            spec: None,
            spec_pending: None,
            quote: None,
            quote_pending: None,
            quote_asked: None,
            preview_for: None,
            preview: None,
            preview_error: None,
            preview_pending: None,
            seen: String::new(),
            changed_at: Instant::now(),
            stage: Stage::Editing,
            error: None,
            #[cfg(test)]
            force_review: false,
        }
    }

    fn sending(&self) -> bool {
        matches!(self.stage, Stage::Sending(_))
    }
}

/// Reads answers that arrived and asks for what is missing. Never blocks.
fn drive(
    state: &mut TicketState,
    host: &BrokerHost,
    snapshot: &BrokerSnapshot,
    ctx: &egui::Context,
) {
    let Some(runtime) = host.runtime(state.broker) else {
        return;
    };
    if let Some(pending) = &state.spec_pending
        && let Some(answer) = pending.poll()
    {
        state.spec_pending = None;
        match answer {
            Ok(spec) => state.spec = Some(spec),
            Err(error) => state.error = Some(error.to_string()),
        }
    }
    // The rules come fresh from the broker when the ticket opens (the ones in the snapshot may be
    // old), and the snapshot's are used until then.
    if state.spec.is_none() && state.spec_pending.is_none() {
        state.spec_pending = Some(runtime.product(state.product.clone()));
    }
    if let Some(pending) = &state.quote_pending
        && let Some(answer) = pending.poll()
    {
        state.quote_pending = None;
        if let Ok(quote) = answer {
            state.quote = Some(quote);
        }
    }
    if state.quote_pending.is_none()
        && state
            .quote_asked
            .is_none_or(|asked| asked.elapsed() >= QUOTE_EVERY)
    {
        state.quote_asked = Some(Instant::now());
        state.quote_pending = Some(runtime.quote(state.product.clone()));
    }
    if let Some((_, pending)) = &state.preview_pending
        && let Some(answer) = pending.poll()
    {
        let (signature, _) = state.preview_pending.take().expect("pending preview");
        if signature == state.seen {
            match answer {
                Ok(preview) => {
                    state.preview = Some(preview);
                    state.preview_error = None;
                }
                Err(error) => {
                    state.preview = None;
                    state.preview_error = Some(error.to_string());
                }
            }
            state.preview_for = Some(signature);
        }
    }
    let _ = snapshot;
    ctx.request_repaint_after(Duration::from_millis(100));
}

pub fn show(ctx: &egui::Context, state: &mut TicketState, host: &BrokerHost) -> TicketEvent {
    let Some(snapshot) = host.snapshot(state.broker) else {
        return TicketEvent::Closed;
    };
    let Some(broker) = degenview_brokers::registry::by_id(state.broker) else {
        return TicketEvent::Closed;
    };
    let descriptor = broker.descriptor();
    drive(state, host, &snapshot, ctx);

    let prefs = host.prefs(state.broker);
    let spec = state
        .spec
        .clone()
        .or_else(|| snapshot.product(&state.product).cloned());
    let caps = snapshot.capabilities_of(&state.product).cloned();
    let (Some(spec), Some(caps)) = (spec, caps) else {
        // The market's rules have not arrived (or the broker has no such market).
        let mut event = TicketEvent::Open;
        sheet::show(ctx, Id::new("broker-ticket"), WIDTH, |ui| {
            sheet::inset(ui, |ui| {
                ui.add_space(24.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(kit::text(
                        match &state.error {
                            Some(error) => error.clone(),
                            None => format!(
                                "Reading {}'s rules for this market…",
                                descriptor.display_name
                            ),
                        },
                        kit::size::BODY,
                        kit::REGULAR,
                        theme::text_secondary(),
                    ));
                });
                ui.add_space(16.0);
                if kit::sheet_button(ui, "Close", false, true).clicked() {
                    event = TicketEvent::Closed;
                }
                ui.add_space(24.0);
            });
        });
        return event;
    };

    let now_ms = crate::model::watchlist::now_ms();
    let context = TicketContext {
        spec: &spec,
        caps: &caps,
        quote: state.quote.as_ref(),
        balances: &snapshot.balances,
        fees: snapshot.fees.as_ref(),
        now_ms,
    };
    let issues = state.draft.issues(&context);
    let built = if issues.is_empty() {
        state.draft.build(&context).ok()
    } else {
        None
    };

    // A change to the order voids what was previewed, and asks again when typing has paused.
    let signature = built.as_ref().map_or(String::new(), |d| format!("{d:?}"));
    if signature != state.seen {
        state.seen = signature.clone();
        state.changed_at = Instant::now();
        state.preview = None;
        state.preview_error = None;
        state.preview_for = None;
        state.preview_pending = None;
    }
    if let Some(order) = &built
        && state.preview_for.as_deref() != Some(signature.as_str())
        && state.preview_pending.is_none()
        && state.changed_at.elapsed() >= PREVIEW_AFTER
        && matches!(state.stage, Stage::Editing)
        && let Some(runtime) = host.runtime(state.broker)
    {
        state.preview_pending = Some((signature.clone(), runtime.preview(order.clone())));
    }
    let previewing = built.is_some()
        && state.preview_for.as_deref() != Some(signature.as_str())
        && state.preview_error.is_none();
    let refused = state
        .preview
        .as_ref()
        .is_some_and(|preview| !preview.is_acceptable());

    // The submission in flight.
    let mut event = TicketEvent::Open;
    if let Stage::Sending(pending) = &state.stage
        && let Some(answer) = pending.poll()
    {
        match answer {
            Ok(_) => event = TicketEvent::Placed,
            Err(error) if error.kind == degenview_brokers::ErrorKind::OutcomeUnknown => {
                event = TicketEvent::Unconfirmed(error.to_string());
            }
            Err(error) => {
                state.error = Some(error.to_string());
                state.stage = Stage::Editing;
            }
        }
    }
    if event != TicketEvent::Open {
        return event;
    }

    // Figures for an order that is not valid yet would only mislead.
    let estimate = if issues.is_empty() {
        state.draft.estimate(&context)
    } else {
        rules::Estimate::default()
    };
    let account = snapshot
        .session
        .as_ref()
        .and_then(|session| session.accounts.first())
        .map_or("Account".to_owned(), |account| account.label.clone());
    let escape = ctx.input(|input| input.key_pressed(egui::Key::Escape));

    // The approval, shown over the ticket.
    if let Stage::Confirming(order) = &state.stage {
        let lines = rules::confirmation_lines(order, &spec, &estimate, &account);
        let message = format!(
            "This sends a real order to {}. Check every line.",
            descriptor.display_name
        );
        let mut dialog_spec = dialog::Dialog::new(
            Icon::ArrowLeftRight,
            broker_badge::LIVE,
            "Place live order?",
            &message,
            "Place order",
        );
        dialog_spec.dismiss = "Back";
        dialog_spec.cautious = true;
        let outcome = dialog::show(ctx, Id::new("broker-confirm"), &dialog_spec, |ui| {
            ui.add_space(14.0);
            for (name, value) in &lines {
                ui.horizontal(|ui| {
                    ui.set_width(ui.available_width());
                    ui.label(kit::text(
                        *name,
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
                let order = (**order).clone();
                if let Some(runtime) = host.runtime(state.broker) {
                    state.error = None;
                    state.stage = Stage::Sending(runtime.submit(order));
                }
            }
            Some(_) => state.stage = Stage::Editing,
            None => {}
        }
        return TicketEvent::Open;
    }

    let mut review = false;
    let mut close = false;
    #[allow(unused_assignments)]
    let mut can_review_now = false;
    sheet::show(ctx, Id::new("broker-ticket"), WIDTH, |ui| {
        broker_badge::rule(ui);
        egui::ScrollArea::vertical()
            .max_height((ctx.content_rect().height() - 120.0).clamp(240.0, 760.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.add_space(20.0);
                sheet::inset(ui, |ui| {
                    form(
                        ui,
                        state,
                        FormFacts {
                            spec: &spec,
                            caps: &caps,
                            snapshot: &snapshot,
                            issues: &issues,
                            estimate: &estimate,
                            account: &account,
                            broker: descriptor.display_name,
                            previewing,
                        },
                    );
                });
                ui.add_space(20.0);
            });
        let can_review = built.is_some() && !refused && !state.sending() && !previewing;
        can_review_now = can_review;
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(24.0);
                let label = if state.draft.needs_confirmation(prefs.confirm_resting_orders) {
                    "Review order"
                } else {
                    "Place order"
                };
                if state.sending() {
                    ui.spinner();
                } else if kit::sheet_button(ui, label, true, can_review).clicked() {
                    review = true;
                }
                if kit::sheet_button(ui, "Cancel", false, !state.sending()).clicked() {
                    close = true;
                }
            });
        });
        ui.add_space(16.0);
    });
    #[cfg(test)]
    if std::mem::take(&mut state.force_review) && can_review_now {
        review = true;
    }
    if escape && !state.sending() {
        close = true;
    }
    if close {
        return TicketEvent::Closed;
    }
    if review && let Some(order) = built {
        if state.draft.needs_confirmation(prefs.confirm_resting_orders) {
            state.stage = Stage::Confirming(Box::new(order));
        } else if let Some(runtime) = host.runtime(state.broker) {
            state.error = None;
            state.stage = Stage::Sending(runtime.submit(order));
        }
    }
    TicketEvent::Open
}

struct FormFacts<'a> {
    spec: &'a ProductSpec,
    caps: &'a degenview_brokers::Capabilities,
    snapshot: &'a BrokerSnapshot,
    issues: &'a [TicketIssue],
    estimate: &'a rules::Estimate,
    account: &'a str,
    broker: &'a str,
    previewing: bool,
}

fn problem_under(ui: &mut egui::Ui, issues: &[TicketIssue], field: TicketField) {
    for issue in issues.iter().filter(|issue| issue.field == field) {
        ui.add_space(4.0);
        ui.add(
            egui::Label::new(kit::text(
                &issue.text,
                kit::size::CAPTION + 0.5,
                kit::REGULAR,
                kit::RED,
            ))
            .wrap(),
        );
    }
}

fn form(ui: &mut egui::Ui, state: &mut TicketState, facts: FormFacts) {
    let spec = facts.spec;
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(kit::text(
                &spec.display_name,
                kit::size::TITLE2 + 1.0,
                kit::SEMIBOLD,
                theme::text_primary(),
            ));
            ui.add_space(2.0);
            ui.label(kit::text(
                format!("{} · {}", facts.broker, facts.account),
                kit::size::SUBHEADLINE + 1.0,
                kit::REGULAR,
                theme::text_secondary(),
            ));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            broker_badge::badge(ui, facts.broker);
        });
    });
    ui.add_space(14.0);

    // The broker's own best prices, said to be the broker's.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        let shown = |value: Option<Decimal>| {
            value.map_or("—".to_owned(), |v| rules::price_display(v, spec))
        };
        for (name, value) in [
            ("Bid", state.quote.as_ref().and_then(|q| q.bid)),
            ("Ask", state.quote.as_ref().and_then(|q| q.ask)),
        ] {
            ui.label(kit::text(
                format!("{name}  {}", shown(value)),
                kit::size::BODY,
                kit::MEDIUM,
                theme::text_primary(),
            ));
        }
        ui.label(kit::text(
            format!("{} prices", facts.broker),
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            theme::text_tertiary(),
        ));
    });
    ui.add_space(14.0);

    kit::segmented(
        ui,
        Id::new("broker-side"),
        &[(Side::Buy, "Buy"), (Side::Sell, "Sell")],
        &mut state.draft.side,
    );
    ui.add_space(12.0);
    let kinds: Vec<(TicketKind, &str)> =
        [TicketKind::Market, TicketKind::Limit, TicketKind::StopLimit]
            .into_iter()
            .filter(|kind| facts.caps.allows(kind.order_type()))
            .map(|kind| (kind, kind.label()))
            .collect();
    if !kinds.contains(&(state.draft.kind, state.draft.kind.label()))
        && let Some((first, _)) = kinds.first()
    {
        state.draft.kind = *first;
    }
    kit::segmented(ui, Id::new("broker-kind"), &kinds, &mut state.draft.kind);
    ui.add_space(14.0);

    // Size.
    let market_buy = state.draft.kind == TicketKind::Market && state.draft.side == Side::Buy;
    if market_buy && facts.caps.quote_sized_market {
        kit::segmented(
            ui,
            Id::new("broker-size-unit"),
            &[(false, "Size"), (true, "Amount to spend")],
            &mut state.draft.spend_in_quote,
        );
        ui.add_space(8.0);
    } else {
        state.draft.spend_in_quote = false;
    }
    let size_title = if state.draft.spend_in_quote {
        format!("Spend ({})", spec.quote)
    } else {
        format!("Size ({})", spec.base)
    };
    field(
        ui,
        &size_title,
        None,
        "0",
        &mut state.draft.size_text,
        false,
    );
    problem_under(ui, facts.issues, TicketField::Size);
    ui.add_space(6.0);
    quick_fill(ui, state, &facts);
    ui.add_space(12.0);

    match state.draft.kind {
        TicketKind::Market => {}
        TicketKind::Limit => {
            field(
                ui,
                &format!("Limit price ({})", spec.quote),
                None,
                "0",
                &mut state.draft.price_text,
                false,
            );
            problem_under(ui, facts.issues, TicketField::Price);
            ui.add_space(6.0);
            price_buttons(ui, state, spec);
            ui.add_space(12.0);
        }
        TicketKind::StopLimit => {
            field(
                ui,
                &format!("Stop price ({})", spec.quote),
                Some("The order is placed when the price reaches this."),
                "0",
                &mut state.draft.stop_text,
                false,
            );
            problem_under(ui, facts.issues, TicketField::StopPrice);
            ui.add_space(12.0);
            field(
                ui,
                &format!("Limit price ({})", spec.quote),
                None,
                "0",
                &mut state.draft.price_text,
                false,
            );
            problem_under(ui, facts.issues, TicketField::LimitPrice);
            ui.add_space(12.0);
        }
    }
    if state.draft.kind == TicketKind::Limit {
        if facts
            .caps
            .allows_tif(degenview_brokers::TifKind::FillOrKill)
        {
            kit::segmented(
                ui,
                Id::new("broker-tif"),
                &[
                    (
                        TicketTif::GoodTilCancelled,
                        TicketTif::GoodTilCancelled.label(),
                    ),
                    (TicketTif::FillOrKill, TicketTif::FillOrKill.label()),
                ],
                &mut state.draft.tif,
            );
            ui.add_space(8.0);
        }
        if facts.caps.post_only && state.draft.tif == TicketTif::GoodTilCancelled {
            ui.horizontal(|ui| {
                kit::switch(ui, &mut state.draft.post_only, "Post-only");
                ui.label(kit::text(
                    "Post-only: refused rather than trade at once",
                    kit::size::BODY,
                    kit::REGULAR,
                    theme::text_secondary(),
                ));
            });
        } else {
            state.draft.post_only = false;
        }
        problem_under(ui, facts.issues, TicketField::TimeInForce);
        ui.add_space(12.0);
    }
    problem_under(ui, facts.issues, TicketField::Type);

    summary(ui, state, &facts);
    if let Some(error) = &state.error {
        ui.add_space(12.0);
        kit::notice_card(
            ui,
            Icon::TriangleFill,
            kit::RED,
            "Order not placed",
            Some(error),
            None,
        );
    }
}

/// 25 / 50 / 75 / 100 percent of what the account can put into this order, rounded *down* to the
/// market's increment - an explicit shortcut, never applied on its own.
fn quick_fill(ui: &mut egui::Ui, state: &mut TicketState, facts: &FormFacts) {
    let spec = facts.spec;
    let available = |asset: &str| -> Decimal {
        facts
            .snapshot
            .balances
            .iter()
            .filter(|b| b.asset == asset)
            .map(|b| b.available)
            .sum()
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let held = match state.draft.side {
            Side::Sell => format!(
                "{} {} available",
                rules::display(available(&spec.base)),
                spec.base
            ),
            Side::Buy => format!(
                "{} {} available",
                rules::display(available(&spec.quote)),
                spec.quote
            ),
        };
        for percent in [25_u32, 50, 75, 100] {
            if kit::sheet_button(ui, &format!("{percent}%"), false, true).clicked() {
                let fraction = Decimal::from(percent) / Decimal::ONE_HUNDRED;
                let text = match (state.draft.side, state.draft.spend_in_quote) {
                    (Side::Sell, _) => Some(floor_to_increment(
                        available(&spec.base) * fraction,
                        spec.base_increment,
                    )),
                    (Side::Buy, true) => Some(floor_to_increment(
                        available(&spec.quote) * fraction,
                        spec.quote_increment,
                    )),
                    (Side::Buy, false) => reference(state, spec).map(|price| {
                        floor_to_increment(
                            available(&spec.quote) * fraction / price,
                            spec.base_increment,
                        )
                    }),
                };
                if let Some(value) = text {
                    state.draft.size_text = value.normalize().to_string();
                }
            }
        }
        ui.label(kit::text(
            held,
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            theme::text_tertiary(),
        ));
    });
}

fn reference(state: &TicketState, spec: &ProductSpec) -> Option<Decimal> {
    let _ = spec;
    let typed = rules::parse_decimal(&state.draft.price_text).ok().flatten();
    match state.draft.kind {
        TicketKind::Market => state.quote.as_ref().and_then(|q| match state.draft.side {
            Side::Buy => q.ask,
            Side::Sell => q.bid,
        }),
        _ => typed,
    }
    .filter(|price| *price > Decimal::ZERO)
}

/// Bid / Ask / Last buttons that fill the price (rounded down to the increment, by the user's
/// choice to press them).
fn price_buttons(ui: &mut egui::Ui, state: &mut TicketState, spec: &ProductSpec) {
    let Some(quote) = state.quote.clone() else {
        return;
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (name, value) in [("Bid", quote.bid), ("Ask", quote.ask), ("Last", quote.last)] {
            let Some(value) = value else { continue };
            if kit::sheet_button(ui, &format!("Use {}", name.to_lowercase()), false, true).clicked()
            {
                state.draft.price_text = floor_to_increment(value, spec.price_increment)
                    .normalize()
                    .to_string();
            }
        }
    });
}

fn summary(ui: &mut egui::Ui, state: &TicketState, facts: &FormFacts) {
    let spec = facts.spec;
    kit::card_frame(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 6.0;
        let row = |ui: &mut egui::Ui, name: &str, value: String, strong: bool| {
            ui.horizontal(|ui| {
                ui.label(kit::text(
                    name,
                    kit::size::BODY,
                    kit::REGULAR,
                    theme::text_secondary(),
                ));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(kit::text(
                        value,
                        kit::size::BODY,
                        if strong { kit::SEMIBOLD } else { kit::MEDIUM },
                        theme::text_primary(),
                    ));
                });
            });
        };
        let estimate = facts.estimate;
        match (&state.preview, estimate.notional) {
            (Some(preview), _) if preview.is_acceptable() => {
                if let Some(total) = preview.order_total {
                    row(
                        ui,
                        &format!("{} total", facts.broker),
                        format!("{} {}", rules::display(total), spec.quote),
                        true,
                    );
                }
                if let Some(fee) = preview.fees {
                    row(
                        ui,
                        "Fee",
                        format!("{} {}", rules::display(fee), spec.quote),
                        false,
                    );
                }
                if let Some(price) = preview.estimated_average_price {
                    row(
                        ui,
                        "Estimated price",
                        format!("{} {}", rules::price_display(price, spec), spec.quote),
                        false,
                    );
                }
            }
            (_, Some(notional)) => {
                row(
                    ui,
                    "Estimated value",
                    format!("{} {}", rules::display(notional.round_dp(8)), spec.quote),
                    true,
                );
                if let Some(fee) = estimate.fee {
                    row(
                        ui,
                        "Estimated fee",
                        format!(
                            "{} {}{}",
                            rules::display(fee.round_dp(8)),
                            spec.quote,
                            estimate.fee_rate.map_or(String::new(), |r| format!(
                                " ({}%)",
                                rules::display((r * Decimal::ONE_HUNDRED).round_dp(4))
                            ))
                        ),
                        false,
                    );
                }
            }
            _ => {
                row(ui, "Estimated value", "—".to_owned(), false);
            }
        }
        if facts.previewing {
            ui.label(kit::text(
                format!("Asking {} for the exact figures…", facts.broker),
                kit::size::CAPTION + 0.5,
                kit::REGULAR,
                theme::text_tertiary(),
            ));
        }
        if let Some(error) = &state.preview_error {
            ui.label(kit::text(
                format!("The preview is unavailable: {error}"),
                kit::size::CAPTION + 0.5,
                kit::REGULAR,
                theme::text_tertiary(),
            ));
        }
        if let Some(preview) = &state.preview {
            for warning in &preview.warnings {
                ui.label(kit::text(
                    warning,
                    kit::size::CAPTION + 0.5,
                    kit::REGULAR,
                    kit::ORANGE,
                ));
            }
            for problem in &preview.problems {
                ui.label(kit::text(
                    &problem.text,
                    kit::size::CAPTION + 0.5,
                    kit::MEDIUM,
                    kit::RED,
                ));
            }
        }
    });
}
