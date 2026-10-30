//! The editor's font picker: a button that opens a searchable list of the installed monospaced
//! fonts, the system's own at the top. Used by Settings ▸ Script Manager, for the editor's font and
//! for a theme's.
//!
//! Names are drawn in the app's font: loading every installed font to draw its name in its own
//! face would load hundreds of files for one list, so the font a person picks is shown in the live
//! preview instead. A font that is chosen but not installed here stays in the list, marked, so it
//! is never silently dropped.

use crate::host::editor_theme_host::FontStatus;
use crate::model::editor_theme::{BUNDLED_FONT, SYSTEM_FONT};
use crate::ui::icons::{self, Icon};
use crate::ui::{kit, theme};
use eframe::egui::{self, Id, Key, Modifiers, Sense, Stroke, StrokeKind, Ui, vec2};

/// The label of the system's monospace font.
pub const SYSTEM_LABEL: &str = SYSTEM_FONT;

const ROW_HEIGHT: f32 = 24.0;
const LIST_HEIGHT: f32 = 264.0;
/// The narrowest and widest the list that opens is: it is as wide as its longest name needs.
const MIN_WIDTH: f32 = 190.0;
const MAX_WIDTH: f32 = 340.0;
/// What a row adds to its text: the menu's margins, the row's insets and the check at its end.
const ROW_EXTRA: f32 = 62.0;
/// The button that opens it: what the Font row reserves room for.
pub const BUTTON_WIDTH: f32 = 190.0;

/// How wide the list is: the longest name in it, in the menu's type, with what a row adds, between
/// [`MIN_WIDTH`] and [`MAX_WIDTH`]. Only the longest name (by characters) is measured.
fn list_width(ui: &Ui, installed: &[String], saved: Option<&str>) -> f32 {
    let missing = saved
        .filter(|name| !installed.iter().any(|f| f.eq_ignore_ascii_case(name)))
        .map(|name| format!("{name} (not installed)"));
    let longest = installed
        .iter()
        .map(|name| {
            if name.eq_ignore_ascii_case(BUNDLED_FONT) {
                format!("{name} (default)")
            } else {
                name.clone()
            }
        })
        .chain(missing)
        .max_by_key(|name| name.chars().count())
        .unwrap_or_else(|| SYSTEM_LABEL.to_owned());
    let measured = kit::galley(
        ui,
        &longest,
        kit::size::BODY,
        kit::REGULAR,
        theme::text_primary(),
        f32::INFINITY,
    )
    .size()
    .x;
    (measured + ROW_EXTRA).clamp(MIN_WIDTH, MAX_WIDTH)
}

#[derive(Default)]
pub struct FontPickerState {
    /// The width of the list, found when it opens.
    width: f32,
    search: String,
    highlighted: usize,
    was_open: bool,
    scroll: f32,
    scroll_to_highlight: bool,
}

/// What the list offers for `search`: the system's font (while nothing, or part of its name, is typed),
/// then the families that contain it (the built-in one first, then the installed), then the chosen
/// family if it is not installed.
#[derive(Debug, Eq, PartialEq)]
pub enum Choice<'a> {
    /// The system's monospace font.
    System,
    Family(&'a str),
    /// The saved family, which this Mac does not have.
    Missing(&'a str),
}

pub fn choices<'a>(
    search: &str,
    installed: &'a [String],
    saved: Option<&'a str>,
    include_system: bool,
) -> Vec<Choice<'a>> {
    let needle = search.trim().to_lowercase();
    let mut choices = Vec::new();
    if include_system && (needle.is_empty() || SYSTEM_LABEL.to_lowercase().contains(&needle)) {
        choices.push(Choice::System);
    }
    choices.extend(
        installed
            .iter()
            .filter(|family| needle.is_empty() || family.to_lowercase().contains(&needle))
            .map(|family| Choice::Family(family)),
    );
    if let Some(saved) = saved
        && !installed
            .iter()
            .any(|family| family.eq_ignore_ascii_case(saved))
        && (needle.is_empty() || saved.to_lowercase().contains(&needle))
    {
        choices.push(Choice::Missing(saved));
    }
    choices
}

/// Moves a highlight by `delta` within `len` rows, stopping at the ends.
pub fn step(highlighted: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    highlighted.saturating_add_signed(delta).min(len - 1)
}

/// The font picker. `current` is the family in use (`None`: the system's); `include_system` is
/// whether "System Mono" is offered. Returns the new choice (`Some(None)` for the system font) on
/// the frame it is made.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    state: &mut FontPickerState,
    salt: &str,
    current: Option<&str>,
    status: &FontStatus,
    installed: &[String],
    fonts_ready: bool,
    include_system: bool,
) -> Option<Option<String>> {
    let label = current.unwrap_or(SYSTEM_LABEL);
    let (rect, response) = ui.allocate_exact_size(vec2(BUTTON_WIDTH, 24.0), Sense::click());
    let response = crate::ui::accessibility::label(response, format!("Font: {label}"))
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter();
    painter.rect_filled(rect, 6.0, theme::control_fill());
    painter.rect_stroke(
        rect,
        6.0,
        Stroke::new(
            1.0,
            if response.has_focus() {
                kit::ACCENT
            } else {
                theme::separator()
            },
        ),
        StrokeKind::Inside,
    );
    let text = kit::galley(
        ui,
        label,
        kit::size::BODY,
        kit::REGULAR,
        theme::text_primary(),
        rect.width() - 34.0,
    );
    ui.painter().galley(
        egui::pos2(rect.left() + 9.0, rect.center().y - text.size().y / 2.0),
        text,
        egui::Color32::PLACEHOLDER,
    );
    icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(
            egui::pos2(rect.right() - 12.0, rect.center().y),
            vec2(9.0, 12.0),
        ),
        Icon::ChevronUpDown,
        theme::text_secondary(),
    );

    let search_id = Id::new(("font-search", salt));
    let saved = match status {
        FontStatus::Missing(name) | FontStatus::Failed(name, _) | FontStatus::Loading(name) => {
            Some(name.as_str())
        }
        _ => current,
    };
    let popup = egui::Popup::from_toggle_button_response(&response)
        .frame(kit::menu_frame().inner_margin(egui::Margin::same(8)))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let opened = !state.was_open;
            if opened {
                state.width = list_width(ui, installed, saved);
            }
            let width = state.width;
            kit::menu_scope(ui, width);
            ui.set_width(width);
            state.was_open = true;
            let rows = choices(&state.search, installed, saved, include_system);
            // Arrows and Return work while the search field has the focus.
            let (down, up, accept) = ui.input_mut(|input| {
                (
                    input.consume_key(Modifiers::NONE, Key::ArrowDown),
                    input.consume_key(Modifiers::NONE, Key::ArrowUp),
                    input.consume_key(Modifiers::NONE, Key::Enter),
                )
            });
            if down || up {
                state.highlighted = step(state.highlighted, if down { 1 } else { -1 }, rows.len());
                state.scroll_to_highlight = true;
            }
            let before = state.search.clone();
            let field = kit::search_field(
                ui,
                search_id,
                &mut state.search,
                "Search fonts",
                width - 16.0,
            );
            if opened {
                field.request_focus();
            }
            if state.search != before {
                state.highlighted = 0;
                state.scroll = 0.0;
            }
            ui.add_space(6.0);
            if !fonts_ready {
                ui.label(kit::text(
                    "Looking for installed fonts…",
                    kit::size::CAPTION + 0.5,
                    kit::REGULAR,
                    theme::text_secondary(),
                ));
            } else if rows.is_empty() {
                ui.label(kit::text(
                    "No installed monospaced font matches.",
                    kit::size::CAPTION + 0.5,
                    kit::REGULAR,
                    theme::text_secondary(),
                ));
            }
            state.highlighted = state.highlighted.min(rows.len().saturating_sub(1));
            let mut scroll = egui::ScrollArea::vertical()
                .max_height(LIST_HEIGHT)
                .min_scrolled_height(0.0)
                .auto_shrink([false, true]);
            if std::mem::take(&mut state.scroll_to_highlight) {
                let top = state.highlighted as f32 * ROW_HEIGHT;
                let offset = if top < state.scroll {
                    top
                } else if top + ROW_HEIGHT > state.scroll + LIST_HEIGHT {
                    top + ROW_HEIGHT - LIST_HEIGHT
                } else {
                    state.scroll
                };
                scroll = scroll.vertical_scroll_offset(offset);
            }
            let mut chosen = None;
            let output = scroll.show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for index in range {
                    let (label, is_current, enabled) = match &rows[index] {
                        Choice::System => (SYSTEM_LABEL.to_owned(), current.is_none(), true),
                        Choice::Family(family) => (
                            if family.eq_ignore_ascii_case(BUNDLED_FONT) {
                                format!("{family} (default)")
                            } else {
                                (*family).to_owned()
                            },
                            current.is_some_and(|c| c.eq_ignore_ascii_case(family)),
                            true,
                        ),
                        Choice::Missing(name) => (format!("{name} (not installed)"), true, false),
                    };
                    let row = kit::menu_row(
                        ui,
                        kit::MenuRow {
                            current: is_current,
                            enabled,
                            focused: index == state.highlighted,
                            reserve_icon: false,
                            ..kit::MenuRow::new(&label)
                        },
                    );
                    if enabled && row.clicked() {
                        chosen = Some(index);
                    }
                    if enabled
                        && row.hovered()
                        && ui.input(|input| input.pointer.delta() != egui::Vec2::ZERO)
                    {
                        state.highlighted = index;
                    }
                }
            });
            state.scroll = output.state.offset.y;
            if accept
                && chosen.is_none()
                && rows
                    .get(state.highlighted)
                    .is_some_and(|row| !matches!(row, Choice::Missing(_)))
            {
                chosen = Some(state.highlighted);
            }
            let choice = match chosen.map(|index| &rows[index]) {
                Some(Choice::System) => Some(None),
                Some(Choice::Family(family)) => Some(Some((*family).to_owned())),
                _ => None,
            };
            if choice.is_some() {
                ui.close();
            }
            choice
        });
    match popup {
        None => {
            state.was_open = false;
            state.search.clear();
            state.highlighted = 0;
            None
        }
        Some(inner) => inner.inner,
    }
}

#[cfg(test)]
mod tests;
