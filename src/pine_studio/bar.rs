//! The Pine Studio's bar, on the shared flat bar (`ui::kit::bar`) so it is as tall and looks like
//! the chart's toolbar. Left to right: the script list and the preview chart's place and
//! visibility; the market and timeframe the script is tried on (the preview chart's own controls,
//! which used to sit over it); and at the right end the assistant, profiler and debugger, for an
//! open script.

use super::commands::Cmd;
use super::library::prefs::ChartPosition;
use super::preview;
use super::{PineStudio, ai, debug, profiler};
use crate::model::Timeframe;
use crate::ui::icons::Icon;
use crate::ui::kit::bar::{self, Button, Item};
use crate::ui::kit::header_controls;
use crate::ui::kit::{self, BOLD, MEDIUM, SEMIBOLD};
use crate::ui::shortcut;
use eframe::egui::{self, Response, Ui};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BarAction {
    ToggleSidebar,
    ToggleChart,
    SetPosition(ChartPosition),
    SetTimeframe(Timeframe),
    ToggleDebugger,
    ToggleAssistant,
    ToggleProfiler,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BarMenu {
    ChartPosition,
    Market,
}

type BarItem = Item<BarAction, BarMenu>;
type BarButton = Button<BarAction, BarMenu>;

/// What the bar shows, from the studio's state.
struct BarState {
    sidebar_visible: bool,
    chart_visible: bool,
    chart_position: ChartPosition,
    script_open: bool,
    debugger_open: bool,
    assistant_open: bool,
    profiler: profiler::Status,
    market: preview::Summary,
}

fn position_icon(position: ChartPosition) -> Icon {
    match position {
        ChartPosition::Left => Icon::RectLeft,
        ChartPosition::Top => Icon::RectTop,
        ChartPosition::Bottom => Icon::RectBottom,
    }
}

fn items(state: &BarState) -> (Vec<BarItem>, Vec<BarItem>) {
    let script_list = shortcut::TOGGLE_SCRIPT_LIST.hint(if state.sidebar_visible {
        "Hide the script list"
    } else {
        "Show the script list"
    });
    let preview_toggle = shortcut::TOGGLE_PREVIEW_CHART.hint(if state.chart_visible {
        "Hide the preview chart"
    } else {
        "Show the preview chart"
    });
    // The market and timeframe act on the preview chart: there has to be one on screen.
    let preview_shown = state.script_open && state.chart_visible;

    let mut market = BarButton::new("market", "Choose the market to test on")
        .icon(Icon::Magnifier)
        .label(state.market.title.as_str())
        .enabled(preview_shown)
        .with_menu(BarMenu::Market);
    market.weight = BOLD;
    market.keep_label = true;
    market.suffix = state.market.price.clone();

    let mut left = vec![
        Item::Button(
            BarButton::new("script-list", script_list)
                .icon(Icon::SidebarLeft)
                .active(state.sidebar_visible)
                .on_click(BarAction::ToggleSidebar),
        ),
        Item::Button(
            BarButton::new(
                "chart-position",
                "Where the preview chart sits next to the code",
            )
            .icon(position_icon(state.chart_position))
            .with_menu(BarMenu::ChartPosition),
        ),
        Item::Button(
            BarButton::new("preview-chart", preview_toggle)
                .icon(if state.chart_visible {
                    Icon::ChartLine
                } else {
                    Icon::ChartLineFlat
                })
                .active(state.chart_visible)
                .on_click(BarAction::ToggleChart),
        ),
        Item::Separator,
        Item::Button(market),
        Item::Separator,
    ];
    for timeframe in Timeframe::ALL {
        let selected = timeframe == state.market.timeframe;
        let mut button = BarButton::new(("timeframe", timeframe.label()), timeframe.description())
            .label(timeframe.label())
            .enabled(preview_shown)
            .active(selected);
        button.quiet = !selected;
        button.weight = if selected { SEMIBOLD } else { MEDIUM };
        button.min_width = 34.0;
        if !selected {
            button.action = Some(BarAction::SetTimeframe(timeframe));
        }
        left.push(Item::Button(button));
    }

    let mut right = Vec::new();
    if state.script_open {
        right.push(Item::Button(ai::panel::panel_button(
            state.assistant_open,
            BarAction::ToggleAssistant,
        )));
        right.push(Item::Button(profiler::ui::toolbar_button(
            &state.profiler,
            BarAction::ToggleProfiler,
        )));
        right.push(Item::Button(debug::panel::panel_button(
            state.debugger_open,
            BarAction::ToggleDebugger,
        )));
    }
    (left, right)
}

impl PineStudio {
    pub(super) fn show_bar(&mut self, ui: &mut Ui) {
        let state = BarState {
            sidebar_visible: self.prefs.sidebar_visible,
            chart_visible: self.prefs.chart_visible,
            chart_position: self.prefs.chart_position,
            script_open: self.open.is_some(),
            debugger_open: self.prefs.debug_open,
            assistant_open: self.prefs.ai_open,
            profiler: self.profiler.status(self.profiler_now.as_ref()),
            market: self.preview.summary(),
        };
        let (left, right) = items(&state);
        let preview = &mut self.preview;
        let actions = bar::show(
            ui,
            &left,
            &right,
            |ui, menu, response, actions| match menu {
                BarMenu::ChartPosition => {
                    position_menu(ui, response, state.chart_position, actions);
                }
                BarMenu::Market => preview::market_picker(ui, preview, response),
            },
        );
        for action in actions {
            match action {
                BarAction::ToggleSidebar => self.cmds.push(Cmd::ToggleSidebar),
                BarAction::ToggleChart => self.cmds.push(Cmd::ToggleChart),
                BarAction::SetPosition(position) => {
                    self.cmds.push(Cmd::SetChartPosition(position));
                }
                BarAction::SetTimeframe(timeframe) => self.preview.select_timeframe(timeframe),
                BarAction::ToggleDebugger => self.cmds.push(Cmd::ToggleDebugPanel),
                BarAction::ToggleAssistant => self.cmds.push(Cmd::ToggleAiPanel),
                BarAction::ToggleProfiler => self.cmds.push(Cmd::ProfilerToggle),
            }
        }
    }
}

fn position_menu(
    ui: &mut Ui,
    response: &Response,
    current: ChartPosition,
    actions: &mut Vec<BarAction>,
) {
    let labels: Vec<&str> = ChartPosition::ALL.iter().map(|p| p.title()).collect();
    kit::fitted_menu(ui, response, egui::RectAlign::BOTTOM_START, &labels, |ui| {
        for position in ChartPosition::ALL {
            if header_controls::choice(ui, position.title(), position == current) {
                actions.push(BarAction::SetPosition(position));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::kit::bar::{EDGE, HEIGHT};
    use eframe::egui::{Rect, pos2, vec2};

    fn state(script_open: bool, chart_visible: bool) -> BarState {
        BarState {
            sidebar_visible: true,
            chart_visible,
            chart_position: ChartPosition::Left,
            script_open,
            debugger_open: false,
            assistant_open: false,
            profiler: profiler::Status::Off,
            market: preview::Summary {
                title: "BTCUSDT".to_owned(),
                price: Some("64,210.50  +1.20%".to_owned()),
                timeframe: Timeframe::Day,
            },
        }
    }

    fn buttons(items: &[BarItem]) -> Vec<&BarButton> {
        items
            .iter()
            .filter_map(|item| match item {
                Item::Button(button) => Some(button),
                Item::Separator => None,
            })
            .collect()
    }

    #[test]
    fn the_assistant_profiler_and_debugger_need_an_open_script() {
        let (_, none) = items(&state(false, true));
        assert!(none.is_empty());
        let (_, some) = items(&state(true, true));
        let actions: Vec<_> = buttons(&some).iter().map(|b| b.action).collect();
        assert_eq!(
            actions,
            vec![
                Some(BarAction::ToggleAssistant),
                Some(BarAction::ToggleProfiler),
                Some(BarAction::ToggleDebugger)
            ]
        );
    }

    #[test]
    fn the_market_and_timeframes_act_only_on_a_preview_chart_that_is_shown() {
        for (open, visible, enabled) in [
            (true, true, true),
            (true, false, false),
            (false, true, false),
        ] {
            let (left, _) = items(&state(open, visible));
            for button in buttons(&left) {
                let is_preview_control = button.menu == Some(BarMenu::Market)
                    || matches!(button.action, Some(BarAction::SetTimeframe(_)))
                    || button
                        .label
                        .as_deref()
                        .is_some_and(|l| Timeframe::ALL.iter().any(|t| t.label() == l));
                if is_preview_control {
                    assert_eq!(button.enabled, enabled, "{:?}", button.label);
                }
            }
        }
    }

    #[test]
    fn the_current_timeframe_is_lit_and_asks_for_nothing() {
        let (left, _) = items(&state(true, true));
        let timeframes: Vec<_> = buttons(&left)
            .into_iter()
            .filter(|b| b.label.as_deref() == Some(Timeframe::Day.label()))
            .collect();
        assert_eq!(timeframes.len(), 1);
        assert!(timeframes[0].active && timeframes[0].action.is_none());
        let week = buttons(&left)
            .into_iter()
            .find(|b| b.label.as_deref() == Some(Timeframe::Week.label()))
            .expect("a week button");
        assert_eq!(week.action, Some(BarAction::SetTimeframe(Timeframe::Week)));
    }

    #[test]
    fn the_market_button_shows_the_price_after_its_name() {
        let (left, _) = items(&state(true, true));
        let market = buttons(&left)
            .into_iter()
            .find(|b| b.menu == Some(BarMenu::Market))
            .expect("a market button");
        assert_eq!(market.label.as_deref(), Some("BTCUSDT"));
        assert_eq!(market.suffix.as_deref(), Some("64,210.50  +1.20%"));
    }

    #[test]
    fn the_bar_fits_the_narrowest_window_without_overlap() {
        let ctx = egui::Context::default();
        for width in [1400.0_f32, 960.0] {
            let (left, right) = items(&state(true, true));
            let mut placed = Vec::new();
            let mut frame = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, HEIGHT))),
                    ..Default::default()
                },
                |ui| placed = bar::geometry(ui, &left, &right, width).1,
            );
            frame.textures_delta.clear();
            placed.sort_by(|a, b| a.from.total_cmp(&b.from));
            for pair in placed.windows(2) {
                assert!(pair[0].to <= pair[1].from, "overlap at {width}");
            }
            assert!(placed.last().expect("buttons").to <= width - EDGE + 0.5);
            assert_eq!(
                placed.len(),
                buttons(&left).len() + buttons(&right).len(),
                "nothing is dropped at {width}"
            );
        }
    }

    /// Pictures of the bar at two widths in both themes, to compare with the chart toolbar's.
    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_the_bar() {
        use crate::ui::render_tests::{Screen, gallery_dir};
        use crate::ui::theme;
        if gallery_dir().is_none() {
            return;
        }
        let _theme = theme::RestoreDefaultTheme;
        for (theme_name, app_theme) in [
            ("dark", theme::AppTheme::Dark),
            ("light", theme::AppTheme::Light),
        ] {
            for width in [1400.0_f32, 960.0] {
                let mut screen = Screen::new(vec2(width, HEIGHT));
                theme::apply(&screen.ctx, app_theme);
                let (left, right) = items(&state(true, true));
                let mut last = None;
                for _ in 0..3 {
                    last = Some(screen.run(Vec::new(), |ui| {
                        bar::show(ui, &left, &right, |_, _, _, _| {});
                    }));
                }
                screen.save(
                    &format!("pine-bar-{width}-{theme_name}"),
                    last.expect("a frame"),
                );
            }
        }
    }
}
