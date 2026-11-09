//! The Trade Journal's trade panel: the numbers, every execution, and the review the user writes.
//! A pure view over a trade and a [`Draft`]; it returns what the user did and the tab decides when
//! to save. Nothing here can place an order or change a balance.

use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Ui, vec2};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::header_controls;
use super::icons::{self, Icon};
use super::journal_shots::{self as shots, Shot};
use super::kit::{self, MEDIUM, REGULAR, SEMIBOLD, size};
use super::paper_grid;
use super::theme;
use crate::model::journal::calendar::Offset;
use crate::model::journal::cells::{self, Tone, status_label, status_tone, tone_of};
use crate::model::journal::excursion::Excursion;
use crate::model::journal::filters::outcome_of;
use crate::model::journal::format;
use crate::model::journal::metric::Metric;
use crate::model::journal::model::{Direction, Side, Trade};
use crate::model::journal::playbook::Playbook;
use crate::model::journal::reviews::{Review, parse_list};
use crate::model::journal::risk;
use crate::model::journal::table::return_fraction;

pub struct Input<'a> {
    pub trade: &'a Trade,
    pub account_name: &'a str,
    pub breakeven_tolerance: Decimal,
    pub offset: Offset<'a>,
    /// 1-based place of this trade among the rows shown, and how many there are.
    pub position: (usize, usize),
    pub playbooks: &'a [Playbook],
    /// Emotion and mistake tags to offer: the defaults and every one already used.
    pub emotion_suggestions: &'a [String],
    pub mistake_suggestions: &'a [String],
    /// The account's initial balance, for risk as a share of it.
    pub account_balance: Option<Decimal>,
    /// The review's screenshots as they are right now.
    pub shots: &'a [Shot],
    pub shots_busy: bool,
    pub shots_error: Option<&'a str>,
    /// How far the trade went for and against, estimated from candles.
    pub excursion: ExcursionView<'a>,
    /// Whether this trade's market can be opened on a chart.
    pub can_view_on_chart: bool,
}

/// What the excursion block has to show.
pub enum ExcursionView<'a> {
    /// Not applicable (the trade is open).
    Hidden,
    Loading,
    /// The estimate could not be made; the message says why. Offers a retry.
    Failed(&'a str),
    Ready(&'a Metric<Excursion>),
}

/// The review being edited. `saved` is what is stored, so the tab can tell when there is
/// something to save.
pub struct Draft {
    pub review: Review,
    pub saved: Review,
    pub tags_text: String,
    pub mistakes_text: String,
    pub emotions_text: String,
    pub stop_text: String,
    pub target_text: String,
    pub planned_entry_text: String,
    /// When the user last changed something not yet saved (egui time, seconds).
    pub edited_at: Option<f64>,
    /// Show "Saved" until this time.
    pub saved_until: Option<f64>,
    pub error: Option<String>,
}

fn decimal_text(value: Option<Decimal>) -> String {
    value.map_or_else(String::new, |value| format::price(value).replace(',', ""))
}

fn parse_price(text: &str) -> Option<Decimal> {
    let text = text.trim().replace(',', "");
    (!text.is_empty())
        .then(|| text.parse::<Decimal>().ok())
        .flatten()
}

impl Draft {
    pub fn new(trade: &Trade, existing: Option<&Review>) -> Self {
        let review = existing
            .cloned()
            .unwrap_or_else(|| Review::new(&trade.key, &trade.account_uid));
        Self {
            tags_text: review.tags.join(", "),
            mistakes_text: review.mistakes.join(", "),
            emotions_text: review.emotions.join(", "),
            stop_text: decimal_text(review.stop),
            target_text: decimal_text(review.target),
            planned_entry_text: decimal_text(review.planned_entry),
            saved: review.clone(),
            review,
            edited_at: None,
            saved_until: None,
            error: None,
        }
    }

    /// Folds the text boxes into the review.
    pub fn sync_fields(&mut self) {
        self.review.tags = parse_list(&self.tags_text);
        self.review.mistakes = parse_list(&self.mistakes_text);
        self.review.emotions = parse_list(&self.emotions_text);
        self.review.stop = parse_price(&self.stop_text);
        self.review.target = parse_price(&self.target_text);
        self.review.planned_entry = parse_price(&self.planned_entry_text);
    }

    /// The text of a price field that is not a number.
    pub fn price_error(text: &str) -> bool {
        !text.trim().is_empty() && parse_price(text).is_none_or(|price| price <= Decimal::ZERO)
    }

    pub fn is_dirty(&self) -> bool {
        let mut now = self.review.clone();
        now.tidy();
        let mut saved = self.saved.clone();
        saved.tidy();
        now.created_ms = saved.created_ms;
        now.updated_ms = saved.updated_ms;
        now != saved
    }
}

#[derive(Default)]
pub struct Output {
    pub previous: bool,
    pub next: bool,
    pub close: bool,
    /// The user changed a field this frame.
    pub changed: bool,
    /// A text field lost focus: a good moment to save.
    pub blurred: bool,
    /// What the user asked of the screenshots; the tab does it.
    pub shots: shots::Action,
    /// The user asked to try the excursion estimate again.
    pub retry_excursion: bool,
    /// The user asked to see the trade on a chart.
    pub view_on_chart: bool,
}

fn tone_color(tone: Tone) -> Color32 {
    match tone {
        Tone::Normal => theme::text_primary(),
        Tone::Positive => theme::WIN,
        Tone::Negative => theme::LOSS,
        Tone::Muted => theme::text_secondary(),
    }
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(16.0);
    ui.label(kit::text(
        title.to_uppercase(),
        size::CAPTION,
        SEMIBOLD,
        theme::text_tertiary(),
    ));
    ui.add_space(6.0);
}

fn pair(ui: &mut Ui, label: &str, value: &str, tone: Tone) {
    ui.label(kit::text(
        label,
        size::BODY,
        REGULAR,
        theme::text_secondary(),
    ));
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.label(kit::text(value, size::BODY, MEDIUM, tone_color(tone)));
    });
    ui.end_row();
}

pub fn show(ui: &mut Ui, input: &Input<'_>, draft: &mut Draft) -> Output {
    let mut output = Output::default();
    let trade = input.trade;
    let outcome = outcome_of(trade, input.breakeven_tolerance);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(kit::text(
            &trade.instrument.display_name,
            17.0,
            SEMIBOLD,
            theme::text_primary(),
        ));
        let (side, tint) = match trade.direction {
            Direction::Long => ("LONG", theme::WIN),
            Direction::Short => ("SHORT", theme::LOSS),
        };
        paper_grid::tag(ui, side, tint);
        paper_grid::tag(ui, status_label(outcome), tone_color(status_tone(outcome)));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if header_controls::icon_button(ui, Icon::Xmark, "Close trade", false).clicked() {
                output.close = true;
            }
            ui.add_space(4.0);
            let (index, total) = input.position;
            if header_controls::icon_button(ui, Icon::ChevronRight, "Next trade", false).clicked() {
                output.next = true;
            }
            ui.label(kit::text(
                format!("{index} of {total}"),
                size::CAPTION + 1.0,
                REGULAR,
                theme::text_tertiary(),
            ));
            if header_controls::icon_button(ui, Icon::ChevronLeft, "Previous trade", false)
                .clicked()
            {
                output.previous = true;
            }
        });
    });
    ui.add_space(2.0);
    ui.label(kit::text(
        input.account_name,
        size::CAPTION + 1.0,
        REGULAR,
        theme::text_tertiary(),
    ));

    if input.can_view_on_chart {
        ui.add_space(8.0);
        if kit::push_button(ui, Some(Icon::ChartLine), "View on chart", true).clicked() {
            output.view_on_chart = true;
        }
    }
    summary(ui, input, draft);
    executions(ui, input);
    excursion_block(ui, input, &mut output);
    review(ui, input, draft, &mut output);
    output
}

fn summary(ui: &mut Ui, input: &Input<'_>, draft: &Draft) {
    let trade = input.trade;
    section(ui, "Summary");
    let money_tone = |value: Decimal| tone_of(value);
    egui::Grid::new(("journal-summary", &trade.key))
        .num_columns(2)
        .spacing(vec2(12.0, 7.0))
        .min_col_width(110.0)
        .show(ui, |ui| {
            pair(
                ui,
                "Opened",
                &format::datetime(trade.open_ts_ms, input.offset),
                Tone::Normal,
            );
            match trade.close_ts_ms {
                Some(close) => pair(
                    ui,
                    "Closed",
                    &format::datetime(close, input.offset),
                    Tone::Normal,
                ),
                None => pair(ui, "Closed", "Still open", Tone::Muted),
            }
            if let Some(duration) = trade.duration_ms() {
                pair(ui, "Held", &format::duration(duration), Tone::Normal);
            }
            pair(
                ui,
                "Quantity",
                &format::quantity(trade.entry_quantity),
                Tone::Normal,
            );
            if trade.is_open() {
                pair(
                    ui,
                    "Still open",
                    &format::quantity(trade.open_quantity),
                    Tone::Normal,
                );
            }
            pair(
                ui,
                "Average entry",
                &format::price(trade.avg_entry),
                Tone::Normal,
            );
            match trade.avg_exit {
                Some(exit) => pair(ui, "Average exit", &format::price(exit), Tone::Normal),
                None => pair(ui, "Average exit", cells::NONE, Tone::Muted),
            }
            let realised = !(trade.is_open() && trade.exit_quantity.is_zero());
            let currency = trade.currency.as_deref().unwrap_or("");
            let money = |value: Decimal| {
                format!("{} {currency}", format::signed_money(value))
                    .trim()
                    .to_owned()
            };
            if realised {
                pair(
                    ui,
                    "Gross P&L",
                    &money(trade.gross_pnl),
                    money_tone(trade.gross_pnl),
                );
                let fees = format::money(trade.fees);
                pair(
                    ui,
                    if trade.fees_known {
                        "Fees"
                    } else {
                        "Fees (known part)"
                    },
                    &fees,
                    Tone::Muted,
                );
                pair(
                    ui,
                    "Net P&L",
                    &money(trade.net_pnl()),
                    money_tone(trade.net_pnl()),
                );
                if let Some(fraction) = return_fraction(trade) {
                    pair(
                        ui,
                        "Return on entry value",
                        &format::signed_percent(fraction),
                        if fraction > 0.0 {
                            Tone::Positive
                        } else if fraction < 0.0 {
                            Tone::Negative
                        } else {
                            Tone::Normal
                        },
                    );
                }
            }
            let stop = draft.review.stop.filter(|stop| *stop > Decimal::ZERO);
            match risk::initial_risk(trade, stop) {
                Metric::Value(risk) => {
                    pair(ui, "Initial risk", &format::money(risk), Tone::Normal);
                    if let Metric::Value(r) = risk::r_multiple(trade, stop) {
                        let r = r.to_f64().unwrap_or(f64::NAN);
                        pair(
                            ui,
                            "Realised R",
                            &format::r_multiple(r),
                            if r > 0.0 {
                                Tone::Positive
                            } else if r < 0.0 {
                                Tone::Negative
                            } else {
                                Tone::Normal
                            },
                        );
                    }
                }
                Metric::Unavailable(reason) => {
                    pair(ui, "Initial risk", &reason.explain(), Tone::Muted);
                }
            }
        });
}

fn executions(ui: &mut Ui, input: &Input<'_>) {
    let trade = input.trade;
    section(ui, &format!("Executions ({})", trade.legs.len()));
    let heads = [
        "Time", "Side", "Qty", "Price", "Fee", "Position", "Realised",
    ];
    egui::Grid::new(("journal-legs", &trade.key))
        .num_columns(heads.len())
        .spacing(vec2(12.0, 6.0))
        .striped(true)
        .show(ui, |ui| {
            for head in heads {
                ui.label(kit::text(
                    head,
                    size::CAPTION,
                    SEMIBOLD,
                    theme::text_tertiary(),
                ));
            }
            ui.end_row();
            for leg in &trade.legs {
                let cell = |ui: &mut Ui, text: String, color: Color32| {
                    ui.label(kit::text(text, size::CAPTION + 1.0, REGULAR, color));
                };
                cell(
                    ui,
                    format::datetime(leg.ts_ms, input.offset),
                    theme::text_primary(),
                );
                let side_color = if leg.side == Side::Buy {
                    theme::WIN
                } else {
                    theme::LOSS
                };
                cell(
                    ui,
                    if leg.side == Side::Buy { "Buy" } else { "Sell" }.to_owned(),
                    side_color,
                );
                cell(ui, format::quantity(leg.quantity), theme::text_primary());
                cell(ui, format::price(leg.price), theme::text_primary());
                cell(
                    ui,
                    leg.fee
                        .map_or_else(|| cells::NONE.to_owned(), format::money),
                    theme::text_secondary(),
                );
                cell(
                    ui,
                    format::quantity(leg.position_after),
                    theme::text_primary(),
                );
                if leg.is_entry {
                    cell(ui, cells::NONE.to_owned(), theme::text_tertiary());
                } else {
                    cell(
                        ui,
                        format::signed_money(leg.realized_gross),
                        tone_color(tone_of(leg.realized_gross)),
                    );
                }
                ui.end_row();
            }
        });
    ui.add_space(4.0);
    ui.label(kit::text(
        "Realised P&L is before fees, by the account's cost basis.",
        size::CAPTION,
        REGULAR,
        theme::text_tertiary(),
    ));
}

/// Best and worst the trade got to while open, and how much of the best the exit kept. Always
/// labelled as an estimate, with the reason when there is no figure.
fn excursion_block(ui: &mut Ui, input: &Input<'_>, output: &mut Output) {
    if matches!(input.excursion, ExcursionView::Hidden) {
        return;
    }
    section(ui, "Excursion");
    let note = |ui: &mut Ui, text: &str| {
        ui.label(kit::text(
            text,
            size::CAPTION + 1.0,
            REGULAR,
            theme::text_tertiary(),
        ));
    };
    match &input.excursion {
        ExcursionView::Hidden => {}
        ExcursionView::Loading => note(ui, "Estimating from candles\u{2026}"),
        ExcursionView::Failed(message) => {
            note(ui, message);
            ui.add_space(4.0);
            if kit::push_button(ui, None, "Try again", true).clicked() {
                output.retry_excursion = true;
            }
        }
        ExcursionView::Ready(Metric::Unavailable(reason)) => note(ui, &reason.explain()),
        ExcursionView::Ready(Metric::Value(value)) => {
            egui::Grid::new(("journal-excursion", &input.trade.key))
                .num_columns(2)
                .spacing(vec2(12.0, 7.0))
                .min_col_width(110.0)
                .show(ui, |ui| {
                    pair(
                        ui,
                        "Best (MFE)",
                        &format::signed_money(value.mfe),
                        tone_of(value.mfe),
                    );
                    pair(
                        ui,
                        "Worst (MAE)",
                        &format::signed_money(-value.mae),
                        tone_of(-value.mae),
                    );
                    pair(
                        ui,
                        "Exit efficiency",
                        &value
                            .exit_efficiency
                            .map_or_else(|| cells::NONE.to_owned(), format::percent),
                        Tone::Normal,
                    );
                });
            ui.add_space(4.0);
            note(ui, &value.provenance());
        }
    }
}

/// A single-line field in the app's form well.
fn field(ui: &mut Ui, id: egui::Id, text: &mut String, hint: &str, error: bool) -> egui::Response {
    let focused = ui.memory(|memory| memory.has_focus(id));
    let mut frame = kit::field_frame(focused);
    if error {
        frame = frame.stroke(egui::Stroke::new(1.5, theme::with_alpha(theme::LOSS, 0.8)));
    }
    frame
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            let width = ui.available_width();
            kit::plain_field(ui, id, text, hint, size::BODY, REGULAR, width)
        })
        .inner
}

fn labelled(ui: &mut Ui, label: &str) {
    ui.add_space(10.0);
    ui.label(kit::text(
        label,
        size::CAPTION + 1.0,
        MEDIUM,
        theme::text_secondary(),
    ));
    ui.add_space(3.0);
}

/// Suggestion chips for a comma-separated list: click one to add or remove it.
fn suggestion_chips(ui: &mut Ui, suggestions: &[String], text: &mut String) {
    if suggestions.is_empty() {
        return;
    }
    let mut items = parse_list(text);
    let mut changed = false;
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(5.0, 5.0);
        for suggestion in suggestions {
            let on = items
                .iter()
                .any(|item| item.eq_ignore_ascii_case(suggestion));
            if kit::value_chip(ui, suggestion, on).clicked() {
                if on {
                    items.retain(|item| !item.eq_ignore_ascii_case(suggestion));
                } else {
                    items.push(suggestion.clone());
                }
                changed = true;
            }
        }
    });
    if changed {
        *text = items.join(", ");
    }
}

/// A 1 to 5 rating as five chips; clicking the chosen one clears it.
fn rating_row(ui: &mut Ui, label: &str, value: &mut Option<u8>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        ui.label(kit::text(
            label,
            size::BODY,
            REGULAR,
            theme::text_secondary(),
        ));
        ui.add_space(6.0);
        for rating in 1..=5u8 {
            if kit::value_chip(ui, &rating.to_string(), *value == Some(rating)).clicked() {
                *value = if *value == Some(rating) {
                    None
                } else {
                    Some(rating)
                };
            }
        }
    });
}

fn review(ui: &mut Ui, input: &Input<'_>, draft: &mut Draft, output: &mut Output) {
    let key = &input.trade.key;
    section(ui, "Review");
    let before = (
        draft.review.clone(),
        draft.tags_text.clone(),
        draft.mistakes_text.clone(),
        draft.emotions_text.clone(),
        draft.stop_text.clone(),
        draft.target_text.clone(),
        draft.planned_entry_text.clone(),
    );

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        for star in 1..=5u8 {
            let filled = draft.review.rating.is_some_and(|rating| rating >= star);
            let (rect, response) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
            icons::paint(
                ui.painter(),
                rect.shrink(2.0),
                if filled { Icon::StarFill } else { Icon::Star },
                if filled {
                    theme::text_primary()
                } else {
                    theme::text_tertiary()
                },
            );
            let response =
                response.on_hover_text(format!("{star} star{}", if star == 1 { "" } else { "s" }));
            if response.clicked() {
                draft.review.rating = if draft.review.rating == Some(star) {
                    None
                } else {
                    Some(star)
                };
            }
        }
        ui.add_space(14.0);
        kit::switch(ui, &mut draft.review.reviewed, "Reviewed");
        ui.label(kit::text(
            "Reviewed",
            size::BODY,
            REGULAR,
            theme::text_primary(),
        ));
    });

    let mut blurred = false;
    let mut text_field = |ui: &mut Ui, salt: &str, text: &mut String, hint: &str, error: bool| {
        let response = field(
            ui,
            egui::Id::new(("journal-review", key, salt)),
            text,
            hint,
            error,
        );
        blurred |= response.lost_focus();
    };

    // Playbook and its checklist.
    labelled(ui, "Playbook");
    let current_name = draft
        .review
        .playbook
        .as_ref()
        .and_then(|id| input.playbooks.iter().find(|p| &p.id == id))
        .map_or("No playbook", |p| p.name.as_str());
    let labels: Vec<&str> = std::iter::once("No playbook")
        .chain(input.playbooks.iter().map(|p| p.name.as_str()))
        .collect();
    let mut chosen: Option<Option<String>> = None;
    header_controls::menu_button(
        ui,
        egui::Id::new(("journal-playbook", key)),
        Icon::Journal,
        current_name,
        &labels,
        false,
        |ui| {
            if header_controls::choice(ui, "No playbook", draft.review.playbook.is_none()) {
                chosen = Some(None);
            }
            for playbook in input.playbooks {
                if header_controls::choice(
                    ui,
                    &playbook.name,
                    draft.review.playbook.as_deref() == Some(playbook.id.as_str()),
                ) {
                    chosen = Some(Some(playbook.id.clone()));
                }
            }
        },
    );
    if let Some(choice) = chosen {
        draft.review.playbook = choice;
        draft.review.checks.clear();
    }
    if let Some(playbook) = draft
        .review
        .playbook
        .as_ref()
        .and_then(|id| input.playbooks.iter().find(|p| &p.id == id))
        && !playbook.checklist.is_empty()
    {
        ui.add_space(6.0);
        for item in &playbook.checklist {
            let mut ticked = draft.review.checks.contains(item);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                kit::switch(ui, &mut ticked, item);
                ui.label(kit::text(item, size::BODY, REGULAR, theme::text_primary()));
            });
            if ticked {
                draft.review.checks.insert(item.clone());
            } else {
                draft.review.checks.remove(item);
            }
        }
        if let Some(share) =
            crate::model::journal::playbook::rule_adherence(&draft.review, playbook)
        {
            ui.add_space(2.0);
            ui.label(kit::text(
                format!("Rule adherence {}", format::percent(share)),
                size::CAPTION + 1.0,
                REGULAR,
                theme::text_tertiary(),
            ));
        }
    }

    labelled(ui, "Setup");
    text_field(
        ui,
        "setup",
        &mut draft.review.setup,
        "What did you see?",
        false,
    );
    labelled(ui, "Tags");
    text_field(ui, "tags", &mut draft.tags_text, "Comma separated", false);
    labelled(ui, "Mistakes");
    text_field(
        ui,
        "mistakes",
        &mut draft.mistakes_text,
        "Comma separated",
        false,
    );
    suggestion_chips(ui, input.mistake_suggestions, &mut draft.mistakes_text);
    labelled(ui, "Emotions");
    text_field(
        ui,
        "emotions",
        &mut draft.emotions_text,
        "Comma separated",
        false,
    );
    suggestion_chips(ui, input.emotion_suggestions, &mut draft.emotions_text);

    ui.add_space(10.0);
    rating_row(ui, "Discipline", &mut draft.review.discipline);
    rating_row(ui, "Confidence", &mut draft.review.confidence);

    // The plan: entry, stop, target.
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let third = (ui.available_width() - 20.0) / 3.0;
        for (label, salt, which) in [
            ("Planned entry", "planned-entry", 0),
            ("Stop", "stop", 1),
            ("Target", "target", 2),
        ] {
            ui.allocate_ui_with_layout(
                vec2(third, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(third);
                    labelled(ui, label);
                    let text = match which {
                        0 => &mut draft.planned_entry_text,
                        1 => &mut draft.stop_text,
                        _ => &mut draft.target_text,
                    };
                    let error = Draft::price_error(text);
                    text_field(ui, salt, text, "Price", error);
                },
            );
        }
    });
    if Draft::price_error(&draft.stop_text)
        || Draft::price_error(&draft.target_text)
        || Draft::price_error(&draft.planned_entry_text)
    {
        ui.add_space(4.0);
        ui.label(kit::text(
            "A price must be a number above zero.",
            size::CAPTION + 1.0,
            REGULAR,
            theme::LOSS,
        ));
    }
    draft.sync_fields();
    plan_readout(ui, input, draft);

    labelled(ui, "Why you entered");
    let id = egui::Id::new(("journal-review", key, "entry-why"));
    let focused = ui.memory(|memory| memory.has_focus(id));
    let response = kit::field_frame(focused)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut draft.review.entry_rationale)
                    .id(id)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .frame(egui::Frame::NONE),
            )
        })
        .inner;
    blurred |= response.lost_focus();
    labelled(ui, "Why you exited");
    let id = egui::Id::new(("journal-review", key, "exit-why"));
    let focused = ui.memory(|memory| memory.has_focus(id));
    let response = kit::field_frame(focused)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut draft.review.exit_rationale)
                    .id(id)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .frame(egui::Frame::NONE),
            )
        })
        .inner;
    blurred |= response.lost_focus();

    labelled(ui, "Notes");
    let id = egui::Id::new(("journal-review", key, "notes"));
    let focused = ui.memory(|memory| memory.has_focus(id));
    let response = kit::field_frame(focused)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut draft.review.notes)
                    .id(id)
                    .desired_rows(6)
                    .desired_width(f32::INFINITY)
                    .frame(egui::Frame::NONE)
                    .hint_text(
                        RichText::new("What happened, and what would you do again?")
                            .color(theme::text_tertiary()),
                    ),
            )
        })
        .inner;
    blurred |= response.lost_focus();

    labelled(ui, "Screenshots");
    output.shots = shots::strip(ui, input.shots, input.shots_busy, input.shots_error);

    draft.sync_fields();
    let after = (
        draft.review.clone(),
        draft.tags_text.clone(),
        draft.mistakes_text.clone(),
        draft.emotions_text.clone(),
        draft.stop_text.clone(),
        draft.target_text.clone(),
        draft.planned_entry_text.clone(),
    );
    output.changed = before != after;
    output.blurred = blurred;

    ui.add_space(8.0);
    let now = ui.input(|input| input.time);
    let status = if let Some(error) = &draft.error {
        Some((error.clone(), theme::LOSS))
    } else if draft.edited_at.is_some() && draft.is_dirty() {
        Some(("Saving\u{2026}".to_owned(), theme::text_tertiary()))
    } else if draft.saved_until.is_some_and(|until| until > now) {
        Some(("Saved".to_owned(), theme::text_secondary()))
    } else {
        None
    };
    if let Some((text, color)) = status {
        ui.label(kit::text(text, size::CAPTION + 1.0, REGULAR, color));
    }
}

/// What the plan implies: reward to risk, and what the risk was as a share of the account.
fn plan_readout(ui: &mut Ui, input: &Input<'_>, draft: &Draft) {
    let trade = input.trade;
    let review = &draft.review;
    let mut lines = Vec::new();
    if let Some(ratio) =
        risk::planned_reward_risk(trade, review.planned_entry, review.stop, review.target)
    {
        lines.push(format!(
            "Planned reward to risk {}",
            format::ratio(ratio)
                .trim_end_matches('0')
                .trim_end_matches('.')
        ));
    }
    if let Metric::Value(risk_amount) = risk::initial_risk(trade, review.stop)
        && let Some(fraction) = risk::risk_fraction(risk_amount, input.account_balance)
    {
        lines.push(format!(
            "Risk {} of the account's starting balance",
            format::percent(fraction)
        ));
    }
    for line in lines {
        ui.label(kit::text(
            line,
            size::CAPTION + 1.0,
            REGULAR,
            theme::text_tertiary(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::journal::model::CostBasis;
    use crate::model::journal::reconstruct::reconstruct;
    use crate::model::journal::testing::{d, exec};

    fn trade() -> Trade {
        reconstruct(
            &[
                exec("1", Side::Buy, "1", "100", 1_000),
                exec("2", Side::Sell, "1", "110", 2_000),
            ],
            |_| CostBasis::AverageCost,
        )
        .remove(0)
    }

    #[test]
    fn a_new_draft_matches_what_is_stored_and_is_not_dirty() {
        let t = trade();
        let draft = Draft::new(&t, None);
        assert!(!draft.is_dirty());
        assert_eq!(draft.review.trade_key, t.key);
    }

    #[test]
    fn editing_makes_it_dirty_and_putting_it_back_does_not() {
        let t = trade();
        let mut draft = Draft::new(&t, None);
        draft.review.notes = "x".into();
        assert!(draft.is_dirty());
        draft.review.notes.clear();
        assert!(!draft.is_dirty());
    }

    #[test]
    fn typed_lists_and_prices_fold_into_the_review() {
        let t = trade();
        let mut draft = Draft::new(&t, None);
        draft.tags_text = "FOMO, fomo ,  late".into();
        draft.stop_text = "1,234.5".into();
        draft.target_text = "x".into();
        draft.sync_fields();
        assert_eq!(draft.review.tags, ["FOMO", "late"]);
        assert_eq!(draft.review.stop, Some(d("1234.5")));
        assert_eq!(draft.review.target, None);
        assert!(Draft::price_error("x"));
        assert!(Draft::price_error("-3"));
        assert!(Draft::price_error("0"));
        assert!(!Draft::price_error(""));
        assert!(!Draft::price_error("12.5"));
    }

    #[test]
    fn an_existing_review_fills_the_text_boxes() {
        let t = trade();
        let mut review = Review::new(&t.key, &t.account_uid);
        review.tags = vec!["a".into(), "b".into()];
        review.stop = Some(d("95"));
        let draft = Draft::new(&t, Some(&review));
        assert_eq!(draft.tags_text, "a, b");
        assert_eq!(draft.stop_text, "95");
        assert!(!draft.is_dirty());
    }

    #[test]
    fn the_panel_draws_for_a_closed_and_an_open_trade() {
        let ctx = egui::Context::default();
        let closed = trade();
        let open = reconstruct(&[exec("1", Side::Buy, "1", "100", 1_000)], |_| {
            CostBasis::AverageCost
        })
        .remove(0);
        for t in [closed, open] {
            let mut draft = Draft::new(&t, None);
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::pos2(0.0, 0.0),
                        vec2(460.0, 900.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let result = show(
                        ui,
                        &Input {
                            trade: &t,
                            account_name: "Main",
                            breakeven_tolerance: Decimal::ZERO,
                            offset: &crate::model::journal::calendar::utc,
                            position: (1, 3),
                            playbooks: &[],
                            emotion_suggestions: &[],
                            mistake_suggestions: &[],
                            account_balance: None,
                            shots: &[],
                            shots_busy: false,
                            shots_error: None,
                            excursion: ExcursionView::Hidden,
                            can_view_on_chart: true,
                        },
                        &mut draft,
                    );
                    assert!(!result.close && !result.changed);
                },
            );
            out.textures_delta.clear();
        }
    }
}
