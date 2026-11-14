//! The Text page: what a drawing says and where its text sits.

use super::DrawingProperties;
use super::rows::{Page, color_card, toggle_row};
use crate::model::drawings::style::{HorizontalAlign, TextOrientation, VerticalAlign};
use crate::model::{Caps, Tool};
use crate::ui::chart::settings::style::{number_field, picker_row};
use crate::ui::icons::Icon;
use crate::ui::{kit, settings::card};
use eframe::egui::{self, Id, Ui};

/// Whether a tool writes its labels itself, so its text page is only their font (and, for a
/// measurer, a note): a pattern's names, a wave's, a position's stats, a forecast's, a measurement.
fn labels_are_written_by_the_tool(caps: Caps) -> bool {
    [
        Caps::PATTERN,
        Caps::WAVE,
        Caps::POSITION,
        Caps::FORECAST,
        Caps::MEASURE,
    ]
    .into_iter()
    .any(|cap| caps.has(cap))
}

/// Whether a tool's text has a vertical alignment that means something: inside a shape (top,
/// middle or bottom of it), or on a line or channel (above it, on it or below it).
fn text_has_position(tool: Tool) -> bool {
    use crate::model::ToolGroup;
    matches!(
        tool,
        Tool::Rectangle | Tool::Circle | Tool::Ellipse | Tool::Table
    ) || matches!(
        tool.spec().group,
        ToolGroup::TrendLines | ToolGroup::Channels
    )
}

impl DrawingProperties {
    pub(super) fn text_page(&mut self, ui: &mut Ui) {
        let mut page = Page::new(true);
        // A price label's text is its price: there is nothing to type.
        if self.tool.caps().has(Caps::TEXT) {
            page.section(ui, "Text", None);
            let mut text = self.draft.text().to_owned();
            let focused_id = Id::new("drawing-text");
            let focused = ui.memory(|memory| memory.has_focus(focused_id));
            kit::field_frame(focused).show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .id(focused_id)
                        .frame(egui::Frame::NONE)
                        .hint_text("Add text to this drawing")
                        .char_limit(self.draft.max_text())
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                );
            });
            if text != self.draft.text() {
                self.draft.set_text(&text);
            }
        }

        let mut style = self.style();
        let before = style;
        page.section(ui, "Font", None);
        color_card(ui, Icon::Palette, "Text color", None, &mut style.text.color);
        if let Some(size) = card::card(ui, Icon::TextFormat, "Size", None, |ui| {
            number_field(
                ui,
                Id::new("drawing-text-size"),
                f64::from(style.text.size),
                1.0,
                f64::from(crate::model::drawings::style::TEXT_SIZE_RANGE.0)
                    ..=f64::from(crate::model::drawings::style::TEXT_SIZE_RANGE.1),
                0,
            )
        }) {
            style.text.size = size as f32;
        }
        toggle_row(ui, Icon::TextFormat, "Bold", "", &mut style.text.bold);
        toggle_row(ui, Icon::TextFormat, "Italic", "", &mut style.text.italic);
        if labels_are_written_by_the_tool(self.tool.caps()) {
            if style != before {
                self.set_style(style);
            }
            return;
        }
        page.section(ui, "Alignment", None);
        let horizontal: Vec<(HorizontalAlign, &str)> = HorizontalAlign::ALL
            .iter()
            .map(|align| (*align, align.label()))
            .collect();
        picker_row(
            ui,
            Icon::Sliders,
            "Alignment",
            None,
            &horizontal,
            &mut style.text.align,
        );
        if text_has_position(self.tool) {
            let vertical: Vec<(VerticalAlign, &str)> = VerticalAlign::ALL
                .iter()
                .map(|align| (*align, align.label()))
                .collect();
            picker_row(
                ui,
                Icon::Sliders,
                "Position",
                None,
                &vertical,
                &mut style.text.valign,
            );
        }
        if self.tool.caps().has(Caps::ORIENTATION) {
            // A vertical line's text reads up the line or across it.
            let options = [
                (TextOrientation::Vertical, "Vertical"),
                (TextOrientation::Horizontal, "Horizontal"),
            ];
            let mut shown = if style.text.orientation == TextOrientation::Horizontal {
                TextOrientation::Horizontal
            } else {
                TextOrientation::Vertical
            };
            picker_row(
                ui,
                Icon::TextFormat,
                "Orientation",
                None,
                &options,
                &mut shown,
            );
            style.text.orientation = shown;
        }
        if self.tool.caps().has(Caps::BOX) && !self.tool.caps().has(Caps::AUTO_TEXT) {
            page.section(ui, "Box", None);
            toggle_row(
                ui,
                Icon::TextFormat,
                "Wrap text",
                "Break long lines to fit a box of the width below.",
                &mut style.wrap,
            );
            if style.wrap {
                use crate::model::drawings::style::{MAX_BOX_WIDTH, MIN_BOX_WIDTH};
                if let Some(width) = card::card(ui, Icon::ArrowLeftRight, "Width", None, |ui| {
                    number_field(
                        ui,
                        Id::new("drawing-box-width"),
                        f64::from(style.wrapped_width()),
                        10.0,
                        f64::from(MIN_BOX_WIDTH)..=f64::from(MAX_BOX_WIDTH),
                        0,
                    )
                }) {
                    style.box_width = width as f32;
                }
            }
        }
        let mut visible = !style.hide_text;
        toggle_row(
            ui,
            Icon::Eye,
            "Show text",
            "Off keeps the text but does not draw it.",
            &mut visible,
        );
        style.hide_text = !visible;
        if style != before {
            self.set_style(style);
        }
    }
}
