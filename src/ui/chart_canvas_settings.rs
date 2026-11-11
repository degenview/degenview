//! The Canvas page of the chart settings: the background, grid, separators, crosshair,
//! watermark, scales, on-chart buttons and margins. Each control writes straight into the chart's
//! [`ChartCanvas`], so the chart behind the sheet changes as the control does. Nothing here
//! touches the market data, the bars or a script: it is the look of the page the chart is drawn on.
//!
//! A colour left unset follows the app theme (the swatch shows what it follows); once picked it
//! stays through theme changes until "Reset" hands it back.

use super::canvas_render::{self, CanvasStyle, Fill};
use super::chart_card::ChartCard;
use super::chart_style_settings::{color_control, int_row, number_field};
use super::icons::Icon;
use super::kit;
use super::settings_card;
use crate::model::ChartCanvas;
use crate::model::chart::canvas::{
    BackgroundMode, ButtonVisibility, ChartMargins, LINE_WIDTHS, LineAppearance, ScaleTextSize,
    WatermarkMode,
};
use crate::model::chart::kind::{LineDash, Rgba};
use eframe::egui::{self, Id, Sense, Stroke, StrokeKind, Ui, vec2};

/// The width of a switch: the column rows without one keep free (or put a preview in), so the
/// controls of every row line up on the right.
const SWITCH_COLUMN: f32 = 36.0;

/// Room kept for the controls of a card that holds several.
const WIDE: f32 = 330.0;

/// The vertical rhythm used by the Appearance page between its setting cards.
const CARD_SPACING: f32 = 8.0;

/// A theme default as the `Rgba` a swatch shows.
fn swatch(color: egui::Color32) -> Rgba {
    color.to_srgba_unmultiplied()
}

/// Draw a section heading without letting its tight title/subtitle spacing leak into the cards
/// below it. Appearance restores this spacing after its heading, so Canvas does the same for each
/// of its sections.
fn section_title(ui: &mut Ui, title: &str, subtitle: Option<&str>) {
    settings_card::section_title(ui, title, subtitle);
    ui.spacing_mut().item_spacing.y = CARD_SPACING;
}

/// What the page draws for `card`. `has_panes` is whether the chart has script panes: the
/// separator setting has nothing to style without them. `beside_reset` draws what sits next to
/// the reset button.
pub fn show(
    ui: &mut Ui,
    card: &mut ChartCard,
    id: Id,
    beside_reset: impl FnOnce(&mut Ui, &mut ChartCard),
) {
    let has_panes = card.pane_count() > 0;
    ui.spacing_mut().item_spacing.y = CARD_SPACING;
    let defaults = CanvasStyle::resolve(&ChartCanvas::default());
    let canvas = &mut card.appearance_mut().canvas;

    section_title(
        ui,
        "Chart basic styles",
        Some("The page the chart is drawn on. Colors you leave alone follow the app theme."),
    );
    background_row(ui, canvas, &defaults);
    grid_row(
        ui,
        "Vertical grid lines",
        "Drawn on the time axis' labels.",
        Icon::SplitColumns,
        &mut canvas.vertical_grid,
        swatch(
            defaults
                .vertical_grid
                .map_or(egui::Color32::GRAY, |line| line.stroke.color),
        ),
    );
    grid_row(
        ui,
        "Horizontal grid lines",
        "Drawn on the price axis' labels.",
        Icon::List,
        &mut canvas.horizontal_grid,
        swatch(
            defaults
                .horizontal_grid
                .map_or(egui::Color32::GRAY, |line| line.stroke.color),
        ),
    );
    separator_row(
        ui,
        id,
        &mut canvas.pane_separator,
        swatch(
            defaults
                .separator
                .map_or(egui::Color32::GRAY, |line| line.stroke.color),
        ),
        has_panes,
    );
    crosshair_row(
        ui,
        id,
        &mut canvas.crosshair,
        swatch(defaults.crosshair.stroke.color),
    );
    watermark_row(ui, canvas, swatch(defaults.watermark));
    readout_row(ui, &mut canvas.ohlc_readout);

    ui.add_space(16.0);
    section_title(ui, "Scales", Some("The price and time axes."));
    scales_rows(ui, canvas, &defaults);

    ui.add_space(16.0);
    section_title(
        ui,
        "Buttons",
        Some(
            "Hiding a button group hides only the buttons: the wheel, the trackpad and the keyboard still work.",
        ),
    );
    button_rows(ui, canvas);

    ui.add_space(16.0);
    section_title(ui, "Margins", Some("The room around the price range."));
    margin_rows(ui, id, &mut canvas.margins);

    ui.add_space(12.0);
    let changed = *canvas != ChartCanvas::default();
    ui.horizontal(|ui| {
        if kit::push_button(ui, Some(Icon::Undo), "Reset Canvas to Defaults", changed)
            .on_hover_text("Puts every setting on this page back. Candle colors and the chart type are not touched.")
            .clicked()
        {
            card.appearance_mut().canvas = ChartCanvas::default();
        }
        beside_reset(ui, card);
    });
}

// MARK: Background

fn background_row(ui: &mut Ui, canvas: &mut ChartCanvas, defaults: &CanvasStyle) {
    let default_fill = defaults.fill;
    let Fill::Solid(surface) = default_fill else {
        return;
    };
    let surface = swatch(surface);
    let bottom_default = {
        let mut gradient = ChartCanvas::default();
        gradient.background.mode = BackgroundMode::Gradient;
        match CanvasStyle::resolve(&gradient).fill {
            Fill::Gradient { bottom, .. } => swatch(bottom),
            Fill::Solid(color) => swatch(color),
        }
    };
    let preview = CanvasStyle::resolve(canvas).fill;
    let background = &mut canvas.background;
    let modes: Vec<String> = BackgroundMode::ALL
        .iter()
        .map(|mode| mode.label().to_owned())
        .collect();
    settings_card::card_reserving(
        ui,
        Icon::Palette,
        "Background",
        Some(
            "One color, or a gradient from top to bottom, behind the plot, its scales and every pane.",
        ),
        WIDE,
        |ui| {
            let custom = match background.mode {
                BackgroundMode::Solid => background.solid.is_some(),
                BackgroundMode::Gradient => background.top.is_some() || background.bottom.is_some(),
            };
            if custom && kit::push_button(ui, None, "Reset", true).clicked() {
                match background.mode {
                    BackgroundMode::Solid => background.solid = None,
                    BackgroundMode::Gradient => {
                        background.top = None;
                        background.bottom = None;
                    }
                }
            }
            match background.mode {
                BackgroundMode::Solid => {
                    well(ui, "Background color", &mut background.solid, surface);
                }
                BackgroundMode::Gradient => {
                    // Right to left: the bottom stop first, so the top reads first.
                    well(
                        ui,
                        "Gradient bottom color",
                        &mut background.bottom,
                        bottom_default,
                    );
                    well(ui, "Gradient top color", &mut background.top, surface);
                }
            }
            let selected = BackgroundMode::ALL
                .iter()
                .position(|mode| *mode == background.mode)
                .unwrap_or(0);
            if let Some(index) = kit::menu_picker(ui, 96.0, &modes, selected, true) {
                background.mode = BackgroundMode::ALL[index];
            }
            fill_preview(ui, preview);
        },
    );
}

/// A small swatch of what the backdrop looks like now.
fn fill_preview(ui: &mut Ui, fill: Fill) {
    let (rect, response) = ui.allocate_exact_size(vec2(SWITCH_COLUMN, 24.0), Sense::hover());
    super::accessibility::label_as(
        response,
        egui::WidgetType::Image,
        match fill {
            Fill::Solid(_) => "Background preview, one color",
            Fill::Gradient { .. } => "Background preview, a gradient",
        },
    );
    let painter = ui.painter().with_clip_rect(rect);
    canvas_render::paint_background(&painter, rect, fill);
    ui.painter().rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, super::theme::ink(40)),
        StrokeKind::Inside,
    );
}

/// A colour well that follows `default` until picked. A colour without a reset button of its
/// own: the card's one "Reset" covers it.
fn well(ui: &mut Ui, name: &str, slot: &mut Option<Rgba>, default: Rgba) {
    let shown = super::series_render::color(slot.unwrap_or(default));
    if let Some(picked) = kit::color_well(
        ui,
        vec2(44.0, 24.0),
        name,
        shown,
        egui::color_picker::Alpha::OnlyBlend,
    ) {
        *slot = Some(picked.to_srgba_unmultiplied());
    }
}

// MARK: Lines

fn dash_labels() -> Vec<String> {
    LineDash::ALL
        .iter()
        .map(|dash| dash.label().to_owned())
        .collect()
}

fn dash_picker(ui: &mut Ui, dash: &mut LineDash) {
    let selected = LineDash::ALL.iter().position(|d| d == dash).unwrap_or(0);
    if let Some(index) = kit::menu_picker(ui, 92.0, &dash_labels(), selected, true) {
        *dash = LineDash::ALL[index];
    }
}

/// A grid in one card: on/off, its colour (its opacity is the colour's) and its line style.
fn grid_row(
    ui: &mut Ui,
    title: &str,
    hint: &str,
    icon: Icon,
    line: &mut LineAppearance,
    default: Rgba,
) {
    settings_card::card_reserving(ui, icon, title, Some(hint), WIDE - 90.0, |ui| {
        settings_card::switch(ui, &mut line.visible);
        if line.visible {
            dash_picker(ui, &mut line.dash);
            color_control(ui, title, &mut line.color, default);
        }
    });
}

fn separator_row(ui: &mut Ui, id: Id, line: &mut LineAppearance, default: Rgba, has_panes: bool) {
    if !has_panes {
        settings_card::card(
            ui,
            Icon::Layers,
            "Pane separators",
            Some(
                "The line between the price chart and a script's pane. It appears once a script with its own pane is on the chart.",
            ),
            |ui| {
                ui.add_enabled(
                    false,
                    egui::Label::new(super::kit::text(
                        "No panes",
                        11.0,
                        kit::REGULAR,
                        super::theme::text_tertiary(),
                    )),
                );
            },
        );
        return;
    }
    settings_card::card_reserving(
        ui,
        Icon::Layers,
        "Pane separators",
        Some("The line between the price chart and each script pane."),
        WIDE + 80.0,
        |ui| {
            settings_card::switch(ui, &mut line.visible);
            if line.visible {
                width_stepper(ui, id.with("separator-width"), &mut line.width);
                dash_picker(ui, &mut line.dash);
                color_control(ui, "Pane separators", &mut line.color, default);
            }
        },
    );
}

fn crosshair_row(ui: &mut Ui, id: Id, line: &mut LineAppearance, default: Rgba) {
    settings_card::card_reserving(
        ui,
        Icon::Plus,
        "Crosshair",
        Some("The lines that follow the pointer. Color, opacity, thickness and style."),
        WIDE + 80.0,
        |ui| {
            ui.add_space(SWITCH_COLUMN + ui.spacing().item_spacing.x);
            width_stepper(ui, id.with("crosshair-width"), &mut line.width);
            dash_picker(ui, &mut line.dash);
            color_control(ui, "Crosshair", &mut line.color, default);
        },
    );
}

/// The thickness in points, a number field with its stepper.
fn width_stepper(ui: &mut Ui, id: Id, width: &mut u8) {
    let range = f64::from(*LINE_WIDTHS.start())..=f64::from(*LINE_WIDTHS.end());
    if let Some(next) = number_field(ui, id, f64::from(*width), 1.0, range, 0) {
        *width = next.round() as u8;
    }
}

// MARK: Watermark

fn watermark_row(ui: &mut Ui, canvas: &mut ChartCanvas, default: Rgba) {
    let watermark = &mut canvas.watermark;
    let options: Vec<(WatermarkMode, &str)> = WatermarkMode::ALL
        .into_iter()
        .map(|mode| (mode, mode.label()))
        .collect();
    let labels: Vec<String> = options
        .iter()
        .map(|(_, label)| (*label).to_owned())
        .collect();
    let selected = options
        .iter()
        .position(|(mode, _)| *mode == watermark.mode)
        .unwrap_or(0);
    let hint = match watermark.mode {
        WatermarkMode::Hidden => "A faint market name behind the candles. Off.",
        WatermarkMode::Symbol => "The market and its timeframe, behind the candles.",
        WatermarkMode::Replay => {
            "The market and its timeframe, behind the candles while a replay is running."
        }
    };
    settings_card::card_reserving(ui, Icon::Tag, "Watermark", Some(hint), 250.0, |ui| {
        ui.add_space(SWITCH_COLUMN + ui.spacing().item_spacing.x);
        if watermark.mode != WatermarkMode::Hidden {
            color_control(ui, "Watermark", &mut watermark.color, default);
        }
        if let Some(index) = kit::menu_picker(ui, 120.0, &labels, selected, true) {
            watermark.mode = options[index].0;
        }
    });
}

// MARK: Readout

/// The box of values under the crosshair, on or off. The crosshair lines and tags stay either way.
fn readout_row(ui: &mut Ui, on: &mut bool) {
    let hint = if *on {
        "Open, high, low and close under the crosshair, with volume where the market has it."
    } else {
        "The values box under the crosshair is hidden. The crosshair itself stays."
    };
    settings_card::card(ui, Icon::Eye, "OHLC values", Some(hint), |ui| {
        settings_card::switch(ui, on);
    });
}

// MARK: Scales

fn scales_rows(ui: &mut Ui, canvas: &mut ChartCanvas, defaults: &CanvasStyle) {
    let scales = &mut canvas.scales;
    let sizes: Vec<String> = ScaleTextSize::ALL.iter().map(|size| size.label()).collect();
    let selected = ScaleTextSize::ALL
        .iter()
        .position(|size| *size == scales.text_size)
        .unwrap_or(0);
    settings_card::card_reserving(
        ui,
        Icon::TextFormat,
        "Text",
        Some("The size and color of the price and time labels."),
        WIDE - 80.0,
        |ui| {
            color_control(
                ui,
                "Scale text",
                &mut scales.text_color,
                swatch(defaults.text_color),
            );
            if let Some(index) = kit::menu_picker(ui, 88.0, &sizes, selected, true) {
                scales.text_size = ScaleTextSize::ALL[index];
            }
        },
    );
    settings_card::card(
        ui,
        Icon::LineDiagonal,
        "Lines",
        Some("The lines that bound the price and time axes."),
        |ui| {
            color_control(
                ui,
                "Scale lines",
                &mut scales.line_color,
                swatch(defaults.scale_line),
            );
        },
    );
}

// MARK: Buttons

fn button_rows(ui: &mut Ui, canvas: &mut ChartCanvas) {
    visibility_row(
        ui,
        Icon::ArrowLeftRight,
        "Navigation",
        "Zoom, scroll and reset, at the foot of the chart.",
        &mut canvas.buttons.navigation,
    );
    visibility_row(
        ui,
        Icon::Layers,
        "Pane",
        "Move, collapse, maximize and remove, on each script pane.",
        &mut canvas.buttons.pane,
    );
}

fn visibility_row(ui: &mut Ui, icon: Icon, title: &str, hint: &str, value: &mut ButtonVisibility) {
    let options: Vec<(ButtonVisibility, &str)> = ButtonVisibility::ALL
        .into_iter()
        .map(|option| (option, option.label()))
        .collect();
    let labels: Vec<String> = options
        .iter()
        .map(|(_, label)| (*label).to_owned())
        .collect();
    let selected = options
        .iter()
        .position(|(option, _)| option == value)
        .unwrap_or(0);
    if let Some(index) = settings_card::card_reserving(ui, icon, title, Some(hint), 190.0, |ui| {
        kit::menu_picker(ui, 170.0, &labels, selected, true)
    }) {
        *value = options[index].0;
    }
}

// MARK: Margins

fn margin_rows(ui: &mut Ui, id: Id, margins: &mut ChartMargins) {
    // The pair may not leave less than a fifth of the height: the bound of one is what the other
    // leaves, so an edit never has to take a value back from the other field.
    let top_room = ChartMargins::MAX_COMBINED
        .saturating_sub(margins.bottom)
        .min(ChartMargins::MAX_VERTICAL);
    let mut top = i64::from(margins.top);
    if int_row(
        ui,
        id.with("margin-top"),
        Icon::Percent,
        "Top",
        Some("Percent of the chart's height kept free above the highest price."),
        &mut top,
        0..=i64::from(top_room),
    ) {
        margins.top = top as u8;
    }
    let bottom_room = ChartMargins::MAX_COMBINED
        .saturating_sub(margins.top)
        .min(ChartMargins::MAX_VERTICAL);
    let mut bottom = i64::from(margins.bottom);
    if int_row(
        ui,
        id.with("margin-bottom"),
        Icon::Percent,
        "Bottom",
        Some("Percent of the chart's height kept free below the lowest price."),
        &mut bottom,
        0..=i64::from(bottom_room),
    ) {
        margins.bottom = bottom as u8;
    }
    let mut right = i64::from(margins.right);
    if int_row(
        ui,
        id.with("margin-right"),
        Icon::Hash,
        "Right",
        Some("Bars of empty space after the newest bar."),
        &mut right,
        0..=i64::from(ChartMargins::MAX_RIGHT),
    ) {
        margins.right = right as u16;
    }
    *margins = margins.sanitized();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(card: &mut ChartCard) -> f32 {
        let ctx = egui::Context::default();
        let mut height = 0.0;
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let start = ui.cursor().top();
                show(ui, card, Id::new("test-canvas"), |_, _| {});
                height = ui.cursor().top() - start;
            });
            output.textures_delta.clear();
        }
        height
    }

    #[test]
    fn the_page_draws_with_every_option_in_every_state() {
        let mut card = ChartCard::default();
        let plain = draw(&mut card);
        assert!(plain > 600.0, "{plain}");
        {
            let canvas = &mut card.appearance_mut().canvas;
            canvas.background.mode = BackgroundMode::Gradient;
            canvas.vertical_grid.visible = false;
            canvas.horizontal_grid.dash = LineDash::Dotted;
            canvas.watermark.mode = WatermarkMode::Symbol;
            canvas.ohlc_readout = false;
        }
        assert!(draw(&mut card) > 600.0);
        assert!(
            !card.appearance().canvas.ohlc_readout,
            "drawing keeps it off"
        );
    }

    #[test]
    fn section_titles_restore_the_appearance_card_spacing() {
        let ctx = egui::Context::default();
        let mut spacing = 0.0;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            section_title(ui, "Canvas section", Some("A subtitle."));
            spacing = ui.spacing().item_spacing.y;
        });
        output.textures_delta.clear();
        assert_eq!(spacing, CARD_SPACING);
    }

    #[test]
    fn the_separator_row_explains_itself_until_a_pane_exists() {
        let mut card = ChartCard::default();
        // With no panes the row is one line of explanation; the settings change nothing.
        let before = card.appearance().canvas.pane_separator;
        draw(&mut card);
        assert_eq!(card.appearance().canvas.pane_separator, before);
        assert_eq!(card.pane_count(), 0);
    }

    #[test]
    fn a_colour_left_alone_stays_unset_after_drawing() {
        let mut card = ChartCard::default();
        draw(&mut card);
        assert_eq!(card.appearance().canvas, ChartCanvas::default());
    }

    #[test]
    fn margin_bounds_leave_the_other_field_its_room() {
        let mut margins = ChartMargins {
            top: 45,
            bottom: 35,
            right: 0,
        };
        // Drawing never changes a valid pair.
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            margin_rows(ui, Id::new("m"), &mut margins);
        });
        output.textures_delta.clear();
        assert_eq!((margins.top, margins.bottom), (45, 35));
    }
}
