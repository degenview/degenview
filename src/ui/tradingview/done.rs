//! The last step: what came over, by name, and where each is found now.

use super::welcome::art;
use super::{Action, scrolling, section_title, words};
use crate::i18n::tr;
use crate::model::tradingview::plan::Summary;
use crate::ui::icons::Icon;
use crate::ui::kit::sheet_parts::{card_background, problem_card};
use crate::ui::kit::{self, sheet};
use crate::ui::theme;
use eframe::egui::{self, Align, Color32, Id, Layout, Sense, Ui, UiBuilder, pos2, vec2};
use std::time::Duration;

const ROW_HEIGHT: f32 = 44.0;
const ROW_GAP: f32 = 6.0;
const FRAME: Duration = Duration::from_millis(16);

/// What was added, in words.
fn added(summary: &Summary) -> Vec<String> {
    [
        (summary.layouts.len(), words::layouts as fn(usize) -> String),
        (summary.drawings, words::drawings),
        (summary.watchlists.len(), words::watchlists),
        (summary.scripts.len(), words::scripts),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, words)| words(count))
    .collect()
}

/// How long the sheet has been on screen, so its picture plays once from the start.
fn since_shown(ui: &Ui) -> f64 {
    let id = Id::new("tradingview-done-since");
    let (now, frame) = (ui.input(|input| input.time), ui.ctx().cumulative_frame_nr());
    let began = ui.data_mut(|data| {
        let seen: Option<(u64, f64)> = data.get_temp(id);
        // Not drawn on the frame before: a new arrival.
        let began = seen
            .filter(|(last, _)| frame <= last + 1)
            .map_or(now, |(_, began)| began);
        data.insert_temp(id, (frame, began));
        began
    });
    now - began
}

pub(super) fn show(ui: &mut Ui, height: f32, summary: &Summary, actions: &mut Vec<Action>) {
    let added = added(summary);
    let whole = !added.is_empty();
    scrolling(ui, height, ("tradingview-done", 0), |ui| {
        let since = since_shown(ui);
        let (picture, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), art::HEIGHT), Sense::hover());
        art::arrived(ui, picture, since, whole);
        if whole && since < art::ARRIVAL {
            ui.ctx().request_repaint_after(FRAME);
        }
        ui.add_space(18.0);
        let title = if whole {
            tr!("Added to DegenView")
        } else {
            tr!("Nothing Was Added")
        };
        centred(
            ui,
            title,
            kit::size::TITLE2 + 4.0,
            kit::SEMIBOLD,
            theme::text_primary(),
        );
        if whole {
            ui.add_space(6.0);
            centred(
                ui,
                &added.join(" \u{00B7} "),
                kit::size::BODY,
                kit::REGULAR,
                theme::text_secondary(),
            );
        }
        if !summary.failures.is_empty() {
            ui.add_space(18.0);
            problem_card(
                ui,
                Icon::TriangleFill,
                theme::DOWN,
                tr!("Some things couldn\u{2019}t be added"),
                None,
                &summary.failures,
                96.0,
            );
        }
        if !summary.layouts.is_empty() {
            heading(
                ui,
                Icon::BarChart,
                tr!("Layouts"),
                tr!("In the toolbar\u{2019}s layout menu"),
            );
            for (id, name) in &summary.layouts {
                if row(ui, name, true) {
                    actions.push(Action::OpenLayout(*id));
                }
                ui.add_space(ROW_GAP);
            }
        }
        for (icon, title, place, names) in [
            (
                Icon::List,
                tr!("Watchlists"),
                tr!("In the watchlist sidebar"),
                &summary.watchlists,
            ),
            (
                Icon::Code,
                tr!("Pine scripts"),
                tr!("In Pine Studio"),
                &summary.scripts,
            ),
        ] {
            if names.is_empty() {
                continue;
            }
            heading(ui, icon, title, place);
            for name in names {
                row(ui, name, false);
                ui.add_space(ROW_GAP);
            }
        }
    });
    footer(ui, summary, actions);
}

fn centred(ui: &mut Ui, text: &str, size: f32, weight: f32, color: Color32) {
    ui.vertical_centered(|ui| {
        ui.add(
            egui::Label::new(kit::text(text, size, weight, color))
                .wrap()
                .halign(Align::Center),
        );
    });
}

/// A kind's title, and at the trailing edge where its things are now.
fn heading(ui: &mut Ui, icon: Icon, title: &str, place: &str) {
    ui.add_space(20.0);
    section_title(ui, icon, title, |ui, line, end| {
        let place = kit::galley(
            ui,
            place,
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            theme::text_tertiary(),
            (line.right() - end - 16.0).max(40.0),
        );
        ui.painter().galley(
            pos2(
                line.right() - place.size().x,
                line.center().y - place.size().y / 2.0,
            ),
            place,
            Color32::PLACEHOLDER,
        );
    });
    ui.add_space(10.0);
}

/// One added thing; `true` when its Open button was pressed.
fn row(ui: &mut Ui, name: &str, opens: bool) -> bool {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    card_background(ui, rect, None);
    let mut side = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(12.0, 0.0)))
            .layout(Layout::right_to_left(Align::Center)),
    );
    let clicked = opens && kit::push_button(&mut side, None, tr!("Open"), true).clicked();
    let room = side.available_width() - 12.0;
    let name = kit::galley(
        ui,
        name,
        kit::size::BODY,
        kit::MEDIUM,
        theme::text_primary(),
        room,
    );
    ui.painter().galley(
        pos2(rect.left() + 14.0, rect.center().y - name.size().y / 2.0),
        name,
        Color32::PLACEHOLDER,
    );
    clicked
}

fn footer(ui: &mut Ui, summary: &Summary, actions: &mut Vec<Action>) {
    let first = summary.layouts.first().map(|(id, _)| *id);
    sheet::footer(ui, |ui| {
        if let Some(id) = first
            && kit::neutral_button(ui, tr!("Open Layout"), true, true, false, 120.0).clicked()
        {
            actions.push(Action::OpenLayout(id));
        }
        let label = if first.is_some() {
            tr!("Close")
        } else {
            tr!("Done")
        };
        if kit::neutral_button(ui, label, first.is_none(), true, false, 90.0).clicked() {
            actions.push(Action::Close);
        }
    });
}
