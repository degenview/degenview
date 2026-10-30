//! The building blocks of Swift's settings pages: a titled section, a `SettingsCardRow` (icon
//! badge, title and hint on the left, the control anchored right) and the macOS switch.

use super::icons::Icon;
use super::theme;
use eframe::egui::{self, Align, CursorIcon, Layout, Sense, Ui, Vec2, vec2};

/// Width kept for a card's control when its text wraps.
const CONTROL_RESERVE: f32 = 130.0;
/// The least room the words of a card may be left beside its control: below it the control moves
/// under them.
const MIN_TEXT_ROOM: f32 = 120.0;
/// The height of the row a stacked card's control sits in.
const STACKED_CONTROL_HEIGHT: f32 = 28.0;

/// A section heading: `.title3` semibold with an optional `.subheadline` secondary line.
pub fn section_title(ui: &mut Ui, title: &str, subtitle: Option<&str>) {
    ui.spacing_mut().item_spacing.y = 2.0;
    ui.label(super::kit::text(
        title,
        15.0,
        super::kit::SEMIBOLD,
        theme::text_primary(),
    ));
    if let Some(subtitle) = subtitle {
        ui.label(super::kit::text(
            subtitle,
            11.5,
            super::kit::REGULAR,
            theme::text_secondary(),
        ));
    }
    ui.add_space(8.0);
}

/// One setting as a card: the control sits on the right at its natural size. Returns what
/// `control` returns.
pub fn card<R>(
    ui: &mut Ui,
    icon: Icon,
    title: &str,
    hint: Option<&str>,
    control: impl FnOnce(&mut Ui) -> R,
) -> R {
    card_reserving(ui, icon, title, hint, CONTROL_RESERVE, control)
}

/// [`card`] for a control wider than the usual one: `reserve` points are kept for it, and the
/// hint wraps within what is left.
pub fn card_reserving<R>(
    ui: &mut Ui,
    icon: Icon,
    title: &str,
    hint: Option<&str>,
    reserve: f32,
    control: impl FnOnce(&mut Ui) -> R,
) -> R {
    let width = ui.available_width();
    let mut result = None;
    // Begun by hand: whether the pointer is over the card is only known once it is laid out, and
    // that decides its fill and hairline.
    let mut prepared = super::kit::card_frame(false).begin(ui);
    let inner = &mut prepared.content_ui;
    let inner_width = width - 28.0 - 2.0;
    inner.set_width(inner_width);
    // The text and the control side by side, unless that leaves the text too little room: then the
    // control goes under it, so a wide control never runs over the words beside it.
    let text_room = inner_width - 30.0 - 14.0 * 2.0 - reserve;
    let stacked = text_room < MIN_TEXT_ROOM;
    // Run in one of two places, whichever the card is laid out in.
    let mut control = Some(control);
    // At least the badge's height, so a card with no hint centres its title on it.
    let row = vec2(inner_width, 30.0);
    inner.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let (badge, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
        super::kit::paint_card_badge(ui, badge, icon);
        ui.vertical(|ui| {
            // The hint wraps within what the control leaves: a card's width never grows.
            ui.set_max_width(if stacked {
                inner_width - 30.0 - 14.0
            } else {
                text_room
            });
            ui.spacing_mut().item_spacing.y = 3.0;
            if hint.is_none() {
                // A lone title centres on the badge beside it.
                ui.add_space(7.0);
            }
            ui.label(super::kit::text(
                title,
                12.0,
                super::kit::SEMIBOLD,
                theme::text_primary(),
            ));
            if let Some(hint) = hint {
                ui.add(
                    egui::Label::new(super::kit::text(
                        hint,
                        10.5,
                        super::kit::REGULAR,
                        theme::text_secondary(),
                    ))
                    .wrap(),
                );
            }
        });
        if !stacked {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                result = control.take().map(|control| control(ui));
            });
        }
    });
    if stacked {
        inner.add_space(10.0);
        // A fixed row: a bare right-to-left layout claims all the height that is left.
        inner.allocate_ui_with_layout(
            vec2(inner_width, STACKED_CONTROL_HEIGHT),
            Layout::right_to_left(Align::Center),
            |ui| {
                result = control.take().map(|control| control(ui));
            },
        );
    }
    let hovered = ui.rect_contains_pointer(prepared.content_ui.min_rect().expand(12.0));
    prepared.frame = super::kit::card_frame(hovered);
    let response = prepared.end(ui);
    super::kit::paint_card_highlight(ui, response.rect);
    result.expect("the control ran")
}

/// A macOS-style switch: a 36×20 capsule with a sliding knob. Returns whether it was toggled.
pub fn switch(ui: &mut Ui, on: &mut bool) -> bool {
    switch_enabled(ui, on, true)
}

pub fn switch_enabled(ui: &mut Ui, on: &mut bool, enabled: bool) -> bool {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(36.0, 20.0), sense);
    let t = ui.ctx().animate_bool_with_time(response.id, *on, 0.12);
    super::kit::paint_switch(ui.painter(), rect, t, !enabled);
    let clicked = enabled && response.on_hover_cursor(CursorIcon::PointingHand).clicked();
    if clicked {
        *on = !*on;
    }
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_runs_its_control_and_the_switch_toggles() {
        let ctx = egui::Context::default();
        let mut on = false;
        let mut ran = false;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            section_title(ui, "Indicators", Some("Add context."));
            card(
                ui,
                Icon::Hash,
                "Volume",
                Some("Turnover per candle."),
                |ui| {
                    ran = true;
                    switch(ui, &mut on);
                },
            );
        });
        output.textures_delta.clear();
        assert!(ran && !on, "drawn, and not toggled without a click");
    }

    /// Runs one card of `card_width` points whose control is `control_width` wide and reserves
    /// `reserve`, and says where the control and the whole card ended up.
    fn laid_out(card_width: f32, control_width: f32, reserve: f32) -> (egui::Rect, egui::Rect) {
        let ctx = egui::Context::default();
        crate::ui::theme::install_offscreen(&ctx);
        let mut control = egui::Rect::NOTHING;
        let mut whole = egui::Rect::NOTHING;
        for _ in 0..2 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        vec2(card_width, 600.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    card_reserving(
                        ui,
                        Icon::Hash,
                        "Font",
                        Some("Built in: JetBrains Mono. Or any monospaced font installed on this Mac."),
                        reserve,
                        |ui| {
                            control = ui.allocate_exact_size(vec2(control_width, 24.0), Sense::hover()).0;
                        },
                    );
                    whole = ui.min_rect();
                },
            );
            output.textures_delta.clear();
        }
        (control, whole)
    }

    #[test]
    fn a_control_beside_the_words_leaves_them_their_room() {
        let (control, whole) = laid_out(600.0, 190.0, 224.0);
        assert!(control.right() <= whole.right(), "{control:?} {whole:?}");
        // The words are to the left of it: the badge is 30 points and the words start after it.
        assert!(control.left() > 30.0 + 14.0 + 120.0, "{control:?}");
        assert!(whole.height() < 90.0, "one row: {whole:?}");
    }

    #[test]
    fn a_control_too_wide_to_sit_beside_the_words_goes_under_them() {
        // Room for the badge and 120 points of words, but not a 220-point control as well.
        let (control, whole) = laid_out(300.0, 190.0, 224.0);
        assert!(
            control.right() <= whole.right(),
            "inside the card: {control:?} {whole:?}"
        );
        assert!(control.left() >= 0.0);
        assert!(
            control.top() > 50.0,
            "under the words, not over them: {control:?}"
        );
        // ...and the card is no taller than its two rows, instead of claiming the screen.
        assert!(whole.height() < 150.0, "{whole:?}");
        // The narrowest Settings page (a 520-point window less its 200-point sidebar).
        let (control, whole) = laid_out(272.0, 130.0, 164.0);
        assert!(
            control.right() <= whole.right() && whole.height() < 150.0,
            "{control:?} {whole:?}"
        );
    }
}
