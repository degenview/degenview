//! Under the preview: the Width and Height rulers with Reset view, and the footer line with the
//! image's summary and the two ways to export it.

use super::{Action, Export, Input, Output, PAD};
use crate::model::chart::snapshot::framing::{Dial, MAX, MIN, Step, multiplier_text};
use crate::ui::icons::Icon;
use crate::ui::kit::ruler::{Ruler, RulerEvent, ruler};
use crate::ui::kit::{self, REGULAR, Tone, size};
use crate::ui::theme;
use eframe::egui::{self, Id, Rect, Sense, Stroke, Ui, pos2, vec2};

/// The height of the rulers' row, and of the footer with its hairline.
pub(super) const FRAMING_HEIGHT: f32 = 46.0;
pub(super) const BAR_HEIGHT: f32 = 62.0;
const RULER_WIDTH: f32 = 42.0 + 8.0 + 128.0 + 8.0 + 46.0;
const RULER_GAP: f32 = 36.0;
const BUTTON_HEIGHT: f32 = 32.0;
const NOT_FOR_LAYOUTS: &str = "Not available for several charts";

/// Width and Height, centred; Reset view at the right while the framing is not the capture's own.
pub(super) fn framing(ui: &mut Ui, input: &Input<'_>, width: f32, output: &mut Output) {
    let (row, _) = ui.allocate_exact_size(vec2(width, FRAMING_HEIGHT), Sense::hover());
    let config = input.config;
    let framing = config.framing;
    let at = pos2(
        row.center().x - (2.0 * RULER_WIDTH + RULER_GAP) / 2.0,
        row.center().y - 11.0,
    );
    for (index, (dial, label)) in [(Dial::Width, "Width"), (Dial::Height, "Height")]
        .into_iter()
        .enumerate()
    {
        let rect = Rect::from_min_size(
            pos2(at.x + index as f32 * (RULER_WIDTH + RULER_GAP), at.y),
            vec2(RULER_WIDTH, 22.0),
        );
        let value = framing.dial(dial);
        let readout = multiplier_text(value);
        let id = Id::new(("studio-dial", index));
        let mut sub = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        let result = ruler(
            &mut sub,
            &Ruler {
                id,
                label,
                name: label,
                value,
                range: (MIN, MAX),
                home: 1.0,
                readout: &readout,
                enabled: input.reframable,
                hint: NOT_FOR_LAYOUTS,
            },
        );
        output.interacting |= result.dragging;
        if input.reframable {
            result.response.on_hover_text(dial_hint(dial));
        }
        for event in result.events {
            output.actions.push(match event {
                RulerEvent::Dragged(value) => Action::DragDial(dial, value),
                RulerEvent::Released => Action::ReleaseDial,
                RulerEvent::Nudged { up, coarse } => Action::StepDial(
                    dial,
                    if coarse { Step::Coarse } else { Step::Fine },
                    if up { 1 } else { -1 },
                ),
                RulerEvent::Reset => Action::ResetDial(dial),
            });
        }
    }
    if input.reframable && !framing.is_default() {
        let button = reset_button(ui, row);
        if button.clicked() {
            output.actions.push(Action::ResetView);
        }
    }
}

fn dial_hint(dial: Dial) -> &'static str {
    match dial {
        Dial::Width => {
            "How much of the time axis the image shows. Drag the ruler left to zoom in, right to \
             zoom out; double-click to reset."
        }
        Dial::Height => {
            "How much of the price axis the image shows. Drag the ruler left to zoom in, right to \
             zoom out; double-click to reset."
        }
    }
}

/// `⟲ Reset view`, a quiet text button at the row's right end.
fn reset_button(ui: &mut Ui, row: Rect) -> egui::Response {
    let label = kit::galley(
        ui,
        "Reset view",
        size::SUBHEADLINE + 1.0,
        REGULAR,
        theme::text_primary(),
        f32::INFINITY,
    );
    let size = vec2(11.0 + 14.0 + 6.0 + label.size().x + 11.0, 26.0);
    let rect = Rect::from_min_size(
        pos2(row.right() - PAD - size.x, row.center().y - size.y / 2.0),
        size,
    );
    let response = ui
        .interact(rect, Id::new("studio-reset-view"), Sense::click())
        .on_hover_text("Back to the view the capture started with")
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let response = crate::ui::accessibility::label(response, "Reset view");
    let painter = ui.painter();
    if response.is_pointer_button_down_on() {
        painter.rect_filled(rect, 7.0, kit::primary(0.14));
    } else if response.hovered() {
        painter.rect_filled(rect, 7.0, kit::primary(0.08));
    }
    if response.has_focus() {
        painter.rect_stroke(
            rect,
            7.0,
            Stroke::new(1.5, theme::text_secondary()),
            egui::StrokeKind::Inside,
        );
    }
    crate::ui::icons::paint(
        painter,
        Rect::from_center_size(
            pos2(rect.left() + 11.0 + 7.0, rect.center().y),
            vec2(14.0, 14.0),
        ),
        Icon::ResetView,
        theme::text_secondary(),
    );
    painter.galley(
        pos2(
            rect.left() + 11.0 + 14.0 + 6.0,
            rect.center().y - label.size().y / 2.0,
        ),
        label,
        theme::text_primary(),
    );
    response
}

/// The hairline, the summary at the left and the export buttons at the right.
pub(super) fn bar(ui: &mut Ui, input: &Input<'_>, width: f32, output: &mut Output) {
    let (row, _) = ui.allocate_exact_size(vec2(width, BAR_HEIGHT), Sense::hover());
    ui.painter().hline(
        row.x_range(),
        row.top(),
        Stroke::new(1.0, theme::separator()),
    );
    let centre = row.center().y + 0.5;
    let mut x = row.right() - PAD;
    let busy = input.exporting;
    let can = input.can_export && busy.is_none();
    // Right to left: the main action first, so it ends the row.
    for (export, label, prominent) in [
        (Export::Download, "Download PNG", true),
        (Export::CopyImage, "Copy image", false),
    ] {
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(
                    pos2(row.left(), centre - BUTTON_HEIGHT / 2.0),
                    vec2(x - row.left(), BUTTON_HEIGHT),
                ))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        let response = kit::neutral_button(
            &mut child,
            label,
            prominent,
            can,
            busy == Some(export),
            104.0,
        );
        x = response.rect.left() - 10.0;
        if response.clicked() && can {
            output.actions.push(Action::Export(export));
        }
    }
    let room = (x - (row.left() + PAD) - 8.0).max(40.0);
    let (text, color) = match input.summary {
        Ok(text) => (text, theme::text_secondary()),
        Err(problem) => (problem, Tone::Warning.text()),
    };
    let galley = kit::galley_words(ui, text, size::CALLOUT, REGULAR, color, room);
    ui.painter().galley(
        pos2(row.left() + PAD, centre - galley.size().y / 2.0),
        galley,
        color,
    );
}
