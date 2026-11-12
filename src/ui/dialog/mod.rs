//! The small modal dialogs: a confirmation, an alert that asks a question, a notice that informs.
//!
//! One look for all of them (`kit::confirm`, `kit::alert` and `kit::notice` draw through
//! [`show`]): a tinted icon badge with the title and message beside it, a close button in the
//! corner, and the buttons side by side at the bottom right, Cancel first and the action last. A
//! destructive action is a solid red button, any other action a neutral one; the tint only colours
//! the badge (never blue, see `docs/agent-guides/core.md`). Nothing here hand-draws a modal frame
//! elsewhere: a dialog with more to say passes a `body` that draws under the message.
//!
//! Keyboard: Escape and a click outside dismiss, Tab moves between the buttons left to right, Enter
//! presses the focused one. A destructive dialog starts on its dismiss button (Enter then cancels),
//! and a `cautious` one starts with no focus at all (Enter does nothing until Tab), so a stray Enter
//! never destroys or approves anything.

use crate::ui::accessibility;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD, size};
use crate::ui::theme;
use eframe::egui::{
    self, Color32, Id, Rect, Response, Sense, Shadow, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};

/// The width of the card's content; the card adds its margin.
pub const WIDTH: f32 = 400.0;
const MARGIN: i8 = 24;
const BADGE: f32 = 40.0;
/// Between the badge and the text.
const GAP: f32 = 14.0;
const CLOSE: f32 = 24.0;
const BUTTON_HEIGHT: f32 = 32.0;
const BUTTON_MIN_WIDTH: f32 = 88.0;
const BUTTON_PADDING: f32 = 18.0;
const BUTTON_GAP: f32 = 8.0;
/// Between the text and the buttons.
const FOOTER_GAP: f32 = 24.0;

/// What a dialog says and offers.
pub struct Dialog<'a> {
    pub icon: Icon,
    /// The badge's colour.
    pub tint: Color32,
    pub title: &'a str,
    /// Under the title; empty for none.
    pub message: &'a str,
    /// The action, on the right.
    pub accept: &'a str,
    /// The way out, left of the action; empty for a dialog that only informs.
    pub dismiss: &'a str,
    /// A third button on the far left ("Don't Save").
    pub leading: Option<&'a str>,
    /// The action destroys something: it is red, and the dialog starts on the dismiss button.
    pub destructive: bool,
    /// The action approves something that should not go through on a stray Enter (a request from
    /// a program, an upload): no button has the focus until the keyboard moves it there.
    pub cautious: bool,
    /// The action can be pressed.
    pub accept_enabled: bool,
    /// The `body` holds a field that takes the focus first, so no button asks for it.
    pub focus_body: bool,
}

impl<'a> Dialog<'a> {
    pub fn new(
        icon: Icon,
        tint: Color32,
        title: &'a str,
        message: &'a str,
        accept: &'a str,
    ) -> Self {
        Self {
            icon,
            tint,
            title,
            message,
            accept,
            dismiss: "Cancel",
            leading: None,
            destructive: false,
            cautious: false,
            accept_enabled: true,
            focus_body: false,
        }
    }
}

/// What the user chose.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Accepted,
    Dismissed,
    Leading,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Kind {
    Neutral,
    Prominent,
    Destructive,
}

/// Draws `spec` over a dimmed backdrop until it returns an outcome. `body` draws under the message
/// (a list, a field) and may be empty.
pub fn show(
    ctx: &egui::Context,
    id: Id,
    spec: &Dialog,
    body: impl FnOnce(&mut Ui),
) -> Option<Outcome> {
    let accept_id = id.with("accept");
    let dismiss_id = id.with("dismiss");
    let first = if spec.cautious {
        None
    } else if spec.destructive && !spec.dismiss.is_empty() {
        Some(dismiss_id)
    } else {
        Some(accept_id)
    };
    let ring = start_on(ctx, id, first.filter(|_| !spec.focus_body));

    let mut outcome = None;
    let modal = egui::Modal::new(id)
        .backdrop_color(Color32::from_black_alpha(110))
        .frame(
            egui::Frame::new()
                .fill(theme::chrome())
                .corner_radius(14.0)
                .stroke(Stroke::new(1.0, theme::ink(22)))
                .inner_margin(egui::Margin::same(MARGIN))
                .shadow(Shadow {
                    offset: [0, 12],
                    blur: 40,
                    spread: 0,
                    color: Color32::from_black_alpha(130),
                }),
        )
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let origin = ui.cursor().min;

            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                let (badge, _) = ui.allocate_exact_size(Vec2::splat(BADGE), Sense::hover());
                ui.painter()
                    .rect_filled(badge, 10.0, theme::with_alpha(spec.tint, 0.14));
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(badge.center(), Vec2::splat(20.0)),
                    spec.icon,
                    spec.tint,
                );
                ui.vertical(|ui| {
                    // Room left of the close button, so a long title never runs under it.
                    ui.set_width(WIDTH - BADGE - GAP - CLOSE);
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    ui.add(
                        egui::Label::new(kit::text(
                            spec.title,
                            size::TITLE2,
                            SEMIBOLD,
                            theme::text_primary(),
                        ))
                        .wrap(),
                    );
                    if !spec.message.is_empty() {
                        ui.add_space(6.0);
                        ui.add(
                            egui::Label::new(kit::text(
                                spec.message,
                                size::BODY,
                                REGULAR,
                                theme::text_secondary(),
                            ))
                            .wrap(),
                        );
                    }
                    body(ui);
                });
            });

            // In the corner, a little into the margin so it sits near the edge as a close button
            // does; the title's line is its centre.
            let close = Rect::from_min_size(
                pos2(origin.x + WIDTH - CLOSE + 10.0, origin.y - 12.0),
                Vec2::splat(CLOSE),
            );
            if close_button(ui, id.with("close"), close).clicked() {
                outcome = Some(Outcome::Dismissed);
            }

            ui.add_space(FOOTER_GAP);
            // Placed from the right edge, and created left to right so Tab follows the picture.
            let (row, _) = ui.allocate_exact_size(vec2(WIDTH, BUTTON_HEIGHT), Sense::hover());
            let kind = if spec.destructive {
                Kind::Destructive
            } else {
                Kind::Prominent
            };
            let accept_width = button_width(ui, spec.accept, kind);
            let dismiss_width = if spec.dismiss.is_empty() {
                0.0
            } else {
                button_width(ui, spec.dismiss, Kind::Neutral)
            };
            let slot = |left: f32, width: f32| {
                Rect::from_min_size(pos2(left, row.top()), vec2(width, BUTTON_HEIGHT))
            };
            let accept_rect = slot(row.right() - accept_width, accept_width);
            let dismiss_rect = slot(
                accept_rect.left() - BUTTON_GAP - dismiss_width,
                dismiss_width,
            );
            if let Some(label) = spec.leading {
                let width = button_width(ui, label, Kind::Neutral);
                let rect = slot(row.left(), width);
                if button(
                    ui,
                    rect,
                    id.with("leading"),
                    label,
                    Kind::Neutral,
                    true,
                    ring,
                )
                .clicked()
                {
                    outcome = Some(Outcome::Leading);
                }
            }
            if !spec.dismiss.is_empty()
                && button(
                    ui,
                    dismiss_rect,
                    dismiss_id,
                    spec.dismiss,
                    Kind::Neutral,
                    true,
                    ring,
                )
                .clicked()
            {
                outcome = Some(Outcome::Dismissed);
            }
            if button(
                ui,
                accept_rect,
                accept_id,
                spec.accept,
                kind,
                spec.accept_enabled,
                ring,
            )
            .clicked()
            {
                outcome = Some(Outcome::Accepted);
            }
        });
    if outcome.is_none() && modal.should_close() {
        outcome = Some(Outcome::Dismissed);
    }
    outcome
}

/// Puts the focus on `first` when the dialog opens, and leaves it alone afterwards. The first
/// frame of an egui modal is a sizing pass whose widgets do not count, so the request repeats until
/// the button has the focus. A dialog that was not drawn on the previous frame is a new one.
/// Returns whether the keyboard has moved the focus since it opened: only then is the focus ring
/// drawn, so a mouse user opens a clean dialog.
fn start_on(ctx: &egui::Context, id: Id, first: Option<Id>) -> bool {
    let key = id.with("focus-state");
    let frame = ctx.cumulative_frame_nr();
    let (last, mut placed, mut keyboard): (u64, bool, bool) = ctx
        .data(|data| data.get_temp(key))
        .unwrap_or((u64::MAX, false, false));
    if last == u64::MAX || frame > last + 1 {
        (placed, keyboard) = (false, false);
    }
    if let Some(first) = first
        && !placed
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

/// The X in the corner: a quiet glyph that takes a tint on hover.
fn close_button(ui: &mut Ui, id: Id, rect: Rect) -> Response {
    // Escape closes too, so the Tab order is the buttons alone.
    let response = ui.interact(rect, id, Sense::CLICK);
    let response = accessibility::label(response, "Close").on_hover_text("Close");
    if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(
            rect,
            7.0,
            theme::ink(if response.is_pointer_button_down_on() {
                26
            } else {
                14
            }),
        );
    }
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(10.0)),
        Icon::Xmark,
        if response.hovered() {
            theme::text_primary()
        } else {
            theme::text_secondary()
        },
    );
    response
}

/// A footer button's width: its label and padding, at least `BUTTON_MIN_WIDTH`.
fn button_width(ui: &Ui, label: &str, kind: Kind) -> f32 {
    let measure = kit::galley(
        ui,
        label,
        size::BODY,
        weight_of(kind),
        Color32::WHITE,
        f32::INFINITY,
    );
    (measure.size().x + 2.0 * BUTTON_PADDING).max(BUTTON_MIN_WIDTH)
}

fn weight_of(kind: Kind) -> f32 {
    if kind == Kind::Neutral {
        MEDIUM
    } else {
        SEMIBOLD
    }
}

/// One footer button in `rect`.
fn button(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    label: &str,
    kind: Kind,
    enabled: bool,
    ring: bool,
) -> Response {
    let weight = weight_of(kind);
    let response = ui.interact(
        rect,
        id,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let response = accessibility::label(response, label);
    let hovered = enabled && response.hovered();
    let pressed = enabled && response.is_pointer_button_down_on();
    let (fill, stroke, text) = match (kind, enabled) {
        (_, false) => (kit::primary(0.05), Stroke::NONE, theme::text_tertiary()),
        (Kind::Destructive, true) => (
            if pressed {
                kit::RED.gamma_multiply(0.8)
            } else if hovered {
                Color32::from_rgb(255, 92, 82)
            } else {
                kit::RED
            },
            Stroke::NONE,
            Color32::WHITE,
        ),
        (Kind::Prominent, true) => (
            kit::primary(if pressed {
                0.32
            } else if hovered {
                0.26
            } else {
                0.2
            }),
            Stroke::new(1.0, theme::ink(40)),
            theme::text_primary(),
        ),
        (Kind::Neutral, true) => (
            kit::primary(if pressed {
                0.18
            } else if hovered {
                0.12
            } else {
                0.08
            }),
            Stroke::new(1.0, theme::ink(26)),
            theme::text_primary(),
        ),
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 8.0, fill);
    if stroke != Stroke::NONE {
        painter.rect_stroke(rect, 8.0, stroke, StrokeKind::Inside);
    }
    if ring && response.has_focus() {
        // Never blue: the ring is the ink colour.
        painter.rect_stroke(
            rect.expand(2.0),
            10.0,
            Stroke::new(1.5, theme::ink(80)),
            StrokeKind::Outside,
        );
    }
    let galley = kit::galley(ui, label, size::BODY, weight, text, rect.width());
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
    response
}

#[cfg(test)]
mod tests;
