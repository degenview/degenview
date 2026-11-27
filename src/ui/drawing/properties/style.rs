//! The Style page: lines, fills, labels and the statistics a tool shows, and a pitchfork's
//! variant.

use super::DrawingProperties;
use super::rows::{Page, color_card, color_well, dash_card, toggle_row};
use crate::i18n::{tr, tr_c};
use crate::model::drawings::style::{DrawingStyle, Emoji, LineEnd, StatsPosition};
use crate::model::{Caps, Geometry, PitchforkVariant, Tool};
use crate::ui::chart::settings::style::{number_field, picker_row};
use crate::ui::icons::Icon;
use crate::ui::settings::card;
use eframe::egui::{Id, Ui};

impl DrawingProperties {
    pub(super) fn style_page(&mut self, ui: &mut Ui) {
        let caps = self.tool.caps();
        let mut style = self.style();
        let before = style;
        let line = caps.has(Caps::COLOR) || caps.has(Caps::WIDTH) || caps.has(Caps::DASH);
        let boxed = caps.has(Caps::BOX);
        let shape = caps.has(Caps::FILL) || caps.has(Caps::MIDDLE_LINE);
        let details = caps.has(Caps::MIDDLE_POINT)
            || caps.has(Caps::PRICE_LABELS)
            || caps.has(Caps::TIME_LABEL);
        let variant = match self.draft.geometry() {
            Some(Geometry::Pitchfork { variant, .. }) => Some(*variant),
            _ => None,
        };
        let groups = [
            variant.is_some(),
            line,
            caps.has(Caps::ENDS),
            shape,
            caps.has(Caps::EXTEND),
            details,
            caps.has(Caps::STATS),
            caps.has(Caps::FIB),
        ];
        // A page of one group needs no heading to say what it is.
        let mut page = Page::new(groups.into_iter().filter(|present| *present).count() > 1);

        // The four pitchforks are one drawing in four constructions: switching keeps the three
        // pivots, so a fork can be tried every way without redrawing it.
        if let Some(current) = variant {
            page.section(ui, tr!("Pitchfork"), None);
            let options: Vec<(PitchforkVariant, &str)> = PitchforkVariant::ALL
                .iter()
                .map(|option| (*option, option.label()))
                .collect();
            let mut chosen = current;
            picker_row(
                ui,
                Icon::ForkAndrews,
                tr!("Style"),
                Some(tr!("Where the median starts and ends.")),
                &options,
                &mut chosen,
            );
            if chosen != current {
                self.switch_variant(chosen);
            }
        }
        if line {
            let title = if variant.is_some() {
                tr!("Median")
            } else if caps.has(Caps::FIB) {
                tr!("Trend line")
            } else if caps.has(Caps::WIDTH) || caps.has(Caps::DASH) {
                tr!("Line")
            } else {
                tr!("Marker")
            };
            page.section(ui, title, None);
        }
        if caps.has(Caps::COLOR) {
            let title = if variant.is_some() {
                tr!("Median color")
            } else if caps.has(Caps::WIDTH) {
                tr!("Line color")
            } else {
                tr!("Color")
            };
            // A wave's labels are in its line's colour until they are given one of their own.
            let labels_follow = caps.has(Caps::WAVE) && style.text.color == style.stroke.color;
            color_card(ui, Icon::Palette, title, None, &mut style.stroke.color);
            // The marks and the marker carry their text in the same colour; a box has its own.
            if !caps.has(Caps::WIDTH) && !boxed || labels_follow {
                style.text.color = style.stroke.color;
            }
        }
        if caps.has(Caps::WIDTH) {
            let max = crate::model::drawings::style::MAX_STROKE_WIDTH as f64;
            if let Some(width) = card::card(ui, Icon::Sliders, tr!("Thickness"), None, |ui| {
                number_field(
                    ui,
                    Id::new("drawing-width"),
                    f64::from(style.stroke.width),
                    1.0,
                    1.0..=max,
                    0,
                )
            }) {
                style.stroke.width = width as f32;
            }
        }
        if caps.has(Caps::DASH) {
            dash_card(ui, tr!("Line style"), &mut style.stroke.dash);
        }
        if caps.has(Caps::ENDS) {
            page.section(ui, tr!("Arrowheads"), None);
            let options: Vec<(LineEnd, &str)> =
                LineEnd::ALL.iter().map(|end| (*end, end.label())).collect();
            picker_row(
                ui,
                Icon::ArrowUpRight,
                tr!("Start"),
                None,
                &options,
                &mut style.head_start,
            );
            picker_row(
                ui,
                Icon::ArrowUpRight,
                tr!("End"),
                None,
                &options,
                &mut style.head_end,
            );
        }
        if shape {
            page.section(ui, if boxed { tr!("Box") } else { tr!("Shape") }, None);
        }
        if caps.has(Caps::FILL) {
            // A channel or a fork fills the space between its lines: a background.
            let title = if self.tool.spec().group == crate::model::ToolGroup::Shapes {
                tr!("Fill")
            } else if self.tool == Tool::Signpost {
                tr!("Plate")
            } else {
                tr!("Background")
            };
            card::card(ui, Icon::RectFill, title, None, |ui| {
                color_well(ui, tr!("Fill color"), &mut style.fill.color);
                card::switch(ui, &mut style.fill.enabled);
            });
        }
        if boxed {
            card::card(ui, Icon::RectDashed, tr!("Border"), None, |ui| {
                color_well(ui, tr!("Border color"), &mut style.border.color);
                card::switch(ui, &mut style.border.enabled);
            });
            if style.border.enabled {
                let max = crate::model::drawings::style::MAX_STROKE_WIDTH as f64;
                if let Some(width) =
                    card::card(ui, Icon::Sliders, tr!("Border thickness"), None, |ui| {
                        number_field(
                            ui,
                            Id::new("drawing-border-width"),
                            f64::from(style.border.width),
                            1.0,
                            1.0..=max,
                            0,
                        )
                    })
                {
                    style.border.width = width as f32;
                }
                dash_card(ui, tr!("Border style"), &mut style.border.dash);
            }
        }
        if caps.has(Caps::EMOJI) {
            page.section(ui, tr!("Emoji pin"), None);
            toggle_row(
                ui,
                Icon::SignpostTool,
                tr!("Emoji pin"),
                tr!("Show an emoji on the marker at the anchor."),
                &mut style.emoji_shown,
            );
            if style.emoji_shown {
                let names: Vec<String> = Emoji::ALL
                    .iter()
                    .map(|emoji| format!("{}  {}", emoji.glyph(), emoji.label()))
                    .collect();
                let options: Vec<(Emoji, &str)> = Emoji::ALL
                    .iter()
                    .zip(&names)
                    .map(|(emoji, name)| (*emoji, name.as_str()))
                    .collect();
                picker_row(
                    ui,
                    Icon::Sliders,
                    tr!("Emoji"),
                    None,
                    &options,
                    &mut style.emoji,
                );
                color_card(
                    ui,
                    Icon::Palette,
                    tr!("Emoji background"),
                    None,
                    &mut style.emoji_fill,
                );
            }
        }
        if caps.has(Caps::MIDDLE_LINE) {
            card::card(ui, Icon::Minus, tr!("Middle line"), None, |ui| {
                color_well(ui, tr!("Middle line color"), &mut style.middle_line.color);
                card::switch(ui, &mut style.middle_line.enabled);
            });
            if style.middle_line.enabled {
                dash_card(ui, tr!("Middle line style"), &mut style.middle_line.dash);
            }
        }
        if caps.has(Caps::EXTEND) {
            page.section(ui, tr!("Extend"), None);
            toggle_row(
                ui,
                Icon::ArrowLeftRight,
                tr!("Extend left"),
                tr!("Continue to the left edge of the chart."),
                &mut style.extend_left,
            );
            toggle_row(
                ui,
                Icon::ArrowLeftRight,
                tr!("Extend right"),
                tr!("Continue to the right edge of the chart."),
                &mut style.extend_right,
            );
        }
        if details {
            page.section(ui, tr!("Details"), None);
        }
        if caps.has(Caps::MIDDLE_POINT) {
            toggle_row(
                ui,
                Icon::Circle,
                tr!("Middle point"),
                tr!("A dot halfway along the line."),
                &mut style.middle_point,
            );
        }
        if caps.has(Caps::PRICE_LABELS) {
            // A line at a price is tagged on the price axis; the others print their end prices.
            let on_axis = matches!(
                self.tool,
                Tool::HorizontalLine | Tool::HorizontalRay | Tool::Crossline
            );
            toggle_row(
                ui,
                Icon::Tag,
                if on_axis {
                    tr!("Price label")
                } else {
                    tr!("Price labels")
                },
                if on_axis {
                    tr!("A tag with the price on the price axis.")
                } else {
                    tr!("The price at each end.")
                },
                &mut style.price_labels,
            );
        }
        if caps.has(Caps::TIME_LABEL) {
            toggle_row(
                ui,
                Icon::Clock,
                tr!("Time label"),
                tr!("A tag with the time on the time axis."),
                &mut style.time_label,
            );
        }
        if caps.has(Caps::STATS) {
            self.stats_section(ui, &mut page, &mut style);
        }
        if caps.has(Caps::FIB) {
            self.fib_style_cards(ui, &mut page, &mut style);
        }
        if style != before {
            self.set_style(style);
        }
    }

    fn stats_section(&self, ui: &mut Ui, page: &mut Page, style: &mut DrawingStyle) {
        // Always titled: its subtitle says what the switches below it print.
        let subtitle = if self.tool == Tool::Arrow {
            tr!("Measurements printed beside the arrow.")
        } else {
            tr!("Measurements printed beside the line.")
        };
        page.titled_section(ui, tr!("Statistics"), Some(subtitle));
        let stats = &mut style.stats;
        toggle_row(
            ui,
            Icon::Hash,
            tr!("Price range"),
            tr!("The price change between the ends."),
            &mut stats.price_range,
        );
        toggle_row(
            ui,
            Icon::Percent,
            tr!("Percent change"),
            tr!("The change as a percentage of the start."),
            &mut stats.percent,
        );
        toggle_row(
            ui,
            Icon::ChartLine,
            tr_c!("number of bars", "Bars"),
            tr!("How many bars the arrow spans."),
            &mut stats.bars,
        );
        toggle_row(
            ui,
            Icon::Clock,
            tr!("Time range"),
            tr!("How long the arrow spans."),
            &mut stats.time,
        );
        toggle_row(
            ui,
            Icon::ArrowLeftRight,
            tr!("Distance"),
            tr!("The signed number of bars from start to end."),
            &mut stats.distance,
        );
        toggle_row(
            ui,
            Icon::ArrowUpRight,
            tr!("Angle"),
            tr!("The arrow's angle as drawn on screen."),
            &mut stats.angle,
        );
        let options: Vec<(StatsPosition, &str)> = StatsPosition::ALL
            .iter()
            .map(|position| (*position, position.label()))
            .collect();
        picker_row(
            ui,
            Icon::Sliders,
            tr!("Position"),
            None,
            &options,
            &mut stats.position,
        );
        toggle_row(
            ui,
            Icon::Eye,
            tr!("Always show"),
            tr!("Off shows the statistics only while the arrow is selected."),
            &mut stats.always_show,
        );
    }

    /// Draws the pitchfork as another variant: the same three pivots, another construction.
    pub(super) fn switch_variant(&mut self, variant: PitchforkVariant) {
        if let Some(Geometry::Pitchfork {
            variant: current, ..
        }) = self.draft.geometry_mut()
        {
            *current = variant;
            self.tool = variant.tool();
        }
    }
}
