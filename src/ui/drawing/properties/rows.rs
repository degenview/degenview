//! The pieces every page is built from: its headings, toggle rows, colour wells and cards, and the
//! hairline between sections.

use super::{CARD_SPACING, SECTION_GAP};
use crate::model::chart::kind::{LineDash, Rgba};
use crate::ui::chart::settings::style::picker_row;
use crate::ui::icons::Icon;
use crate::ui::{chart::series, kit, settings::card, theme};
use eframe::egui::{Color32, Sense, Ui, color_picker::Alpha, vec2};

/// The headings of one page. Every card of a page sits `CARD_SPACING` apart; a heading adds
/// `SECTION_GAP` above itself (except the first) and hands the card spacing back to the cards
/// below it, because `card::section_title` tightens it for its own subtitle.
pub(super) struct Page {
    titled: bool,
    started: bool,
}

impl Page {
    /// `titled` is whether [`Page::section`] prints anything.
    pub(super) fn new(titled: bool) -> Self {
        Self {
            titled,
            started: false,
        }
    }

    /// A heading, when the page has more than one group.
    pub(super) fn section(&mut self, ui: &mut Ui, title: &str, subtitle: Option<&str>) {
        if self.titled {
            self.titled_section(ui, title, subtitle);
        }
    }

    /// A heading whatever the page.
    pub(super) fn titled_section(&mut self, ui: &mut Ui, title: &str, subtitle: Option<&str>) {
        if self.started {
            ui.add_space(SECTION_GAP);
        }
        self.started = true;
        card::section_title(ui, title, subtitle);
        ui.spacing_mut().item_spacing.y = CARD_SPACING;
    }
}

/// A card with a switch; an empty `hint` is no hint at all, so the title centres on its badge.
pub(super) fn toggle_row(ui: &mut Ui, icon: Icon, title: &str, hint: &str, value: &mut bool) {
    let hint = Some(hint).filter(|hint| !hint.is_empty());
    card::card(ui, icon, title, hint, |ui| {
        card::switch(ui, value);
    });
}

pub(super) fn rule(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::separator());
}

fn paint_color(color: Rgba) -> Color32 {
    series::color(color)
}

pub(super) fn color_well(ui: &mut Ui, title: &str, color: &mut Rgba) {
    if let Some(picked) = kit::color_well(
        ui,
        vec2(44.0, 24.0),
        title,
        paint_color(*color),
        Alpha::OnlyBlend,
    ) {
        *color = picked.to_srgba_unmultiplied();
    }
}

pub(super) fn color_card(
    ui: &mut Ui,
    icon: Icon,
    title: &str,
    hint: Option<&str>,
    color: &mut Rgba,
) {
    card::card(ui, icon, title, hint, |ui| color_well(ui, title, color));
}

pub(super) fn dash_card(ui: &mut Ui, title: &str, dash: &mut LineDash) {
    let options: Vec<(LineDash, &str)> = LineDash::ALL
        .iter()
        .map(|option| (*option, option.label()))
        .collect();
    picker_row(ui, Icon::Minus, title, None, &options, dash);
}
