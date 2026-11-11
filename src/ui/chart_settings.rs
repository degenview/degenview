//! The chart settings sheet: Indicators, Scripts, Appearance and Canvas above "Remove Chart",
//! the Templates menu and "Done". Every control writes through to the chart as it changes.

use super::chart_card::ChartCard;
use super::chart_settings_compare::{self, CompareInput};
use super::chart_templates::{self, TemplateAction, TemplateState};
use super::icons::{self, Icon};
use super::kit::{self, TabItem};
use super::script_catalog;
use super::settings_card;
use super::sheet;
use super::theme;
use crate::model::chart::price_format::MAX_DECIMALS;
use crate::model::chart::price_scale::PriceScaleMode;
use crate::model::pine::instance::{PineInstance, VisualOrder};
use crate::model::{
    BOLLINGER_MULTIPLIER, BOLLINGER_PERIOD, ChartAppearance, DataSource, EMA_PERIODS,
    IndicatorConfig, PriceFormat, RSI_OVERBOUGHT, RSI_OVERSOLD, RSI_PERIOD, SUPERTREND_MULTIPLIER,
    SUPERTREND_PERIOD, SmallPrices,
};
use crate::pine_studio::drawer;
use crate::pine_studio::model::{LocalScript, ScriptType};
use crate::store::SharedStore;
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Id, Layout, Margin, Rect,
    RichText, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use std::sync::Arc;

/// Swift's ideal sheet width.
const WIDTH: f32 = 760.0;
/// The scrolling page's height in a tall window; enough to show most Appearance and Canvas
/// sections at once, while still leaving room around the sheet.
const MAX_PAGE_HEIGHT: f32 = 650.0;
const MIN_PAGE_HEIGHT: f32 = 260.0;
/// What the sheet spends on everything but the page: header, tab bar, two rules and the footer.
const CHROME_HEIGHT: f32 = 250.0;

fn page_height(parent_height: f32) -> f32 {
    (parent_height - CHROME_HEIGHT).clamp(MIN_PAGE_HEIGHT, MAX_PAGE_HEIGHT)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Tab {
    #[default]
    Indicators,
    Scripts,
    Appearance,
    Canvas,
    /// The markets compared on the chart.
    Overlays,
}

impl Tab {
    const ALL: [Self; 5] = [
        Self::Indicators,
        Self::Scripts,
        Self::Overlays,
        Self::Appearance,
        Self::Canvas,
    ];

    /// The pages a chart from `source` offers: CoinMarketCap's indices take no indicators or
    /// scripts.
    fn for_source(source: DataSource) -> &'static [Self] {
        if source == DataSource::CoinMarketCap {
            &[Self::Appearance, Self::Canvas]
        } else {
            &Self::ALL
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Indicators => "Indicators",
            Self::Scripts => "Scripts",
            Self::Appearance => "Appearance",
            Self::Canvas => "Canvas",
            Self::Overlays => "Compare",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Indicators => Icon::Waveform,
            Self::Scripts => Icon::Curlybraces,
            Self::Appearance => Icon::Palette,
            Self::Canvas => Icon::MacWindow,
            Self::Overlays => Icon::Compare,
        }
    }
}

#[derive(Default)]
pub struct ChartSettingsState {
    pub tab: Tab,
    /// What the Scripts page's search field holds.
    script_query: String,
    templates: TemplateState,
    /// The Compare page's open cards, search and pending confirmation.
    compare: chart_settings_compare::CompareState,
}

impl ChartSettingsState {
    /// Says what an action on the whole workspace did, in the sheet's footer.
    pub fn say(&mut self, text: impl Into<String>) {
        self.templates.say(text);
    }
}

/// What the sheet reads and edits for one chart.
pub struct ChartSettingsContext<'a> {
    pub title: &'a str,
    pub symbol: &'a str,
    pub source: DataSource,
    /// How many candles the chart has loaded, for the RSI warm-up note.
    pub loaded_candles: usize,
    pub indicators: &'a mut IndicatorConfig,
    pub card: &'a mut ChartCard,
    /// The chart's price-scale mode, which is not part of its appearance.
    pub scale_mode: PriceScaleMode,
    /// Where chart templates are kept.
    pub store: &'a SharedStore,
}

/// What the sheet asks the workspace to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChartSettingsAction {
    Done,
    RemoveChart,
    OpenPineStudio,
    /// Open (or put away) the Inputs and Style popover of an applied script, pointing at `Rect`.
    ConfigureScript(u64, Rect),
    /// Give every chart of the workspace this look.
    ApplyToAll(crate::model::chart::template::ApplyScope),
    /// Read the price axis in this mode.
    SetScaleMode(PriceScaleMode),
    /// Change a compared market.
    Overlay(crate::model::overlay::OverlayCommand),
    /// Pick another market to compare, in the symbol picker.
    AddOverlay,
}

pub fn show(
    ctx: &egui::Context,
    state: &mut ChartSettingsState,
    mut c: ChartSettingsContext,
) -> Option<ChartSettingsAction> {
    let mut action = None;
    let was_templating = state.templates.has_dialog();
    let page_height = page_height(ctx.content_rect().height());
    let tabs = Tab::for_source(c.source);
    if !tabs.contains(&state.tab) {
        state.tab = tabs[0];
    }
    let ((), clicked_outside) =
        sheet::show_dismissable(ctx, Id::new("chart-settings"), WIDTH, |ui| {
            ui.add_space(24.0);
            sheet::inset(ui, |ui| header(ui, &c));
            ui.add_space(16.0);
            if tabs.len() > 1 {
                tab_bar(ui, state, tabs);
                ui.add_space(16.0);
            }
            rule(ui);
            egui::ScrollArea::vertical()
                .id_salt(("chart-settings-page", state.tab as u8))
                .max_height(page_height)
                .min_scrolled_height(page_height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Frame::new()
                        .inner_margin(Margin::same(sheet::INSET))
                        .show(ui, |ui| {
                            ui.set_width(WIDTH - 2.0 * f32::from(sheet::INSET));
                            match state.tab {
                                Tab::Indicators => indicators_page(ui, &mut c),
                                Tab::Scripts => {
                                    if let Some(next) = scripts_page(ui, state, &mut c) {
                                        action = Some(next);
                                    }
                                }
                                Tab::Appearance => {
                                    if let Some(next) =
                                        appearance_page(ui, &mut c, &mut state.templates)
                                    {
                                        action = Some(next);
                                    }
                                }
                                Tab::Overlays => {
                                    let input = CompareInput {
                                        card: &*c.card,
                                        chart_name: c.title,
                                        scale_mode: c.scale_mode,
                                        price_based: c
                                            .card
                                            .effective_chart_type(c.source)
                                            .is_synthetic(),
                                    };
                                    if let Some(next) =
                                        chart_settings_compare::page(ui, &mut state.compare, &input)
                                    {
                                        action = Some(next);
                                    }
                                }
                                Tab::Canvas => {
                                    let (store, templates) = (c.store, &mut state.templates);
                                    super::chart_canvas_settings::show(
                                        ui,
                                        c.card,
                                        Id::new(("chart-canvas", c.symbol)),
                                        |ui, card| {
                                            chart_templates::menu(ui, templates, store, card)
                                        },
                                    );
                                }
                            }
                        });
                });
            rule(ui);
            sheet::inset(ui, |ui| {
                if let Some(next) = footer(ui, state) {
                    action = Some(next);
                }
            });
        });
    // A click on the dimmed area around the sheet puts it away, like Done.
    if action.is_none() && clicked_outside {
        action = Some(ChartSettingsAction::Done);
    }
    // A template dialog takes Escape first; a second press closes the sheet.
    if let Some(TemplateAction::ApplyToAll(scope)) =
        chart_templates::dialogs(ctx, &mut state.templates, c.store, c.card)
    {
        action = Some(ChartSettingsAction::ApplyToAll(scope));
    }
    if action.is_none()
        && !state.templates.has_dialog()
        && !was_templating
        && ctx.input(|i| i.key_pressed(egui::Key::Escape))
    {
        action = Some(ChartSettingsAction::Done);
    }
    action
}

/// A hairline across the sheet, between the header, the page and the footer.
fn rule(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::separator());
}

fn header(ui: &mut Ui, c: &ChartSettingsContext) {
    let subtitle = format!(
        "{} · Changes apply as you make them",
        c.source.display_name()
    );
    sheet::header_with(ui, c.title, &subtitle, |ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(46.0), Sense::hover());
        kit::chart_icon(ui, rect.shrink(1.0), c.symbol, c.source, true);
    });
}

/// The icon tab bar, centred under the header.
fn tab_bar(ui: &mut Ui, state: &mut ChartSettingsState, tabs: &[Tab]) {
    let items: Vec<TabItem<Tab>> = tabs
        .iter()
        .map(|&tab| TabItem {
            value: tab,
            title: tab.title(),
            icon: tab.icon(),
            count: 0,
        })
        .collect();
    ui.vertical_centered(|ui| {
        kit::tab_bar_regular(ui, Id::new("chart-settings-tabs"), &items, &mut state.tab);
    });
}

/// The footer row: "Remove Chart" on the left, the last template action's notice, "Done" on the
/// right. (The Templates menu sits beside each page's Reset button.) Returns what a button asked
/// for. Removing is not confirmed: the chart's drawings are kept, so it can be added back.
fn footer(ui: &mut Ui, state: &mut ChartSettingsState) -> Option<ChartSettingsAction> {
    let mut action = None;
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        if kit::destructive_sheet_button(ui, Icon::Trash, "Remove Chart").clicked() {
            action = Some(ChartSettingsAction::RemoveChart);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if kit::sheet_button(ui, "Done", true, true).clicked() {
                action = Some(ChartSettingsAction::Done);
            }
            // What the last template action did, between the buttons.
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                chart_templates::notice(ui, &mut state.templates);
            });
        });
    });
    ui.add_space(14.0);
    action
}

// MARK: Appearance

fn appearance_page(
    ui: &mut Ui,
    c: &mut ChartSettingsContext,
    templates: &mut TemplateState,
) -> Option<ChartSettingsAction> {
    ui.spacing_mut().item_spacing.y = 8.0;
    if c.source.is_line_chart() {
        // A market drawn as a line has no chart types: its two colours are all there is to set.
        line_colors(ui, c, templates);
    } else {
        let store = c.store;
        super::chart_style_settings::show(
            ui,
            c.card,
            c.source,
            Id::new(("chart-style", c.symbol)),
            |ui, card| chart_templates::menu(ui, templates, store, card),
        );
    }

    ui.add_space(16.0);
    price_axis(ui, c.card, c.scale_mode).map(ChartSettingsAction::SetScaleMode)
}

/// What the price scale reads, for the line under the Scale picker.
fn scale_hint(mode: PriceScaleMode) -> &'static str {
    match mode {
        PriceScaleMode::Regular => {
            "The axis reads prices; equal price differences are equally tall."
        }
        PriceScaleMode::Percent => {
            "The change from the first bar shown, so 0% is that bar's close."
        }
        PriceScaleMode::IndexedTo100 => {
            "The first bar shown reads 100; the rest are 100 plus their change."
        }
        PriceScaleMode::Logarithmic => {
            "The axis reads prices; equal price ratios are equally tall."
        }
    }
}

/// The prices the example writes, each with what it stands for.
const PRICE_SAMPLES: [(&str, f64); 3] = [
    ("Large", 67_432.19),
    ("Small", 0.123_456),
    ("Tiny", 0.000_002_78),
];

/// How the chart writes a price (axis, crosshair, readouts, rulers): how many decimal places, and
/// how a price under 0.001 reads, with a preview of the choice.
///
/// The scale mode is the chart's, not the look's: it is not in the appearance or its templates, so
/// a pick is handed back for the workspace to set, the way the corner menu and the shortcuts do.
fn price_axis(ui: &mut Ui, card: &mut ChartCard, mode: PriceScaleMode) -> Option<PriceScaleMode> {
    let appearance = card.appearance();
    let (decimals, small) = (appearance.axis_decimals, appearance.axis_small_prices);
    settings_card::section_title(
        ui,
        "Price axis",
        Some("How prices are written on the axis, the crosshair and the readouts."),
    );
    // The section title tightens the spacing for its own subtitle; the cards keep the page's.
    ui.spacing_mut().item_spacing.y = 8.0;
    let mut picked = None;
    settings_card::card(
        ui,
        Icon::PriceScale,
        "Scale",
        Some(scale_hint(mode)),
        |ui| {
            let labels: Vec<String> = PriceScaleMode::ALL
                .iter()
                .map(|mode| mode.label().to_owned())
                .collect();
            let selected = PriceScaleMode::ALL
                .iter()
                .position(|candidate| *candidate == mode)
                .unwrap_or_default();
            if let Some(index) = kit::menu_picker(ui, 150.0, &labels, selected, true) {
                picked = Some(PriceScaleMode::ALL[index]);
            }
        },
    );
    settings_card::card(
        ui,
        Icon::Hash,
        "Decimal places",
        Some(&decimals_hint(decimals)),
        |ui| {
            let labels: Vec<String> = std::iter::once("Auto".to_owned())
                .chain((0..=MAX_DECIMALS).map(|places| places.to_string()))
                .collect();
            let selected = decimals.map_or(0, |places| usize::from(places) + 1);
            if let Some(index) = kit::menu_picker(ui, 90.0, &labels, selected, true) {
                card.appearance_mut().axis_decimals = index
                    .checked_sub(1)
                    .and_then(|places| u8::try_from(places).ok());
            }
        },
    );
    settings_card::card_reserving(
        ui,
        Icon::Sliders,
        "Small prices",
        Some(&small_prices_hint(small)),
        200.0,
        |ui| {
            let labels: Vec<String> = SmallPrices::ALL
                .iter()
                .map(|style| format!("{}  {}", style.example(), style.label()))
                .collect();
            let selected = SmallPrices::ALL
                .iter()
                .position(|style| *style == small)
                .unwrap_or_default();
            if let Some(index) = kit::menu_picker(ui, 200.0, &labels, selected, true) {
                card.appearance_mut().axis_small_prices = SmallPrices::ALL[index];
            }
        },
    );
    price_preview(ui, card.appearance().price_format());
    picked
}

fn decimals_hint(decimals: Option<u8>) -> String {
    match decimals {
        None => "Picks the precision from the price: 67,432, 12.35, 0.12, 0.0031.".to_owned(),
        Some(places) => format!(
            "Shows {places} decimal place{} on the price axis.",
            if places == 1 { "" } else { "s" }
        ),
    }
}

fn small_prices_hint(style: SmallPrices) -> String {
    match style {
        SmallPrices::ZeroCount => "Below 0.001 the small number counts the zeros after the \
            point: 0.0₅278 is 0.00000278, whatever the decimal places."
            .to_owned(),
        SmallPrices::Decimals => "Below 0.001 every zero is written out. Automatic places keep \
            three digits after them."
            .to_owned(),
    }
}

/// A card showing the sample prices as the chart will write them now: one tile each, the price
/// in the chart's own monospace face under what it stands for.
fn price_preview(ui: &mut Ui, format: PriceFormat) {
    const TILE_HEIGHT: f32 = 58.0;
    const TILE_GAP: f32 = 10.0;
    let width = ui.available_width();
    kit::card_frame(false).show(ui, |ui| {
        ui.set_width(width - 28.0 - 2.0);
        // The header is a card's own: a tile, a title and a hint.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            let (badge, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
            kit::paint_card_badge(ui, badge, Icon::Eye);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.label(kit::text(
                    "Example",
                    12.0,
                    kit::SEMIBOLD,
                    theme::text_primary(),
                ));
                ui.label(kit::text(
                    "The same prices, written the way this chart will show them.",
                    10.5,
                    kit::REGULAR,
                    theme::text_secondary(),
                ));
            });
        });
        ui.add_space(12.0);
        let count = PRICE_SAMPLES.len() as f32;
        let tile_width = (ui.available_width() - TILE_GAP * (count - 1.0)) / count;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = TILE_GAP;
            for (caption, sample) in PRICE_SAMPLES {
                let (tile, _) =
                    ui.allocate_exact_size(vec2(tile_width, TILE_HEIGHT), Sense::hover());
                kit::paint_well(ui, tile, 10.0);
                let caption = kit::galley(
                    ui,
                    &caption.to_uppercase(),
                    kit::size::CAPTION - 1.0,
                    kit::SEMIBOLD,
                    theme::text_tertiary(),
                    tile_width - 28.0,
                );
                ui.painter()
                    .galley(tile.left_top() + vec2(14.0, 11.0), caption, Color32::WHITE);
                ui.painter().text(
                    tile.left_bottom() + vec2(14.0, -11.0),
                    Align2::LEFT_BOTTOM,
                    format.format(sample),
                    FontId::monospace(15.0),
                    theme::text_primary(),
                );
            }
        });
    });
}

/// The colours of a market drawn as a line (prediction markets, CoinMarketCap): where it rises
/// and where it falls, with a preview and "Reset Colors".
fn line_colors(ui: &mut Ui, c: &mut ChartSettingsContext, templates: &mut TemplateState) {
    settings_card::section_title(ui, "Colors", Some("Used for rising and falling prices."));
    ui.spacing_mut().item_spacing.y = 8.0;
    settings_card::card(
        ui,
        Icon::ArrowUpRight,
        "Line up",
        Some("Stretches where the price rises."),
        |ui| color_well(ui, "bullish", &mut c.card.appearance_mut().bullish),
    );
    settings_card::card(
        ui,
        Icon::ArrowDownRight,
        "Line down",
        Some("Stretches where the price falls."),
        |ui| color_well(ui, "bearish", &mut c.card.appearance_mut().bearish),
    );
    preview_row(ui, c.card, c.store, templates);
}

/// The line colour preview, "Reset Colors" and the Templates menu: a rising and a falling run on
/// the window colour.
fn preview_row(
    ui: &mut Ui,
    card: &mut ChartCard,
    store: &SharedStore,
    templates: &mut TemplateState,
) {
    let appearance = card.appearance();
    let width = ui.available_width();
    kit::card_frame(false).show(ui, |ui| {
        ui.set_width(width - 28.0 - 2.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            let (well, _) = ui.allocate_exact_size(vec2(96.0 + 20.0, 56.0 + 20.0), Sense::hover());
            kit::paint_well(ui, well, 10.0);
            paint_preview(
                ui,
                well.shrink(10.0),
                rgb(appearance.bullish),
                rgb(appearance.bearish),
            );
            let text_width = (ui.available_width() - 120.0).max(120.0);
            let (title_size, detail_size) =
                (kit::size::SUBHEADLINE + 1.0, kit::size::CAPTION + 0.5);
            let line_height = |ui: &Ui, size: f32| {
                kit::galley(ui, "Ag", size, kit::REGULAR, Color32::WHITE, f32::INFINITY)
                    .size()
                    .y
            };
            let block = line_height(ui, title_size) + 3.0 + line_height(ui, detail_size);
            ui.allocate_ui(vec2(text_width, well.height()), |ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                // Centre the two lines against the well.
                ui.add_space(((well.height() - block) / 2.0).max(0.0));
                ui.label(kit::text(
                    "Preview",
                    title_size,
                    kit::SEMIBOLD,
                    theme::text_primary(),
                ));
                ui.label(kit::text(
                    "The chart behind this window updates as you pick.",
                    detail_size,
                    kit::REGULAR,
                    theme::text_secondary(),
                ));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Right to left: Templates first puts it after Reset Colors.
                chart_templates::menu(ui, templates, store, card);
                if kit::push_button(ui, None, "Reset Colors", true).clicked() {
                    let defaults = ChartAppearance::default();
                    let appearance = card.appearance_mut();
                    appearance.bullish = defaults.bullish;
                    appearance.bearish = defaults.bearish;
                }
            });
        });
    });
}

/// `ColorPicker` as the macOS well, for a colour with no opacity. Edits the colour in place.
fn color_well(ui: &mut Ui, name: &str, color: &mut [u8; 3]) {
    if let Some(picked) = kit::color_well(
        ui,
        vec2(44.0, 24.0),
        name,
        rgb(*color),
        egui::color_picker::Alpha::Opaque,
    ) {
        *color = [picked.r(), picked.g(), picked.b()];
    }
}

fn rgb([r, g, b]: [u8; 3]) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// Two joined line runs, rising in `up` and falling in `down`, with every point a fraction of
/// `rect`.
fn paint_preview(ui: &Ui, rect: Rect, up: Color32, down: Color32) {
    let painter = ui.painter();
    let point = |x: f32, y: f32| {
        pos2(
            rect.left() + x * rect.width(),
            rect.top() + y * rect.height(),
        )
    };
    let stroke = |color| Stroke::new(2.25, color);
    let runs = [
        (
            up,
            [(0.04, 0.8), (0.2, 0.6), (0.34, 0.68), (0.5, 0.28)].as_slice(),
        ),
        (
            down,
            [(0.5, 0.28), (0.66, 0.42), (0.8, 0.3), (0.96, 0.82)].as_slice(),
        ),
    ];
    for (color, points) in runs {
        let points: Vec<_> = points.iter().map(|&(x, y)| point(x, y)).collect();
        painter.add(egui::Shape::line(points.clone(), stroke(color)));
        // Round joins and caps.
        for p in points {
            painter.circle_filled(p, 1.125, color);
        }
    }
}

// MARK: Indicators

fn indicators_page(ui: &mut Ui, c: &mut ChartSettingsContext) {
    settings_card::section_title(
        ui,
        "Indicators",
        Some("Add context to the chart without changing its underlying data."),
    );
    ui.spacing_mut().item_spacing.y = 8.0;
    let source = c.source;
    let loaded = c.loaded_candles;
    let config = &mut *c.indicators;
    // Line sources report one price per timestamp and no turnover at all.
    if !source.is_line_chart() {
        let hint = if source.provides_volume() {
            "Turnover per candle, drawn along the bottom of the chart.".to_owned()
        } else {
            format!(
                "{} doesn't report per-candle volume, so bars aren't available for this chart.",
                source.display_name()
            )
        };
        settings_card::card(ui, Icon::BarChart, "Volume Bars", Some(&hint), |ui| {
            // Left visible rather than hidden: a greyed-out switch with a reason reads better
            // than a setting that silently does nothing.
            settings_card::switch_enabled(ui, &mut config.show_volume, source.provides_volume());
        });
    }
    // RSI has no value for its first `period` candles, so a short range draws a stub of a
    // line or none at all.
    let rsi_hint = if loaded > 0 && loaded <= RSI_PERIOD {
        format!(
            "Only {loaded} candles loaded — RSI needs more than {RSI_PERIOD}. Zoom out or pick a longer range."
        )
    } else {
        format!(
            "Momentum from 0–100 across the bottom, with guides at {} and {}.",
            RSI_OVERSOLD as i64, RSI_OVERBOUGHT as i64
        )
    };
    settings_card::card(
        ui,
        Icon::Waveform,
        &format!("RSI ({RSI_PERIOD})"),
        Some(&rsi_hint),
        |ui| {
            settings_card::switch(ui, &mut config.show_rsi);
        },
    );
    settings_card::card(
        ui,
        Icon::ChartLine,
        "EMA",
        Some("Exponential moving average over the closes, drawn on the price scale."),
        |ui| {
            settings_card::switch(ui, &mut config.show_ema);
            ui.add_space(4.0);
            let labels: Vec<String> = EMA_PERIODS.iter().map(usize::to_string).collect();
            let selected = EMA_PERIODS
                .iter()
                .position(|&period| period == config.ema_period)
                .unwrap_or(0);
            if let Some(index) = kit::menu_picker(ui, 72.0, &labels, selected, config.show_ema) {
                config.ema_period = EMA_PERIODS[index];
            }
        },
    );
    settings_card::card(
        ui,
        Icon::Bands,
        "Bollinger Bands",
        Some(&format!(
            "{BOLLINGER_PERIOD}-period average with bands {} standard deviations either side — wide when volatile, tight when calm.",
            BOLLINGER_MULTIPLIER as i64
        )),
        |ui| {
            settings_card::switch(ui, &mut config.show_bollinger);
        },
    );
    settings_card::card(
        ui,
        Icon::Cycle,
        "Trend Flips",
        Some(&format!(
            "Bullish and bearish markers from {SUPERTREND_PERIOD}-period ATR × {}. Signals appear after the candle closes and can whipsaw in sideways markets.",
            SUPERTREND_MULTIPLIER as i64
        )),
        |ui| {
            settings_card::switch(ui, &mut config.show_trend_flips);
        },
    );
}

// MARK: Scripts

/// What one row of the "On this chart" list shows, copied out so drawing needs no borrow of the
/// chart.
struct RowData {
    id: u64,
    title: String,
    is_strategy: bool,
    /// `None` until the script has run.
    overlay: Option<bool>,
    visible: bool,
    failure: Option<String>,
    evaluating: bool,
    order: VisualOrder,
    /// Where the instance sits among all of the chart's scripts, whatever the search shows.
    position: usize,
    total: usize,
}

enum RowEvent {
    ToggleVisible(u64),
    Remove(u64),
    MoveTo(u64, usize),
    /// The script's settings, anchored to the control that asked.
    Configure(u64, Rect),
    /// Where the script's visuals sit against the chart's candles.
    SetOrder(u64, VisualOrder),
    /// Turn a saved script on for this chart (its first instance).
    Enable(Arc<LocalScript>),
    /// Apply the instance's script again, with default inputs.
    Duplicate(u64),
}

/// The list gets a search field once it has more rows than this.
const SEARCH_FROM: usize = 5;

fn scripts_page(
    ui: &mut Ui,
    state: &mut ChartSettingsState,
    c: &mut ChartSettingsContext,
) -> Option<ChartSettingsAction> {
    let mut action = None;
    let saved = script_catalog::scripts(ui.ctx());
    let instances: Vec<PineInstance> = c.card.pine_instances().to_vec();
    if instances.is_empty() && saved.is_empty() {
        settings_card::section_title(ui, "Scripts", Some("Pine scripts running on this chart."));
        empty_state(ui, &mut action);
        return action;
    }
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            settings_card::section_title(
                ui,
                "Scripts",
                Some("Switch a script on to run it on this chart."),
            );
        });
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            if kit::push_button(ui, Some(Icon::Curlybraces), "Pine Studio", true)
                .on_hover_text("Write, edit and organise scripts")
                .clicked()
            {
                action = Some(ChartSettingsAction::OpenPineStudio);
            }
        });
    });

    let off_chart: Vec<&Arc<LocalScript>> = saved
        .iter()
        .filter(|script| {
            !instances
                .iter()
                .any(|i| i.script_id.as_deref() == Some(script.id.as_str()))
        })
        .collect();
    if instances.len() + off_chart.len() > SEARCH_FROM {
        ui.add_space(2.0);
        kit::search_field(
            ui,
            Id::new("chart-settings-script-search"),
            &mut state.script_query,
            "Search scripts",
            ui.available_width(),
        );
        ui.add_space(4.0);
    } else {
        // A filter nobody can see must not hide rows.
        state.script_query.clear();
    }
    let query = state.script_query.trim().to_lowercase();
    let mut available = script_catalog::matching(off_chart.iter().copied(), &query);
    if query.is_empty() {
        // Indicators, then strategies; each already by name.
        available.sort_by_key(|script| script.script_type == ScriptType::Strategy);
    }

    // Copy what the rows show out of the chart, so drawing needs no borrow of it.
    let rows: Vec<RowData> = instances
        .iter()
        .enumerate()
        .filter(|(_, instance)| query.is_empty() || instance.name.to_lowercase().contains(&query))
        .map(|(position, instance)| {
            let status = c.card.pine_status(instance.id);
            RowData {
                id: instance.id,
                title: instance.name.clone(),
                is_strategy: status.as_ref().is_some_and(|s| s.output.strategy.is_some()),
                overlay: status.as_ref().map(|s| s.output.overlay),
                visible: instance.visible,
                failure: status
                    .as_ref()
                    .and_then(|s| s.error.map(|e| e.message.clone())),
                evaluating: instance.visible && status.is_none(),
                order: instance.visual_order,
                position,
                total: instances.len(),
            }
        })
        .collect();
    let bars = instances
        .iter()
        .filter_map(|i| c.card.pine_status(i.id))
        .map(|s| s.output.bar_count)
        .max()
        .filter(|bars| *bars > 0);

    let mut events: Vec<RowEvent> = Vec::new();
    if !rows.is_empty() {
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(kit::text(
                "On this chart",
                kit::size::SUBHEADLINE + 1.0,
                kit::SEMIBOLD,
                theme::text_primary(),
            ));
            count_chip(ui, &rows.len().to_string());
            if let Some(bars) = bars {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    status_badge(ui, &format!("{bars} bars"), theme::text_secondary());
                });
            }
        });
        ui.add_space(6.0);
        row_card(ui, rows.len(), |ui, index| {
            instance_row(ui, &rows[index], index, rows.len(), &mut events);
        });
    }
    if !available.is_empty() {
        ui.add_space(if rows.is_empty() { 12.0 } else { 20.0 });
        ui.horizontal(|ui| {
            ui.label(kit::text(
                if rows.is_empty() {
                    "Your scripts"
                } else {
                    "Available"
                },
                kit::size::SUBHEADLINE + 1.0,
                kit::SEMIBOLD,
                theme::text_primary(),
            ));
            count_chip(ui, &available.len().to_string());
        });
        ui.add_space(6.0);
        row_card(ui, available.len(), |ui, index| {
            available_row(ui, available[index], index, available.len(), &mut events);
        });
    }
    if rows.is_empty() && available.is_empty() {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(kit::text(
                format!("No scripts match “{}”", state.script_query.trim()),
                kit::size::SUBHEADLINE + 1.0,
                kit::REGULAR,
                theme::text_tertiary(),
            ));
        });
    }

    if let Some(asked) = apply_events(c.card, &instances, events) {
        action = Some(asked);
    }

    // One backtest report per applied strategy, so several never hide behind "the first one".
    let strategies: Vec<&PineInstance> = instances
        .iter()
        .filter(|i| {
            c.card
                .pine_status(i.id)
                .is_some_and(|s| s.output.strategy.is_some())
        })
        .collect();
    if !strategies.is_empty() {
        ui.add_space(20.0);
        settings_card::section_title(ui, "Backtest", None);
        for instance in &strategies {
            if strategies.len() > 1 {
                ui.label(kit::text(
                    &instance.name,
                    kit::size::SUBHEADLINE + 1.0,
                    kit::SEMIBOLD,
                    theme::text_secondary(),
                ));
            }
            if let Some(status) = c.card.pine_status(instance.id) {
                drawer::report_content(ui, Some(status.output));
            }
            ui.add_space(8.0);
        }
    }
    action
}

/// Applies what the rows asked for to the chart; returns the sheet-level action, if one asked.
fn apply_events(
    card: &mut ChartCard,
    instances: &[PineInstance],
    events: Vec<RowEvent>,
) -> Option<ChartSettingsAction> {
    let mut action = None;
    for event in events {
        match event {
            RowEvent::ToggleVisible(id) => {
                let visible = instances
                    .iter()
                    .find(|i| i.id == id)
                    .is_some_and(|i| i.visible);
                card.set_pine_visible(id, !visible);
            }
            RowEvent::Remove(id) => {
                card.remove_pine_instance(id);
            }
            RowEvent::MoveTo(id, index) => card.move_pine_instance(id, index),
            RowEvent::SetOrder(id, order) => card.set_pine_visual_order(id, order),
            RowEvent::Configure(id, anchor) => {
                action = Some(ChartSettingsAction::ConfigureScript(id, anchor));
            }
            RowEvent::Enable(script) => {
                card.add_pine_instance(Some(script.id.clone()), &script.name, &script.source);
            }
            RowEvent::Duplicate(id) => {
                if let Some(instance) = instances.iter().find(|i| i.id == id) {
                    card.add_pine_instance(
                        instance.script_id.clone(),
                        &instance.name,
                        &instance.source,
                    );
                }
            }
        }
    }
    action
}

fn count_chip(ui: &mut Ui, text: &str) {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        FontId::monospace(10.0),
        theme::text_secondary(),
    );
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 14.0, 16.0), Sense::hover());
    ui.painter().rect_filled(rect, 8.0, theme::quaternary(1.0));
    ui.painter().galley(
        pos2(rect.min.x + 7.0, rect.center().y - galley.size().y / 2.0),
        galley,
        theme::text_secondary(),
    );
}

fn status_badge(ui: &mut Ui, text: &str, color: Color32) {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), FontId::proportional(10.0), color);
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 18.0, 20.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, 10.0, theme::with_alpha(color, 0.16));
    ui.painter().galley(
        pos2(rect.min.x + 9.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
}

/// `ScriptsEmptyState`: a disc glyph, a title, a message and an optional call to action on a
/// dashed card.
fn empty_state(ui: &mut Ui, action: &mut Option<ChartSettingsAction>) {
    ui.add_space(12.0);
    let card = egui::Frame::new()
        .fill(theme::card_fill(false))
        .corner_radius(kit::CARD_RADIUS)
        .inner_margin(Margin::symmetric(24, 36))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.vertical_centered(|ui| {
                let (disc, _) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::hover());
                ui.painter()
                    .circle_filled(disc.center(), 24.0, theme::quaternary(1.3));
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(disc.center(), Vec2::splat(20.0)),
                    Icon::Curlybraces,
                    theme::text_secondary(),
                );
                ui.add_space(12.0);
                let (title, message) = (
                    "No scripts yet",
                    "Write one in the Pine Studio and it shows up here.",
                );
                ui.label(kit::text(
                    title,
                    kit::size::HEADLINE + 1.0,
                    kit::BOLD,
                    theme::text_primary(),
                ));
                ui.add_space(3.0);
                ui.label(kit::text(
                    message,
                    kit::size::SUBHEADLINE + 1.0,
                    kit::REGULAR,
                    theme::text_secondary(),
                ));
                ui.add_space(12.0);
                if kit::sheet_button(ui, "Open Pine Studio", true, true).clicked() {
                    *action = Some(ChartSettingsAction::OpenPineStudio);
                }
            });
        });
    dashed_rounded_rect(
        ui.painter(),
        card.response.rect,
        kit::CARD_RADIUS,
        Stroke::new(1.0, theme::with_alpha(theme::separator(), 0.9)),
    );
}

/// A rounded rectangle outlined with `[5, 4]` dashes, as `StrokeStyle(dash:)` draws one.
fn dashed_rounded_rect(painter: &egui::Painter, rect: Rect, radius: f32, stroke: Stroke) {
    let rect = rect.shrink(stroke.width / 2.0);
    let corners = [
        (pos2(rect.left() + radius, rect.top() + radius), 180.0_f32),
        (pos2(rect.right() - radius, rect.top() + radius), 270.0),
        (pos2(rect.right() - radius, rect.bottom() - radius), 0.0),
        (pos2(rect.left() + radius, rect.bottom() - radius), 90.0),
    ];
    let mut points = Vec::new();
    for (center, start) in corners {
        for step in 0..=8 {
            let angle = (start + 90.0 * step as f32 / 8.0).to_radians();
            points.push(center + radius * vec2(angle.cos(), angle.sin()));
        }
    }
    points.push(points[0]);
    painter.extend(egui::Shape::dashed_line(&points, stroke, 5.0, 4.0));
}

/// A card of `count` rows with an inset hairline between them.
fn row_card(ui: &mut Ui, count: usize, mut row: impl FnMut(&mut Ui, usize)) {
    kit::card_frame(false)
        .inner_margin(Margin::ZERO)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for index in 0..count {
                if index > 0 {
                    let y = ui.cursor().min.y;
                    let rect = ui.max_rect();
                    ui.painter().line_segment(
                        [pos2(rect.min.x + 54.0, y), pos2(rect.max.x - 12.0, y)],
                        Stroke::new(1.0, kit::hairline()),
                    );
                }
                row(ui, index);
            }
        });
}

/// A wash under the pointer. The first and last rows round the corners they share with the card,
/// so the wash never pokes out of it.
fn hover_wash(ui: &Ui, rect: Rect, index: usize, count: usize) {
    let radius = (kit::CARD_RADIUS - 1.0) as u8;
    ui.painter().rect_filled(
        rect.shrink(1.0),
        CornerRadius {
            nw: if index == 0 { radius } else { 0 },
            ne: if index == 0 { radius } else { 0 },
            sw: if index + 1 == count { radius } else { 0 },
            se: if index + 1 == count { radius } else { 0 },
        },
        theme::ink(6),
    );
}

/// The switch at a row's trailing edge, `on` or off, animated. `what` names the script for
/// screen readers and the tooltip.
fn switch_at(ui: &mut Ui, row: Rect, id: Id, on: bool, what: &str) -> egui::Response {
    let rect = Rect::from_center_size(
        pos2(row.max.x - 12.0 - 18.0, row.center().y),
        vec2(36.0, 20.0),
    );
    let response = ui.interact(rect, id, Sense::click());
    let t = ui.ctx().animate_bool_with_time(id, on, 0.12);
    kit::paint_switch(ui.painter(), rect, t, false);
    let tip = if on {
        format!("Turn {what} off — it keeps its settings")
    } else {
        format!("Turn {what} on")
    };
    crate::ui::accessibility::label(
        response,
        format!("{what}, {}", if on { "on" } else { "off" }),
    )
    .on_hover_cursor(CursorIcon::PointingHand)
    .on_hover_text(tip)
}

/// A neutral 30pt tile holding `icon`, as on every settings card.
fn badge_tile(ui: &Ui, row: Rect, tint: Color32, icon: Icon, stroke: Option<Color32>) -> Rect {
    let badge = Rect::from_center_size(pos2(row.min.x + 27.0, row.center().y), Vec2::splat(30.0));
    match stroke {
        Some(color) => {
            ui.painter()
                .rect_filled(badge, 8.0, theme::with_alpha(color, 0.14));
            ui.painter().rect_stroke(
                badge,
                8.0,
                Stroke::new(1.0, theme::with_alpha(color, 0.3)),
                StrokeKind::Inside,
            );
        }
        None => {
            ui.painter().rect_filled(badge, 8.0, theme::ink(10));
            ui.painter().rect_stroke(
                badge,
                8.0,
                Stroke::new(1.0, theme::ink(14)),
                StrokeKind::Inside,
            );
        }
    }
    icons::paint(
        ui.painter(),
        Rect::from_center_size(badge.center(), Vec2::splat(15.0)),
        icon,
        tint,
    );
    badge
}

/// A saved script that is not on this chart yet: switching it on, or clicking its row, applies it.
fn available_row(
    ui: &mut Ui,
    script: &Arc<LocalScript>,
    index: usize,
    count: usize,
    events: &mut Vec<RowEvent>,
) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
    if response.hovered() {
        hover_wash(ui, rect, index, count);
    }
    let strategy = script.script_type == ScriptType::Strategy;
    let badge = badge_tile(
        ui,
        rect,
        theme::text_tertiary(),
        if strategy {
            Icon::Cycle
        } else {
            Icon::ChartLine
        },
        None,
    );
    let switch = switch_at(
        ui,
        rect,
        Id::new(("script-switch", &script.id)),
        false,
        &script.name,
    );
    if switch.clicked() || response.clicked() {
        events.push(RowEvent::Enable(script.clone()));
    }
    let width = (switch.rect.min.x - 12.0 - (badge.max.x + 12.0)).max(40.0);
    let title = kit::galley(
        ui,
        &script.name,
        kit::size::SUBHEADLINE + 1.0,
        kit::SEMIBOLD,
        theme::text_primary(),
        width,
    );
    let meta = kit::galley(
        ui,
        script.script_type.display_name(),
        kit::size::CAPTION + 0.5,
        kit::REGULAR,
        theme::text_secondary(),
        width,
    );
    let top = rect.center().y - (title.size().y + 2.0 + meta.size().y) / 2.0;
    let left = badge.max.x + 12.0;
    ui.painter()
        .galley(pos2(left, top), title.clone(), theme::text_primary());
    ui.painter().galley(
        pos2(left, top + title.size().y + 2.0),
        meta,
        theme::text_secondary(),
    );
    crate::ui::accessibility::label(response, format!("Turn on {}", script.name));
}

/// One applied script: what it is, how it is doing, and what can be done to it.
fn instance_row(
    ui: &mut Ui,
    row: &RowData,
    index: usize,
    count: usize,
    events: &mut Vec<RowEvent>,
) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
    let failing = row.failure.is_some();
    let dim = if row.visible { 1.0 } else { 0.55 };

    if response.hovered() {
        hover_wash(ui, rect, index, count);
    }

    // A neutral tile, as on every settings card. Colour says something here: orange when the
    // script fails, the accent while it draws, and nothing once it is off.
    let kind_icon = if failing {
        Icon::TriangleFill
    } else if row.is_strategy {
        Icon::Cycle
    } else {
        Icon::ChartLine
    };
    let tint = if failing {
        theme::ORANGE
    } else if row.visible {
        theme::SYSTEM_ACCENT
    } else {
        theme::text_tertiary()
    };
    let badge = badge_tile(ui, rect, tint, kind_icon, failing.then_some(theme::ORANGE));

    // Controls, right to left: the switch, settings, remove.
    let switch = switch_at(
        ui,
        rect,
        Id::new(("instance-switch", row.id)),
        row.visible,
        &row.title,
    );
    if switch.clicked() {
        events.push(RowEvent::ToggleVisible(row.id));
    }
    let mut x = switch.rect.min.x - 6.0;
    let mut button = |ui: &mut Ui, icon: Icon, tip: &str, destructive: bool| -> egui::Response {
        let hit = Rect::from_center_size(pos2(x - 12.0, rect.center().y), Vec2::splat(24.0));
        x -= 26.0;
        let response = ui.interact(
            hit,
            Id::new(("instance-button", row.id, tip.to_owned())),
            Sense::click(),
        );
        if response.hovered() {
            ui.painter().rect_filled(hit, 6.0, theme::ink(18));
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        let color = if destructive && response.hovered() {
            theme::syntax::string()
        } else {
            theme::text_secondary()
        };
        icons::paint(
            ui.painter(),
            Rect::from_center_size(hit.center(), Vec2::splat(13.0)),
            icon,
            color,
        );
        response.on_hover_text(tip.to_owned())
    };
    let settings = button(
        ui,
        Icon::Sliders,
        &format!("Edit {} settings", row.title),
        false,
    );
    let anchor = settings.rect;
    if settings.clicked() {
        events.push(RowEvent::Configure(row.id, anchor));
    }
    if button(
        ui,
        Icon::Xmark,
        &format!("Remove {} from this chart", row.title),
        true,
    )
    .clicked()
    {
        events.push(RowEvent::Remove(row.id));
    }
    let text_right = x - 4.0;

    let title_width = (text_right - (badge.max.x + 12.0)).max(40.0);
    let title = kit::galley(
        ui,
        &row.title,
        kit::size::SUBHEADLINE + 1.0,
        kit::SEMIBOLD,
        theme::text_primary().gamma_multiply(dim),
        title_width,
    );
    let meta_text = row.failure.clone().unwrap_or_else(|| {
        if row.evaluating {
            "Evaluating…".to_owned()
        } else {
            let mut parts = vec![
                if row.is_strategy {
                    "Strategy"
                } else {
                    "Indicator"
                }
                .to_owned(),
            ];
            if !row.is_strategy
                && let Some(overlay) = row.overlay
            {
                parts.push(if overlay { "On chart" } else { "Own pane" }.to_owned());
            }
            if !row.visible {
                parts.push("Off".to_owned());
            }
            parts.join(" · ")
        }
    });
    let meta_color = if failing {
        theme::ORANGE
    } else {
        theme::text_secondary()
    };
    let mut job = egui::text::LayoutJob::simple_singleline(
        meta_text,
        FontId::proportional(kit::size::CAPTION + 0.5),
        meta_color.gamma_multiply(dim),
    );
    job.wrap.max_width = title_width;
    job.wrap.max_rows = 1;
    let meta = ui.painter().layout_job(job);
    let top = rect.center().y - (title.size().y + 2.0 + meta.size().y) / 2.0;
    let left = badge.max.x + 12.0;
    ui.painter()
        .galley(pos2(left, top), title.clone(), theme::text_primary());
    ui.painter()
        .galley(pos2(left, top + title.size().y + 2.0), meta, meta_color);

    response.context_menu(|ui| {
        let label = if row.visible { "Turn Off" } else { "Turn On" };
        if ui.button(label).clicked() {
            events.push(RowEvent::ToggleVisible(row.id));
            ui.close();
        }
        if ui.button("Settings…").clicked() {
            events.push(RowEvent::Configure(row.id, anchor));
            ui.close();
        }
        // Where the visuals sit against the chart's own candles: only a script drawing on the
        // chart has a choice.
        if row.overlay != Some(false) {
            ui.menu_button("Visual Order", |ui| {
                for (order, label) in [
                    (VisualOrder::Script, "As the Script Declares"),
                    (VisualOrder::Front, "Bring to Front"),
                    (VisualOrder::Behind, "Send to Back"),
                ] {
                    if ui.selectable_label(row.order == order, label).clicked() {
                        events.push(RowEvent::SetOrder(row.id, order));
                        ui.close();
                    }
                }
            });
        }
        if ui.button("Add Another Copy").clicked() {
            events.push(RowEvent::Duplicate(row.id));
            ui.close();
        }
        ui.separator();
        if ui
            .add_enabled(row.position > 0, egui::Button::new("Move Up"))
            .clicked()
        {
            events.push(RowEvent::MoveTo(row.id, row.position - 1));
            ui.close();
        }
        if ui
            .add_enabled(row.position + 1 < row.total, egui::Button::new("Move Down"))
            .clicked()
        {
            events.push(RowEvent::MoveTo(row.id, row.position + 1));
            ui.close();
        }
        ui.separator();
        if ui
            .button(RichText::new("Remove from Chart").color(theme::syntax::string()))
            .clicked()
        {
            events.push(RowEvent::Remove(row.id));
            ui.close();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use std::collections::HashMap;
    use std::sync::Mutex;

    fn test_store() -> SharedStore {
        Arc::new(Mutex::new(Store::open_in_memory().expect("open")))
    }

    fn draw(
        state: &mut ChartSettingsState,
        card: &mut ChartCard,
        indicators: &mut IndicatorConfig,
    ) {
        draw_in(&egui::Context::default(), state, card, indicators);
    }

    fn draw_in(
        ctx: &egui::Context,
        state: &mut ChartSettingsState,
        card: &mut ChartCard,
        indicators: &mut IndicatorConfig,
    ) {
        let store = test_store();
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let _ = show(
                    ui.ctx(),
                    state,
                    ChartSettingsContext {
                        title: "BTC/USDT",
                        symbol: "BTCUSDT",
                        source: DataSource::Binance,
                        loaded_candles: 0,
                        indicators,
                        card,
                        scale_mode: PriceScaleMode::Regular,
                        store: &store,
                    },
                );
            });
            output.textures_delta.clear();
        }
    }

    #[test]
    fn coinmarketcap_charts_offer_only_the_pages_that_apply_to_them() {
        assert_eq!(
            Tab::for_source(DataSource::CoinMarketCap),
            &[Tab::Appearance, Tab::Canvas]
        );
        assert_eq!(Tab::for_source(DataSource::Binance), &Tab::ALL);
    }

    #[test]
    fn the_sheet_draws_each_page_empty_and_with_scripts() {
        let mut state = ChartSettingsState::default();
        let mut card = ChartCard::default();
        let mut indicators = IndicatorConfig::default();
        for tab in Tab::ALL {
            state.tab = tab;
            draw(&mut state, &mut card, &mut indicators);
        }
        card.add_pine_instance(
            None,
            "RSI",
            "//@version=6\nindicator(\"x\")\nlen = input.int(14)\nplot(close)\n",
        );
        card.add_pine_instance(
            None,
            "Strat",
            "//@version=6\nstrategy(\"s\")\nplot(close)\n",
        );
        card.set_pine_visible(2, false);
        state.tab = Tab::Scripts;
        draw(&mut state, &mut card, &mut indicators);
    }

    #[test]
    fn the_compare_page_lists_every_market_however_many_and_says_what_is_wrong_with_one() {
        use crate::model::overlay::OverlayList;
        use crate::model::overlay::feed::{OverlayFeed, OverlayStatus};
        let main = crate::model::InstrumentId::new(DataSource::Binance, "BTCUSDT");
        let mut list = OverlayList::new();
        let mut feeds = Vec::new();
        for n in 0..40 {
            list.add(
                &main,
                crate::model::InstrumentId::new(DataSource::Binance, format!("S{n}USDT")),
                None,
            );
            feeds.push(OverlayFeed::empty(match n {
                3 => OverlayStatus::Error("no such market".into()),
                4 => OverlayStatus::Blocked("Same market as the chart"),
                _ => OverlayStatus::Ready,
            }));
        }
        let mut card = ChartCard::default();
        card.set_overlays(list, feeds, 1, DataSource::Binance, "BTCUSDT");
        let mut state = ChartSettingsState {
            tab: Tab::Overlays,
            ..ChartSettingsState::default()
        };
        let mut indicators = IndicatorConfig::default();
        let ctx = egui::Context::default();
        let store = test_store();
        let mut texts = Vec::new();
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let _ = show(
                    ui.ctx(),
                    &mut state,
                    ChartSettingsContext {
                        title: "BTC/USDT",
                        symbol: "BTCUSDT",
                        source: DataSource::Binance,
                        loaded_candles: 0,
                        indicators: &mut indicators,
                        card: &mut card,
                        scale_mode: PriceScaleMode::Percent,
                        store: &store,
                    },
                );
            });
            texts.clear();
            fn walk(shape: &egui::Shape, texts: &mut Vec<String>) {
                match shape {
                    egui::Shape::Text(text) => texts.push(text.galley.text().to_owned()),
                    egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, texts)),
                    _ => {}
                }
            }
            for clipped in &output.shapes {
                walk(&clipped.shape, &mut texts);
            }
            output.textures_delta.clear();
        }
        let has = |needle: &str| texts.iter().any(|text| text.contains(needle));
        assert!(has("40 markets"), "{texts:?}");
        assert!(has("Add symbol"));
        assert!(has("S0USDT"));
        assert!(has("Couldn't load: no such market"));
        assert!(has("Same market as the chart"));
    }

    fn library() -> Vec<LocalScript> {
        use crate::ui::script_catalog::sample;
        (0..9)
            .map(|i| {
                sample(
                    &i.to_string(),
                    &format!("Script {i}"),
                    if i % 3 == 0 {
                        ScriptType::Strategy
                    } else {
                        ScriptType::Indicator
                    },
                )
            })
            .collect()
    }

    #[test]
    fn the_scripts_page_lists_saved_scripts_with_and_without_a_search_field() {
        let ctx = egui::Context::default();
        let mut state = ChartSettingsState {
            tab: Tab::Scripts,
            ..ChartSettingsState::default()
        };
        let mut card = ChartCard::default();
        let mut indicators = IndicatorConfig::default();
        script_catalog::publish(&ctx, library());
        draw_in(&ctx, &mut state, &mut card, &mut indicators);
        // One script on the chart moves out of "Available"; a search narrows both lists.
        let saved = script_catalog::scripts(&ctx);
        card.add_pine_instance(Some(saved[0].id.clone()), &saved[0].name, &saved[0].source);
        state.script_query = "script 4".to_owned();
        draw_in(&ctx, &mut state, &mut card, &mut indicators);
        state.script_query = "nothing like it".to_owned();
        draw_in(&ctx, &mut state, &mut card, &mut indicators);
        // A short library has no search field, and a stale query must not hide its rows.
        script_catalog::publish(&ctx, library().into_iter().take(2).collect());
        draw_in(&ctx, &mut state, &mut card, &mut indicators);
        assert!(state.script_query.is_empty());
    }

    #[test]
    fn switching_a_script_on_applies_it_and_off_keeps_its_settings() {
        use crate::pine::compiler::program::InputValue;
        let script = Arc::new(crate::ui::script_catalog::sample(
            "abc",
            "Squeeze",
            ScriptType::Indicator,
        ));
        let mut card = ChartCard::default();
        apply_events(&mut card, &[], vec![RowEvent::Enable(script.clone())]);
        let instances = card.pine_instances().to_vec();
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].script_id.as_deref(), Some("abc"));
        assert!(instances[0].visible);

        card.set_pine_instance_inputs(
            instances[0].id,
            &HashMap::from([("len".to_owned(), InputValue::Int(21))]),
        );
        card.set_pine_instance_style(
            instances[0].id,
            HashMap::from([("plot.0.color".to_owned(), "#ff0000".to_owned())]),
        );
        let id = instances[0].id;
        apply_events(&mut card, &instances, vec![RowEvent::ToggleVisible(id)]);
        let off = card.pine_instances()[0].clone();
        assert!(!off.visible);
        assert!(!off.inputs.is_empty() && !off.style_overrides.is_empty());

        let instances = card.pine_instances().to_vec();
        apply_events(&mut card, &instances, vec![RowEvent::ToggleVisible(id)]);
        let on = card.pine_instances()[0].clone();
        assert!(on.visible);
        assert_eq!(on.inputs, off.inputs);
        assert_eq!(card.pine_instances().len(), 1, "never a second instance");

        let instances = card.pine_instances().to_vec();
        apply_events(&mut card, &instances, vec![RowEvent::Duplicate(id)]);
        assert_eq!(card.pine_instances().len(), 2);
        assert!(card.pine_instances()[1].inputs.is_empty());
        let instances = card.pine_instances().to_vec();
        apply_events(&mut card, &instances, vec![RowEvent::Remove(id)]);
        assert_eq!(card.pine_instances().len(), 1);
    }

    #[test]
    fn the_sheet_draws_each_line_source() {
        let mut state = ChartSettingsState::default();
        let mut card = ChartCard::default();
        let mut indicators = IndicatorConfig::default();
        draw(&mut state, &mut card, &mut indicators);
        state.tab = Tab::Appearance;
        draw(&mut state, &mut card, &mut indicators);
    }

    #[test]
    fn the_price_axis_hints_say_what_each_choice_does() {
        assert!(decimals_hint(None).contains("Picks the precision"));
        assert_eq!(
            decimals_hint(Some(1)),
            "Shows 1 decimal place on the price axis."
        );
        assert_eq!(
            decimals_hint(Some(0)),
            "Shows 0 decimal places on the price axis."
        );
        assert_eq!(
            decimals_hint(Some(MAX_DECIMALS)),
            "Shows 12 decimal places on the price axis."
        );
        for style in SmallPrices::ALL {
            let hint = small_prices_hint(style);
            assert!(hint.contains("0.001") && !hint.contains("  "), "{hint}");
        }
        assert!(
            small_prices_hint(SmallPrices::ZeroCount).contains(SmallPrices::ZeroCount.example())
        );
    }

    #[test]
    fn the_example_prices_follow_the_chosen_format() {
        let write = |format: PriceFormat| PRICE_SAMPLES.map(|(_, price)| format.format(price));
        assert_eq!(write(PriceFormat::AUTO), ["67,432", "0.12", "0.0₅278"]);
        let pinned = PriceFormat {
            decimals: Some(2),
            small: SmallPrices::Decimals,
        };
        assert_eq!(write(pinned), ["67,432.19", "0.12", "0.00"]);
    }

    #[test]
    fn tabs_have_titles() {
        assert_eq!(
            Tab::ALL.map(Tab::title),
            ["Indicators", "Scripts", "Compare", "Appearance", "Canvas"]
        );
    }

    #[test]
    fn the_sheet_uses_more_of_a_tall_parent_without_crowding_a_small_one() {
        assert_eq!(page_height(480.0), MIN_PAGE_HEIGHT);
        assert_eq!(page_height(800.0), 550.0);
        assert_eq!(page_height(900.0), MAX_PAGE_HEIGHT);
        assert_eq!(page_height(1_400.0), MAX_PAGE_HEIGHT);
    }

    fn shape_text(shape: &egui::Shape, out: &mut String) {
        match shape {
            egui::Shape::Text(text) => {
                out.push_str(text.galley.text());
                out.push('\n');
            }
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| shape_text(shape, out)),
            _ => {}
        }
    }

    /// Every piece of text the Appearance page draws for a chart from `source`.
    fn appearance_text(source: DataSource) -> String {
        let ctx = egui::Context::default();
        let store = test_store();
        let mut state = ChartSettingsState {
            tab: Tab::Appearance,
            ..ChartSettingsState::default()
        };
        let mut card = ChartCard::default();
        let mut indicators = IndicatorConfig::default();
        let mut text = String::new();
        for _ in 0..2 {
            text.clear();
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let _ = show(
                    ui.ctx(),
                    &mut state,
                    ChartSettingsContext {
                        title: "Chart",
                        symbol: "SYM",
                        source,
                        loaded_candles: 0,
                        indicators: &mut indicators,
                        card: &mut card,
                        scale_mode: PriceScaleMode::Regular,
                        store: &store,
                    },
                );
            });
            for clipped in &output.shapes {
                shape_text(&clipped.shape, &mut text);
            }
            output.textures_delta.clear();
        }
        text
    }

    #[test]
    fn the_price_axis_section_names_the_scale_and_says_what_each_mode_reads() {
        let hints: Vec<&str> = PriceScaleMode::ALL.into_iter().map(scale_hint).collect();
        for (index, hint) in hints.iter().enumerate() {
            assert!(!hint.is_empty());
            assert!(hints.iter().skip(index + 1).all(|other| other != hint));
        }
        for mode in PriceScaleMode::ALL {
            let ctx = egui::Context::default();
            let mut card = ChartCard::default();
            let mut text = String::new();
            let mut picked = Some(PriceScaleMode::Regular);
            for _ in 0..2 {
                text.clear();
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    picked = price_axis(ui, &mut card, mode);
                });
                for clipped in &output.shapes {
                    shape_text(&clipped.shape, &mut text);
                }
                output.textures_delta.clear();
            }
            assert!(text.contains("Scale"), "{mode:?}: {text}");
            assert!(text.contains(scale_hint(mode)), "{mode:?}: {text}");
            assert_eq!(picked, None, "drawing the section picks nothing");
        }
    }

    #[test]
    fn a_candle_chart_has_one_set_of_colours_and_a_line_chart_keeps_its_own() {
        let candles = appearance_text(DataSource::Binance);
        assert!(candles.contains("Chart type") && candles.contains("Preview"));
        // Rows below the fold are culled in a small test window, so look at the first one.
        assert!(
            candles.contains("Show candle bodies."),
            "the type's own settings: {candles}"
        );
        assert!(!candles.contains("Bullish candle") && !candles.contains("Bearish candle"));
        assert!(!candles.contains("Reset Colors"));
        let line = appearance_text(DataSource::CoinMarketCap);
        assert!(line.contains("Line up") && line.contains("Reset Colors"));
        assert!(!line.contains("Chart type"));
    }

    #[test]
    fn the_preview_sits_between_the_style_picker_and_the_settings() {
        let text = appearance_text(DataSource::Binance);
        let (style, preview, body) = (
            text.find("Style").expect("picker"),
            text.find("Preview").expect("preview"),
            text.find("Show candle bodies.").expect("settings"),
        );
        assert!(style < preview && preview < body, "{text}");
    }

    /// The Scripts page as a picture. Run by hand with `DEGENVIEW_GALLERY_DIR` set.
    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_the_scripts_page() {
        use crate::ui::render_tests::{Screen, gallery_dir};
        if gallery_dir().is_none() {
            return;
        }
        for (name, typed, applied) in [
            ("scripts-page", "", 2),
            ("scripts-page-search", "sq", 2),
            ("scripts-page-none", "", 0),
        ] {
            let mut screen = Screen::new(egui::vec2(820.0, 780.0));
            script_catalog::publish(&screen.ctx, library());
            let mut state = ChartSettingsState {
                tab: Tab::Scripts,
                script_query: typed.to_owned(),
                ..ChartSettingsState::default()
            };
            let mut card = ChartCard::default();
            let saved = script_catalog::scripts(&screen.ctx);
            for script in saved.iter().take(applied) {
                card.add_pine_instance(Some(script.id.clone()), &script.name, &script.source);
            }
            if applied > 1 {
                card.set_pine_visible(2, false);
            }
            let mut indicators = IndicatorConfig::default();
            let store = test_store();
            let mut last = None;
            for _ in 0..4 {
                last = Some(screen.run(Vec::new(), |ui| {
                    let _ = show(
                        ui.ctx(),
                        &mut state,
                        ChartSettingsContext {
                            title: "BTC/USDT",
                            symbol: "BTCUSDT",
                            source: DataSource::Binance,
                            loaded_candles: 0,
                            indicators: &mut indicators,
                            card: &mut card,
                            scale_mode: PriceScaleMode::Regular,
                            store: &store,
                        },
                    );
                }));
            }
            screen.save(name, last.expect("a frame"));
        }
    }

    /// A card comparing `count` markets, a few of them in trouble, for the Compare page's pictures.
    fn card_with_overlays(count: usize) -> ChartCard {
        use crate::model::overlay::feed::{OverlayFeed, OverlayStatus};
        use crate::model::overlay::{OverlayCommand, OverlayList, ScaleChoice};
        let main = crate::model::InstrumentId::new(DataSource::Binance, "BTCUSDT");
        let names = [
            "ETHUSDT",
            "SOLUSDT",
            "BNBUSDT",
            "DOGEUSDT",
            "XRPUSDT",
            "ADAUSDT",
            "AVAXUSDT",
            "LINKUSDT",
            "DOTUSDT",
            "MATICUSDT",
            "LTCUSDT",
            "ATOMUSDT",
        ];
        let mut list = OverlayList::new();
        for n in 0..count {
            let name = names
                .get(n)
                .map_or_else(|| format!("COIN{n}USDT"), |name| (*name).to_owned());
            list.add(
                &main,
                crate::model::InstrumentId::new(DataSource::Binance, name),
                None,
            );
        }
        let ids: Vec<_> = list.iter().map(|overlay| overlay.id).collect();
        if let Some(id) = ids.get(3) {
            list.apply(OverlayCommand::SetScale(*id, ScaleChoice::Own));
        }
        if let Some(id) = ids.get(4) {
            list.apply(OverlayCommand::SetScale(
                *id,
                ScaleChoice::SharedWith(ids[3]),
            ));
        }
        if let Some(id) = ids.get(1) {
            list.apply(OverlayCommand::SetVisible(*id, false));
        }
        let feeds = list
            .iter()
            .enumerate()
            .map(|(n, _)| {
                let mut feed = OverlayFeed::empty(match n {
                    2 => OverlayStatus::Error("HTTP 429: rate limited".into()),
                    5 => OverlayStatus::Loading,
                    6 => OverlayStatus::Queued,
                    7 => OverlayStatus::Blocked("Same market as the chart"),
                    _ => OverlayStatus::Ready,
                });
                feed.candles = std::sync::Arc::new(vec![crate::model::Kline {
                    open_time: 0,
                    close_time: 1,
                    open: 1.0,
                    high: 1.0,
                    low: 1.0,
                    close: 100.0 / (n as f64 + 1.0),
                    volume: 1.0,
                }]);
                feed
            })
            .collect();
        let mut card = ChartCard::default();
        card.set_overlays(list, feeds, 1, DataSource::Binance, "BTCUSDT");
        card
    }

    /// The Compare page as a picture. Run by hand with `DEGENVIEW_GALLERY_DIR` set.
    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_the_compare_page() {
        use crate::ui::render_tests::{Screen, gallery_dir};
        if gallery_dir().is_none() {
            return;
        }
        for (name, count, mode) in [
            ("compare-empty", 0, PriceScaleMode::Percent),
            ("compare-few", 3, PriceScaleMode::Percent),
            ("compare-regular", 2, PriceScaleMode::Regular),
            ("compare-many", 12, PriceScaleMode::Percent),
        ] {
            let mut screen = Screen::new(egui::vec2(820.0, 860.0));
            let mut state = ChartSettingsState {
                tab: Tab::Overlays,
                ..ChartSettingsState::default()
            };
            let mut card = card_with_overlays(count);
            let mut indicators = IndicatorConfig::default();
            let store = test_store();
            let mut last = None;
            for _ in 0..4 {
                last = Some(screen.run(Vec::new(), |ui| {
                    let _ = show(
                        ui.ctx(),
                        &mut state,
                        ChartSettingsContext {
                            title: "BTC/USDT",
                            symbol: "BTCUSDT",
                            source: DataSource::Binance,
                            loaded_candles: 0,
                            indicators: &mut indicators,
                            card: &mut card,
                            scale_mode: mode,
                            store: &store,
                        },
                    );
                }));
            }
            screen.save(name, last.expect("a frame"));
        }
    }
}
