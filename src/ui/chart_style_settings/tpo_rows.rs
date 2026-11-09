//! The settings rows of the Time price opportunity chart type: the profile (period, block, rows,
//! value area, sessions), how it looks, the levels it marks and the volume profile beside it.
//!
//! Splits and merges are not here: they are made from the right-click menu on a letter, and changing
//! the period or the block size drops them (TradingView's rule), which the page says.

use super::{Pair, color_row, int_row, picker_row, switch_row};
use crate::model::chart_type::{
    PeriodKind, RowMode, SessionMode, TPO_BLOCK_MINUTES, TpoLevel, TpoSettings, VolumePlacement,
};
use crate::model::market_profile::sessions::SessionSpan;
use crate::ui::icons::Icon;
use crate::ui::settings_card;
use eframe::egui::{Id, Ui};

pub(super) fn rows(ui: &mut Ui, id: Id, settings: &mut TpoSettings, palette: Pair) {
    let _ = palette;
    // Profile
    picker_row(
        ui,
        Icon::Layers,
        "Period",
        Some(
            "How much time one profile covers. Changing it, or the block size, drops the splits and merges.",
        ),
        &[
            (PeriodKind::Day, "Days"),
            (PeriodKind::Week, "Weeks"),
            (PeriodKind::Month, "Months"),
        ],
        &mut settings.period,
    );
    let mut count = i64::from(settings.period_count);
    if int_row(
        ui,
        id.with("period-count"),
        Icon::Hash,
        "Periods per profile",
        None,
        &mut count,
        1..=60,
    ) {
        settings.period_count = count as u32;
    }
    let sizes: Vec<(u32, String)> = TPO_BLOCK_MINUTES
        .into_iter()
        .map(|minutes| {
            (
                minutes,
                if minutes >= 60 {
                    format!(
                        "{} hour{}",
                        minutes / 60,
                        if minutes == 60 { "" } else { "s" }
                    )
                } else {
                    format!("{minutes} minutes")
                },
            )
        })
        .collect();
    let options: Vec<(u32, &str)> = sizes
        .iter()
        .map(|(m, label)| (*m, label.as_str()))
        .collect();
    picker_row(
        ui,
        Icon::Sliders,
        "Block size",
        Some(
            "Each block of this length gets a letter. Built from 5-minute candles whatever the chart's timeframe.",
        ),
        &options,
        &mut settings.block_minutes,
    );
    picker_row(
        ui,
        Icon::Sliders,
        "Row size",
        Some("Automatic follows the range of the newest 300 bars, about 80 rows."),
        &[(RowMode::Auto, "Auto"), (RowMode::Manual, "Manual")],
        &mut settings.row_mode,
    );
    if settings.row_mode == RowMode::Manual {
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
    let mut percent = i64::from(settings.value_area_percent);
    if int_row(
        ui,
        id.with("va"),
        Icon::Percent,
        "Value area",
        Some(
            "The share of a profile's TPO blocks its value area holds. Counted in blocks, not volume.",
        ),
        &mut percent,
        1..=100,
    ) {
        settings.value_area_percent = percent as u32;
    }
    picker_row(
        ui,
        Icon::Layers,
        "Sessions",
        Some("Custom cuts the day into your own windows (UTC), one profile each."),
        &[(SessionMode::All, "All"), (SessionMode::Custom, "Custom")],
        &mut settings.sessions,
    );
    if settings.sessions == SessionMode::Custom {
        custom_sessions(ui, id, settings);
    }
    let mut blocks = i64::from(settings.initial_balance_blocks);
    if int_row(
        ui,
        id.with("ib"),
        Icon::Hash,
        "Initial balance blocks",
        Some("The first blocks of a profile whose range is the initial balance."),
        &mut blocks,
        1..=20,
    ) {
        settings.initial_balance_blocks = blocks as u32;
    }
    // Appearance
    for (n, title) in [
        "Gradient: A start",
        "Gradient: Z end",
        "Gradient: a start",
        "Gradient: z end",
    ]
    .into_iter()
    .enumerate()
    {
        color_row(
            ui,
            Icon::Palette,
            title,
            if n == 0 {
                Some("The first two colours span the blocks A to Z, the last two a to z.")
            } else {
                None
            },
            &mut settings.gradient[n],
            super::super::series_render::tpo_default_gradient()[n],
        );
    }
    switch_row(
        ui,
        Icon::Eye,
        "Blocks",
        "Fill each letter's cell with its colour. Shown on their own when there is no room for letters.",
        &mut settings.show_blocks,
    );
    switch_row(
        ui,
        Icon::Hash,
        "Letters",
        "Write each block's letter where there is room.",
        &mut settings.show_letters,
    );
    let mut opacity = i64::from(settings.opacity_outside_value_area);
    if int_row(
        ui,
        id.with("opacity"),
        Icon::Percent,
        "Opacity outside the value area",
        Some("How much of its colour a row outside the value area keeps, in percent."),
        &mut opacity,
        0..=100,
    ) {
        settings.opacity_outside_value_area = opacity as u32;
    }
    switch_row(
        ui,
        Icon::Layers,
        "Expand blocks",
        "Put each letter in the column of its block, leaving gaps, instead of packing a row's letters together.",
        &mut settings.expand_blocks,
    );
    // Levels
    level_rows(
        ui,
        "Point of control",
        &mut settings.poc,
        [240, 185, 11, 255],
        true,
    );
    level_rows(
        ui,
        "Value area high",
        &mut settings.vah,
        [160, 160, 160, 255],
        false,
    );
    level_rows(
        ui,
        "Value area low",
        &mut settings.val,
        [160, 160, 160, 255],
        false,
    );
    level_rows(
        ui,
        "TPO midpoint",
        &mut settings.midpoint,
        [190, 190, 190, 255],
        false,
    );
    level_rows(ui, "Open", &mut settings.open, [120, 200, 120, 255], false);
    level_rows(
        ui,
        "Close",
        &mut settings.close,
        [220, 120, 120, 255],
        false,
    );
    level_rows(
        ui,
        "Poor high",
        &mut settings.poor_high,
        [255, 112, 67, 255],
        true,
    );
    level_rows(
        ui,
        "Poor low",
        &mut settings.poor_low,
        [255, 112, 67, 255],
        true,
    );
    level_rows(
        ui,
        "Single prints",
        &mut settings.single_prints,
        [255, 235, 59, 255],
        true,
    );
    level_rows(
        ui,
        "Initial balance range",
        &mut settings.initial_balance,
        [200, 200, 200, 255],
        false,
    );
    // Volume profile
    let volume = &mut settings.volume;
    switch_row(
        ui,
        Icon::BarChart,
        "Volume profile",
        "A histogram of traded volume beside the profile. Volume, not TPO counts.",
        &mut volume.show,
    );
    picker_row(
        ui,
        Icon::BarChart,
        "Volume profile placement",
        None,
        &[
            (VolumePlacement::Left, "Left"),
            (VolumePlacement::Right, "Right"),
        ],
        &mut volume.placement,
    );
    switch_row(
        ui,
        Icon::Hash,
        "Volume values",
        "Write each row's volume at the end of its bar.",
        &mut volume.values,
    );
    switch_row(
        ui,
        Icon::Eye,
        "Volume point of control",
        "A line at the row with the most volume.",
        &mut volume.show_poc,
    );
    switch_row(
        ui,
        Icon::Eye,
        "Volume value area high",
        "A line at the top of the volume value area.",
        &mut volume.show_vah,
    );
    switch_row(
        ui,
        Icon::Eye,
        "Volume value area low",
        "A line at the bottom of the volume value area.",
        &mut volume.show_val,
    );
    color_row(
        ui,
        Icon::Palette,
        "Volume colour",
        None,
        &mut volume.color,
        [140, 150, 160, 255],
    );
    color_row(
        ui,
        Icon::Palette,
        "Volume value area colour",
        None,
        &mut volume.value_area_color,
        [200, 170, 90, 255],
    );
}

fn level_rows(ui: &mut Ui, title: &str, level: &mut TpoLevel, default: [u8; 4], extendable: bool) {
    switch_row(
        ui,
        Icon::Eye,
        title,
        "Mark it on each profile.",
        &mut level.show,
    );
    if level.show {
        if extendable {
            switch_row(
                ui,
                Icon::ArrowUpRight,
                &format!("Extend {}", title.to_lowercase()),
                "Carry the level to the right until price reaches it again.",
                &mut level.extend,
            );
        }
        color_row(
            ui,
            Icon::Palette,
            &format!("{title} colour"),
            None,
            &mut level.color,
            default,
        );
    }
}

/// Up to five `HHMM-HHMM[:days]` windows, one per line. A line that does not read is left as it was
/// and marked; an empty line removes its window.
fn custom_sessions(ui: &mut Ui, id: Id, settings: &mut TpoSettings) {
    for slot in 0..settings.custom.len() {
        let field_id = id.with(("session", slot));
        let mut text: String = ui
            .data(|data| data.get_temp::<String>(field_id))
            .unwrap_or_else(|| {
                settings.custom[slot]
                    .map(|span| span.to_text())
                    .unwrap_or_default()
            });
        let valid = text.trim().is_empty() || SessionSpan::parse(&text).is_some();
        settings_card::card(
            ui,
            Icon::Layers,
            &format!("Session {}", slot + 1),
            Some(if valid {
                "HHMM-HHMM, optionally :days (1 is Sunday), in UTC. Blank for none."
            } else {
                "Not a window: use HHMM-HHMM, optionally :days, for example 0930-1600:23456."
            }),
            |ui| {
                ui.add(eframe::egui::TextEdit::singleline(&mut text).desired_width(120.0));
            },
        );
        if valid {
            settings.custom[slot] = if text.trim().is_empty() {
                None
            } else {
                SessionSpan::parse(&text)
            };
        }
        ui.data_mut(|data| data.insert_temp(field_id, text));
    }
}
