//! The Paper Trading panel's top and bottom bars. The header holds the PAPER badge, the account
//! switcher, the section tabs with their counts and the panel's own buttons; the footer holds the
//! one-line summary of the section and its bulk action (Close All, Cancel All, Export) behind a
//! confirmation. Pure views returning actions; the host owns the store and the engine.

// Transient: the paper-trading panel hosts these once the host is wired.
#![allow(dead_code)]

use crate::model::paper::PaperCurrency;
use crate::ui::icons::Icon;
use crate::ui::kit::{self, Confirm, ConfirmOutcome};
use crate::ui::paper_tables::{self, FooterFacts, PaperTab, footer_note};
use crate::ui::trade::panel_header::{Chip, ChipRow, MoreItem};
use eframe::egui::{self, Id};

#[derive(Clone, Debug, PartialEq)]
pub struct AccountChoice {
    pub id: u64,
    pub name: String,
}

pub struct PaperHeaderInput<'a> {
    pub accounts: &'a [AccountChoice],
    pub selected: Option<u64>,
    pub currency: Option<PaperCurrency>,
    pub position_count: usize,
    pub working_order_count: usize,
    pub show_trading_on_charts: bool,
}

#[derive(Default)]
pub struct PaperHeaderState {
    pub tab: PaperTab,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PaperHeaderAction {
    SelectAccount(u64),
    NewAccount,
    ExportData,
    ResetAccount,
    DeleteAccount,
    ToggleShowOnCharts,
    Close,
}

/// Deleting needs another account to fall back to: an empty list is refilled with a default.
pub fn can_delete_account(input: &PaperHeaderInput) -> bool {
    input.selected.is_some() && input.accounts.len() > 1
}

pub fn delete_title(name: &str) -> String {
    format!("Delete \u{201c}{name}\u{201d}?")
}

pub const DELETE_MESSAGE: &str = "Every position, order, fill, history and journal entry in this account is removed. This can't be undone.";

/// The confirmation for deleting an account; `None` while it is still open.
pub fn delete_confirm(ctx: &egui::Context, name: &str) -> Option<ConfirmOutcome> {
    kit::confirm(
        ctx,
        Id::new("paper-delete-account"),
        &Confirm {
            icon: Icon::Trash,
            title: &delete_title(name),
            message: DELETE_MESSAGE,
            destructive: "Delete Account",
        },
    )
}

/// The count shown beside a tab; only the live sections carry one.
pub fn tab_count(tab: PaperTab, input: &PaperHeaderInput) -> Option<usize> {
    match tab {
        PaperTab::Positions => Some(input.position_count),
        PaperTab::Orders => Some(input.working_order_count),
        PaperTab::History | PaperTab::Trades | PaperTab::Journal => None,
    }
}

/// "Positions 2", or just "Positions" for a section without a count.
pub fn tab_text(tab: PaperTab, count: Option<usize>) -> String {
    match count {
        Some(count) => format!("{} {count}", tab.title()),
        None => tab.title().to_owned(),
    }
}

pub fn account_name<'a>(input: &'a PaperHeaderInput) -> &'a str {
    input
        .selected
        .and_then(|id| input.accounts.iter().find(|account| account.id == id))
        .map_or("No account", |account| account.name.as_str())
}

pub fn eye_label(showing: bool) -> &'static str {
    if showing {
        "Hide trading on charts"
    } else {
        "Show trading on charts"
    }
}

/// The Swift tab glyphs.
pub fn tab_icon(tab: PaperTab) -> Icon {
    match tab {
        PaperTab::Positions => Icon::ChartUptrend,
        PaperTab::Orders => Icon::List,
        PaperTab::History => Icon::Clock,
        PaperTab::Trades => Icon::CheckmarkSeal,
        PaperTab::Journal => Icon::Books,
    }
}

/// What the shared header is given for the paper account: its sections with their counts, the
/// account switcher, and the `⋯` menu with the actions each entry stands for.
pub struct HeaderParts {
    pub tabs: Vec<kit::TabItem<PaperTab>>,
    pub chip: Chip,
    pub more: Vec<MoreItem>,
    pub more_actions: Vec<PaperHeaderAction>,
}

pub fn header_parts(input: &PaperHeaderInput) -> HeaderParts {
    let tabs = PaperTab::ALL
        .into_iter()
        .map(|tab| kit::TabItem {
            value: tab,
            title: tab.title(),
            icon: tab_icon(tab),
            count: tab_count(tab, input).unwrap_or(0),
        })
        .collect();
    let chip = Chip {
        label: account_name(input).to_owned(),
        sub: input.currency.map(|currency| currency.code().to_owned()),
        tip: "Switch paper account".to_owned(),
        rows: input
            .accounts
            .iter()
            .map(|choice| ChipRow {
                label: choice.name.clone(),
                selected: input.selected == Some(choice.id),
            })
            .collect(),
        add: Some("New Account\u{2026}".to_owned()),
    };
    let mut more = vec![
        MoreItem {
            icon: Icon::ShareUp,
            label: "Export Trading Data\u{2026}".to_owned(),
            danger: false,
            rule_above: false,
        },
        MoreItem {
            icon: Icon::Undo,
            label: "Reset Account\u{2026}".to_owned(),
            danger: true,
            rule_above: false,
        },
    ];
    let mut more_actions = vec![
        PaperHeaderAction::ExportData,
        PaperHeaderAction::ResetAccount,
    ];
    if can_delete_account(input) {
        more.push(MoreItem {
            icon: Icon::Trash,
            label: "Delete Account\u{2026}".to_owned(),
            danger: true,
            rule_above: false,
        });
        more_actions.push(PaperHeaderAction::DeleteAccount);
    }
    HeaderParts {
        tabs,
        chip,
        more,
        more_actions,
    }
}

// MARK: Footer

/// What the footer's bulk button asks the host to do once confirmed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaperFooterAction {
    CloseAllPositions,
    CancelAllOrders,
    /// The orders ticked in the table; the panel knows which.
    CancelSelectedOrders,
    Export,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaperFooterConfirm {
    CloseAll,
    CancelAll,
}

#[derive(Default)]
pub struct PaperFooterState {
    pub pending: Option<PaperFooterConfirm>,
}

/// "Close 1 Position" / "Cancel 3 Orders".
pub fn confirm_button(confirm: PaperFooterConfirm, count: usize) -> String {
    let (verb, noun) = match confirm {
        PaperFooterConfirm::CloseAll => ("Close", "Position"),
        PaperFooterConfirm::CancelAll => ("Cancel", "Order"),
    };
    format!("{verb} {count} {noun}{}", if count == 1 { "" } else { "s" })
}

pub fn confirm_title(confirm: PaperFooterConfirm) -> &'static str {
    match confirm {
        PaperFooterConfirm::CloseAll => "Close all positions?",
        PaperFooterConfirm::CancelAll => "Cancel all working orders?",
    }
}

pub fn confirm_message(confirm: PaperFooterConfirm) -> &'static str {
    match confirm {
        PaperFooterConfirm::CloseAll => "Each position is closed at market.",
        PaperFooterConfirm::CancelAll => {
            "Protective take-profit and stop-loss orders are canceled too."
        }
    }
}

/// The label on the section's bulk button.
pub fn bulk_label(tab: PaperTab) -> &'static str {
    match tab {
        PaperTab::Positions => "Close All",
        PaperTab::Orders => "Cancel All",
        PaperTab::History | PaperTab::Trades | PaperTab::Journal => "Export…",
    }
}

/// Whether the bulk button has anything to act on.
pub fn bulk_enabled(tab: PaperTab, facts: &FooterFacts) -> bool {
    match tab {
        PaperTab::Positions => !facts.positions.is_empty(),
        PaperTab::Orders => facts.working_orders > 0,
        PaperTab::History | PaperTab::Trades | PaperTab::Journal => true,
    }
}

pub fn footer(
    ui: &mut egui::Ui,
    state: &mut PaperFooterState,
    tab: PaperTab,
    facts: &FooterFacts,
    selected_orders: usize,
) -> Vec<PaperFooterAction> {
    let mut actions = Vec::new();
    if tab == PaperTab::Orders {
        // The orders bar is the same for every venue: how many are working, and the two ways to
        // cancel several.
        let bar = crate::ui::trade::tables::orders_bar(ui, facts.working_orders, selected_orders);
        if bar.cancel_all {
            state.pending = Some(PaperFooterConfirm::CancelAll);
        }
        if bar.cancel_selected {
            actions.push(PaperFooterAction::CancelSelectedOrders);
        }
        confirm_dialog(ui.ctx(), state, facts, &mut actions);
        return actions;
    }
    let export = matches!(
        tab,
        PaperTab::History | PaperTab::Trades | PaperTab::Journal
    );
    let clicked = kit::footer_bar_with(
        ui,
        tab_icon(tab),
        &footer_note(tab, facts),
        export.then_some(Icon::ShareUp),
        bulk_label(tab),
        bulk_enabled(tab, facts),
        match tab {
            PaperTab::Positions => "Close every open position at market",
            PaperTab::Orders => "Cancel every working order",
            _ => "Export the account's trading data",
        },
    );
    if clicked {
        match tab {
            PaperTab::Positions => state.pending = Some(PaperFooterConfirm::CloseAll),
            PaperTab::Orders => state.pending = Some(PaperFooterConfirm::CancelAll),
            _ => actions.push(PaperFooterAction::Export),
        }
    }
    confirm_dialog(ui.ctx(), state, facts, &mut actions);
    actions
}

fn confirm_dialog(
    ctx: &egui::Context,
    state: &mut PaperFooterState,
    facts: &FooterFacts,
    actions: &mut Vec<PaperFooterAction>,
) {
    let Some(confirm) = state.pending else {
        return;
    };
    let count = match confirm {
        PaperFooterConfirm::CloseAll => facts.positions.len(),
        PaperFooterConfirm::CancelAll => facts.working_orders,
    };
    let button = confirm_button(confirm, count);
    let result = kit::confirm(
        ctx,
        Id::new("paper-footer-confirm"),
        &Confirm {
            icon: match confirm {
                PaperFooterConfirm::CloseAll => Icon::XmarkCircleFill,
                PaperFooterConfirm::CancelAll => Icon::Trash,
            },
            title: confirm_title(confirm),
            message: confirm_message(confirm),
            destructive: &button,
        },
    );
    if result == Some(ConfirmOutcome::Confirmed) {
        actions.push(match confirm {
            PaperFooterConfirm::CloseAll => PaperFooterAction::CloseAllPositions,
            PaperFooterConfirm::CancelAll => PaperFooterAction::CancelAllOrders,
        });
    }
    if result.is_some() {
        state.pending = None;
    }
}

// MARK: Empty state

pub const NO_ACCOUNT_TITLE: &str = "No paper account";
pub const NO_ACCOUNT_MESSAGE: &str = "Create one to practise trading with simulated money.";

/// What the panel shows when there is no account yet; true when "Create Account" was clicked.
pub fn no_account_state(ui: &mut egui::Ui) -> bool {
    paper_tables::empty_with_action(
        ui,
        Icon::DocText,
        NO_ACCOUNT_TITLE,
        NO_ACCOUNT_MESSAGE,
        Some("Create Account"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(accounts: &'a [AccountChoice], selected: Option<u64>) -> PaperHeaderInput<'a> {
        PaperHeaderInput {
            accounts,
            selected,
            currency: Some(PaperCurrency::Usd),
            position_count: 2,
            working_order_count: 0,
            show_trading_on_charts: true,
        }
    }

    fn accounts() -> Vec<AccountChoice> {
        vec![
            AccountChoice {
                id: 1,
                name: "Main".into(),
            },
            AccountChoice {
                id: 2,
                name: "Swing".into(),
            },
        ]
    }

    #[test]
    fn only_live_sections_carry_a_count() {
        let accounts = accounts();
        let input = input(&accounts, Some(1));
        assert_eq!(tab_count(PaperTab::Positions, &input), Some(2));
        assert_eq!(tab_count(PaperTab::Orders, &input), Some(0));
        for tab in [PaperTab::History, PaperTab::Trades, PaperTab::Journal] {
            assert_eq!(tab_count(tab, &input), None);
        }
        assert_eq!(tab_text(PaperTab::Positions, Some(2)), "Positions 2");
        assert_eq!(tab_text(PaperTab::Journal, None), "Journal");
    }

    #[test]
    fn the_switcher_names_the_selected_account() {
        let accounts = accounts();
        assert_eq!(account_name(&input(&accounts, Some(2))), "Swing");
        assert_eq!(account_name(&input(&accounts, None)), "No account");
        assert_eq!(account_name(&input(&accounts, Some(9))), "No account");
    }

    #[test]
    fn the_empty_state_invites_a_first_account() {
        assert_eq!(NO_ACCOUNT_TITLE, "No paper account");
        assert!(NO_ACCOUNT_MESSAGE.contains("simulated money"));
    }

    #[test]
    fn the_eye_says_what_a_click_will_do() {
        assert_eq!(eye_label(true), "Hide trading on charts");
        assert_eq!(eye_label(false), "Show trading on charts");
    }

    #[test]
    fn confirmations_count_and_pluralise() {
        assert_eq!(
            confirm_button(PaperFooterConfirm::CloseAll, 1),
            "Close 1 Position"
        );
        assert_eq!(
            confirm_button(PaperFooterConfirm::CancelAll, 3),
            "Cancel 3 Orders"
        );
        assert_eq!(
            confirm_title(PaperFooterConfirm::CloseAll),
            "Close all positions?"
        );
        assert!(confirm_message(PaperFooterConfirm::CancelAll).contains("take-profit"));
    }

    #[test]
    fn the_bulk_button_follows_the_section() {
        assert_eq!(bulk_label(PaperTab::Positions), "Close All");
        assert_eq!(bulk_label(PaperTab::Orders), "Cancel All");
        assert_eq!(bulk_label(PaperTab::Journal), "Export…");
        let facts = FooterFacts {
            positions: &[],
            unrealized_total: 0.0,
            working_orders: 0,
            events: 0,
            trades: &[],
            journal_entries: 0,
            currency: PaperCurrency::Usd,
        };
        assert!(!bulk_enabled(PaperTab::Positions, &facts));
        assert!(!bulk_enabled(PaperTab::Orders, &facts));
        assert!(
            bulk_enabled(PaperTab::History, &facts),
            "export is always on"
        );
    }

    #[test]
    fn an_account_can_be_deleted_only_when_another_remains() {
        let two = [
            AccountChoice {
                id: 1,
                name: "A".into(),
            },
            AccountChoice {
                id: 2,
                name: "B".into(),
            },
        ];
        assert!(can_delete_account(&input(&two, Some(1))));
        assert!(!can_delete_account(&input(&two, None)));
        assert!(!can_delete_account(&input(&two[..1], Some(1))));
        assert!(!can_delete_account(&input(&[], None)));
    }

    #[test]
    fn the_delete_confirmation_names_the_account() {
        assert_eq!(delete_title("Swing"), "Delete \u{201c}Swing\u{201d}?");
        assert!(DELETE_MESSAGE.contains("can't be undone"));
    }
}
