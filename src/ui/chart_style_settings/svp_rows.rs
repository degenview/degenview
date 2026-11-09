//! The settings rows of the Session volume profile chart type: which sessions, what the bars show,
//! how rows are sized, the lines and the developing levels, and the look.
//!
//! Session presets other than All and Custom (each phase, pre-market, market, post-market) are not
//! here: the app has no extended-hours metadata, and a preset it could not honour would be a lie.

use super::{Pair, color_row, int_row, picker_row, switch_row, tpo_rows};
use crate::model::chart_type::{
    RowsLayout, SessionMode, SvpLine, SvpSettings, SvpVolume, VolumePlacement,
};
use crate::ui::icons::Icon;
use eframe::egui::{Id, Ui};

pub(super) fn rows(ui: &mut Ui, id: Id, settings: &mut SvpSettings, palette: Pair) {
    let _ = palette;
    picker_row(
        ui,
        Icon::Layers,
        "Sessions",
        Some(
            "One profile per UTC day, or per custom window (up to five, in UTC). Needs intraday candles: not available on weekly and longer charts.",
        ),
        &[(SessionMode::All, "All"), (SessionMode::Custom, "Custom")],
        &mut settings.sessions,
    );
    if settings.sessions == SessionMode::Custom {
        tpo_rows::custom_sessions(ui, id, &mut settings.custom);
    }
    picker_row(
        ui,
        Icon::BarChart,
        "Volume",
        Some("Up and down volume side by side, the total, or up minus down."),
        &[
            (SvpVolume::UpDown, "Up/Down"),
            (SvpVolume::Total, "Total"),
            (SvpVolume::Delta, "Delta"),
        ],
        &mut settings.volume,
    );
    let mut percent = i64::from(settings.value_area_percent);
    if int_row(
        ui,
        id.with("va"),
        Icon::Percent,
        "Value area volume",
        Some("The share of a session's volume its value area holds, in percent."),
        &mut percent,
        1..=100,
    ) {
        settings.value_area_percent = percent as u32;
    }
    picker_row(
        ui,
        Icon::Sliders,
        "Rows layout",
        Some(
            "Number of rows picks a row size so a session has about that many rows (more may be added); ticks per row sets the size.",
        ),
        &[
            (RowsLayout::NumberOfRows, "Number of rows"),
            (RowsLayout::TicksPerRow, "Ticks per row"),
        ],
        &mut settings.layout,
    );
    let mut size = i64::from(settings.row_size);
    if int_row(
        ui,
        id.with("rows"),
        Icon::Hash,
        "Row size",
        None,
        &mut size,
        1..=1_000,
    ) {
        settings.row_size = size as u32;
    }
    line_rows(
        ui,
        "Point of control",
        &mut settings.poc,
        [240, 185, 11, 255],
    );
    line_rows(
        ui,
        "Value area high",
        &mut settings.vah,
        [160, 160, 160, 255],
    );
    line_rows(
        ui,
        "Value area low",
        &mut settings.val,
        [160, 160, 160, 255],
    );
    switch_row(
        ui,
        Icon::Waveform,
        "Developing POC",
        "How the point of control moved through the session, from the volume that had traded by each moment.",
        &mut settings.developing_poc,
    );
    switch_row(
        ui,
        Icon::Waveform,
        "Developing value area",
        "How the value area's edges moved through the session, with no later volume in them.",
        &mut settings.developing_value_area,
    );
    switch_row(
        ui,
        Icon::Hash,
        "Values",
        "Write each row's volume at the end of its bar.",
        &mut settings.values,
    );
    let mut width = i64::from(settings.width_percent);
    if int_row(
        ui,
        id.with("width"),
        Icon::Sliders,
        "Width",
        Some("How much of the session's width the histogram takes, in percent."),
        &mut width,
        5..=100,
    ) {
        settings.width_percent = width as u32;
    }
    picker_row(
        ui,
        Icon::BarChart,
        "Placement",
        None,
        &[
            (VolumePlacement::Left, "Left"),
            (VolumePlacement::Right, "Right"),
        ],
        &mut settings.placement,
    );
    let [up, down] = crate::ui::series_render_defaults();
    color_row(
        ui,
        Icon::ArrowUpRight,
        "Up volume",
        None,
        &mut settings.up_color,
        up,
    );
    color_row(
        ui,
        Icon::ArrowDownRight,
        "Down volume",
        None,
        &mut settings.down_color,
        down,
    );
    color_row(
        ui,
        Icon::ArrowUpRight,
        "Value area up volume",
        Some("Follows the up volume colour until set."),
        &mut settings.value_area_up_color,
        settings.up_color.unwrap_or(up),
    );
    color_row(
        ui,
        Icon::ArrowDownRight,
        "Value area down volume",
        Some("Follows the down volume colour until set."),
        &mut settings.value_area_down_color,
        settings.down_color.unwrap_or(down),
    );
    color_row(
        ui,
        Icon::Palette,
        "Histogram box",
        Some("A translucent box behind each histogram, over the session's range."),
        &mut settings.box_color,
        [128, 128, 128, 255],
    );
    let mut opacity = i64::from(settings.box_opacity);
    if int_row(
        ui,
        id.with("box"),
        Icon::Percent,
        "Histogram box opacity",
        None,
        &mut opacity,
        0..=100,
    ) {
        settings.box_opacity = opacity as u32;
    }
}

fn line_rows(ui: &mut Ui, title: &str, line: &mut SvpLine, default: [u8; 4]) {
    switch_row(
        ui,
        Icon::Eye,
        title,
        "Mark it on each profile.",
        &mut line.show,
    );
    if line.show {
        switch_row(
            ui,
            Icon::ArrowUpRight,
            &format!("Extend {}", title.to_lowercase()),
            "Carry the line to the right until a bar crosses it.",
            &mut line.extend,
        );
        color_row(
            ui,
            Icon::Palette,
            &format!("{title} colour"),
            None,
            &mut line.color,
            default,
        );
    }
}
