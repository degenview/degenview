//! The Portfolio tab's bar, on the shared flat bar (`ui::kit::bar`) so it is as tall and looks
//! like the chart's toolbar: the portfolio and currency pull-downs on the left; the privacy
//! toggle, the Import or Export menu and Add Transaction on the right (the last two only once
//! there is a portfolio to add to).

use super::{
    CreateState, IMPORT_EXPORT_ACTIONS, ManageState, PORTFOLIO_CURRENCIES, PendingExport,
    PortfolioHost, PortfolioTab, add_transaction_picker, cmc_destination, currency_choice,
    has_portfolio_scope, portfolio, portfolio_scope,
};
use crate::model::portfolio::PortfolioLedgerSnapshot;
use crate::ui::icons::Icon;
use crate::ui::kit;
use crate::ui::kit::bar::{self, Button, Item};
use crate::ui::kit::header_controls;
use eframe::egui::{self, Response, Ui};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BarAction {
    AddTransaction,
    TogglePrivacy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BarMenu {
    Portfolio,
    Currency,
    ImportExport,
}

type BarItem = Item<BarAction, BarMenu>;
type BarButton = Button<BarAction, BarMenu>;

/// What the bar shows, from the tab's state: names and flags only, so the items own no borrow of
/// the tab and the popups can change it.
struct BarState {
    portfolio: String,
    currency: &'static str,
    /// The valuation is being rebuilt in the new currency; it cannot be changed again until it lands.
    converting: bool,
    privacy: bool,
    /// There is a portfolio to add transactions to, import into or export from.
    has_scope: bool,
}

fn items(state: &BarState) -> (Vec<BarItem>, Vec<BarItem>) {
    let mut portfolio = BarButton::new("portfolio", "Choose portfolio")
        .icon(Icon::Briefcase)
        .label(state.portfolio.as_str())
        .with_menu(BarMenu::Portfolio)
        .chevron();
    portfolio.keep_label = true;
    let mut currency = BarButton::new("currency", "Reporting currency")
        .icon(Icon::Banknote)
        .label(state.currency)
        .enabled(!state.converting)
        .busy(state.converting)
        .with_menu(BarMenu::Currency)
        .chevron();
    currency.keep_label = true;
    let left = vec![Item::Button(portfolio), Item::Button(currency)];

    let mut right = Vec::new();
    if state.has_scope {
        right.push(Item::Button(
            BarButton::new("import-export", "Import or Export")
                .icon(Icon::Exchange)
                .with_menu(BarMenu::ImportExport),
        ));
        right.push(Item::Button(
            BarButton::new("add-transaction", "Add a transaction")
                .icon(Icon::Plus)
                .label("Add Transaction")
                .on_click(BarAction::AddTransaction),
        ));
        right.push(Item::Separator);
    }
    right.push(Item::Button(
        BarButton::new(
            "privacy",
            if state.privacy {
                "Reveal portfolio values"
            } else {
                "Hide portfolio values"
            },
        )
        .icon(if state.privacy {
            Icon::EyeSlash
        } else {
            Icon::Eye
        })
        .active(state.privacy)
        .on_click(BarAction::TogglePrivacy),
    ));
    (left, right)
}

impl PortfolioTab {
    pub(super) fn show_bar(
        &mut self,
        ui: &mut Ui,
        host: &mut PortfolioHost,
        snapshot: &PortfolioLedgerSnapshot,
    ) {
        let scope = portfolio_scope(snapshot);
        let quote_state = self.quote_feed.state();
        let state = BarState {
            portfolio: snapshot
                .selected_portfolio_id
                .and_then(|id| {
                    snapshot
                        .portfolios
                        .iter()
                        .find(|portfolio| portfolio.id == id)
                })
                .map_or("All Portfolios", |portfolio| portfolio.name.as_str())
                .to_owned(),
            currency: self.reporting_currency.code(),
            // Converting every transaction rebuilds the valuation from scratch.
            converting: snapshot
                .transactions
                .iter()
                .any(|transaction| scope.contains(&transaction.portfolio_id))
                && quote_state.projection.is_none()
                && quote_state.is_updating,
            privacy: self.privacy,
            has_scope: has_portfolio_scope(snapshot),
        };
        let (left, right) = items(&state);
        let chosen_before = self.reporting_currency;
        let actions = bar::show(ui, &left, &right, |ui, menu, response, _| match menu {
            BarMenu::Portfolio => self.portfolio_menu(ui, response, host, snapshot),
            BarMenu::Currency => self.currency_menu(ui, response),
            BarMenu::ImportExport => self.import_export_menu(ui, response, snapshot),
        });
        if self.reporting_currency != chosen_before {
            host.set_reporting_currency(snapshot.selected_portfolio_id, self.reporting_currency);
        }
        for action in actions {
            match action {
                BarAction::AddTransaction => self.picker = Some(add_transaction_picker()),
                BarAction::TogglePrivacy => {
                    self.privacy = !self.privacy;
                    host.set_privacy_mode(self.privacy);
                }
            }
        }
    }

    fn portfolio_menu(
        &mut self,
        ui: &mut Ui,
        response: &Response,
        host: &mut PortfolioHost,
        snapshot: &PortfolioLedgerSnapshot,
    ) {
        let labels: Vec<&str> = [
            "All Portfolios",
            "Create Portfolio\u{2026}",
            "Manage Portfolios\u{2026}",
        ]
        .into_iter()
        .chain(
            snapshot
                .portfolios
                .iter()
                .filter(|portfolio| !portfolio.is_archived)
                .map(|portfolio| portfolio.name.as_str()),
        )
        .collect();
        kit::fitted_menu(ui, response, egui::RectAlign::BOTTOM_START, &labels, |ui| {
            if header_controls::choice(
                ui,
                "All Portfolios",
                snapshot.selected_portfolio_id.is_none(),
            ) {
                let _ = host.select(None);
            }
            header_controls::separator(ui);
            for portfolio in snapshot
                .portfolios
                .iter()
                .filter(|portfolio| !portfolio.is_archived)
            {
                if header_controls::choice(
                    ui,
                    &portfolio.name,
                    snapshot.selected_portfolio_id == Some(portfolio.id),
                ) {
                    let _ = host.select(Some(portfolio.id));
                }
            }
            header_controls::separator(ui);
            if header_controls::action(ui, Icon::Plus, "Create Portfolio\u{2026}") {
                self.create = Some(CreateState::default());
            }
            if header_controls::action(ui, Icon::Sliders, "Manage Portfolios\u{2026}") {
                self.manage = Some(ManageState::default());
            }
        });
    }

    fn currency_menu(&mut self, ui: &mut Ui, response: &Response) {
        let labels: Vec<String> = PORTFOLIO_CURRENCIES
            .iter()
            .map(|currency| currency_choice(*currency))
            .collect();
        let current = self.reporting_currency;
        kit::fitted_menu(ui, response, egui::RectAlign::BOTTOM_START, &labels, |ui| {
            for currency in PORTFOLIO_CURRENCIES {
                if header_controls::choice(ui, &currency_choice(currency), currency == current) {
                    self.reporting_currency = currency;
                }
            }
        });
    }

    fn import_export_menu(
        &mut self,
        ui: &mut Ui,
        response: &Response,
        snapshot: &PortfolioLedgerSnapshot,
    ) {
        let ctx = ui.ctx().clone();
        kit::fitted_menu(
            ui,
            response,
            egui::RectAlign::BOTTOM_END,
            &IMPORT_EXPORT_ACTIONS[..],
            |ui| {
                if header_controls::action(ui, Icon::Download, "Import Transactions\u{2026}") {
                    self.import.begin_import_csv(&ctx);
                }
                if header_controls::action(ui, Icon::Download, "Import from CoinMarketCap\u{2026}")
                {
                    self.import
                        .begin_import_cmc(&ctx, cmc_destination(snapshot));
                }
                header_controls::separator(ui);
                if header_controls::action(ui, Icon::ShareUp, "Export Transactions\u{2026}") {
                    let text = portfolio::csv::export_transactions(
                        &snapshot.transactions,
                        &snapshot.portfolios,
                    );
                    self.import.begin_export(
                        &ctx,
                        text,
                        crate::host::portfolio::import_flow::CSV_FILENAME,
                    );
                }
                if header_controls::action(ui, Icon::ShareUp, "Export Current Holdings\u{2026}") {
                    self.pending_export = Some(PendingExport::Holdings);
                }
                if header_controls::action(ui, Icon::ShareUp, "Export Portfolio History\u{2026}") {
                    self.pending_export = Some(PendingExport::History);
                }
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::kit::bar::{EDGE, HEIGHT};
    use eframe::egui::{Rect, pos2, vec2};

    fn state(has_scope: bool) -> BarState {
        BarState {
            portfolio: "Long-term holdings".to_owned(),
            currency: "USD",
            converting: false,
            privacy: false,
            has_scope,
        }
    }

    fn actions_of(items: &[BarItem]) -> Vec<(Option<BarAction>, bool)> {
        items
            .iter()
            .filter_map(|item| match item {
                Item::Button(button) => Some((button.action, button.enabled)),
                Item::Separator => None,
            })
            .collect()
    }

    fn screen(width: f32) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, HEIGHT))),
            ..Default::default()
        }
    }

    #[test]
    fn a_ledger_without_portfolios_keeps_only_the_privacy_toggle_on_the_right() {
        let (left, right) = items(&state(false));
        assert_eq!(left.len(), 2, "the portfolio and the currency");
        assert_eq!(
            actions_of(&right),
            vec![(Some(BarAction::TogglePrivacy), true)]
        );
    }

    #[test]
    fn add_transaction_and_import_export_come_with_a_portfolio() {
        let (_, right) = items(&state(true));
        let actions = actions_of(&right);
        assert!(actions.contains(&(Some(BarAction::AddTransaction), true)));
        assert!(actions.contains(&(Some(BarAction::TogglePrivacy), true)));
        // The Import or Export menu has no click action of its own.
        assert!(actions.contains(&(None, true)));
    }

    #[test]
    fn the_currency_cannot_change_while_the_valuation_converts() {
        let mut converting = state(true);
        converting.converting = true;
        let (left, _) = items(&converting);
        let currency = match &left[1] {
            Item::Button(button) => button,
            Item::Separator => panic!("the currency is a button"),
        };
        assert!(!currency.enabled && currency.busy);
        let (left, _) = items(&state(true));
        match &left[1] {
            Item::Button(button) => assert!(button.enabled && !button.busy),
            Item::Separator => panic!("the currency is a button"),
        }
    }

    #[test]
    fn the_bar_keeps_every_button_at_the_narrowest_window_without_overlap() {
        let ctx = egui::Context::default();
        for width in [1400.0_f32, 960.0] {
            let (left, right) = items(&state(true));
            let mut placed = Vec::new();
            let mut frame = ctx.run_ui(screen(width), |ui| {
                placed = bar::geometry(ui, &left, &right, width).1;
            });
            frame.textures_delta.clear();
            assert_eq!(placed.len(), 5, "nothing is dropped at {width}");
            placed.sort_by(|a, b| a.from.total_cmp(&b.from));
            for pair in placed.windows(2) {
                assert!(pair[0].to <= pair[1].from, "overlap at {width}");
            }
            assert!(placed.last().expect("buttons").to <= width - EDGE + 0.5);
        }
    }

    /// Pictures of the bar at two widths in both themes, to compare with the chart toolbar's.
    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_the_bar() {
        use crate::ui::render_tests::{Screen, gallery_dir};
        use crate::ui::theme;
        if gallery_dir().is_none() {
            return;
        }
        let _theme = theme::RestoreDefaultTheme;
        for (theme_name, app_theme) in [
            ("dark", theme::AppTheme::Dark),
            ("light", theme::AppTheme::Light),
        ] {
            for width in [1400.0_f32, 960.0] {
                let mut screen = Screen::new(vec2(width, HEIGHT));
                theme::apply(&screen.ctx, app_theme);
                let (left, right) = items(&state(true));
                let mut last = None;
                for _ in 0..3 {
                    last = Some(screen.run(Vec::new(), |ui| {
                        bar::show(ui, &left, &right, |_, _, _, _| {});
                    }));
                }
                screen.save(
                    &format!("portfolio-bar-{width}-{theme_name}"),
                    last.expect("a frame"),
                );
            }
        }
    }
}
