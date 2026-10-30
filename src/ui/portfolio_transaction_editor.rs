//! Add, edit or duplicate one portfolio transaction. A pure view over a `TransactionDraft`
//! (parsing, the price/total link, validation and the summary live in
//! `model::portfolio_editor`) that returns an outcome; the host saves it to the ledger and
//! reports a rejection back through `TransactionEditorState::failed`.

// Transient: the portfolio host opens the editor once it is wired.
#![allow(dead_code)]

use crate::model::portfolio::{PortfolioCurrency, PortfolioTransaction, PortfolioTransactionType};
use crate::model::portfolio_editor::{Category, EditorField, Summary, TransactionDraft};
use crate::ui::date_picker;
use crate::ui::icons::Icon;
use crate::ui::kit::{self, TabItem};
use crate::ui::portfolio_transactions::{type_name, type_tint};
use crate::ui::sheet;
use crate::ui::theme;
use eframe::egui::{self, Color32, Id, Rect, RichText, Stroke, Vec2};

pub const WIDTH: f32 = 460.0;

fn date_id() -> Id {
    Id::new("portfolio-transaction-date")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditorMode {
    New,
    Edit,
    Duplicate,
}

impl EditorMode {
    pub fn title(self) -> &'static str {
        match self {
            Self::New => "Add Transaction",
            Self::Edit => "Edit Transaction",
            Self::Duplicate => "Duplicate Transaction",
        }
    }

    pub fn submit_title(self) -> &'static str {
        match self {
            Self::Edit => "Save Changes",
            Self::New | Self::Duplicate => "Add Transaction",
        }
    }
}

pub struct TransactionEditorState {
    pub mode: EditorMode,
    base: PortfolioTransaction,
    pub draft: TransactionDraft,
    pub submitting: bool,
    pub error: Option<String>,
    /// The asset's price now, in the transaction's currency; None until it arrives.
    pub market_price: Option<f64>,
    first_frame: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TransactionEditorOutcome {
    /// Save it: `Edit` replaces the transaction with this id, the others add a new one.
    Save {
        transaction: Box<PortfolioTransaction>,
        mode: EditorMode,
    },
    Cancel,
}

impl TransactionEditorState {
    pub fn new(mode: EditorMode, transaction: PortfolioTransaction, utc_offset_secs: i64) -> Self {
        Self {
            mode,
            draft: TransactionDraft::new(&transaction, utc_offset_secs),
            base: transaction,
            submitting: false,
            error: None,
            market_price: None,
            first_frame: true,
        }
    }

    pub fn asset_ticker(&self) -> String {
        self.base.asset.display_ticker()
    }

    pub fn price_currency(&self) -> PortfolioCurrency {
        self.base.price_currency
    }

    /// The host looked the price up: a new transaction opens with it in the price field; an edit
    /// or a duplicate keeps its own price and only offers this through the field's Market button.
    pub fn set_market_price(&mut self, price: f64) {
        self.market_price = Some(price);
        if self.mode == EditorMode::New {
            self.draft.prefill_price(price);
        }
    }

    /// The ledger rejected the save: the sheet stays open and says why.
    pub fn failed(&mut self, message: impl Into<String>) {
        self.submitting = false;
        self.error = Some(message.into());
    }

    fn subtitle(&self) -> String {
        format!(
            "{} · {}",
            self.base.asset.display_ticker(),
            self.base.asset.source.display_name()
        )
    }
}

// MARK: Pure text

pub fn price_label(ticker: &str) -> String {
    format!("Price per {ticker}")
}

pub fn price_hint(requires_price: bool) -> &'static str {
    if requires_price {
        "Required"
    } else {
        "Optional"
    }
}

pub fn summary_rows(summary: &Summary, currency: PortfolioCurrency) -> Vec<(String, String, bool)> {
    let mut rows = vec![("Value".to_owned(), currency.format(summary.value), false)];
    if summary.fee > 0.0 {
        rows.push(("Fee".to_owned(), currency.format(summary.fee), false));
    }
    if let Some((label, amount)) = summary.net {
        rows.push((label.to_owned(), currency.format(amount), true));
    }
    rows
}

pub fn primary_color(kind: PortfolioTransactionType) -> Color32 {
    if kind == PortfolioTransactionType::Adjustment {
        theme::ACCENT
    } else {
        type_tint(kind)
    }
}

// MARK: View

pub fn show(
    ctx: &egui::Context,
    state: &mut TransactionEditorState,
) -> Option<TransactionEditorOutcome> {
    let mut outcome = None;
    let previous_draft = state.draft.clone();
    // A calendar left open by an earlier editor must not greet this one; and Escape closes an
    // open calendar before it closes the sheet.
    if state.first_frame {
        date_picker::close(ctx, date_id());
    }
    let picker_was_open = date_picker::is_open(ctx, date_id());
    sheet::show(ctx, Id::new("portfolio-transaction-editor"), WIDTH, |ui| {
        egui::ScrollArea::vertical()
            .max_height((ctx.content_rect().height() - 120.0).clamp(240.0, 650.0))
            .show(ui, |ui| {
                ui.add_space(24.0);
                sheet::inset(ui, |ui| form(ui, state));
                ui.add_space(20.0);
            });
        ui.separator();
        sheet::inset(ui, |ui| {
            ui.add_space(12.0);
            footer(ui, state, &mut outcome);
            ui.add_space(8.0);
        });
    });
    if state.draft != previous_draft {
        state.error = None;
    }
    state.first_frame = false;
    if outcome.is_none()
        && !state.submitting
        && !picker_was_open
        && ctx.input(|i| i.key_pressed(egui::Key::Escape))
    {
        outcome = Some(TransactionEditorOutcome::Cancel);
    }
    if outcome.is_some() {
        date_picker::close(ctx, date_id());
    }
    outcome
}

fn label(ui: &mut egui::Ui, text: &str) {
    ui.label(kit::text(
        text,
        kit::size::CAPTION + 0.5,
        kit::MEDIUM,
        theme::text_secondary(),
    ));
}

fn footnote(ui: &mut egui::Ui, error: Option<&String>, hint: Option<&str>) {
    if let Some(error) = error {
        ui.label(RichText::new(error).small().color(theme::DOWN));
    } else if let Some(hint) = hint {
        ui.label(RichText::new(hint).small().weak());
    }
}

fn field(
    ui: &mut egui::Ui,
    id: &str,
    text: &mut String,
    hint: &str,
    suffix: &str,
    error: bool,
    (width, accessory): (f32, bool),
) -> (egui::Response, bool) {
    let field_id = ui.make_persistent_id(id);
    let focused = ui.memory(|memory| memory.has_focus(field_id));
    let mut accessory_clicked = false;
    // The outline is painted over the frame rather than being part of it: a frame's stroke
    // adds to its size, which made fields wider than their row and grew them when focused.
    let frame = egui::Frame::new()
        .fill(theme::quaternary(0.5))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(11, 8))
        .show(ui, |ui| {
            ui.set_width((width - 22.0).max(80.0));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                // What follows the text, measured, so the row fits the frame exactly and every
                // field in a column ends on the same edge.
                let measure = |text: &str, size: f32, weight: f32| {
                    kit::galley(ui, text, size, weight, Color32::WHITE, 400.0)
                        .size()
                        .x
                };
                let suffix_width = measure(suffix, kit::size::SUBHEADLINE + 1.0, kit::REGULAR);
                let market_width = if accessory {
                    measure("Market", kit::size::CAPTION, kit::SEMIBOLD)
                        + 2.0 * ui.spacing().button_padding.x
                        + 7.0
                } else {
                    0.0
                };
                let trailing = suffix_width + 7.0 + market_width;
                let response = ui.add(
                    egui::TextEdit::singleline(text)
                        .id(field_id)
                        .frame(egui::Frame::NONE)
                        .font(egui::FontId::proportional(kit::size::BODY))
                        .hint_text(hint)
                        .desired_width((ui.available_width() - trailing).max(36.0)),
                );
                if accessory
                    && ui
                        .add(
                            egui::Button::new(kit::text(
                                "Market",
                                kit::size::CAPTION,
                                kit::SEMIBOLD,
                                theme::SYSTEM_ACCENT,
                            ))
                            .frame(false),
                        )
                        .clicked()
                {
                    accessory_clicked = true;
                }
                ui.label(kit::text(
                    suffix,
                    kit::size::SUBHEADLINE + 1.0,
                    kit::REGULAR,
                    theme::text_secondary(),
                ));
                response
            })
            .inner
        });
    let (stroke_width, stroke_color) = if error {
        (1.5, theme::DOWN)
    } else if focused {
        (1.5, theme::text_secondary())
    } else {
        (1.0, theme::separator().gamma_multiply(0.6))
    };
    ui.painter().rect_stroke(
        frame.response.rect,
        8.0,
        Stroke::new(stroke_width, stroke_color),
        egui::StrokeKind::Inside,
    );
    (frame.inner, accessory_clicked)
}

fn form(ui: &mut egui::Ui, state: &mut TransactionEditorState) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(46.0), egui::Sense::hover());
        crate::ui::portfolio_table::paint_asset_icon(
            ui,
            Rect::from_center_size(rect.center(), Vec2::splat(40.0)),
            &state.base.asset,
        );
        kit::source_badge(
            ui,
            Rect::from_center_size(rect.center(), Vec2::splat(40.0)),
            state.base.asset.source,
        );
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(kit::text(
                state.mode.title(),
                kit::size::TITLE2 + 1.0,
                kit::SEMIBOLD,
                theme::text_primary(),
            ));
            ui.label(kit::text(
                state.subtitle(),
                kit::size::SUBHEADLINE + 1.0,
                kit::REGULAR,
                theme::text_secondary(),
            ));
        });
    });
    ui.add_space(16.0);

    // Type: category tabs, then the types inside a multi-type category.
    let mut category = state.draft.category();
    kit::tab_bar(
        ui,
        Id::new("portfolio-transaction-category"),
        &[
            TabItem {
                value: Category::Buy,
                title: "Buy",
                icon: Icon::Plus,
                count: 0,
            },
            TabItem {
                value: Category::Sell,
                title: "Sell",
                icon: Icon::ArrowDown,
                count: 0,
            },
            TabItem {
                value: Category::Transfer,
                title: "Transfer",
                icon: Icon::ArrowLeftRight,
                count: 0,
            },
            TabItem {
                value: Category::Income,
                title: "Income",
                icon: Icon::Banknote,
                count: 0,
            },
            TabItem {
                value: Category::Other,
                title: "Other",
                icon: Icon::Ellipsis,
                count: 0,
            },
        ],
        &mut category,
    );
    if category != state.draft.category() {
        state.draft.select(category);
    }
    let types = state.draft.category().types();
    if types.len() > 1 {
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for kind in types {
                let selected = state.draft.kind == *kind;
                let tint = type_tint(*kind);
                let button = egui::Button::new(kit::text(
                    type_name(*kind),
                    kit::size::CAPTION + 0.5,
                    kit::SEMIBOLD,
                    if selected {
                        tint
                    } else {
                        theme::text_secondary()
                    },
                ))
                .fill(if selected {
                    theme::with_alpha(tint, 0.14)
                } else {
                    theme::quaternary(0.35)
                })
                .corner_radius(999.0);
                if ui.add(button).clicked() {
                    state.draft.kind = *kind;
                }
            }
        });
    }
    ui.add_space(16.0);

    let issues = state.draft.issues();
    let ticker = state.asset_ticker();
    let currency = state.price_currency();

    label(ui, "Quantity");
    let quantity = field(
        ui,
        "quantity",
        &mut state.draft.quantity_text,
        "0",
        &ticker,
        issues.contains_key(&EditorField::Quantity),
        (ui.available_width(), false),
    )
    .0;
    if std::mem::take(&mut state.first_frame) {
        quantity.request_focus();
    }
    footnote(ui, issues.get(&EditorField::Quantity), None);
    ui.add_space(16.0);

    if state.draft.uses_price() {
        price_fields(ui, state, &issues, currency, &ticker);
        ui.add_space(16.0);
    }

    let date_width = (ui.available_width() - 162.0).max(140.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        ui.vertical(|ui| {
            ui.set_width(150.0);
            label(ui, "Fee");
            field(
                ui,
                "fee",
                &mut state.draft.fee_text,
                "0",
                state.base.fee_currency.code(),
                issues.contains_key(&EditorField::Fee),
                (150.0, false),
            );
            footnote(ui, issues.get(&EditorField::Fee), None);
        });
        ui.vertical(|ui| {
            ui.set_width(date_width);
            label(ui, "Date & time");
            let zone = state.draft.utc_offset_secs();
            date_picker::show(
                ui,
                date_id(),
                &mut state.draft.date_text,
                zone,
                date_width,
                date_picker::Look::Roomy,
                issues.contains_key(&EditorField::Date),
            );
            footnote(ui, issues.get(&EditorField::Date), None);
        });
    });
    ui.add_space(16.0);

    label(ui, "Notes");
    let notes_id = ui.make_persistent_id("portfolio-transaction-notes");
    let notes_focused = ui.memory(|memory| memory.has_focus(notes_id));
    egui::Frame::new()
        .fill(theme::quaternary(0.5))
        .corner_radius(8.0)
        .stroke(if notes_focused {
            Stroke::new(1.5, theme::text_secondary())
        } else {
            Stroke::new(1.0, theme::separator().gamma_multiply(0.6))
        })
        .inner_margin(egui::Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                egui::TextEdit::multiline(&mut state.draft.notes)
                    .id(notes_id)
                    .frame(egui::Frame::NONE)
                    .hint_text("Add a note (optional)")
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            );
        });
    ui.add_space(16.0);

    if let Some(summary) = state.draft.summary() {
        egui::Frame::new()
            .fill(theme::ink(10))
            .stroke(egui::Stroke::new(1.0, theme::border().gamma_multiply(0.6)))
            .corner_radius(10.0)
            .inner_margin(egui::Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                for (title, value, emphasized) in summary_rows(&summary, currency) {
                    if emphasized {
                        ui.separator();
                    }
                    ui.horizontal(|ui| {
                        ui.label(kit::text(
                            title,
                            12.0,
                            if emphasized {
                                kit::SEMIBOLD
                            } else {
                                kit::REGULAR
                            },
                            if emphasized {
                                theme::text_primary()
                            } else {
                                theme::text_secondary()
                            },
                        ));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(kit::text(
                                value,
                                12.0,
                                if emphasized {
                                    kit::SEMIBOLD
                                } else {
                                    kit::MEDIUM
                                },
                                theme::text_primary(),
                            ));
                        });
                    });
                    ui.add_space(4.0);
                }
            });
    }
    if let Some(error) = &state.error {
        ui.add_space(12.0);
        kit::notice_card(
            ui,
            Icon::TriangleFill,
            theme::DOWN,
            "Transaction not saved",
            Some(error),
            None,
        );
    }
    ui.add_space(8.0);
}

fn price_fields(
    ui: &mut egui::Ui,
    state: &mut TransactionEditorState,
    issues: &std::collections::HashMap<EditorField, String>,
    currency: PortfolioCurrency,
    ticker: &str,
) {
    ui.columns(2, |columns| {
        let price_width = columns[0].available_width();
        let total_width = columns[1].available_width();
        label(&mut columns[0], &price_label(ticker));
        let mut price = state.draft.displayed_price();
        let changed = field(
            &mut columns[0],
            "price",
            &mut price,
            "0.00",
            currency.code(),
            issues.contains_key(&EditorField::Price),
            (price_width, state.market_price.is_some()),
        );
        let (response, market_clicked) = changed;
        let changed = response.changed();
        if changed {
            state.draft.set_price(&price);
        }
        if market_clicked && let Some(market) = state.market_price {
            let text = state.draft.text_for_market_price(market);
            state.draft.set_price(&text);
        }
        footnote(
            &mut columns[0],
            issues.get(&EditorField::Price),
            Some(price_hint(state.draft.requires_price())),
        );

        label(&mut columns[1], "Total");
        let mut total = state.draft.displayed_total();
        let changed = field(
            &mut columns[1],
            "total",
            &mut total,
            "0.00",
            currency.code(),
            issues.contains_key(&EditorField::Total),
            (total_width, false),
        )
        .0
        .changed();
        if changed {
            state.draft.set_total(&total);
        }
        footnote(&mut columns[1], issues.get(&EditorField::Total), None);
    });
}

fn footer(
    ui: &mut egui::Ui,
    state: &mut TransactionEditorState,
    outcome: &mut Option<TransactionEditorOutcome>,
) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if kit::footer_button(
            ui,
            "Cancel",
            kit::FooterButton::Secondary,
            !state.submitting,
            0.0,
        )
        .clicked()
        {
            *outcome = Some(TransactionEditorOutcome::Cancel);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let submit = kit::footer_button(
                ui,
                state.mode.submit_title(),
                kit::FooterButton::Prominent(primary_color(state.draft.kind)),
                state.draft.can_submit() && !state.submitting,
                130.0,
            );
            if state.submitting {
                ui.spinner();
            }
            if submit.clicked()
                && let Some(transaction) = state
                    .draft
                    .make_transaction(&state.base, crate::model::watchlist::now_ms())
            {
                state.submitting = true;
                state.error = None;
                *outcome = Some(TransactionEditorOutcome::Save {
                    transaction: Box::new(transaction),
                    mode: state.mode,
                });
            }
        });
    });
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::{PortfolioAsset, PortfolioSource};
    use std::collections::HashMap;

    fn transaction(price: Option<f64>) -> PortfolioTransaction {
        PortfolioTransaction {
            id: 5,
            portfolio_id: 1,
            asset: PortfolioAsset {
                key: "Binance:BTCUSDT".into(),
                symbol: "BTC/USDT".into(),
                name: "Bitcoin".into(),
                source: DataSource::Binance,
                quote_currency: PortfolioCurrency::Usd,
                metadata: HashMap::new(),
            },
            kind: PortfolioTransactionType::Buy,
            quantity: if price.is_some() { 0.5 } else { 0.0 },
            price,
            price_currency: PortfolioCurrency::Usd,
            fee: 0.0,
            fee_currency: PortfolioCurrency::Usd,
            timestamp: 1_709_649_000_000,
            notes: String::new(),
            source: PortfolioSource::Manual,
            external_transaction_id: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn modes_read_in_words() {
        assert_eq!(EditorMode::New.title(), "Add Transaction");
        assert_eq!(EditorMode::Edit.title(), "Edit Transaction");
        assert_eq!(EditorMode::Duplicate.title(), "Duplicate Transaction");
        assert_eq!(EditorMode::Edit.submit_title(), "Save Changes");
        assert_eq!(EditorMode::Duplicate.submit_title(), "Add Transaction");
        assert_eq!(price_label("BTC"), "Price per BTC");
        assert_eq!(price_hint(true), "Required");
        assert_eq!(price_hint(false), "Optional");
    }

    #[test]
    fn only_a_new_transaction_takes_the_market_price_as_its_own() {
        let mut new = TransactionEditorState::new(EditorMode::New, transaction(None), 0);
        new.set_market_price(67_432.19);
        assert_eq!(new.draft.displayed_price(), "67,432.19");
        assert_eq!(new.market_price, Some(67_432.19));
        let mut edit = TransactionEditorState::new(EditorMode::Edit, transaction(None), 0);
        edit.set_market_price(67_432.19);
        assert_eq!(
            edit.draft.displayed_price(),
            "",
            "offered through the Market button only"
        );
        assert_eq!(edit.market_price, Some(67_432.19));
        let mut duplicate =
            TransactionEditorState::new(EditorMode::Duplicate, transaction(Some(50.0)), 0);
        duplicate.set_market_price(1.0);
        assert_eq!(duplicate.draft.displayed_price(), "50.00");
    }

    #[test]
    fn a_rejected_save_reopens_the_form_with_the_reason() {
        let mut state = TransactionEditorState::new(EditorMode::Edit, transaction(Some(10.0)), 0);
        state.submitting = true;
        state.failed("Sell exceeds the holding.");
        assert!(!state.submitting);
        assert_eq!(state.error.as_deref(), Some("Sell exceeds the holding."));
        assert_eq!(state.subtitle(), "BTC · Binance");
        assert_eq!(state.asset_ticker(), "BTC");
    }

    #[test]
    fn the_summary_card_lists_value_fee_and_the_net() {
        let summary = Summary {
            value: 1_000.0,
            fee: 2.5,
            net: Some(("Total cost", 1_002.5)),
        };
        let rows = summary_rows(&summary, PortfolioCurrency::Usd);
        assert_eq!(
            rows,
            vec![
                ("Value".to_owned(), "$1,000.00".to_owned(), false),
                ("Fee".to_owned(), "$2.50".to_owned(), false),
                ("Total cost".to_owned(), "$1,002.50".to_owned(), true),
            ]
        );
        let bare = Summary {
            value: 5.0,
            fee: 0.0,
            net: None,
        };
        assert_eq!(summary_rows(&bare, PortfolioCurrency::Usd).len(), 1);
        assert_eq!(
            primary_color(PortfolioTransactionType::Adjustment),
            theme::ACCENT
        );
        assert_eq!(primary_color(PortfolioTransactionType::Sell), theme::DOWN);
    }
}
