//! The Paper Trading panel's top and bottom bars. The header holds the PAPER badge, the account
//! switcher, the section tabs with their counts and the panel's own buttons; the footer holds the
//! one-line summary of the section and its bulk action (Close All, Cancel All, Export) behind a
//! confirmation. Pure views returning actions; the host owns the store and the engine.

// Transient: the paper-trading panel hosts these once the host is wired.
#![allow(dead_code)]

use crate::model::paper::PaperCurrency;
use crate::ui::accessibility;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, Confirm, ConfirmOutcome};
use crate::ui::paper_tables::{self, FooterFacts, PaperTab, footer_note};
use crate::ui::theme;
use eframe::egui::{self, Id, Sense, Vec2, pos2, vec2};

/// Width below which the tabs drop to a second row instead of being cut off.
pub const COMPACT_WIDTH: f32 = 900.0;

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

/// The one "PAPER" marker, so simulated money is never mistaken for the real thing.
pub fn badge(ui: &mut egui::Ui) {
    let galley = kit::galley(ui, "PAPER", 10.5, kit::BOLD, kit::ACCENT, f32::INFINITY);
    let size = vec2(
        8.0 + 9.5 + 4.0 + galley.size().x + 8.0,
        galley.size().y + 6.0,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, size.y / 2.0, theme::with_alpha(kit::ACCENT, 0.14));
    icons::paint(
        painter,
        egui::Rect::from_center_size(pos2(rect.left() + 12.75, rect.center().y), Vec2::splat(9.5)),
        Icon::DocText,
        kit::ACCENT,
    );
    painter.galley(
        pos2(rect.left() + 21.5, rect.center().y - galley.size().y / 2.0),
        galley,
        kit::ACCENT,
    );
    accessibility::label_as(response, egui::WidgetType::Label, "Paper trading")
        .on_hover_text("Paper trading");
}

/// `PaperIconGlyph`'s 26-point target: it lights up on hover and, when `active`, in the accent.
fn glyph_response(
    ui: &mut egui::Ui,
    id: Id,
    icon: Icon,
    active: bool,
    label: &str,
) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    let response = accessibility::label(response, label);
    let fill = if active {
        theme::with_alpha(kit::ACCENT, 0.14)
    } else if response.is_pointer_button_down_on() {
        kit::primary(0.12)
    } else if response.hovered() {
        kit::primary(0.08)
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 7.0, fill);
    icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(rect.center(), Vec2::splat(13.0)),
        icon,
        if active {
            kit::ACCENT
        } else {
            theme::text_secondary()
        },
    );
    response.on_hover_text(label)
}

/// An icon-only button; `label` is its tooltip.
pub fn icon_button(ui: &mut egui::Ui, icon: Icon, label: &str, active: bool) -> egui::Response {
    glyph_response(ui, Id::new(("paper-icon", label)), icon, active, label)
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

pub fn header(
    ui: &mut egui::Ui,
    state: &mut PaperHeaderState,
    input: &PaperHeaderInput,
) -> Vec<PaperHeaderAction> {
    let mut actions = Vec::new();
    let compact = ui.available_width() < COMPACT_WIDTH;
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(12.0, 0.0);
            if compact {
                row(ui, 26.0, |ui| {
                    identity(ui, input, &mut actions);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        buttons(ui, input, &mut actions);
                    });
                });
                ui.add_space(8.0);
                egui::ScrollArea::horizontal()
                    .id_salt("paper-tabs")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| tabs(ui, state, input));
            } else {
                row(ui, 34.0, |ui| {
                    identity(ui, input, &mut actions);
                    tabs(ui, state, input);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        buttons(ui, input, &mut actions);
                    });
                });
            }
        });
    actions
}

/// One header row, `height` tall, its children centred on it.
fn row(ui: &mut egui::Ui, height: f32, add: impl FnOnce(&mut egui::Ui)) {
    let size = vec2(ui.available_width(), height);
    ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Center), add);
}

fn identity(ui: &mut egui::Ui, input: &PaperHeaderInput, actions: &mut Vec<PaperHeaderAction>) {
    ui.spacing_mut().item_spacing.x = 8.0;
    badge(ui);
    account_menu(ui, input, actions);
}

/// The account switcher: a quiet pill with the name, the currency and a chevron.
fn account_menu(ui: &mut egui::Ui, input: &PaperHeaderInput, actions: &mut Vec<PaperHeaderAction>) {
    let name = kit::galley(
        ui,
        account_name(input),
        13.0,
        kit::SEMIBOLD,
        theme::text_primary(),
        170.0,
    );
    let currency = input.currency.map(|currency| {
        kit::galley(
            ui,
            currency.code(),
            10.0,
            kit::SEMIBOLD,
            theme::text_secondary(),
            f32::INFINITY,
        )
    });
    let width = 9.0
        + name.size().x
        + currency.as_ref().map_or(0.0, |code| 6.0 + code.size().x)
        + 6.0
        + 9.0
        + 9.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, 26.0), Sense::click());
    let response = accessibility::label(response, "Paper account");
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    let fill = if open || response.is_pointer_button_down_on() {
        kit::primary(0.14)
    } else if response.hovered() {
        kit::primary(0.1)
    } else {
        kit::primary(0.06)
    };
    ui.painter().rect_filled(rect, 7.0, fill);
    let mut x = rect.left() + 9.0;
    let name_width = name.size().x;
    ui.painter().galley(
        pos2(x, rect.center().y - name.size().y / 2.0),
        name,
        theme::text_primary(),
    );
    x += name_width;
    if let Some(code) = currency {
        x += 6.0;
        let code_width = code.size().x;
        ui.painter().galley(
            pos2(x, rect.center().y - code.size().y / 2.0),
            code,
            theme::text_secondary(),
        );
        x += code_width;
    }
    icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(pos2(x + 6.0 + 4.5, rect.center().y), Vec2::splat(9.0)),
        Icon::ChevronDown,
        theme::text_secondary(),
    );
    let response = response.on_hover_text("Switch paper account");
    egui::Popup::menu(&response)
        .frame(kit::menu_frame())
        .align(egui::RectAlign::BOTTOM_START)
        .gap(4.0)
        .show(|ui| {
            ui.set_min_width(180.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            for account in input.accounts {
                let selected = input.selected == Some(account.id);
                if kit::menu_item_marked(ui, Icon::Check, &account.name, selected) {
                    actions.push(PaperHeaderAction::SelectAccount(account.id));
                    ui.close();
                }
            }
            kit::menu_separator(ui);
            if kit::menu_item(ui, Icon::Plus, "New Account…", true, false) {
                actions.push(PaperHeaderAction::NewAccount);
                ui.close();
            }
        });
}

/// The section tabs: Swift's compact `IconTabBar`, with the live counts.
fn tabs(ui: &mut egui::Ui, state: &mut PaperHeaderState, input: &PaperHeaderInput) {
    let items: Vec<kit::TabItem<PaperTab>> = PaperTab::ALL
        .into_iter()
        .map(|tab| kit::TabItem {
            value: tab,
            title: tab.title(),
            icon: tab_icon(tab),
            count: tab_count(tab, input).unwrap_or(0),
        })
        .collect();
    kit::tab_bar_compact(ui, Id::new("paper-tab-bar"), &items, &mut state.tab);
}

fn buttons(ui: &mut egui::Ui, input: &PaperHeaderInput, actions: &mut Vec<PaperHeaderAction>) {
    ui.spacing_mut().item_spacing.x = 2.0;
    // Right-to-left, so the last one added sits leftmost.
    if icon_button(ui, Icon::Xmark, "Close Paper Trading panel", false).clicked() {
        actions.push(PaperHeaderAction::Close);
    }
    let more = glyph_response(
        ui,
        Id::new("paper-more"),
        Icon::Ellipsis,
        false,
        "Account actions",
    );
    egui::Popup::menu(&more)
        .frame(kit::menu_frame())
        .align(egui::RectAlign::BOTTOM_END)
        .gap(4.0)
        .show(|ui| {
            ui.set_min_width(200.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            if kit::menu_item(ui, Icon::Plus, "New Account…", true, false) {
                actions.push(PaperHeaderAction::NewAccount);
                ui.close();
            }
            kit::menu_separator(ui);
            if kit::menu_item(ui, Icon::ShareUp, "Export Trading Data…", true, false) {
                actions.push(PaperHeaderAction::ExportData);
                ui.close();
            }
            if kit::menu_item(ui, Icon::Undo, "Reset Account…", true, true) {
                actions.push(PaperHeaderAction::ResetAccount);
                ui.close();
            }
            if can_delete_account(input)
                && kit::menu_item(ui, Icon::Trash, "Delete Account…", true, true)
            {
                actions.push(PaperHeaderAction::DeleteAccount);
                ui.close();
            }
        });
    let showing = input.show_trading_on_charts;
    let icon = if showing { Icon::Eye } else { Icon::EyeSlash };
    if icon_button(ui, icon, eye_label(showing), showing).clicked() {
        actions.push(PaperHeaderAction::ToggleShowOnCharts);
    }
}

// MARK: Footer

/// What the footer's bulk button asks the host to do once confirmed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaperFooterAction {
    CloseAllPositions,
    CancelAllOrders,
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
) -> Vec<PaperFooterAction> {
    let mut actions = Vec::new();
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
