//! The rounded search box of the pickers (the symbol picker, the portfolio's asset picker, the
//! timeframe picker and the chart-type menu): a leading icon, the query, a busy indicator and a
//! clear button. Full pickers use [`search_box`]; the small pull-downs use [`compact_search_box`],
//! the same box at the height of their rows.

use crate::ui::icons::{self, Icon};
use crate::ui::kit::loading::LoadingIndicator;
use crate::ui::kit::{size, text};
use crate::ui::theme;
use eframe::egui::{self, FontId, Id, Sense, Stroke, StrokeKind, Ui, vec2};

/// How tall the box is, and the buttons that sit beside it in a picker's header.
pub const HEIGHT: f32 = 36.0;

/// How tall [`compact_search_box`] is: a pull-down's field, level with its rows and chips.
pub const COMPACT_HEIGHT: f32 = 30.0;

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
    show(
        ui,
        Size::Regular,
        Content {
            icon,
            query,
            searching,
            placeholder,
            id,
            focus,
        },
    )
}

/// [`search_box`] at [`COMPACT_HEIGHT`], for the pull-downs that hold a short list.
pub fn compact_search_box(
    ui: &mut Ui,
    icon: Icon,
    query: &mut String,
    searching: bool,
    placeholder: &str,
    id: Id,
    focus: bool,
) -> SearchBoxEvent {
    show(
        ui,
        Size::Compact,
        Content {
            icon,
            query,
            searching,
            placeholder,
            id,
            focus,
        },
    )
}

#[derive(Clone, Copy)]
enum Size {
    Regular,
    Compact,
}

impl Size {
    fn height(self) -> f32 {
        match self {
            Self::Regular => HEIGHT,
            Self::Compact => COMPACT_HEIGHT,
        }
    }

    fn radius(self) -> f32 {
        self.height() / 4.0
    }

    fn inset(self) -> f32 {
        match self {
            Self::Regular => 12.0,
            Self::Compact => 10.0,
        }
    }

    fn icon(self) -> f32 {
        match self {
            Self::Regular => 14.0,
            Self::Compact => 13.0,
        }
    }
}

struct Content<'a> {
    icon: Icon,
    query: &'a mut String,
    searching: bool,
    placeholder: &'a str,
    id: Id,
    focus: bool,
}

fn show(ui: &mut Ui, box_size: Size, content: Content) -> SearchBoxEvent {
    let Content {
        icon,
        query,
        searching,
        placeholder,
        id,
        focus,
    } = content;
    let focused = ui.memory(|memory| memory.has_focus(id));
    let mut event = SearchBoxEvent::default();
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, box_size.height()), Sense::hover());
    let radius = box_size.radius();
    ui.painter()
        .rect_filled(rect, radius, theme::quaternary(0.5));
    ui.painter().rect_stroke(
        rect,
        radius,
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
            .max_rect(rect.shrink2(vec2(box_size.inset(), 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    inner.spacing_mut().item_spacing.x = 8.0;
    icons::inline(
        &mut inner,
        icon,
        box_size.icon(),
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
