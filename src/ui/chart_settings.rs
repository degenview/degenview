//! The chart settings sheet: Indicators, Scripts, Appearance and Canvas above "Remove Chart",
//! the Templates menu and "Done". Every control writes through to the chart as it changes.

use super::chart_card::ChartCard;
use super::chart_templates::{self, TemplateAction, TemplateState};
use super::icons::{self, Icon};
use super::kit::{self, TabItem};
use super::settings_card;
use super::sheet;
use super::theme;
use crate::model::pine_instance::{PineInstance, VisualOrder};
use crate::model::price_format::MAX_DECIMALS;
use crate::model::price_scale::PriceScaleMode;
use crate::model::{
    BOLLINGER_MULTIPLIER, BOLLINGER_PERIOD, ChartAppearance, DataSource, EMA_PERIODS,
    IndicatorConfig, PriceFormat, RSI_OVERBOUGHT, RSI_OVERSOLD, RSI_PERIOD, SUPERTREND_MULTIPLIER,
    SUPERTREND_PERIOD, SmallPrices,
};
use crate::script_manager::drawer;
use crate::script_manager::model::{LocalScript, ScriptType};
use crate::script_manager::store::ScriptStore;
use crate::store::SharedStore;
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Id, Layout, Margin, Rect,
    RichText, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use std::time::{Duration, Instant};

/// Swift's ideal sheet width.
const WIDTH: f32 = 760.0;
/// The scrolling page's height in a tall window; enough to show most Appearance and Canvas
/// sections at once, while still leaving room around the sheet.
const MAX_PAGE_HEIGHT: f32 = 650.0;
const MIN_PAGE_HEIGHT: f32 = 260.0;
/// What the sheet spends on everything but the page: header, tab bar, two rules and the footer.
const CHROME_HEIGHT: f32 = 250.0;
const SCRIPT_REFRESH: Duration = Duration::from_secs(2);

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
}

impl Tab {
    const ALL: [Self; 4] = [
        Self::Indicators,
        Self::Scripts,
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
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Indicators => Icon::Waveform,
            Self::Scripts => Icon::Curlybraces,
            Self::Appearance => Icon::Palette,
            Self::Canvas => Icon::MacWindow,
        }
    }
}

#[derive(Default)]
pub struct ChartSettingsState {
    pub tab: Tab,
    confirming_removal: bool,
    scripts: Vec<LocalScript>,
    scripts_read_at: Option<Instant>,
    templates: TemplateState,
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
    OpenScriptManager,
    /// Open (or put away) the Inputs and Style popover of an applied script, pointing at `Rect`.
    ConfigureScript(u64, Rect),
    /// Give every chart of the workspace this look.
    ApplyToAll(crate::model::chart_template::ApplyScope),
    /// Read the price axis in this mode.
    SetScaleMode(PriceScaleMode),
}

pub fn show(
    ctx: &egui::Context,
    state: &mut ChartSettingsState,
    mut c: ChartSettingsContext,
) -> Option<ChartSettingsAction> {
    let mut action = None;
    let was_confirming = state.confirming_removal;
    let was_templating = state.templates.has_dialog();
    let page_height = page_height(ctx.content_rect().height());
    let tabs = Tab::for_source(c.source);
    if !tabs.contains(&state.tab) {
        state.tab = tabs[0];
    }
    sheet::show(ctx, Id::new("chart-settings"), WIDTH, |ui| {
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
                            Tab::Canvas => {
                                let (store, templates) = (c.store, &mut state.templates);
                                super::chart_canvas_settings::show(
                                    ui,
                                    c.card,
                                    Id::new(("chart-canvas", c.symbol)),
                                    |ui, card| chart_templates::menu(ui, templates, store, card),
                                );
                            }
                        }
                    });
            });
        rule(ui);
        sheet::inset(ui, |ui| {
            if footer(ui, state) {
                action = Some(ChartSettingsAction::Done);
            }
        });
    });
    if state.confirming_removal {
        let title = format!("Remove {}?", c.title);
        let outcome = kit::confirm(
            ctx,
            Id::new("chart-settings-remove"),
            &kit::Confirm {
                icon: Icon::Trash,
                title: &title,
                message: "The chart leaves this tab; its drawings are kept. To change a chart's market, remove it and add a new one.",
                destructive: "Remove Chart",
            },
        );
        match outcome {
            Some(kit::ConfirmOutcome::Confirmed) => {
                state.confirming_removal = false;
                action = Some(ChartSettingsAction::RemoveChart);
            }
            Some(kit::ConfirmOutcome::Cancelled) => state.confirming_removal = false,
            None => {}
        }
    }
    // A template dialog takes Escape first; a second press closes the sheet.
    if let Some(TemplateAction::ApplyToAll(scope)) =
        chart_templates::dialogs(ctx, &mut state.templates, c.store, c.card)
    {
        action = Some(ChartSettingsAction::ApplyToAll(scope));
    }
    if action.is_none()
        && !was_confirming
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
/// right. (The Templates menu sits beside each page's Reset button.) Returns whether Done was
/// clicked.
fn footer(ui: &mut Ui, state: &mut ChartSettingsState) -> bool {
    let mut done = false;
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        if kit::destructive_sheet_button(ui, Icon::Trash, "Remove Chart").clicked() {
            state.confirming_removal = true;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            done = kit::sheet_button(ui, "Done", true, true).clicked();
            // What the last template action did, between the buttons.
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                chart_templates::notice(ui, &mut state.templates);
            });
        });
    });
    ui.add_space(14.0);
    done
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
    let card = kit::card_frame(false).show(ui, |ui| {
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
    kit::paint_card_highlight(ui, card.response.rect);
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
    let card = kit::card_frame(false).show(ui, |ui| {
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
    kit::paint_card_highlight(ui, card.response.rect);
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

/// What one row of the applied list shows, copied out so drawing needs no borrow of the chart.
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
}

enum RowEvent {
    ToggleVisible(u64),
    Remove(u64),
    MoveTo(u64, usize),
    /// The script's settings, anchored to the control that asked.
    Configure(u64, Rect),
    /// Where the script's visuals sit against the chart's candles.
    SetOrder(u64, VisualOrder),
}

fn refresh_scripts(state: &mut ChartSettingsState) {
    if cfg!(test) {
        return;
    }
    if state
        .scripts_read_at
        .is_some_and(|at| at.elapsed() < SCRIPT_REFRESH)
    {
        return;
    }
    state.scripts_read_at = Some(Instant::now());
    if let Some(mut store) = ScriptStore::default_location()
        && let Ok(mut scripts) = store.all_scripts()
    {
        scripts.retain(|s| s.script_type != ScriptType::Library);
        scripts.sort_by_key(|s| s.name.to_lowercase());
        state.scripts = scripts;
    }
}

fn scripts_page(
    ui: &mut Ui,
    state: &mut ChartSettingsState,
    c: &mut ChartSettingsContext,
) -> Option<ChartSettingsAction> {
    refresh_scripts(state);
    let mut action = None;
    let mut add: Option<LocalScript> = None;
    let has_saved = !state.scripts.is_empty();

    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            settings_card::section_title(
                ui,
                "Scripts",
                Some("Pine scripts running on this chart."),
            );
        });
        if has_saved {
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                let button = kit::push_button(ui, Some(Icon::Plus), "Add Script", true)
                    .on_hover_text("Apply a script to this chart — add the same one again with different inputs");
                egui::Popup::menu(&button)
                    .frame(kit::menu_frame())
                    .show(|ui| {
                        ui.set_min_width(220.0);
                        for (heading, kind, icon) in [
                            ("Indicators", ScriptType::Indicator, Icon::ChartLine),
                            ("Strategies", ScriptType::Strategy, Icon::Cycle),
                        ] {
                            let scripts: Vec<&LocalScript> = state
                                .scripts
                                .iter()
                                .filter(|s| s.script_type == kind)
                                .collect();
                            if scripts.is_empty() {
                                continue;
                            }
                            ui.horizontal(|ui| {
                                ui.add_space(8.0);
                                ui.label(kit::text(
                                    heading,
                                    kit::size::CAPTION + 0.5,
                                    kit::SEMIBOLD,
                                    theme::text_secondary(),
                                ));
                            });
                            for script in scripts {
                                if kit::menu_item(ui, icon, &script.name, true, false) {
                                    add = Some(script.clone());
                                    ui.close();
                                }
                            }
                            kit::menu_separator(ui);
                        }
                        if kit::menu_item(ui, Icon::Curlybraces, "Script Manager…", true, false) {
                            action = Some(ChartSettingsAction::OpenScriptManager);
                            ui.close();
                        }
                    });
            });
        }
    });
    if let Some(script) = add {
        c.card
            .add_pine_instance(Some(script.id.clone()), &script.name, &script.source);
    }

    let instances: Vec<PineInstance> = c.card.pine_instances().to_vec();
    if instances.is_empty() {
        empty_state(ui, has_saved, &mut action);
        return action;
    }

    // Copy what the rows show out of the chart, so drawing needs no borrow of it.
    let rows: Vec<RowData> = instances
        .iter()
        .map(|instance| {
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
            }
        })
        .collect();
    let bars = instances
        .iter()
        .filter_map(|i| c.card.pine_status(i.id))
        .map(|s| s.output.bar_count)
        .max()
        .filter(|bars| *bars > 0);

    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.label(kit::text(
            "Applied",
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

    let mut events: Vec<RowEvent> = Vec::new();
    let list = kit::card_frame(false)
        .inner_margin(Margin::ZERO)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let count = rows.len();
            for (index, row) in rows.iter().enumerate() {
                if index > 0 {
                    let y = ui.cursor().min.y;
                    let rect = ui.max_rect();
                    ui.painter().line_segment(
                        [pos2(rect.min.x + 54.0, y), pos2(rect.max.x - 12.0, y)],
                        Stroke::new(1.0, kit::hairline()),
                    );
                }
                instance_row(ui, row, index, count, &mut events);
            }
        });
    kit::paint_card_highlight(ui, list.response.rect);

    for event in events {
        match event {
            RowEvent::ToggleVisible(id) => {
                let visible = instances
                    .iter()
                    .find(|i| i.id == id)
                    .is_some_and(|i| i.visible);
                c.card.set_pine_visible(id, !visible);
            }
            RowEvent::Remove(id) => {
                c.card.remove_pine_instance(id);
            }
            RowEvent::MoveTo(id, index) => c.card.move_pine_instance(id, index),
            RowEvent::SetOrder(id, order) => c.card.set_pine_visual_order(id, order),
            RowEvent::Configure(id, anchor) => {
                action = Some(ChartSettingsAction::ConfigureScript(id, anchor));
            }
        }
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
fn empty_state(ui: &mut Ui, has_saved: bool, action: &mut Option<ChartSettingsAction>) {
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
                    if has_saved {
                        Icon::ChartLine
                    } else {
                        Icon::Curlybraces
                    },
                    theme::text_secondary(),
                );
                ui.add_space(12.0);
                let (title, message) = if has_saved {
                    (
                        "No scripts on this chart",
                        "Choose Add Script to overlay an indicator or strategy on the candles.",
                    )
                } else {
                    (
                        "No scripts yet",
                        "Write one in the Script Manager and it shows up here.",
                    )
                };
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
                if !has_saved {
                    ui.add_space(12.0);
                    if kit::sheet_button(ui, "Open Script Manager", true, true).clicked() {
                        *action = Some(ChartSettingsAction::OpenScriptManager);
                    }
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

    // A wash under the pointer. The first and last rows round the corners they share with the
    // card, so the wash never pokes out of it.
    if response.hovered() {
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

    // A neutral tile, as on every settings card. Colour says something here: orange when the
    // script fails, the accent while it draws, and nothing once it is hidden.
    let badge = Rect::from_center_size(pos2(rect.min.x + 27.0, rect.center().y), Vec2::splat(30.0));
    let tint = if failing {
        ui.painter()
            .rect_filled(badge, 8.0, theme::with_alpha(theme::ORANGE, 0.14));
        ui.painter().rect_stroke(
            badge,
            8.0,
            Stroke::new(1.0, theme::with_alpha(theme::ORANGE, 0.3)),
            StrokeKind::Inside,
        );
        theme::ORANGE
    } else {
        ui.painter().rect_filled(badge, 8.0, theme::ink(10));
        ui.painter().rect_stroke(
            badge,
            8.0,
            Stroke::new(1.0, theme::ink(14)),
            StrokeKind::Inside,
        );
        if row.visible {
            theme::SYSTEM_ACCENT
        } else {
            theme::text_tertiary()
        }
    };
    let kind_icon = if failing {
        Icon::TriangleFill
    } else if row.is_strategy {
        Icon::Cycle
    } else {
        Icon::ChartLine
    };
    icons::paint(
        ui.painter(),
        Rect::from_center_size(badge.center(), Vec2::splat(15.0)),
        kind_icon,
        tint,
    );

    // Controls, right to left: remove, settings, eye.
    let mut x = rect.max.x - 12.0;
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
    let eye_tip = if row.visible {
        format!("Hide {} on the chart", row.title)
    } else {
        format!("Show {} on the chart", row.title)
    };
    let eye = if row.visible {
        Icon::Eye
    } else {
        Icon::EyeSlash
    };
    if button(ui, eye, &eye_tip, false).clicked() {
        events.push(RowEvent::ToggleVisible(row.id));
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
                parts.push("Hidden".to_owned());
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
        let label = if row.visible {
            "Hide on Chart"
        } else {
            "Show on Chart"
        };
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
        ui.separator();
        if ui
            .add_enabled(index > 0, egui::Button::new("Move Up"))
            .clicked()
        {
            events.push(RowEvent::MoveTo(row.id, index - 1));
            ui.close();
        }
        if ui
            .add_enabled(index + 1 < count, egui::Button::new("Move Down"))
            .clicked()
        {
            events.push(RowEvent::MoveTo(row.id, index + 1));
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
    use std::sync::{Arc, Mutex};

    fn test_store() -> SharedStore {
        Arc::new(Mutex::new(Store::open_in_memory().expect("open")))
    }

    fn draw(
        state: &mut ChartSettingsState,
        card: &mut ChartCard,
        indicators: &mut IndicatorConfig,
    ) {
        let ctx = egui::Context::default();
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
    fn the_sheet_draws_the_removal_alert_and_each_line_source() {
        let mut state = ChartSettingsState::default();
        let mut card = ChartCard::default();
        let mut indicators = IndicatorConfig::default();
        state.confirming_removal = true;
        draw(&mut state, &mut card, &mut indicators);
        assert!(
            state.confirming_removal,
            "nothing dismisses the alert unasked"
        );
        state.confirming_removal = false;
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
            ["Indicators", "Scripts", "Appearance", "Canvas"]
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
}
