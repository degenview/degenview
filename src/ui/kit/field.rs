//! Text fields: the search field, the wells inputs sit in and a frameless field.

use super::{REGULAR, primary, size, text};
use crate::ui::icons::{self, Icon};
use crate::ui::theme;
use eframe::egui::{self, FontId, Id, Sense, Stroke, StrokeKind, Ui, text::LayoutJob, vec2};

/// `PortfolioSearchField`: a compact rounded field with a magnifier and a clear button.
pub fn search_field(
    ui: &mut Ui,
    id: Id,
    query: &mut String,
    prompt: &str,
    width: f32,
) -> egui::Response {
    let focused = ui.memory(|memory| memory.has_focus(id));
    let (rect, _) = ui.allocate_exact_size(vec2(width, 26.0), Sense::hover());
    ui.painter().rect_filled(rect, 7.0, theme::quaternary(0.5));
    ui.painter().rect_stroke(
        rect,
        7.0,
        Stroke::new(
            1.0,
            if focused {
                theme::with_alpha(theme::text_secondary(), 0.7)
            } else {
                primary(0.1)
            },
        ),
        StrokeKind::Inside,
    );
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(8.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    inner.spacing_mut().item_spacing.x = 6.0;
    icons::inline(&mut inner, Icon::Magnifier, 12.0, theme::text_secondary());
    let clear = if query.is_empty() { 0.0 } else { 22.0 };
    let response = inner.add(
        egui::TextEdit::singleline(query)
            .id(id)
            .frame(egui::Frame::NONE)
            .font(FontId::proportional(size::BODY))
            .text_color(theme::text_primary())
            .hint_text(text(prompt, size::BODY, REGULAR, theme::text_tertiary()))
            .desired_width((inner.available_width() - clear).max(40.0)),
    );
    if !query.is_empty()
        && icons::button(
            &mut inner,
            Icon::XmarkCircleFill,
            18.0,
            theme::text_secondary(),
            "Clear search",
        )
        .clicked()
    {
        query.clear();
        response.request_focus();
    }
    response
}

/// The height of a text field in a Settings page and in a sheet: one size, so a form never
/// mixes tall and short inputs. The frame's hairline is outside it.
pub const FIELD_HEIGHT: f32 = 36.0;

/// The well of a text field of [`FIELD_HEIGHT`]: fill the field's own content to that height
/// (`ui.set_height(FIELD_HEIGHT)`) with the content centred. Settings pages and the webhook sheet
/// use it; `focused` lightens the hairline.
pub fn input_frame(focused: bool) -> egui::Frame {
    egui::Frame::new()
        .fill(theme::with_alpha(theme::editor_background(), 0.7))
        .corner_radius(8.0)
        .stroke(Stroke::new(
            1.0,
            if focused {
                theme::with_alpha(theme::text_secondary(), 0.7)
            } else {
                primary(0.15)
            },
        ))
        .inner_margin(egui::Margin::symmetric(10, 0))
}

/// The rounded well a form input sits in; `focused` rings it in the accent colour.
pub fn field_frame(focused: bool) -> egui::Frame {
    egui::Frame::new()
        .fill(theme::quaternary(0.45))
        .corner_radius(10.0)
        .stroke(Stroke::new(
            1.0,
            if focused {
                theme::with_alpha(theme::text_secondary(), 0.7)
            } else {
                primary(0.12)
            },
        ))
        .inner_margin(egui::Margin::symmetric(12, 9))
}

/// A plain single-line text field at `size` and `weight`, drawn without a frame of its own.
pub fn plain_field(
    ui: &mut Ui,
    id: Id,
    text_value: &mut String,
    hint: &str,
    font_size: f32,
    weight: f32,
    width: f32,
) -> egui::Response {
    let mut layouter = |ui: &Ui, buffer: &dyn egui::TextBuffer, wrap_width: f32| {
        let mut job = LayoutJob::default();
        job.append(
            buffer.as_str(),
            0.0,
            egui::TextFormat {
                font_id: FontId::proportional(font_size),
                color: theme::text_primary(),
                coords: egui::epaint::text::VariationCoords::new([("wght", weight)]),
                ..Default::default()
            },
        );
        job.wrap.max_width = wrap_width;
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    ui.add(
        egui::TextEdit::singleline(text_value)
            .id(id)
            .frame(egui::Frame::NONE)
            .font(FontId::proportional(font_size))
            .hint_text(text(hint, font_size, REGULAR, theme::text_tertiary()))
            .desired_width(width)
            .layouter(&mut layouter),
    )
}
