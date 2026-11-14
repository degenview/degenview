//! The page's small controls: secondary text and captions, rows, fields, copy buttons, capsules,
//! the permissions overview and the clickable and action cards.

use super::{CONTROL_HEIGHT, CONTROL_RADIUS, COPIED_FOR, Copied, access_overview};
use crate::model::mcp::PermissionSet;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::Tone;
use crate::ui::{kit, theme};
use eframe::egui::{
    self, Align, Id, Layout, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use std::time::Instant;

pub(super) fn secondary(ui: &mut Ui, text: impl Into<String>) {
    ui.label(kit::text(
        text,
        kit::size::BODY,
        kit::REGULAR,
        theme::text_secondary(),
    ));
}

pub(super) fn caption(ui: &mut Ui, text: impl Into<String>) {
    ui.add(
        egui::Label::new(kit::text(
            text,
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            theme::text_secondary(),
        ))
        .wrap(),
    );
}

/// A row of a card: its title on the left, the value or control on the right, tall enough for a
/// 28-point control so rows holding one are not taller than the rest.
pub(super) fn row(ui: &mut Ui, title: &str, value: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(CONTROL_HEIGHT);
        ui.label(kit::text(
            title,
            kit::size::SUBHEADLINE + 1.0,
            kit::REGULAR,
            theme::text_primary(),
        ));
        ui.with_layout(Layout::right_to_left(Align::Center), value);
    });
}

/// A single-line text field exactly as tall as a button: a rounded capsule with a focus ring.
pub(super) fn field(
    ui: &mut Ui,
    id: Id,
    text: &mut String,
    hint: &str,
    width: f32,
) -> egui::Response {
    field_with_rect(ui, id, text, hint, width).0
}

/// [`field`], also giving the capsule's rectangle (the text edit inside is shorter than it).
pub(super) fn field_with_rect(
    ui: &mut Ui,
    id: Id,
    text: &mut String,
    hint: &str,
    width: f32,
) -> (egui::Response, Rect) {
    let focused = ui.memory(|memory| memory.has_focus(id));
    let (rect, _) = ui.allocate_exact_size(vec2(width, CONTROL_HEIGHT), Sense::hover());
    ui.painter()
        .rect_filled(rect, CONTROL_RADIUS, theme::quaternary(0.6));
    ui.painter().rect_stroke(
        rect,
        CONTROL_RADIUS,
        Stroke::new(
            1.0,
            if focused {
                theme::with_alpha(theme::text_secondary(), 0.7)
            } else {
                kit::primary(0.12)
            },
        ),
        StrokeKind::Inside,
    );
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(10.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    let response = kit::plain_field(
        &mut inner,
        id,
        text,
        hint,
        kit::size::BODY,
        kit::REGULAR,
        rect.width() - 20.0,
    );
    (response, rect)
}

/// A copy button that says "Copied" for a moment afterwards.
pub(super) fn copy_button(
    ui: &mut Ui,
    copied: &mut Option<(Copied, Instant)>,
    kind: Copied,
    label: &str,
    text: &str,
    prominent: bool,
) {
    copy_button_with(ui, copied, kind, label, prominent, || text.to_owned());
}

/// `copy_button` for text that costs something to make (the connection JSON): made only when the
/// button is clicked, never while it is merely drawn.
pub(super) fn copy_button_with(
    ui: &mut Ui,
    copied: &mut Option<(Copied, Instant)>,
    kind: Copied,
    label: &str,
    prominent: bool,
    text: impl FnOnce() -> String,
) {
    let recent = copied.is_some_and(|(what, at)| what == kind && at.elapsed() < COPIED_FOR);
    let shown = if recent { "Copied" } else { label };
    if kit::sheet_button(ui, shown, prominent && !recent, true).clicked() {
        ui.ctx().copy_text(text());
        *copied = Some((kind, Instant::now()));
    }
    if recent {
        ui.ctx().request_repaint_after(COPIED_FOR);
    }
}

/// A label capsule in a tone's soft colours (a faint plate, a legible shade, a hairline edge),
/// painted at its exact size. With a `width` the text is centred in a capsule that wide, so a
/// column of them lines up whatever the text.
pub(in crate::ui) fn capsule(
    ui: &mut Ui,
    text: &str,
    tone: Tone,
    width: Option<f32>,
) -> egui::Response {
    let ink = tone.text();
    let galley = kit::galley(
        ui,
        text,
        kit::size::CAPTION,
        kit::MEDIUM,
        ink,
        f32::INFINITY,
    );
    let size = vec2(
        width.unwrap_or(galley.size().x + 14.0),
        galley.size().y + 4.0,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, 6.0, tone.fill());
    ui.painter()
        .rect_stroke(rect, 6.0, Stroke::new(1.0, tone.edge()), StrokeKind::Inside);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, ink);
    response
}

/// What a set of permissions can do: one capsule ("Read only" is quiet, "Read & write" amber) and
/// the areas beside it, wrapping under it on a narrow line. Returns the area it fills, which never
/// reaches past the width the caller leaves.
pub(super) fn overview(ui: &mut Ui, set: &PermissionSet) -> Rect {
    let overview = access_overview(set);
    if overview.headline == "No access" {
        return ui
            .label(kit::text(
                "No access",
                kit::size::CAPTION + 0.5,
                kit::REGULAR,
                theme::text_tertiary(),
            ))
            .rect;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
        let tone = if overview.writes {
            Tone::Warning
        } else {
            Tone::Neutral
        };
        capsule(ui, overview.headline, tone, None);
        ui.label(kit::text(
            overview.detail,
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            theme::text_secondary(),
        ));
    })
    .response
    .rect
}

/// A card that opens something when clicked; a chevron on the right says so. Returns the click.
pub(super) fn clickable_card(ui: &mut Ui, id: Id, content: impl FnOnce(&mut Ui)) -> bool {
    let width = ui.available_width();
    let inner = kit::card_frame(false).show(ui, |ui| {
        ui.set_width(width - 30.0);
        // Leave room for the chevron.
        ui.scope(|ui| {
            ui.set_max_width(width - 30.0 - 26.0);
            content(ui);
        });
    });
    let rect = inner.response.rect;
    let response = ui.interact(rect, id, Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(
            rect,
            kit::CARD_RADIUS,
            kit::primary(if response.is_pointer_button_down_on() {
                0.07
            } else {
                0.04
            }),
        );
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            pos2(rect.right() - 22.0, rect.center().y),
            Vec2::splat(12.0),
        ),
        Icon::ChevronRight,
        theme::text_tertiary(),
    );
    response.clicked()
}

/// A settings card: an icon badge with a title and hint, and below them one control row as tall as
/// a button. The row is given a fixed height, so it can never stretch the card to fill the page.
pub(super) fn action_card(
    ui: &mut Ui,
    icon: Icon,
    title: &str,
    hint: &str,
    controls: impl FnOnce(&mut Ui),
) {
    let width = ui.available_width();
    kit::card_frame(false).show(ui, |ui| {
        ui.set_width(width - 30.0);
        ui.spacing_mut().item_spacing.y = 12.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            let (badge, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
            kit::paint_card_badge(ui, badge, icon);
            ui.vertical(|ui| {
                ui.set_max_width(width - 30.0 - 44.0);
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.label(kit::text(title, 12.5, kit::SEMIBOLD, theme::text_primary()));
                caption(ui, hint);
            });
        });
        // Right to left: the button goes in first, the field fills what is left of it.
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), CONTROL_HEIGHT),
            Layout::right_to_left(Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                controls(ui);
            },
        );
    });
}
