//! Capsules: status badges, icon chips, counts and quick-value pills.

use super::{MEDIUM, SEMIBOLD, Tone, digits, galley, primary, size};
use crate::ui::icons::{self, Icon};
use crate::ui::theme;
use eframe::egui::{self, Color32, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

/// The size `paint_status_badge` draws `label` at.
pub fn status_badge_size(ui: &Ui, label: &str) -> Vec2 {
    let galley = galley(
        ui,
        label,
        size::CAPTION + 0.5,
        SEMIBOLD,
        Color32::WHITE,
        f32::INFINITY,
    );
    vec2(
        9.0 + 7.0 + 6.0 + galley.size().x + 9.0,
        galley.size().y + 8.0,
    )
}

/// A tinted capsule with a dot and a semibold caption, its right edge at `right`.
pub fn paint_status_badge(ui: &Ui, right: f32, center_y: f32, label: &str, tone: Tone) -> Rect {
    let text = tone.text();
    let galley = galley(
        ui,
        label,
        size::CAPTION + 0.5,
        SEMIBOLD,
        text,
        f32::INFINITY,
    );
    let size = status_badge_size(ui, label);
    let rect = Rect::from_min_size(pos2(right - size.x, center_y - size.y / 2.0), size);
    let painter = ui.painter();
    painter.rect_filled(rect, size.y / 2.0, tone.fill());
    painter.rect_stroke(
        rect,
        size.y / 2.0,
        Stroke::new(1.0, tone.edge()),
        StrokeKind::Inside,
    );
    // The dot is where a status reads from, so it keeps more colour than the words.
    painter.circle_filled(pos2(rect.left() + 12.5, center_y), 3.5, tone.dot());
    painter.galley(
        pos2(rect.left() + 22.0, center_y - galley.size().y / 2.0),
        galley,
        text,
    );
    rect
}

/// `paint_status_badge`, laid out inline where the cursor is.
pub fn status_badge(ui: &mut Ui, label: &str, tone: Tone) -> Rect {
    let (rect, _) = ui.allocate_exact_size(status_badge_size(ui, label), Sense::hover());
    paint_status_badge(ui, rect.right(), rect.center().y, label, tone)
}

/// `paint_chip`'s size for `label`.
pub fn chip_size(ui: &Ui, label: &str) -> Vec2 {
    let galley = galley(
        ui,
        label,
        size::CAPTION + 0.5,
        MEDIUM,
        Color32::WHITE,
        f32::INFINITY,
    );
    vec2(
        8.0 + 10.0 + 5.0 + galley.size().x + 8.0,
        galley.size().y + 8.0,
    )
}

/// A tinted capsule with a small bold glyph and a medium caption, right edge at `right`.
pub fn paint_chip(
    ui: &Ui,
    right: f32,
    center_y: f32,
    icon: Icon,
    label: &str,
    tint: Color32,
) -> Rect {
    let galley = galley(ui, label, size::CAPTION + 0.5, MEDIUM, tint, f32::INFINITY);
    let size = vec2(
        8.0 + 10.0 + 5.0 + galley.size().x + 8.0,
        galley.size().y + 8.0,
    );
    let rect = Rect::from_min_size(pos2(right - size.x, center_y - size.y / 2.0), size);
    let painter = ui.painter();
    painter.rect_filled(rect, size.y / 2.0, theme::with_alpha(tint, 0.14));
    icons::paint(
        painter,
        Rect::from_center_size(pos2(rect.left() + 13.0, center_y), Vec2::splat(9.5)),
        icon,
        tint,
    );
    painter.galley(
        pos2(rect.left() + 23.0, center_y - galley.size().y / 2.0),
        galley,
        tint,
    );
    rect
}

/// A neutral count capsule ("12"), its left edge at `left`.
pub fn paint_count(
    ui: &Ui,
    left: f32,
    center_y: f32,
    count: usize,
    color: Color32,
    fill: Color32,
) -> Rect {
    let galley = digits(
        ui,
        &count.to_string(),
        size::CAPTION + 0.5,
        SEMIBOLD,
        color,
        f32::INFINITY,
    );
    let size = vec2(galley.size().x + 12.0, galley.size().y + 2.0);
    let rect = Rect::from_min_size(pos2(left, center_y - size.y / 2.0), size);
    ui.painter().rect_filled(rect, size.y / 2.0, fill);
    ui.painter().galley(
        pos2(rect.left() + 6.0, center_y - galley.size().y / 2.0),
        galley,
        color,
    );
    rect
}

/// A quick-value pill; the one already chosen is tinted with the accent.
pub fn value_chip(ui: &mut Ui, label: &str, selected: bool) -> egui::Response {
    let color = if selected {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    let text = galley(
        ui,
        label,
        size::CAPTION + 0.5,
        SEMIBOLD,
        color,
        f32::INFINITY,
    );
    let size = vec2(text.size().x + 20.0, text.size().y + 8.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let fill = if selected {
        theme::selected_tint(response.hovered())
    } else {
        primary(if response.hovered() { 0.1 } else { 0.06 })
    };
    ui.painter().rect_filled(rect, size.y / 2.0, fill);
    ui.painter()
        .galley(rect.center() - text.size() / 2.0, text, color);
    response
}
