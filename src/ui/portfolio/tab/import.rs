//! The import and export sheets the tab shows over `PortfolioImportFlow`: the CoinMarketCap
//! matching sheet and its "Match…" picker, the import preview, and the alert for a problem. The
//! flow does the work (dialogs, parsing, workers, the ledger write); these only draw and route.

use crate::host::portfolio::PortfolioHost;
use crate::host::portfolio::import_flow::{FlowEvent, PortfolioImportFlow};
use crate::service::portfolio::asset_auto_mapper::asset_from_search_result;
use crate::ui::icons::Icon;
use crate::ui::kit::sheet;
use crate::ui::portfolio::asset_picker::{self, AssetPickerOutcome, AssetPickerState};
use crate::ui::portfolio::cmc_import::{self, CmcImportAction, CmcImportInput};
use crate::ui::portfolio::import_preview::{self, ImportPreviewInput, ImportPreviewOutcome};
use crate::ui::theme;
use eframe::egui::{self, Align};

/// Drives the flow and draws its open sheet. Returns what happened, if anything.
pub(super) fn show(
    ctx: &egui::Context,
    flow: &mut PortfolioImportFlow,
    host: &mut PortfolioHost,
    utc_offset_secs: i64,
) -> Option<FlowEvent> {
    flow.poll(ctx, host);
    show_cmc(ctx, flow, host);
    let event = show_preview(ctx, flow, host, utc_offset_secs);
    show_notice(ctx, flow);
    event
}

fn show_cmc(ctx: &egui::Context, import: &mut PortfolioImportFlow, host: &PortfolioHost) {
    let Some(flow) = &mut import.cmc else {
        return;
    };
    // The "Match…" picker sits on top of the sheet.
    if let Some((symbol, picker)) = &mut flow.picking {
        match asset_picker::show(ctx, picker, host.store()) {
            Some(AssetPickerOutcome::Choose(result)) => {
                let symbol = symbol.clone();
                flow.state
                    .set_mapping(&symbol, asset_from_search_result(&result));
                flow.picking = None;
            }
            Some(AssetPickerOutcome::Cancel) => flow.picking = None,
            None => {}
        }
        return;
    }
    let actions = cmc_import::show(
        ctx,
        &mut flow.state,
        &CmcImportInput {
            preview: &flow.preview,
            portfolio_currency: flow.currency,
        },
    );
    for action in actions {
        match action {
            CmcImportAction::Match(symbol) => {
                flow.picking = Some((
                    symbol.clone(),
                    AssetPickerState::new(
                        &format!("Match {symbol}"),
                        "Pick the market that matches this CoinMarketCap ticker.",
                        "Use Asset",
                    ),
                ));
            }
            CmcImportAction::Continue => {
                import.continue_cmc();
                return;
            }
            CmcImportAction::Cancel => {
                import.cmc = None;
                return;
            }
        }
    }
}

fn show_preview(
    ctx: &egui::Context,
    flow: &mut PortfolioImportFlow,
    host: &mut PortfolioHost,
    utc_offset_secs: i64,
) -> Option<FlowEvent> {
    let (preview, state) = flow.csv.as_mut()?;
    let outcome = import_preview::show(
        ctx,
        state,
        &ImportPreviewInput {
            preview,
            utc_offset_secs,
        },
    );
    match outcome {
        Some(ImportPreviewOutcome::Import) => return flow.commit_preview(host),
        Some(ImportPreviewOutcome::Cancel) => flow.csv = None,
        None => {}
    }
    None
}

fn show_notice(ctx: &egui::Context, flow: &mut PortfolioImportFlow) {
    let Some(message) = flow.notice.clone() else {
        return;
    };
    sheet::show(ctx, egui::Id::new("portfolio-import-notice"), 380.0, |ui| {
        ui.add_space(20.0);
        sheet::inset(ui, |ui| {
            sheet::header(
                ui,
                Icon::Triangle,
                "Import or Export",
                &message,
                theme::ORANGE,
            );
            ui.add_space(20.0);
        });
        ui.separator();
        sheet::inset(ui, |ui| {
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ui.button("OK").clicked() {
                    flow.notice = None;
                }
            });
            ui.add_space(12.0);
        });
    });
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        flow.notice = None;
    }
}
