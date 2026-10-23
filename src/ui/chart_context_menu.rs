//! Draws the chart's right-click menu and reports what was chosen.
//!
//! The entries come from `model::chart_context` (what to offer and whether it is allowed); this
//! file only places the menu, handles the pointer and keyboard, and hands back the [`Command`].
//! It owns no chart state: the workspace keeps the snapshot and applies the command.
//!
//! Placement and the submenu's safe triangle are pure functions below, with tests.

use crate::model::chart_context::{Command, Entry, Glyph, Hint, Item};
use crate::model::paper::PaperOrderSide;
use crate::ui::icons::Icon;
use crate::ui::{kit, shortcut};
use eframe::egui::{self, Context, Id, Key, Order, Pos2, Rect, Vec2, pos2, vec2};

/// Row and separator heights, as `kit` paints them.
const ROW: f32 = 24.0;
const SEPARATOR: f32 = 9.0;
const HEADING: f32 = 24.0;
/// The frame's inner margin on each side.
const FRAME: f32 = 5.0;
/// Space kept between a menu and the window edge.
const MARGIN: f32 = 8.0;
/// How far the menu sits from the click, so the pointer is not already over a row.
const OFFSET: f32 = 2.0;
/// A short menu (the alert's three rows) hugs its labels instead of padding out to a wide one.
const MIN_WIDTH: f32 = 148.0;
const MAX_WIDTH: f32 = 420.0;

/// What a frame of the menu decided.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// Still open.
    Open,
    /// Dismissed (Escape, a click elsewhere, nothing left to offer).
    Close,
    /// An enabled entry was chosen.
    Run(Command),
}

/// The menu's own interaction state; the entries are passed in fresh each frame.
#[derive(Default)]
pub struct MenuState {
    /// The highlighted entry of the main menu.
    focus: Option<usize>,
    /// The main-menu entry whose submenu is open, and its highlighted child.
    sub: Option<usize>,
    sub_focus: Option<usize>,
    /// The pointer a frame ago, the apex of the submenu's safe triangle.
    last_pointer: Option<Pos2>,
    /// The widest row of the menu and of the open submenu, measured on the first (invisible)
    /// frame so the menu can be placed knowing its size.
    width: Option<f32>,
    sub_width: Option<f32>,
    sub_rect: Option<Rect>,
}

// MARK: Geometry

/// The height of a column of entries, frame included.
fn height_of(entries: &[Entry]) -> f32 {
    FRAME * 2.0
        + entries
            .iter()
            .map(|entry| match entry {
                Entry::Item(_) => ROW,
                Entry::Separator => SEPARATOR,
                Entry::Heading(_) => HEADING,
            })
            .sum::<f32>()
}

/// Where a menu of `size` opens for a click at `anchor`: below and right of it, flipped to the
/// other side where that would leave the window, and always kept inside `bounds`.
pub fn menu_origin(anchor: Pos2, size: Vec2, bounds: Rect) -> Pos2 {
    let place = |at: f32, extent: f32, low: f32, high: f32| {
        let mut start = at + OFFSET;
        if start + extent > high - MARGIN {
            start = at - OFFSET - extent;
        }
        let latest = (high - MARGIN - extent).max(low + MARGIN);
        start.clamp(low + MARGIN, latest)
    };
    pos2(
        place(anchor.x, size.x, bounds.left(), bounds.right()),
        place(anchor.y, size.y, bounds.top(), bounds.bottom()),
    )
}

/// Where a submenu of `size` opens beside its parent `menu`, level with the row `row`: to the
/// right, or the left when the right has no room, and kept inside `bounds` vertically.
pub fn submenu_origin(menu: Rect, row: Rect, size: Vec2, bounds: Rect) -> Pos2 {
    let overlap = 2.0;
    let mut x = menu.right() - overlap;
    if x + size.x > bounds.right() - MARGIN {
        x = menu.left() + overlap - size.x;
    }
    let x = x.max(bounds.left() + MARGIN);
    let latest = (bounds.bottom() - MARGIN - size.y).max(bounds.top() + MARGIN);
    let y = (row.top() - FRAME).clamp(bounds.top() + MARGIN, latest);
    pos2(x, y)
}

/// Whether `point` is inside the triangle `a`, `b`, `c` (either winding).
pub fn in_triangle(point: Pos2, a: Pos2, b: Pos2, c: Pos2) -> bool {
    let side = |p: Pos2, q: Pos2, r: Pos2| (p.x - r.x) * (q.y - r.y) - (q.x - r.x) * (p.y - r.y);
    let (d1, d2, d3) = (side(point, a, b), side(point, b, c), side(point, c, a));
    let negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(negative && positive)
}

/// Whether a pointer heading from `from` toward the submenu `sub` is still on its way there:
/// inside the triangle from where it was to the submenu's near edge. While it is, hovering the
/// rows it crosses must not close the submenu.
pub fn heading_into(from: Pos2, point: Pos2, sub: Rect, sub_on_right: bool) -> bool {
    let edge = if sub_on_right {
        sub.left()
    } else {
        sub.right()
    };
    in_triangle(point, from, pos2(edge, sub.top()), pos2(edge, sub.bottom()))
}

/// The next enabled entry after `from` in the direction of `step` (wrapping), if any.
pub fn step_focus(entries: &[Entry], from: Option<usize>, step: i32) -> Option<usize> {
    let count = entries.len() as i32;
    if count == 0 {
        return None;
    }
    let mut index = match (from, step >= 0) {
        (Some(from), _) => from as i32,
        (None, true) => -1,
        (None, false) => count,
    };
    for _ in 0..count {
        index = (index + step.signum()).rem_euclid(count);
        if matches!(&entries[index as usize], Entry::Item(item) if item.is_enabled()) {
            return Some(index as usize);
        }
    }
    None
}

// MARK: Drawing

fn hint_label(hint: Hint) -> String {
    match hint {
        Hint::Paste => shortcut::PASTE.label(),
    }
}

fn icon_for(item: &Item) -> Option<Icon> {
    if let Some(glyph) = item.glyph {
        return Some(match glyph {
            Glyph::Template => Icon::Palette,
            Glyph::Layers => Icon::Layers,
            Glyph::Pause => Icon::PauseFill,
            Glyph::Play => Icon::PlayFill,
        });
    }
    Some(match item.command.as_ref()? {
        Command::CopyPrice => Icon::CopyDoc,
        Command::Paste => Icon::Clipboard,
        Command::AddAlert => Icon::Bell,
        Command::QuickOrder(order) => match order.side {
            PaperOrderSide::Sell => Icon::ArrowDown,
            PaperOrderSide::Buy => Icon::ArrowUp,
        },
        Command::AddOrder => Icon::Plus,
        Command::ToggleTimeLock => Icon::LockFill,
        Command::ToggleTable => Icon::List,
        Command::ToggleObjectTree => Icon::Layers,
        Command::ApplyTemplate(_) => Icon::Palette,
        Command::SaveTemplate => Icon::Plus,
        Command::ManageTemplates => Icon::Sliders,
        Command::ResetTemplate | Command::ResetPriceScale | Command::ResetChartView => Icon::Undo,
        Command::RemoveDrawings
        | Command::RemoveIndicators
        | Command::PaneRemove(_)
        | Command::DeleteAlert(_) => Icon::Trash,
        Command::OpenSettings | Command::PaneSettings(_) => Icon::Sliders,
        Command::EditAlert(_) => Icon::Pencil,
        Command::CancelOrder(_) | Command::ClosePosition(_) => Icon::Xmark,
        _ => return None,
    })
}

/// Whether any row of a column has an icon or a check, and so needs the column they sit in. A
/// column with none starts its labels at the edge instead of behind an empty gutter.
fn needs_icon_column(entries: &[Entry]) -> bool {
    entries.iter().any(|entry| match entry {
        Entry::Item(item) => icon_for(item).is_some() || item.checked,
        Entry::Separator | Entry::Heading(_) => false,
    })
}

/// What one column of rows reported.
#[derive(Default)]
struct Rows {
    hovered: Option<usize>,
    clicked: Option<usize>,
    rects: Vec<Rect>,
}

fn rows(ui: &mut egui::Ui, entries: &[Entry], focus: Option<usize>) -> Rows {
    let mut out = Rows::default();
    let reserve_icon = needs_icon_column(entries);
    for (index, entry) in entries.iter().enumerate() {
        match entry {
            Entry::Heading(text) => {
                let before = ui.cursor().top();
                kit::menu_heading(ui, text);
                out.rects.push(Rect::from_min_size(
                    pos2(ui.min_rect().left(), before),
                    vec2(0.0, HEADING),
                ));
            }
            Entry::Separator => {
                let before = ui.cursor().top();
                kit::menu_separator(ui);
                out.rects.push(Rect::from_min_size(
                    pos2(ui.min_rect().left(), before),
                    vec2(0.0, SEPARATOR),
                ));
            }
            Entry::Item(item) => {
                let hint = item.hint.map(hint_label);
                let mut response = kit::menu_row(
                    ui,
                    kit::MenuRow {
                        icon: icon_for(item),
                        shortcut: hint.as_deref(),
                        enabled: item.is_enabled(),
                        checked: item.checked,
                        current: item.current,
                        reserve_icon,
                        chevron: !item.children.is_empty(),
                        destructive: item.destructive,
                        focused: focus == Some(index),
                        ..kit::MenuRow::new(&item.label)
                    },
                );
                if let Some(reason) = item.disabled {
                    response = response.on_hover_text(reason);
                }
                if response.hovered() {
                    out.hovered = Some(index);
                }
                if response.clicked() {
                    out.clicked = Some(index);
                }
                out.rects.push(response.rect);
            }
        }
    }
    out
}

/// The width a row needs: the label starts `text_left` in, then comes its shortcut, then the
/// chevron or check, then the edge padding.
fn row_width(text_left: f32, label: f32, hint: f32, trailing: f32) -> f32 {
    text_left + label + hint + trailing + 14.0
}

/// The width the widest row of `entries` needs.
fn measure(ui: &egui::Ui, entries: &[Entry]) -> f32 {
    let text_left = if needs_icon_column(entries) {
        32.0
    } else {
        14.0
    };
    let widest = entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Item(item) => Some(item),
            Entry::Separator | Entry::Heading(_) => None,
        })
        .map(|item| {
            let label = kit::galley(
                ui,
                &item.label,
                kit::size::BODY,
                kit::REGULAR,
                egui::Color32::WHITE,
                f32::INFINITY,
            )
            .size()
            .x;
            let hint = item.hint.map_or(0.0, |hint| {
                kit::galley(
                    ui,
                    &hint_label(hint),
                    kit::size::BODY,
                    kit::REGULAR,
                    egui::Color32::WHITE,
                    f32::INFINITY,
                )
                .size()
                .x + 24.0
            });
            let chevron = if item.children.is_empty() && !item.current {
                0.0
            } else {
                22.0
            };
            row_width(text_left, label, hint, chevron)
        })
        .fold(0.0f32, f32::max);
    widest.clamp(MIN_WIDTH, MAX_WIDTH)
}

/// Draws one column in an area at `origin`.
fn column(
    ctx: &Context,
    id: Id,
    origin: Pos2,
    entries: &[Entry],
    focus: Option<usize>,
    width: f32,
    bounds: Rect,
) -> (Rows, Rect) {
    let max_height = (bounds.height() - MARGIN * 2.0).max(ROW * 3.0);
    let scrolls = height_of(entries) > max_height;
    let mut out = Rows::default();
    let area = egui::Area::new(id)
        .order(Order::Foreground)
        .fixed_pos(origin)
        .show(ctx, |ui| {
            kit::menu_frame().show(ui, |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                if scrolls {
                    egui::ScrollArea::vertical()
                        .max_height(max_height - FRAME * 2.0)
                        .show(ui, |ui| out = rows(ui, entries, focus));
                } else {
                    out = rows(ui, entries, focus);
                }
            });
        });
    (out, area.response.rect)
}

/// Shows the menu for the entries of a click at `anchor` and returns what happened this frame.
/// `bounds` is the window area the menu may use (the egui content rect).
pub fn show(
    ctx: &Context,
    id: Id,
    anchor: Pos2,
    entries: &[Entry],
    state: &mut MenuState,
    bounds: Rect,
) -> Outcome {
    if !entries.iter().any(|entry| matches!(entry, Entry::Item(_))) {
        return Outcome::Close;
    }
    let (escape, down, up, right, left, activate, moved, pressed, pointer) = ctx.input(|input| {
        (
            input.key_pressed(Key::Escape),
            input.key_pressed(Key::ArrowDown),
            input.key_pressed(Key::ArrowUp),
            input.key_pressed(Key::ArrowRight),
            input.key_pressed(Key::ArrowLeft),
            input.key_pressed(Key::Enter) || input.key_pressed(Key::Space),
            input.pointer.delta() != Vec2::ZERO,
            input
                .pointer
                .any_pressed()
                .then(|| input.pointer.interact_pos())
                .flatten(),
            input.pointer.hover_pos(),
        )
    });

    // The first frame only measures, invisibly, so the menu is placed knowing its size.
    let Some(width) = state.width else {
        let mut measured = MIN_WIDTH;
        egui::Area::new(id.with("measure"))
            .order(Order::Background)
            .fixed_pos(anchor)
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_invisible();
                measured = measure(ui, entries);
            });
        state.width = Some(measured);
        ctx.request_repaint();
        return Outcome::Open;
    };

    // Keyboard.
    if escape {
        if state.sub.take().is_some() {
            state.sub_focus = None;
        } else {
            return Outcome::Close;
        }
    }
    let in_sub = state.sub.is_some() && state.sub_focus.is_some();
    let level_step = i32::from(down) - i32::from(up);
    if level_step != 0 {
        if in_sub {
            if let Some(Entry::Item(parent)) = state.sub.and_then(|index| entries.get(index)) {
                state.sub_focus = step_focus(&parent.children, state.sub_focus, level_step);
            }
        } else {
            state.focus = step_focus(entries, state.focus, level_step);
        }
    }
    let focused_parent = state.focus.and_then(|index| match entries.get(index) {
        Some(Entry::Item(item)) if !item.children.is_empty() && item.is_enabled() => Some(index),
        _ => None,
    });
    if right
        && !in_sub
        && let Some(index) = focused_parent
    {
        state.sub = Some(index);
        state.sub_focus = match &entries[index] {
            Entry::Item(parent) => step_focus(&parent.children, None, 1),
            Entry::Separator | Entry::Heading(_) => None,
        };
    }
    if left && in_sub {
        state.sub = None;
        state.sub_focus = None;
    }

    let size = vec2(width + FRAME * 2.0, height_of(entries));
    let origin = menu_origin(anchor, size, bounds);
    let main_id = id.with("menu");
    let (main_rows, main_rect) = column(ctx, main_id, origin, entries, state.focus, width, bounds);

    // The pointer: focus follows it, and a submenu row opens its submenu.
    let mut chosen: Option<Command> = None;
    if let Some(index) = main_rows.hovered
        && moved
    {
        let heading = match (state.sub_rect, state.last_pointer, pointer) {
            (Some(sub), Some(from), Some(point)) if state.sub.is_some() => {
                heading_into(from, point, sub, sub.center().x > main_rect.center().x)
            }
            _ => false,
        };
        if !heading {
            state.focus = Some(index);
            let opens = matches!(&entries[index], Entry::Item(item) if !item.children.is_empty() && item.is_enabled());
            if opens {
                if state.sub != Some(index) {
                    state.sub = Some(index);
                    state.sub_focus = None;
                    state.sub_width = None;
                }
            } else if state.sub.take().is_some() {
                state.sub_focus = None;
                state.sub_rect = None;
            }
        }
    }
    if let Some(index) = main_rows.clicked
        && let Some(Entry::Item(item)) = entries.get(index)
    {
        match &item.command {
            Some(command) => chosen = Some(command.clone()),
            None => {
                state.sub = Some(index);
                state.focus = Some(index);
            }
        }
    }
    if activate
        && !in_sub
        && let Some(Entry::Item(item)) = state.focus.and_then(|i| entries.get(i))
    {
        match (&item.command, item.is_enabled()) {
            (Some(command), true) => chosen = Some(command.clone()),
            (None, true) if !item.children.is_empty() => {
                state.sub = state.focus;
                if let Some(parent) = state.focus.and_then(|i| entries.get(i))
                    && let Entry::Item(parent) = parent
                {
                    state.sub_focus = step_focus(&parent.children, None, 1);
                }
            }
            _ => {}
        }
    }

    // The submenu.
    let mut sub_area: Option<Rect> = None;
    if let Some(parent_index) = state.sub
        && let Some(Entry::Item(parent)) = entries.get(parent_index)
        && !parent.children.is_empty()
    {
        let measuring = state.sub_width.is_none();
        let sub_width = state.sub_width.unwrap_or(MIN_WIDTH);
        let sub_size = vec2(sub_width + FRAME * 2.0, height_of(&parent.children));
        let row = main_rows
            .rects
            .get(parent_index)
            .copied()
            .unwrap_or(main_rect);
        let sub_origin = submenu_origin(main_rect, row, sub_size, bounds);
        let sub_id = id.with("submenu");
        if measuring {
            let mut measured = MIN_WIDTH;
            egui::Area::new(sub_id.with("measure"))
                .order(Order::Background)
                .fixed_pos(sub_origin)
                .interactable(false)
                .show(ctx, |ui| {
                    ui.set_invisible();
                    measured = measure(ui, &parent.children);
                });
            state.sub_width = Some(measured);
            ctx.request_repaint();
        } else {
            let (sub_rows, rect) = column(
                ctx,
                sub_id,
                sub_origin,
                &parent.children,
                state.sub_focus,
                sub_width,
                bounds,
            );
            if let Some(index) = sub_rows.hovered
                && moved
            {
                state.sub_focus = Some(index);
            }
            if let Some(Entry::Item(child)) = sub_rows.clicked.and_then(|i| parent.children.get(i))
                && let Some(command) = &child.command
            {
                chosen = Some(command.clone());
            }
            if activate
                && in_sub
                && let Some(Entry::Item(child)) =
                    state.sub_focus.and_then(|i| parent.children.get(i))
                && child.is_enabled()
                && let Some(command) = &child.command
            {
                chosen = Some(command.clone());
            }
            sub_area = Some(rect);
        }
    }
    state.sub_rect = sub_area;
    state.last_pointer = pointer;

    if let Some(command) = chosen {
        return Outcome::Run(command);
    }
    let outside = pressed.is_some_and(|point| {
        !main_rect.contains(point) && !sub_area.is_some_and(|rect| rect.contains(point))
    });
    if outside {
        return Outcome::Close;
    }
    Outcome::Open
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::chart_context::{Command, Entry, Item};

    fn bounds() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 700.0))
    }

    #[test]
    fn a_menu_opens_just_below_and_right_of_the_click() {
        let origin = menu_origin(pos2(100.0, 100.0), vec2(250.0, 300.0), bounds());
        assert_eq!(origin, pos2(102.0, 102.0));
    }

    #[test]
    fn a_menu_flips_left_and_up_at_the_far_edges() {
        let size = vec2(250.0, 300.0);
        let origin = menu_origin(pos2(900.0, 600.0), size, bounds());
        assert_eq!(origin, pos2(900.0 - OFFSET - 250.0, 600.0 - OFFSET - 300.0));
        let menu = Rect::from_min_size(origin, size);
        assert!(bounds().contains_rect(menu));
    }

    #[test]
    fn a_menu_never_leaves_the_window() {
        let size = vec2(250.0, 300.0);
        for x in [-50.0, 0.0, 5.0, 500.0, 995.0, 1200.0] {
            for y in [-50.0, 0.0, 5.0, 350.0, 695.0, 900.0] {
                let origin = menu_origin(pos2(x, y), size, bounds());
                assert!(origin.x >= bounds().left() + MARGIN - 0.001, "{x},{y}");
                assert!(origin.y >= bounds().top() + MARGIN - 0.001, "{x},{y}");
                assert!(
                    origin.x + size.x <= bounds().right() - MARGIN + 0.001,
                    "{x},{y}"
                );
                assert!(
                    origin.y + size.y <= bounds().bottom() - MARGIN + 0.001,
                    "{x},{y}"
                );
            }
        }
    }

    #[test]
    fn a_menu_taller_than_the_window_is_pinned_to_the_top() {
        let origin = menu_origin(pos2(100.0, 400.0), vec2(250.0, 900.0), bounds());
        assert_eq!(origin.y, bounds().top() + MARGIN);
    }

    #[test]
    fn a_submenu_opens_to_the_right_then_flips_left() {
        let menu = Rect::from_min_size(pos2(100.0, 100.0), vec2(250.0, 300.0));
        let row = Rect::from_min_size(pos2(105.0, 200.0), vec2(240.0, ROW));
        let size = vec2(220.0, 150.0);
        let right = submenu_origin(menu, row, size, bounds());
        assert!(right.x >= menu.right() - 3.0);
        assert_eq!(right.y, row.top() - FRAME);

        let near_edge = Rect::from_min_size(pos2(700.0, 100.0), vec2(250.0, 300.0));
        let flipped = submenu_origin(near_edge, row, size, bounds());
        assert!(flipped.x + size.x <= near_edge.left() + 3.0);
        assert!(flipped.x >= bounds().left());
    }

    #[test]
    fn a_submenu_is_kept_inside_the_window_vertically() {
        let menu = Rect::from_min_size(pos2(100.0, 300.0), vec2(250.0, 380.0));
        let row = Rect::from_min_size(pos2(105.0, 640.0), vec2(240.0, ROW));
        let size = vec2(220.0, 200.0);
        let origin = submenu_origin(menu, row, size, bounds());
        assert!(origin.y + size.y <= bounds().bottom() - MARGIN + 0.001);
    }

    #[test]
    fn the_safe_triangle_covers_the_path_to_the_submenu() {
        let sub = Rect::from_min_size(pos2(350.0, 200.0), vec2(200.0, 150.0));
        let from = pos2(300.0, 210.0);
        // Straight at the submenu, and diagonally down toward its far corner.
        assert!(heading_into(from, pos2(340.0, 215.0), sub, true));
        assert!(heading_into(from, pos2(340.0, 300.0), sub, true));
        // Back across the menu's own rows, away from the submenu, is not "heading in".
        assert!(!heading_into(from, pos2(200.0, 400.0), sub, true));
        assert!(!heading_into(from, pos2(300.0, 600.0), sub, true));
    }

    #[test]
    fn the_safe_triangle_works_for_a_submenu_on_the_left() {
        let sub = Rect::from_min_size(pos2(50.0, 200.0), vec2(200.0, 150.0));
        let from = pos2(300.0, 210.0);
        assert!(heading_into(from, pos2(260.0, 260.0), sub, false));
        assert!(!heading_into(from, pos2(400.0, 260.0), sub, false));
    }

    fn entries() -> Vec<Entry> {
        let item = |label: &str, disabled: bool| {
            Entry::Item(Item {
                command: Some(Command::OpenSettings),
                label: label.to_owned(),
                hint: None,
                disabled: disabled.then_some("no"),
                checked: false,
                current: false,
                glyph: None,
                destructive: false,
                children: Vec::new(),
            })
        };
        vec![
            item("a", false),
            Entry::Separator,
            item("b", true),
            item("c", false),
        ]
    }

    #[test]
    fn keyboard_focus_skips_separators_and_disabled_rows_and_wraps() {
        let entries = entries();
        assert_eq!(step_focus(&entries, None, 1), Some(0));
        assert_eq!(step_focus(&entries, Some(0), 1), Some(3));
        assert_eq!(step_focus(&entries, Some(3), 1), Some(0));
        assert_eq!(step_focus(&entries, None, -1), Some(3));
        assert_eq!(step_focus(&entries, Some(3), -1), Some(0));
        assert_eq!(step_focus(&entries, Some(0), -1), Some(3));
    }

    #[test]
    fn a_menu_with_nothing_enabled_has_no_focus() {
        let only_disabled = vec![Entry::Item(Item {
            command: Some(Command::OpenSettings),
            label: "x".to_owned(),
            hint: None,
            disabled: Some("no"),
            checked: false,
            current: false,
            glyph: None,
            destructive: false,
            children: Vec::new(),
        })];
        assert_eq!(step_focus(&only_disabled, None, 1), None);
        assert_eq!(step_focus(&[], None, 1), None);
    }

    #[test]
    fn heights_add_rows_and_separators() {
        let entries = entries();
        assert_eq!(height_of(&entries), FRAME * 2.0 + ROW * 3.0 + SEPARATOR);
    }

    fn plain(label: &str) -> Entry {
        Entry::Item(Item {
            command: None,
            label: label.to_owned(),
            hint: None,
            disabled: None,
            checked: false,
            current: false,
            glyph: None,
            destructive: false,
            children: Vec::new(),
        })
    }

    #[test]
    fn a_column_without_icons_does_not_reserve_their_gutter() {
        let none = vec![plain("One"), Entry::Separator, plain("Two")];
        assert!(!needs_icon_column(&none));
        // One icon, or one check, anywhere in the column brings the gutter back for all of it.
        let mut with_icon = none.clone();
        with_icon.push(Entry::Item(Item {
            command: Some(Command::OpenSettings),
            ..match plain("Settings") {
                Entry::Item(item) => item,
                _ => unreachable!(),
            }
        }));
        assert!(needs_icon_column(&with_icon));
        let mut with_check = none.clone();
        if let Some(Entry::Item(item)) = with_check.first_mut() {
            item.checked = true;
        }
        assert!(needs_icon_column(&with_check));
        // A trailing "current" mark is not a leading one.
        let mut current = none;
        if let Some(Entry::Item(item)) = current.first_mut() {
            item.current = true;
        }
        assert!(!needs_icon_column(&current));
        assert!(!needs_icon_column(&[Entry::Heading("Presets".to_owned())]));
    }

    #[test]
    fn rows_that_can_have_an_icon_do() {
        let icon = |command: Command| {
            icon_for(&Item {
                command: Some(command),
                ..match plain("x") {
                    Entry::Item(item) => item,
                    _ => unreachable!(),
                }
            })
        };
        assert_eq!(icon(Command::Paste), Some(Icon::Clipboard));
        assert_eq!(icon(Command::ApplyTemplate(1)), Some(Icon::Palette));
        assert_eq!(icon(Command::SaveTemplate), Some(Icon::Plus));
        assert_eq!(icon(Command::ManageTemplates), Some(Icon::Sliders));
        assert_eq!(icon(Command::ResetTemplate), Some(Icon::Undo));
        assert_eq!(
            icon(Command::QuickOrder(
                crate::model::chart_context::QuickOrder {
                    side: PaperOrderSide::Sell,
                    kind: crate::model::paper::PaperOrderType::Limit,
                }
            )),
            Some(Icon::ArrowDown)
        );
        assert_eq!(
            icon(Command::QuickOrder(
                crate::model::chart_context::QuickOrder {
                    side: PaperOrderSide::Buy,
                    kind: crate::model::paper::PaperOrderType::Stop,
                }
            )),
            Some(Icon::ArrowUp)
        );
        // A submenu takes its glyph from the model, and a placeholder has none.
        let submenu = Item {
            glyph: Some(Glyph::Template),
            ..match plain("Chart template") {
                Entry::Item(item) => item,
                _ => unreachable!(),
            }
        };
        assert_eq!(icon_for(&submenu), Some(Icon::Palette));
        assert_eq!(
            icon_for(&match plain("None saved yet") {
                Entry::Item(item) => item,
                _ => unreachable!(),
            }),
            None
        );
    }

    #[test]
    fn the_alert_toggle_has_an_icon_for_each_state() {
        let toggle = |glyph| Item {
            glyph: Some(glyph),
            ..match plain("Pause alert") {
                Entry::Item(item) => item,
                _ => unreachable!(),
            }
        };
        assert_eq!(icon_for(&toggle(Glyph::Pause)), Some(Icon::PauseFill));
        assert_eq!(icon_for(&toggle(Glyph::Play)), Some(Icon::PlayFill));
    }

    #[test]
    fn a_short_menu_hugs_its_labels_and_a_long_row_still_wins() {
        // "Delete alert" is about 75pt of text behind the icon column.
        let short = row_width(32.0, 75.0, 0.0, 0.0).clamp(MIN_WIDTH, MAX_WIDTH);
        assert!(short < 170.0, "{short}");
        let long = row_width(32.0, 260.0, 0.0, 0.0).clamp(MIN_WIDTH, MAX_WIDTH);
        assert_eq!(long, 306.0);
    }

    #[test]
    fn headings_take_a_slot_in_the_height() {
        let entries = vec![
            Entry::Heading("Presets".to_owned()),
            plain("One"),
            Entry::Separator,
        ];
        assert_eq!(height_of(&entries), FRAME * 2.0 + HEADING + ROW + SEPARATOR);
        // A heading is never a focus stop.
        assert_eq!(step_focus(&entries, None, 1), Some(1));
        assert_eq!(step_focus(&entries, Some(1), 1), Some(1));
    }
}
