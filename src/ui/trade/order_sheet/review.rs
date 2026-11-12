//! The review dialog an order passes through before it is sent.

use eframe::egui::{self, Id};

use super::form::FormContext;
use super::state::{SheetState, Stage};
use crate::trading::confirm;
use crate::trading::draft::DraftCheck;
use crate::ui::icons::Icon;
use crate::ui::{
    broker::badge,
    kit::{self, dialog},
    theme,
};

/// Asks for the go-ahead while a review is open, and sends the order on it.
pub(super) fn show(
    ui: &mut egui::Ui,
    state: &mut SheetState,
    context: &FormContext,
    check: &DraftCheck,
) {
    let FormContext {
        info,
        backend,
        market,
        rules,
        ..
    } = *context;
    let Stage::Confirming(reviewed) = &state.stage else {
        return;
    };
    let reviewed = reviewed.clone();

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
        badge::LIVE,
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
            state.stage = Stage::Sending(backend.submit(market, &state.draft, &reviewed));
        }
        Some(_) => state.stage = Stage::Editing,
        None => {}
    }
}
