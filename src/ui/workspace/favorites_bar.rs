//! The floating bar of favorite drawing tools over the chart area: a grip to move it by, then one
//! button per starred tool. A button arms its tool exactly as the same tool's tile or menu row does
//! (the caller routes `Select` through the tab's one tool choice), and lights while that tool is
//! armed. A right-click on the bar offers to hide it.
//!
//! The bar is an area above the charts, so a press on it never reaches them; everywhere else the
//! charts see the pointer as before. Dragging it keeps its position in egui's memory until the
//! pointer is let go, and only then reports `Moved` for the caller to save.

use crate::model::Tool;
use crate::model::drawings::favorites::{BarPlacement, BarPrefs, placement_from};
use crate::ui::workspace::tool_sidebar::tool_icon;
use crate::ui::{icons, kit, shortcut, theme};
use eframe::egui::{
    self, Context, CursorIcon, Id, Order, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2,
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
    /// The starred tools, in the order to show them.
    pub tools: &'a [Tool],
    /// The tool that is armed now.
    pub active_tool: Tool,
    pub prefs: BarPrefs,
    /// The part of the window the bar may be moved over: the charts.
    pub area: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FavoritesBarAction {
    /// The button of this tool was pressed.
    Select(Tool),
    /// The bar was moved and let go here.
    Moved(BarPlacement),
    /// Hide the bar (its stars stay).
    Hide,
}

/// The size of a bar holding `count` tools.
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

/// Where the bar's top-left corner is: its saved place, or the default one.
pub fn resolve(prefs: &BarPrefs, area: Rect, count: usize) -> Pos2 {
    corner(prefs.resolve(
        pair(area.min.to_vec2()),
        pair(area.size()),
        pair(bar_size(count)),
    ))
}

/// `position`, brought inside the area.
fn clamped(position: Pos2, area: Rect, count: usize) -> Pos2 {
    let placement = placement_from(
        pair(position.to_vec2()),
        pair(area.min.to_vec2()),
        pair(area.size()),
        pair(bar_size(count)),
    );
    resolve(
        &BarPrefs {
            placement: Some(placement),
            hidden: false,
        },
        area,
        count,
    )
}

/// Whether the bar is shown for this input: there is a tool to show, it is not hidden, and the
/// area has room for it.
pub fn is_shown(input: &FavoritesBarInput) -> bool {
    let size = bar_size(input.tools.len());
    !input.tools.is_empty()
        && !input.prefs.hidden
        && input.area.width() >= size.x + MIN_ROOM
        && input.area.height() >= size.y + MIN_ROOM
}

fn tooltip(tool: Tool) -> String {
    match shortcut::tool(tool) {
        Some(shortcut) => format!("{} ({})", tool.label(), shortcut.label()),
        None => tool.label().to_owned(),
    }
}

pub fn show(ctx: &Context, input: &FavoritesBarInput) -> Vec<FavoritesBarAction> {
    let mut actions = Vec::new();
    if !is_shown(input) {
        return actions;
    }
    let count = input.tools.len();
    let drag_id = input.id.with("drag");
    let mut live: Option<Pos2> = ctx.data(|data| data.get_temp(drag_id));
    if let Some(at) = live.as_mut() {
        // Follows the pointer in this frame, not the next one.
        *at = clamped(
            *at + ctx.input(|input| input.pointer.delta()),
            input.area,
            count,
        );
        ctx.data_mut(|data| data.insert_temp(drag_id, *at));
    }
    let position = live.unwrap_or_else(|| resolve(&input.prefs, input.area, count));

    egui::Area::new(input.id)
        .order(Order::Foreground)
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
    let count = input.tools.len();
    let inner = bar_size(count) - 2.0 * vec2(MARGIN_X + STROKE, MARGIN_Y + STROKE);
    let (row, _) = ui.allocate_exact_size(inner, Sense::hover());
    let mut responses: Vec<egui::Response> = Vec::with_capacity(count + 1);

    let grip_rect = Rect::from_min_size(row.min, vec2(GRIP, BUTTON));
    let grip = ui
        .interact(grip_rect, input.id.with("grip"), Sense::drag())
        .on_hover_cursor(if dragging {
            CursorIcon::Grabbing
        } else {
            CursorIcon::Grab
        });
    let grip = crate::ui::accessibility::label(grip, "Move favorites toolbar");
    if grip.drag_started() {
        ui.ctx()
            .data_mut(|data| data.insert_temp(drag_id, position));
    }
    if grip.drag_stopped() {
        ui.ctx().data_mut(|data| data.remove::<Pos2>(drag_id));
        actions.push(FavoritesBarAction::Moved(placement_from(
            pair(position.to_vec2()),
            pair(input.area.min.to_vec2()),
            pair(input.area.size()),
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

    for (index, tool) in input.tools.iter().copied().enumerate() {
        let left = row.left() + GRIP + SPACING + index as f32 * (BUTTON + SPACING);
        let rect = Rect::from_min_size(pos2(left, row.top()), Vec2::splat(BUTTON));
        let mut response = ui.interact(rect, input.id.with(("tool", tool)), Sense::click());
        if !dragging {
            response = response
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(tooltip(tool));
        }
        let response = crate::ui::accessibility::label(response, tool.label());
        let active = input.active_tool == tool;
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
            tool_icon(tool),
            theme::text_primary(),
        );
        if response.clicked() {
            actions.push(FavoritesBarAction::Select(tool));
        }
        responses.push(response);
    }

    // A right-click lands on a button or the grip, whichever is under the pointer.
    let rows = [(HIDE_LABEL, None)];
    for response in &responses {
        kit::fitted_context_menu(ui, response, &rows, |ui| {
            if kit::menu_item(ui, icons::Icon::EyeSlash, HIDE_LABEL, true, false) {
                actions.push(FavoritesBarAction::Hide);
                ui.close();
            }
        });
    }
}

const HIDE_LABEL: &str = "Hide favorites toolbar";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::render_tests::{Screen, gallery_dir};

    const AREA: Rect = Rect {
        min: pos2(44.0, 40.0),
        max: pos2(944.0, 640.0),
    };
    const TOOLS: [Tool; 3] = [Tool::Brush, Tool::Highlighter, Tool::TrendLine];

    fn input(prefs: BarPrefs, active_tool: Tool) -> FavoritesBarInput<'static> {
        FavoritesBarInput {
            id: Id::new("test-bar"),
            tools: &TOOLS,
            active_tool,
            prefs,
            area: AREA,
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
    fn a_button_arms_its_tool_and_nothing_else() {
        let prefs = BarPrefs::default();
        let at = resolve(&prefs, AREA, TOOLS.len());
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
            tools: &[],
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
        let mut rig = Rig::new();
        rig.settle(&hidden);
        assert!(!rig.ctx.is_pointer_over_egui());
        assert!(rig.frame(Vec::new(), &hidden).is_empty());
    }

    #[test]
    fn the_bar_takes_the_pointer_where_it_is_and_nowhere_else() {
        let prefs = BarPrefs::default();
        let at = resolve(&prefs, AREA, TOOLS.len());
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
        let at = resolve(&prefs, AREA, TOOLS.len());
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
        let [FavoritesBarAction::Moved(placement)] = actions[..] else {
            panic!("a drag asked for {actions:?}");
        };
        let moved = resolve(
            &BarPrefs {
                placement: Some(placement),
                hidden: false,
            },
            AREA,
            TOOLS.len(),
        );
        assert!((moved.x - (at.x - 120.0)).abs() < 1.0, "{moved:?}");
        assert!((moved.y - (at.y + 250.0)).abs() < 1.0, "{moved:?}");
    }

    #[test]
    fn a_bar_dragged_out_of_the_window_stays_in_the_chart_area() {
        let prefs = BarPrefs::default();
        let at = resolve(&prefs, AREA, TOOLS.len());
        let start = center(at, None);
        let mut rig = Rig::new();
        let input = input(prefs, Tool::Cursor);
        rig.settle(&input);
        rig.frame(vec![egui::Event::PointerMoved(start)], &input);
        rig.button(start, egui::PointerButton::Primary, true, &input);
        let far = pos2(5000.0, 5000.0);
        rig.frame(vec![egui::Event::PointerMoved(far)], &input);
        let actions = rig.button(far, egui::PointerButton::Primary, false, &input);
        let [FavoritesBarAction::Moved(placement)] = actions[..] else {
            panic!("a drag asked for {actions:?}");
        };
        assert_eq!(placement, BarPlacement { x: 1.0, y: 1.0 });
    }

    #[test]
    fn a_right_click_offers_to_hide_the_bar() {
        let prefs = BarPrefs::default();
        let at = resolve(&prefs, AREA, TOOLS.len());
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

    /// Hand-run: the bar in both themes, with one, two and many tools and the second armed.
    /// `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib gallery_of_the_favorites_bar -- --ignored`
    #[test]
    #[ignore]
    fn gallery_of_the_favorites_bar() {
        if gallery_dir().is_none() {
            return;
        }
        let many = [
            Tool::Highlighter,
            Tool::ArrowMarkDown,
            Tool::Rectangle,
            Tool::Brush,
            Tool::TrendLine,
            Tool::FibRetracement,
            Tool::Text,
        ];
        for (name, tools, active) in [
            ("one", &many[..1], Tool::Cursor),
            ("two", &many[3..5], Tool::TrendLine),
            ("many", &many[..], Tool::Rectangle),
        ] {
            let mut screen = Screen::new(vec2(700.0, 200.0));
            let input = FavoritesBarInput {
                id: Id::new("gallery-bar"),
                tools,
                active_tool: active,
                prefs: BarPrefs::default(),
                area: Rect::from_min_size(pos2(0.0, 0.0), vec2(700.0, 200.0)),
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
