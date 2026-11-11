//! The small painted parts of the Session statistics panel: section headings, label/value rows,
//! a bar for a share, the interval bar, and a gray link. They own no state and read no model; the
//! panel and its result cards compose them.

use crate::model::session_stats::Estimate;
use crate::ui::{accessibility, kit, theme};
use eframe::egui::{self, Align, Color32, Layout, Rect, Sense, Stroke, Ui, WidgetType, pos2, vec2};

pub const ROW_HEIGHT: f32 = 22.0;
const BAR_HEIGHT: f32 = 6.0;
const TEXT_SIZE: f32 = kit::size::SUBHEADLINE + 0.5;

/// The fill of a share bar and of the interval: the label colour, dimmed.
fn bar_fill() -> Color32 {
    theme::with_alpha(theme::text_primary(), 0.5)
}

fn track_fill() -> Color32 {
    kit::primary(0.1)
}

/// Text that wraps inside the width left, allocated and painted.
pub fn paragraph(ui: &mut Ui, text: &str, size: f32, weight: f32, color: Color32) {
    let galley = kit::paragraph(ui, text, size, weight, color, ui.available_width());
    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
    ui.painter().galley(rect.min, galley, color);
}

/// A heading for a group of rows.
pub fn section_label(ui: &mut Ui, text: &str) {
    ui.label(kit::text(
        text,
        kit::size::SUBHEADLINE,
        kit::MEDIUM,
        theme::text_secondary(),
    ));
}

/// A heading with something at the right end of its line.
pub fn section_header(ui: &mut Ui, text: &str, trailing: impl FnOnce(&mut Ui)) {
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        vec2(width, ROW_HEIGHT),
        Layout::left_to_right(Align::Center),
        |ui| {
            section_label(ui, text);
            ui.with_layout(Layout::right_to_left(Align::Center), trailing);
        },
    );
}

/// A row whose label and control share a centre line, however tall the control is.
pub fn centered_row(ui: &mut Ui, height: f32, contents: impl FnOnce(&mut Ui)) {
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        vec2(width, height),
        Layout::left_to_right(Align::Center),
        contents,
    );
}

/// A gray underlined text button: a link in this app is never blue.
pub fn link(ui: &mut Ui, label: &str) -> egui::Response {
    let color = theme::text_secondary();
    let galley = kit::galley(
        ui,
        label,
        kit::size::SUBHEADLINE,
        kit::REGULAR,
        color,
        f32::INFINITY,
    );
    let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::click());
    let response =
        accessibility::label(response, label).on_hover_cursor(egui::CursorIcon::PointingHand);
    ui.painter().galley(rect.min, galley, color);
    ui.painter().hline(
        rect.left()..=rect.right(),
        rect.bottom() - 1.0,
        Stroke::new(1.0, color),
    );
    response
}

/// A label at the left and its value at the right of one line.
pub fn key_value(ui: &mut Ui, key: &str, value: &str) {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::hover());
    accessibility::label_as(response, WidgetType::Label, format!("{key}: {value}"));
    let value_galley = kit::digits(
        ui,
        value,
        TEXT_SIZE,
        kit::MEDIUM,
        theme::text_primary(),
        width * 0.7,
    );
    let key_galley = kit::galley(
        ui,
        key,
        TEXT_SIZE,
        kit::REGULAR,
        theme::text_secondary(),
        (width - value_galley.size().x - 8.0).max(24.0),
    );
    let painter = ui.painter();
    painter.galley(
        pos2(rect.left(), rect.center().y - key_galley.size().y / 2.0),
        key_galley,
        theme::text_secondary(),
    );
    painter.galley(
        pos2(
            rect.right() - value_galley.size().x,
            rect.center().y - value_galley.size().y / 2.0,
        ),
        value_galley,
        theme::text_primary(),
    );
}

/// A bar for a share of days, the way the halves and the years show one. The share fills from
/// the left; an empty sample draws only its track.
pub fn share_bar(ui: &Ui, rect: Rect, estimate: &Estimate) {
    let track = Rect::from_center_size(rect.center(), vec2(rect.width(), BAR_HEIGHT));
    ui.painter()
        .rect_filled(track, BAR_HEIGHT / 2.0, track_fill());
    if let Some(share) = estimate.proportion().filter(|share| *share > 0.0) {
        let filled = Rect::from_min_size(
            track.min,
            vec2((track.width() * share as f32).max(BAR_HEIGHT), BAR_HEIGHT),
        );
        ui.painter()
            .rect_filled(filled, BAR_HEIGHT / 2.0, bar_fill());
    }
}

/// The width of the widest of `texts` as a share row writes them, so every row of a group gives
/// its bar the same length.
pub fn text_width(ui: &Ui, texts: &[String]) -> f32 {
    texts
        .iter()
        .map(|text| {
            kit::digits(
                ui,
                text,
                TEXT_SIZE,
                kit::MEDIUM,
                theme::text_primary(),
                f32::INFINITY,
            )
            .size()
            .x
        })
        .fold(0.0, f32::max)
}

/// A line of `label`, a bar, and `share · hits/n` at the right. The bar takes what a text column
/// `text_width` wide leaves (see [`text_width`]) and is left out when that is too little to read.
pub fn share_row(
    ui: &mut Ui,
    label: &str,
    label_width: f32,
    text_width: f32,
    estimate: &Estimate,
    text: &str,
) {
    const GAP: f32 = 10.0;
    const MIN_BAR: f32 = 28.0;
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::hover());
    accessibility::label_as(response, WidgetType::Label, format!("{label}: {text}"));
    let value = kit::digits(
        ui,
        text,
        TEXT_SIZE,
        kit::MEDIUM,
        theme::text_primary(),
        width * 0.7,
    );
    let name = kit::galley(
        ui,
        label,
        TEXT_SIZE,
        kit::REGULAR,
        theme::text_secondary(),
        label_width,
    );
    ui.painter().galley(
        pos2(rect.left(), rect.center().y - name.size().y / 2.0),
        name,
        theme::text_secondary(),
    );
    let value_left = rect.right() - value.size().x;
    let column_left = rect.right() - text_width.max(value.size().x);
    let bar_left = rect.left() + label_width + GAP;
    let bar_right = column_left - GAP;
    if bar_right - bar_left >= MIN_BAR {
        share_bar(
            ui,
            Rect::from_min_max(pos2(bar_left, rect.top()), pos2(bar_right, rect.bottom())),
            estimate,
        );
    }
    ui.painter().galley(
        pos2(value_left, rect.center().y - value.size().y / 2.0),
        value,
        theme::text_primary(),
    );
}

/// The estimate on a 0% to 100% axis: a marker at the share, the 95% interval as a band around it.
/// The axis is always the whole range, so two answers can be compared by eye.
pub fn interval_bar(ui: &mut Ui, estimate: &Estimate) {
    const HEIGHT: f32 = 36.0;
    const TRACK: f32 = 8.0;
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, HEIGHT), Sense::hover());
    let (Some(share), Some((low, high))) = (estimate.proportion(), estimate.interval()) else {
        return;
    };
    accessibility::label_as(
        response,
        WidgetType::Label,
        format!(
            "{:.1}% of days, 95% interval {:.1}% to {:.1}%",
            share * 100.0,
            low * 100.0,
            high * 100.0
        ),
    );
    let at = |fraction: f64| rect.left() + width * fraction as f32;
    let track = Rect::from_min_size(pos2(rect.left(), rect.top() + 5.0), vec2(width, TRACK));
    let painter = ui.painter();
    painter.rect_filled(track, TRACK / 2.0, track_fill());
    // The interval, never thinner than a marker so a certain answer still shows its band.
    let band = Rect::from_min_max(pos2(at(low), track.top()), pos2(at(high), track.bottom()));
    let band = Rect::from_center_size(band.center(), vec2(band.width().max(TRACK), TRACK));
    painter.rect_filled(
        band,
        TRACK / 2.0,
        theme::with_alpha(theme::text_primary(), 0.28),
    );
    // The share itself.
    let marker = Rect::from_center_size(pos2(at(share), track.center().y), vec2(3.0, 16.0));
    painter.rect_filled(marker, 1.5, theme::text_primary());
    // The axis.
    for (fraction, label, align) in [
        (0.0, "0%", egui::Align2::LEFT_TOP),
        (0.5, "50%", egui::Align2::CENTER_TOP),
        (1.0, "100%", egui::Align2::RIGHT_TOP),
    ] {
        painter.text(
            pos2(at(fraction), rect.bottom() - 12.0),
            align,
            label,
            egui::FontId::proportional(kit::size::CAPTION),
            theme::text_tertiary(),
        );
    }
}

/// A callout for something the reader should know about the numbers, in a tint.
pub fn callout(ui: &mut Ui, icon: crate::ui::icons::Icon, tint: Color32, title: &str, text: &str) {
    egui::Frame::new()
        .fill(theme::with_alpha(tint, 0.08))
        .corner_radius(10.0)
        .stroke(Stroke::new(1.0, theme::with_alpha(tint, 0.25)))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                crate::ui::icons::inline(ui, icon, 14.0, tint);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.label(kit::text(
                        title,
                        kit::size::SUBHEADLINE + 1.0,
                        kit::SEMIBOLD,
                        theme::text_primary(),
                    ));
                    paragraph(
                        ui,
                        text,
                        kit::size::CAPTION + 0.5,
                        kit::REGULAR,
                        theme::text_secondary(),
                    );
                });
            });
        });
}
