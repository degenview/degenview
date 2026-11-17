//! The Levels and Inputs pages: a pitchfork's or channel's extra lines, a tool's inputs and a
//! pattern's, with the rows a level is edited in.

use super::DrawingProperties;
use super::rows::{Page, color_well, dash_card, toggle_row};
use crate::i18n::{tr, tr_n};
use crate::model::drawings::style::{Level, MiddleLine};
use crate::model::{Caps, Geometry};
use crate::ui::chart::settings::style::{number_field, picker_row};
use crate::ui::icons::Icon;
use crate::ui::settings::card;
use eframe::egui::{Id, Ui};

impl DrawingProperties {
    /// The extra lines of a pitchfork (each on both sides of the median) or of a parallel channel.
    pub(super) fn levels_page(&mut self, ui: &mut Ui) {
        if self.tool.caps().has(Caps::FIB) {
            return self.fib_levels_page(ui);
        }
        let mut style = self.style();
        let before = style;
        let fork = matches!(self.draft.geometry(), Some(Geometry::Pitchfork { .. }));
        let mut page = Page::new(true);
        if fork {
            page.titled_section(
                ui,
                tr!("Colors"),
                Some(tr!("Each level is drawn on both sides of the median.")),
            );
            toggle_row(
                ui,
                Icon::Palette,
                tr!("Use one color"),
                tr!("Draw every line in the median's color."),
                &mut style.use_one_color,
            );
            for (index, level) in style.pitchfork_levels.iter_mut().enumerate() {
                level_cards(ui, &mut page, ("fork", index), level);
            }
        } else {
            for (index, level) in style.channel_levels.iter_mut().enumerate() {
                level_cards(ui, &mut page, ("channel", index), level);
            }
        }
        if style != before {
            self.set_style(style);
        }
    }

    /// The regression trend's source, the deviations its bands sit at, and its three lines.
    pub(super) fn inputs_page(&mut self, ui: &mut Ui) {
        use crate::model::chart::kind::PriceSource;
        let caps = self.tool.caps();
        if caps.has(Caps::PATTERN) || caps.has(Caps::WAVE) {
            return self.pattern_inputs_page(ui);
        }
        if !caps.has(Caps::REGRESSION) {
            return self.forecast_inputs_page(ui);
        }
        let mut style = self.style();
        let before = style;
        let mut page = Page::new(true);
        page.section(ui, tr!("Inputs"), None);
        let options: Vec<(PriceSource, &str)> = PriceSource::ALL
            .iter()
            .map(|source| (*source, source.label()))
            .collect();
        picker_row(
            ui,
            Icon::ChartLine,
            tr!("Source"),
            Some(tr!("The price the line is fitted to.")),
            &options,
            &mut style.regression.source,
        );
        let max = f64::from(crate::model::drawings::style::MAX_DEVIATION);
        let regression = &mut style.regression;
        for (title, hint, id, value) in [
            (
                tr!("Upper deviation"),
                tr!("Standard deviations above the line."),
                "regression-upper",
                &mut regression.upper_dev,
            ),
            (
                tr!("Lower deviation"),
                tr!("Standard deviations from the line, negative for below."),
                "regression-lower",
                &mut regression.lower_dev,
            ),
        ] {
            if let Some(picked) = card::card(ui, Icon::Hash, title, Some(hint), |ui| {
                number_field(ui, Id::new(id), f64::from(*value), 0.1, -max..=max, 2)
            }) {
                *value = picked as f32;
            }
        }
        toggle_row(
            ui,
            Icon::Percent,
            tr!("Pearson's R"),
            tr!("Print how closely the price follows the line (-1 to 1)."),
            &mut regression.pearson,
        );
        page.titled_section(ui, tr!("Base line"), None);
        line_cards(ui, "regression-base", &mut regression.base);
        page.titled_section(ui, tr!("Upper line"), None);
        line_cards(ui, "regression-upper-line", &mut regression.upper);
        page.titled_section(ui, tr!("Lower line"), None);
        line_cards(ui, "regression-lower-line", &mut regression.lower);
        if style != before {
            self.set_style(style);
        }
    }

    /// What a chart pattern or an Elliott wave prints on itself: the names of its pivots, the
    /// ratios between a pattern's legs and how many places they have, a wave's degree.
    fn pattern_inputs_page(&mut self, ui: &mut Ui) {
        use crate::model::drawings::style::MAX_RATIO_PRECISION;
        use crate::model::pattern::elliott::Degree;
        let caps = self.tool.caps();
        let mut style = self.style();
        let before = style;
        let mut page = Page::new(true);
        if caps.has(Caps::WAVE) {
            page.section(ui, tr!("Wave"), None);
            let options: Vec<(Degree, String)> = Degree::ALL
                .iter()
                .map(|degree| (*degree, format!("{}  {}", degree.label(), degree.sample())))
                .collect();
            let shown: Vec<(Degree, &str)> = options
                .iter()
                .map(|(degree, label)| (*degree, label.as_str()))
                .collect();
            picker_row(
                ui,
                Icon::Hash,
                tr!("Degree"),
                Some(tr!(
                    "The scale of the move: it sets how the waves are written."
                )),
                &shown,
                &mut style.pattern.degree,
            );
            toggle_row(
                ui,
                Icon::TextFormat,
                tr!("Labels"),
                tr!("Print the name of each wave at its end."),
                &mut style.pattern.labels,
            );
        } else {
            page.section(ui, tr!("Labels"), None);
            toggle_row(
                ui,
                Icon::TextFormat,
                tr!("Pivot names"),
                tr!("Print the name of each pivot beside it."),
                &mut style.pattern.labels,
            );
            toggle_row(
                ui,
                Icon::Percent,
                tr!("Ratios"),
                tr!(
                    "Print the Fibonacci ratio of each leg on the dashed line it is measured along."
                ),
                &mut style.pattern.ratios,
            );
            if style.pattern.ratios {
                let names: Vec<(u8, String)> = (0..=MAX_RATIO_PRECISION)
                    .map(|places| (places, tr_n!(places, "{count} place", "{count} places")))
                    .collect();
                let places: Vec<(u8, &str)> = names
                    .iter()
                    .map(|(places, name)| (*places, name.as_str()))
                    .collect();
                picker_row(
                    ui,
                    Icon::Hash,
                    tr!("Precision"),
                    Some(tr!("Decimal places of a ratio.")),
                    &places,
                    &mut style.pattern.precision,
                );
            }
        }
        if style != before {
            self.set_style(style);
        }
    }
}

/// A ratio as a short number: `0.382`, `1`, `-0.25`.
fn ratio_text(ratio: f32) -> String {
    let text = format!("{ratio:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// The cards of one level: whether and in what colour it is drawn, and, once it is, where it
/// sits and how it looks.
fn level_cards(ui: &mut Ui, page: &mut Page, id: (&'static str, usize), level: &mut Level) {
    page.titled_section(
        ui,
        &tr!("Level {ratio}", ratio = ratio_text(level.ratio)),
        None,
    );
    line_cards_with_ratio(ui, id, level);
}

fn line_cards_with_ratio(ui: &mut Ui, id: (&'static str, usize), level: &mut Level) {
    card::card(ui, Icon::Eye, tr!("Show"), None, |ui| {
        color_well(ui, tr!("Level color"), &mut level.line.color);
        card::switch(ui, &mut level.line.enabled);
    });
    if !level.line.enabled {
        return;
    }
    let max = f64::from(crate::model::drawings::style::MAX_LEVEL_RATIO);
    if let Some(ratio) = card::card(ui, Icon::Hash, tr!("Ratio"), None, |ui| {
        number_field(
            ui,
            Id::new(("drawing-level-ratio", id)),
            f64::from(level.ratio),
            0.05,
            -max..=max,
            3,
        )
    }) {
        level.ratio = ratio as f32;
    }
    thickness_and_style(ui, Id::new(("drawing-level-width", id)), &mut level.line);
}

/// The cards of a line with its own look: shown or not with its colour, then thickness and style.
fn line_cards(ui: &mut Ui, id: &'static str, line: &mut MiddleLine) {
    card::card(ui, Icon::Eye, tr!("Show"), None, |ui| {
        color_well(ui, tr!("Line color"), &mut line.color);
        card::switch(ui, &mut line.enabled);
    });
    if line.enabled {
        thickness_and_style(ui, Id::new(("drawing-line-width", id)), line);
    }
}

fn thickness_and_style(ui: &mut Ui, id: Id, line: &mut MiddleLine) {
    let max = f64::from(crate::model::drawings::style::MAX_STROKE_WIDTH);
    if let Some(width) = card::card(ui, Icon::Sliders, tr!("Thickness"), None, |ui| {
        number_field(ui, id, f64::from(line.width), 1.0, 1.0..=max, 0)
    }) {
        line.width = width as f32;
    }
    dash_card(ui, tr!("Line style"), &mut line.dash);
}
