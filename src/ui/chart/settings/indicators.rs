//! The Indicators page: the built-in indicators and their settings.

use super::ChartSettingsContext;
use crate::model::{
    BOLLINGER_MULTIPLIER, BOLLINGER_PERIOD, EMA_PERIODS, RSI_OVERBOUGHT, RSI_OVERSOLD, RSI_PERIOD,
    SUPERTREND_MULTIPLIER, SUPERTREND_PERIOD,
};
use crate::ui::icons::Icon;
use crate::ui::kit;
use crate::ui::settings::card;
use crate::ui::theme;
use eframe::egui::{CursorIcon, Label, RichText, Sense, Ui};

const CLASSIC_SCALE_PERCENTAGES: [u8; 8] = [15, 20, 25, 30, 35, 40, 45, 50];
const ADAPTIVE_PERCENTAGES: [u8; 17] = [
    5, 6, 7, 8, 9, 10, 12, 14, 15, 16, 18, 20, 22, 25, 28, 30, 35,
];

fn pct_picker(ui: &mut Ui, current: &mut u8) {
    let labels: Vec<String> = ADAPTIVE_PERCENTAGES
        .iter()
        .map(|p| format!("{p}%"))
        .collect();
    let selected = ADAPTIVE_PERCENTAGES
        .iter()
        .position(|&p| p == *current)
        .unwrap_or(0);
    if let Some(idx) = kit::menu_picker(ui, 64.0, &labels, selected, true) {
        *current = ADAPTIVE_PERCENTAGES[idx];
    }
}

pub(super) fn indicators_page(ui: &mut Ui, c: &mut ChartSettingsContext) {
    card::section_title(
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
        card::card(ui, Icon::BarChart, "Volume Bars", Some(&hint), |ui| {
            // Left visible rather than hidden: a greyed-out switch with a reason reads better
            // than a setting that silently does nothing.
            card::switch_enabled(ui, &mut config.show_volume, source.provides_volume());
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
    card::card(
        ui,
        Icon::Waveform,
        &format!("RSI ({RSI_PERIOD})"),
        Some(&rsi_hint),
        |ui| {
            card::switch(ui, &mut config.show_rsi);
        },
    );
    card::card(
        ui,
        Icon::ChartLine,
        "EMA",
        Some("Exponential moving average over the closes, drawn on the price scale."),
        |ui| {
            card::switch(ui, &mut config.show_ema);
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
    card::card(
        ui,
        Icon::Bands,
        "Bollinger Bands",
        Some(&format!(
            "{BOLLINGER_PERIOD}-period average with bands {} standard deviations either side — wide when volatile, tight when calm.",
            BOLLINGER_MULTIPLIER as i64
        )),
        |ui| {
            card::switch(ui, &mut config.show_bollinger);
        },
    );
    card::card(
        ui,
        Icon::Cycle,
        "Trend Flips",
        Some(&format!(
            "Bullish and bearish markers from {SUPERTREND_PERIOD}-period ATR × {}. Signals appear after the candle closes and can whipsaw in sideways markets.",
            SUPERTREND_MULTIPLIER as i64
        )),
        |ui| {
            card::switch(ui, &mut config.show_trend_flips);
        },
    );

    ui.add_space(8.0);
    card::section_title(
        ui,
        "Subpane Scale",
        Some("Configure the height and distribution of lower indicator panes."),
    );

    // 1. Adaptive Scale (Smart)
    let mut adaptive_on = config.subpane_adaptive;
    card::card(
        ui,
        Icon::Layers,
        "Adaptive Scale (Smart)",
        Some(
            "Adapts heights dynamically: 1 indicator takes 20%, 2 take 25% (16% top / 9% bottom), 3 take 30% (15% / 8% / 7%).",
        ),
        |ui| {
            if card::switch(ui, &mut adaptive_on) {
                config.subpane_adaptive = adaptive_on;
            }
        },
    );

    if config.subpane_adaptive {
        card::card(
            ui,
            Icon::ChartLine,
            "1 indicator height",
            Some("Total height share for 1 subpane"),
            |ui| {
                pct_picker(ui, &mut config.adaptive_p1);
            },
        );
        let sum2 = config.adaptive_p2_top + config.adaptive_p2_bot;
        let hint2 = format!(
            "Total: {}% + {}% = {}%",
            config.adaptive_p2_top, config.adaptive_p2_bot, sum2
        );
        card::card(
            ui,
            Icon::SplitColumns,
            "2 indicators heights",
            Some(&hint2),
            |ui| {
                ui.horizontal(|ui| {
                    ui.label(kit::text(
                        "Top:",
                        11.0,
                        kit::REGULAR,
                        theme::text_secondary(),
                    ));
                    pct_picker(ui, &mut config.adaptive_p2_top);
                    ui.add_space(8.0);
                    ui.label(kit::text(
                        "Bottom:",
                        11.0,
                        kit::REGULAR,
                        theme::text_secondary(),
                    ));
                    pct_picker(ui, &mut config.adaptive_p2_bot);
                });
            },
        );
        let sum3 = config.adaptive_p3_top + config.adaptive_p3_mid + config.adaptive_p3_bot;
        let hint3 = format!(
            "Total: {}% + {}% + {}% = {}%",
            config.adaptive_p3_top, config.adaptive_p3_mid, config.adaptive_p3_bot, sum3
        );
        card::card(
            ui,
            Icon::BarChart,
            "3 indicators heights",
            Some(&hint3),
            |ui| {
                ui.horizontal(|ui| {
                    ui.label(kit::text(
                        "Top:",
                        11.0,
                        kit::REGULAR,
                        theme::text_secondary(),
                    ));
                    pct_picker(ui, &mut config.adaptive_p3_top);
                    ui.add_space(8.0);
                    ui.label(kit::text(
                        "Mid:",
                        11.0,
                        kit::REGULAR,
                        theme::text_secondary(),
                    ));
                    pct_picker(ui, &mut config.adaptive_p3_mid);
                    ui.add_space(8.0);
                    ui.label(kit::text(
                        "Bot:",
                        11.0,
                        kit::REGULAR,
                        theme::text_secondary(),
                    ));
                    pct_picker(ui, &mut config.adaptive_p3_bot);
                });
            },
        );

        let is_default_adaptive = config.adaptive_p1 == 20
            && config.adaptive_p2_top == 16
            && config.adaptive_p2_bot == 9
            && config.adaptive_p3_top == 15
            && config.adaptive_p3_mid == 8
            && config.adaptive_p3_bot == 7;
        if !is_default_adaptive {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                let reset = ui.add(
                    Label::new(
                        RichText::new("Restore adaptive defaults")
                            .size(kit::size::SUBHEADLINE)
                            .underline()
                            .color(theme::text_secondary()),
                    )
                    .sense(Sense::click()),
                );
                if reset.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    config.adaptive_p1 = 20;
                    config.adaptive_p2_top = 16;
                    config.adaptive_p2_bot = 9;
                    config.adaptive_p3_top = 15;
                    config.adaptive_p3_mid = 8;
                    config.adaptive_p3_bot = 7;
                }
            });
        }
    }

    // 2. Classic Scale (Equal)
    let mut classic_on = !config.subpane_adaptive;
    let pct_labels: Vec<String> = CLASSIC_SCALE_PERCENTAGES
        .iter()
        .map(|p| format!("{p}%"))
        .collect();
    let selected_pct = CLASSIC_SCALE_PERCENTAGES
        .iter()
        .position(|&p| p == config.subpane_classic_pct)
        .unwrap_or(3); // default 30% is index 3

    card::card(
        ui,
        Icon::Percent,
        "Classic Scale (Equal)",
        Some("Fixed total share divided equally across all open indicators."),
        |ui| {
            if card::switch(ui, &mut classic_on) {
                config.subpane_adaptive = !classic_on;
            }
            ui.add_space(4.0);
            if let Some(index) = kit::menu_picker(ui, 72.0, &pct_labels, selected_pct, classic_on) {
                config.subpane_classic_pct = CLASSIC_SCALE_PERCENTAGES[index];
                config.subpane_adaptive = false;
            }
        },
    );
}
