//! The states around the two steps: waiting on work, and nothing to read.

use super::{Action, Work};
use crate::i18n::tr;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, loading, sheet};
use crate::ui::theme;
use eframe::egui::{self, Align, Layout, Rect, Sense, Ui, Vec2, vec2};

pub(super) fn working(ui: &mut Ui, height: f32, work: Work, actions: &mut Vec<Action>) {
    let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let (message, way_out) = match work {
        Work::Reading => (
            tr!("Reading TradingView\u{2019}s data\u{2026}"),
            Some((tr!("Cancel"), Action::Close)),
        ),
        Work::Preparing => (
            tr!("Preparing the preview\u{2026}"),
            Some((tr!("Back"), Action::Back)),
        ),
        Work::Importing => (tr!("Importing\u{2026}"), None),
    };
    loading::message(ui, area, message);
    sheet::footer(ui, |ui| {
        if let Some((label, action)) = way_out
            && kit::footer_button(ui, label, kit::FooterButton::Secondary, true, 0.0).clicked()
        {
            actions.push(action);
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

pub(super) fn failed(
    ui: &mut Ui,
    height: f32,
    message: &str,
    hint: &str,
    actions: &mut Vec<Action>,
) {
    centred(
        ui,
        height,
        Icon::TriangleFill,
        tr!("Nothing to Import Yet"),
        |ui| {
            explanation(ui, message);
            ui.add_space(8.0);
            explanation(ui, hint);
        },
    );
    let mut choose = false;
    sheet::footer_split(
        ui,
        |ui| {
            if !crate::platform::has_file_dialogs() {
                return;
            }
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
