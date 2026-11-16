//! The rounded search box of the pickers (the symbol picker, the portfolio's asset picker and the
//! timeframe picker): a leading icon, the query, a busy indicator and a clear button.

use crate::ui::icons::{self, Icon};
use crate::ui::kit::loading::LoadingIndicator;
use crate::ui::kit::{size, text};
use crate::ui::theme;
use eframe::egui::{self, FontId, Id, Sense, Stroke, StrokeKind, Ui, vec2};

/// How tall the box is, and the buttons that sit beside it in a picker's header.
pub const HEIGHT: f32 = 36.0;

/// What happened in a [`search_box`] this frame.
#[derive(Default)]
pub struct SearchBoxEvent {
    pub changed: bool,
    pub submitted: bool,
    pub cleared: bool,
}

/// The box across the width left to `ui`. `focus` asks for the keyboard on every frame it is set.
pub fn search_box(
    ui: &mut Ui,
    icon: Icon,
    query: &mut String,
    searching: bool,
    placeholder: &str,
    id: Id,
    focus: bool,
) -> SearchBoxEvent {
    let focused = ui.memory(|memory| memory.has_focus(id));
    let mut event = SearchBoxEvent::default();
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, HEIGHT), Sense::hover());
    ui.painter().rect_filled(rect, 9.0, theme::quaternary(0.5));
    ui.painter().rect_stroke(
        rect,
        9.0,
        Stroke::new(
            1.0,
            if focused {
                theme::with_alpha(theme::text_secondary(), 0.7)
            } else {
                theme::ink(31)
            },
        ),
        StrokeKind::Inside,
    );
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(12.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    inner.spacing_mut().item_spacing.x = 8.0;
    icons::inline(
        &mut inner,
        icon,
        14.0,
        if focused {
            theme::text_primary()
        } else {
            theme::text_secondary()
        },
    );
    let trailing = if query.is_empty() { 0.0 } else { 24.0 } + if searching { 24.0 } else { 0.0 };
    let edit = egui::TextEdit::singleline(query)
        .id(id)
        .frame(egui::Frame::NONE)
        .font(FontId::proportional(size::BODY))
        .text_color(theme::text_primary())
        .hint_text(text(placeholder, size::BODY, 400.0, theme::text_tertiary()))
        .desired_width((inner.available_width() - trailing).max(60.0));
    let response = inner.add(edit);
    if focus {
        response.request_focus();
    }
    event.changed = response.changed();
    if response.lost_focus() && inner.input(|input| input.key_pressed(egui::Key::Enter)) {
        event.submitted = true;
        response.request_focus();
    }
    if searching {
        inner.add(LoadingIndicator::new().size(13.0));
    }
    if !query.is_empty()
        && icons::button(
            &mut inner,
            Icon::XmarkCircleFill,
            22.0,
            theme::text_tertiary(),
            "Clear search",
        )
        .clicked()
    {
        event.cleared = true;
        response.request_focus();
    }
    event
}
