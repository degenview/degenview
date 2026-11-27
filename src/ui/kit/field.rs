//! Text fields: the labelled field of a form, the search field, the wells inputs sit in and a
//! frameless field.

use super::{MEDIUM, REGULAR, primary, size, text};
use crate::i18n::tr;
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
            tr!("Clear search"),
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

/// The identity of a [`field`]: its focus, its text edit and whether a secret is shown are kept under
/// it. It comes from `id_salt`, a fixed name for the field; never from its title, which is words in
/// the language in use (a field would lose its focus and its reveal when the language changes, and two
/// fields with the same title would share both).
pub(crate) fn field_id(id_salt: impl std::hash::Hash + std::fmt::Debug) -> Id {
    Id::new("settings-field").with(id_salt)
}

/// The title over a form control: a text field, or any other control laid out like one. The
/// rows from here on are spaced by hand, so the layout's own vertical spacing is turned off.
pub fn field_title(ui: &mut egui::Ui, title: &str) {
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.label(text(
        title,
        size::SUBHEADLINE + 1.0,
        MEDIUM,
        theme::text_primary(),
    ));
    ui.add_space(6.0);
}

/// The explanation under a form control.
pub fn field_hint(ui: &mut egui::Ui, hint: &str) {
    ui.add_space(6.0);
    ui.label(text(
        hint,
        size::CAPTION + 0.5,
        REGULAR,
        theme::text_secondary(),
    ));
}

/// A labelled text field in the app's field style. A secret field hides its text and offers an
/// eye button to reveal it, so a pasted key can be checked before saving. `id_salt` names the field
/// (see [`field_id`]).
pub fn field(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    title: &str,
    hint: Option<&str>,
    prompt: &str,
    value: &mut String,
    secret: bool,
) -> egui::Response {
    let id = field_id(id_salt);
    let revealed_id = id.with("revealed");
    let mut revealed: bool = ui.data(|data| data.get_temp(revealed_id)).unwrap_or(false);
    field_title(ui, title);
    let focused = ui.memory(|memory| memory.has_focus(id));
    let mut edit = None;
    input_frame(focused).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_height(FIELD_HEIGHT);
        ui.horizontal_centered(|ui| {
            let eye = if secret {
                22.0 + ui.spacing().item_spacing.x
            } else {
                0.0
            };
            edit = Some(
                ui.add(
                    egui::TextEdit::singleline(value)
                        .id(id)
                        .frame(egui::Frame::NONE)
                        .font(egui::FontId::proportional(size::BODY))
                        .text_color(theme::text_primary())
                        .hint_text(text(prompt, size::BODY, REGULAR, theme::text_tertiary()))
                        .password(secret && !revealed)
                        .desired_width(ui.available_width() - eye),
                ),
            );
            if secret {
                let (icon, tip) = if revealed {
                    (Icon::EyeSlash, tr!("Hide {title}", title = title))
                } else {
                    (Icon::Eye, tr!("Show {title}", title = title))
                };
                if icons::button(ui, icon, 22.0, theme::text_secondary(), &tip).clicked() {
                    revealed = !revealed;
                    ui.data_mut(|data| data.insert_temp(revealed_id, revealed));
                }
            }
        });
    });
    if let Some(hint) = hint {
        field_hint(ui, hint);
    }
    edit.expect("the field's text edit ran")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Locale, with_locale};
    use crate::ui::render_tests::Screen;

    /// One frame of two fields: `titles` are their words, `salts` their names.
    fn frame(screen: &mut Screen, locale: Locale, salts: [&'static str; 2], titles: [&str; 2]) {
        let mut values = [String::new(), String::new()];
        with_locale(locale, || {
            screen.run(Vec::new(), |ui| {
                let [first, second] = &mut values;
                field(ui, salts[0], titles[0], None, "", first, true);
                field(ui, salts[1], titles[1], None, "", second, false);
            });
        });
    }

    fn focused(screen: &Screen, salt: &'static str) -> bool {
        screen.ctx.memory(|memory| memory.has_focus(field_id(salt)))
    }

    /// A field is the same field when the language changes: it keeps its focus (and its reveal, which
    /// lives under the same id) while its title is other words.
    #[test]
    fn a_field_keeps_its_focus_when_its_title_changes_language() {
        let mut screen = Screen::new(vec2(400.0, 300.0));
        let salts = ["password", "user"];
        frame(
            &mut screen,
            Locale::ENGLISH,
            salts,
            ["Password", "User name"],
        );
        screen
            .ctx
            .memory_mut(|memory| memory.request_focus(field_id("password")));
        frame(
            &mut screen,
            Locale::ENGLISH,
            salts,
            ["Password", "User name"],
        );
        assert!(focused(&screen, "password"));
        frame(
            &mut screen,
            Locale::PSEUDO,
            salts,
            ["[Þåššŵöŕð ~~]", "[Ûšéŕ ñåmé ~~]"],
        );
        frame(
            &mut screen,
            Locale::PSEUDO,
            salts,
            ["[Þåššŵöŕð ~~]", "[Ûšéŕ ñåmé ~~]"],
        );
        assert!(
            focused(&screen, "password"),
            "focus survives the language change"
        );
        assert!(!focused(&screen, "user"));
        // The same title does not share the field: the title is not its identity.
        frame(&mut screen, Locale::ENGLISH, salts, ["Name", "Name"]);
        frame(&mut screen, Locale::ENGLISH, salts, ["Name", "Name"]);
        assert!(focused(&screen, "password") && !focused(&screen, "user"));
    }
}
