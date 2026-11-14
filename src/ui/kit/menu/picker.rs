//! `Picker` in the `.menu` style: a bordered pull-down over a list of choices.

use super::{
    MENU_FRAME_MARGIN, MENU_MAX_HEIGHT, menu_frame, menu_item_marked_response, menu_width_for,
};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{REGULAR, galley, primary, size};
use crate::ui::theme;
use eframe::egui::{self, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

/// `Picker` in the `.menu` style: a bordered pull-down showing the current choice with up/down
/// chevrons. Returns the index chosen from `labels` this frame.
///
/// `width` is the trigger's width; a choice too long for it ends in an ellipsis there (with the
/// whole text on hover) while the menu opens wide enough for every row, scrolling when tall.
pub fn menu_picker(
    ui: &mut Ui,
    width: f32,
    labels: &[String],
    selected: usize,
    enabled: bool,
) -> Option<usize> {
    menu_picker_hinted(ui, width, labels, selected, enabled, "")
}

/// `menu_picker` that shows `hint`, quietly, while `selected` is not one of `labels`.
pub fn menu_picker_hinted(
    ui: &mut Ui,
    width: f32,
    labels: &[String],
    selected: usize,
    enabled: bool,
    hint: &str,
) -> Option<usize> {
    let chosen_label = labels.get(selected).map(String::as_str);
    let color = if enabled && chosen_label.is_some() {
        theme::text_primary()
    } else {
        theme::text_tertiary()
    };
    let current = chosen_label.unwrap_or(hint);
    let text = galley(ui, current, size::BODY, REGULAR, color, width - 30.0);
    let truncated = galley(ui, current, size::BODY, REGULAR, color, f32::INFINITY)
        .size()
        .x
        > width - 30.0 + 0.5;
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, 24.0), sense);
    let response = crate::ui::accessibility::label(response, current);
    let response = if truncated && !current.is_empty() {
        response.on_hover_text(current)
    } else {
        response
    };
    let popup_id = egui::Popup::default_response_id(&response);
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let fill = if !enabled {
        primary(0.05)
    } else if open || response.is_pointer_button_down_on() {
        primary(0.2)
    } else if response.hovered() {
        primary(0.14)
    } else {
        primary(0.1)
    };
    ui.painter().rect_filled(rect, 6.0, fill);
    ui.painter().rect_stroke(
        rect,
        6.0,
        Stroke::new(1.0, primary(0.06)),
        StrokeKind::Inside,
    );
    ui.painter().galley(
        pos2(rect.left() + 10.0, rect.center().y - text.size().y / 2.0),
        text,
        color,
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            pos2(rect.right() - 12.0, rect.center().y),
            Vec2::splat(10.0),
        ),
        Icon::ChevronUpDown,
        if enabled {
            theme::text_secondary()
        } else {
            theme::text_tertiary()
        },
    );
    // The chosen row is scrolled into view once per opening, not every frame (that would fight
    // the user's own scrolling).
    let scrolled = popup_id.with("scrolled-to-choice");
    if !open {
        ui.data_mut(|data| data.remove::<bool>(scrolled));
    }
    // Measuring every label is for the open menu; a long list is not laid out every frame.
    let menu_width = if open {
        menu_width_for(ui, labels, width)
    } else {
        width
    };
    let mut chosen = None;
    egui::Popup::menu(&response)
        .frame(menu_frame())
        .width(menu_width)
        .show(|ui| {
            ui.set_min_width(menu_width - MENU_FRAME_MARGIN);
            egui::ScrollArea::vertical()
                .max_height(MENU_MAX_HEIGHT)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (index, label) in labels.iter().enumerate() {
                        let (clicked, row) =
                            menu_item_marked_response(ui, Icon::Check, label, index == selected);
                        if index == selected
                            && !ui.data(|data| data.get_temp::<bool>(scrolled).unwrap_or(false))
                        {
                            row.scroll_to_me(Some(egui::Align::Center));
                            ui.data_mut(|data| data.insert_temp(scrolled, true));
                        }
                        if clicked {
                            chosen = Some(index);
                            ui.close();
                        }
                    }
                });
        });
    chosen
}
