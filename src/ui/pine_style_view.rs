//! The Style page of a script's settings, ported from `PineStyleView`: one row per output the
//! script draws, with a visibility button and, where the script uses one fixed colour or width, a
//! colour well and a width stepper. A row the user changed shows a revert button. Stateless: it
//! reads the overrides and returns the next map when something changed.

use super::icons::{self, Icon};
use super::kit::{self, MEDIUM, REGULAR, size};
use super::theme;
use crate::pine::outputs::style_overrides::{StyleAttribute, StyleKind, StyleOverrides, StyleRow};
use crate::pine_studio::inputs::{color_from_rgba, rgba_from_color};
use eframe::egui::{self, Align, Id, Layout, Margin, Rect, Sense, Ui, UiBuilder, vec2};
use std::collections::HashMap;

const ROW_HEIGHT: f32 = 36.0;
const GAP: f32 = 10.0;
const WIDTH_COLUMN: f32 = 78.0;
const COLOR_COLUMN: f32 = 38.0;
const BUTTON_COLUMN: f32 = 28.0;
/// The widest line a row offers.
const WIDTHS: std::ops::RangeInclusive<i64> = 1..=10;

pub fn show(
    ui: &mut Ui,
    rows: &[StyleRow],
    overrides: &HashMap<String, String>,
) -> Option<HashMap<String, String>> {
    let mut next = None;
    let style = StyleOverrides::new(overrides.clone());
    kit::card_frame(false)
        .inner_margin(Margin::ZERO)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (index, row) in rows.iter().enumerate() {
                if index > 0 {
                    let (line, _) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                    ui.painter()
                        .rect_filled(line, 0.0, theme::with_alpha(theme::separator(), 0.5));
                }
                if let Some(edited) = row_view(ui, row, &style, overrides) {
                    next = Some(edited);
                }
            }
        });
    if has_per_bar_rows(rows) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(kit::text(
                    "Outputs without a color well take their color bar by bar from the script.",
                    size::CAPTION + 0.5,
                    REGULAR,
                    theme::text_tertiary(),
                ))
                .wrap(),
            );
        });
    }
    next
}

/// Whether any row's colour is computed by the script, so the footnote is worth showing.
pub fn has_per_bar_rows(rows: &[StyleRow]) -> bool {
    rows.iter()
        .any(|row| row.default_color.is_none() && !is_candle_part(row.kind))
}

/// A candle's body, wick or border, whose colour the chart may leave unsaid.
fn is_candle_part(kind: StyleKind) -> bool {
    matches!(
        kind,
        StyleKind::Candle | StyleKind::CandleWick | StyleKind::CandleBorder
    )
}

fn row_view(
    ui: &mut Ui,
    row: &StyleRow,
    style: &StyleOverrides,
    overrides: &HashMap<String, String>,
) -> Option<HashMap<String, String>> {
    let hidden = style.is_hidden(row.kind, row.output_id);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    let id = Id::new(("style-row", row.id()));
    let mut next = None;

    // Right to left: revert, visibility, colour, width — then the title takes what is left.
    let mut right = rect.right() - 12.0;
    let mut slot = |width: f32| {
        let slot = Rect::from_min_max(
            egui::pos2(right - width, rect.top()),
            egui::pos2(right, rect.bottom()),
        );
        right -= width + GAP;
        slot
    };
    let revert_slot = slot(BUTTON_COLUMN);
    let eye_slot = slot(BUTTON_COLUMN);
    let color_slot = slot(COLOR_COLUMN);
    let width_slot = slot(WIDTH_COLUMN);
    let title_right = right + GAP - 8.0;

    // The kind's glyph and the title.
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            egui::pos2(rect.left() + 12.0 + 9.0, rect.center().y),
            vec2(13.0, 13.0),
        ),
        icon(row.kind),
        theme::text_secondary(),
    );
    let title = kit::galley(
        ui,
        &row.title,
        size::SUBHEADLINE + 1.0,
        MEDIUM,
        if hidden {
            theme::text_tertiary()
        } else {
            theme::text_primary()
        },
        (title_right - (rect.left() + 12.0 + 18.0 + 10.0)).max(40.0),
    );
    ui.painter().galley(
        egui::pos2(
            rect.left() + 12.0 + 18.0 + 10.0,
            rect.center().y - title.size().y / 2.0,
        ),
        title,
        theme::text_primary(),
    );

    let place = |ui: &mut Ui, slot: Rect, layout: Layout, add: &mut dyn FnMut(&mut Ui)| {
        ui.scope_builder(UiBuilder::new().max_rect(slot).layout(layout), |ui| add(ui));
    };

    if let Some(default_width) = row.default_width {
        let width = style
            .width(row.kind, row.output_id)
            .unwrap_or(default_width) as i64;
        place(
            ui,
            width_slot,
            Layout::right_to_left(Align::Center),
            &mut |ui| {
                ui.add_enabled_ui(!hidden, |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let stepped = kit::stepper(ui, id.with("width"), width, WIDTHS, 1);
                    ui.label(kit::text(
                        width.to_string(),
                        size::SUBHEADLINE + 1.0,
                        REGULAR,
                        theme::text_secondary(),
                    ));
                    if let Some(value) = stepped {
                        next = Some(StyleOverrides::setting(
                            overrides.clone(),
                            row.kind,
                            row.output_id,
                            StyleAttribute::Width,
                            (value as usize != default_width).then(|| value.to_string()),
                        ));
                    }
                })
                .response
                .on_hover_text("Line width");
            },
        );
    }

    if let Some(default_color) = row.default_color {
        let current = style
            .color(row.kind, row.output_id)
            .unwrap_or(default_color);
        place(
            ui,
            color_slot,
            Layout::centered_and_justified(egui::Direction::LeftToRight),
            &mut |ui| {
                ui.add_enabled_ui(!hidden, |ui| {
                    let picked = kit::color_well(
                        ui,
                        vec2(COLOR_COLUMN - 6.0, 22.0),
                        &format!("{} color", row.title),
                        color_from_rgba(current),
                        egui::color_picker::Alpha::OnlyBlend,
                    );
                    if let Some(color) = picked {
                        let rgba = rgba_from_color(color);
                        next = Some(StyleOverrides::setting(
                            overrides.clone(),
                            row.kind,
                            row.output_id,
                            StyleAttribute::Color,
                            (rgba != default_color).then(|| StyleOverrides::hex(rgba)),
                        ));
                    }
                });
            },
        );
    }

    let eye_label = if hidden {
        format!("Show {}", row.title)
    } else {
        format!("Hide {}", row.title)
    };
    place(
        ui,
        eye_slot,
        Layout::centered_and_justified(egui::Direction::LeftToRight),
        &mut |ui| {
            let eye = if hidden { Icon::EyeSlash } else { Icon::Eye };
            if icons::button(ui, eye, 24.0, theme::text_secondary(), &eye_label).clicked() {
                next = Some(StyleOverrides::setting(
                    overrides.clone(),
                    row.kind,
                    row.output_id,
                    StyleAttribute::Visible,
                    (!hidden).then(|| "false".to_owned()),
                ));
            }
        },
    );

    if style.has_changes(row) {
        let label = format!("Revert {} to the script's style", row.title);
        place(
            ui,
            revert_slot,
            Layout::centered_and_justified(egui::Direction::LeftToRight),
            &mut |ui| {
                if icons::button(ui, Icon::Undo, 24.0, theme::text_secondary(), &label).clicked() {
                    next = Some(StyleOverrides::cleared(overrides.clone(), row));
                }
            },
        );
    }
    next
}

fn icon(kind: StyleKind) -> Icon {
    match kind {
        StyleKind::Plot => Icon::ChartLine,
        StyleKind::Hline => Icon::Minus,
        StyleKind::Fill => Icon::RectFill,
        StyleKind::Marker => Icon::Triangle,
        StyleKind::Candle | StyleKind::CandleWick | StyleKind::CandleBorder => Icon::BarChart,
        StyleKind::Bgcolor => Icon::RectDashed,
        StyleKind::Barcolor => Icon::Paintbrush,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: StyleKind, color: Option<u32>, width: Option<usize>) -> StyleRow {
        StyleRow {
            kind,
            output_id: 0,
            title: "Plot 1".into(),
            default_color: color,
            default_width: width,
        }
    }

    #[test]
    fn the_footnote_shows_only_for_outputs_the_script_colours_per_bar() {
        assert!(!has_per_bar_rows(&[row(StyleKind::Plot, Some(1), Some(2))]));
        assert!(has_per_bar_rows(&[row(StyleKind::Plot, None, Some(2))]));
        assert!(!has_per_bar_rows(&[row(StyleKind::Candle, None, None)]));
    }

    #[test]
    fn the_page_draws_rows_with_and_without_overrides() {
        let rows = [
            row(StyleKind::Plot, Some(0xff00_00ff), Some(2)),
            row(StyleKind::Fill, None, None),
        ];
        let mut overrides = HashMap::new();
        overrides.insert("plot.0.visible".to_owned(), "false".to_owned());
        let ctx = egui::Context::default();
        for overrides in [HashMap::new(), overrides] {
            for _ in 0..2 {
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    assert!(show(ui, &rows, &overrides).is_none(), "nothing clicked");
                });
                output.textures_delta.clear();
            }
        }
    }
}
