//! The sheet an available update opens: what is new and fixed between this build and the release on
//! offer, then Update or Cancel. Installing starts here and nowhere else. A build that cannot
//! install the release itself gets the same list and a way to its page instead.
//!
//! Both the tab strip's mark and Settings ▸ About open it (`UpdateHost::review`) and call
//! `show_review` every frame; only the window the host names draws it.

use crate::build_info::VERSION;
use crate::host::update::{Phase, UpdateHost, UpdateView};
use crate::model::changelog::ReleaseNotes;
use crate::ui::kit::{self, REGULAR, SEMIBOLD, sheet, size};
use crate::ui::theme;
use eframe::egui::{self, Id, Key, Ui};

mod art;
#[cfg(test)]
mod tests;

const WIDTH: f32 = 480.0;
/// The notes scroll beyond this share of the window's height, and never grow past `MAX_LIST`.
const LIST_SHARE: f32 = 0.55;
const MAX_LIST: f32 = 360.0;
const MIN_LIST: f32 = 120.0;
/// Between the list and the link and note under it, and between the two lists.
const GROUP_GAP: f32 = 16.0;
/// The picture's card starts this far left of its box so it lines up with the list under it.
const PICTURE_LEAD: f32 = 16.0;
const NO_NOTES: &str = "No summary of changes is available for this version.";

/// How the person answered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Update,
    ViewRelease,
    Cancel,
}

/// Draws the sheet when `host` names this window for it, and carries out the answer: Update
/// installs, View Release opens the page, either one and Cancel close the sheet.
pub fn show_review(ctx: &egui::Context, host: &UpdateHost, view: &UpdateView) {
    if !host.review_open_in(ctx.viewport_id()) {
        return;
    }
    let Some(outcome) = show(ctx, view, VERSION) else {
        return;
    };
    match outcome {
        Outcome::Update => host.install(ctx),
        Outcome::ViewRelease => {
            if let Phase::Available { url, .. } = &view.phase {
                ctx.open_url(egui::OpenUrl::new_tab(url));
            }
        }
        Outcome::Cancel => {}
    }
    host.close_review();
}

/// The sheet for the update `view` offers, over a build at `installed`; none when no update is on
/// offer. The answer comes back once: a click on a button, Return for the primary one, Escape or a
/// click outside for Cancel.
pub fn show(ctx: &egui::Context, view: &UpdateView, installed: &str) -> Option<Outcome> {
    let Phase::Available {
        version,
        url,
        installable,
        note,
        notes,
    } = &view.phase
    else {
        return None;
    };
    let mut outcome = None;
    let list_height = (ctx.content_rect().height() * LIST_SHARE).clamp(MIN_LIST, MAX_LIST);
    let primary = if *installable {
        Outcome::Update
    } else {
        Outcome::ViewRelease
    };
    let ((), outside) = sheet::show_dismissable(ctx, Id::new("update-review"), WIDTH, |ui| {
        ui.add_space(24.0);
        sheet::inset(ui, |ui| {
            sheet::header_picture(
                ui,
                art::SIZE,
                PICTURE_LEAD,
                &format!("Update to DegenView {version}"),
                &format!("You have {installed}."),
                art::paint,
            );
        });
        ui.add_space(20.0);
        notes_list(ui, notes, list_height);
        ui.add_space(GROUP_GAP);
        sheet::inset(ui, |ui| {
            if !installable && let Some(note) = note {
                ui.add(
                    egui::Label::new(kit::text(
                        note,
                        size::SUBHEADLINE + 1.0,
                        REGULAR,
                        theme::text_secondary(),
                    ))
                    .wrap(),
                );
                ui.add_space(10.0);
            }
            release_notes_link(ui, url);
            ui.add_space(22.0);
        });
        sheet::footer(ui, |ui| {
            let label = if *installable {
                "Update"
            } else {
                "View Release"
            };
            if kit::sheet_button(ui, label, true, true).clicked() {
                outcome = Some(primary);
            }
            if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                outcome = Some(Outcome::Cancel);
            }
        });
    });
    let (enter, escape) = ctx.input(|input| {
        (
            input.key_pressed(Key::Enter),
            input.key_pressed(Key::Escape),
        )
    });
    if outcome.is_none() {
        if enter {
            outcome = Some(primary);
        } else if escape || outside {
            outcome = Some(Outcome::Cancel);
        }
    }
    outcome
}

/// The changes in a list that scrolls past `height`. While there is more above or below the
/// visible part its edge fades into the sheet and the bar stays in view, so a list never looks
/// finished where it is only cut.
fn notes_list(ui: &mut Ui, notes: &ReleaseNotes, height: f32) {
    let output = ui
        .scope(|ui| {
            kit::scroll::keep_bars_visible(ui);
            egui::ScrollArea::vertical()
                .id_salt("update-review-notes")
                .max_height(height)
                .auto_shrink([false, true])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                .show(ui, |ui| sheet::inset(ui, |ui| changes(ui, notes)))
        })
        .inner;
    let beyond = kit::scroll::Beyond::of(
        output.state.offset.y,
        output.content_size.y,
        output.inner_rect.height(),
    );
    kit::scroll::fade_edges(ui.painter(), output.inner_rect, theme::chrome(), beyond);
}

/// New, then Fixed, each only when it has entries, then how many more there were; a calm line
/// when there are none.
fn changes(ui: &mut Ui, notes: &ReleaseNotes) {
    ui.spacing_mut().item_spacing.y = 0.0;
    if notes.is_empty() {
        ui.add(
            egui::Label::new(kit::text(
                NO_NOTES,
                size::BODY,
                REGULAR,
                theme::text_secondary(),
            ))
            .wrap(),
        );
        return;
    }
    let groups = [("New", &notes.new), ("Fixed", &notes.fixed)];
    for (index, (title, entries)) in groups
        .into_iter()
        .filter(|(_, entries)| !entries.is_empty())
        .enumerate()
    {
        if index > 0 {
            ui.add_space(GROUP_GAP);
        }
        ui.label(kit::text(
            title,
            size::SUBHEADLINE + 1.0,
            SEMIBOLD,
            theme::text_secondary(),
        ));
        ui.add_space(8.0);
        for (at, entry) in entries.iter().enumerate() {
            if at > 0 {
                ui.add_space(7.0);
            }
            bullet(ui, entry);
        }
    }
    if notes.omitted > 0 {
        ui.add_space(GROUP_GAP);
        ui.label(kit::text(
            omitted_line(notes.omitted),
            size::SUBHEADLINE + 1.0,
            REGULAR,
            theme::text_secondary(),
        ));
    }
}

/// What ends a list that left `count` changes out.
fn omitted_line(count: usize) -> String {
    let changes = if count == 1 { "change" } else { "changes" };
    format!("And {count} more {changes} in the full release notes.")
}

/// One change: a bullet, and its text wrapped to the width left beside it so a long line hangs
/// under its first word.
fn bullet(ui: &mut Ui, text: &str) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 9.0;
        ui.label(kit::text(
            "\u{2022}",
            size::BODY,
            REGULAR,
            theme::text_secondary(),
        ));
        ui.add(
            egui::Label::new(kit::text(text, size::BODY, REGULAR, theme::text_primary())).wrap(),
        );
    });
}

/// "Full release notes", gray and underlined like every link in the app. The sheet stays open.
fn release_notes_link(ui: &mut Ui, url: &str) {
    let response = ui
        .add(
            egui::Label::new(
                kit::text(
                    "Full release notes",
                    size::SUBHEADLINE + 1.0,
                    REGULAR,
                    theme::text_secondary(),
                )
                .underline(),
            )
            .sense(egui::Sense::click()),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(url);
    if response.clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
    }
}
