//! Form controls: the switch, segmented controls, the stepper and the colour well.

use super::{MEDIUM, galley, hairline, menu_frame, outline, size};
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
    let segments: Vec<Segment<T>> = options
        .iter()
        .map(|(value, label)| Segment::new(*value, label))
        .collect();
    if let Some(value) = segments_with(ui, id, &segments, *selected, true, fill) {
        *selected = value;
    }
}

/// One choice of a [`segments`] control.
#[derive(Clone, Copy, Debug)]
pub struct Segment<'a, T> {
    pub value: T,
    pub label: &'a str,
    /// What hovering it says.
    pub tooltip: Option<&'a str>,
    /// A disabled choice is dimmed and cannot be picked.
    pub enabled: bool,
}

impl<'a, T> Segment<'a, T> {
    pub fn new(value: T, label: &'a str) -> Self {
        Self {
            value,
            label,
            tooltip: None,
            enabled: true,
        }
    }

    pub fn tooltip(mut self, tooltip: &'a str) -> Self {
        self.tooltip = Some(tooltip);
        self
    }
}

/// A segmented control with a tooltip on each choice, choices and the whole control that can be
/// disabled, and a focus ring for the keyboard. Returns the choice that was picked this frame.
/// A disabled control still shows its choice, dimmed.
pub fn segments<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: Id,
    options: &[Segment<'_, T>],
    selected: T,
    enabled: bool,
) -> Option<T> {
    segments_with(ui, id, options, selected, enabled, None)
}

/// The room [`segments`] takes for `options`, whichever one is chosen.
pub fn segments_width<T>(ui: &Ui, options: &[Segment<'_, T>]) -> f32 {
    segment_widths(ui, options, None).iter().sum::<f32>() + 2.0 * outline::INSET
}

fn segment_labels<T: Copy + PartialEq>(
    ui: &Ui,
    options: &[Segment<'_, T>],
    selected: T,
) -> Vec<Arc<Galley>> {
    options
        .iter()
        .map(|option| {
            let weight = if option.value == selected {
                outline::SELECTED_WEIGHT
            } else {
                MEDIUM
            };
            segment_label(ui, option.label, weight)
        })
        .collect()
}

fn segment_label(ui: &Ui, label: &str, weight: f32) -> Arc<Galley> {
    galley(
        ui,
        label,
        size::SUBHEADLINE + 1.0,
        weight,
        theme::text_primary(),
        f32::INFINITY,
    )
}

/// The width of each segment: every label measured as the chosen (bold) one, so the control does
/// not change width as the choice moves; or an equal share of `fill`.
fn segment_widths<T>(ui: &Ui, options: &[Segment<'_, T>], fill: Option<f32>) -> Vec<f32> {
    match fill {
        Some(total) => {
            let share = (total - 2.0 * outline::INSET) / options.len().max(1) as f32;
            vec![share; options.len()]
        }
        None => options
            .iter()
            .map(|option| {
                segment_label(ui, option.label, outline::SELECTED_WEIGHT)
                    .size()
                    .x
                    + 24.0
            })
            .collect(),
    }
}

fn segments_with<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: Id,
    options: &[Segment<'_, T>],
    selected: T,
    enabled: bool,
    fill: Option<f32>,
) -> Option<T> {
    let labels = segment_labels(ui, options, selected);
    let widths = segment_widths(ui, options, fill);
    let total = widths.iter().sum::<f32>() + 2.0 * outline::INSET;
    let (track, _) = ui.allocate_exact_size(vec2(total, outline::HEIGHT), Sense::hover());
    outline::paint_track(ui.painter(), track, outline::RADIUS, outline::Wash::Rest);
    let plate_height = outline::HEIGHT - 2.0 * outline::INSET;
    let mut picked = None;
    let mut x = track.left() + outline::INSET;
    for (index, (option, galley)) in options.iter().zip(labels).enumerate() {
        let usable = enabled && option.enabled;
        let segment = outline::plate_rect(x, track.top(), widths[index], plate_height);
        let response = ui.interact(
            segment,
            id.with(index),
            if usable {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        let mut response = crate::ui::accessibility::label(response, option.label);
        if let Some(tooltip) = option.tooltip {
            response = response.on_hover_text(tooltip);
        }
        if usable {
            response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        }
        let fade = if usable { 1.0 } else { 0.45 };
        if option.value == selected {
            outline::paint_plate(ui.painter(), segment, outline::PLATE_RADIUS, !enabled);
        } else if index > 0 && options[index - 1].value != selected {
            outline::paint_divider(ui.painter(), segment.left() - 0.5, segment);
        }
        if usable && response.has_focus() {
            ui.painter().rect_stroke(
                segment,
                outline::PLATE_RADIUS,
                Stroke::new(1.5, theme::text_secondary()),
                StrokeKind::Inside,
            );
        }
        ui.painter().galley(
            segment.center() - galley.size() / 2.0,
            galley,
            theme::text_primary().gamma_multiply(fade),
        );
        if usable && response.clicked() {
            picked = Some(option.value);
        }
        x += widths[index];
    }
    picked
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
