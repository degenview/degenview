//! The settings rows of the Volume footprint chart type.
//!
//! Only controls that do something are here. *Tick-based calculation* is offered where the market
//! has trades (Binance) and shown off, with the reason, where it does not; imbalance controls say
//! that only bars built from trades have imbalances.

use super::{Pair, candle_rows, color_row, int_row, picker_row, switch_row};
use crate::model::DataSource;
use crate::model::chart_type::{
    FootprintDisplay, FootprintKind, FootprintSettings, FootprintSummary, RowMode,
};
use crate::ui::icons::Icon;
use crate::ui::settings_card;
use eframe::egui::{Id, Ui};

/// Whether `source` has trades to build a footprint from (so tick-based calculation can be on).
pub fn has_trades(source: DataSource) -> bool {
    source == DataSource::Binance
}

pub(super) fn rows(
    ui: &mut Ui,
    id: Id,
    settings: &mut FootprintSettings,
    palette: Pair,
    source: DataSource,
) {
    // Calculation
    let trades = has_trades(source);
    let mut on = settings.tick_based && trades;
    settings_card::card(
        ui,
        Icon::Waveform,
        "Tick-based calculation",
        Some(if trades {
            "Build each bar from the exchange's own trades, each with its buyer or seller side. Bars the trades do not cover use finer candles."
        } else {
            "Needs trade data, which only Binance markets have here. This market's bars are built from finer candles and are estimates."
        }),
        |ui| {
            ui.add_enabled_ui(trades, |ui| settings_card::switch(ui, &mut on));
        },
    );
    if trades {
        settings.tick_based = on;
    }
    picker_row(
        ui,
        Icon::Sliders,
        "Row size",
        Some("Automatic is a fifth of the average true range."),
        &[(RowMode::Auto, "Auto"), (RowMode::Manual, "Manual")],
        &mut settings.row_mode,
    );
    match settings.row_mode {
        RowMode::Auto => {
            let mut length = i64::from(settings.atr_length);
            if int_row(
                ui,
                id.with("atr"),
                Icon::Sliders,
                "ATR length",
                None,
                &mut length,
                1..=500,
            ) {
                settings.atr_length = length as u32;
            }
        }
        RowMode::Manual => {
            let mut ticks = i64::from(settings.ticks_per_row);
            if int_row(
                ui,
                id.with("ticks"),
                Icon::Sliders,
                "Ticks per row",
                Some("A row this many price steps tall."),
                &mut ticks,
                1..=100_000,
            ) {
                settings.ticks_per_row = ticks as u32;
            }
        }
    }
    // Appearance
    picker_row(
        ui,
        Icon::BarChart,
        "Display",
        Some("Cluster draws equal cells, Profile cells as wide as their volume."),
        &[
            (FootprintDisplay::Cluster, "Cluster"),
            (FootprintDisplay::Profile, "Profile"),
        ],
        &mut settings.display,
    );
    picker_row(
        ui,
        Icon::BarChart,
        "Type",
        None,
        &[
            (FootprintKind::BuyAndSell, "Buy and sell"),
            (FootprintKind::Delta, "Delta"),
            (FootprintKind::Total, "Total"),
            (FootprintKind::Ladder, "Ladder"),
        ],
        &mut settings.kind,
    );
    switch_row(
        ui,
        Icon::Palette,
        "Gradient",
        "Shade each cell by its volume against the busiest on screen, in four steps.",
        &mut settings.gradient,
    );
    color_row(
        ui,
        Icon::ArrowUpRight,
        "Buy colour",
        Some("Follows the bullish colour until set."),
        &mut settings.buy_color,
        palette.0,
    );
    color_row(
        ui,
        Icon::ArrowDownRight,
        "Sell colour",
        Some("Follows the bearish colour until set."),
        &mut settings.sell_color,
        palette.1,
    );
    // Value area and point of control
    switch_row(
        ui,
        Icon::Layers,
        "Value area",
        "Shade the rows that hold the share of each bar's volume set below.",
        &mut settings.show_value_area,
    );
    let mut percent = i64::from(settings.value_area_percent);
    if int_row(
        ui,
        id.with("va"),
        Icon::Percent,
        "Value area volume",
        Some("The share of a bar's volume the value area holds, in percent."),
        &mut percent,
        1..=100,
    ) {
        settings.value_area_percent = percent as u32;
    }
    switch_row(
        ui,
        Icon::Eye,
        "Point of control",
        "Outline the row with the most volume.",
        &mut settings.show_poc,
    );
    picker_row(
        ui,
        Icon::Hash,
        "Summary",
        Some("The delta and total volume written under each bar."),
        &[
            (FootprintSummary::InfoBox, "Info box"),
            (FootprintSummary::Off, "Off"),
        ],
        &mut settings.summary,
    );
    // Imbalances
    let mut imbalance = i64::from(settings.imbalance_percent);
    if int_row(
        ui,
        id.with("imbalance"),
        Icon::Percent,
        "Imbalance",
        Some(
            "A buy imbalance is a row whose buy volume is this percent of the sell volume one row below (and the other way for sells). Only bars built from trades have imbalances.",
        ),
        &mut imbalance,
        100..=10_000,
    ) {
        settings.imbalance_percent = imbalance as u32;
    }
    switch_row(
        ui,
        Icon::Eye,
        "Highlight imbalances",
        "Mark imbalanced rows at the cell's outer edge.",
        &mut settings.highlight_imbalances,
    );
    color_row(
        ui,
        Icon::ArrowUpRight,
        "Buy imbalance colour",
        None,
        &mut settings.buy_imbalance_color,
        [0, 230, 118, 255],
    );
    color_row(
        ui,
        Icon::ArrowDownRight,
        "Sell imbalance colour",
        None,
        &mut settings.sell_imbalance_color,
        [255, 23, 68, 255],
    );
    switch_row(
        ui,
        Icon::Layers,
        "Highlight stacked imbalances",
        "Project stacks of imbalances to the right until later prices reach them.",
        &mut settings.highlight_stacked,
    );
    let mut levels = i64::from(settings.stacked_levels);
    if int_row(
        ui,
        id.with("stacked"),
        Icon::Hash,
        "Stacked levels",
        Some("Consecutive imbalanced rows of one side that make a stack."),
        &mut levels,
        2..=20,
    ) {
        settings.stacked_levels = levels as u32;
    }
    candle_rows(ui, &mut settings.candle, palette, false);
}
