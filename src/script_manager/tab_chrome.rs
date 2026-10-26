//! What the drawer's tabs have in common, so Report, Problems and Profile look like the Logs tab
//! beside them: a toolbar row with a rounded state field, a hairline under it, cards of rows, and
//! a body that spends the width before the height and scrolls rather than clips.
//!
//! The Logs tab lives in `ui::pine_logs_view`, which the chart workspace shares, so it keeps its
//! own copies of the toolbar constants; keep `TOOLBAR_HEIGHT`, `MARGIN` and `BUTTON` equal to its.

use crate::ui::kit;
use crate::ui::theme;
use eframe::egui::{
    self, Align, Color32, FontId, Layout, Margin, ScrollArea, Sense, Stroke, StrokeKind, Ui, pos2,
    vec2,
};

pub const TOOLBAR_HEIGHT: f32 = 46.0;
pub const MARGIN: f32 = 12.0;
pub const BUTTON: f32 = 26.0;
pub const FIELD_HEIGHT: f32 = 24.0;

/// The width from which a tab puts its parts side by side instead of one under the other.
pub const WIDE: f32 = 520.0;
/// The space between two columns.
pub const GAP: f32 = 12.0;

/// Whether the room left is wide enough for columns.
pub fn wide(ui: &Ui) -> bool {
    ui.available_width() >= WIDE
}

/// A one-pixel hairline across the pane.
pub fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, kit::hairline());
}

/// The toolbar row, its hairline included. `content` gets the row's width and starts after the
/// margin; the rows above and below touch, so the toolbar's content is centred in it whatever
/// surrounds it.
pub fn toolbar(ui: &mut Ui, content: impl FnOnce(&mut Ui, f32)) {
    let spacing = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = 0.0;
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        vec2(width, TOOLBAR_HEIGHT),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
            ui.add_space(MARGIN);
            content(ui, width);
        },
    );
    divider(ui);
    ui.spacing_mut().item_spacing.y = spacing;
}

/// The room the state field has when `buttons` icon buttons follow it.
pub fn field_width(row: f32, buttons: usize) -> f32 {
    let beside = if buttons == 0 {
        0.0
    } else {
        buttons as f32 * BUTTON + (buttons as f32 - 1.0) * 2.0 + 8.0
    };
    (row - MARGIN * 2.0 - beside).max(60.0)
}

/// A rounded field in the toolbar: a coloured dot, what the tab is showing, and, at the right edge
/// if there is room beside the words, a second text in `trailing`.
pub fn state_field(
    ui: &mut Ui,
    width: f32,
    dot: Color32,
    label: &str,
    trailing: Option<(&str, Color32)>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, FIELD_HEIGHT), Sense::hover());
    let response = crate::ui::accessibility::label(response, label);
    ui.painter().rect_filled(rect, 6.0, kit::primary(0.1));
    ui.painter().rect_stroke(
        rect,
        6.0,
        Stroke::new(1.0, kit::primary(0.06)),
        StrokeKind::Inside,
    );
    ui.painter()
        .circle_filled(pos2(rect.left() + 13.0, rect.center().y), 3.5, dot);
    let words = kit::galley(
        ui,
        label,
        kit::size::BODY,
        kit::REGULAR,
        theme::text_primary(),
        width - 34.0,
    );
    let words_right = rect.left() + 26.0 + words.size().x;
    if let Some((text, color)) = trailing {
        let room = rect.right() - 10.0 - words_right - 16.0;
        if room > 40.0 {
            let galley = kit::galley(ui, text, kit::size::SUBHEADLINE, kit::REGULAR, color, room);
            ui.painter().galley(
                pos2(
                    rect.right() - 10.0 - galley.size().x,
                    rect.center().y - galley.size().y / 2.0,
                ),
                galley,
                color,
            );
        }
    }
    ui.painter().galley(
        pos2(rect.left() + 26.0, rect.center().y - words.size().y / 2.0),
        words,
        theme::text_primary(),
    );
    response
}

/// A small heading above a list or a card.
pub fn caption(ui: &mut Ui, text: &str) {
    ui.label(kit::text(
        text,
        kit::size::SUBHEADLINE,
        kit::SEMIBOLD,
        theme::text_secondary(),
    ));
    ui.add_space(6.0);
}

/// The rounded card that holds a list of rows.
pub fn card<R>(ui: &mut Ui, rows: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(theme::ink(7))
        .stroke(Stroke::new(1.0, theme::ink(14)))
        .corner_radius(10.0)
        .inner_margin(Margin::symmetric(0, 3))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            rows(ui)
        })
        .inner
}

/// The line between two rows of a card.
pub fn hairline(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        rect.left() + 12.0..=rect.right() - 12.0,
        rect.center().y,
        Stroke::new(1.0, theme::ink(10)),
    );
}

/// A small capsule of text in `tint`, like the counts in the tab strip.
pub fn chip(ui: &mut Ui, text: &str, tint: Color32) {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), FontId::proportional(9.0), tint);
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 14.0, 15.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, 8.0, theme::with_alpha(tint, 0.14));
    ui.painter().galley(
        pos2(rect.min.x + 7.0, rect.center().y - galley.size().y / 2.0),
        galley,
        tint,
    );
}

/// The body of a tab: scrolls, so a short drawer shows the rest by scrolling instead of cutting it.
pub fn scroll<R>(ui: &mut Ui, id: &str, content: impl FnOnce(&mut Ui) -> R) {
    ScrollArea::vertical()
        .id_salt(id)
        .auto_shrink([false, false])
        .min_scrolled_height(0.0)
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(Margin::symmetric(MARGIN as i8, 12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    content(ui)
                })
                .inner
        });
}

/// Two columns of `left_fraction` and what is left of the width, tops aligned. A column only takes
/// the height of what it holds, so the shorter one leaves room under it.
pub fn columns(
    ui: &mut Ui,
    left_fraction: f32,
    left: impl FnOnce(&mut Ui),
    right: impl FnOnce(&mut Ui),
) {
    let width = ui.available_width();
    let left_width = ((width - GAP) * left_fraction).floor();
    let right_width = width - GAP - left_width;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        for (width, content) in [
            (left_width, Box::new(left) as Box<dyn FnOnce(&mut Ui)>),
            (right_width, Box::new(right)),
        ] {
            ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(width);
                content(ui);
            });
        }
    });
}

/// A quiet strip above the figures: a caution about what follows.
pub fn banner(ui: &mut Ui, text: &str) {
    egui::Frame::new()
        .fill(theme::with_alpha(theme::profiler_caution(), 0.12))
        .corner_radius(8.0)
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(kit::text(
                text,
                kit::size::SUBHEADLINE,
                kit::REGULAR,
                theme::profiler_caution(),
            ));
        });
    ui.add_space(10.0);
}
