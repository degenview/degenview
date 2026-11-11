//! The settings of one compared market, shared by the Compare page of the chart settings and the
//! popover on a chart's overlay card: colour, line, price, scale and the last-value tag, each
//! answered with the [`OverlayCommand`] that makes the change.

use crate::model::chart::kind::{LineDash, PriceSource};
use crate::model::chart::price_scale::PriceScaleMode;
use crate::model::editor_theme::Rgba;
use crate::model::overlay::{
    ChartOverlay, OverlayCommand, OverlayId, OverlayScale, PALETTE, ScaleChoice,
};
use crate::ui::{kit, settings::card, theme};
use eframe::egui::{self, Align, Color32, Layout, Sense, Stroke, Ui, Vec2, vec2};

const WIDTHS: [f32; 6] = [0.5, 1.0, 1.5, 2.0, 3.0, 4.0];
/// The width of a setting's name in the grid.
const NAME_COLUMN: f32 = 104.0;

/// How roomy the settings are laid out.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Density {
    /// The Compare page: a wide name column, fixed-width pickers and a sentence under most settings.
    Roomy,
    /// The popover on the overlay card: a narrow name column, pickers as wide as their longest
    /// choice, and a sentence only where the setting is not self-explanatory (the scale).
    Compact,
}

impl Density {
    fn name_column(self) -> f32 {
        match self {
            Self::Roomy => NAME_COLUMN,
            Self::Compact => 0.0,
        }
    }

    fn spacing(self) -> [f32; 2] {
        match self {
            Self::Roomy => [16.0, 12.0],
            Self::Compact => [12.0, 10.0],
        }
    }

    fn roomy(self) -> bool {
        self == Self::Roomy
    }
}

/// A picker's width: its longest choice, the text's inset and the chevron's room.
fn fitted(ui: &Ui, labels: &[String]) -> f32 {
    labels
        .iter()
        .map(|label| {
            kit::galley(
                ui,
                label,
                kit::size::BODY,
                kit::REGULAR,
                theme::text_primary(),
                f32::INFINITY,
            )
            .size()
            .x
        })
        .fold(0.0, f32::max)
        + 38.0
}

/// The settings of one compared market, as a two-column grid of rows. Every change comes back as
/// the [`OverlayCommand`] that makes it; nothing here edits the chart. `own_scales` are the
/// markets with a scale of their own (the choices for "Scale of X"), `mode` the chart's axis.
pub fn body(
    ui: &mut Ui,
    overlay: &ChartOverlay,
    own_scales: &[(OverlayId, String)],
    mode: PriceScaleMode,
    density: Density,
) -> Option<OverlayCommand> {
    let mut action = None;
    let id = overlay.id;
    let mut send = |command: OverlayCommand| action = Some(command);
    egui::Grid::new(("compare-details", id, density.roomy()))
        .num_columns(2)
        .spacing(density.spacing())
        .min_col_width(density.name_column())
        .show(ui, |ui| {
            setting_name(ui, "Colour", 20.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = if density.roomy() { 6.0 } else { 4.0 };
                for hex in PALETTE {
                    let color = Rgba::hex(hex);
                    if swatch_button(ui, rgb(color), overlay.color == color) {
                        send(OverlayCommand::SetColor(id, color));
                    }
                }
                ui.add_space(4.0);
                if density.roomy() {
                    ui.label(kit::text(
                        "Custom",
                        10.5,
                        kit::REGULAR,
                        theme::text_secondary(),
                    ));
                }
                let custom = !PALETTE.iter().any(|hex| Rgba::hex(*hex) == overlay.color);
                if let Some(picked) = kit::color_well(
                    ui,
                    vec2(26.0, 20.0),
                    &format!("{} custom colour", overlay.name()),
                    rgb(overlay.color),
                    egui::color_picker::Alpha::Opaque,
                ) {
                    send(OverlayCommand::SetColor(
                        id,
                        Rgba::new(picked.r(), picked.g(), picked.b(), 255),
                    ));
                }
                if custom && density.roomy() {
                    ui.label(kit::text(
                        overlay.color.to_hex(),
                        10.5,
                        kit::REGULAR,
                        theme::text_tertiary(),
                    ));
                }
            });
            ui.end_row();

            setting_name(ui, "Line", 24.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let widths: Vec<String> = WIDTHS.iter().map(|w| format!("{w} px")).collect();
                let at = WIDTHS
                    .iter()
                    .position(|w| (*w - overlay.width).abs() < 0.01)
                    .unwrap_or(2);
                let width = if density.roomy() {
                    84.0
                } else {
                    fitted(ui, &widths)
                };
                if let Some(pick) = kit::menu_picker(ui, width, &widths, at, true) {
                    send(OverlayCommand::SetWidth(id, WIDTHS[pick]));
                }
                let dashes: Vec<String> =
                    LineDash::ALL.iter().map(|d| d.label().to_owned()).collect();
                let at = LineDash::ALL
                    .iter()
                    .position(|d| *d == overlay.dash)
                    .unwrap_or(0);
                let width = if density.roomy() {
                    96.0
                } else {
                    fitted(ui, &dashes)
                };
                if let Some(pick) = kit::menu_picker(ui, width, &dashes, at, true) {
                    send(OverlayCommand::SetDash(id, LineDash::ALL[pick]));
                }
            });
            ui.end_row();

            setting_name(ui, "Price", 24.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                let sources: Vec<String> = PriceSource::ALL
                    .iter()
                    .map(|s| s.label().to_owned())
                    .collect();
                let at = PriceSource::ALL
                    .iter()
                    .position(|s| *s == overlay.source)
                    .unwrap_or(3);
                let width = if density.roomy() {
                    96.0
                } else {
                    fitted(ui, &sources)
                };
                if let Some(pick) = kit::menu_picker(ui, width, &sources, at, true) {
                    send(OverlayCommand::SetSource(id, PriceSource::ALL[pick]));
                }
                if density.roomy() {
                    hint(ui, "The value of each candle the line follows.");
                }
            });
            ui.end_row();

            setting_name(ui, "Scale", 24.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                let mut labels = vec!["Chart's scale".to_owned(), "Its own scale".to_owned()];
                let mut choices = vec![ScaleChoice::Main, ScaleChoice::Own];
                for (other, name) in own_scales.iter().filter(|(other, _)| *other != id) {
                    labels.push(format!("Scale of {name}"));
                    choices.push(ScaleChoice::SharedWith(*other));
                }
                let at = match overlay.scale {
                    OverlayScale::Main => 0,
                    OverlayScale::Own(group) if group == id => 1,
                    OverlayScale::Own(group) => own_scales
                        .iter()
                        .filter(|(other, _)| *other != id)
                        .position(|(other, _)| *other == group)
                        .map_or(1, |n| n + 2),
                };
                let width = if density.roomy() {
                    190.0
                } else {
                    fitted(ui, &labels)
                };
                if let Some(pick) = kit::menu_picker(ui, width, &labels, at, true) {
                    send(OverlayCommand::SetScale(id, choices[pick]));
                }
                hint(ui, &scale_hint(overlay, own_scales, mode));
            });
            ui.end_row();

            // The roomy page keeps it as a row of the grid; the popover puts it under the grid, so
            // the grid's name column is as wide as its own names and not as "Last value tag".
            if density.roomy() {
                setting_name(ui, "Last value tag", 20.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    let mut shown = overlay.show_label;
                    if card::switch(ui, &mut shown) {
                        send(OverlayCommand::SetLabel(id, shown));
                    }
                    hint(ui, "A label at the end of the line with its latest value.");
                });
                ui.end_row();
            }
        });
    if !density.roomy() {
        ui.add_space(12.0);
        // The switch right after its name, not at the far edge, so the two read as one setting.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.label(kit::text(
                "Last value tag",
                11.5,
                kit::MEDIUM,
                theme::text_secondary(),
            ));
            let mut shown = overlay.show_label;
            if card::switch(ui, &mut shown) {
                action = Some(OverlayCommand::SetLabel(id, shown));
            }
        });
    }
    action
}

/// One sentence on what a market's scale does to its line.
pub fn scale_hint(
    overlay: &ChartOverlay,
    own_scales: &[(OverlayId, String)],
    mode: PriceScaleMode,
) -> String {
    match overlay.scale {
        OverlayScale::Main => match mode {
            PriceScaleMode::Percent | PriceScaleMode::IndexedTo100 => {
                "Starts where the chart does, so the axis reads this market's own change.".to_owned()
            }
            PriceScaleMode::Regular | PriceScaleMode::Logarithmic => {
                "Starts where the chart does. Use the Percent or Indexed axis to read it as a change."
                    .to_owned()
            }
        },
        OverlayScale::Own(group) if group == overlay.id => {
            "Fitted to the height of the chart. Its price is in the tag and the legend, not on the axis."
                .to_owned()
        }
        OverlayScale::Own(group) => {
            let owner = own_scales
                .iter()
                .find(|(other, _)| *other == group)
                .map_or("another market", |(_, name)| name.as_str());
            format!("Fitted together with {owner}, so they can be read against each other.")
        }
    }
}

/// The name of a setting, level with the middle of the first control of its row: `control` is that
/// control's height (a 24 pt picker, a 20 pt swatch or switch).
pub(in crate::ui) fn setting_name(ui: &mut Ui, name: &str, control: f32) {
    ui.with_layout(Layout::top_down(Align::Min), |ui| {
        ui.add_space(((control - 14.0) / 2.0).max(0.0));
        ui.add(
            egui::Label::new(kit::text(name, 11.5, kit::MEDIUM, theme::text_secondary()))
                .wrap_mode(egui::TextWrapMode::Extend),
        );
    });
}

pub(in crate::ui) fn hint(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(kit::text(text, 10.5, kit::REGULAR, theme::text_secondary())).wrap());
}

/// A palette colour as a round button, ringed while it is the market's colour.
fn swatch_button(ui: &mut Ui, color: Color32, selected: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let center = rect.center();
    ui.painter().circle_filled(center, 7.5, color);
    if selected {
        ui.painter()
            .circle_stroke(center, 9.5, Stroke::new(1.5, theme::text_primary()));
    } else if response.hovered() {
        ui.painter()
            .circle_stroke(center, 9.5, Stroke::new(1.0, theme::ink(40)));
    }
    crate::ui::accessibility::label(response, format!("Colour {}", color_name(color))).clicked()
}

fn color_name(color: Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", color.r(), color.g(), color.b())
}

pub(in crate::ui) fn rgb(color: Rgba) -> Color32 {
    Color32::from_rgb(color.r, color.g, color.b)
}
