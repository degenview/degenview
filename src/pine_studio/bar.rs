//! The Pine Studio's bar, on the shared flat bar (`ui::kit::bar`) so it is as tall and looks like
//! the chart's toolbar. Left to right: the market and timeframe the script is tried on (the
//! preview chart's own controls, which used to sit over it) and the script button, which opens
//! the script picker. Its right end is empty: the panels and the preview chart's place are on
//! the editor's rail (`rail`).

use super::PineStudio;
use super::library;
use super::preview;
use crate::model::Timeframe;
use crate::ui::icons::Icon;
use crate::ui::kit::bar::{self, Button, Item};
use crate::ui::kit::{BOLD, MEDIUM, SEMIBOLD};
use crate::ui::shortcut;
use eframe::egui::Ui;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BarAction {
    SetTimeframe(Timeframe),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BarMenu {
    Market,
    Scripts,
}

type BarItem = Item<BarAction, BarMenu>;
type BarButton = Button<BarAction, BarMenu>;

/// What the bar shows, from the studio's state.
struct BarState {
    /// The open script's name.
    script_name: Option<String>,
    picker_open: bool,
    chart_visible: bool,
    market: preview::Summary,
}

fn items(state: &BarState) -> (Vec<BarItem>, Vec<BarItem>) {
    // The market and timeframe act on the preview chart: there has to be one on screen.
    let script_open = state.script_name.is_some();
    let preview_shown = script_open && state.chart_visible;

    let mut market = BarButton::new("market", "Choose the market to test on")
        .icon(Icon::Magnifier)
        .label(state.market.title.as_str())
        .enabled(preview_shown)
        .with_menu(BarMenu::Market);
    market.weight = BOLD;
    market.keep_label = true;
    market.suffix = state.market.price.clone();

    let mut left = vec![Item::Button(market), Item::Separator];
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
    left.push(Item::Separator);
    let mut scripts = BarButton::new(
        "scripts",
        shortcut::OPEN_SCRIPT_PICKER.hint("Open a script"),
    )
    .icon(Icon::DocText)
    .label(state.script_name.as_deref().unwrap_or("Scripts"))
    .active(state.picker_open)
    .chevron()
    .with_menu(BarMenu::Scripts);
    scripts.keep_label = true;
    left.push(Item::Button(scripts));
    (left, Vec::new())
}

impl PineStudio {
    pub(super) fn show_bar(&mut self, ui: &mut Ui) {
        let state = BarState {
            script_name: self.open.as_ref().map(|open| open.name.clone()),
            picker_open: self.picker.is_open(),
            chart_visible: self.prefs.chart_visible,
            market: self.preview.summary(),
        };
        let (left, right) = items(&state);
        let preview = &mut self.preview;
        let picker = &mut self.picker;
        let cmds = &mut self.cmds;
        let scripts = &self.scripts;
        let open_id = self.open.as_ref().map(|open| open.id.as_str());
        let actions = bar::show(ui, &left, &right, |ui, menu, response, _| match menu {
            BarMenu::Market => preview::market_picker::market_picker(ui, preview, response),
            BarMenu::Scripts => {
                library::picker::show(ui, picker, scripts, open_id, response, cmds);
            }
        });
        for action in actions {
            match action {
                BarAction::SetTimeframe(timeframe) => self.preview.select_timeframe(timeframe),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::kit::bar::{EDGE, HEIGHT};
    use eframe::egui::{self, Rect, pos2, vec2};

    /// Longer than the bar lets a label be: it is cut, and nothing else gives way.
    const LONG_NAME: &str = "Multi timeframe adaptive volatility breakout with trailing stop";

    fn state(script_open: bool, chart_visible: bool) -> BarState {
        BarState {
            script_name: script_open.then(|| "Trend Bands".to_owned()),
            picker_open: false,
            chart_visible,
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
    fn the_right_end_of_the_bar_is_empty() {
        for open in [true, false] {
            assert!(items(&state(open, true)).1.is_empty());
        }
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

    fn script_button(left: &[BarItem]) -> &BarButton {
        buttons(left)
            .into_iter()
            .find(|b| b.menu == Some(BarMenu::Scripts))
            .expect("a script button")
    }

    #[test]
    fn the_script_button_follows_the_timeframes_and_is_always_enabled() {
        for open in [true, false] {
            let (left, _) = items(&state(open, true));
            let script = script_button(&left);
            assert!(script.enabled, "open: {open}");
            assert!(script.keep_label && script.chevron);
            assert_eq!(
                script.label.as_deref(),
                Some(if open { "Trend Bands" } else { "Scripts" })
            );
            assert!(
                matches!(left.last(), Some(Item::Button(b)) if b.menu == Some(BarMenu::Scripts))
            );
            assert!(matches!(left[left.len() - 2], Item::Separator));
        }
    }

    #[test]
    fn the_script_button_is_lit_while_the_picker_is_up() {
        let mut up = state(true, true);
        up.picker_open = true;
        assert!(script_button(&items(&up).0).active);
        assert!(!script_button(&items(&state(true, true)).0).active);
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
        for (width, name) in [
            (1400.0_f32, "Trend Bands"),
            (960.0, "Trend Bands"),
            (960.0, LONG_NAME),
        ] {
            let mut bar_state = state(true, true);
            bar_state.script_name = Some(name.to_owned());
            let (left, right) = items(&bar_state);
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
            for (width, name) in [
                (1400.0_f32, "Trend Bands"),
                (960.0, "Trend Bands"),
                (960.0, LONG_NAME),
            ] {
                let mut screen = Screen::new(vec2(width, HEIGHT));
                theme::apply(&screen.ctx, app_theme);
                let mut bar_state = state(true, true);
                bar_state.script_name = Some(name.to_owned());
                let (left, right) = items(&bar_state);
                let mut last = None;
                for _ in 0..3 {
                    last = Some(screen.run(Vec::new(), |ui| {
                        bar::show(ui, &left, &right, |_, _, _, _| {});
                    }));
                }
                screen.save(
                    &format!(
                        "pine-bar-{width}{}-{theme_name}",
                        if name == LONG_NAME { "-long" } else { "" }
                    ),
                    last.expect("a frame"),
                );
            }
        }
    }
}
