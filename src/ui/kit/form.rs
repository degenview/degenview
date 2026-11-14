//! Form controls: the switch, segmented controls, the stepper and the colour well.

use super::{MEDIUM, REGULAR, galley, hairline, menu_frame, primary, size};
use crate::ui::icons::{self, Icon};
use crate::ui::theme;
use eframe::egui::{
    self, Color32, Galley, Id, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use std::sync::Arc;

/// A macOS switch at the small control size; returns whether it was flipped.
pub fn switch(ui: &mut Ui, on: &mut bool, label: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(32.0, 18.0), Sense::click());
    let response = crate::ui::accessibility::label(response, label);
    if response.clicked() {
        *on = !*on;
    }
    let t = ui.ctx().animate_bool(response.id, *on);
    paint_switch(ui.painter(), rect, t, false);
    response
}

/// A small segmented control that hugs its labels (`Picker` with `.segmented` style).
pub fn segmented<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: Id,
    options: &[(T, &str)],
    selected: &mut T,
) {
    segmented_with(ui, id, options, selected, None);
}

/// `segmented` stretched to `width`, as a `.segmented` picker fills a column.
pub fn segmented_filling<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: Id,
    options: &[(T, &str)],
    selected: &mut T,
    width: f32,
) {
    segmented_with(ui, id, options, selected, Some(width));
}

fn segmented_with<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: Id,
    options: &[(T, &str)],
    selected: &mut T,
    fill: Option<f32>,
) {
    let labels: Vec<Arc<Galley>> = options
        .iter()
        .map(|(value, label)| {
            galley(
                ui,
                label,
                size::SUBHEADLINE + 0.5,
                if *value == *selected { MEDIUM } else { REGULAR },
                theme::text_primary(),
                f32::INFINITY,
            )
        })
        .collect();
    let widths: Vec<f32> = match fill {
        Some(total) => vec![(total - 4.0) / options.len().max(1) as f32; options.len()],
        None => labels.iter().map(|label| label.size().x + 20.0).collect(),
    };
    let total = widths.iter().sum::<f32>() + 4.0;
    let (track, _) = ui.allocate_exact_size(vec2(total, 22.0), Sense::hover());
    ui.painter().rect_filled(track, 6.0, primary(0.1));
    let mut x = track.left() + 2.0;
    for (index, ((value, label), galley)) in options.iter().zip(labels).enumerate() {
        let segment = Rect::from_min_size(pos2(x, track.top() + 2.0), vec2(widths[index], 18.0));
        let response = ui
            .interact(segment, id.with(index), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let response = crate::ui::accessibility::label(response, *label);
        if *value == *selected {
            ui.painter().add(
                egui::Shadow {
                    offset: [0, 1],
                    blur: 2,
                    spread: 0,
                    color: Color32::from_black_alpha(50),
                }
                .as_shape(segment, 5.0),
            );
            ui.painter()
                .rect_filled(segment, 5.0, theme::pick(primary(0.24), Color32::WHITE));
        } else if index > 0 && options[index - 1].0 != *selected {
            ui.painter().vline(
                segment.left() - 0.5,
                (segment.top() + 4.0)..=(segment.bottom() - 4.0),
                Stroke::new(1.0, primary(0.12)),
            );
        }
        ui.painter().galley(
            segment.center() - galley.size() / 2.0,
            galley,
            theme::text_primary(),
        );
        if response.clicked() {
            *selected = *value;
        }
        x += widths[index];
    }
}

/// A switch painted into `rect` with the knob `progress` (0 off, 1 on) of the way across: an
/// inset well that fills with the accent colour, and a white knob that casts a faint shadow.
/// `dimmed` fades it for a disabled control.
pub fn paint_switch(painter: &egui::Painter, rect: Rect, progress: f32, dimmed: bool) {
    let fade = if dimmed { 0.45 } else { 1.0 };
    let radius = rect.height() / 2.0;
    painter.rect_filled(rect, radius, theme::well_fill().gamma_multiply(fade));
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, hairline().gamma_multiply(fade)),
        StrokeKind::Inside,
    );
    if progress > 0.0 {
        let on = fade * progress;
        painter.rect_filled(rect, radius, theme::text_secondary().gamma_multiply(on));
        // The top edge catches a little light, so the track is not one flat colour.
        painter.hline(
            (rect.left() + radius)..=(rect.right() - radius),
            rect.top() + 1.0,
            Stroke::new(1.0, Color32::from_white_alpha(70).gamma_multiply(on)),
        );
    }
    let knob = radius - 2.0;
    let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), progress);
    painter.circle_filled(
        pos2(x, rect.center().y + 1.0),
        knob,
        Color32::from_black_alpha(60).gamma_multiply(fade),
    );
    painter.circle_filled(
        pos2(x, rect.center().y),
        knob,
        Color32::WHITE.gamma_multiply(fade),
    );
}

/// Which chevron of a [`stepper`] was clicked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StepDirection {
    Up,
    Down,
}

/// The macOS stepper: a 16×22 capsule of up/down chevrons. Returns the value after a click,
/// clamped to `range` and moved by `step`.
pub fn stepper(
    ui: &mut Ui,
    id: Id,
    value: i64,
    range: std::ops::RangeInclusive<i64>,
    step: i64,
) -> Option<i64> {
    let delta = match stepper_arrows(ui, id)? {
        StepDirection::Up => step,
        StepDirection::Down => -step,
    };
    Some(
        value
            .saturating_add(delta)
            .clamp(*range.start(), *range.end()),
    )
    .filter(|next| *next != value)
}

/// The stepper's capsule alone: reports which chevron was clicked and leaves the arithmetic to
/// the caller, for values that are not whole numbers.
pub fn stepper_arrows(ui: &mut Ui, id: Id) -> Option<StepDirection> {
    use StepDirection as Direction;
    let (rect, _) = ui.allocate_exact_size(vec2(16.0, 22.0), Sense::hover());
    ui.painter().rect_filled(rect, 5.0, theme::control_fill());
    ui.painter().rect_stroke(
        rect,
        5.0,
        Stroke::new(1.0, theme::separator()),
        StrokeKind::Inside,
    );
    let mut clicked = None;
    for (index, (icon, direction)) in [
        (Icon::ChevronUp, Direction::Up),
        (Icon::ChevronDown, Direction::Down),
    ]
    .into_iter()
    .enumerate()
    {
        let half = Rect::from_min_size(
            pos2(rect.min.x, rect.min.y + index as f32 * 11.0),
            vec2(16.0, 11.0),
        );
        let response = ui.interact(half, id.with(index), Sense::click());
        if response.hovered() {
            ui.painter()
                .rect_filled(half.shrink(1.0), 4.0, theme::ink(18));
        }
        icons::paint(
            ui.painter(),
            Rect::from_center_size(half.center(), Vec2::splat(7.0)),
            icon,
            theme::text_primary(),
        );
        if response.clicked() {
            clicked = Some(direction);
        }
    }
    clicked
}

/// `ColorPicker` as the macOS well: a rounded swatch (over a checkerboard when it is translucent)
/// that opens [`color_picker`](crate::ui::kit::color_picker). Returns the new colour when it changed.
pub fn color_well(
    ui: &mut Ui,
    size: Vec2,
    name: &str,
    color: Color32,
    alpha: egui::color_picker::Alpha,
) -> Option<Color32> {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let response = crate::ui::accessibility::label(response, name)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    crate::ui::kit::color_picker::paint_swatch(ui.painter(), rect, color, 6.0);
    if response.hovered() {
        ui.painter().rect_stroke(
            rect,
            6.0,
            Stroke::new(1.0, theme::ink(60)),
            StrokeKind::Inside,
        );
    }
    let mut picked = None;
    egui::Popup::from_toggle_button_response(&response)
        .frame(menu_frame().inner_margin(egui::Margin::same(12)))
        .show(|ui| {
            let mut value = color;
            if crate::ui::kit::color_picker::show(ui, &mut value, alpha) {
                picked = Some(value);
            }
        });
    picked
}
