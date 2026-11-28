//! The pill at the bottom of a plot that says which point to click. Closing it only puts the pill
//! away: the chart keeps waiting for the click.

use crate::ui::kit::{self, dialog};
use crate::ui::theme;
use eframe::egui::{self, Align2, Id, Rect, Ui, Vec2, pos2};

/// How far the pill stands above the plot's bottom edge.
const RISE: f32 = 12.0;

/// Whether the user closed it.
pub fn show(ui: &Ui, chart_id: u64, plot: Rect, text: &str) -> bool {
    let mut closed = false;
    egui::Area::new(Id::new(("script-point-prompt", chart_id)))
        .order(egui::Order::Foreground)
        .pivot(Align2::CENTER_BOTTOM)
        .fixed_pos(pos2(plot.center().x, plot.bottom() - RISE))
        .show(ui.ctx(), |ui| {
            kit::menu_frame()
                .inner_margin(egui::Margin::symmetric(12, 7))
                .show(ui, |ui| {
                    ui.set_max_width((plot.width() - 24.0).max(160.0));
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(8.0, 0.0);
                        ui.add(
                            egui::Label::new(kit::text(
                                text,
                                kit::size::BODY,
                                kit::REGULAR,
                                theme::text_primary(),
                            ))
                            .wrap(),
                        );
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::splat(18.0), egui::Sense::hover());
                        let id = Id::new(("script-point-prompt-close", chart_id));
                        closed = dialog::close_button(ui, id, rect).clicked();
                    });
                });
        });
    closed
}
