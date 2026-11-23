//! The Inputs pages of the forecasting, volume and measurement tools: a position's account and
//! risk, a forecast's result colours, a bars pattern's mode, a ghost feed's size and candles, a
//! VWAP's source and bands, a profile's rows and value area, what a measurer prints.
//!
//! Every control here is one the drawing honours: nothing is offered that the painter or the
//! arithmetic ignores.

use super::rows::{Page, color_card, color_well, toggle_row};
use super::{DrawingProperties, Icon, Id, Ui, card, number_field, picker_row};
use crate::i18n::{tr, tr_n};
use crate::model::chart::kind::PriceSource;
use crate::model::forecast::style::{
    MAX_ACCOUNT, MAX_BAND_MULTIPLIER, MAX_QTY_PRECISION, MAX_ROW_TICKS, ProfileVolume, RowsLayout,
};
use crate::model::forecast::volume_profile::MAX_ROWS;
use crate::model::pattern::bars::BarsMode;
use crate::model::{Caps, Geometry};

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
    card::card(ui, icon, title, hint, |ui| {
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
            tr!("Account"),
            Some(tr!(
                "A drawing only: it never places an order, and what it says is hypothetical."
            )),
        );
        if let Some(value) = number_card(
            ui,
            Icon::Hash,
            tr!("Account size"),
            Some(tr!("The balance the risk and the size are worked from.")),
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
            tr!("Lot size"),
            Some(tr!("The smallest amount traded: a quantity is in lots.")),
            "position-lot",
            options.lot,
            1.0,
            1e-9..=1e9,
            4,
        ) {
            options.lot = value;
        }
        let units = [(true, tr!("% of account")), (false, tr!("Amount"))];
        picker_row(
            ui,
            Icon::Percent,
            tr!("Risk is"),
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
            tr!("Risk"),
            Some(tr!("What is lost if the stop is reached.")),
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
            tr!("Leverage"),
            Some(tr!(
                "Caps the size at the account times this over the entry price."
            )),
            "position-leverage",
            options.leverage,
            1.0,
            0.01..=1_000.0,
            2,
        ) {
            options.leverage = value;
        }
        let names: Vec<(u8, String)> = (0..=MAX_QTY_PRECISION)
            .map(|places| (places, tr_n!(places, "{count} place", "{count} places")))
            .collect();
        let places: Vec<(u8, &str)> = names
            .iter()
            .map(|(places, name)| (*places, name.as_str()))
            .collect();
        picker_row(
            ui,
            Icon::Hash,
            tr!("Quantity precision"),
            None,
            &places,
            &mut options.qty_precision,
        );
        page.section(ui, tr!("Stats"), None);
        toggle_row(
            ui,
            Icon::TextFormat,
            tr!("Compact stats"),
            tr!("Shorter tags."),
            &mut options.compact,
        );
        toggle_row(
            ui,
            Icon::Eye,
            tr!("Always show stats"),
            tr!("Off shows them only while the position is selected."),
            &mut options.always_stats,
        );
        page.section(ui, tr!("Colors"), None);
        color_card(ui, Icon::Palette, tr!("Target"), None, &mut options.profit);
        color_card(ui, Icon::Palette, tr!("Stop"), None, &mut options.stop);
        color_card(
            ui,
            Icon::Palette,
            tr!("Stats plate"),
            None,
            &mut options.label,
        );
        color_card(
            ui,
            Icon::Palette,
            tr!("Target reached"),
            None,
            &mut options.success,
        );
        color_card(
            ui,
            Icon::Palette,
            tr!("Stop reached"),
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
            tr!("Result"),
            Some(tr!(
                "Shown once the bars say whether the price reached the target in time."
            )),
        );
        color_card(
            ui,
            Icon::Palette,
            tr!("Success"),
            None,
            &mut style.position.success,
        );
        color_card(
            ui,
            Icon::Palette,
            tr!("Failure"),
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
            tr!("Bars"),
            Some(tr!(
                "A copy kept in the drawing: later changes to the chart do not move it."
            )),
        );
        let modes: Vec<(BarsMode, &str)> = BarsMode::ALL
            .iter()
            .map(|mode| (*mode, mode.label()))
            .collect();
        picker_row(
            ui,
            Icon::ChartLine,
            tr!("Mode"),
            None,
            &modes,
            &mut edited.mode,
        );
        toggle_row(
            ui,
            Icon::ChevronUpDown,
            tr!("Mirrored"),
            tr!("Reflect the pattern vertically."),
            &mut edited.mirrored,
        );
        toggle_row(
            ui,
            Icon::ArrowLeftRight,
            tr!("Flipped"),
            tr!("Run the pattern backwards in time."),
            &mut edited.flipped,
        );
        page.section(ui, tr!("Copied from"), None);
        card::card(
            ui,
            Icon::Clock,
            &tr_n!(edited.bars.len(), "{count} bar", "{count} bars"),
            Some(&tr!(
                "{from} to {to}",
                from = crate::format::timestamp(edited.from_ms),
                to = crate::format::timestamp(edited.to_ms)
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
            tr!("Candles"),
            Some(tr!(
                "A made-up run between the two points: not a forecast, and never real candles."
            )),
        );
        if let Some(value) = number_card(
            ui,
            Icon::Percent,
            tr!("Average high-low"),
            Some(tr!(
                "A candle's range, in percent of the first price. Zero takes the average range of the chart's last 20 bars."
            )),
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
            tr!("Variance"),
            Some(tr!("How much the ranges and the path wander, in percent.")),
            "ghost-variance",
            f64::from(options.variance),
            5.0,
            0.0..=100.0,
            0,
        ) {
            options.variance = value as f32;
        }
        page.section(ui, tr!("Colors"), None);
        color_card(ui, Icon::Palette, tr!("Up candle"), None, &mut options.up);
        color_card(
            ui,
            Icon::Palette,
            tr!("Down candle"),
            None,
            &mut options.down,
        );
        toggle_row(ui, Icon::Eye, tr!("Borders"), "", &mut options.draw_border);
        if options.draw_border {
            color_card(
                ui,
                Icon::Palette,
                tr!("Up border"),
                None,
                &mut options.up_border,
            );
            color_card(
                ui,
                Icon::Palette,
                tr!("Down border"),
                None,
                &mut options.down_border,
            );
        }
        toggle_row(ui, Icon::Eye, tr!("Wicks"), "", &mut options.draw_wick);
        if options.draw_wick {
            color_card(ui, Icon::Palette, tr!("Wick"), None, &mut options.wick);
        }
        if style != before {
            self.set_style(style);
        }
        // Another run: the same inputs and a different seed draw different candles.
        page.section(ui, tr!("Run"), None);
        let clicked = card::card(
            ui,
            Icon::Switch,
            tr!("Draw another run"),
            Some(tr!("The same size, different candles.")),
            |ui| ui.button(tr!("New run")).clicked(),
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
            tr!("Inputs"),
            Some(tr!(
                "Worked from the volume of the bars the chart draws: no volume, no line."
            )),
        );
        let sources: Vec<(PriceSource, &str)> = PriceSource::ALL
            .iter()
            .map(|source| (*source, source.label()))
            .collect();
        picker_row(
            ui,
            Icon::ChartLine,
            tr!("Source"),
            Some(tr!("The price that is averaged.")),
            &sources,
            &mut options.source,
        );
        toggle_row(
            ui,
            Icon::Eye,
            tr!("Fill the bands"),
            tr!("Shade between each band and its mirror."),
            &mut options.fill,
        );
        for (index, band) in options.bands.iter_mut().enumerate() {
            page.titled_section(ui, &tr!("Band {number}", number = index + 1), None);
            card::card(ui, Icon::Eye, tr!("Show"), None, |ui| {
                color_well(ui, tr!("Band color"), &mut band.color);
                card::switch(ui, &mut band.enabled);
            });
            if band.enabled {
                let id = ["vwap-band-1", "vwap-band-2", "vwap-band-3"][index];
                if let Some(value) = number_card(
                    ui,
                    Icon::Hash,
                    tr!("Standard deviations"),
                    Some(tr!("How far from the VWAP, either side.")),
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
            tr!("Rows"),
            Some(tr!(
                "The volume of each bar is spread over its high to low: an approximation, not tick data."
            )),
        );
        let layouts: Vec<(RowsLayout, &str)> = RowsLayout::ALL
            .iter()
            .map(|layout| (*layout, layout.label()))
            .collect();
        picker_row(
            ui,
            Icon::Sliders,
            tr!("Rows layout"),
            None,
            &layouts,
            &mut options.layout,
        );
        match options.layout {
            RowsLayout::Rows => {
                if let Some(value) = number_card(
                    ui,
                    Icon::Hash,
                    tr!("Row size"),
                    Some(tr!("How many rows the range is cut into.")),
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
                    tr!("Row size"),
                    Some(tr!(
                        "Each row's height, in steps of the chart's last printed price place."
                    )),
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
            tr!("Volume"),
            None,
            &volumes,
            &mut options.volume,
        );
        if let Some(value) = number_card(
            ui,
            Icon::Percent,
            tr!("Value area volume"),
            Some(tr!(
                "The share of the volume the value area holds, grown out from the point of control."
            )),
            "profile-value-area",
            f64::from(options.value_area),
            1.0,
            0.0..=100.0,
            0,
        ) {
            options.value_area = value as f32;
        }
        page.section(ui, tr!("Look"), None);
        if let Some(value) = number_card(
            ui,
            Icon::Percent,
            tr!("Width"),
            Some(tr!("The longest bar, in percent of the range's width.")),
            "profile-width",
            f64::from(options.width),
            5.0,
            5.0..=100.0,
            0,
        ) {
            options.width = value as f32;
        }
        let placements = [(false, tr!("Left")), (true, tr!("Right"))];
        picker_row(
            ui,
            Icon::ArrowLeftRight,
            tr!("Placement"),
            None,
            &placements,
            &mut options.right,
        );
        toggle_row(
            ui,
            Icon::Eye,
            tr!("Point of control"),
            tr!("The row with the most volume."),
            &mut options.poc,
        );
        toggle_row(
            ui,
            Icon::Eye,
            tr!("Value area high and low"),
            "",
            &mut options.value_area_lines,
        );
        toggle_row(
            ui,
            Icon::ArrowLeftRight,
            tr!("Extend point of control"),
            tr!("Run it on to the edge of the chart."),
            &mut options.extend_poc,
        );
        page.section(ui, tr!("Colors"), None);
        color_card(ui, Icon::Palette, tr!("Up volume"), None, &mut options.up);
        color_card(
            ui,
            Icon::Palette,
            tr!("Down volume"),
            None,
            &mut options.down,
        );
        color_card(
            ui,
            Icon::Palette,
            tr!("Point of control"),
            None,
            &mut options.poc_color,
        );
        color_card(
            ui,
            Icon::Palette,
            tr!("Value area lines"),
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
            page.section(ui, tr!("Price"), None);
            toggle_row(ui, Icon::Hash, tr!("Price change"), "", &mut options.price);
            toggle_row(
                ui,
                Icon::Percent,
                tr!("Percent change"),
                "",
                &mut options.percent,
            );
            toggle_row(
                ui,
                Icon::Hash,
                tr!("Change in ticks"),
                tr!("Steps of the chart's last printed price place."),
                &mut options.ticks,
            );
        }
        if kind.has_time() {
            page.section(ui, tr!("Time"), None);
            toggle_row(ui, Icon::Hash, tr!("Bars"), "", &mut options.bars);
            toggle_row(ui, Icon::Clock, tr!("Date and time"), "", &mut options.time);
            toggle_row(
                ui,
                Icon::ChartLine,
                tr!("Volume"),
                tr!("Shown when the market has any."),
                &mut options.volume,
            );
        }
        page.section(ui, tr!("Label"), None);
        color_card(
            ui,
            Icon::Palette,
            tr!("Label background"),
            None,
            &mut options.label,
        );
        if style != before {
            self.set_style(style);
        }
    }
}
