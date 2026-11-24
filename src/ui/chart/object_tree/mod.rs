//! The Object Tree: the chart's symbol, indicators, scripts and drawings as a list you can
//! select, show or hide, lock, reorder and delete from.
//!
//! A docked panel over `model::chart::object_tree`'s rows: a toolbar, a filter field, and collapsible
//! sections whose rows carry the glyph of what they are. It owns nothing: selecting a drawing
//! here is selecting it on the chart (the workspace's one selection), and every change is
//! returned as a [`TreeAction`] for the workspace to apply through the same paths the chart's own
//! controls use. Only the rows on screen are laid out.

use crate::i18n::{tr, tr_n};
use crate::model::BuiltIn;
use crate::model::chart::object_tree::{
    IndicatorKey, Key, Row, Section, draggable, drop_over, siblings,
};
use crate::ui::icons::{self, Icon};
use crate::ui::workspace::tool_sidebar::tool_icon;
use crate::ui::{accessibility, kit, theme};
use eframe::egui::{self, Align, Layout, Pos2, Rect, Sense, Ui, pos2, vec2};
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
    /// The row being carried to a new place in its stack.
    pub dragging: Option<Key>,
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
    /// A row was dropped into another's place: `key` takes the place of `over`, both of the same
    /// kind (a drawing, a compared market or a script).
    Drop {
        key: Key,
        over: Key,
    },
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
    kit::divider(ui);
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
    // Rows touch: the area sizes its content from the spacing it is shown with, so it must not
    // add a gap per row that is never drawn (it made the list scroll with nothing to scroll to,
    // and put the rows' places off the rows' own height).
    let area = ui
        .scope(|ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            egui::ScrollArea::vertical()
                .id_salt("object-tree-rows")
                .auto_shrink([false, true])
                .max_height(list_height)
                .show_rows(ui, ROW_HEIGHT, list.len(), |ui, range| {
                    if state.dragging.is_some() {
                        scroll_towards_pointer(ui);
                    }
                    for row in &list[range] {
                        if let Key::Heading(section) = row.key {
                            heading(ui, row, section, state, filtering);
                        } else {
                            row_ui(ui, row, selected, state, filtering, &mut actions);
                        }
                    }
                })
        })
        .inner;
    if let Some(key) = state.dragging {
        let rows_area = RowsArea {
            rect: area.inner_rect,
            scrolled: area.state.offset.y,
        };
        if !carry(ui, &rows_area, rows, &list, key, &mut actions) {
            state.dragging = None;
        }
    }
    if only_the_symbol(rows) && !filtering {
        hint(ui);
    }
    actions
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
                tr!("Object tree"),
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
                    tr!("Close object tree"),
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
        .hint_text(tr!("Filter objects"))
        .font(egui::FontId::proportional(kit::size::BODY))
        .margin(egui::Margin::ZERO)
        .vertical_align(Align::Center)
        .desired_width(field.width());
    let response = ui.put(field, edit);
    accessibility::label(response, tr!("Filter objects"));
    if !state.filter.is_empty() {
        let slot =
            Rect::from_center_size(pos2(well.right() - 15.0, well.center().y), vec2(22.0, 22.0));
        let clear = ui.interact(slot, id.with("clear"), Sense::click());
        let clear =
            accessibility::label(clear, tr!("Clear filter")).on_hover_text(tr!("Clear filter"));
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
            tr!("No matches"),
            kit::size::HEADLINE,
            kit::SEMIBOLD,
            theme::text_secondary(),
        ));
        ui.add_space(2.0);
        ui.label(kit::text(
            tr!("Nothing on this chart matches the filter."),
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_tertiary(),
        ));
        ui.add_space(10.0);
        if kit::push_button(ui, None, tr!("Clear filter"), true).clicked() {
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
            tr!("Nothing else here yet"),
            kit::size::BODY,
            kit::SEMIBOLD,
            theme::text_secondary(),
        ));
        ui.add_space(2.0);
        ui.add(
            egui::Label::new(kit::text(
                tr!("Draw with the tools on the left or add an indicator, and it is listed here."),
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
    let description = if folded {
        tr_n!(
            row.count,
            "{label}, {count} item, collapsed",
            "{label}, {count} items, collapsed",
            label = row.label
        )
    } else {
        tr_n!(
            row.count,
            "{label}, {count} item, expanded",
            "{label}, {count} items, expanded",
            label = row.label
        )
    };
    let response = accessibility::label(response, description);
    if !filtering && response.clicked() {
        if folded {
            state.collapsed.remove(&section);
        } else {
            state.collapsed.insert(section);
        }
    }
}

// MARK: Reordering

/// Where the rows are on screen: their viewport and how far the list is scrolled. The one place
/// that knows a row's place: the list is shown with no spacing, so `show_rows` lays row `n` at
/// `n * ROW_HEIGHT`, and both the drop and its line are measured here.
struct RowsArea {
    rect: Rect,
    scrolled: f32,
}

impl RowsArea {
    /// Where the top of list row `index` is on screen; `index == len` is the bottom of the last.
    fn row_top(&self, index: usize) -> f32 {
        self.rect.top() - self.scrolled + index as f32 * ROW_HEIGHT
    }

    /// The row boundary nearest `pointer`, as an index into the list (0 is above the first row),
    /// when the pointer is over the list: inside its viewport, or just beside it.
    fn line_at(&self, pointer: Pos2) -> Option<usize> {
        let over = pointer.x >= self.rect.left() - DROP_REACH
            && pointer.x <= self.rect.right() + DROP_REACH
            && pointer.y >= self.rect.top()
            && pointer.y <= self.rect.bottom();
        over.then(|| ((pointer.y - self.rect.top() + self.scrolled) / ROW_HEIGHT).round() as usize)
    }
}

/// How near the viewport's top or bottom a carried row scrolls the list, and how fast (points a
/// second).
const SCROLL_EDGE: f32 = 28.0;
const SCROLL_SPEED: f32 = 480.0;
/// How far beside the list a drop still counts; further out it is let go of, which cancels.
const DROP_REACH: f32 = 12.0;
/// How long, in seconds, a finger rests on a row to pick it up (a finger that moves is scrolling).
const LONG_PRESS: f64 = 0.4;

/// Where the pointer is for a carried row. A finger that has just lifted has no pointer position,
/// but still has the place it lifted from.
fn pointer_of(ui: &Ui) -> Option<Pos2> {
    ui.input(|input| input.pointer.interact_pos())
}

/// Picks the row `key` up under a press that is on it. A mouse takes it as soon as it moves; a
/// finger moving over a row is scrolling the list, so it scrolls and only a finger that rests on the
/// row picks it up.
fn grab(ui: &Ui, response: &egui::Response, key: Key, state: &mut TreeState) {
    if state.dragging.is_some() {
        return;
    }
    if !ui.input(|input| input.any_touches()) {
        if response.drag_started_by(egui::PointerButton::Primary) {
            state.dragging = Some(key);
        }
        return;
    }
    if response.dragged() {
        // Under the finger, not eased towards it.
        ui.scroll_with_delta_animation(response.drag_delta(), egui::style::ScrollAnimation::none());
    } else if response.is_pointer_button_down_on() {
        let held = ui.input(|input| {
            input
                .pointer
                .press_start_time()
                .map_or(0.0, |start| input.time - start)
        });
        if held >= LONG_PRESS {
            state.dragging = Some(key);
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(LONG_PRESS - held));
        }
    }
}

/// Scrolls the list while a carried row is held near its top or bottom edge.
fn scroll_towards_pointer(ui: &mut Ui) {
    let Some(pointer) = pointer_of(ui) else {
        return;
    };
    let view = ui.clip_rect();
    let step = ui.input(|input| input.stable_dt) * SCROLL_SPEED;
    let delta = if pointer.y < view.top() + SCROLL_EDGE {
        step
    } else if pointer.y > view.bottom() - SCROLL_EDGE {
        -step
    } else {
        return;
    };
    ui.scroll_with_delta(vec2(0.0, delta));
    ui.ctx().request_repaint();
}

/// Follows the carried row `key` while the button is down, with a line where it would land, and
/// reports the drop when it is let go over the list. Returns whether the drag goes on.
fn carry(
    ui: &Ui,
    area: &RowsArea,
    rows: &[Row],
    list: &[&Row],
    key: Key,
    actions: &mut Vec<TreeAction>,
) -> bool {
    let stack = siblings(rows, key);
    let Some(first) = stack
        .first()
        .and_then(|top| list.iter().position(|row| row.key == *top))
    else {
        // The row went away, or was folded away, under the pointer.
        return false;
    };
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    // The line nearest the pointer among the stack's lines (0 above its first row): over another
    // section of the list it is the stack's end, over anything outside the list there is none.
    let gap = pointer_of(ui)
        .and_then(|pointer| area.line_at(pointer))
        .map(|line| line.saturating_sub(first).min(stack.len()));
    let landing = gap.and_then(|gap| drop_over(&stack, key, gap).map(|over| (gap, over)));
    if let Some((gap, _)) = landing {
        let y = area.row_top(first + gap);
        ui.painter_at(area.rect).line_segment(
            [
                pos2(area.rect.left() + INSET, y),
                pos2(area.rect.right() - INSET, y),
            ],
            egui::Stroke::new(2.0, theme::text_primary()),
        );
    }
    if ui.input(|input| input.pointer.primary_down()) {
        return true;
    }
    if let Some((_, over)) = landing {
        actions.push(TreeAction::Drop { key, over });
    }
    false
}

/// The buttons a row can have, rightmost first.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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
            BuiltIn::Ema | BuiltIn::Bollinger | BuiltIn::TrendFlips => Icon::ChartUptrend,
        },
        Key::Indicator(IndicatorKey::Script(_)) => Icon::AngleBrackets,
        Key::Overlay(_) => Icon::Compare,
        Key::Drawing(_) => row.tool.map_or(Icon::Pencil, tool_icon),
    }
}

fn row_ui(
    ui: &mut Ui,
    row: &Row,
    selected: &[u64],
    state: &mut TreeState,
    filtering: bool,
    actions: &mut Vec<TreeAction>,
) {
    let width = ui.available_width();
    let is_selected = matches!(row.key, Key::Drawing(id) if selected.contains(&id));
    // Rows move within their own stack, and not in a filtered list, where a row's neighbours are
    // not the ones it would be placed among. Only a row that can move senses a drag: on any other
    // a finger's swipe goes on to scroll the list.
    let movable = draggable(row) && !filtering;
    let sense = if movable {
        Sense::click_and_drag()
    } else {
        Sense::click()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), sense);
    #[cfg(test)]
    probe::record(row.key, rect);
    if movable {
        grab(ui, &response, row.key, state);
    }
    let carried = state.dragging == Some(row.key);
    // Over the row by position, not by the row's own response: a button on top of the row takes
    // the pointer from it, and a row that stopped counting as hovered would drop its buttons the
    // moment the pointer reached one. While a row is carried none lights up or offers buttons.
    let over = state.dragging.is_none() && ui.rect_contains_pointer(rect);
    let dimmed = row.shown == Some(false) || carried;
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
                tr!("Unlock to delete"),
                theme::text_tertiary(),
                false,
            ),
            Btn::Delete => (Icon::Trash, tr!("Delete"), theme::text_secondary(), true),
            Btn::Lock if row.locked == Some(true) => {
                (Icon::LockFill, tr!("Unlock"), kit::ACCENT, true)
            }
            Btn::Lock => (Icon::LockOpen, tr!("Lock"), theme::text_secondary(), true),
            Btn::Show if row.shown == Some(false) => {
                (Icon::EyeSlash, tr!("Show"), theme::text_secondary(), true)
            }
            Btn::Show => (Icon::Eye, tr!("Hide"), theme::text_secondary(), true),
        };
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let button = ui.interact(slot, response.id.with(*kind), sense);
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
    // A row held down to be carried is not asking for its menu, which a long touch would open.
    if !carried {
        row_menu(&response, row, actions);
    }
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
                    tr!("Settings…"),
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
                    (tr!("Bring forward"), 1),
                    (tr!("Send backward"), -1),
                    (tr!("Bring to front"), i32::MAX / 2),
                    (tr!("Send to back"), i32::MIN / 2),
                ] {
                    if kit::menu_item(ui, Icon::Layers, label, readable(row), false) {
                        actions.push(TreeAction::Reorder(id, step));
                        ui.close();
                    }
                }
            }
            if !matches!(row.key, Key::Symbol) && row.locked != Some(true) {
                kit::menu_separator(ui);
                if kit::menu_item(ui, Icon::Trash, tr!("Delete"), true, true) {
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
        Section::Symbol => tr!("SYMBOL"),
        Section::Overlays => tr!("COMPARED SYMBOLS"),
        Section::Indicators => tr!("INDICATORS"),
        Section::Drawings => tr!("DRAWINGS"),
    }
}

fn accessible_name(row: &Row) -> String {
    let kind = match row.key {
        Key::Symbol => tr!("Symbol"),
        Key::Overlay(_) => tr!("Compared symbol"),
        Key::Indicator(IndicatorKey::BuiltIn(_)) => tr!("Indicator"),
        Key::Indicator(IndicatorKey::Script(_)) => tr!("Script"),
        Key::Drawing(_) => tr!("Drawing"),
        Key::Heading(_) => "",
    };
    let name = tr!("{kind} {label}", kind = kind, label = row.label);
    match (row.shown == Some(false), row.locked == Some(true)) {
        (false, false) => name,
        (true, false) => tr!("{name}, hidden", name = name),
        (false, true) => tr!("{name}, locked", name = name),
        (true, true) => tr!("{name}, hidden, locked", name = name),
    }
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
