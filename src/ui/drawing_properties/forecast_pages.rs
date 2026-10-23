//! The Inputs pages of the forecasting, volume and measurement tools: a position's account and
//! risk, a forecast's result colours, a bars pattern's mode, a ghost feed's size and candles, a
//! VWAP's source and bands, a profile's rows and value area, what a measurer prints.
//!
//! Every control here is one the drawing honours: nothing is offered that the painter or the
//! arithmetic ignores.

use super::{
    DrawingProperties, Icon, Id, Page, Ui, color_card, color_well, number_field, picker_row,
    settings_card, toggle_row,
};
use crate::model::bars_pattern::BarsMode;
use crate::model::chart_type::PriceSource;
use crate::model::forecast_style::{
    MAX_ACCOUNT, MAX_BAND_MULTIPLIER, MAX_QTY_PRECISION, MAX_ROW_TICKS, ProfileVolume, RowsLayout,
};
use crate::model::volume_profile::MAX_ROWS;
use crate::model::{Caps, Geometry};

/// The names of the places a quantity can be rounded to.
const QTY_PLACES: [&str; 9] = [
    "0 places", "1 place", "2 places", "3 places", "4 places", "5 places", "6 places", "7 places",
    "8 places",
];

/// A card with a number in it. `Some` is the new value when it was edited.
#[allow(clippy::too_many_arguments)]
fn number_card(
    ui: &mut Ui,
    icon: Icon,
    title: &str,
    hint: Option<&str>,
    id: &'static str,
    value: f64,
    step: f64,
    range: std::ops::RangeInclusive<f64>,
    decimals: usize,
) -> Option<f64> {
    settings_card::card(ui, icon, title, hint, |ui| {
        number_field(ui, Id::new(id), value, step, range, decimals)
    })
}

impl DrawingProperties {
    /// Whichever of the inputs pages this tool has.
    pub(super) fn forecast_inputs_page(&mut self, ui: &mut Ui) {
        let caps = self.tool.caps();
        if caps.has(Caps::POSITION) {
            self.position_inputs(ui);
        } else if caps.has(Caps::FORECAST) {
            self.forecast_inputs(ui);
        } else if caps.has(Caps::BARS) {
            self.bars_inputs(ui);
        } else if caps.has(Caps::GHOST) {
            self.ghost_inputs(ui);
        } else if caps.has(Caps::VWAP) {
            self.vwap_inputs(ui);
        } else if caps.has(Caps::PROFILE) {
            self.profile_inputs(ui);
        } else if caps.has(Caps::MEASURE) {
            self.measure_inputs(ui);
        }
    }

    // MARK: Positions

    fn position_inputs(&mut self, ui: &mut Ui) {
        let mut style = self.style();
        let before = style;
        let mut page = Page::new(true);
        let options = &mut style.position;
        page.section(
            ui,
            "Account",
            Some("A drawing only: it never places an order, and what it says is hypothetical."),
        );
        if let Some(value) = number_card(
            ui,
            Icon::Hash,
            "Account size",
            Some("The balance the risk and the size are worked from."),
            "position-account",
            options.account,
            100.0,
            0.0..=MAX_ACCOUNT,
            2,
        ) {
            options.account = value;
        }
        if let Some(value) = number_card(
            ui,
            Icon::Hash,
            "Lot size",
            Some("The smallest amount traded: a quantity is in lots."),
            "position-lot",
            options.lot,
            1.0,
            1e-9..=1e9,
            4,
        ) {
            options.lot = value;
        }
        let units = [(true, "Percent of account"), (false, "Amount")];
        picker_row(
            ui,
            Icon::Percent,
            "Risk is",
            None,
            &units,
            &mut options.risk_percent,
        );
        let risk_max = if options.risk_percent {
            100.0
        } else {
            MAX_ACCOUNT
        };
        if let Some(value) = number_card(
            ui,
            Icon::Percent,
            "Risk",
            Some("What is lost if the stop is reached."),
            "position-risk",
            options.risk,
            1.0,
            0.0..=risk_max,
            2,
        ) {
            options.risk = value;
        }
        if let Some(value) = number_card(
            ui,
            Icon::Hash,
            "Leverage",
            Some("Caps the size at the account times this over the entry price."),
            "position-leverage",
            options.leverage,
            1.0,
            0.01..=1_000.0,
            2,
        ) {
            options.leverage = value;
        }
        let places: Vec<(u8, &str)> = (0..=MAX_QTY_PRECISION)
            .map(|places| (places, QTY_PLACES[usize::from(places)]))
            .collect();
        picker_row(
            ui,
            Icon::Hash,
            "Quantity precision",
            None,
            &places,
            &mut options.qty_precision,
        );
        page.section(ui, "Stats", None);
        toggle_row(
            ui,
            Icon::TextFormat,
            "Compact stats",
            "Shorter tags.",
            &mut options.compact,
        );
        toggle_row(
            ui,
            Icon::Eye,
            "Always show stats",
            "Off shows them only while the position is selected.",
            &mut options.always_stats,
        );
        page.section(ui, "Colors", None);
        color_card(ui, Icon::Palette, "Target", None, &mut options.profit);
        color_card(ui, Icon::Palette, "Stop", None, &mut options.stop);
        color_card(ui, Icon::Palette, "Stats plate", None, &mut options.label);
        color_card(
            ui,
            Icon::Palette,
            "Target reached",
            None,
            &mut options.success,
        );
        color_card(
            ui,
            Icon::Palette,
            "Stop reached",
            None,
            &mut options.failure,
        );
        if style != before {
            self.set_style(style);
        }
    }

    fn forecast_inputs(&mut self, ui: &mut Ui) {
        let mut style = self.style();
        let before = style;
        let mut page = Page::new(true);
        page.section(
            ui,
            "Result",
            Some("Shown once the bars say whether the price reached the target in time."),
        );
        color_card(
            ui,
            Icon::Palette,
            "Success",
            None,
            &mut style.position.success,
        );
        color_card(
            ui,
            Icon::Palette,
            "Failure",
            None,
            &mut style.position.failure,
        );
        if style != before {
            self.set_style(style);
        }
    }

    // MARK: Bars pattern

    fn bars_inputs(&mut self, ui: &mut Ui) {
        let Some(Geometry::Bars(pattern)) = self.draft.geometry().cloned() else {
            return;
        };
        let mut edited = pattern.clone();
        let mut page = Page::new(true);
        page.section(
            ui,
            "Bars",
            Some("A copy kept in the drawing: later changes to the chart do not move it."),
        );
        let modes: Vec<(BarsMode, &str)> = BarsMode::ALL
            .iter()
            .map(|mode| (*mode, mode.label()))
            .collect();
        picker_row(ui, Icon::ChartLine, "Mode", None, &modes, &mut edited.mode);
        toggle_row(
            ui,
            Icon::ChevronUpDown,
            "Mirrored",
            "Reflect the pattern vertically.",
            &mut edited.mirrored,
        );
        toggle_row(
            ui,
            Icon::ArrowLeftRight,
            "Flipped",
            "Run the pattern backwards in time.",
            &mut edited.flipped,
        );
        page.section(ui, "Copied from", None);
        settings_card::card(
            ui,
            Icon::Clock,
            &format!("{} bars", edited.bars.len()),
            Some(&format!(
                "{} to {}",
                crate::format::timestamp(edited.from_ms),
                crate::format::timestamp(edited.to_ms)
            )),
            |_| {},
        );
        if edited != pattern
            && let Some(geometry) = self.draft.geometry_mut()
        {
            *geometry = Geometry::Bars(edited);
        }
    }

    // MARK: Ghost feed

    fn ghost_inputs(&mut self, ui: &mut Ui) {
        let mut style = self.style();
        let before = style;
        let mut page = Page::new(true);
        let options = &mut style.ghost;
        page.section(
            ui,
            "Candles",
            Some("A made-up run between the two points: not a forecast, and never real candles."),
        );
        if let Some(value) = number_card(
            ui,
            Icon::Percent,
            "Average high-low",
            Some("A candle's range, in percent of the first price."),
            "ghost-average",
            f64::from(options.average),
            0.1,
            0.0..=100.0,
            2,
        ) {
            options.average = value as f32;
        }
        if let Some(value) = number_card(
            ui,
            Icon::Percent,
            "Variance",
            Some("How much the ranges and the path wander, in percent."),
            "ghost-variance",
            f64::from(options.variance),
            5.0,
            0.0..=100.0,
            0,
        ) {
            options.variance = value as f32;
        }
        page.section(ui, "Colors", None);
        color_card(ui, Icon::Palette, "Up candle", None, &mut options.up);
        color_card(ui, Icon::Palette, "Down candle", None, &mut options.down);
        toggle_row(ui, Icon::Eye, "Borders", "", &mut options.draw_border);
        if options.draw_border {
            color_card(ui, Icon::Palette, "Up border", None, &mut options.up_border);
            color_card(
                ui,
                Icon::Palette,
                "Down border",
                None,
                &mut options.down_border,
            );
        }
        toggle_row(ui, Icon::Eye, "Wicks", "", &mut options.draw_wick);
        if options.draw_wick {
            color_card(ui, Icon::Palette, "Wick", None, &mut options.wick);
        }
        if style != before {
            self.set_style(style);
        }
        // Another run: the same inputs and a different seed draw different candles.
        page.section(ui, "Run", None);
        let clicked = settings_card::card(
            ui,
            Icon::Switch,
            "Draw another run",
            Some("The same size, different candles."),
            |ui| ui.button("New run").clicked(),
        );
        if clicked && let Some(Geometry::Ghost { seed, .. }) = self.draft.geometry_mut() {
            *seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        }
    }

    // MARK: Anchored VWAP

    fn vwap_inputs(&mut self, ui: &mut Ui) {
        let mut style = self.style();
        let before = style;
        let mut page = Page::new(true);
        let options = &mut style.vwap;
        page.section(
            ui,
            "Inputs",
            Some("Worked from the volume of the bars the chart draws: no volume, no line."),
        );
        let sources: Vec<(PriceSource, &str)> = PriceSource::ALL
            .iter()
            .map(|source| (*source, source.label()))
            .collect();
        picker_row(
            ui,
            Icon::ChartLine,
            "Source",
            Some("The price that is averaged."),
            &sources,
            &mut options.source,
        );
        toggle_row(
            ui,
            Icon::Eye,
            "Fill the bands",
            "Shade between each band and its mirror.",
            &mut options.fill,
        );
        for (index, band) in options.bands.iter_mut().enumerate() {
            page.titled_section(ui, &format!("Band {}", index + 1), None);
            settings_card::card(ui, Icon::Eye, "Show", None, |ui| {
                color_well(ui, "Band color", &mut band.color);
                settings_card::switch(ui, &mut band.enabled);
            });
            if band.enabled {
                let id = ["vwap-band-1", "vwap-band-2", "vwap-band-3"][index];
                if let Some(value) = number_card(
                    ui,
                    Icon::Hash,
                    "Standard deviations",
                    Some("How far from the VWAP, either side."),
                    id,
                    f64::from(band.multiplier),
                    0.5,
                    0.0..=f64::from(MAX_BAND_MULTIPLIER),
                    1,
                ) {
                    band.multiplier = value as f32;
                }
            }
        }
        if style != before {
            self.set_style(style);
        }
    }

    // MARK: Volume profile

    fn profile_inputs(&mut self, ui: &mut Ui) {
        let mut style = self.style();
        let before = style;
        let mut page = Page::new(true);
        let options = &mut style.profile;
        page.section(
            ui,
            "Rows",
            Some(
                "The volume of each bar is spread over its high to low: an approximation, not \
                 tick data.",
            ),
        );
        let layouts: Vec<(RowsLayout, &str)> = RowsLayout::ALL
            .iter()
            .map(|layout| (*layout, layout.label()))
            .collect();
        picker_row(
            ui,
            Icon::Sliders,
            "Rows layout",
            None,
            &layouts,
            &mut options.layout,
        );
        match options.layout {
            RowsLayout::Rows => {
                if let Some(value) = number_card(
                    ui,
                    Icon::Hash,
                    "Row size",
                    Some("How many rows the range is cut into."),
                    "profile-rows",
                    f64::from(options.rows),
                    1.0,
                    1.0..=f64::from(MAX_ROWS),
                    0,
                ) {
                    options.rows = value as u32;
                }
            }
            RowsLayout::Ticks => {
                if let Some(value) = number_card(
                    ui,
                    Icon::Hash,
                    "Row size",
                    Some("Each row's height, in steps of the chart's last printed price place."),
                    "profile-ticks",
                    f64::from(options.ticks),
                    1.0,
                    1.0..=f64::from(MAX_ROW_TICKS),
                    0,
                ) {
                    options.ticks = value as u32;
                }
            }
        }
        let volumes: Vec<(ProfileVolume, &str)> = ProfileVolume::ALL
            .iter()
            .map(|volume| (*volume, volume.label()))
            .collect();
        picker_row(
            ui,
            Icon::ChartLine,
            "Volume",
            None,
            &volumes,
            &mut options.volume,
        );
        if let Some(value) = number_card(
            ui,
            Icon::Percent,
            "Value area volume",
            Some(
                "The share of the volume the value area holds, grown out from the point of control.",
            ),
            "profile-value-area",
            f64::from(options.value_area),
            1.0,
            0.0..=100.0,
            0,
        ) {
            options.value_area = value as f32;
        }
        page.section(ui, "Look", None);
        if let Some(value) = number_card(
            ui,
            Icon::Percent,
            "Width",
            Some("The longest bar, in percent of the range's width."),
            "profile-width",
            f64::from(options.width),
            5.0,
            5.0..=100.0,
            0,
        ) {
            options.width = value as f32;
        }
        let placements = [(false, "Left"), (true, "Right")];
        picker_row(
            ui,
            Icon::ArrowLeftRight,
            "Placement",
            None,
            &placements,
            &mut options.right,
        );
        toggle_row(
            ui,
            Icon::Eye,
            "Point of control",
            "The row with the most volume.",
            &mut options.poc,
        );
        toggle_row(
            ui,
            Icon::Eye,
            "Value area high and low",
            "",
            &mut options.value_area_lines,
        );
        toggle_row(
            ui,
            Icon::ArrowLeftRight,
            "Extend point of control",
            "Run it on to the edge of the chart.",
            &mut options.extend_poc,
        );
        page.section(ui, "Colors", None);
        color_card(ui, Icon::Palette, "Up volume", None, &mut options.up);
        color_card(ui, Icon::Palette, "Down volume", None, &mut options.down);
        color_card(
            ui,
            Icon::Palette,
            "Point of control",
            None,
            &mut options.poc_color,
        );
        color_card(
            ui,
            Icon::Palette,
            "Value area lines",
            None,
            &mut options.value_area_color,
        );
        if style != before {
            self.set_style(style);
        }
    }

    // MARK: Measurers

    fn measure_inputs(&mut self, ui: &mut Ui) {
        let Some(kind) = (match self.draft.geometry() {
            Some(Geometry::Measure { kind, .. }) => Some(*kind),
            _ => None,
        }) else {
            return;
        };
        let mut style = self.style();
        let before = style;
        let mut page = Page::new(true);
        let options = &mut style.measure;
        if kind.has_price() {
            page.section(ui, "Price", None);
            toggle_row(ui, Icon::Hash, "Price change", "", &mut options.price);
            toggle_row(
                ui,
                Icon::Percent,
                "Percent change",
                "",
                &mut options.percent,
            );
            toggle_row(
                ui,
                Icon::Hash,
                "Change in ticks",
                "Steps of the chart's last printed price place.",
                &mut options.ticks,
            );
        }
        if kind.has_time() {
            page.section(ui, "Time", None);
            toggle_row(ui, Icon::Hash, "Bars", "", &mut options.bars);
            toggle_row(ui, Icon::Clock, "Date and time", "", &mut options.time);
            toggle_row(
                ui,
                Icon::ChartLine,
                "Volume",
                "Shown when the market has any.",
                &mut options.volume,
            );
        }
        page.section(ui, "Label", None);
        color_card(
            ui,
            Icon::Palette,
            "Label background",
            None,
            &mut options.label,
        );
        if style != before {
            self.set_style(style);
        }
    }
}
