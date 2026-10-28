//! The question over the editor when a script's file changed outside the app while it holds unsaved
//! typing. A view only: it reports which button was pressed and `ScriptManager` acts on it.

use crate::ui::icons::{self, Icon};
use crate::ui::{kit, theme};
use eframe::egui::{self, Margin, Ui, vec2};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BannerAction {
    /// Take the file's text. Undo brings the typing back.
    Reload,
    /// Keep the editor's text; saving will overwrite the file.
    Keep,
}

pub fn show(ui: &mut Ui, name: &str) -> Option<BannerAction> {
    let mut action = None;
    let tint = kit::ORANGE;
    egui::Frame::new()
        .fill(theme::with_alpha(tint, 0.12))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
            ui.horizontal_top(|ui| {
                icons::inline(ui, Icon::InfoCircle, 13.0, tint);
                ui.vertical(|ui| {
                    ui.set_max_width((ui.available_width() - 4.0).max(40.0));
                    ui.add(
                        egui::Label::new(kit::text(
                            format!("{name} changed outside the app"),
                            kit::size::SUBHEADLINE + 0.5,
                            kit::SEMIBOLD,
                            theme::text_primary(),
                        ))
                        .wrap(),
                    );
                    ui.add(
                        egui::Label::new(kit::text(
                            "You have unsaved changes too. Saving is on hold until you choose.",
                            kit::size::SUBHEADLINE,
                            kit::REGULAR,
                            theme::text_secondary(),
                        ))
                        .wrap(),
                    );
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                        if kit::sheet_button(ui, "Reload from disk", true, true)
                            .on_hover_text("Shows the file's text. Undo brings your changes back.")
                            .clicked()
                        {
                            action = Some(BannerAction::Reload);
                        }
                        if kit::sheet_button(ui, "Keep my version", false, true)
                            .on_hover_text("Keeps what you typed. Saving will overwrite the file.")
                            .clicked()
                        {
                            action = Some(BannerAction::Keep);
                        }
                    });
                });
            });
        });
    action
}
