//! The floating bar of favorites over the chart area: a grip to move it by, then one button per
//! starred drawing tool or cursor. A button does what the same tool's tile or menu row, or the
//! cursor's menu row, does (the caller routes `Select` through the tab's one tool and cursor
//! choice), and lights while that tool is armed or that cursor is in force. A right-click on the
//! bar offers to hide it.
//!
//! The bar is an area above the charts, so a press on it never reaches them, and below every popup
//! and dialog; everywhere else the charts see the pointer as before. Dragging it keeps its
//! position in egui's memory until the pointer is let go, and only then reports `Moved` for the
//! caller to save.

use crate::i18n::tr;
use crate::model::drawings::favorites::{BarPrefs, Favorite, Region};
use crate::model::{CursorMode, Tool};
use crate::ui::workspace::cursor_menu;
use crate::ui::workspace::tool_sidebar::tool_icon;
use crate::ui::{icons, kit, shortcut, theme};
use eframe::egui::{
    self, Context, CursorIcon, Id, Order, PointerButton, Pos2, Rect, Sense, Stroke, Vec2, pos2,
    vec2,
};

const BUTTON: f32 = 28.0;
const ICON: f32 = 17.0;
const GRIP: f32 = 16.0;
const SPACING: f32 = 2.0;
const MARGIN_X: f32 = 5.0;
const MARGIN_Y: f32 = 4.0;
/// The frame's hairline, on every side.
const STROKE: f32 = 1.0;
/// Room the chart area must have, beyond the bar, for the bar to be shown at all.
const MIN_ROOM: f32 = 8.0;

pub struct FavoritesBarInput<'a> {
    /// Names the bar's area, so each tab has its own.
    pub id: Id,
    /// The starred tools and cursors, in the order to show them.
    pub items: &'a [Favorite],
    /// The tool that is armed now.
    pub active_tool: Tool,
    /// The cursor in force, and whether Zoom In holds the pointer instead.
    pub cursor: CursorMode,
    pub zoom_armed: bool,
    pub prefs: BarPrefs,
    /// The charts' area, where a bar that was never moved sits.
    pub area: Rect,
    /// The part of the window the bar may be moved over: all of it below the tab strip, so it can
    /// be put over the toolbar or a side panel as well as the charts.
    pub window: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FavoritesBarAction {
    /// The button of this favorite was pressed.
    Select(Favorite),
    /// The bar was moved and let go here.
    /// The bar was let go here: the prefs to save.
    Moved(BarPrefs),
    /// Hide the bar (its stars stay).
    Hide,
}

/// The size of a bar holding `count` favorites.
pub fn bar_size(count: usize) -> Vec2 {
    let buttons = count as f32 * BUTTON + count.saturating_sub(1) as f32 * SPACING;
    vec2(
        GRIP + SPACING + buttons + 2.0 * (MARGIN_X + STROKE),
        BUTTON + 2.0 * (MARGIN_Y + STROKE),
    )
}

fn pair(vec: Vec2) -> [f32; 2] {
    [vec.x, vec.y]
}

fn corner(at: [f32; 2]) -> Pos2 {
    pos2(at[0], at[1])
}

fn region(rect: Rect) -> Region {
    Region {
        origin: pair(rect.min.to_vec2()),
        size: pair(rect.size()),
    }
}

/// Where the bar's top-left corner is: its saved place, or the default one.
pub fn resolve(prefs: &BarPrefs, area: Rect, window: Rect, count: usize) -> Pos2 {
    corner(prefs.resolve(region(area), region(window), pair(bar_size(count))))
}

/// `position`, brought inside the window.
fn clamped(position: Pos2, input: &FavoritesBarInput) -> Pos2 {
    let moved = BarPrefs::default().moved_to(
        pair(position.to_vec2()),
        region(input.window),
        pair(bar_size(input.items.len())),
    );
    resolve(&moved, input.area, input.window, input.items.len())
}

/// Whether the bar is shown for this input: there is a favorite to show, it is not hidden, and the
/// room it sits in has space for it. A bar placed in the window needs the window to have it, so
/// opening a side panel that narrows the charts does not make it vanish; one that was never moved,
/// or was saved by an older build, needs the charts' area to.
pub fn is_shown(input: &FavoritesBarInput) -> bool {
    let size = bar_size(input.items.len());
    let room = if input.prefs.in_window() {
        input.window
    } else {
        input.area
    };
    !input.items.is_empty()
        && !input.prefs.hidden
        && room.width() >= size.x + MIN_ROOM
        && room.height() >= size.y + MIN_ROOM
}

fn label(item: Favorite) -> &'static str {
    match item {
        Favorite::Tool(tool) => tool.label(),
        Favorite::Cursor(mode) => mode.label(),
    }
}

fn icon(item: Favorite) -> icons::Icon {
    match item {
        Favorite::Tool(tool) => tool_icon(tool),
        Favorite::Cursor(mode) => cursor_menu::icon(mode),
    }
}

fn tooltip(item: Favorite) -> String {
    match item {
        Favorite::Tool(tool) => match shortcut::tool(tool) {
            Some(shortcut) => format!("{} ({})", tool.label(), shortcut.label()),
            None => tool.label().to_owned(),
        },
        Favorite::Cursor(mode) => format!("{} cursor", mode.label()),
    }
}

/// Whether the button of `item` is lit: its tool is armed, or its cursor is the one in force and
/// no tool or Zoom In holds the pointer.
fn is_lit(item: Favorite, input: &FavoritesBarInput) -> bool {
    match item {
        Favorite::Tool(tool) => input.active_tool == tool,
        Favorite::Cursor(mode) => {
            input.active_tool == Tool::Cursor && !input.zoom_armed && input.cursor == mode
        }
    }
}

/// The grip being dragged: where the bar is, and the last frame that moved it.
#[derive(Clone, Copy)]
struct Drag {
    at: Pos2,
    frame: u64,
}

fn keep_drag(ctx: &Context, id: Id, at: Pos2) {
    let drag = Drag {
        at,
        frame: ctx.cumulative_frame_nr(),
    };
    ctx.data_mut(|data| data.insert_temp(id, drag));
}

/// Where the bar is in a drag that is still going on. A drag whose end was never seen (the bar was
/// not drawn when the button was let go) is dropped, or the bar would follow the pointer with no
/// button held when it came back: one that missed a frame, or that has no button down now. The
/// frame the button is let go is kept, for the grip to report where the bar was left.
fn drag_in_progress(ctx: &Context, id: Id) -> Option<Pos2> {
    let drag: Drag = ctx.data(|data| data.get_temp(id))?;
    let (down, released) = ctx.input(|input| {
        (
            input.pointer.primary_down(),
            input.pointer.primary_released(),
        )
    });
    let missed_a_frame = ctx.cumulative_frame_nr() > drag.frame + 1;
    if missed_a_frame || !(down || released) {
        ctx.data_mut(|data| data.remove::<Drag>(id));
        return None;
    }
    Some(drag.at)
}

pub fn show(ctx: &Context, input: &FavoritesBarInput) -> Vec<FavoritesBarAction> {
    let mut actions = Vec::new();
    if !is_shown(input) {
        return actions;
    }
    let count = input.items.len();
    let drag_id = input.id.with("drag");
    let mut live = drag_in_progress(ctx, drag_id);
    if let Some(at) = live.as_mut() {
        // Follows the pointer in this frame, not the next one.
        *at = clamped(*at + ctx.input(|input| input.pointer.delta()), input);
        keep_drag(ctx, drag_id, *at);
    }
    let position = live.unwrap_or_else(|| resolve(&input.prefs, input.area, input.window, count));

    egui::Area::new(input.id)
        // Above the charts but below every popup, menu, drawing toolbar and dialog, however late
        // the bar is first drawn (an area made later lands on top of its layer order): a flyout
        // still open when its first star makes the bar appear must keep the pointer.
        .order(Order::Middle)
        .fixed_pos(position)
        .interactable(true)
        .show(ctx, |ui| {
            kit::menu_frame()
                .inner_margin(egui::Margin::symmetric(MARGIN_X as i8, MARGIN_Y as i8))
                .stroke(Stroke::new(STROKE, theme::border()))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    bar_body(ui, input, live.is_some(), drag_id, position, &mut actions);
                });
        });
    actions
}

fn bar_body(
    ui: &mut egui::Ui,
    input: &FavoritesBarInput,
    dragging: bool,
    drag_id: Id,
    position: Pos2,
    actions: &mut Vec<FavoritesBarAction>,
) {
    let count = input.items.len();
    let inner = bar_size(count) - 2.0 * vec2(MARGIN_X + STROKE, MARGIN_Y + STROKE);
    let (row, _) = ui.allocate_exact_size(inner, Sense::hover());
    let mut responses: Vec<egui::Response> = Vec::with_capacity(count + 1);

    let grip_rect = Rect::from_min_size(row.min, vec2(GRIP, BUTTON));
    // A press that does not move is a click, which is what a right-click needs to open the menu.
    let grip = ui
        .interact(grip_rect, input.id.with("grip"), Sense::click_and_drag())
        .on_hover_cursor(if dragging {
            CursorIcon::Grabbing
        } else {
            CursorIcon::Grab
        });
    let grip = crate::ui::accessibility::label(grip, tr!("Move favorites toolbar"));
    if grip.drag_started_by(PointerButton::Primary) {
        // The drag starts once the pointer has moved a little: take that distance with it.
        let (pressed_at, now) = ui
            .ctx()
            .input(|input| (input.pointer.press_origin(), input.pointer.interact_pos()));
        let travelled = pressed_at
            .zip(now)
            .map_or(Vec2::ZERO, |(from, to)| to - from);
        keep_drag(ui.ctx(), drag_id, clamped(position + travelled, input));
    }
    if grip.drag_stopped_by(PointerButton::Primary) {
        ui.ctx().data_mut(|data| data.remove::<Drag>(drag_id));
        actions.push(FavoritesBarAction::Moved(input.prefs.moved_to(
            pair(position.to_vec2()),
            region(input.window),
            pair(bar_size(count)),
        )));
    }
    let grip_color = if grip.hovered() || dragging {
        theme::text_primary()
    } else {
        theme::text_tertiary()
    };
    for column in [-2.5, 2.5] {
        for line in [-5.0, 0.0, 5.0] {
            ui.painter()
                .circle_filled(grip_rect.center() + vec2(column, line), 1.3, grip_color);
        }
    }
    responses.push(grip);

    for (index, item) in input.items.iter().copied().enumerate() {
        let left = row.left() + GRIP + SPACING + index as f32 * (BUTTON + SPACING);
        let rect = Rect::from_min_size(pos2(left, row.top()), Vec2::splat(BUTTON));
        let mut response = ui.interact(rect, input.id.with(("favorite", item)), Sense::click());
        if !dragging {
            response = response
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(tooltip(item));
        }
        let active = is_lit(item, input);
        let response = crate::ui::accessibility::selected_as(
            response,
            egui::WidgetType::Button,
            label(item),
            active,
        );
        if active {
            ui.painter()
                .rect_filled(rect, 6.0, theme::selected_tint(true));
        } else if response.hovered() && !dragging {
            ui.painter()
                .rect_filled(rect, 6.0, theme::selected_tint(false));
        }
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect.shrink(1.0),
                6.0,
                Stroke::new(1.0, theme::text_secondary()),
                egui::StrokeKind::Inside,
            );
        }
        icons::paint(
            ui.painter(),
            Rect::from_center_size(rect.center(), Vec2::splat(ICON)),
            icon(item),
            theme::text_primary(),
        );
        if response.clicked() {
            actions.push(FavoritesBarAction::Select(item));
        }
        responses.push(response);
    }

    // A right-click lands on a button or the grip, whichever is under the pointer.
    let rows = [(hide_label(), None)];
    for response in &responses {
        kit::fitted_context_menu(ui, response, &rows, |ui| {
            if kit::menu_item(ui, icons::Icon::EyeSlash, hide_label(), true, false) {
                actions.push(FavoritesBarAction::Hide);
                ui.close();
            }
        });
    }
    // Which menu is open, for the next frame's Escape to reach before the charts do.
    let open = responses
        .iter()
        .map(egui::Popup::default_response_id)
        .find(|popup| egui::Popup::is_id_open(ui.ctx(), *popup));
    let slot = open_menu_slot(input.id);
    ui.ctx().data_mut(|data| {
        if let Some(popup) = open {
            data.insert_temp(slot, popup);
        } else {
            data.remove::<Id>(slot);
        }
    });
}

/// Where the bar keeps the id of its open menu's popup, for `close_menu_on_escape`.
fn open_menu_slot(bar: Id) -> Id {
    bar.with("open-menu")
}

/// Closes the bar's open menu on Escape and spends the key, so that it does not also end a draft,
/// disarm a tool or deselect a drawing behind the menu. egui's popup closes on any Escape without
/// taking it, and the charts read the key before the bar is drawn: call this first in the frame,
/// as the chart scale menu does (`scale_menu::close_on_escape`). Returns whether it took the key.
pub fn close_menu_on_escape(ctx: &Context, bar: Id) -> bool {
    let Some(popup) = ctx.data(|data| data.get_temp::<Id>(open_menu_slot(bar))) else {
        return false;
    };
    if egui::Popup::is_id_open(ctx, popup)
        && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        egui::Popup::close_id(ctx, popup);
        return true;
    }
    false
}

fn hide_label() -> &'static str {
    tr!("Hide favorites toolbar")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::drawings::favorites::BarPlacement;
    use crate::ui::render_tests::{Screen, gallery_dir};

    const AREA: Rect = Rect {
        min: pos2(44.0, 40.0),
        max: pos2(944.0, 640.0),
    };
    /// The whole window below the tab strip: it holds the toolbar above the charts and the tool
    /// rail beside them.
    const WINDOW: Rect = Rect {
        min: pos2(0.0, 0.0),
        max: pos2(1000.0, 700.0),
    };
    const TOOLS: [Favorite; 3] = [
        Favorite::Tool(Tool::Brush),
        Favorite::Tool(Tool::Highlighter),
        Favorite::Tool(Tool::TrendLine),
    ];

    fn input(prefs: BarPrefs, active_tool: Tool) -> FavoritesBarInput<'static> {
        FavoritesBarInput {
            id: Id::new("test-bar"),
            items: &TOOLS,
            active_tool,
            cursor: CursorMode::Cross,
            zoom_armed: false,
            prefs,
            area: AREA,
            window: WINDOW,
        }
    }

    /// The centre of the grip (`None`) or of a tool's button, for a bar at `at`.
    fn center(at: Pos2, tool: Option<usize>) -> Pos2 {
        let top = at.y + STROKE + MARGIN_Y + BUTTON / 2.0;
        let left = at.x + STROKE + MARGIN_X;
        match tool {
            None => pos2(left + GRIP / 2.0, top),
            Some(index) => pos2(
                left + GRIP + SPACING + index as f32 * (BUTTON + SPACING) + BUTTON / 2.0,
                top,
            ),
        }
    }

    struct Rig {
        ctx: Context,
        time: f64,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                ctx: Context::default(),
                time: 0.0,
            }
        }

        fn frame(
            &mut self,
            events: Vec<egui::Event>,
            input: &FavoritesBarInput,
        ) -> Vec<FavoritesBarAction> {
            self.time += 0.1;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let mut actions = Vec::new();
            let mut output = self.ctx.run_ui(raw, |ui| {
                actions = show(ui.ctx(), input);
            });
            output.textures_delta.clear();
            actions
        }

        fn settle(&mut self, input: &FavoritesBarInput) {
            for _ in 0..3 {
                self.frame(Vec::new(), input);
            }
        }

        fn button(
            &mut self,
            at: Pos2,
            button: egui::PointerButton,
            pressed: bool,
            input: &FavoritesBarInput,
        ) -> Vec<FavoritesBarAction> {
            self.frame(
                vec![egui::Event::PointerButton {
                    pos: at,
                    button,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
                input,
            )
        }

        fn click(&mut self, at: Pos2, input: &FavoritesBarInput) -> Vec<FavoritesBarAction> {
            self.frame(vec![egui::Event::PointerMoved(at)], input);
            self.button(at, egui::PointerButton::Primary, true, input);
            self.button(at, egui::PointerButton::Primary, false, input)
        }
    }

    #[test]
    fn a_cursor_button_lights_while_that_cursor_is_in_force_and_no_tool_or_zoom_holds_the_pointer()
    {
        const MIXED: [Favorite; 3] = [
            Favorite::Cursor(CursorMode::Dot),
            Favorite::Tool(Tool::Brush),
            Favorite::Cursor(CursorMode::Eraser),
        ];
        let lit = |cursor, active_tool, zoom_armed| {
            let input = FavoritesBarInput {
                items: &MIXED,
                cursor,
                active_tool,
                zoom_armed,
                ..input(BarPrefs::default(), Tool::Cursor)
            };
            MIXED.map(|item| is_lit(item, &input))
        };
        assert_eq!(
            lit(CursorMode::Dot, Tool::Cursor, false),
            [true, false, false]
        );
        assert_eq!(
            lit(CursorMode::Eraser, Tool::Cursor, false),
            [false, false, true]
        );
        assert_eq!(lit(CursorMode::Cross, Tool::Cursor, false), [false; 3]);
        // A tool in hand lights its own button and puts every cursor's out.
        assert_eq!(
            lit(CursorMode::Cross, Tool::Brush, false),
            [false, true, false]
        );
        assert_eq!(lit(CursorMode::Dot, Tool::Cursor, true), [false; 3]);
    }

    #[test]
    fn a_cursor_button_reports_its_cursor_and_is_told_to_a_screen_reader() {
        let prefs = BarPrefs::default();
        let items = [
            Favorite::Cursor(CursorMode::Arrow),
            Favorite::Tool(Tool::Brush),
        ];
        let input = FavoritesBarInput {
            items: &items,
            ..input(prefs, Tool::Cursor)
        };
        let at = resolve(&prefs, AREA, WINDOW, items.len());
        let mut rig = Rig::new();
        rig.settle(&input);
        let actions = rig.click(center(at, Some(0)), &input);
        assert_eq!(
            actions,
            [FavoritesBarAction::Select(Favorite::Cursor(
                CursorMode::Arrow
            ))]
        );
        assert_eq!(label(items[0]), "Arrow");
        assert_eq!(tooltip(items[0]), "Arrow cursor");
    }

    #[test]
    fn a_button_arms_its_tool_and_nothing_else() {
        let prefs = BarPrefs::default();
        let at = resolve(&prefs, AREA, WINDOW, TOOLS.len());
        for (index, tool) in TOOLS.into_iter().enumerate() {
            let mut rig = Rig::new();
            let input = input(prefs, Tool::Cursor);
            rig.settle(&input);
            let actions = rig.click(center(at, Some(index)), &input);
            assert_eq!(actions, [FavoritesBarAction::Select(tool)]);
        }
    }

    #[test]
    fn nothing_shows_without_a_star_while_hidden_or_in_a_room_too_small() {
        let shown = input(BarPrefs::default(), Tool::Cursor);
        assert!(is_shown(&shown));
        let empty = FavoritesBarInput {
            items: &[],
            ..input(BarPrefs::default(), Tool::Cursor)
        };
        assert!(!is_shown(&empty));
        let hidden = input(
            BarPrefs {
                hidden: true,
                ..BarPrefs::default()
            },
            Tool::Cursor,
        );
        assert!(!is_shown(&hidden));
        let small = FavoritesBarInput {
            area: Rect::from_min_size(AREA.min, vec2(120.0, 300.0)),
            ..input(BarPrefs::default(), Tool::Cursor)
        };
        assert!(!is_shown(&small));
        // A bar placed in the window does not depend on the charts' room: a side panel can
        // narrow the charts below the bar's width without hiding it.
        let moved = BarPrefs::default().moved_to([300.0, 40.0], region(WINDOW), [200.0, 36.0]);
        let parked = FavoritesBarInput {
            area: small.area,
            ..input(moved, Tool::Cursor)
        };
        assert!(is_shown(&parked));
        let window_too_small = FavoritesBarInput {
            window: Rect::from_min_size(WINDOW.min, vec2(120.0, 300.0)),
            ..parked
        };
        assert!(!is_shown(&window_too_small));
        let mut rig = Rig::new();
        rig.settle(&hidden);
        assert!(!rig.ctx.is_pointer_over_egui());
        assert!(rig.frame(Vec::new(), &hidden).is_empty());
    }

    #[test]
    fn the_bar_takes_the_pointer_where_it_is_and_nowhere_else() {
        let prefs = BarPrefs::default();
        let at = resolve(&prefs, AREA, WINDOW, TOOLS.len());
        let input = input(prefs, Tool::Cursor);
        let mut rig = Rig::new();
        rig.settle(&input);
        for (point, over) in [
            (center(at, Some(1)), true),
            (center(at, None), true),
            (at + vec2(AREA.width() / 2.0, 300.0), false),
            (pos2(AREA.left() + 10.0, AREA.top() + 10.0), false),
        ] {
            rig.frame(vec![egui::Event::PointerMoved(point)], &input);
            rig.settle(&input);
            assert_eq!(rig.ctx.is_pointer_over_egui(), over, "{point:?}");
        }
    }

    #[test]
    fn dragging_the_grip_moves_the_bar_and_reports_where_it_was_let_go() {
        let prefs = BarPrefs::default();
        let at = resolve(&prefs, AREA, WINDOW, TOOLS.len());
        let start = center(at, None);
        let mut rig = Rig::new();
        let before = input(prefs, Tool::Cursor);
        rig.settle(&before);
        rig.frame(vec![egui::Event::PointerMoved(start)], &before);
        let mut actions = rig.button(start, egui::PointerButton::Primary, true, &before);
        let mut at_pointer = start;
        for step in 1..=10 {
            at_pointer = start + vec2(-12.0, 25.0) * step as f32;
            actions.extend(rig.frame(vec![egui::Event::PointerMoved(at_pointer)], &before));
        }
        actions.extend(rig.button(at_pointer, egui::PointerButton::Primary, false, &before));
        let [FavoritesBarAction::Moved(saved)] = actions[..] else {
            panic!("a drag asked for {actions:?}");
        };
        assert!(saved.window, "the place is a share of the whole window");
        let moved = resolve(&saved, AREA, WINDOW, TOOLS.len());
        assert!((moved.x - (at.x - 120.0)).abs() < 1.0, "{moved:?}");
        assert!((moved.y - (at.y + 250.0)).abs() < 1.0, "{moved:?}");
    }

    #[test]
    fn a_bar_dragged_out_of_the_window_stays_in_the_window() {
        let prefs = BarPrefs::default();
        let at = resolve(&prefs, AREA, WINDOW, TOOLS.len());
        let start = center(at, None);
        let mut rig = Rig::new();
        let input = input(prefs, Tool::Cursor);
        rig.settle(&input);
        rig.frame(vec![egui::Event::PointerMoved(start)], &input);
        rig.button(start, egui::PointerButton::Primary, true, &input);
        let far = pos2(5000.0, 5000.0);
        rig.frame(vec![egui::Event::PointerMoved(far)], &input);
        let actions = rig.button(far, egui::PointerButton::Primary, false, &input);
        let [FavoritesBarAction::Moved(saved)] = actions[..] else {
            panic!("a drag asked for {actions:?}");
        };
        assert_eq!(saved.placement, Some(BarPlacement { x: 1.0, y: 1.0 }));
        let at = resolve(&saved, AREA, WINDOW, TOOLS.len());
        let size = bar_size(TOOLS.len());
        assert_eq!(at, pos2(WINDOW.right() - size.x, WINDOW.bottom() - size.y));
    }

    #[test]
    fn the_bar_can_be_dragged_above_the_charts_and_comes_back_there() {
        let prefs = BarPrefs::default();
        let at = resolve(&prefs, AREA, WINDOW, TOOLS.len());
        assert!(at.y >= AREA.top(), "unmoved, it starts in the charts");
        let start = center(at, None);
        let mut rig = Rig::new();
        let input = input(prefs, Tool::Cursor);
        rig.settle(&input);
        rig.frame(vec![egui::Event::PointerMoved(start)], &input);
        rig.button(start, egui::PointerButton::Primary, true, &input);
        // Up into the toolbar, which lies above the charts' area.
        let target = pos2(start.x, WINDOW.top() + 12.0);
        let mut pointer = start;
        for step in 1..=10 {
            pointer = start + (target - start) * (step as f32 / 10.0);
            rig.frame(vec![egui::Event::PointerMoved(pointer)], &input);
        }
        let actions = rig.button(pointer, egui::PointerButton::Primary, false, &input);
        let [FavoritesBarAction::Moved(saved)] = actions[..] else {
            panic!("a drag asked for {actions:?}");
        };
        let moved = resolve(&saved, AREA, WINDOW, TOOLS.len());
        assert!(moved.y < AREA.top(), "{moved:?} is still inside the charts");
        assert!(moved.y >= WINDOW.top());
        // Saved and read back, a later frame draws it in the same place.
        let again = FavoritesBarInput {
            prefs: saved,
            ..input
        };
        rig.settle(&again);
        assert!((rig.bar_at().y - moved.y).abs() < 0.5);
    }

    #[test]
    fn a_right_click_offers_to_hide_the_bar() {
        let prefs = BarPrefs::default();
        let at = resolve(&prefs, AREA, WINDOW, TOOLS.len());
        let target = center(at, Some(0));
        let input = input(prefs, Tool::Cursor);
        let mut rig = Rig::new();
        rig.settle(&input);
        rig.frame(vec![egui::Event::PointerMoved(target)], &input);
        rig.button(target, egui::PointerButton::Secondary, true, &input);
        let opened = rig.button(target, egui::PointerButton::Secondary, false, &input);
        assert!(opened.is_empty(), "the right-click armed {opened:?}");
        rig.settle(&input);
        // The menu opens at the pointer: its one row is just below and right of it.
        let row = target + vec2(40.0, 17.0);
        let actions = rig.click(row, &input);
        assert_eq!(actions, [FavoritesBarAction::Hide]);
    }

    impl Rig {
        /// A frame in which the bar is not drawn at all (another tab is showing, say).
        fn away(&mut self, events: Vec<egui::Event>) {
            self.time += 0.1;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let _ = self.ctx.run_ui(raw, |_| {});
        }

        /// Where the bar's area is now.
        fn bar_at(&self) -> Pos2 {
            self.ctx
                .memory(|memory| memory.area_rect(Id::new("test-bar")))
                .expect("drawn")
                .min
        }
    }

    /// A drag is begun on the grip, the bar goes away (`away` runs its frames) and the button is let
    /// go meanwhile. When the bar is back, moving the pointer with no button held must not move it.
    fn a_drag_cut_short_is_not_resumed(away: impl Fn(&mut Rig, Vec<egui::Event>)) {
        let prefs = BarPrefs::default();
        let at = resolve(&prefs, AREA, WINDOW, TOOLS.len());
        let shown = input(prefs, Tool::Cursor);
        let start = center(at, None);
        let mut rig = Rig::new();
        rig.settle(&shown);
        rig.frame(vec![egui::Event::PointerMoved(start)], &shown);
        rig.button(start, egui::PointerButton::Primary, true, &shown);
        let mut held = start;
        for step in 1..=4 {
            held = start + vec2(20.0, 10.0) * step as f32;
            rig.frame(vec![egui::Event::PointerMoved(held)], &shown);
        }
        assert!(rig.bar_at().x > at.x + 40.0, "the drag moved the bar");

        away(&mut rig, vec![egui::Event::PointerMoved(held)]);
        away(
            &mut rig,
            vec![egui::Event::PointerButton {
                pos: held,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        away(&mut rig, Vec::new());

        // Back, with the button up: the pointer wanders and the bar stays where it is saved.
        let mut actions = Vec::new();
        for step in 1..=4 {
            let wander = held + vec2(-30.0, 15.0) * step as f32;
            actions.extend(rig.frame(vec![egui::Event::PointerMoved(wander)], &shown));
        }
        assert!(actions.is_empty(), "{actions:?}");
        let now = rig.bar_at();
        assert!(
            (now - at).length() < 0.5,
            "the bar followed the pointer: {now:?}"
        );
    }

    #[test]
    fn a_drag_cut_short_by_the_bar_being_hidden_is_not_resumed() {
        let hidden = input(
            BarPrefs {
                hidden: true,
                ..BarPrefs::default()
            },
            Tool::Cursor,
        );
        a_drag_cut_short_is_not_resumed(|rig, events| {
            rig.frame(events, &hidden);
        });
    }

    #[test]
    fn a_drag_cut_short_by_the_bar_not_being_drawn_is_not_resumed() {
        a_drag_cut_short_is_not_resumed(|rig, events| rig.away(events));
    }

    #[test]
    fn a_drag_with_no_button_down_is_dropped_even_while_the_bar_is_drawn() {
        let prefs = BarPrefs::default();
        let at = resolve(&prefs, AREA, WINDOW, TOOLS.len());
        let shown = input(prefs, Tool::Cursor);
        let mut rig = Rig::new();
        rig.settle(&shown);
        // A drag whose end was lost: left in memory with no button held.
        keep_drag(&rig.ctx, Id::new("test-bar").with("drag"), at);
        let mut actions = Vec::new();
        for step in 1..=4 {
            let wander = at + vec2(40.0, 30.0) * step as f32;
            actions.extend(rig.frame(vec![egui::Event::PointerMoved(wander)], &shown));
        }
        assert!(actions.is_empty(), "{actions:?}");
        assert!((rig.bar_at() - at).length() < 0.5);
    }

    /// A frame as the workspace runs it: the menu's Escape is taken first, then a chart reads the key,
    /// then the bar is drawn. Returns whether the menu took the key and whether the chart still saw it.
    fn escape_frame(rig: &mut Rig, input: &FavoritesBarInput) -> (bool, bool) {
        rig.time += 0.1;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0))),
            time: Some(rig.time),
            events: vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: Some(egui::Key::Escape),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        };
        let mut seen = (false, false);
        let _ = rig.ctx.run_ui(raw, |ui| {
            seen.0 = close_menu_on_escape(ui.ctx(), input.id);
            seen.1 = ui.input(|state| state.key_pressed(egui::Key::Escape));
            show(ui.ctx(), input);
        });
        seen
    }

    fn open_the_menu(rig: &mut Rig, input: &FavoritesBarInput, on: Pos2) {
        rig.settle(input);
        rig.frame(vec![egui::Event::PointerMoved(on)], input);
        rig.button(on, egui::PointerButton::Secondary, true, input);
        rig.button(on, egui::PointerButton::Secondary, false, input);
        rig.settle(input);
        assert!(egui::Popup::is_any_open(&rig.ctx), "the menu is open");
    }

    #[test]
    fn escape_closes_the_menu_and_is_spent_before_the_charts_see_it() {
        let prefs = BarPrefs::default();
        let at = resolve(&prefs, AREA, WINDOW, TOOLS.len());
        // Opened from a tool's button and from the grip.
        for on in [center(at, Some(1)), center(at, None)] {
            let input = input(prefs, Tool::Cursor);
            let mut rig = Rig::new();
            open_the_menu(&mut rig, &input, on);
            let (took, chart_saw) = escape_frame(&mut rig, &input);
            assert!(took, "the open menu did not take Escape");
            assert!(!chart_saw, "a chart still saw Escape behind the menu");
            rig.settle(&input);
            assert!(!egui::Popup::is_any_open(&rig.ctx), "the menu stayed open");
        }
    }

    #[test]
    fn escape_with_no_menu_open_is_left_for_the_charts() {
        let prefs = BarPrefs::default();
        let input = input(prefs, Tool::Cursor);
        let mut rig = Rig::new();
        rig.settle(&input);
        let (took, chart_saw) = escape_frame(&mut rig, &input);
        assert!(!took);
        assert!(chart_saw, "Escape was taken with no menu to close");
    }

    #[test]
    fn the_armed_tool_is_told_to_a_screen_reader() {
        use crate::ui::accessibility::AccessNodes;
        use egui::accesskit::Toggled;
        let prefs = BarPrefs::default();
        let input = input(prefs, Tool::Highlighter);
        let mut rig = Rig::new();
        let mut nodes = AccessNodes::start(&rig.ctx);
        for _ in 0..3 {
            rig.time += 0.1;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0))),
                time: Some(rig.time),
                ..Default::default()
            };
            let output = rig.ctx.run_ui(raw, |ui| {
                show(ui.ctx(), &input);
            });
            nodes.absorb(output);
        }
        let toggled = |label: &str| nodes.find(label).expect(label).toggled();
        assert_eq!(toggled("Brush"), Some(Toggled::False));
        assert_eq!(toggled("Highlighter"), Some(Toggled::True));
        assert_eq!(toggled("Trend line"), Some(Toggled::False));
        assert_eq!(toggled("Move favorites toolbar"), None);
    }

    #[test]
    fn a_right_click_on_the_grip_offers_to_hide_the_bar() {
        let prefs = BarPrefs::default();
        let at = resolve(&prefs, AREA, WINDOW, TOOLS.len());
        let target = center(at, None);
        let input = input(prefs, Tool::Cursor);
        let mut rig = Rig::new();
        rig.settle(&input);
        rig.frame(vec![egui::Event::PointerMoved(target)], &input);
        rig.button(target, egui::PointerButton::Secondary, true, &input);
        let opened = rig.button(target, egui::PointerButton::Secondary, false, &input);
        assert!(opened.is_empty(), "the right-click asked for {opened:?}");
        rig.settle(&input);
        let actions = rig.click(target + vec2(40.0, 17.0), &input);
        assert_eq!(actions, [FavoritesBarAction::Hide]);
    }

    #[test]
    fn only_the_main_button_moves_the_bar_and_a_plain_click_on_the_grip_does_nothing() {
        let prefs = BarPrefs::default();
        let at = resolve(&prefs, AREA, WINDOW, TOOLS.len());
        let start = center(at, None);
        let input = input(prefs, Tool::Cursor);

        let mut rig = Rig::new();
        rig.settle(&input);
        assert!(rig.click(start, &input).is_empty());

        let mut rig = Rig::new();
        rig.settle(&input);
        rig.frame(vec![egui::Event::PointerMoved(start)], &input);
        let mut actions = rig.button(start, egui::PointerButton::Secondary, true, &input);
        let mut at_pointer = start;
        for step in 1..=6 {
            at_pointer = start + vec2(15.0, 10.0) * step as f32;
            actions.extend(rig.frame(vec![egui::Event::PointerMoved(at_pointer)], &input));
        }
        actions.extend(rig.button(at_pointer, egui::PointerButton::Secondary, false, &input));
        rig.settle(&input);
        assert!(
            actions.is_empty(),
            "a right-button drag asked for {actions:?}"
        );
        assert!((rig.bar_at() - at).length() < 0.5);
    }

    /// Hand-run: the bar in both themes, with one, two and many tools and the second armed.
    /// `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib gallery_of_the_favorites_bar -- --ignored`
    #[test]
    #[ignore]
    fn gallery_of_the_favorites_bar() {
        if gallery_dir().is_none() {
            return;
        }
        let many = [
            Favorite::Cursor(CursorMode::Dot),
            Favorite::Cursor(CursorMode::Eraser),
            Favorite::Tool(Tool::Highlighter),
            Favorite::Tool(Tool::ArrowMarkDown),
            Favorite::Tool(Tool::Rectangle),
            Favorite::Tool(Tool::Brush),
            Favorite::Tool(Tool::TrendLine),
            Favorite::Tool(Tool::FibRetracement),
            Favorite::Tool(Tool::Text),
        ];
        for (name, items, active) in [
            ("one", &many[..1], Tool::Cursor),
            ("two", &many[5..7], Tool::TrendLine),
            ("many", &many[..], Tool::Rectangle),
        ] {
            let mut screen = Screen::new(vec2(700.0, 200.0));
            let input = FavoritesBarInput {
                id: Id::new("gallery-bar"),
                items,
                active_tool: active,
                cursor: CursorMode::Dot,
                zoom_armed: false,
                prefs: BarPrefs::default(),
                area: Rect::from_min_size(pos2(0.0, 0.0), vec2(700.0, 200.0)),
                window: Rect::from_min_size(pos2(0.0, 0.0), vec2(700.0, 200.0)),
            };
            let mut output = None;
            for _ in 0..3 {
                output = Some(screen.run(Vec::new(), |ui| {
                    ui.painter()
                        .rect_filled(ui.max_rect(), 0.0, theme::background());
                    show(ui.ctx(), &input);
                }));
            }
            screen.save(&format!("favorites-bar-{name}"), output.expect("ran"));
        }
    }
}
