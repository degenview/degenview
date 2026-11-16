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
use crate::ui::kit::timeframe_picker::{self, TimeframePick};
use crate::ui::kit::{BOLD, SEMIBOLD};
use crate::ui::shortcut;
use eframe::egui::Ui;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BarAction {
    Timeframe(TimeframePick),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BarMenu {
    Market,
    Scripts,
    Timeframe,
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
    /// The custom timeframes pinned in the picker.
    pinned: Vec<Timeframe>,
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
    let mut timeframe = BarButton::new("timeframe", "Timeframe")
        .label(state.market.timeframe.label())
        .chevron()
        .enabled(preview_shown)
        .with_menu(BarMenu::Timeframe);
    timeframe.keep_label = true;
    timeframe.weight = SEMIBOLD;
    timeframe.access = Some(format!(
        "Timeframe: {}. Choose another",
        state.market.timeframe.description()
    ));
    left.push(Item::Button(timeframe));
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
            pinned: self
                .shared
                .lock()
                .map(|store| store.pinned_timeframes_cached().to_vec())
                .unwrap_or_default(),
        };
        let (left, right) = items(&state);
        let preview = &mut self.preview;
        let picker = &mut self.picker;
        let cmds = &mut self.cmds;
        let scripts = &self.scripts;
        let open_id = self.open.as_ref().map(|open| open.id.as_str());
        let actions = bar::show(
            ui,
            &left,
            &right,
            |ui, menu, response, actions| match menu {
                BarMenu::Market => preview::market_picker::market_picker(ui, preview, response),
                BarMenu::Scripts => {
                    library::picker::show(ui, picker, scripts, open_id, response, cmds);
                }
                BarMenu::Timeframe => {
                    let input = timeframe_picker::TimeframePickerInput {
                        current: state.market.timeframe,
                        pinned: &state.pinned,
                        sources: &[state.market.source],
                    };
                    if let Some(pick) = timeframe_picker::show(ui, response, &input) {
                        actions.push(BarAction::Timeframe(pick));
                    }
                }
            },
        );
        for action in actions {
            match action {
                BarAction::Timeframe(TimeframePick::Chart(timeframe)) => {
                    self.preview.select_timeframe(timeframe);
                }
                BarAction::Timeframe(TimeframePick::ChartAndPin(timeframe)) => {
                    self.with_pins(|store| store.pin_timeframe(timeframe));
                    self.preview.select_timeframe(timeframe);
                }
                BarAction::Timeframe(TimeframePick::Unpin(timeframe)) => {
                    self.with_pins(|store| store.unpin_timeframe(timeframe));
                }
            }
        }
    }
}

impl PineStudio {
    /// Changes the pinned timeframes, shared with the chart tabs. A preference: a write that
    /// fails leaves the list as it was.
    fn with_pins(&self, change: impl FnOnce(&crate::store::Store) -> rusqlite::Result<()>) {
        if let Ok(store) = self.shared.lock() {
            let _ = change(&store);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
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
                timeframe: Timeframe::DAY_1,
                source: DataSource::Binance,
            },
            pinned: Vec::new(),
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
    fn the_market_and_timeframe_act_only_on_a_preview_chart_that_is_shown() {
        for (open, visible, enabled) in [
            (true, true, true),
            (true, false, false),
            (false, true, false),
        ] {
            let (left, _) = items(&state(open, visible));
            let preview_controls: Vec<_> = buttons(&left)
                .into_iter()
                .filter(|button| matches!(button.menu, Some(BarMenu::Market | BarMenu::Timeframe)))
                .collect();
            assert_eq!(preview_controls.len(), 2, "the market and the timeframe");
            for button in preview_controls {
                assert_eq!(button.enabled, enabled, "{:?}", button.label);
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
    fn one_button_names_the_current_timeframe_and_opens_the_picker() {
        let (left, _) = items(&state(true, true));
        let timeframes: Vec<_> = buttons(&left)
            .into_iter()
            .filter(|button| button.menu == Some(BarMenu::Timeframe))
            .collect();
        assert_eq!(timeframes.len(), 1);
        let button = timeframes[0];
        assert_eq!(button.label.as_deref(), Some("1D"));
        assert!(button.chevron && button.keep_label);
        assert!(button.action.is_none(), "the picker asks, not the button");
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
