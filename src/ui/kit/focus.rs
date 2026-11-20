//! Keyboard focus for the modal dialogs and sheets: where it starts when one opens, and the ring
//! drawn once the keyboard has moved it.

use eframe::egui::{self, Id, Painter, Rect, Stroke, StrokeKind};

/// Puts the focus on `first` from the frame after a modal opens, and leaves it alone afterwards.
/// The request repeats until the button has the focus. A modal that was not drawn on the previous
/// frame is a new one, so call this on every frame it is drawn and on no other.
///
/// The opening frame asks for nothing. egui lays a modal out in a sizing pass the first time only,
/// so a modal it has drawn before is live at once, and a button focused in that frame would answer
/// the very Enter that asked for the modal. From the next frame on that key is gone, and a press is
/// the person's own. Read the answer from the button's response, never from the key.
///
/// Returns whether the keyboard has moved the focus since the modal opened: only then is the ring
/// drawn, so a mouse user opens a clean modal.
pub fn start_on(ctx: &egui::Context, id: Id, first: Option<Id>) -> bool {
    let key = id.with("focus-state");
    let frame = ctx.cumulative_frame_nr();
    let (last, mut placed, mut keyboard): (u64, bool, bool) = ctx
        .data(|data| data.get_temp(key))
        .unwrap_or((u64::MAX, false, false));
    let opening = last == u64::MAX || frame > last + 1;
    if opening {
        (placed, keyboard) = (false, false);
    }
    if let Some(first) = first
        && !placed
        && !opening
    {
        if ctx.memory(|memory| memory.has_focus(first)) {
            placed = true;
        } else {
            ctx.memory_mut(|memory| memory.request_focus(first));
        }
    }
    keyboard |= ctx.input(|input| {
        input.key_pressed(egui::Key::Tab)
            || input.key_pressed(egui::Key::ArrowLeft)
            || input.key_pressed(egui::Key::ArrowRight)
    });
    ctx.data_mut(|data| data.insert_temp(key, (frame, placed, keyboard)));
    keyboard
}

/// The ring around a focused button of corner `radius` in `rect`. Never blue: it is the ink
/// colour.
pub fn paint_ring(painter: &Painter, rect: Rect, radius: f32) {
    painter.rect_stroke(
        rect.expand(2.0),
        radius + 2.0,
        Stroke::new(1.5, crate::ui::theme::ink(80)),
        StrokeKind::Outside,
    );
}
