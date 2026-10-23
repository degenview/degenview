//! The Object Tree: the chart's symbol, indicators, scripts and drawings as a list you can
//! select, show or hide, lock, reorder and delete from.
//!
//! A docked panel over `model::object_tree`'s rows: a toolbar, a filter field, and collapsible
//! sections whose rows carry the glyph of what they are. It owns nothing: selecting a drawing
//! here is selecting it on the chart (the workspace's one selection), and every change is
//! returned as a [`TreeAction`] for the workspace to apply through the same paths the chart's own
//! controls use. Only the rows on screen are laid out.

use crate::model::BuiltIn;
use crate::model::object_tree::{IndicatorKey, Key, Row, Section};
use crate::ui::icons::{self, Icon};
use crate::ui::tool_sidebar::tool_icon;
use crate::ui::{accessibility, kit, theme};
use eframe::egui::{self, Align, Layout, Rect, Sense, Ui, pos2, vec2};
use std::collections::HashSet;

const TOOLBAR_HEIGHT: f32 = 44.0;
const ROW_HEIGHT: f32 = 32.0;
/// Space between the panel's edge and a row's hover or selection pill.
const INSET: f32 = 8.0;
const BUTTON: f32 = 24.0;
const ICON: f32 = 15.0;
const FIELD_HEIGHT: f32 = 30.0;
const FIELD_MARGIN: f32 = 12.0;

#[derive(Default)]
pub struct TreeState {
    pub filter: String,
    /// The sections folded away. Not kept across sessions.
    pub collapsed: HashSet<Section>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TreeAction {
    Close,
    /// A row was clicked.
    Select(Key),
    /// A row was double-clicked or "Settings…" was chosen.
    Settings(Key),
    ToggleShown(Key),
    ToggleLocked(u64),
    Delete(Key),
    /// Move a drawing in the stack: positive toward the front.
    Reorder(u64, i32),
}

// MARK: Rows to show

/// The rows to list: everything, minus the rows of folded sections. A filter lists every match
/// whatever is folded, so a search never hides its own results.
fn visible<'a>(rows: &'a [Row], collapsed: &HashSet<Section>, filtering: bool) -> Vec<&'a Row> {
    let mut section = None;
    rows.iter()
        .filter(|row| {
            if let Key::Heading(heading) = row.key {
                section = Some(heading);
                return true;
            }
            filtering || section.is_none_or(|section| !collapsed.contains(&section))
        })
        .collect()
}

/// Whether the chart holds nothing but its symbol, so the list can say how objects get here.
fn only_the_symbol(rows: &[Row]) -> bool {
    rows.iter()
        .all(|row| matches!(row.key, Key::Heading(_) | Key::Symbol))
}

// MARK: Panel

pub fn show(
    ui: &mut Ui,
    state: &mut TreeState,
    title: &str,
    rows: &[Row],
    selected: &[u64],
) -> Vec<TreeAction> {
    let mut actions = Vec::new();
    // The content fills the panel however short the list is.
    ui.set_min_height(ui.available_height());
    toolbar(ui, title, &mut actions);
    divider(ui);
    ui.add_space(10.0);
    search(ui, state);
    ui.add_space(8.0);

    let filtering = !state.filter.trim().is_empty();
    if rows.is_empty() {
        no_matches(ui, state);
        return actions;
    }
    let list = visible(rows, &state.collapsed, filtering);
    let list_height = (list.len() as f32 * ROW_HEIGHT).min(ui.available_height());
    egui::ScrollArea::vertical()
        .id_salt("object-tree-rows")
        .auto_shrink([false, true])
        .max_height(list_height)
        .show_rows(ui, ROW_HEIGHT, list.len(), |ui, range| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            for row in &list[range] {
                if let Key::Heading(section) = row.key {
                    heading(ui, row, section, state, filtering);
                } else {
                    row_ui(ui, row, selected, &mut actions);
                }
            }
        });
    if only_the_symbol(rows) && !filtering {
        hint(ui);
    }
    actions
}

fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, kit::hairline());
}

fn toolbar(ui: &mut Ui, title: &str, actions: &mut Vec<TreeAction>) {
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        vec2(width, TOOLBAR_HEIGHT),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
            ui.add_space(16.0);
            ui.label(kit::text(
                "Object tree",
                kit::size::HEADLINE,
                kit::SEMIBOLD,
                theme::text_primary(),
            ));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                let close = icons::button(
                    ui,
                    Icon::Xmark,
                    26.0,
                    theme::text_secondary(),
                    "Close object tree",
                );
                if close.clicked() {
                    actions.push(TreeAction::Close);
                }
                ui.add_space(2.0);
                // What the tree is of, quietly, in the space between.
                let room = (ui.available_width() - 8.0).max(0.0);
                let galley = kit::galley(
                    ui,
                    title,
                    kit::size::SUBHEADLINE,
                    kit::REGULAR,
                    theme::text_tertiary(),
                    room,
                );
                let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
                ui.painter()
                    .galley(rect.min, galley, theme::text_tertiary());
            });
        },
    );
}

/// The filter field: a rounded well with a magnifier, and a clear button once there is text.
fn search(ui: &mut Ui, state: &mut TreeState) {
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), FIELD_HEIGHT), Sense::hover());
    let well = Rect::from_min_size(
        pos2(rect.left() + FIELD_MARGIN, rect.top()),
        vec2(rect.width() - FIELD_MARGIN * 2.0, FIELD_HEIGHT),
    );
    let id = ui.id().with("object-tree-filter");
    let focused = ui.memory(|memory| memory.has_focus(id));
    ui.painter().rect_filled(well, 8.0, theme::ink(10));
    ui.painter().rect_stroke(
        well,
        8.0,
        egui::Stroke::new(
            if focused { 1.5 } else { 1.0 },
            if focused { kit::ACCENT } else { theme::ink(14) },
        ),
        egui::StrokeKind::Inside,
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(well.left() + 16.0, well.center().y), vec2(13.0, 13.0)),
        Icon::Magnifier,
        theme::text_tertiary(),
    );
    let clear_width = if state.filter.is_empty() { 8.0 } else { 30.0 };
    let field = Rect::from_min_max(
        pos2(well.left() + 30.0, well.top()),
        pos2(well.right() - clear_width, well.bottom()),
    );
    let edit = egui::TextEdit::singleline(&mut state.filter)
        .id(id)
        .frame(egui::Frame::NONE)
        .hint_text("Filter objects")
        .font(egui::FontId::proportional(kit::size::BODY))
        .margin(egui::Margin::ZERO)
        .vertical_align(Align::Center)
        .desired_width(field.width());
    let response = ui.put(field, edit);
    accessibility::label(response, "Filter objects");
    if !state.filter.is_empty() {
        let slot =
            Rect::from_center_size(pos2(well.right() - 15.0, well.center().y), vec2(22.0, 22.0));
        let clear = ui.interact(slot, id.with("clear"), Sense::click());
        let clear = accessibility::label(clear, "Clear filter").on_hover_text("Clear filter");
        if clear.hovered() {
            ui.painter().rect_filled(slot, 11.0, theme::ink(18));
        }
        icons::paint(
            ui.painter(),
            Rect::from_center_size(slot.center(), vec2(9.0, 9.0)),
            Icon::Xmark,
            theme::text_secondary(),
        );
        if clear.clicked() {
            state.filter.clear();
            ui.memory_mut(|memory| memory.request_focus(id));
        }
    }
}

fn no_matches(ui: &mut Ui, state: &mut TreeState) {
    ui.add_space(28.0);
    ui.vertical_centered(|ui| {
        ui.label(kit::text(
            "No matches",
            kit::size::HEADLINE,
            kit::SEMIBOLD,
            theme::text_secondary(),
        ));
        ui.add_space(2.0);
        ui.label(kit::text(
            "Nothing on this chart matches the filter.",
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_tertiary(),
        ));
        ui.add_space(10.0);
        if kit::push_button(ui, None, "Clear filter", true).clicked() {
            state.filter.clear();
        }
    });
}

/// Under a list of nothing but the symbol: where the rest comes from.
fn hint(ui: &mut Ui) {
    ui.add_space(20.0);
    ui.vertical_centered(|ui| {
        ui.set_max_width(ui.available_width() - FIELD_MARGIN * 4.0);
        icons::inline(ui, Icon::Layers, 20.0, theme::text_tertiary());
        ui.add_space(6.0);
        ui.label(kit::text(
            "Nothing else here yet",
            kit::size::BODY,
            kit::SEMIBOLD,
            theme::text_secondary(),
        ));
        ui.add_space(2.0);
        ui.add(
            egui::Label::new(kit::text(
                "Draw with the tools on the left or add an indicator, and it is listed here.",
                kit::size::SUBHEADLINE,
                kit::REGULAR,
                theme::text_tertiary(),
            ))
            .halign(Align::Center),
        );
    });
}

// MARK: Rows

/// A section's heading: a chevron that folds it, its name and how many it holds.
fn heading(ui: &mut Ui, row: &Row, section: Section, state: &mut TreeState, filtering: bool) {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        vec2(width, ROW_HEIGHT),
        if filtering {
            Sense::hover()
        } else {
            Sense::click()
        },
    );
    #[cfg(test)]
    probe::record(row.key, rect);
    let folded = !filtering && state.collapsed.contains(&section);
    let pill = rect.shrink2(vec2(INSET, 1.0));
    if !filtering && ui.rect_contains_pointer(rect) {
        ui.painter().rect_filled(pill, 6.0, theme::ink(8));
    }
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(pill.left() + 14.0, rect.center().y), vec2(10.0, 10.0)),
        if folded {
            Icon::ChevronRight
        } else {
            Icon::ChevronDown
        },
        theme::text_tertiary(),
    );
    let title = kit::galley(
        ui,
        heading_text(section),
        kit::size::CAPTION,
        kit::SEMIBOLD,
        theme::text_secondary(),
        f32::INFINITY,
    );
    ui.painter().galley(
        pos2(pill.left() + 30.0, rect.center().y - title.size().y / 2.0),
        title,
        theme::text_secondary(),
    );
    if row.count > 0 {
        let text = kit::galley(
            ui,
            &row.count.to_string(),
            kit::size::CAPTION,
            kit::SEMIBOLD,
            theme::text_secondary(),
            f32::INFINITY,
        );
        let badge = Rect::from_center_size(
            pos2(pill.right() - 14.0, rect.center().y),
            vec2((text.size().x + 12.0).max(20.0), 18.0),
        );
        ui.painter().rect_filled(badge, 9.0, theme::ink(16));
        ui.painter().galley(
            pos2(
                badge.center().x - text.size().x / 2.0,
                badge.center().y - text.size().y / 2.0,
            ),
            text,
            theme::text_secondary(),
        );
    }
    let description = format!(
        "{}, {} item{}, {}",
        row.label,
        row.count,
        if row.count == 1 { "" } else { "s" },
        if folded { "collapsed" } else { "expanded" }
    );
    let response = accessibility::label(response, description);
    if !filtering && response.clicked() {
        if folded {
            state.collapsed.remove(&section);
        } else {
            state.collapsed.insert(section);
        }
    }
}

/// The buttons a row can have, rightmost first.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Btn {
    Delete,
    Lock,
    Show,
}

/// A row's buttons, in slot order: everything but the symbol can be deleted, only drawings lock,
/// and whatever can be hidden has an eye. A locked drawing keeps its (greyed) delete so locking
/// moves nothing.
fn row_buttons(row: &Row) -> Vec<Btn> {
    let mut buttons = Vec::new();
    if matches!(row.key, Key::Heading(_)) {
        return buttons;
    }
    if !matches!(row.key, Key::Symbol) {
        buttons.push(Btn::Delete);
    }
    if matches!(row.key, Key::Drawing(_)) && row.locked.is_some() {
        buttons.push(Btn::Lock);
    }
    if row.shown.is_some() {
        buttons.push(Btn::Show);
    }
    buttons
}

/// Whether a button shows without the pointer on the row: a lock that is on, an object hidden.
fn shows_at_rest(row: &Row, button: Btn) -> bool {
    match button {
        Btn::Delete => false,
        Btn::Lock => row.locked == Some(true),
        Btn::Show => row.shown == Some(false),
    }
}

/// Where a row's button `index` sits: 0 is the rightmost.
fn button_slot(row: Rect, index: usize) -> Rect {
    let center_x = row.right() - INSET - 6.0 - BUTTON / 2.0 - index as f32 * (BUTTON + 2.0);
    Rect::from_center_size(pos2(center_x, row.center().y), vec2(BUTTON, BUTTON))
}

/// The glyph a row leads with: what it is.
fn row_icon(row: &Row) -> Icon {
    match row.key {
        Key::Symbol | Key::Heading(_) => Icon::ChartLine,
        Key::Indicator(IndicatorKey::BuiltIn(indicator)) => match indicator {
            BuiltIn::Volume => Icon::BarChart,
            BuiltIn::Rsi => Icon::Waveform,
            BuiltIn::Ema | BuiltIn::Bollinger | BuiltIn::TrendFlips => Icon::ChartUptrend,
        },
        Key::Indicator(IndicatorKey::Script(_)) => Icon::Curlybraces,
        Key::Drawing(_) => row.tool.map_or(Icon::Pencil, tool_icon),
    }
}

fn row_ui(ui: &mut Ui, row: &Row, selected: &[u64], actions: &mut Vec<TreeAction>) {
    let width = ui.available_width();
    let is_selected = matches!(row.key, Key::Drawing(id) if selected.contains(&id));
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click());
    #[cfg(test)]
    probe::record(row.key, rect);
    // Over the row by position, not by the row's own response: a button on top of the row takes
    // the pointer from it, and a row that stopped counting as hovered would drop its buttons the
    // moment the pointer reached one.
    let over = ui.rect_contains_pointer(rect);
    let dimmed = row.shown == Some(false);
    let pill = rect.shrink2(vec2(INSET, 1.0));
    if is_selected {
        ui.painter()
            .rect_filled(pill, 6.0, kit::ACCENT.gamma_multiply(0.30));
    } else if over {
        ui.painter().rect_filled(pill, 6.0, theme::ink(12));
    }
    let color = if dimmed {
        theme::text_tertiary()
    } else {
        theme::text_primary()
    };

    // The row's buttons sit in slots numbered over the buttons it can have, so a row without a
    // lock has no gap where one would be, and nothing shifts under the pointer as buttons
    // appear: a lock or a hide stays visible, the rest show on hover.
    let buttons = row_buttons(row);
    let mut reserved = 0;
    for (index, kind) in buttons.iter().enumerate() {
        if !(over || shows_at_rest(row, *kind)) {
            continue;
        }
        reserved = index + 1;
        let slot = button_slot(rect, index);
        let (icon, tip, tint, enabled) = match *kind {
            Btn::Delete if row.locked == Some(true) => (
                Icon::Trash,
                "Unlock to delete",
                theme::text_tertiary(),
                false,
            ),
            Btn::Delete => (Icon::Trash, "Delete", theme::text_secondary(), true),
            Btn::Lock if row.locked == Some(true) => (Icon::LockFill, "Unlock", kit::ACCENT, true),
            Btn::Lock => (Icon::LockOpen, "Lock", theme::text_secondary(), true),
            Btn::Show if row.shown == Some(false) => {
                (Icon::EyeSlash, "Show", theme::text_secondary(), true)
            }
            Btn::Show => (Icon::Eye, "Hide", theme::text_secondary(), true),
        };
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let button = ui.interact(slot, response.id.with(tip), sense);
        let button = accessibility::label(button, tip).on_hover_text(tip);
        let hot = enabled && button.hovered();
        if hot {
            ui.painter().rect_filled(slot, 6.0, theme::ink(20));
        }
        icons::paint(
            ui.painter(),
            Rect::from_center_size(slot.center(), vec2(14.0, 14.0)),
            icon,
            if hot { theme::text_primary() } else { tint },
        );
        if enabled && button.clicked() {
            actions.push(match (*kind, row.key) {
                (Btn::Lock, Key::Drawing(id)) => TreeAction::ToggleLocked(id),
                (Btn::Delete, key) => TreeAction::Delete(key),
                (_, key) => TreeAction::ToggleShown(key),
            });
        }
    }
    let right = rect.right() - INSET - 6.0 - reserved as f32 * (BUTTON + 2.0);

    // The glyph, the label and the quiet detail after it.
    let icon_color = if dimmed {
        theme::text_tertiary()
    } else if is_selected {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(pill.left() + 18.0, rect.center().y), vec2(ICON, ICON)),
        row_icon(row),
        icon_color,
    );
    let text_left = pill.left() + 36.0;
    let galley = kit::galley(
        ui,
        &row.label,
        kit::size::BODY,
        kit::REGULAR,
        color,
        (right - text_left - 6.0).max(40.0),
    );
    ui.painter().galley(
        pos2(text_left, rect.center().y - galley.size().y / 2.0),
        galley.clone(),
        color,
    );
    if !row.detail.is_empty() {
        let detail_left = text_left + galley.size().x + 8.0;
        let room = right - detail_left - 6.0;
        if room > 30.0 {
            let detail = kit::galley(
                ui,
                &row.detail,
                kit::size::SUBHEADLINE,
                kit::REGULAR,
                theme::text_tertiary(),
                room,
            );
            ui.painter().galley(
                pos2(detail_left, rect.center().y - detail.size().y / 2.0),
                detail,
                theme::text_tertiary(),
            );
        }
    }
    let response = accessibility::label(response, accessible_name(row));

    if response.double_clicked() {
        actions.push(TreeAction::Settings(row.key));
    } else if response.clicked() {
        actions.push(TreeAction::Select(row.key));
    }
    row_menu(&response, row, actions);
}

fn row_menu(response: &egui::Response, row: &Row, actions: &mut Vec<TreeAction>) {
    egui::Popup::context_menu(response)
        .frame(kit::menu_frame())
        .show(|ui| {
            ui.set_width(190.0);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            if !matches!(row.key, Key::Symbol)
                && kit::menu_item(
                    ui,
                    Icon::Sliders,
                    "Settings…",
                    row.locked != Some(true) && readable(row),
                    false,
                )
            {
                actions.push(TreeAction::Settings(row.key));
                ui.close();
            }
            if let Key::Drawing(id) = row.key {
                kit::menu_separator(ui);
                for (label, step) in [
                    ("Bring forward", 1),
                    ("Send backward", -1),
                    ("Bring to front", i32::MAX / 2),
                    ("Send to back", i32::MIN / 2),
                ] {
                    if kit::menu_item(ui, Icon::Layers, label, readable(row), false) {
                        actions.push(TreeAction::Reorder(id, step));
                        ui.close();
                    }
                }
            }
            if !matches!(row.key, Key::Symbol) && row.locked != Some(true) {
                kit::menu_separator(ui);
                if kit::menu_item(ui, Icon::Trash, "Delete", true, true) {
                    actions.push(TreeAction::Delete(row.key));
                    ui.close();
                }
            }
        });
}

/// Whether a row can be changed: a drawing from a newer version (it has no tool) can only be deleted.
fn readable(row: &Row) -> bool {
    !matches!(row.key, Key::Drawing(_)) || row.tool.is_some()
}

fn heading_text(section: Section) -> &'static str {
    match section {
        Section::Symbol => "SYMBOL",
        Section::Indicators => "INDICATORS",
        Section::Drawings => "DRAWINGS",
    }
}

fn accessible_name(row: &Row) -> String {
    let kind = match row.key {
        Key::Symbol => "Symbol",
        Key::Indicator(IndicatorKey::BuiltIn(_)) => "Indicator",
        Key::Indicator(IndicatorKey::Script(_)) => "Script",
        Key::Drawing(_) => "Drawing",
        Key::Heading(_) => "",
    };
    let mut name = format!("{kind} {}", row.label);
    if row.shown == Some(false) {
        name.push_str(", hidden");
    }
    if row.locked == Some(true) {
        name.push_str(", locked");
    }
    name
}

/// Where the rows were drawn last, for the tests that click them.
#[cfg(test)]
mod probe {
    use super::Key;
    use eframe::egui::Rect;
    use std::cell::RefCell;

    thread_local! {
        static ROWS: RefCell<Vec<(Key, Rect)>> = const { RefCell::new(Vec::new()) };
    }

    pub fn record(key: Key, rect: Rect) {
        ROWS.with(|rows| {
            let mut rows = rows.borrow_mut();
            rows.retain(|(existing, _)| *existing != key);
            rows.push((key, rect));
        });
    }

    pub fn rect_of(key: Key) -> Option<Rect> {
        ROWS.with(|rows| {
            rows.borrow()
                .iter()
                .find(|(existing, _)| *existing == key)
                .map(|(_, rect)| *rect)
        })
    }

    pub fn clear() {
        ROWS.with(|rows| rows.borrow_mut().clear());
    }
}

#[cfg(test)]
mod tests;
