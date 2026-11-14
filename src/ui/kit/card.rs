//! Cards: the flat frame, the bare badge, the inset well and the hairline between rows.

use super::hairline;
use crate::ui::icons::{self, Icon};
use crate::ui::theme;
use eframe::egui::{self, Color32, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, vec2};

/// Corner radius of a settings card.
pub const CARD_RADIUS: f32 = 10.0;

/// The frame of a settings card: flat, a faint fill and a hairline, no highlight or shadow.
/// `hovered` lifts the fill and the hairline.
pub fn card_frame(hovered: bool) -> egui::Frame {
    egui::Frame::new()
        .fill(theme::card_fill(hovered))
        .stroke(Stroke::new(1.0, theme::card_border(hovered)))
        .corner_radius(CARD_RADIUS)
        .inner_margin(egui::Margin::symmetric(14, 12))
}

/// The icon at the left of a settings card: plain and neutral, with no tile, disc or circle
/// behind it, so colour is kept for the switch and the selected states.
pub fn paint_card_badge(ui: &Ui, badge: Rect, icon: Icon) {
    icons::paint(
        ui.painter(),
        Rect::from_center_size(badge.center(), Vec2::splat(18.0)),
        icon,
        theme::text_secondary(),
    );
}

/// An inset well in a card, for a preview or a sample to sit in: darker than the card, with a
/// hairline and a shadow along the top edge, as if the card were cut away there.
pub fn paint_well(ui: &Ui, rect: Rect, radius: f32) {
    let painter = ui.painter();
    painter.rect_filled(rect, radius, theme::well_fill());
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, theme::ink(10)),
        StrokeKind::Inside,
    );
    let inset = radius * 0.8;
    painter.hline(
        (rect.left() + inset)..=(rect.right() - inset),
        rect.top() + 1.0,
        Stroke::new(
            1.0,
            theme::pick(Color32::from_black_alpha(70), theme::ink(18)),
        ),
    );
}

/// A one-point hairline across the width: between a card's rows, under a pane's toolbar.
pub fn divider(ui: &mut Ui) {
    line_across(ui, hairline());
}

/// [`divider`] in the separator colour, between the bands of a docked panel.
pub fn panel_divider(ui: &mut Ui) {
    line_across(ui, theme::separator());
}

fn line_across(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, color);
}
