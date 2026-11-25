//! The "Chart type" section of the chart settings' Appearance page: the type, and the settings of
//! the selected type only. Each type keeps its own, so a switch never loses what was set, and a
//! setting that means nothing for the type on screen is not offered.

use crate::i18n::tr;
use crate::model::DataSource;
use crate::model::chart::kind::{
    AreaSettings, BarSettings, BaselineSettings, CandleSettings, ChartGroup, ChartType,
    ColumnSettings, ConstructionSource, HlcAreaSettings, LineDash, LineSettings, PriceSource,
    RangeLook, Rgba, SizeMethod, SizeRule,
};
use crate::ui::chart::card::ChartCard;
use crate::ui::chart::series::{HLC_FILL_ALPHA, color as paint_color, line_default, neutral_line};
use crate::ui::icons::Icon;
use crate::ui::kit;
use crate::ui::settings::card;
use eframe::egui::{self, Id, Ui, color_picker::Alpha, vec2};

type Pair = (Rgba, Rgba);

/// Draws the section for `card`'s chart (`source` decides which types are offered).
/// `beside_reset` draws what sits next to the "Reset … settings" button.
pub fn show(
    ui: &mut Ui,
    card: &mut ChartCard,
    source: DataSource,
    chart_id: Id,
    beside_reset: impl FnOnce(&mut Ui, &mut ChartCard),
) {
    let appearance = card.appearance();
    let palette: Pair = (opaque(appearance.bullish), opaque(appearance.bearish));
    let current = card.effective_chart_type(source);
    card::section_title(
        ui,
        tr!("Chart type"),
        Some(tr!(
            "How the price is drawn. Each type remembers its own settings."
        )),
    );
    ui.spacing_mut().item_spacing.y = 8.0;
    let types: Vec<ChartType> = ChartGroup::ALL
        .into_iter()
        .flat_map(ChartType::in_group)
        .filter(|kind| kind.supported_by(source))
        .collect();
    let labels: Vec<String> = types.iter().map(|kind| kind.label().to_owned()).collect();
    let selected = types.iter().position(|kind| *kind == current).unwrap_or(0);
    let hint = match current.group() {
        ChartGroup::Standard => tr!("Draws the market's own open, high, low and close."),
        ChartGroup::LinesAndAreas => {
            tr!("Draws one value of each bar. Scripts and indicators still read the real OHLC.")
        }
        ChartGroup::PriceBased => {
            tr!(
                "Builds its own bars from the market's. Scripts and indicators run over those bars."
            )
        }
    };
    if let Some(index) = card::card(ui, Icon::BarChart, tr!("Style"), Some(hint), |ui| {
        kit::menu_picker_fitted(ui, 170.0, &labels, selected, true)
    }) {
        card.set_chart_type(types[index]);
    }
    // The picker may have just changed the type: the preview and rows below belong to the new one.
    let current = card.effective_chart_type(source);
    super::style_preview::show(ui, &card.appearance(), current);
    let id = chart_id.with(current as u8);
    let style = card.chart_style_mut();
    match current {
        ChartType::Candles | ChartType::HeikinAshi | ChartType::VolumeCandles => {
            if let Some(settings) = style.candle_settings_mut() {
                candle_rows(ui, settings, palette, false);
            }
        }
        ChartType::HollowCandles => {
            if let Some(settings) = style.candle_settings_mut() {
                candle_rows(ui, settings, palette, true);
            }
        }
        ChartType::Bars | ChartType::HighLowBars => {
            let high_low = current == ChartType::HighLowBars;
            if let Some(settings) = style.bar_settings_mut() {
                bar_rows(ui, settings, palette, high_low);
            }
        }
        ChartType::Line | ChartType::LineWithMarkers | ChartType::StepLine => {
            let markers = current == ChartType::LineWithMarkers;
            if let Some(settings) = style.line_settings_mut() {
                line_rows(ui, id, settings, markers);
            }
        }
        ChartType::Area => area_rows(ui, id, &mut style.area),
        ChartType::HlcArea => hlc_area_rows(ui, id, &mut style.hlc_area, palette),
        ChartType::Baseline => baseline_rows(ui, id, &mut style.baseline, palette),
        ChartType::Columns => column_rows(ui, &mut style.columns, palette),
        ChartType::Renko => {
            size_rows(
                ui,
                id,
                &mut style.renko.size,
                tr!("Box size"),
                tr!("Each brick covers this much price."),
            );
            source_row(
                ui,
                tr!("Source"),
                tr!(
                    "The close of each bar, or its highs and lows too. Candles do not record the order prices traded in."
                ),
                &mut style.renko.source,
            );
            switch_row(
                ui,
                Icon::Waveform,
                tr!("Wicks"),
                tr!("Show the bars' highs and lows on the bricks."),
                &mut style.renko.wicks,
            );
            candle_rows(ui, &mut style.renko.candle, palette, false);
        }
        ChartType::Kagi => {
            size_rows(
                ui,
                id,
                &mut style.kagi.size,
                tr!("Reversal amount"),
                tr!("The move against the line that turns it."),
            );
            source_row(
                ui,
                tr!("Source"),
                tr!("The close of each bar, or its highs and lows too."),
                &mut style.kagi.source,
            );
            color_row(
                ui,
                Icon::ArrowUpRight,
                tr!("Thick line"),
                Some(tr!("Drawn while price is breaking highs.")),
                &mut style.kagi.up,
                palette.0,
            );
            color_row(
                ui,
                Icon::ArrowDownRight,
                tr!("Thin line"),
                Some(tr!("Drawn while price is breaking lows.")),
                &mut style.kagi.down,
                palette.1,
            );
            width_row(ui, id, &mut style.kagi.width);
        }
        ChartType::LineBreak => {
            let mut lines = i64::from(style.line_break.lines);
            if int_row(
                ui,
                id.with("lines"),
                Icon::Hash,
                tr!("Number of lines"),
                Some(tr!(
                    "How many previous lines a close has to break to turn the trend."
                )),
                &mut lines,
                1..=20,
            ) {
                style.line_break.lines = lines as u8;
            }
            candle_rows(ui, &mut style.line_break.candle, palette, false);
        }
        ChartType::PointAndFigure => {
            size_rows(
                ui,
                id,
                &mut style.point_figure.size,
                tr!("Box size"),
                tr!("The price each X or O covers."),
            );
            let mut reversal = i64::from(style.point_figure.reversal);
            if int_row(
                ui,
                id.with("reversal"),
                Icon::Hash,
                tr!("Reversal"),
                Some(tr!("Boxes against a column that start the next one.")),
                &mut reversal,
                1..=20,
            ) {
                style.point_figure.reversal = reversal as u8;
            }
            source_row(
                ui,
                tr!("Source"),
                tr!("The close of each bar, or its highs and lows too."),
                &mut style.point_figure.source,
            );
            color_row(
                ui,
                Icon::ArrowUpRight,
                tr!("X columns"),
                Some(tr!("Rising columns.")),
                &mut style.point_figure.up,
                palette.0,
            );
            color_row(
                ui,
                Icon::ArrowDownRight,
                tr!("O columns"),
                Some(tr!("Falling columns.")),
                &mut style.point_figure.down,
                palette.1,
            );
            switch_row(
                ui,
                Icon::Eye,
                tr!("Box outlines"),
                tr!("Draw the grid behind the symbols."),
                &mut style.point_figure.outlines,
            );
        }
        ChartType::Range => {
            size_rows(
                ui,
                id,
                &mut style.range.size,
                tr!("Range"),
                tr!("A bar closes once its high-to-low span reaches this."),
            );
            let looks = [
                (RangeLook::Candles, tr!("Candles")),
                (RangeLook::Bars, tr!("Bars")),
            ];
            picker_row(
                ui,
                Icon::BarChart,
                tr!("Bars as"),
                None,
                &looks,
                &mut style.range.look,
            );
            match style.range.look {
                RangeLook::Candles => candle_rows(ui, &mut style.range.candle, palette, false),
                RangeLook::Bars => bar_rows(ui, &mut style.range.bars, palette, false),
            }
        }
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let label = tr!("Reset {chart_type} settings", chart_type = current.label());
        if kit::push_button(ui, None, &label, true).clicked() {
            card.chart_style_mut().reset_selected();
        }
        beside_reset(ui, card);
    });
}

fn opaque([r, g, b]: [u8; 3]) -> Rgba {
    [r, g, b, 255]
}

// MARK: Rows

pub(in crate::ui) fn switch_row(
    ui: &mut Ui,
    icon: Icon,
    title: &str,
    hint: &str,
    value: &mut bool,
) {
    card::card(ui, icon, title, Some(hint), |ui| {
        card::switch(ui, value);
    });
}

/// A colour that follows `default` until the user picks one; "Reset" returns to following it.
pub(in crate::ui) fn color_row(
    ui: &mut Ui,
    icon: Icon,
    title: &str,
    hint: Option<&str>,
    slot: &mut Option<Rgba>,
    default: Rgba,
) {
    card::card(ui, icon, title, hint, |ui| {
        color_control(ui, title, slot, default);
    });
}

/// The control of a [`color_row`] alone, for a card that holds more than one.
pub(in crate::ui) fn color_control(
    ui: &mut Ui,
    title: &str,
    slot: &mut Option<Rgba>,
    default: Rgba,
) {
    if slot.is_some() && kit::push_button(ui, None, tr!("Reset"), true).clicked() {
        *slot = None;
    }
    let shown = paint_color(slot.unwrap_or(default));
    if let Some(picked) = kit::color_well(ui, vec2(44.0, 24.0), title, shown, Alpha::OnlyBlend) {
        *slot = Some(picked.to_srgba_unmultiplied());
    }
}

pub(in crate::ui) fn picker_row<T: Copy + PartialEq>(
    ui: &mut Ui,
    icon: Icon,
    title: &str,
    hint: Option<&str>,
    options: &[(T, &str)],
    value: &mut T,
) {
    let labels: Vec<String> = options
        .iter()
        .map(|(_, label)| (*label).to_owned())
        .collect();
    let selected = options
        .iter()
        .position(|(option, _)| option == value)
        .unwrap_or(0);
    if let Some(index) = card::card(ui, icon, title, hint, |ui| {
        kit::menu_picker_fitted(ui, 130.0, &labels, selected, true)
    }) {
        *value = options[index].0;
    }
}

fn price_source_row(ui: &mut Ui, value: &mut PriceSource) {
    let options: Vec<(PriceSource, &str)> = PriceSource::ALL
        .into_iter()
        .map(|source| (source, source.label()))
        .collect();
    picker_row(
        ui,
        Icon::Sliders,
        tr!("Source"),
        Some(tr!(
            "The value of each bar the chart follows. Scripts still read the real OHLC."
        )),
        &options,
        value,
    );
}

fn source_row(ui: &mut Ui, title: &str, hint: &str, value: &mut ConstructionSource) {
    let options: Vec<(ConstructionSource, &str)> = ConstructionSource::ALL
        .into_iter()
        .map(|source| (source, source.label()))
        .collect();
    picker_row(ui, Icon::Sliders, title, Some(hint), &options, value);
}

fn width_row(ui: &mut Ui, id: Id, width: &mut u8) {
    let mut value = i64::from(*width);
    if int_row(
        ui,
        id.with("width"),
        Icon::Sliders,
        tr!("Line width"),
        None,
        &mut value,
        1..=4,
    ) {
        *width = value as u8;
    }
}

/// A whole-number field with the stepper beside it. Returns whether the value changed.
pub(in crate::ui) fn int_row(
    ui: &mut Ui,
    id: Id,
    icon: Icon,
    title: &str,
    hint: Option<&str>,
    value: &mut i64,
    range: std::ops::RangeInclusive<i64>,
) -> bool {
    let mut changed = false;
    card::card(ui, icon, title, hint, |ui| {
        let bounds = *range.start() as f64..=*range.end() as f64;
        if let Some(next) = number_field(ui, id, *value as f64, 1.0, bounds, 0) {
            *value = next.round() as i64;
            changed = true;
        }
    });
    changed
}

/// The text area of a [`number_field`], inside its border.
const FIELD_WIDTH: f32 = 56.0;
const FIELD_HEIGHT: f32 = 22.0;

/// A decimal field with the stepper beside it: the buffer is kept while the field has focus and
/// committed on Return or when it loses focus, clamped to `range`.
pub(in crate::ui) fn number_field(
    ui: &mut Ui,
    id: Id,
    value: f64,
    step: f64,
    range: std::ops::RangeInclusive<f64>,
    decimals: usize,
) -> Option<f64> {
    number_field_sized(ui, id, value, step, range, decimals, FIELD_WIDTH)
}

/// [`number_field`] with a text area of `width` points, for a value that needs more room.
pub(in crate::ui) fn number_field_sized(
    ui: &mut Ui,
    id: Id,
    value: f64,
    step: f64,
    range: std::ops::RangeInclusive<f64>,
    decimals: usize,
    width: f32,
) -> Option<f64> {
    let mut result = kit::stepper_arrows(ui, id.with("stepper")).map(|direction| match direction {
        kit::StepDirection::Up => value + step,
        kit::StepDirection::Down => value - step,
    });
    let edit_id = id.with("edit");
    let shown = format!("{value:.decimals$}");
    let mut text = ui
        .data(|data| data.get_temp::<String>(id))
        .unwrap_or_else(|| shown.clone());
    // The same 24-point height and corner as the menu pickers and buttons beside it: the frame's
    // one-point border plus a 22-point row the text is centred in.
    let frame = kit::field_frame(ui.memory(|memory| memory.has_focus(edit_id)))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(8, 0));
    let response = frame
        .show(ui, |ui| {
            ui.allocate_ui_with_layout(
                vec2(width, FIELD_HEIGHT),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| kit::plain_field(ui, edit_id, &mut text, "", 12.0, kit::REGULAR, width),
            )
            .inner
        })
        .inner;
    // Only text the user changed is a new value: the shown one is rounded, and leaving the
    // field must not round the value to it.
    if response.lost_focus() && result.is_none() && text.trim() != shown.trim() {
        result = text.trim().parse::<f64>().ok();
    }
    if response.has_focus() {
        ui.data_mut(|data| data.insert_temp(id, text));
    } else {
        ui.data_mut(|data| data.remove::<String>(id));
    }
    result
        .filter(|next| next.is_finite())
        .map(|next| next.clamp(*range.start(), *range.end()))
        .filter(|next| (*next - value).abs() > f64::EPSILON)
}

#[allow(clippy::too_many_arguments)]
fn float_row(
    ui: &mut Ui,
    id: Id,
    icon: Icon,
    title: &str,
    hint: Option<&str>,
    value: &mut f64,
    step: f64,
    range: std::ops::RangeInclusive<f64>,
    decimals: usize,
) {
    card::card(ui, icon, title, hint, |ui| {
        if let Some(next) = number_field(ui, id, *value, step, range, decimals) {
            *value = next;
        }
    });
}

/// The method picker and the value that method reads.
fn size_rows(ui: &mut Ui, id: Id, rule: &mut SizeRule, title: &str, hint: &str) {
    let options: Vec<(SizeMethod, &str)> = SizeMethod::ALL
        .into_iter()
        .map(|method| (method, method.label()))
        .collect();
    picker_row(
        ui,
        Icon::Gauge,
        title,
        Some(hint),
        &options,
        &mut rule.method,
    );
    match rule.method {
        SizeMethod::Traditional => {
            // A tenth of the amount's magnitude per step, so it works from satoshis to indices.
            let step = 10f64.powf(rule.amount.max(f64::MIN_POSITIVE).log10().floor() - 1.0);
            float_row(
                ui,
                id.with("amount"),
                Icon::Hash,
                tr!("Size in price units"),
                None,
                &mut rule.amount,
                step,
                f64::MIN_POSITIVE..=1e12,
                8,
            );
        }
        SizeMethod::Atr => {
            let mut length = i64::from(rule.atr_length);
            if int_row(
                ui,
                id.with("atr"),
                Icon::Hash,
                tr!("ATR length"),
                Some(tr!("Averaged over the market's bars, as of the newest.")),
                &mut length,
                1..=500,
            ) {
                rule.atr_length = length as u32;
            }
        }
        SizeMethod::Percent => {
            float_row(
                ui,
                id.with("percent"),
                Icon::Percent,
                tr!("Percent of price"),
                Some(tr!("Of the newest close.")),
                &mut rule.percent,
                0.1,
                0.0001..=100.0,
                3,
            );
        }
    }
}

// MARK: Per-type groups

fn candle_rows(ui: &mut Ui, settings: &mut CandleSettings, palette: Pair, hollow: bool) {
    switch_row(
        ui,
        Icon::Eye,
        tr!("Body"),
        tr!("Show candle bodies."),
        &mut settings.body,
    );
    switch_row(
        ui,
        Icon::Eye,
        tr!("Border"),
        tr!("Outline the bodies."),
        &mut settings.border,
    );
    switch_row(
        ui,
        Icon::Eye,
        tr!("Wick"),
        tr!("Show the high and low as wicks."),
        &mut settings.wick,
    );
    let up = settings.up_body.unwrap_or(palette.0);
    let down = settings.down_body.unwrap_or(palette.1);
    color_row(
        ui,
        Icon::ArrowUpRight,
        tr!("Up body"),
        Some(tr!("Follows the bullish colour until set.")),
        &mut settings.up_body,
        palette.0,
    );
    color_row(
        ui,
        Icon::ArrowDownRight,
        tr!("Down body"),
        Some(tr!("Follows the bearish colour until set.")),
        &mut settings.down_body,
        palette.1,
    );
    color_row(
        ui,
        Icon::ArrowUpRight,
        tr!("Up border"),
        None,
        &mut settings.up_border,
        up,
    );
    color_row(
        ui,
        Icon::ArrowDownRight,
        tr!("Down border"),
        None,
        &mut settings.down_border,
        down,
    );
    color_row(
        ui,
        Icon::ArrowUpRight,
        tr!("Up wick"),
        None,
        &mut settings.up_wick,
        up,
    );
    color_row(
        ui,
        Icon::ArrowDownRight,
        tr!("Down wick"),
        None,
        &mut settings.down_wick,
        down,
    );
    if !hollow {
        switch_row(
            ui,
            Icon::Cycle,
            tr!("Color based on previous close"),
            tr!("Colour by the close against the previous close instead of the open."),
            &mut settings.color_by_previous_close,
        );
    }
}

fn bar_rows(ui: &mut Ui, settings: &mut BarSettings, palette: Pair, high_low: bool) {
    if high_low {
        switch_row(
            ui,
            Icon::Eye,
            tr!("Labels"),
            tr!("Name the highest high and lowest low in view."),
            &mut settings.labels,
        );
    } else {
        switch_row(
            ui,
            Icon::Sliders,
            tr!("Thin bars"),
            tr!("Keep the bars one pixel wide at every zoom."),
            &mut settings.thin,
        );
        switch_row(
            ui,
            Icon::Eye,
            tr!("Open and close ticks"),
            tr!("The left tick is the open, the right the close."),
            &mut settings.ticks,
        );
    }
    color_row(
        ui,
        Icon::ArrowUpRight,
        tr!("Up bars"),
        Some(tr!("Follows the bullish colour until set.")),
        &mut settings.up,
        palette.0,
    );
    color_row(
        ui,
        Icon::ArrowDownRight,
        tr!("Down bars"),
        Some(tr!("Follows the bearish colour until set.")),
        &mut settings.down,
        palette.1,
    );
    switch_row(
        ui,
        Icon::Cycle,
        tr!("Color based on previous close"),
        tr!("Colour by the close against the previous close instead of the open."),
        &mut settings.color_by_previous_close,
    );
}

fn line_rows(ui: &mut Ui, id: Id, settings: &mut LineSettings, markers: bool) {
    price_source_row(ui, &mut settings.source);
    color_row(
        ui,
        Icon::Palette,
        tr!("Line color"),
        None,
        &mut settings.color,
        line_default(),
    );
    width_row(ui, id, &mut settings.width);
    let options: Vec<(LineDash, &str)> = LineDash::ALL
        .into_iter()
        .map(|dash| (dash, dash.label()))
        .collect();
    picker_row(
        ui,
        Icon::Sliders,
        tr!("Line style"),
        None,
        &options,
        &mut settings.dash,
    );
    if markers {
        let mut size = i64::from(settings.marker_size);
        if int_row(
            ui,
            id.with("marker"),
            Icon::Circle,
            tr!("Marker size"),
            None,
            &mut size,
            1..=8,
        ) {
            settings.marker_size = size as u8;
        }
    }
}

fn area_rows(ui: &mut Ui, id: Id, settings: &mut AreaSettings) {
    price_source_row(ui, &mut settings.source);
    let default_line = line_default();
    let line = settings.line.unwrap_or(default_line);
    color_row(
        ui,
        Icon::Palette,
        tr!("Line color"),
        None,
        &mut settings.line,
        default_line,
    );
    width_row(ui, id, &mut settings.width);
    color_row(
        ui,
        Icon::Palette,
        tr!("Fill at the line"),
        Some(tr!("Its opacity sets how strong the fill is.")),
        &mut settings.top,
        fade(line, 0.38),
    );
    color_row(
        ui,
        Icon::Palette,
        tr!("Fill at the bottom"),
        None,
        &mut settings.bottom,
        fade(line, 0.0),
    );
}

fn hlc_area_rows(ui: &mut Ui, id: Id, settings: &mut HlcAreaSettings, palette: Pair) {
    color_row(
        ui,
        Icon::ArrowUpRight,
        tr!("High line"),
        None,
        &mut settings.high_line,
        palette.0,
    );
    color_row(
        ui,
        Icon::ArrowDownRight,
        tr!("Low line"),
        None,
        &mut settings.low_line,
        palette.1,
    );
    color_row(
        ui,
        Icon::Palette,
        tr!("Close line"),
        None,
        &mut settings.close_line,
        neutral_line(),
    );
    let high = settings.high_line.unwrap_or(palette.0);
    let low = settings.low_line.unwrap_or(palette.1);
    color_row(
        ui,
        Icon::Palette,
        tr!("Fill above close"),
        Some(tr!("Its opacity sets how strong the fill is.")),
        &mut settings.high_fill,
        fade(high, HLC_FILL_ALPHA),
    );
    color_row(
        ui,
        Icon::Palette,
        tr!("Fill below close"),
        None,
        &mut settings.low_fill,
        fade(low, HLC_FILL_ALPHA),
    );
    width_row(ui, id, &mut settings.width);
}

fn baseline_rows(ui: &mut Ui, id: Id, settings: &mut BaselineSettings, palette: Pair) {
    price_source_row(ui, &mut settings.source);
    float_row(
        ui,
        id.with("level"),
        Icon::Percent,
        tr!("Base level"),
        Some(tr!(
            "Where the baseline sits, as a percentage of the chart's height from the bottom."
        )),
        &mut settings.level_percent,
        5.0,
        0.0..=100.0,
        1,
    );
    color_row(
        ui,
        Icon::ArrowUpRight,
        tr!("Line above"),
        None,
        &mut settings.top_line,
        palette.0,
    );
    color_row(
        ui,
        Icon::ArrowDownRight,
        tr!("Line below"),
        None,
        &mut settings.bottom_line,
        palette.1,
    );
    let (above, below) = (
        settings.top_line.unwrap_or(palette.0),
        settings.bottom_line.unwrap_or(palette.1),
    );
    color_row(
        ui,
        Icon::Palette,
        tr!("Fill above"),
        None,
        &mut settings.top_fill,
        fade(above, 0.3),
    );
    color_row(
        ui,
        Icon::Palette,
        tr!("Fill below"),
        None,
        &mut settings.bottom_fill,
        fade(below, 0.3),
    );
    width_row(ui, id, &mut settings.width);
}

fn column_rows(ui: &mut Ui, settings: &mut ColumnSettings, palette: Pair) {
    price_source_row(ui, &mut settings.source);
    color_row(
        ui,
        Icon::ArrowUpRight,
        tr!("Up columns"),
        None,
        &mut settings.up,
        palette.0,
    );
    color_row(
        ui,
        Icon::ArrowDownRight,
        tr!("Down columns"),
        None,
        &mut settings.down,
        palette.1,
    );
    switch_row(
        ui,
        Icon::Cycle,
        tr!("Color based on previous close"),
        tr!("Colour by the value against the previous bar's instead of the open and close."),
        &mut settings.color_by_previous_close,
    );
}

fn fade(color: Rgba, alpha: f32) -> Rgba {
    [
        color[0],
        color[1],
        color[2],
        (f32::from(color[3]) * alpha).round() as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(card: &mut ChartCard, source: DataSource) -> f32 {
        let ctx = egui::Context::default();
        let mut height = 0.0;
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let start = ui.cursor().top();
                show(ui, card, source, Id::new("test-chart"), |_, _| {});
                height = ui.cursor().top() - start;
            });
            output.textures_delta.clear();
        }
        height
    }

    #[test]
    fn a_number_field_is_as_tall_as_the_pickers_and_buttons_beside_it() {
        let ctx = egui::Context::default();
        let (mut field, mut picker, mut button) = (0.0, 0.0, 0.0);
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.horizontal(|ui| {
                    number_field(ui, Id::new("n"), 3.0, 1.0, 0.0..=9.0, 0);
                    field = ui.min_rect().height();
                });
                ui.horizontal(|ui| {
                    kit::menu_picker(ui, 90.0, &["A".to_owned()], 0, true);
                    picker = ui.min_rect().height();
                });
                ui.horizontal(|ui| {
                    kit::push_button(ui, None, "Reset", true);
                    button = ui.min_rect().height();
                });
            });
            output.textures_delta.clear();
        }
        assert!(picker > 0.0 && button > 0.0);
        assert_eq!(
            field.round(),
            picker.round(),
            "field {field} picker {picker}"
        );
        assert_eq!(picker.round(), button.round());
    }

    #[test]
    fn every_type_draws_its_own_settings_without_panicking() {
        for kind in ChartType::ALL {
            let mut card = ChartCard::default();
            card.set_chart_type(kind);
            let height = draw(&mut card, DataSource::Binance);
            assert!(height > 120.0, "{kind:?} drew only {height}");
        }
    }

    #[test]
    fn settings_a_type_does_not_use_are_not_offered() {
        let mut candles = ChartCard::default();
        candles.set_chart_type(ChartType::Candles);
        let mut baseline = ChartCard::default();
        baseline.set_chart_type(ChartType::Baseline);
        // Baseline has no wicks or bodies to set up but its own level, fills and source.
        assert_ne!(
            draw(&mut candles, DataSource::Binance),
            draw(&mut baseline, DataSource::Binance)
        );
    }

    #[test]
    fn a_source_that_cannot_show_a_type_offers_the_candles_it_falls_back_to() {
        let mut card = ChartCard::default();
        card.set_chart_type(ChartType::VolumeCandles);
        draw(&mut card, DataSource::CoinGecko);
        // The choice is kept for a market that can show it.
        assert_eq!(card.appearance().style.chart_type, ChartType::VolumeCandles);
        assert_eq!(
            card.effective_chart_type(DataSource::CoinGecko),
            ChartType::Candles
        );
    }
}
