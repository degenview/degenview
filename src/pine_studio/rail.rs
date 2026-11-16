//! The rail down the right edge of the code editor: the assistant, profiler and debugger, then
//! where the preview chart sits and whether it shows. It spans the whole editor column (code,
//! assistant and debugger panels) and never reaches over the preview chart. Icons only, on the
//! shared bar engine's column ([`bar::show_column`]), so its buttons look like the top bar's.

use super::commands::Cmd;
use super::library::prefs::{ChartPosition, LayoutPrefs};
use super::{ai, debug, profiler};
use crate::ui::icons::Icon;
use crate::ui::kit;
use crate::ui::kit::bar::{self, Button, Item};
use crate::ui::kit::header_controls;
use crate::ui::shortcut;
use crate::ui::theme;
use eframe::egui::{self, Response, Ui};
use std::cell::RefCell;

/// The panel's width with the separator egui draws on its inner edge, which leaves 4 pt either
/// side of a 32 pt icon button.
pub(super) const WIDTH: f32 = 41.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RailAction {
    ToggleAssistant,
    ToggleProfiler,
    ToggleDebugger,
    SetPosition(ChartPosition),
    ToggleChart,
}

impl From<RailAction> for Cmd {
    fn from(action: RailAction) -> Self {
        match action {
            RailAction::ToggleAssistant => Cmd::ToggleAiPanel,
            RailAction::ToggleProfiler => Cmd::ProfilerToggle,
            RailAction::ToggleDebugger => Cmd::ToggleDebugPanel,
            RailAction::SetPosition(position) => Cmd::SetChartPosition(position),
            RailAction::ToggleChart => Cmd::ToggleChart,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RailMenu {
    ChartPosition,
}

type RailItem = Item<RailAction, RailMenu>;
type RailButton = Button<RailAction, RailMenu>;

/// What the rail shows, from the studio's state.
pub(super) struct RailState {
    pub(super) chart_visible: bool,
    pub(super) chart_position: ChartPosition,
    pub(super) debugger_open: bool,
    pub(super) assistant_open: bool,
    pub(super) profiler: profiler::Status,
}

impl RailState {
    pub(super) fn new(prefs: &LayoutPrefs, profiler: profiler::Status) -> Self {
        Self {
            chart_visible: prefs.chart_visible,
            chart_position: prefs.chart_position,
            debugger_open: prefs.debug_open,
            assistant_open: prefs.ai_open,
            profiler,
        }
    }
}

#[cfg(test)]
impl RailState {
    /// The preview chart shown at the left, every panel closed, the profiler off.
    pub(in crate::pine_studio) fn idle() -> Self {
        Self {
            chart_visible: true,
            chart_position: ChartPosition::Left,
            debugger_open: false,
            assistant_open: false,
            profiler: profiler::Status::Off,
        }
    }
}

fn position_icon(position: ChartPosition) -> Icon {
    match position {
        ChartPosition::Left => Icon::RectLeft,
        ChartPosition::Top => Icon::RectTop,
        ChartPosition::Bottom => Icon::RectBottom,
    }
}

fn items(state: &RailState) -> Vec<RailItem> {
    let preview_toggle = shortcut::TOGGLE_PREVIEW_CHART.hint(if state.chart_visible {
        "Hide the preview chart"
    } else {
        "Show the preview chart"
    });
    vec![
        Item::Button(ai::panel::panel_button(
            state.assistant_open,
            RailAction::ToggleAssistant,
        )),
        Item::Button(profiler::ui::toolbar_button(
            &state.profiler,
            RailAction::ToggleProfiler,
        )),
        Item::Button(debug::panel::panel_button(
            state.debugger_open,
            RailAction::ToggleDebugger,
        )),
        Item::Separator,
        Item::Button(
            RailButton::new(
                "chart-position",
                "Where the preview chart sits next to the code",
            )
            .icon(position_icon(state.chart_position))
            .with_menu(RailMenu::ChartPosition),
        ),
        Item::Button(
            RailButton::new("preview-chart", preview_toggle)
                .icon(if state.chart_visible {
                    Icon::ChartLine
                } else {
                    Icon::ChartLineFlat
                })
                .active(state.chart_visible)
                .on_click(RailAction::ToggleChart),
        ),
    ]
}

/// Draws the rail at the right edge of `ui` and then `editor` in what is left of it. What the
/// buttons ask for is pushed to `cmds`.
pub(super) fn show(
    ui: &mut Ui,
    state: &RailState,
    cmds: &RefCell<Vec<Cmd>>,
    editor: impl FnOnce(&mut Ui),
) {
    let actions = egui::Panel::right("pine-studio-rail")
        .exact_size(WIDTH)
        .resizable(false)
        .frame(egui::Frame::new().fill(theme::chrome()))
        .show(ui, |ui| {
            bar::show_column(
                ui,
                &items(state),
                |ui, menu, response, actions| match menu {
                    RailMenu::ChartPosition => {
                        position_menu(ui, response, state.chart_position, actions);
                    }
                },
            )
        })
        .inner;
    cmds.borrow_mut().extend(actions.into_iter().map(Cmd::from));
    editor(ui);
}

/// Opens to the left of the rail, over the code, since there is no room beside it.
fn position_menu(
    ui: &mut Ui,
    response: &Response,
    current: ChartPosition,
    actions: &mut Vec<RailAction>,
) {
    let labels: Vec<&str> = ChartPosition::ALL.iter().map(|p| p.title()).collect();
    kit::fitted_menu(ui, response, egui::RectAlign::LEFT_START, &labels, |ui| {
        for position in ChartPosition::ALL {
            if header_controls::choice(ui, position.title(), position == current) {
                actions.push(RailAction::SetPosition(position));
            }
        }
    });
}

/// The middle of the `n`th button of a rail whose panel spans `from_top` to the bottom of a
/// window `width` wide, for the tests that click it.
#[cfg(test)]
pub(super) fn button_centre(width: f32, from_top: f32, n: usize) -> egui::Pos2 {
    let ctx = egui::Context::default();
    let mut placed = Vec::new();
    let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| {
        placed = bar::column_geometry(ui, &items(&RailState::idle()));
    });
    frame.textures_delta.clear();
    egui::pos2(
        width - (WIDTH - 1.0) / 2.0,
        from_top + (placed[n].from + placed[n].to) / 2.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, PointerButton, Pos2, Rect, pos2, vec2};

    const WINDOW: egui::Vec2 = vec2(600.0, 500.0);

    fn buttons(items: &[RailItem]) -> Vec<&RailButton> {
        items
            .iter()
            .filter_map(|item| match item {
                Item::Button(button) => Some(button),
                Item::Separator => None,
            })
            .collect()
    }

    #[test]
    fn the_panels_come_first_then_a_separator_then_the_preview_controls() {
        let items = items(&RailState::idle());
        let actions: Vec<_> = buttons(&items).iter().map(|b| b.action).collect();
        assert_eq!(
            actions,
            vec![
                Some(RailAction::ToggleAssistant),
                Some(RailAction::ToggleProfiler),
                Some(RailAction::ToggleDebugger),
                None,
                Some(RailAction::ToggleChart),
            ]
        );
        assert!(matches!(items[3], Item::Separator));
        assert_eq!(buttons(&items)[3].menu, Some(RailMenu::ChartPosition));
    }

    #[test]
    fn every_button_is_icon_only_and_says_what_it_does_in_its_tooltip() {
        for button in buttons(&items(&RailState::idle())) {
            assert!(button.glyph.is_some() && button.label.is_none());
            assert!(!button.tip.is_empty());
            assert!(button.enabled);
        }
    }

    #[test]
    fn the_position_icon_and_the_preview_toggle_follow_the_state() {
        for position in ChartPosition::ALL {
            let mut shown = RailState::idle();
            shown.chart_position = position;
            let items = items(&shown);
            let position_button = buttons(&items)[3];
            assert!(matches!(
                position_button.glyph,
                Some(bar::Glyph::Icon(icon)) if icon == position_icon(position)
            ));
        }
        for visible in [true, false] {
            let mut shown = RailState::idle();
            shown.chart_visible = visible;
            let items = items(&shown);
            let toggle = buttons(&items)[4];
            assert_eq!(toggle.active, visible);
            assert!(matches!(
                toggle.glyph,
                Some(bar::Glyph::Icon(icon))
                    if icon == if visible { Icon::ChartLine } else { Icon::ChartLineFlat }
            ));
        }
    }

    #[test]
    fn the_open_panels_are_lit_and_a_profiler_that_needs_attention_is_tinted() {
        let mut shown = RailState::idle();
        shown.assistant_open = true;
        shown.debugger_open = true;
        shown.profiler = profiler::Status::Partial;
        let lit = items(&shown);
        let panels = buttons(&lit);
        assert!(panels[0].active && panels[1].active && panels[2].active);
        assert!(panels[1].tint.is_some());
        let idle = items(&RailState::idle());
        assert!(!buttons(&idle)[1].active && buttons(&idle)[1].tint.is_none());
    }

    #[test]
    fn each_action_asks_for_its_command() {
        assert!(matches!(
            Cmd::from(RailAction::ToggleAssistant),
            Cmd::ToggleAiPanel
        ));
        assert!(matches!(
            Cmd::from(RailAction::ToggleProfiler),
            Cmd::ProfilerToggle
        ));
        assert!(matches!(
            Cmd::from(RailAction::ToggleDebugger),
            Cmd::ToggleDebugPanel
        ));
        assert!(matches!(
            Cmd::from(RailAction::ToggleChart),
            Cmd::ToggleChart
        ));
        assert!(matches!(
            Cmd::from(RailAction::SetPosition(ChartPosition::Top)),
            Cmd::SetChartPosition(ChartPosition::Top)
        ));
    }

    /// The rail alone in a window, over a body that records it was given the rest of the room.
    struct Harness {
        ctx: egui::Context,
        time: f64,
        body: Option<Rect>,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                ctx: egui::Context::default(),
                time: 0.0,
                body: None,
            }
        }

        fn frame(&mut self, events: Vec<Event>) -> Vec<Cmd> {
            self.time += 0.1;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, WINDOW)),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let cmds = RefCell::new(Vec::new());
            let body = &mut self.body;
            let mut frame = self.ctx.run_ui(raw, |ui| {
                show(ui, &RailState::idle(), &cmds, |ui| {
                    *body = Some(ui.available_rect_before_wrap())
                });
            });
            frame.textures_delta.clear();
            cmds.into_inner()
        }

        fn click(&mut self, at: Pos2) -> Vec<Cmd> {
            let press = |pressed| Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            self.frame(vec![]);
            self.frame(vec![Event::PointerMoved(at)]);
            self.frame(vec![press(true)]);
            let mut cmds = self.frame(vec![press(false)]);
            cmds.extend(self.frame(vec![]));
            cmds
        }
    }

    fn centre_of(n: usize) -> Pos2 {
        button_centre(WINDOW.x, 0.0, n)
    }

    #[test]
    fn the_rail_takes_its_width_off_the_right_and_leaves_the_rest_to_the_editor() {
        let mut harness = Harness::new();
        harness.frame(vec![]);
        let body = harness.body.expect("the editor is drawn");
        assert_eq!(body.right(), WINDOW.x - WIDTH);
        assert_eq!(body.left(), 0.0);
    }

    #[test]
    fn clicking_a_button_pushes_its_command() {
        for (n, expected) in [
            (0, Cmd::ToggleAiPanel),
            (1, Cmd::ProfilerToggle),
            (2, Cmd::ToggleDebugPanel),
            (4, Cmd::ToggleChart),
        ] {
            let cmds = Harness::new().click(centre_of(n));
            assert_eq!(cmds.len(), 1, "button {n}");
            assert_eq!(
                std::mem::discriminant(&cmds[0]),
                std::mem::discriminant(&expected),
                "button {n}"
            );
        }
    }

    #[test]
    fn the_position_menu_opens_left_of_the_rail_and_a_row_sets_the_position() {
        let mut harness = Harness::new();
        let button = centre_of(3);
        assert!(harness.click(button).is_empty());
        assert!(egui::Popup::is_any_open(&harness.ctx));
        // A row of the menu, to the left of the rail and level with its button; the third is
        // "Below Code" whatever the row height.
        let row = pos2(WINDOW.x - WIDTH - 30.0, button.y - 15.0 + 75.0);
        let cmds = harness.click(row);
        assert!(
            matches!(
                cmds.as_slice(),
                [Cmd::SetChartPosition(ChartPosition::Bottom)]
            ),
            "{}",
            cmds.len()
        );
    }
}
