//! The row above the ladder: the size of a row, which trades stand out, and the way back to the
//! market after the ladder was scrolled away from it.

use super::state::BigTrade;
use super::{BookViewState, MARGIN};
use crate::i18n::tr;
use crate::model::depth::Ladder;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM, size};
use crate::ui::theme;
use eframe::egui::{Align, Layout, Ui, vec2};

const HEIGHT: f32 = 36.0;
const GROUP_WIDTH: f32 = 72.0;
const BIG_WIDTH: f32 = 92.0;

pub(super) fn show(ui: &mut Ui, state: &mut BookViewState, ladder: &Ladder) {
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        vec2(width, HEIGHT),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.add_space(MARGIN);
            ui.label(kit::text(
                tr!("Group"),
                size::CAPTION,
                MEDIUM,
                theme::text_secondary(),
            ));
            group_picker(ui, state, ladder);
            big_picker(ui, state);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(MARGIN - 8.0 - 4.0);
                if state.centre.is_some()
                    && icons::button(
                        ui,
                        Icon::Target,
                        26.0,
                        theme::text_secondary(),
                        tr!("Centre on the market"),
                    )
                    .clicked()
                {
                    state.centre = None;
                }
            });
        },
    );
}

/// The rows' size, picked from those the market's tick allows.
fn group_picker(ui: &mut Ui, state: &mut BookViewState, ladder: &Ladder) {
    let choices = &ladder.group_choices;
    let labels: Vec<String> = choices
        .iter()
        .map(|group| group.format(group.decimals()))
        .collect();
    let selected = choices
        .iter()
        .position(|group| *group == ladder.group)
        .unwrap_or(0);
    if let Some(chosen) = kit::menu_picker_fitted(ui, GROUP_WIDTH, &labels, selected, true) {
        state.group = Some(choices[chosen]);
    }
}

fn big_picker(ui: &mut Ui, state: &mut BookViewState) {
    let labels: Vec<String> = BigTrade::ALL
        .iter()
        .map(|big| big.label().to_owned())
        .collect();
    let selected = BigTrade::ALL
        .iter()
        .position(|big| *big == state.big)
        .unwrap_or(0);
    if let Some(chosen) = kit::menu_picker_fitted(ui, BIG_WIDTH, &labels, selected, true) {
        state.big = BigTrade::ALL[chosen];
    }
}
