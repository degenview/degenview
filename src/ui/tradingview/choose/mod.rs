//! The first step: what to bring over, and which market each of its symbols becomes.

mod cards;
mod pictures;
mod symbols;

use super::{Action, Content, scrolling, words};
use crate::i18n::{tr, tr_n};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::loading::LoadingIndicator;
use crate::ui::kit::matching::Tally;
use crate::ui::kit::{self, sheet};
use crate::ui::theme;
use eframe::egui::{
    self, Align, CursorIcon, Id, Layout, Rect, Sense, Stroke, Ui, UiBuilder, Vec2, pos2, vec2,
};

pub(super) fn show(ui: &mut Ui, height: f32, content: &Content<'_>, actions: &mut Vec<Action>) {
    let listed = content.selection.symbols(content.library);
    let tally = symbols::tally(content, &listed);
    // Set by the footer's note, taken up here on the next frame.
    let jump = Id::new(("tradingview-to-matching", content.opening));
    let jumping = ui.data_mut(|data| data.remove_temp::<bool>(jump)).is_some();
    scrolling(ui, height, ("tradingview-choose", content.opening), |ui| {
        cards::show(ui, content, actions);
        ui.add_space(14.0);
        missing(ui, content, actions);
        if !listed.is_empty() {
            ui.add_space(24.0);
            let heading = Rect::from_min_size(ui.cursor().min, vec2(1.0, 1.0));
            symbols::show(ui, content, &listed, &tally, actions);
            if jumping {
                ui.scroll_to_rect(heading, Some(Align::TOP));
            }
        }
    });
    footer(ui, content, &tally, jump, actions);
}

/// Why something may be missing, with the ways to look again.
fn missing(ui: &mut Ui, content: &Content<'_>, actions: &mut Vec<Action>) {
    let text = tr!(
        "Only what the TradingView app has opened recently on this computer is here. Missing \
         something? Open it there, then reload."
    );
    let choose = tr!("Choose Folder\u{2026}");
    let reload = tr!("Reload");
    let panels = crate::platform::has_file_dialogs();
    egui::Frame::new()
        .stroke(Stroke::new(1.0, theme::card_border(false)))
        .corner_radius(kit::CARD_RADIUS)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            let width = ui.available_width();
            let mut buttons = kit::push_button_width(ui, None, reload);
            if panels {
                buttons += kit::push_button_width(ui, None, choose) + 8.0;
            }
            // The glyph, the words and the buttons share one middle, however the words wrap.
            let words = kit::paragraph(
                ui,
                text,
                kit::size::CAPTION + 0.5,
                kit::REGULAR,
                theme::text_secondary(),
                (width - 26.0 - buttons - 16.0).max(200.0),
            );
            let height = words.size().y.max(24.0);
            let (row, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
            icons::paint(
                ui.painter(),
                Rect::from_center_size(pos2(row.left() + 8.0, row.center().y), Vec2::splat(15.0)),
                Icon::InfoCircle,
                theme::text_secondary(),
            );
            ui.painter().galley(
                pos2(row.left() + 26.0, row.center().y - words.size().y / 2.0),
                words,
                theme::text_secondary(),
            );
            let end = Rect::from_min_max(pos2(row.right() - buttons, row.top()), row.max);
            let mut side = ui.new_child(
                UiBuilder::new()
                    .max_rect(end)
                    .layout(Layout::right_to_left(Align::Center)),
            );
            side.spacing_mut().item_spacing.x = 8.0;
            if panels && kit::push_button(&mut side, None, choose, true).clicked() {
                actions.push(Action::ChooseFolder);
            }
            if kit::push_button(&mut side, None, reload, true)
                .on_hover_text(content.folder.display().to_string())
                .clicked()
            {
                actions.push(Action::Reload);
            }
        });
}

/// What is chosen, and what still stands in the way, beside the two ways on.
fn footer(ui: &mut Ui, content: &Content<'_>, tally: &Tally, jump: Id, actions: &mut Vec<Action>) {
    let selection = content.selection;
    let parts: Vec<String> = [
        (
            selection.layouts.len(),
            words::layouts as fn(usize) -> String,
        ),
        (selection.watchlists.len(), words::watchlists),
        (selection.scripts.len(), words::scripts),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, words)| words(count))
    .collect();
    let summary = if parts.is_empty() {
        tr!("Nothing selected").to_owned()
    } else {
        parts.join(" \u{00B7} ")
    };
    sheet::footer_split(
        ui,
        |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(kit::text(
                    summary,
                    kit::size::BODY,
                    kit::SEMIBOLD,
                    theme::text_primary(),
                ));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    status(ui, tally, jump);
                });
            });
        },
        |ui| {
            let ready = !selection.is_empty() && tally.resolving == 0;
            if kit::neutral_button(ui, tr!("Continue to Preview"), true, ready, false, 150.0)
                .clicked()
            {
                actions.push(Action::Preview);
            }
            if kit::neutral_button(ui, tr!("Cancel"), false, true, false, 90.0).clicked() {
                actions.push(Action::Close);
            }
        },
    );
}

/// Where the symbols stand. Those without a market are a way down to them: they are below the
/// first screenful, and left out unless matched.
fn status(ui: &mut Ui, tally: &Tally, jump: Id) {
    let caption = |ui: &mut Ui, text: String| {
        ui.label(kit::text(
            text,
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            theme::text_secondary(),
        ));
    };
    let dot = |ui: &mut Ui, color: egui::Color32| {
        let (dot, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
        ui.painter().circle_filled(dot.center(), 3.5, color);
    };
    if tally.resolving > 0 {
        let (busy, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
        LoadingIndicator::new().size(12.0).paint_at(ui, busy);
        caption(ui, tr!("Searching exchanges\u{2026}").to_owned());
    } else if tally.unmatched > 0 {
        dot(ui, theme::REPLAY);
        let link = ui
            .add(
                egui::Label::new(
                    kit::text(
                        tr!("{count} to match", count = tally.unmatched),
                        kit::size::CAPTION + 0.5,
                        kit::REGULAR,
                        theme::text_secondary(),
                    )
                    .underline(),
                )
                .sense(Sense::click()),
            )
            .on_hover_cursor(CursorIcon::PointingHand);
        if link.clicked() {
            ui.data_mut(|data| data.insert_temp(jump, true));
            ui.ctx().request_repaint();
        }
    } else if tally.matched > 0 {
        dot(ui, theme::UP);
        caption(
            ui,
            tr_n!(tally.matched, "{count} matched", "{count} matched"),
        );
    }
}

/// A heading of the step: a bare glyph and a title, with what follows it on the same line left to
/// `add`.
fn heading(ui: &mut Ui, icon: Icon, title: &str, add: impl FnOnce(&Ui, Rect, f32)) {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(row.left() + 8.0, row.center().y), Vec2::splat(16.0)),
        icon,
        theme::text_secondary(),
    );
    let title = kit::galley(
        ui,
        title,
        kit::size::HEADLINE + 1.0,
        kit::SEMIBOLD,
        theme::text_primary(),
        row.width() - 24.0,
    );
    let end = row.left() + 24.0 + title.size().x;
    ui.painter().galley(
        pos2(row.left() + 24.0, row.center().y - title.size().y / 2.0),
        title,
        egui::Color32::PLACEHOLDER,
    );
    add(ui, row, end);
}
