//! The Indicators page: the built-in indicators and their settings.

use super::ChartSettingsContext;
use crate::i18n::tr;
use crate::model::{
    BOLLINGER_MULTIPLIER, BOLLINGER_PERIOD, EMA_PERIODS, SUPERTREND_MULTIPLIER, SUPERTREND_PERIOD,
};
use crate::ui::icons::Icon;
use crate::ui::kit;
use crate::ui::settings::card;
use eframe::egui::Ui;

pub(super) fn indicators_page(ui: &mut Ui, c: &mut ChartSettingsContext) {
    card::section_title(
        ui,
        tr!("Indicators"),
        Some(tr!(
            "Add context to the chart without changing its underlying data."
        )),
    );
    ui.spacing_mut().item_spacing.y = 8.0;
    let source = c.source;
    let config = &mut *c.indicators;
    // Line sources report one price per timestamp and no turnover at all.
    if !source.is_line_chart() {
        let hint = if source.provides_volume() {
            tr!("Turnover per candle, drawn along the bottom of the chart.").to_owned()
        } else {
            tr!(
                "{market} doesn't report per-candle volume, so bars aren't available for this chart.",
                market = source.display_name()
            )
        };
        card::card(ui, Icon::BarChart, tr!("Volume Bars"), Some(&hint), |ui| {
            // Left visible rather than hidden: a greyed-out switch with a reason reads better
            // than a setting that silently does nothing.
            card::switch_enabled(ui, &mut config.show_volume, source.provides_volume());
        });
    }
    card::card(
        ui,
        Icon::ChartLine,
        tr!("EMA"),
        Some(tr!(
            "Exponential moving average over the closes, drawn on the price scale."
        )),
        |ui| {
            card::switch(ui, &mut config.show_ema);
            ui.add_space(4.0);
            let labels: Vec<String> = EMA_PERIODS.iter().map(usize::to_string).collect();
            let selected = EMA_PERIODS
                .iter()
                .position(|&period| period == config.ema_period)
                .unwrap_or(0);
            if let Some(index) =
                kit::menu_picker_fitted(ui, 72.0, &labels, selected, config.show_ema)
            {
                config.ema_period = EMA_PERIODS[index];
            }
        },
    );
    card::card(
        ui,
        Icon::Bands,
        tr!("Bollinger Bands"),
        Some(&tr!(
            "{period}-period average with bands {multiplier} standard deviations either side — wide when volatile, tight when calm.",
            period = BOLLINGER_PERIOD,
            multiplier = BOLLINGER_MULTIPLIER as i64
        )),
        |ui| {
            card::switch(ui, &mut config.show_bollinger);
        },
    );
    card::card(
        ui,
        Icon::Cycle,
        tr!("Trend Flips"),
        Some(&tr!(
            "Bullish and bearish markers from {period}-period ATR × {multiplier}. Signals appear after the candle closes and can whipsaw in sideways markets.",
            period = SUPERTREND_PERIOD,
            multiplier = SUPERTREND_MULTIPLIER as i64
        )),
        |ui| {
            card::switch(ui, &mut config.show_trend_flips);
        },
    );
}
