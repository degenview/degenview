//! What the trading screens show when no venue is enabled: the list of places to trade, each to
//! be chosen. Nothing is connected, and nothing is sent anywhere, until the user picks one.

use eframe::egui;

use crate::model::trading_venue::TradingVenue;
use crate::trading::model::{VenueInfo, VenueKind};
use crate::ui::broker_badge::LIVE;
use crate::ui::kit::{self, Tone};
use crate::ui::theme;

/// Draws the list; returns the venue whose button was pressed.
pub fn show(ui: &mut egui::Ui, venues: &[VenueInfo]) -> Option<TradingVenue> {
    let mut chosen = None;
    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        ui.label(kit::text(
            "Choose where to trade",
            kit::size::TITLE2,
            kit::SEMIBOLD,
            theme::text_primary(),
        ));
        ui.add_space(4.0);
        ui.add(
            egui::Label::new(kit::text(
                "Nothing is connected until you choose. You can add or remove places at any time in Settings.",
                kit::size::CAPTION + 1.0,
                kit::REGULAR,
                theme::text_secondary(),
            ))
            .wrap(),
        );
    });
    ui.add_space(16.0);
    for info in venues {
        egui::Frame::new()
            .fill(theme::quaternary(0.35))
            .stroke(egui::Stroke::new(
                1.0,
                theme::with_alpha(theme::separator(), 0.5),
            ))
            .corner_radius(10.0)
            .inner_margin(egui::Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(kit::text(
                        &info.name,
                        kit::size::BODY,
                        kit::SEMIBOLD,
                        theme::text_primary(),
                    ));
                    let (label, tone) = match info.kind {
                        VenueKind::Live => ("Real money", Tone::Warning),
                        VenueKind::Test => ("Test", Tone::Neutral),
                        VenueKind::Simulated => ("Simulated", Tone::Neutral),
                    };
                    kit::status_badge(ui, label, tone);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if kit::sheet_button(ui, "Connect", false, true).clicked() {
                            chosen = Some(info.venue);
                        }
                    });
                });
                ui.add_space(4.0);
                let note_color = if info.kind == VenueKind::Live {
                    LIVE
                } else {
                    theme::text_secondary()
                };
                ui.add(
                    egui::Label::new(kit::text(
                        &info.summary,
                        kit::size::CAPTION + 1.0,
                        kit::REGULAR,
                        note_color,
                    ))
                    .wrap(),
                );
            });
        ui.add_space(8.0);
    }
    chosen
}
