//! Create a paper account, or reset the selected one. A reset keeps the account but wipes its
//! positions, orders, fills and journal, so it opens on a red warning and starts from the
//! account's current settings rather than defaults. A pure view: it returns the chosen values and
//! the host applies them to the store.

// Transient: the paper-trading host opens this once it is wired.
#![allow(dead_code)]

use crate::model::paper::{
    PaperAccount, PaperAccountSettings, PaperCommissionConfiguration, PaperCurrency,
    PaperLeverageConfiguration,
};
use crate::model::paper_format as fmt;
use crate::model::paper_ticket::{decimal_text, parse_decimal};
use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::paper_form::{dropdown_field, field_footer, field_label, text_field};
use crate::ui::sheet;
use crate::ui::theme;
use eframe::egui::{self, Id};

pub const WIDTH: f32 = 460.0;
pub const LEVERAGE_PRESETS: [f64; 7] = [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0];
pub const MAX_LEVERAGE: f64 = 1000.0;
pub const DEFAULT_NAME: &str = "Paper Trading";
pub const DEFAULT_BALANCE: f64 = 100_000.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommissionKind {
    None,
    Fixed,
    Percentage,
    PerContract,
}

impl CommissionKind {
    pub const ALL: [Self; 4] = [Self::None, Self::Fixed, Self::Percentage, Self::PerContract];

    pub fn title(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Fixed => "Fixed",
            Self::Percentage => "Percent",
            Self::PerContract => "Per contract",
        }
    }

    /// The unit beside the amount.
    pub fn unit(self, currency: PaperCurrency) -> String {
        match self {
            Self::None => String::new(),
            Self::Fixed => format!("{} per order", currency.code()),
            Self::Percentage => "% of value".to_owned(),
            Self::PerContract => format!("{} per contract", currency.code()),
        }
    }

    fn of(configuration: PaperCommissionConfiguration) -> (Self, f64) {
        match configuration {
            PaperCommissionConfiguration::None => (Self::None, 0.0),
            PaperCommissionConfiguration::FixedPerOrder { value } => (Self::Fixed, value),
            PaperCommissionConfiguration::Percentage { value } => (Self::Percentage, value),
            PaperCommissionConfiguration::PerContract { value } => (Self::PerContract, value),
        }
    }

    fn configuration(self, value: f64) -> PaperCommissionConfiguration {
        match self {
            Self::None => PaperCommissionConfiguration::None,
            Self::Fixed => PaperCommissionConfiguration::FixedPerOrder { value },
            Self::Percentage => PaperCommissionConfiguration::Percentage { value },
            Self::PerContract => PaperCommissionConfiguration::PerContract { value },
        }
    }
}

/// What the sheet hands the host on Create / Reset.
#[derive(Clone, Debug, PartialEq)]
pub struct AccountSheetResult {
    pub name: String,
    pub currency: PaperCurrency,
    pub balance: f64,
    pub settings: PaperAccountSettings,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AccountSheetOutcome {
    Submit(Box<AccountSheetResult>),
    Cancel,
}

pub struct PaperAccountSheetState {
    pub is_reset: bool,
    /// The settings a reset keeps (slippage, chart toggles); a new account starts from defaults.
    base_settings: PaperAccountSettings,
    pub name: String,
    pub balance_text: String,
    pub currency: PaperCurrency,
    pub leverage_text: String,
    pub commission_kind: CommissionKind,
    pub commission_text: String,
    /// The name field takes the keyboard when a new sheet opens; a reset leaves the name alone.
    focus_pending: bool,
}

impl PaperAccountSheetState {
    pub fn create() -> Self {
        Self {
            is_reset: false,
            base_settings: PaperAccountSettings::default(),
            name: DEFAULT_NAME.to_owned(),
            balance_text: decimal_text(DEFAULT_BALANCE, false),
            currency: PaperCurrency::Usd,
            leverage_text: "1".to_owned(),
            commission_kind: CommissionKind::None,
            commission_text: "0".to_owned(),
            focus_pending: true,
        }
    }

    /// Starts from the account's current settings, not the defaults.
    pub fn reset(account: &PaperAccount) -> Self {
        let (commission_kind, commission) = CommissionKind::of(account.settings.commission);
        Self {
            is_reset: true,
            base_settings: account.settings.clone(),
            name: account.name.clone(),
            balance_text: decimal_text(account.initial_balance, false),
            currency: account.base_currency,
            leverage_text: decimal_text(account.settings.leverage.crypto, false),
            commission_kind,
            commission_text: decimal_text(commission, false),
            focus_pending: false,
        }
    }

    pub fn balance(&self) -> Option<f64> {
        parse_decimal(&self.balance_text, false)
    }

    pub fn leverage(&self) -> Option<f64> {
        parse_decimal(&self.leverage_text, false)
    }

    pub fn commission_value(&self) -> Option<f64> {
        parse_decimal(&self.commission_text, false)
    }

    pub fn balance_error(&self) -> Option<&'static str> {
        match self.balance() {
            None => Some("Enter a number."),
            Some(balance) if balance > 0.0 => None,
            Some(_) => Some("Must be greater than zero."),
        }
    }

    pub fn leverage_error(&self) -> Option<&'static str> {
        match self.leverage() {
            None => Some("Enter a number."),
            Some(leverage) if (1.0..=MAX_LEVERAGE).contains(&leverage) => None,
            Some(_) => Some("Must be between 1× and 1000×."),
        }
    }

    pub fn commission_error(&self) -> Option<&'static str> {
        if self.commission_kind == CommissionKind::None {
            return None;
        }
        match self.commission_value() {
            None => Some("Enter a number."),
            Some(value) if value >= 0.0 => None,
            Some(_) => Some("Can't be negative."),
        }
    }

    pub fn can_submit(&self) -> bool {
        !self.name.trim().is_empty()
            && self.balance_error().is_none()
            && self.leverage_error().is_none()
            && self.commission_error().is_none()
    }

    /// The balance error, once something has been typed.
    pub fn visible_balance_error(&self) -> Option<&'static str> {
        (!self.balance_text.is_empty())
            .then(|| self.balance_error())
            .flatten()
    }

    pub fn visible_leverage_error(&self) -> Option<&'static str> {
        (!self.leverage_text.is_empty())
            .then(|| self.leverage_error())
            .flatten()
    }

    pub fn title(&self) -> &'static str {
        if self.is_reset {
            "Reset Paper Account"
        } else {
            "New Paper Account"
        }
    }

    pub fn subtitle(&self) -> &'static str {
        if self.is_reset {
            "Start over with a fresh balance."
        } else {
            "Practice with simulated money."
        }
    }

    pub fn submit_title(&self) -> &'static str {
        if self.is_reset {
            "Reset Account"
        } else {
            "Create Account"
        }
    }

    /// "Balance resets to $100,000.00." for a reset with a valid balance.
    pub fn reset_note(&self) -> Option<String> {
        if !self.is_reset {
            return None;
        }
        self.balance()
            .map(|balance| format!("Balance resets to {}.", fmt::money(balance, self.currency)))
    }

    /// One leverage applies to every market, as in the Swift sheet.
    pub fn result(&self) -> Option<AccountSheetResult> {
        if !self.can_submit() {
            return None;
        }
        let (balance, leverage) = (self.balance()?, self.leverage()?);
        let fee = self.commission_value().unwrap_or(0.0);
        let mut settings = self.base_settings.clone();
        settings.commission = self.commission_kind.configuration(fee);
        settings.leverage = PaperLeverageConfiguration {
            stocks: leverage,
            crypto: leverage,
            forex: leverage,
            futures: leverage,
            prediction: leverage,
        };
        Some(AccountSheetResult {
            name: self.name.trim().to_owned(),
            currency: self.currency,
            balance,
            settings,
        })
    }
}

// MARK: View

/// Space between the sheet's sections (Swift's `VStack(spacing: 18)`).
const GAP: f32 = 18.0;

pub fn show(
    ctx: &egui::Context,
    state: &mut PaperAccountSheetState,
) -> Option<AccountSheetOutcome> {
    let mut outcome = None;
    sheet::show(ctx, Id::new("paper-account-sheet"), WIDTH, |ui| {
        ui.add_space(24.0);
        sheet::inset(ui, |ui| form(ui, state));
        ui.add_space(24.0);
        footer(ui, state, &mut outcome);
    });
    if outcome.is_none() && ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        outcome = Some(AccountSheetOutcome::Cancel);
    }
    outcome
}

fn form(ui: &mut egui::Ui, state: &mut PaperAccountSheetState) {
    ui.spacing_mut().item_spacing.y = 0.0;
    let tint = if state.is_reset {
        kit::RED
    } else {
        theme::text_primary()
    };
    sheet::header(
        ui,
        if state.is_reset {
            Icon::Undo
        } else {
            Icon::Plus
        },
        state.title(),
        state.subtitle(),
        tint,
    );
    ui.add_space(GAP);

    if state.is_reset {
        kit::notice_card(
            ui,
            Icon::TriangleFill,
            kit::RED,
            "This can't be undone",
            Some(
                "Every position, order, fill, history and journal entry in this account is removed.",
            ),
            None,
        );
        ui.add_space(GAP);
    }

    field_label(ui, "Account name");
    if std::mem::take(&mut state.focus_pending) {
        ui.memory_mut(|memory| memory.request_focus(Id::new(("paper-text-field", "account-name"))));
    }
    ui.add_enabled_ui(!state.is_reset, |ui| {
        text_field(
            ui,
            "account-name",
            &mut state.name,
            DEFAULT_NAME,
            "",
            false,
            None,
        );
    });
    ui.add_space(GAP);

    let balance_error = state.visible_balance_error();
    let code = state.currency.code();
    let labels: Vec<String> = PaperCurrency::ALL
        .iter()
        .map(|currency| currency.code().to_owned())
        .collect();
    let selected = PaperCurrency::ALL
        .iter()
        .position(|currency| *currency == state.currency)
        .unwrap_or(0);
    let width = ui.available_width();
    let currency_width = 120.0;
    let gap = 12.0;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        ui.allocate_ui_with_layout(
            egui::vec2(width - currency_width - gap, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(width - currency_width - gap);
                field_label(ui, "Starting balance");
                text_field(
                    ui,
                    "account-balance",
                    &mut state.balance_text,
                    "100000",
                    code,
                    balance_error.is_some(),
                    None,
                );
                field_footer(ui, balance_error.map(str::to_owned).as_ref(), None);
            },
        );
        ui.add_space(gap);
        ui.allocate_ui_with_layout(
            egui::vec2(currency_width, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(currency_width);
                field_label(ui, "Currency");
                if let Some(index) = dropdown_field(ui, &labels, selected, true) {
                    state.currency = PaperCurrency::ALL[index];
                }
            },
        );
    });
    ui.add_space(GAP);

    leverage_field(ui, state);
    ui.add_space(GAP);
    commission_field(ui, state);

    if let Some(note) = state.reset_note() {
        ui.add_space(GAP);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            icons::inline(ui, Icon::Undo, 11.0, theme::text_secondary());
            ui.label(kit::text(
                note,
                kit::size::CAPTION + 1.0,
                kit::REGULAR,
                theme::text_secondary(),
            ));
        });
    }
}

/// The action bar: a hairline across the whole sheet, Cancel on the left and the one primary
/// action on the right, as the order ticket's footer is laid out.
fn footer(
    ui: &mut egui::Ui,
    state: &PaperAccountSheetState,
    outcome: &mut Option<AccountSheetOutcome>,
) {
    let (line, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(line, 0.0, theme::separator());
    sheet::inset(ui, |ui| {
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            if kit::large_button(ui, "Cancel").clicked() {
                *outcome = Some(AccountSheetOutcome::Cancel);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let enabled = state.can_submit();
                let clicked = if state.is_reset {
                    kit::tinted_button(ui, state.submit_title(), kit::RED, enabled, false, 130.0)
                } else {
                    kit::large_prominent_button(ui, state.submit_title(), enabled, 130.0)
                }
                .clicked();
                let entered = enabled && ui.input(|input| input.key_pressed(egui::Key::Enter));
                if (clicked || entered)
                    && let Some(result) = state.result()
                {
                    *outcome = Some(AccountSheetOutcome::Submit(Box::new(result)));
                }
            });
        });
        ui.add_space(14.0);
    });
}

fn leverage_field(ui: &mut egui::Ui, state: &mut PaperAccountSheetState) {
    field_label(ui, "Leverage");
    let error = state.visible_leverage_error();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.allocate_ui(egui::vec2(110.0, 34.0), |ui| {
            text_field(
                ui,
                "account-leverage",
                &mut state.leverage_text,
                "1",
                "×",
                error.is_some(),
                None,
            );
        });
        let current = state.leverage();
        // The presets share what the field leaves, so the row ends where the others do.
        let gap = ui.spacing().item_spacing.x;
        let count = LEVERAGE_PRESETS.len() as f32;
        let each = ((ui.available_width() - gap * (count - 1.0)) / count).max(32.0);
        for preset in LEVERAGE_PRESETS {
            let selected = current == Some(preset);
            if leverage_preset(ui, preset, selected, each).clicked() {
                state.leverage_text = decimal_text(preset, false);
            }
        }
    });
    field_footer(
        ui,
        error.map(str::to_owned).as_ref(),
        Some("Applies to every market. 1× means no borrowing."),
    );
}

/// A "10×" capsule, tinted when it is the current leverage.
fn leverage_preset(ui: &mut egui::Ui, preset: f64, selected: bool, width: f32) -> egui::Response {
    let label = format!("{preset}×");
    let color = if selected {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    let galley = kit::galley(
        ui,
        &label,
        kit::size::CAPTION + 1.0,
        kit::SEMIBOLD,
        color,
        f32::INFINITY,
    );
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 28.0), egui::Sense::click());
    let response = crate::ui::accessibility::label(response, label)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let fill = if selected {
        kit::primary(0.2)
    } else if response.hovered() {
        kit::primary(0.09)
    } else {
        kit::primary(0.05)
    };
    ui.painter().rect_filled(rect, 8.0, fill);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, color);
    response
}

fn commission_field(ui: &mut egui::Ui, state: &mut PaperAccountSheetState) {
    field_label(ui, "Commission");
    let width = ui.available_width();
    kit::segmented_filling(
        ui,
        Id::new("paper-account-commission-kind"),
        &CommissionKind::ALL.map(|kind| (kind, kind.title())),
        &mut state.commission_kind,
        width,
    );
    if state.commission_kind != CommissionKind::None {
        ui.add_space(8.0);
        let error = state.commission_error();
        let unit = state.commission_kind.unit(state.currency);
        text_field(
            ui,
            "account-commission",
            &mut state.commission_text,
            "0",
            &unit,
            error.is_some(),
            None,
        );
        field_footer(ui, error.map(str::to_owned).as_ref(), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> PaperAccount {
        let mut account = PaperAccount::new(3, "Swing", 0);
        account.base_currency = PaperCurrency::Eur;
        account.initial_balance = 25_000.5;
        account.settings.slippage_ticks = 2.0;
        account.settings.show_orders_on_chart = false;
        account.settings.leverage.crypto = 5.0;
        account.settings.commission = PaperCommissionConfiguration::Percentage { value: 0.1 };
        account
    }

    #[test]
    fn a_new_account_starts_from_the_defaults() {
        let state = PaperAccountSheetState::create();
        assert!(!state.is_reset && state.can_submit());
        assert_eq!(state.title(), "New Paper Account");
        assert_eq!(state.submit_title(), "Create Account");
        let result = state.result().unwrap();
        assert_eq!(result.name, "Paper Trading");
        assert_eq!(result.balance, 100_000.0);
        assert_eq!(result.currency, PaperCurrency::Usd);
        assert_eq!(result.settings, PaperAccountSettings::default());
        assert_eq!(state.reset_note(), None);
    }

    #[test]
    fn a_reset_starts_from_the_accounts_own_settings() {
        let state = PaperAccountSheetState::reset(&account());
        assert!(state.is_reset);
        assert_eq!(state.name, "Swing");
        assert_eq!(state.balance_text, "25000.5");
        assert_eq!(state.leverage_text, "5");
        assert_eq!(state.commission_kind, CommissionKind::Percentage);
        assert_eq!(state.commission_text, "0.1");
        assert_eq!(state.currency, PaperCurrency::Eur);
        assert_eq!(
            state.reset_note().as_deref(),
            Some("Balance resets to €25,000.50.")
        );
        assert_eq!(state.submit_title(), "Reset Account");
        let result = state.result().unwrap();
        assert_eq!(
            result.settings.slippage_ticks, 2.0,
            "other settings survive a reset"
        );
        assert!(!result.settings.show_orders_on_chart);
        assert_eq!(
            result.settings.leverage.stocks, 5.0,
            "one leverage for every market"
        );
        assert_eq!(
            result.settings.commission,
            PaperCommissionConfiguration::Percentage { value: 0.1 }
        );
    }

    #[test]
    fn each_field_has_its_own_rule() {
        let mut state = PaperAccountSheetState::create();
        state.balance_text = "abc".into();
        assert_eq!(state.balance_error(), Some("Enter a number."));
        state.balance_text = "0".into();
        assert_eq!(state.balance_error(), Some("Must be greater than zero."));
        assert!(!state.can_submit());
        state.balance_text.clear();
        assert_eq!(
            state.visible_balance_error(),
            None,
            "an empty field is not red yet"
        );
        assert!(!state.can_submit(), "but it still blocks the button");
        state.balance_text = "50,000".into();
        assert!(state.balance_error().is_none());

        for (text, error) in [
            ("0.5", Some("Must be between 1× and 1000×.")),
            ("1001", Some("Must be between 1× and 1000×.")),
            ("x", Some("Enter a number.")),
            ("1000", None),
            ("1", None),
        ] {
            state.leverage_text = text.into();
            assert_eq!(state.leverage_error(), error, "{text}");
        }
        state.leverage_text = "1".into();

        state.commission_text = "-1".into();
        assert_eq!(
            state.commission_error(),
            None,
            "ignored while commission is None"
        );
        state.commission_kind = CommissionKind::Fixed;
        assert_eq!(state.commission_error(), Some("Can't be negative."));
        state.commission_text = "".into();
        assert_eq!(state.commission_error(), Some("Enter a number."));
        state.commission_text = "2".into();
        assert_eq!(state.commission_error(), None);

        state.name = "   ".into();
        assert!(!state.can_submit());
        assert_eq!(state.result(), None);
    }

    #[test]
    fn the_result_trims_the_name_and_builds_the_commission() {
        let mut state = PaperAccountSheetState::create();
        state.name = "  Scalping ".into();
        state.commission_kind = CommissionKind::PerContract;
        state.commission_text = "1,5".into();
        state.leverage_text = "10".into();
        let result = state.result().unwrap();
        assert_eq!(result.name, "Scalping");
        assert_eq!(
            result.settings.commission,
            PaperCommissionConfiguration::PerContract { value: 1.5 }
        );
        assert_eq!(result.settings.leverage.prediction, 10.0);
        state.commission_kind = CommissionKind::None;
        assert_eq!(
            state.result().unwrap().settings.commission,
            PaperCommissionConfiguration::None
        );
    }

    #[test]
    fn commission_units_follow_the_kind_and_currency() {
        assert_eq!(CommissionKind::None.unit(PaperCurrency::Usd), "");
        assert_eq!(
            CommissionKind::Fixed.unit(PaperCurrency::Gbp),
            "GBP per order"
        );
        assert_eq!(
            CommissionKind::Percentage.unit(PaperCurrency::Usd),
            "% of value"
        );
        assert_eq!(
            CommissionKind::PerContract.unit(PaperCurrency::Usdt),
            "USDT per contract"
        );
        let titles: Vec<_> = CommissionKind::ALL
            .iter()
            .map(|kind| kind.title())
            .collect();
        assert_eq!(titles, ["None", "Fixed", "Percent", "Per contract"]);
    }
}
