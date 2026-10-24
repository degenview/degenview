//! The Fibonacci and Gann tools' pages of the properties dialog: the switches of their Style page and
//! the Levels page, where every list of levels is edited the same way (switch, ratio, colour, look,
//! reorder, remove) and a level is added or the tool's own set restored.
//!
//! Every control here is one the drawing honours: a tool shows exactly the controls its
//! [`Features`](crate::model::fib::Features) list.

use super::{
    DrawingProperties, Icon, Id, Page, Ui, color_well, number_field, picker_row, settings_card,
    toggle_row,
};
use crate::model::Tool;
use crate::model::chart_type::LineDash;
use crate::model::drawing_style::{DrawingStyle, HorizontalAlign, MAX_LEVEL_RATIO, VerticalAlign};
use crate::model::fib::Caption;
use crate::model::fib_style::{FibLevel, FibStyle, LevelSet};
use crate::model::gann;
use crate::ui::{icons, kit, theme};
use eframe::egui::{self, Align, Layout};

/// The widest a ratio field is: room for `-100.000`.
const RATIO_WIDTH: f32 = 52.0;
/// The font sizes a label may have.
const FONT_SIZES: std::ops::RangeInclusive<f64> = 8.0..=72.0;
/// The width a level's caption (`8x1`, `1.5x0`) is given.
const CAPTION_WIDTH: f32 = 38.0;

/// What a level's row asks for besides editing the level in place.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RowAction {
    Up,
    Down,
    Remove,
}

/// The tool's features, or none for a tool that is not a Fibonacci or Gann one.
fn features(tool: Tool) -> Option<crate::model::fib::Features> {
    crate::model::fib::features_of(tool)
}

impl DrawingProperties {
    /// The style's Fibonacci part, as the drawing is drawn: its own, or the tool's defaults.
    fn fib_style_of(&self, style: &DrawingStyle) -> FibStyle {
        style
            .fib
            .or_else(|| FibStyle::for_tool(self.tool))
            .unwrap_or_default()
    }

    /// The switches of the Style page that belong to a Fibonacci or Gann tool: its levels' colours
    /// and scale, its labels, and whatever else it alone has.
    pub(super) fn fib_style_cards(
        &mut self,
        ui: &mut Ui,
        page: &mut Page,
        style: &mut DrawingStyle,
    ) {
        let Some(features) = features(self.tool) else {
            return;
        };
        let mut fib = self.fib_style_of(style);
        page.section(ui, "Levels", None);
        toggle_row(
            ui,
            Icon::Palette,
            "Use one color",
            "Draw every level in the line color.",
            &mut style.use_one_color,
        );
        if features.trend_line {
            toggle_row(
                ui,
                Icon::LineTrend,
                "Trend line",
                "The line joining the anchors.",
                &mut fib.trend_line,
            );
        }
        if features.reverse {
            toggle_row(
                ui,
                Icon::ArrowUpRight,
                "Reverse",
                "Flip the drawing round its anchors.",
                &mut fib.reverse,
            );
        }
        if features.log_levels {
            toggle_row(
                ui,
                Icon::PriceScale,
                "Levels based on log scale",
                "On a logarithmic chart, space levels by the ratio of the prices.",
                &mut fib.log_levels,
            );
        }
        if features.full_circles {
            toggle_row(
                ui,
                Icon::Circle,
                "Full circles",
                "Draw each arc as a whole circle.",
                &mut fib.full_circles,
            );
        }
        if features.counterclockwise {
            toggle_row(
                ui,
                Icon::Repeat,
                "Counterclockwise",
                "Turn the other way round.",
                &mut fib.counterclockwise,
            );
        }
        if !features.second_fill.is_empty() {
            toggle_row(
                ui,
                Icon::RectFill,
                features.second_fill,
                "A second background, behind the other set of levels.",
                &mut fib.second_fill,
            );
        }
        if !features.grid.is_empty() {
            settings_card::card(ui, Icon::Hash, features.grid, None, |ui| {
                color_well(ui, "Grid color", &mut fib.grid.color);
                settings_card::switch(ui, &mut fib.grid.enabled);
            });
        }
        if features.scale {
            let max = f64::from(crate::model::fib_style::MAX_SCALE);
            if let Some(scale) = settings_card::card(
                ui,
                Icon::Ruler,
                "Price per bar",
                Some("Zero takes it from the two anchors."),
                |ui| {
                    number_field(
                        ui,
                        Id::new("fib-scale"),
                        f64::from(fib.scale),
                        0.1,
                        0.0..=max,
                        4,
                    )
                },
            ) {
                fib.scale = scale as f32;
            }
        }
        if features.readout {
            toggle_row(
                ui,
                Icon::InfoCircle,
                "Ranges and ratio",
                "Show the time range, the price range and their ratio.",
                &mut fib.readout,
            );
        }

        if features.labels {
            page.section(ui, "Labels", None);
            toggle_row(
                ui,
                Icon::Hash,
                "Levels",
                "Print each level's value.",
                &mut fib.show_labels,
            );
            if features.percent && fib.show_labels {
                toggle_row(
                    ui,
                    Icon::Percent,
                    "Percents",
                    "Print values as percentages.",
                    &mut fib.as_percent,
                );
            }
            if features.prices {
                toggle_row(
                    ui,
                    Icon::Banknote,
                    "Prices",
                    "Print each level's price.",
                    &mut fib.show_prices,
                );
            }
            if features.align {
                let horizontal: Vec<(HorizontalAlign, &str)> = HorizontalAlign::ALL
                    .iter()
                    .map(|align| (*align, align.label()))
                    .collect();
                picker_row(
                    ui,
                    Icon::ArrowLeftRight,
                    "Label position",
                    None,
                    &horizontal,
                    &mut fib.label_h,
                );
                let vertical: Vec<(VerticalAlign, &str)> = VerticalAlign::ALL
                    .iter()
                    .map(|align| (*align, align.label()))
                    .collect();
                picker_row(
                    ui,
                    Icon::ChevronUpDown,
                    "Label placement",
                    Some("Bottom puts the text on the line, top under it."),
                    &vertical,
                    &mut fib.label_v,
                );
            }
            if features.sides {
                for (title, value) in [
                    ("Left labels", &mut fib.sides.left),
                    ("Right labels", &mut fib.sides.right),
                    ("Top labels", &mut fib.sides.top),
                    ("Bottom labels", &mut fib.sides.bottom),
                ] {
                    toggle_row(ui, Icon::Tag, title, "", value);
                }
            }
            if let Some(size) = settings_card::card(ui, Icon::TextFormat, "Font size", None, |ui| {
                number_field(
                    ui,
                    Id::new("fib-font-size"),
                    f64::from(style.text.size),
                    1.0,
                    FONT_SIZES,
                    0,
                )
            }) {
                style.text.size = size as f32;
            }
        }
        style.fib = Some(fib);
    }

    /// The Levels page: each list of the tool, its levels one to a row.
    pub(super) fn fib_levels_page(&mut self, ui: &mut Ui) {
        let Some(features) = features(self.tool) else {
            return;
        };
        let mut style = self.style();
        let before = style;
        let mut fib = self.fib_style_of(&style);
        let mut page = Page::new(true);
        let lists = features.lists;
        let captions = features.captions;
        if !lists[0].is_empty() {
            let list = (lists[0], captions[0]);
            edit_list(ui, &mut page, 0, list, &mut fib.levels, false);
        }
        if !lists[1].is_empty() {
            let list = (lists[1], captions[1]);
            edit_list(ui, &mut page, 1, list, &mut fib.second, false);
        }
        if !lists[2].is_empty() {
            let list = (lists[2], captions[2]);
            edit_list(ui, &mut page, 2, list, &mut fib.third, features.corners);
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if kit::push_button(ui, Some(Icon::Undo), "Restore defaults", true).clicked() {
                let defaults = FibStyle::for_tool(self.tool).unwrap_or_default();
                fib.levels = defaults.levels;
                fib.second = defaults.second;
                fib.third = defaults.third;
            }
        });
        style.fib = Some(fib);
        if style != before {
            self.set_style(style);
        }
    }
}

/// One list of levels: a heading, a row for each level, and the button that adds another.
fn edit_list<const N: usize>(
    ui: &mut Ui,
    page: &mut Page,
    list: usize,
    (title, caption): (&str, Caption),
    levels: &mut LevelSet<N>,
    corners: bool,
) {
    page.titled_section(ui, title, None);
    let count = levels.len();
    let mut action: Option<(usize, RowAction)> = None;
    for (index, level) in levels.as_mut_slice().iter_mut().enumerate() {
        let row = Row {
            id: (list, index),
            position: (index, count),
            caption,
            corners,
        };
        if let Some(asked) = level_row(ui, row, level) {
            action = Some((index, asked));
        }
    }
    match action {
        Some((index, RowAction::Remove)) => levels.remove(index),
        Some((index, RowAction::Up)) => levels.shift(index, -1),
        Some((index, RowAction::Down)) => levels.shift(index, 1),
        None => {}
    }
    ui.horizontal(|ui| {
        let add = kit::push_button(ui, Some(Icon::Plus), "Add level", !levels.is_full());
        if add.clicked() && !levels.is_full() {
            // A new level continues the list: one step past the last ratio, in the last colour.
            let next = if levels.is_empty() {
                FibLevel::default()
            } else {
                let last = levels.as_slice()[levels.len() - 1];
                FibLevel {
                    ratio: (last.ratio + 1.0).min(MAX_LEVEL_RATIO),
                    ..last
                }
            };
            levels.push(next);
        }
    });
}

/// Where a level's row is and what it shows beside the ratio.
#[derive(Clone, Copy)]
struct Row {
    id: (usize, usize),
    /// Its place in its list, and the list's length.
    position: (usize, usize),
    caption: Caption,
    /// An arc: which corner it is about is a choice of its own.
    corners: bool,
}

/// One level on one row: shown or not, its ratio, its colour, its look and where it is in the list.
fn level_row(ui: &mut Ui, row: Row, level: &mut FibLevel) -> Option<RowAction> {
    let mut action = None;
    let id = Id::new(("fib-level", row.id.0, row.id.1));
    kit::card_frame(false)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                settings_card::switch(ui, &mut level.enabled);
                let name = match row.caption {
                    Caption::None => None,
                    Caption::Angle => Some(gann::angle_label(level.ratio)),
                    Caption::Arc => Some(gann::arc_label(level)),
                };
                if let Some(name) = name {
                    ui.add_sized(
                        [CAPTION_WIDTH, 22.0],
                        egui::Label::new(kit::text(
                            name,
                            11.5,
                            kit::SEMIBOLD,
                            theme::text_secondary(),
                        )),
                    );
                }
                let max = f64::from(MAX_LEVEL_RATIO);
                if let Some(ratio) = number_field_row(ui, id, level.ratio, max) {
                    level.ratio = ratio;
                }
                color_well(ui, "Level color", &mut level.color);
                if let Some(asked) = look_chip(ui, id, level, row.position) {
                    action = Some(asked);
                }
                if row.corners {
                    let label = if level.alt { "Far" } else { "Near" };
                    if kit::value_chip(ui, label, level.alt)
                        .on_hover_text("The corner the arc is centred on")
                        .clicked()
                    {
                        level.alt = !level.alt;
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let color = theme::text_secondary();
                    if icons::button(ui, Icon::Trash, 22.0, color, "Remove level").clicked() {
                        action = Some(RowAction::Remove);
                    }
                });
            });
        });
    action
}

fn number_field_row(ui: &mut Ui, id: Id, ratio: f32, max: f64) -> Option<f32> {
    crate::ui::chart_style_settings::number_field_sized(
        ui,
        id.with("ratio"),
        f64::from(ratio),
        0.05,
        -max..=max,
        3,
        RATIO_WIDTH,
    )
    .map(|value| value as f32)
}

/// The chip that shows a level's thickness and opens the menu where it, its line style and its
/// place in the list are set.
fn look_chip(
    ui: &mut Ui,
    id: Id,
    level: &mut FibLevel,
    (position, count): (usize, usize),
) -> Option<RowAction> {
    let dash = match level.dash {
        LineDash::Solid => "",
        LineDash::Dashed => " dashed",
        LineDash::Dotted => " dotted",
    };
    let mut action = None;
    let chip = kit::value_chip(ui, &format!("{}px{dash}", level.width), false);
    egui::Popup::menu(&chip)
        .frame(kit::menu_frame())
        .width(190.0)
        .show(|ui| {
            kit::menu_scope(ui, 190.0);
            kit::menu_heading(ui, "Thickness");
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                let max = f64::from(crate::model::drawing_style::MAX_STROKE_WIDTH);
                if let Some(width) = number_field(
                    ui,
                    id.with("width"),
                    f64::from(level.width),
                    1.0,
                    1.0..=max,
                    0,
                ) {
                    level.width = width as f32;
                }
            });
            kit::menu_separator(ui);
            kit::menu_heading(ui, "Line style");
            for dash in LineDash::ALL {
                if kit::menu_item_marked(ui, Icon::Check, dash.label(), level.dash == dash) {
                    level.dash = dash;
                }
            }
            kit::menu_separator(ui);
            if kit::menu_item(ui, Icon::ChevronUp, "Move up", position > 0, false) {
                action = Some(RowAction::Up);
                ui.close();
            }
            if kit::menu_item(
                ui,
                Icon::ChevronDown,
                "Move down",
                position + 1 < count,
                false,
            ) {
                action = Some(RowAction::Down);
                ui.close();
            }
        });
    action
}
