//! The one control that says where orders go and lets the user change it: a pill with the venue's
//! name, a dot for how it is doing, and a menu of the venues the user enabled.
//!
//! Real money is amber (`broker_badge::LIVE`), a test environment and the paper account are
//! neutral. The same pill sits in the order sheet and the orders panel, so the two always agree.

use eframe::egui::{self, Color32, Sense, Ui, vec2};

use crate::model::trading_venue::TradingVenue;
use crate::trading::model::{BackendState, VenueInfo, VenueKind};
use crate::ui::broker_badge::LIVE;
use crate::ui::icons::{self, Icon};
use crate::ui::{kit, theme};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PillAction {
    Pick(TradingVenue),
    /// "Add broker…": open Settings on the list of venues.
    AddBroker,
}

/// How the dot is coloured for a venue's state.
pub fn state_color(state: &BackendState) -> Color32 {
    match state {
        BackendState::Ready => kit::GREEN,
        BackendState::Connecting | BackendState::Paused(_) => kit::ORANGE,
        BackendState::NeedsAttention(_) => kit::RED,
        BackendState::NotConnected => theme::text_tertiary(),
    }
}

fn tint(kind: VenueKind) -> Color32 {
    match kind {
        VenueKind::Live => LIVE,
        VenueKind::Test | VenueKind::Simulated => theme::text_secondary(),
    }
}

/// What the pill says: `LIVE · COINBASE`, `TEST · COINBASE SANDBOX`, `PAPER TRADING`.
pub fn title(info: &VenueInfo) -> String {
    match info.kind {
        VenueKind::Live => format!("LIVE · {}", info.name.to_uppercase()),
        VenueKind::Test => format!("TEST · {}", info.name.to_uppercase()),
        VenueKind::Simulated => info.name.to_uppercase(),
    }
}

/// The text of a row in the menu.
pub fn row_label(info: &VenueInfo) -> String {
    match info.kind {
        VenueKind::Simulated => info.name.clone(),
        kind => format!("{} · {}", info.name, kind.label()),
    }
}

/// Draws the pill for `current` and its menu of `venues`. Returns what the user chose.
pub fn show(
    ui: &mut Ui,
    current: &VenueInfo,
    state: &BackendState,
    venues: &[VenueInfo],
) -> Option<PillAction> {
    let tint = tint(current.kind);
    let text = title(current);
    let galley = kit::galley(ui, &text, 10.5, kit::BOLD, tint, f32::INFINITY);
    let size = vec2(galley.size().x + 44.0, galley.size().y + 10.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let response = crate::ui::accessibility::label(response, &text)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    let alpha = if current.kind == VenueKind::Live {
        0.16
    } else {
        0.08
    };
    let alpha = if open || response.hovered() {
        alpha + 0.06
    } else {
        alpha
    };
    ui.painter()
        .rect_filled(rect, size.y / 2.0, theme::with_alpha(tint, alpha));
    ui.painter().rect_stroke(
        rect,
        size.y / 2.0,
        egui::Stroke::new(1.0, theme::with_alpha(tint, 0.45)),
        egui::StrokeKind::Inside,
    );
    ui.painter().circle_filled(
        rect.left_center() + vec2(11.0, 0.0),
        3.5,
        state_color(state),
    );
    ui.painter().galley(
        rect.left_center() + vec2(22.0, -galley.size().y / 2.0),
        galley,
        tint,
    );
    icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(rect.right_center() - vec2(11.0, 0.0), vec2(9.0, 9.0)),
        Icon::ChevronDown,
        tint,
    );

    let labels: Vec<String> = venues
        .iter()
        .map(row_label)
        .chain(std::iter::once("Add broker…".to_owned()))
        .collect();
    let mut chosen = None;
    kit::fitted_menu(
        ui,
        &response,
        egui::RectAlign::BOTTOM_START,
        &labels,
        |ui| {
            for info in venues {
                let marked = info.venue == current.venue;
                if kit::menu_item_marked(ui, Icon::Check, &row_label(info), marked) {
                    chosen = Some(PillAction::Pick(info.venue));
                    ui.close();
                }
            }
            if !venues.is_empty() {
                kit::menu_separator(ui);
            }
            if kit::menu_item(ui, Icon::Plus, "Add broker…", true, false) {
                chosen = Some(PillAction::AddBroker);
                ui.close();
            }
        },
    );
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trading::model::VenueInfo;

    fn info(kind: VenueKind, name: &str) -> VenueInfo {
        VenueInfo {
            venue: TradingVenue::Paper,
            name: name.to_owned(),
            kind,
            summary: String::new(),
            needs_no_credentials: true,
        }
    }

    #[test]
    fn the_pill_says_what_kind_of_orders_these_are() {
        assert_eq!(title(&info(VenueKind::Live, "Coinbase")), "LIVE · COINBASE");
        assert_eq!(
            title(&info(VenueKind::Test, "Coinbase Sandbox")),
            "TEST · COINBASE SANDBOX"
        );
        assert_eq!(
            title(&info(VenueKind::Simulated, "Paper trading")),
            "PAPER TRADING"
        );
        assert_eq!(
            row_label(&info(VenueKind::Live, "Coinbase")),
            "Coinbase · Live"
        );
        assert_eq!(
            row_label(&info(VenueKind::Simulated, "Paper trading")),
            "Paper trading"
        );
    }

    #[test]
    fn only_real_money_is_amber() {
        assert_eq!(tint(VenueKind::Live), LIVE);
        assert_ne!(tint(VenueKind::Test), LIVE);
        assert_ne!(tint(VenueKind::Simulated), LIVE);
    }
}
