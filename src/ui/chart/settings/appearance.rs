//! The Appearance page: the price axis (scale, decimals, small prices) with its example prices,
//! and the line colours with their preview.

use super::{ChartSettingsAction, ChartSettingsContext, templates};
use crate::model::chart::price_format::MAX_DECIMALS;
use crate::model::chart::price_scale::PriceScaleMode;
use crate::model::{ChartAppearance, PriceFormat, SmallPrices};
use crate::store::SharedStore;
use crate::ui::chart::card::ChartCard;
use crate::ui::chart::settings::templates::TemplateState;
use crate::ui::icons::Icon;
use crate::ui::kit;
use crate::ui::settings::card;
use crate::ui::theme;
use eframe::egui::{
    self, Align, Align2, Color32, FontId, Id, Layout, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2,
};

pub(super) fn appearance_page(
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
        super::style::show(
            ui,
            c.card,
            c.source,
            Id::new(("chart-style", c.symbol)),
            |ui, card| templates::menu(ui, templates, store, card),
        );
    }

    ui.add_space(16.0);
    price_axis(ui, c.card, c.scale_mode).map(ChartSettingsAction::SetScaleMode)
}

/// What the price scale reads, for the line under the Scale picker.
pub(super) fn scale_hint(mode: PriceScaleMode) -> &'static str {
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
pub(super) const PRICE_SAMPLES: [(&str, f64); 3] = [
    ("Large", 67_432.19),
    ("Small", 0.123_456),
    ("Tiny", 0.000_002_78),
];

/// How the chart writes a price (axis, crosshair, readouts, rulers): how many decimal places, and
/// how a price under 0.001 reads, with a preview of the choice.
///
/// The scale mode is the chart's, not the look's: it is not in the appearance or its templates, so
/// a pick is handed back for the workspace to set, the way the corner menu and the shortcuts do.
pub(super) fn price_axis(
    ui: &mut Ui,
    card: &mut ChartCard,
    mode: PriceScaleMode,
) -> Option<PriceScaleMode> {
    let appearance = card.appearance();
    let (decimals, small) = (appearance.axis_decimals, appearance.axis_small_prices);
    card::section_title(
        ui,
        "Price axis",
        Some("How prices are written on the axis, the crosshair and the readouts."),
    );
    // The section title tightens the spacing for its own subtitle; the cards keep the page's.
    ui.spacing_mut().item_spacing.y = 8.0;
    let mut picked = None;
    card::card(
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
    card::card(
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
    card::card_reserving(
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

pub(super) fn decimals_hint(decimals: Option<u8>) -> String {
    match decimals {
        None => "Picks the precision from the price: 67,432, 12.35, 0.12, 0.0031.".to_owned(),
        Some(places) => format!(
            "Shows {places} decimal place{} on the price axis.",
            if places == 1 { "" } else { "s" }
        ),
    }
}

pub(super) fn small_prices_hint(style: SmallPrices) -> String {
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
    card::section_title(ui, "Colors", Some("Used for rising and falling prices."));
    ui.spacing_mut().item_spacing.y = 8.0;
    card::card(
        ui,
        Icon::ArrowUpRight,
        "Line up",
        Some("Stretches where the price rises."),
        |ui| color_well(ui, "bullish", &mut c.card.appearance_mut().bullish),
    );
    card::card(
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
                templates::menu(ui, templates, store, card);
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
