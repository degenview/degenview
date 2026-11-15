//! The preview: the image as last made, fitted into a rounded well. Dragging it moves the
//! picture inside the frame; the arrow keys do the same a step at a time, Home or 0 and a
//! double-click put the position back. Where the picture cannot be moved (an arrangement of
//! several charts) it only shows.

use super::{Action, Image, Input, Output, PAD, Preview};
use crate::ui::kit::{self, REGULAR, size};
use crate::ui::theme;
use eframe::egui::{
    self, Color32, CursorIcon, Id, Key, Modifiers, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2,
    vec2,
};

/// The well's margin around the picture, and its corner radius.
const INSET: f32 = 12.0;
const RADIUS: f32 = 10.0;
/// The room above and below the well.
const GAP: f32 = 4.0;

/// The largest rectangle of `aspect` (width over height) that fits in `room`, centred in it.
pub(super) fn fit(room: Rect, aspect: f32) -> Rect {
    if !(aspect.is_finite() && aspect > 0.0 && room.is_positive()) {
        return Rect::from_center_size(room.center(), Vec2::ZERO);
    }
    let size = if room.width() / room.height() > aspect {
        vec2(room.height() * aspect, room.height())
    } else {
        vec2(room.width(), room.width() / aspect)
    };
    Rect::from_center_size(room.center(), size)
}

/// Where a picture of `pixels` is drawn in `room`: as large as fits, and exactly its own size
/// in points (one texel to one pixel) where `room` is what it was asked for, on the pixel grid.
fn picture_rect(room: Rect, pixels: [usize; 2], pixels_per_point: f32) -> Rect {
    let natural = vec2(pixels[0] as f32, pixels[1] as f32) / pixels_per_point;
    let fitted = fit(room, natural.x / natural.y);
    let snap = |value: f32| (value * pixels_per_point).round() / pixels_per_point;
    // A picture within a pixel of the room's size is drawn at its own size.
    if (fitted.size() - natural).abs().max_elem() <= 1.5 / pixels_per_point {
        let min = pos2(
            snap(room.center().x - natural.x / 2.0),
            snap(room.center().y - natural.y / 2.0),
        );
        return Rect::from_min_size(min, natural);
    }
    fitted
}

pub(super) fn show(ui: &mut Ui, input: &Input<'_>, width: f32, height: f32, output: &mut Output) {
    let (area, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let well = Rect::from_min_max(
        pos2(area.left() + PAD, area.top() + GAP),
        pos2(area.right() - PAD, area.bottom() - GAP),
    );
    let painter = ui.painter();
    painter.rect_filled(well, RADIUS, theme::well_fill());
    painter.rect_stroke(
        well,
        RADIUS,
        Stroke::new(1.0, theme::card_border(false)),
        StrokeKind::Inside,
    );
    let room = well.shrink(INSET);
    let pixels_per_point = ui.ctx().pixels_per_point();
    output.preview_pixels = (
        (room.width() * pixels_per_point).floor().max(1.0) as u32,
        (room.height() * pixels_per_point).floor().max(1.0) as u32,
    );
    match input.preview {
        Preview {
            image: Some(image), ..
        } => picture(ui, input, room, image, pixels_per_point, output),
        Preview {
            failure: Some(failure),
            ..
        } => message(ui, room, failure),
        Preview { .. } => kit::loading::message(ui, room, "Drawing the image…"),
    }
}

fn message(ui: &Ui, room: Rect, text: &str) {
    let galley = kit::galley(
        ui,
        text,
        size::BODY,
        REGULAR,
        theme::text_secondary(),
        room.width() - 2.0 * INSET,
    );
    ui.painter().galley(
        room.center() - galley.size() / 2.0,
        galley,
        theme::text_secondary(),
    );
}

fn picture(
    ui: &mut Ui,
    input: &Input<'_>,
    room: Rect,
    image: Image,
    pixels_per_point: f32,
    output: &mut Output,
) {
    let rect = picture_rect(room, image.pixels, pixels_per_point);
    let id = Id::new("studio-preview");
    let sense = if input.reframable {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, id, sense);
    ui.painter().image(
        image.texture,
        rect,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    ui.painter().rect_stroke(
        rect,
        0.0,
        Stroke::new(1.0, theme::ink(18)),
        StrokeKind::Outside,
    );
    let response = crate::ui::accessibility::label_as(
        response,
        egui::WidgetType::Other,
        "Preview. Drag to move the picture; arrow keys move it, Home resets.",
    );
    if !input.reframable {
        response.on_hover_text("Moving the picture is not available for several charts");
        return;
    }
    if response.is_pointer_button_down_on() {
        response.request_focus();
    }
    let mut dragging = false;
    if response.dragged() {
        dragging = true;
        let scale = image.composition.x / rect.width().max(1.0);
        if let Some(total) = response.total_drag_delta() {
            output.actions.push(Action::DragPreview(total * scale));
        }
    }
    if response.drag_stopped() {
        output.actions.push(Action::ReleasePreview);
    }
    if response.double_clicked() {
        output.actions.push(Action::ResetPosition);
    }
    if response.has_focus() {
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: false,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: false,
                },
            );
        });
        keys(ui, output);
        ui.painter().rect_stroke(
            rect.expand(3.0),
            3.0,
            Stroke::new(1.5, theme::text_secondary()),
            StrokeKind::Outside,
        );
    }
    if response.hovered() || dragging {
        ui.ctx().set_cursor_icon(if dragging {
            CursorIcon::Grabbing
        } else {
            CursorIcon::Grab
        });
    }
    output.interacting |= dragging;
}

/// The arrows move the picture the way they point, Home and 0 put it back.
fn keys(ui: &Ui, output: &mut Output) {
    ui.input_mut(|input| {
        let none = Modifiers::NONE;
        for (key, across, down) in [
            (Key::ArrowLeft, -1, 0),
            (Key::ArrowRight, 1, 0),
            (Key::ArrowUp, 0, -1),
            (Key::ArrowDown, 0, 1),
        ] {
            if input.consume_key(none, key) {
                output.actions.push(Action::NudgePreview { across, down });
            }
        }
        if input.consume_key(none, Key::Home) || input.consume_key(none, Key::Num0) {
            output.actions.push(Action::ResetPosition);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(width: f32, height: f32) -> Rect {
        Rect::from_min_size(pos2(10.0, 20.0), vec2(width, height))
    }

    #[test]
    fn a_picture_fits_the_room_whatever_its_shape() {
        let wide = fit(room(400.0, 200.0), 1.5);
        assert!((wide.width() / wide.height() - 1.5).abs() < 1e-4);
        assert_eq!(wide.height(), 200.0);
        assert_eq!(wide.center(), room(400.0, 200.0).center());
        let tall = fit(room(400.0, 200.0), 9.0 / 16.0);
        assert_eq!(tall.height(), 200.0);
        let flat = fit(room(100.0, 200.0), 2.0);
        assert_eq!(flat.width(), 100.0);
        assert_eq!(flat.height(), 50.0);
    }

    #[test]
    fn nothing_fits_in_no_room_or_for_no_shape() {
        assert_eq!(fit(room(0.0, 10.0), 1.0).size(), Vec2::ZERO);
        assert_eq!(fit(room(10.0, 10.0), 0.0).size(), Vec2::ZERO);
        assert_eq!(fit(room(10.0, 10.0), f32::NAN).size(), Vec2::ZERO);
    }

    #[test]
    fn a_picture_made_for_the_room_is_drawn_texel_for_pixel() {
        let room = room(401.0, 267.0);
        // 802 x 534 pixels at 2 points to a pixel: the room's own size.
        let rect = picture_rect(room, [802, 534], 2.0);
        assert_eq!(rect.size(), vec2(401.0, 267.0));
        for corner in [rect.min.x, rect.min.y] {
            assert_eq!((corner * 2.0).fract(), 0.0, "on the pixel grid");
        }
        // One made for another room is fitted into this one.
        let other = picture_rect(room, [300, 600], 2.0);
        assert!(other.height() <= 267.0 + 1e-3 && other.width() < 150.0 + 1e-3);
    }
}
