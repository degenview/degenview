//! The chart window's toolbar, on the shared flat bar (`ui::kit::bar`). Left to right: the symbol
//! of the focused chart, the chart type, Indicators, the timeframe, Replay and Alert (the last two
//! icon only), Undo and Redo, in groups divided by separators. On the right: the layout menu,
//! chart settings, the snapshot menu, Portfolio, Pine Studio and the watchlist toggle. The replay
//! bar under it (`replay_bar.rs`) uses the same flat style. The symbol, chart type, Indicators and
//! chart settings act on the focused chart.

use crate::model::{ChartType, DataSource, Timeframe};
use crate::ui::chart::type_menu;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::bar::{self, Glyph};
use crate::ui::kit::{self, BOLD, MEDIUM, REGULAR, SEMIBOLD, size};
use crate::ui::shortcut;
use crate::ui::theme;
use eframe::egui::{self, Color32, Id, Rect, Response, Sense, Ui, Vec2, pos2, vec2};
use std::sync::Arc;

pub struct ToolbarInput<'a> {
    pub timeframe: Timeframe,
    pub layout_name: &'a str,
    pub unsaved: bool,
    pub autosave: AutosaveState,
    /// `(id, name)` of every saved layout, and which one is open.
    pub layouts: &'a [(u64, String)],
    pub active_layout: Option<u64>,
    pub watchlist_visible: bool,
    /// The market the focused chart shows, as its exchange names it; `None` with no chart.
    pub symbol: Option<&'a str>,
    /// The focused market chart the chart type, Indicators and settings buttons act on; `None`
    /// on an empty tab or with a portfolio card focused.
    pub chart: Option<ToolbarChart>,
    pub can_undo: bool,
    pub can_redo: bool,
    pub replay_open: bool,
    /// There is a market to replay (or a replay to close).
    pub can_replay: bool,
    /// What the camera menu needs: whether a snapshot is under way, and what the hosted rows lack.
    pub snapshot: crate::ui::workspace::snapshot_menu::SnapshotMenu<'a>,
}

/// What the toolbar needs of the focused chart.
#[derive(Clone, Copy, Debug)]
pub struct ToolbarChart {
    pub id: u64,
    pub chart_type: ChartType,
    pub source: DataSource,
    /// The market draws types other than its own line (a line-only source does not).
    pub chart_types: bool,
    /// The market has a settings sheet (a CoinMarketCap gauge has none).
    pub settings: bool,
    /// Another market can be drawn on it (not a line-only source or a price-based type).
    pub compare: bool,
    /// The market can carry indicators (a prediction market or a gauge cannot).
    pub indicators: bool,
}

/// What the layout menu's Autosave row shows.
#[derive(Clone, Copy, Debug, Default)]
pub struct AutosaveState {
    /// The tab is on a saved layout; an unnamed tab has nothing to autosave.
    pub available: bool,
    pub on: bool,
    /// The last autosave did not reach disk.
    pub failed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolbarAction {
    SetTimeframe(Timeframe),
    /// Open the symbol picker, to change the focused chart's market.
    OpenSymbolSearch,
    /// Open the symbol picker to compare another market on the focused chart.
    CompareSymbol,
    /// A type picked from the chart-type menu, for the focused chart.
    SetChartType(ChartType),
    SaveLayout,
    ToggleAutosave,
    CopyLayout,
    RenameLayout,
    OpenLayout(u64),
    DeleteLayout(u64),
    ToggleWatchlist,
    OpenPortfolio,
    OpenPineStudio,
    /// Open the indicator picker for the focused chart.
    OpenIndicators,
    /// Open the focused chart's settings sheet.
    OpenChartSettings,
    ToggleReplay,
    /// Start a price alert on the focused chart's market.
    CreateAlert,
    /// Take this snapshot of the focused chart.
    Snapshot(crate::model::chart::snapshot::SnapshotAction),
    /// Open the Snapshots page of Settings.
    SnapshotSettings,
    Undo,
    Redo,
}

/// The tooltip of the Replay button.
pub fn replay_tooltip(replay_open: bool, has_market: bool) -> &'static str {
    if replay_open {
        "Close Replay"
    } else if has_market {
        "Historical Replay"
    } else {
        "Historical Replay — add a market chart first"
    }
}

/// The popup a button owns, drawn once the button is.
#[derive(Clone, Copy, PartialEq)]
enum Menu {
    ChartType,
    Layout,
    Snapshot,
}

/// The popup of a button that owns one: the chart type, the layout and the camera menus.
type Item = bar::Item<ToolbarAction, Menu>;
type Button = bar::Button<ToolbarAction, Menu>;

pub fn show(ui: &mut Ui, input: &ToolbarInput) -> Vec<ToolbarAction> {
    bar::show(
        ui,
        &left_items(input),
        &right_items(input),
        |ui, menu, response, actions| match menu {
            Menu::ChartType => {
                if let Some(chart) = input.chart
                    && let Some(picked) =
                        type_menu::menu(ui, response, chart.source, chart.chart_type, chart.id)
                {
                    actions.push(ToolbarAction::SetChartType(picked));
                }
            }
            Menu::Layout => layout_popup(ui, response, input, actions),
            Menu::Snapshot => {
                match crate::ui::workspace::snapshot_menu::menu(ui, response, &input.snapshot) {
                    Some(crate::ui::workspace::snapshot_menu::Pick::Take(picked)) => {
                        actions.push(ToolbarAction::Snapshot(picked));
                    }
                    Some(crate::ui::workspace::snapshot_menu::Pick::Configure) => {
                        actions.push(ToolbarAction::SnapshotSettings);
                    }
                    None => {}
                }
            }
        },
    )
}

fn left_items(input: &ToolbarInput) -> Vec<Item> {
    let mut items = Vec::new();
    // The market on the chart, as its exchange names it; a click searches for another.
    let mut symbol = Button::new(
        "symbol",
        format!("Search symbol ({})", shortcut::SYMBOL_SEARCH.label()),
    )
    .icon(Icon::Magnifier)
    .label(input.symbol.unwrap_or("Search"))
    .on_click(ToolbarAction::OpenSymbolSearch);
    symbol.weight = BOLD;
    symbol.keep_label = true;
    symbol.access = Some(match input.symbol {
        Some(symbol) => format!("Symbol: {symbol}. Search for another"),
        None => "Search symbol".to_owned(),
    });
    items.push(Item::Button(symbol));
    // Beside the symbol, as TradingView has it: another market on this chart. The Compare page
    // of the chart settings opens the same picker.
    items.push(Item::Button(
        Button::new("compare", "Compare or add symbol")
            .icon(Icon::PlusCircle)
            .enabled(input.chart.is_some_and(|chart| chart.compare))
            .on_click(ToolbarAction::CompareSymbol),
    ));
    items.push(Item::Separator);
    let chart_type = input
        .chart
        .map_or(ChartType::Candles, |chart| chart.chart_type);
    let mut type_button = Button::new("chart-type", format!("Chart type: {}", chart_type.label()))
        .enabled(input.chart.is_some_and(|chart| chart.chart_types));
    type_button.glyph = Some(Glyph::Custom(Arc::new(move |painter, rect, color| {
        type_menu::glyph(painter, rect, chart_type, color);
    })));
    type_button.menu = Some(Menu::ChartType);
    items.push(Item::Button(type_button));
    items.push(Item::Separator);
    let indicators_tip = if input.chart.is_some_and(|chart| !chart.indicators) {
        "Indicators: this chart takes none"
    } else {
        "Indicators: add one to the focused chart"
    };
    items.push(Item::Button(
        Button::new("indicators", indicators_tip)
            .icon(Icon::Waveform)
            .label("Indicators")
            .enabled(input.chart.is_some_and(|chart| chart.indicators))
            .on_click(ToolbarAction::OpenIndicators),
    ));
    items.push(Item::Separator);
    for timeframe in Timeframe::ALL {
        let selected = timeframe == input.timeframe;
        let mut button = Button::new(("timeframe", timeframe.label()), timeframe.description());
        button.label = Some(timeframe.label().to_owned());
        button.quiet = !selected;
        button.active = selected;
        button.weight = if selected { SEMIBOLD } else { MEDIUM };
        button.min_width = 34.0;
        if !selected {
            button.action = Some(ToolbarAction::SetTimeframe(timeframe));
        }
        items.push(Item::Button(button));
    }
    items.push(Item::Separator);

    let mut replay = Button::new(
        "replay",
        replay_tooltip(input.replay_open, input.can_replay),
    )
    .icon(Icon::Rewind)
    .enabled(input.can_replay || input.replay_open)
    .active(input.replay_open)
    .on_click(ToolbarAction::ToggleReplay);
    replay.tint = Some(theme::REPLAY);
    items.push(Item::Button(replay));
    items.push(Item::Button(
        Button::new("alert", "Create price alert")
            .icon(Icon::BellPlus)
            .enabled(
                input
                    .chart
                    .is_some_and(|chart| crate::ui::supports_alerts(chart.source)),
            )
            .on_click(ToolbarAction::CreateAlert),
    ));
    items.push(Item::Separator);

    items.push(Item::Button(
        Button::new("undo", format!("Undo ({})", shortcut::UNDO.label()))
            .icon(Icon::Undo)
            .enabled(input.can_undo)
            .on_click(ToolbarAction::Undo),
    ));
    items.push(Item::Button(
        Button::new("redo", format!("Redo ({})", shortcut::REDO.label()))
            .icon(Icon::Redo)
            .enabled(input.can_redo)
            .on_click(ToolbarAction::Redo),
    ));
    items
}

/// In display order, left to right.
fn right_items(input: &ToolbarInput) -> Vec<Item> {
    let help = if input.autosave.failed {
        format!(
            "{} — Autosave could not save; choose Save layout",
            input.layout_name
        )
    } else if input.unsaved {
        format!("{} — unsaved changes", input.layout_name)
    } else {
        input.layout_name.to_owned()
    };
    let mut layout = Button::new("layout", help.clone());
    layout.label = Some(input.layout_name.to_owned());
    layout.suffix = input.unsaved.then(|| "(unsaved)".to_owned());
    layout.chevron = true;
    layout.menu = Some(Menu::Layout);
    layout.access = Some(format!("Layout: {help}"));

    let mut snapshot = Button::new("snapshot", "Take a snapshot")
        .icon(Icon::Camera)
        .enabled(input.chart.is_some())
        .busy(input.snapshot.working);
    snapshot.menu = Some(Menu::Snapshot);

    let watchlist_tip = if input.watchlist_visible {
        "Hide Watchlists"
    } else {
        "Show Watchlists"
    };
    vec![
        Item::Button(layout),
        Item::Button(
            Button::new("chart-settings", "Chart settings")
                .icon(Icon::Gear)
                .enabled(input.chart.is_some_and(|chart| chart.settings))
                .on_click(ToolbarAction::OpenChartSettings),
        ),
        Item::Button(snapshot),
        Item::Separator,
        Item::Button(
            Button::new("portfolio", "Portfolio Tracker")
                .icon(Icon::Briefcase)
                .on_click(ToolbarAction::OpenPortfolio),
        ),
        Item::Button(
            Button::new("pine-studio", "Pine Studio")
                .icon(Icon::AngleBrackets)
                .on_click(ToolbarAction::OpenPineStudio),
        ),
        Item::Button(
            Button::new("watchlist", watchlist_tip)
                .icon(Icon::List)
                .active(input.watchlist_visible)
                .on_click(ToolbarAction::ToggleWatchlist),
        ),
    ]
}

/// The menu under the layout button: what can be done with the layout.
fn layout_popup(
    ui: &mut Ui,
    response: &Response,
    input: &ToolbarInput,
    actions: &mut Vec<ToolbarAction>,
) {
    // As wide as the fixed rows and the layout names need. A layout row also holds a trash button
    // at its end, which the two em spaces stand in for when measuring.
    let save_hint = shortcut::SAVE.label();
    let mut rows: Vec<(&str, Option<&str>)> = vec![
        ("Save layout", Some(&save_hint)),
        ("Autosave", None),
        ("Make a copy", None),
        ("Rename", None),
    ];
    rows.extend(
        input
            .layouts
            .iter()
            .map(|(_, name)| (name.as_str(), Some("\u{2003}\u{2003}"))),
    );
    kit::fitted_menu_rows(ui, response, egui::RectAlign::BOTTOM_END, &rows, |ui| {
        if kit::menu_item_with_shortcut(
            ui,
            Icon::Download,
            "Save layout",
            Some(&shortcut::SAVE.label()),
            true,
            false,
        ) {
            actions.push(ToolbarAction::SaveLayout);
            ui.close();
        }
        if autosave_row(ui, input.autosave) {
            actions.push(ToolbarAction::ToggleAutosave);
            ui.close();
        }
        kit::menu_separator(ui);
        if kit::menu_item(
            ui,
            Icon::CopyDoc,
            "Make a copy",
            input.active_layout.is_some(),
            false,
        ) {
            actions.push(ToolbarAction::CopyLayout);
            ui.close();
        }
        if kit::menu_item(
            ui,
            Icon::Pencil,
            "Rename",
            input.active_layout.is_some(),
            false,
        ) {
            actions.push(ToolbarAction::RenameLayout);
            ui.close();
        }
        if !input.layouts.is_empty() {
            kit::menu_separator(ui);
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                ui.label(kit::text(
                    "Layouts",
                    size::CAPTION + 0.5,
                    SEMIBOLD,
                    theme::text_tertiary(),
                ));
            });
            ui.add_space(4.0);
            for (id, name) in input.layouts {
                match layout_row(ui, *id, name, input.active_layout == Some(*id)) {
                    Some(LayoutRow::Open) => {
                        actions.push(ToolbarAction::OpenLayout(*id));
                        ui.close();
                    }
                    Some(LayoutRow::Delete) => {
                        actions.push(ToolbarAction::DeleteLayout(*id));
                        ui.close();
                    }
                    None => {}
                }
            }
        }
    });
}

/// The Autosave toggle: a check while it is on, dimmed for a tab with no saved layout, and a
/// warning tint when the last autosave failed.
fn autosave_row(ui: &mut Ui, state: AutosaveState) -> bool {
    let width = ui.available_width();
    let sense = if state.available {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, 24.0), sense);
    let hovered = state.available && response.hovered();
    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, 5.0, theme::selected_tint(true));
    }
    let color = if state.available {
        theme::text_primary()
    } else {
        theme::text_tertiary()
    };
    if state.on {
        icons::paint(
            painter,
            Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), Vec2::splat(11.0)),
            Icon::Check,
            color,
        );
    }
    let label = kit::galley(ui, "Autosave", size::BODY, REGULAR, color, width - 70.0);
    let label_width = label.size().x;
    painter.galley(
        pos2(rect.left() + 32.0, rect.center().y - label.size().y / 2.0),
        label,
        color,
    );
    if state.failed {
        icons::paint(
            painter,
            Rect::from_center_size(
                pos2(rect.left() + 32.0 + label_width + 14.0, rect.center().y),
                Vec2::splat(11.0),
            ),
            Icon::TriangleFill,
            if hovered { Color32::WHITE } else { kit::ORANGE },
        );
    }
    let response = crate::ui::accessibility::label(
        response,
        if state.on {
            "Autosave, on"
        } else {
            "Autosave, off"
        },
    );
    let response = if state.available {
        response.on_hover_text(if state.failed {
            "The last autosave could not be written. Choose Save layout."
        } else {
            "Write changes to this layout a moment after you make them."
        })
    } else {
        response.on_hover_text("Save the layout first, then Autosave can keep it up to date.")
    };
    state.available && response.clicked()
}

enum LayoutRow {
    Open,
    Delete,
}

/// A saved layout in the menu: a check on the open one, the name, and a trash button on hover.
fn layout_row(ui: &mut Ui, id: u64, name: &str, current: bool) -> Option<LayoutRow> {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 24.0), Sense::hover());
    let trash = Rect::from_min_size(
        pos2(rect.right() - 26.0, rect.top() + 2.0),
        Vec2::splat(20.0),
    );
    let row = ui.interact(rect, Id::new(("toolbar-layout-row", id)), Sense::click());
    let delete = ui
        .interact(
            trash,
            Id::new(("toolbar-layout-delete", id)),
            Sense::click(),
        )
        .on_hover_text("Delete this layout");
    let hovered = row.hovered() || delete.hovered();
    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, 5.0, theme::selected_tint(true));
    }
    let color = theme::text_primary();
    if current {
        icons::paint(
            painter,
            Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), Vec2::splat(11.0)),
            Icon::Check,
            color,
        );
    }
    let label = kit::galley(ui, name, size::BODY, REGULAR, color, width - 70.0);
    painter.galley(
        pos2(rect.left() + 32.0, rect.center().y - label.size().y / 2.0),
        label,
        color,
    );
    if hovered {
        if delete.hovered() {
            painter.rect_filled(trash, 5.0, theme::ink(40));
        }
        icons::paint(
            painter,
            Rect::from_center_size(trash.center(), Vec2::splat(11.0)),
            Icon::Trash,
            theme::text_primary(),
        );
    }
    if delete.clicked() {
        Some(LayoutRow::Delete)
    } else if row.clicked() {
        Some(LayoutRow::Open)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::kit::bar::{EDGE, HEIGHT};

    fn run(ctx: &egui::Context, raw: egui::RawInput, body: impl FnMut(&mut Ui)) {
        let mut output = ctx.run_ui(raw, body);
        output.textures_delta.clear();
    }

    fn input<'a>(layouts: &'a [(u64, String)], focused: bool) -> ToolbarInput<'a> {
        ToolbarInput {
            timeframe: Timeframe::Day,
            layout_name: "Morning",
            unsaved: false,
            autosave: AutosaveState::default(),
            layouts,
            active_layout: None,
            watchlist_visible: true,
            symbol: focused.then_some("BTCUSDT"),
            chart: focused.then_some(ToolbarChart {
                id: 1,
                chart_type: ChartType::Candles,
                source: DataSource::Binance,
                chart_types: true,
                settings: true,
                compare: true,
                indicators: true,
            }),
            can_undo: true,
            can_redo: false,
            replay_open: false,
            can_replay: focused,
            snapshot: crate::ui::workspace::snapshot_menu::SnapshotMenu {
                busy: false,
                working: false,
                hosting_unavailable: None,
                shortcut: &|_| None,
            },
        }
    }

    #[test]
    fn the_toolbar_draws_headlessly_with_and_without_a_focused_chart() {
        let layouts = vec![(1, "Morning".to_owned()), (2, "Swing trades".to_owned())];
        let ctx = egui::Context::default();
        for (unsaved, active, focused) in [
            (false, None, false),
            (true, Some(2), true),
            (false, None, true),
        ] {
            let mut input = input(&layouts, focused);
            input.unsaved = unsaved;
            input.active_layout = active;
            input.autosave = AutosaveState {
                available: active.is_some(),
                on: unsaved,
                failed: false,
            };
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert!(show(ui, &input).is_empty(), "nothing was clicked");
            });
            output.textures_delta.clear();
        }
    }

    /// How soon the toolbar, once it has settled, asks for the next frame.
    fn settled_repaint(input: &ToolbarInput) -> std::time::Duration {
        let ctx = egui::Context::default();
        let mut out = None;
        for step in 0..6 {
            let raw = egui::RawInput {
                time: Some(f64::from(step) * 0.1),
                ..screen(1400.0)
            };
            let mut frame = ctx.run_ui(raw, |ui| {
                show(ui, input);
            });
            frame.textures_delta.clear();
            out = Some(frame);
        }
        out.expect("frames")
            .viewport_output
            .values()
            .map(|viewport| viewport.repaint_delay)
            .min()
            .expect("a viewport")
    }

    #[test]
    fn the_camera_runs_while_a_snapshot_is_made_and_the_bar_is_quiet_otherwise() {
        let layouts = vec![(1, "Morning".to_owned())];
        let mut busy = input(&layouts, true);
        busy.snapshot.working = true;
        assert!(
            settled_repaint(&busy) < std::time::Duration::from_millis(100),
            "the candles only scroll if the window keeps painting"
        );
        let idle = input(&layouts, true);
        assert!(
            settled_repaint(&idle) >= std::time::Duration::from_millis(100),
            "an idle bar asks for nothing"
        );
    }

    fn screen(width: f32) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, HEIGHT))),
            ..Default::default()
        }
    }

    /// Every button's x-range, as the toolbar lays them out at `width`, and whether labels showed.
    fn layout_at(
        input: &ToolbarInput,
        width: f32,
    ) -> (bool, Vec<(f32, f32, Option<ToolbarAction>)>) {
        let ctx = egui::Context::default();
        let mut out = (false, Vec::new());
        run(&ctx, screen(width), |ui| {
            let (labels, placed) =
                bar::geometry(ui, &left_items(input), &right_items(input), width);
            out = (
                labels,
                placed
                    .into_iter()
                    .map(|button| (button.from, button.to, button.action))
                    .collect(),
            );
        });
        out
    }

    #[test]
    fn labels_fall_back_to_icons_and_no_two_buttons_overlap() {
        let layouts = Vec::new();
        let input = input(&layouts, true);
        let mut previous_labels = true;
        for width in [1600.0_f32, 1100.0, 900.0, 760.0] {
            let (labels, mut buttons) = layout_at(&input, width);
            buttons.sort_by(|a, b| a.0.total_cmp(&b.0));
            for pair in buttons.windows(2) {
                assert!(pair[0].1 <= pair[1].0, "overlap at {width}: {pair:?}");
            }
            let last = buttons.last().expect("buttons");
            assert!(last.1 <= width - EDGE + 0.5, "past the edge at {width}");
            assert!(
                previous_labels || !labels,
                "labels only go away as the bar narrows (at {width})"
            );
            previous_labels = labels;
            if width >= 1600.0 {
                assert!(labels, "a wide bar has room for the labels");
            }
        }
        assert!(!previous_labels, "760 wide has no room for labels");
    }

    /// Clicks the centre of the button that asks for `target` and returns what the bar asked for.
    fn click_on(input: &ToolbarInput, width: f32, target: ToolbarAction) -> Vec<ToolbarAction> {
        let (_, buttons) = layout_at(input, width);
        let (from, to, _) = *buttons
            .iter()
            .find(|button| button.2 == Some(target))
            .unwrap_or_else(|| panic!("{target:?} is not on the bar at {width}"));
        let at = pos2((from + to) / 2.0, HEIGHT / 2.0);
        let ctx = egui::Context::default();
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let mut asked = Vec::new();
        let mut time = 0.0;
        for events in [
            vec![],
            vec![],
            vec![egui::Event::PointerMoved(at)],
            vec![button(true)],
            vec![button(false)],
            vec![],
        ] {
            time += 0.1;
            let mut raw = screen(width);
            raw.time = Some(time);
            raw.events = events;
            run(&ctx, raw, |ui| asked.extend(show(ui, input)));
        }
        asked
    }

    #[test]
    fn indicators_are_off_on_a_chart_that_takes_none() {
        let layouts = Vec::new();
        let mut input = input(&layouts, true);
        input.chart = input.chart.map(|chart| ToolbarChart {
            indicators: false,
            ..chart
        });
        assert!(click_on(&input, 1400.0, ToolbarAction::OpenIndicators).is_empty());
    }

    #[test]
    fn each_new_button_asks_for_its_action() {
        let layouts = Vec::new();
        let mut input = input(&layouts, true);
        input.can_undo = true;
        input.can_redo = true;
        for target in [
            ToolbarAction::Undo,
            ToolbarAction::Redo,
            ToolbarAction::ToggleReplay,
            ToolbarAction::CreateAlert,
            ToolbarAction::CompareSymbol,
            ToolbarAction::OpenIndicators,
            ToolbarAction::OpenChartSettings,
            ToolbarAction::SetTimeframe(Timeframe::Week),
        ] {
            for width in [1400.0, 960.0] {
                assert_eq!(click_on(&input, width, target), vec![target], "{target:?}");
            }
        }
    }

    #[test]
    fn a_disabled_button_never_asks_for_anything() {
        let layouts = Vec::new();
        // No focused chart, nothing to undo or redo, nothing to replay.
        let mut input = input(&layouts, false);
        input.can_undo = false;
        let ctx = egui::Context::default();
        let mut buttons = Vec::new();
        run(&ctx, screen(1400.0), |ui| {
            let (left, right) = (left_items(&input), right_items(&input));
            for item in left.iter().chain(&right) {
                if let Item::Button(button) = item
                    && !button.enabled
                {
                    buttons.push(button.action);
                }
            }
            let _ = ui;
        });
        let disabled: Vec<ToolbarAction> = buttons.into_iter().flatten().collect();
        for expected in [
            ToolbarAction::Undo,
            ToolbarAction::Redo,
            ToolbarAction::ToggleReplay,
            ToolbarAction::CreateAlert,
            ToolbarAction::CompareSymbol,
            ToolbarAction::OpenIndicators,
            ToolbarAction::OpenChartSettings,
        ] {
            assert!(disabled.contains(&expected), "{expected:?} is not disabled");
        }
        // The chart-type button is disabled too, and has no action of its own.
        for target in disabled {
            assert!(click_on(&input, 1400.0, target).is_empty(), "{target:?}");
        }
    }

    /// Pictures of the bar in the states worth a look: wide, hovered, replaying, narrow, nothing
    /// focused, and with the chart-type menu open.
    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_the_toolbar() {
        use crate::ui::render_tests::{Screen, gallery_dir};
        if gallery_dir().is_none() {
            return;
        }
        let layouts = vec![(1, "Morning".to_owned()), (2, "Swing trades".to_owned())];
        let centre_of = |input: &ToolbarInput, width: f32, target: ToolbarAction| {
            let (_, buttons) = layout_at(input, width);
            let (from, to, _) = *buttons
                .iter()
                .find(|button| button.2 == Some(target))
                .expect("on the bar");
            pos2((from + to) / 2.0, HEIGHT / 2.0)
        };
        type Tweak = fn(&mut ToolbarInput);
        let cases: [(&str, f32, Tweak, Option<ToolbarAction>); 7] = [
            ("toolbar-wide", 1400.0, |_| {}, None),
            (
                "toolbar-hover",
                1400.0,
                |_| {},
                Some(ToolbarAction::OpenIndicators),
            ),
            (
                "toolbar-replay",
                1400.0,
                |input| {
                    input.replay_open = true;
                    input.unsaved = true;
                    input.can_redo = true;
                },
                None,
            ),
            ("toolbar-narrow", 760.0, |_| {}, None),
            (
                "toolbar-empty",
                1400.0,
                |input| {
                    input.chart = None;
                    input.can_undo = false;
                    input.can_replay = false;
                },
                None,
            ),
            ("toolbar-tiny", 900.0, |input| input.unsaved = true, None),
            (
                "toolbar-snapshot-busy",
                1400.0,
                |input| input.snapshot.working = true,
                None,
            ),
        ];
        for (name, width, tweak, hover) in cases {
            let mut input = input(&layouts, true);
            tweak(&mut input);
            let mut screen = Screen::new(vec2(width, HEIGHT));
            let events = hover
                .map(|target| vec![egui::Event::PointerMoved(centre_of(&input, width, target))])
                .unwrap_or_default();
            let mut last = None;
            for frame in 0..3 {
                let events = if frame == 0 {
                    events.clone()
                } else {
                    Vec::new()
                };
                last = Some(screen.run(events, |ui| {
                    show(ui, &input);
                }));
            }
            screen.save(name, last.expect("a frame"));
        }

        // The chart-type menu, opened with a click on its button.
        let input = input(&layouts, true);
        let width = 1400.0;
        let at = centre_of(&input, width, ToolbarAction::OpenIndicators);
        let _ = at;
        let (_, buttons) = layout_at(&input, width);
        // The left run is symbol | chart type | Indicators | timeframes | Replay, Alert | undo, redo; a
        // separator is the only gap wider than a button gap.
        let left_run: Vec<_> = buttons.iter().filter(|button| button.0 < 700.0).collect();
        let starts: Vec<usize> = std::iter::once(0)
            .chain(
                left_run
                    .windows(2)
                    .enumerate()
                    .filter(|(_, pair)| pair[1].0 - pair[0].1 > 8.0)
                    .map(|(at, _)| at + 1),
            )
            .collect();
        let type_button = *left_run[starts[1]];
        assert!(
            type_button.2.is_none(),
            "the chart-type button opens a menu"
        );
        let at = pos2((type_button.0 + type_button.1) / 2.0, HEIGHT / 2.0);
        let click = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let mut screen = Screen::new(vec2(width, 540.0));
        let mut last = None;
        for events in [
            vec![egui::Event::PointerMoved(at)],
            vec![click(true)],
            vec![click(false)],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ] {
            last = Some(screen.run(events, |ui| {
                let bar = Rect::from_min_size(pos2(0.0, 0.0), vec2(width, HEIGHT));
                let mut bar_ui = ui.new_child(egui::UiBuilder::new().max_rect(bar));
                show(&mut bar_ui, &input);
            }));
        }
        screen.save("toolbar-chart-type-menu", last.expect("a frame"));
    }
}
