//! The chart window's toolbar, flat like TradingView's and LuxAlgo's: borderless buttons that
//! tint on hover, hairline separators between groups and labels that fall back to icons when the
//! window is too narrow for them. Left to right: the timeframe, the chart type, Indicators,
//! Replay, Undo and Redo. On the right: the layout menu, chart settings, Portfolio, Pine Studio,
//! the watchlist toggle and Add Chart. The replay bar under it (`replay_bar.rs`) uses the same
//! flat style. The chart type, Indicators and chart settings act on the
//! focused chart.

use crate::model::{ChartType, DataSource, Timeframe};
use crate::ui::chart_type_menu;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD, size};
use crate::ui::shortcut;
use crate::ui::theme;
use eframe::egui::{self, Color32, Galley, Id, Rect, Response, Sense, Ui, Vec2, pos2, vec2};
use std::sync::Arc;

pub const HEIGHT: f32 = 44.0;
/// A button's height.
const BUTTON: f32 = 30.0;
const RADIUS: f32 = 6.0;
const ICON: f32 = 16.0;
/// Space inside a button, either side of its content.
const PAD: f32 = 8.0;
/// Between a button's icon and its label.
const ICON_GAP: f32 = 6.0;
/// Between two buttons of one group.
const GAP: f32 = 2.0;
/// Either side of a separator's hairline.
const SEPARATOR_MARGIN: f32 = 7.0;
const SEPARATOR_HEIGHT: f32 = 18.0;
const EDGE: f32 = 10.0;
/// The most the two clusters may crowd each other before the right one starts to drop buttons.
const CLUSTER_GAP: f32 = 16.0;
/// Long layout names are cut so the toolbar keeps its shape.
const NAME_MAX_WIDTH: f32 = 180.0;
const LABEL_SIZE: f32 = size::BODY;

pub struct ToolbarInput<'a> {
    pub timeframe: Timeframe,
    pub layout_name: &'a str,
    pub unsaved: bool,
    pub autosave: AutosaveState,
    /// `(id, name)` of every saved layout, and which one is open.
    pub layouts: &'a [(u64, String)],
    pub active_layout: Option<u64>,
    pub watchlist_visible: bool,
    /// The focused market chart the chart type, Indicators and settings buttons act on; `None`
    /// on an empty tab or with a portfolio card focused.
    pub chart: Option<ToolbarChart>,
    pub can_undo: bool,
    pub can_redo: bool,
    pub replay_open: bool,
    /// There is a market to replay (or a replay to close).
    pub can_replay: bool,
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
    AddChart,
    /// Open the indicator picker for the focused chart.
    OpenIndicators,
    /// Open the focused chart's settings sheet.
    OpenChartSettings,
    ToggleReplay,
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

/// What a button draws before its label.
#[derive(Clone, Copy)]
enum Glyph {
    Icon(Icon),
    ChartType(ChartType),
}

/// The popup a button owns, drawn once the button is.
#[derive(Clone, Copy, PartialEq)]
enum Menu {
    ChartType,
    Layout,
}

struct Button {
    id: Id,
    glyph: Option<Glyph>,
    label: Option<String>,
    /// Quieter text after the label.
    suffix: Option<String>,
    chevron: bool,
    tip: String,
    /// What assistive tech reads, when it differs from the tooltip.
    access: Option<String>,
    enabled: bool,
    /// Lit: the thing it toggles is on.
    active: bool,
    /// The colour of its content while lit.
    tint: Option<Color32>,
    /// Quiet text until hovered or lit (a timeframe that is not the current one).
    quiet: bool,
    weight: f32,
    min_width: f32,
    menu: Option<Menu>,
    action: Option<ToolbarAction>,
}

impl Button {
    fn new(id: impl std::hash::Hash + std::fmt::Debug, tip: impl Into<String>) -> Self {
        Self {
            id: Id::new(("toolbar", id)),
            glyph: None,
            label: None,
            suffix: None,
            chevron: false,
            tip: tip.into(),
            access: None,
            enabled: true,
            active: false,
            tint: None,
            quiet: false,
            weight: MEDIUM,
            min_width: 0.0,
            menu: None,
            action: None,
        }
    }

    fn icon(mut self, icon: Icon) -> Self {
        self.glyph = Some(Glyph::Icon(icon));
        self
    }

    /// Text beside the glyph; it is dropped before the glyph when space runs out. A button with
    /// no glyph always keeps its text.
    fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    fn on_click(mut self, action: ToolbarAction) -> Self {
        self.action = Some(action);
        self
    }

    fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }
}

enum Item {
    Button(Button),
    Separator,
}

/// A button measured for one frame: its label is laid out and its width known.
struct Measured<'a> {
    button: &'a Button,
    label: Option<Arc<Galley>>,
    suffix: Option<Arc<Galley>>,
    width: f32,
}

enum Slot<'a> {
    Button(Measured<'a>),
    Separator,
}

impl Slot<'_> {
    /// The room the slot takes, without the gap after it.
    fn span(&self) -> f32 {
        match self {
            Slot::Button(measured) => measured.width,
            Slot::Separator => 1.0 + 2.0 * SEPARATOR_MARGIN,
        }
    }

    fn gap_after(&self) -> f32 {
        match self {
            Slot::Button(_) => GAP,
            Slot::Separator => 0.0,
        }
    }
}

fn measure<'a>(ui: &Ui, items: &'a [Item], labels: bool) -> Vec<Slot<'a>> {
    items
        .iter()
        .map(|item| match item {
            Item::Separator => Slot::Separator,
            Item::Button(button) => {
                let shown = button.glyph.is_none() || labels;
                let text = |value: &Option<String>, weight: f32| {
                    value.as_ref().filter(|_| shown).map(|value| {
                        // The colour is chosen when the button is painted.
                        kit::galley(
                            ui,
                            value,
                            LABEL_SIZE,
                            weight,
                            Color32::PLACEHOLDER,
                            NAME_MAX_WIDTH,
                        )
                    })
                };
                let label = text(&button.label, button.weight);
                let suffix = text(&button.suffix, REGULAR);
                let content = content_width(button, label.as_deref(), suffix.as_deref());
                Slot::Button(Measured {
                    button,
                    label,
                    suffix,
                    width: (content + 2.0 * PAD).max(button.min_width),
                })
            }
        })
        .collect()
}

fn content_width(button: &Button, label: Option<&Galley>, suffix: Option<&Galley>) -> f32 {
    let mut width = 0.0;
    if button.glyph.is_some() {
        width += ICON;
    }
    if let Some(label) = label {
        width += label.size().x
            + if button.glyph.is_some() {
                ICON_GAP
            } else {
                0.0
            };
        if let Some(suffix) = suffix {
            width += 4.0 + suffix.size().x;
        }
        if button.chevron {
            width += 4.0 + 9.0;
        }
    } else if button.chevron {
        width += 9.0;
    }
    width
}

fn run_width(slots: &[Slot]) -> f32 {
    slots
        .iter()
        .map(|slot| slot.span() + slot.gap_after())
        .sum()
}

pub fn show(ui: &mut Ui, input: &ToolbarInput) -> Vec<ToolbarAction> {
    let mut actions = Vec::new();
    let bar = ui.max_rect();
    ui.painter().rect_filled(bar, 0.0, theme::chrome());
    let y = bar.center().y - BUTTON / 2.0;

    let left_items = left_items(input);
    let right_items = right_items(input);
    // Labels go first when the two ends meet; then the right end drops what does not fit.
    let wide = (
        measure(ui, &left_items, true),
        measure(ui, &right_items, true),
    );
    let (left, right) = if fits(&wide.0, &wide.1, bar.width()) {
        wide
    } else {
        (
            measure(ui, &left_items, false),
            measure(ui, &right_items, false),
        )
    };
    for (x, slot) in arrange(bar.width(), &left, &right) {
        paint_slot(ui, slot, bar.left() + x, y, input, &mut actions);
    }
    actions
}

/// Whether both runs fit side by side in a bar `width` wide.
fn fits(left: &[Slot], right: &[Slot], width: f32) -> bool {
    run_width(left) + run_width(right) <= width - 2.0 * EDGE - CLUSTER_GAP
}

/// Where each slot starts, from the bar's left edge: the left run from the edge, the right run
/// from the other edge inwards, so what is dropped for lack of room is the right run's first slot.
fn arrange<'a, 'b>(
    width: f32,
    left: &'b [Slot<'a>],
    right: &'b [Slot<'a>],
) -> Vec<(f32, &'b Slot<'a>)> {
    let mut placed = Vec::new();
    let mut x = EDGE;
    for slot in left {
        placed.push((x, slot));
        x += slot.span() + slot.gap_after();
    }
    let mut edge = width - EDGE;
    for slot in right.iter().rev() {
        let at = edge - slot.span();
        if at < x + CLUSTER_GAP {
            break;
        }
        placed.push((at, slot));
        edge = at - slot.gap_after();
    }
    placed
}

fn left_items(input: &ToolbarInput) -> Vec<Item> {
    let mut items = Vec::new();
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

    let has_chart = input.chart.is_some();
    let chart_type = input
        .chart
        .map_or(ChartType::Candles, |chart| chart.chart_type);
    let mut type_button = Button::new("chart-type", format!("Chart type: {}", chart_type.label()))
        .enabled(input.chart.is_some_and(|chart| chart.chart_types));
    type_button.glyph = Some(Glyph::ChartType(chart_type));
    type_button.menu = Some(Menu::ChartType);
    items.push(Item::Button(type_button));
    items.push(Item::Button(
        Button::new("indicators", "Indicators: add one to the focused chart")
            .icon(Icon::Waveform)
            .label("Indicators")
            .enabled(has_chart)
            .on_click(ToolbarAction::OpenIndicators),
    ));
    let mut replay = Button::new(
        "replay",
        replay_tooltip(input.replay_open, input.can_replay),
    )
    .icon(Icon::Rewind)
    .label("Replay")
    .enabled(input.can_replay || input.replay_open)
    .active(input.replay_open)
    .on_click(ToolbarAction::ToggleReplay);
    replay.tint = Some(theme::REPLAY);
    items.push(Item::Button(replay));
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
        Item::Separator,
        Item::Button(
            Button::new("portfolio", "Portfolio Tracker")
                .icon(Icon::Briefcase)
                .on_click(ToolbarAction::OpenPortfolio),
        ),
        Item::Button(
            Button::new("pine-studio", "Pine Studio")
                .icon(Icon::Curlybraces)
                .on_click(ToolbarAction::OpenPineStudio),
        ),
        Item::Button(
            Button::new("watchlist", watchlist_tip)
                .icon(Icon::SidebarRight)
                .active(input.watchlist_visible)
                .on_click(ToolbarAction::ToggleWatchlist),
        ),
        Item::Separator,
        Item::Button(
            Button::new("add-chart", "Add Chart")
                .icon(Icon::Plus)
                .label("Add Chart")
                .on_click(ToolbarAction::AddChart),
        ),
    ]
}

fn paint_slot(
    ui: &mut Ui,
    slot: &Slot,
    x: f32,
    y: f32,
    input: &ToolbarInput,
    actions: &mut Vec<ToolbarAction>,
) {
    let measured = match slot {
        Slot::Separator => {
            ui.painter().rect_filled(
                Rect::from_center_size(
                    pos2(x + SEPARATOR_MARGIN + 0.5, y + BUTTON / 2.0),
                    vec2(1.0, SEPARATOR_HEIGHT),
                ),
                0.0,
                theme::separator(),
            );
            return;
        }
        Slot::Button(measured) => measured,
    };
    let response = paint_button(ui, measured, x, y);
    let button = measured.button;
    if response.clicked()
        && button.enabled
        && let Some(action) = button.action
    {
        actions.push(action);
    }
    match button.menu {
        Some(Menu::ChartType) => {
            if let Some(chart) = input.chart
                && let Some(picked) =
                    chart_type_menu::menu(ui, &response, chart.source, chart.chart_type, chart.id)
            {
                actions.push(ToolbarAction::SetChartType(picked));
            }
        }
        Some(Menu::Layout) => layout_popup(ui, &response, input, actions),
        None => {}
    }
}

/// One flat button: nothing at rest, a faint tint on hover, a stronger one while pressed or lit.
fn paint_button(ui: &mut Ui, measured: &Measured, x: f32, y: f32) -> Response {
    let button = measured.button;
    let rect = Rect::from_min_size(pos2(x, y), vec2(measured.width, BUTTON));
    let sense = if button.enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, button.id, sense);
    let tip = if button.tip.is_empty() {
        button.label.clone().unwrap_or_default()
    } else {
        button.tip.clone()
    };
    let response = crate::ui::accessibility::label(
        response.on_hover_text(&tip),
        button.access.clone().unwrap_or(tip),
    );
    let menu_open = button.menu.is_some()
        && egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    let lit = button.active || menu_open;
    let hovered = button.enabled && response.hovered();
    let pressed = button.enabled && response.is_pointer_button_down_on();
    let tint = if pressed {
        0.16
    } else if lit && hovered {
        0.15
    } else if hovered {
        0.09
    } else if lit {
        0.11
    } else {
        0.0
    };
    let painter = ui.painter();
    if tint > 0.0 {
        painter.rect_filled(rect, RADIUS, kit::primary(tint));
    }
    let color = if !button.enabled {
        theme::text_tertiary()
    } else if let (Some(tint), true) = (button.tint, lit) {
        tint
    } else if button.quiet && !hovered && !lit {
        theme::text_secondary()
    } else {
        theme::text_primary()
    };
    let content = content_width(
        button,
        measured.label.as_deref(),
        measured.suffix.as_deref(),
    );
    let mut at = rect.center().x - content / 2.0;
    let middle = rect.center().y;
    if let Some(glyph) = button.glyph {
        let glyph_rect = Rect::from_min_size(pos2(at, middle - ICON / 2.0), Vec2::splat(ICON));
        match glyph {
            Glyph::Icon(icon) => icons::paint(painter, glyph_rect, icon, color),
            Glyph::ChartType(kind) => chart_type_menu::glyph(painter, glyph_rect, kind, color),
        }
        at += ICON
            + if measured.label.is_some() {
                ICON_GAP
            } else {
                0.0
            };
    }
    if let Some(label) = &measured.label {
        let width = label.size().x;
        painter.galley(
            pos2(at, middle - label.size().y / 2.0),
            label.clone(),
            color,
        );
        at += width;
        if let Some(suffix) = &measured.suffix {
            at += 4.0;
            painter.galley(
                pos2(at, middle - suffix.size().y / 2.0),
                suffix.clone(),
                if button.enabled {
                    theme::text_secondary()
                } else {
                    theme::text_tertiary()
                },
            );
            at += suffix.size().x;
        }
        if button.chevron {
            at += 4.0;
        }
    }
    if button.chevron {
        icons::paint(
            painter,
            Rect::from_min_size(pos2(at, middle - 4.5), Vec2::splat(9.0)),
            Icon::ChevronDown,
            theme::text_secondary(),
        );
    }
    response
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
        painter.rect_filled(rect, 5.0, kit::ACCENT);
    }
    let color = match (state.available, hovered) {
        (false, _) => theme::text_tertiary(),
        (true, true) => Color32::WHITE,
        (true, false) => theme::text_primary(),
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
        painter.rect_filled(rect, 5.0, kit::ACCENT);
    }
    let color = if hovered {
        Color32::WHITE
    } else {
        theme::text_primary()
    };
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
            Color32::WHITE,
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
            chart: focused.then_some(ToolbarChart {
                id: 1,
                chart_type: ChartType::Candles,
                source: DataSource::Binance,
                chart_types: true,
                settings: true,
            }),
            can_undo: true,
            can_redo: false,
            replay_open: false,
            can_replay: focused,
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
            let (left, right) = (left_items(input), right_items(input));
            let wide = (measure(ui, &left, true), measure(ui, &right, true));
            let labels = fits(&wide.0, &wide.1, width);
            let (left, right) = if labels {
                wide
            } else {
                (measure(ui, &left, false), measure(ui, &right, false))
            };
            out = (
                labels,
                arrange(width, &left, &right)
                    .into_iter()
                    .filter_map(|(x, slot)| match slot {
                        Slot::Button(measured) => {
                            Some((x, x + measured.width, measured.button.action))
                        }
                        Slot::Separator => None,
                    })
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
            // The minimum window keeps the layout menu and Add Chart.
            if width >= 760.0 {
                assert!(
                    buttons.iter().any(|b| b.2 == Some(ToolbarAction::AddChart)),
                    "Add Chart is kept at {width}"
                );
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
            .expect("the button is on the bar");
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
    fn each_new_button_asks_for_its_action() {
        let layouts = Vec::new();
        let mut input = input(&layouts, true);
        input.can_undo = true;
        input.can_redo = true;
        for target in [
            ToolbarAction::Undo,
            ToolbarAction::Redo,
            ToolbarAction::ToggleReplay,
            ToolbarAction::OpenIndicators,
            ToolbarAction::OpenChartSettings,
            ToolbarAction::SetTimeframe(Timeframe::Week),
            ToolbarAction::AddChart,
        ] {
            for width in [1400.0, 800.0] {
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
        let cases: [(&str, f32, Tweak, Option<ToolbarAction>); 6] = [
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
        let type_button = buttons
            .iter()
            .find(|button| button.2.is_none() && button.0 < 300.0 && button.0 > 200.0)
            .copied()
            .expect("the chart-type button sits after the timeframes");
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
