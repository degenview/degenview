//! The states around the two steps: reading, nothing to read, and done.

use super::{Action, words};
use crate::i18n::tr;
use crate::model::tradingview::plan::Summary;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::sheet_parts::problem_card;
use crate::ui::kit::{self, loading, sheet};
use crate::ui::theme;
use eframe::egui::{self, Align, Layout, Rect, Sense, Ui, Vec2, vec2};

pub(super) fn reading(ui: &mut Ui, height: f32, actions: &mut Vec<Action>) {
    let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    loading::message(ui, area, tr!("Reading TradingView\u{2019}s data\u{2026}"));
    sheet::footer(ui, |ui| {
        if kit::footer_button(ui, tr!("Cancel"), kit::FooterButton::Secondary, true, 0.0).clicked()
        {
            actions.push(Action::Close);
        }
    });
}

/// A glyph, a title and a line of explanation in the middle of `height`.
fn centred(ui: &mut Ui, height: f32, icon: Icon, title: &str, add: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), height),
        Layout::top_down(Align::Center),
        |ui| {
            ui.set_min_height(height);
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add_space((height * 0.22).max(16.0));
            let (badge, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
            icons::paint(
                ui.painter(),
                Rect::from_center_size(badge.center(), Vec2::splat(30.0)),
                icon,
                theme::text_secondary(),
            );
            ui.add_space(12.0);
            ui.label(kit::text(
                title,
                kit::size::TITLE2,
                kit::SEMIBOLD,
                theme::text_primary(),
            ));
            ui.add_space(8.0);
            ui.allocate_ui_with_layout(vec2(460.0, 0.0), Layout::top_down(Align::Center), add);
        },
    );
}

fn explanation(ui: &mut Ui, text: &str) {
    ui.add(
        egui::Label::new(kit::text(
            text,
            kit::size::SUBHEADLINE + 1.0,
            kit::REGULAR,
            theme::text_secondary(),
        ))
        .wrap()
        .halign(Align::Center),
    );
}

pub(super) fn failed(ui: &mut Ui, height: f32, message: &str, actions: &mut Vec<Action>) {
    centred(
        ui,
        height,
        Icon::TriangleFill,
        tr!("Nothing to Import Yet"),
        |ui| {
            explanation(ui, message);
            ui.add_space(8.0);
            explanation(
                ui,
                tr!(
                    "Install the TradingView desktop app, sign in, and open the layouts and watchlists \
                 you want to bring over."
                ),
            );
        },
    );
    let mut choose = false;
    sheet::footer_split(
        ui,
        |ui| {
            choose = kit::footer_button(
                ui,
                tr!("Choose Folder\u{2026}"),
                kit::FooterButton::Secondary,
                true,
                0.0,
            )
            .clicked();
        },
        |ui| {
            if kit::footer_button(
                ui,
                tr!("Try Again"),
                kit::FooterButton::Primary,
                true,
                110.0,
            )
            .clicked()
            {
                actions.push(Action::Reload);
            }
            if kit::footer_button(ui, tr!("Cancel"), kit::FooterButton::Secondary, true, 0.0)
                .clicked()
            {
                actions.push(Action::Close);
            }
        },
    );
    if choose {
        actions.push(Action::ChooseFolder);
    }
}

/// What was added, in words.
pub(super) fn added(summary: &Summary) -> Vec<String> {
    [
        (summary.layouts.len(), words::layouts as fn(usize) -> String),
        (summary.drawings, words::drawings),
        (summary.watchlists, words::watchlists),
        (summary.scripts, words::scripts),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, words)| words(count))
    .collect()
}

pub(super) fn done(ui: &mut Ui, height: f32, summary: &Summary, actions: &mut Vec<Action>) {
    let added = added(summary);
    let (icon, title) = if added.is_empty() {
        (Icon::TriangleFill, tr!("Nothing Was Added"))
    } else {
        (Icon::Check, tr!("Added to DegenView"))
    };
    centred(ui, height, icon, title, |ui| {
        if !added.is_empty() {
            explanation(ui, &added.join(" \u{00B7} "));
            if !summary.layouts.is_empty() {
                ui.add_space(8.0);
                explanation(
                    ui,
                    tr!("Imported layouts are in the toolbar\u{2019}s layout menu."),
                );
            }
        }
        if !summary.failures.is_empty() {
            ui.add_space(16.0);
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
    });
    let first = summary.layouts.first().map(|(id, _)| *id);
    sheet::footer(ui, |ui| {
        if let Some(id) = first
            && kit::footer_button(
                ui,
                tr!("Open Layout"),
                kit::FooterButton::Primary,
                true,
                120.0,
            )
            .clicked()
        {
            actions.push(Action::OpenLayout(id));
        }
        let (label, style) = match first {
            Some(_) => (tr!("Close"), kit::FooterButton::Secondary),
            None => (tr!("Done"), kit::FooterButton::Primary),
        };
        if kit::footer_button(ui, label, style, true, 0.0).clicked() {
            actions.push(Action::Close);
        }
    });
}
