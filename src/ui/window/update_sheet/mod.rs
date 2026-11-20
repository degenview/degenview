//! The sheet an available update opens: what is new and fixed between this build and the release on
//! offer, then Update or Cancel. Installing starts here and nowhere else. A build that cannot
//! install the release itself gets the same list and a way to its page instead.
//!
//! Both the tab strip's mark and Settings ▸ About open it (`UpdateHost::review`) and call
//! `show_review` every frame; only the window the host names draws it.

use crate::build_info::VERSION;
use crate::host::update::{Phase, UpdateHost, UpdateView};
use crate::model::changelog::{Coverage, ReleaseNotes};
use crate::ui::kit::{self, REGULAR, SEMIBOLD, sheet, size};
use crate::ui::theme;
use eframe::egui::{self, Id, Key, Ui};

mod art;
#[cfg(test)]
mod tests;

const WIDTH: f32 = 480.0;
/// The notes scroll beyond what the window leaves them, and never grow past `MAX_LIST`.
const MAX_LIST: f32 = 360.0;
/// The least the notes keep when the window is too short for everything else, so a list is never
/// squeezed to nothing. The smallest windows the app opens still hold the whole sheet at this.
const MIN_LIST: f32 = 96.0;
/// Clear space kept between the sheet and the top and bottom of its window.
const EDGE_MARGIN: f32 = 16.0;
/// Between the list and the link and note under it, and between the two lists.
const GROUP_GAP: f32 = 16.0;
/// The picture's card starts this far left of its box so it lines up with the list under it.
const PICTURE_LEAD: f32 = 16.0;
/// Above the header picture.
const TOP_SPACE: f32 = 24.0;
/// Between the header and the list.
const HEADER_GAP: f32 = 20.0;
/// Between the note about a manual download and the link under it.
const NOTE_GAP: f32 = 10.0;
/// Under the link, before the footer.
const BOTTOM_SPACE: f32 = 22.0;
/// Between a list's title and its first change.
const TITLE_GAP: f32 = 8.0;
/// Between two changes of a list.
const ENTRY_GAP: f32 = 7.0;
/// Between a bullet and its text.
const BULLET_GAP: f32 = 9.0;
/// How far the sheet's height without its list may drift between frames before the next one is laid
/// out again.
const CHROME_TOLERANCE: f32 = 0.5;
/// The size of the notes' titles, the footnotes, the note and the link under the list.
const DETAIL_TEXT: f32 = size::SUBHEADLINE + 1.0;
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
///
/// Update installs the release the sheet showed, not whichever one is on offer by the time it is
/// pressed: a check that replaced the offer in between leaves the sheet open on the new one. A new
/// offer drawn for the first time in the frame of a press is not answered either: the person has
/// not seen it yet (`UpdateHost::note_shown`). Cancel always counts.
pub fn show_review(ctx: &egui::Context, host: &UpdateHost, view: &UpdateView) {
    if !host.review_open_in(ctx.viewport_id()) {
        return;
    }
    let Phase::Available { version, url, .. } = &view.phase else {
        return;
    };
    let seen = host.note_shown(version);
    let Some(outcome) = show(ctx, view, VERSION) else {
        return;
    };
    if !seen && outcome != Outcome::Cancel {
        return;
    }
    match outcome {
        Outcome::Update => {
            if !host.install(ctx, version) {
                return;
            }
        }
        Outcome::ViewRelease => ctx.open_url(egui::OpenUrl::new_tab(url)),
        Outcome::Cancel => {}
    }
    host.close_review();
}

/// The sheet for the update `view` offers, over a build at `installed`; none when no update is on
/// offer. The answer comes back once: a press of a button, Escape or a click outside for Cancel.
///
/// The primary button takes the keyboard focus when the sheet opens, so Return or Space confirms
/// it like any focused button. Nothing reads Return itself: the key that pressed the button which
/// opened the sheet would otherwise confirm it in the same frame.
pub fn show(ctx: &egui::Context, view: &UpdateView, installed: &str) -> Option<Outcome> {
    let Phase::Available {
        version,
        url,
        installable,
        note,
        notes,
        coverage,
    } = &view.phase
    else {
        return None;
    };
    let id = Id::new("update-review");
    let primary_id = id.with("primary");
    let ring = kit::focus::start_on(ctx, id, Some(primary_id));
    let mut outcome = None;
    let list_height = list_height(ctx.content_rect().height(), remembered_chrome(ctx, id));
    let ((), outside) = sheet::show_dismissable(ctx, id, WIDTH, |ui| {
        ui.add_space(TOP_SPACE);
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
        ui.add_space(HEADER_GAP);
        let shown_list = notes_list(ui, notes, coverage, list_height);
        ui.add_space(GROUP_GAP);
        sheet::inset(ui, |ui| {
            if !installable && let Some(note) = note {
                ui.add(
                    egui::Label::new(kit::text(
                        note,
                        DETAIL_TEXT,
                        REGULAR,
                        theme::text_secondary(),
                    ))
                    .wrap(),
                );
                ui.add_space(NOTE_GAP);
            }
            release_notes_link(ui, url);
            ui.add_space(BOTTOM_SPACE);
        });
        sheet::footer(ui, |ui| {
            let label = if *installable {
                "Update"
            } else {
                "View Release"
            };
            if kit::sheet_button_focusable(ui, primary_id, ring, label, true, true).clicked() {
                outcome = Some(if *installable {
                    Outcome::Update
                } else {
                    Outcome::ViewRelease
                });
            }
            if kit::sheet_button_focusable(ui, id.with("cancel"), ring, "Cancel", false, true)
                .clicked()
            {
                outcome = Some(Outcome::Cancel);
            }
        });
        remember_chrome(ui, id, ui.min_rect().height() - shown_list);
    });
    if outcome.is_none() && (outside || ctx.input(|input| input.key_pressed(Key::Escape))) {
        outcome = Some(Outcome::Cancel);
    }
    outcome
}

/// How tall the notes may be in a window `window` points high, when everything else in the sheet
/// takes `chrome`: what is left between the margins, never more than `MAX_LIST` and, when even
/// that is too little, `MIN_LIST` all the same.
fn list_height(window: f32, chrome: f32) -> f32 {
    (window - chrome - 2.0 * EDGE_MARGIN).clamp(MIN_LIST, MAX_LIST)
}

/// The height of the sheet without its notes, as the previous frame laid it out (the header, the
/// note under the list, the link, the footer). The first frame of a sheet is a sizing pass nobody
/// sees, so it may assume the list is free to grow.
fn remembered_chrome(ctx: &egui::Context, id: Id) -> f32 {
    ctx.data(|data| data.get_temp(id.with("chrome")))
        .unwrap_or(0.0)
}

fn remember_chrome(ui: &Ui, id: Id, chrome: f32) {
    let ctx = ui.ctx();
    let before = remembered_chrome(ctx, id);
    ctx.data_mut(|data| data.insert_temp(id.with("chrome"), chrome));
    if (chrome - before).abs() > CHROME_TOLERANCE {
        ctx.request_repaint();
    }
}

/// The changes in a list that scrolls past `height`. While there is more above or below the
/// visible part its edge fades into the sheet and the bar stays in view, so a list never looks
/// finished where it is only cut. Returns the height the list took.
fn notes_list(ui: &mut Ui, notes: &ReleaseNotes, coverage: &Coverage, height: f32) -> f32 {
    let output = ui
        .scope(|ui| {
            kit::scroll::keep_bars_visible(ui);
            egui::ScrollArea::vertical()
                .id_salt("update-review-notes")
                .max_height(height)
                .auto_shrink([false, true])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                .show(ui, |ui| sheet::inset(ui, |ui| changes(ui, notes, coverage)))
        })
        .inner;
    let beyond = kit::scroll::Beyond::of(
        output.state.offset.y,
        output.content_size.y,
        output.inner_rect.height(),
    );
    kit::scroll::fade_edges(ui.painter(), output.inner_rect, theme::chrome(), beyond);
    output.inner_rect.height()
}

/// New, then Fixed, each only when it has entries, then how many more there were (or that there are
/// more, when a body was cut short and the count would be wrong) and how many releases had no
/// summary or were never listed; a calm line when there is nothing to list.
fn changes(ui: &mut Ui, notes: &ReleaseNotes, coverage: &Coverage) {
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
            DETAIL_TEXT,
            SEMIBOLD,
            theme::text_secondary(),
        ));
        ui.add_space(TITLE_GAP);
        for (at, entry) in entries.iter().enumerate() {
            if at > 0 {
                ui.add_space(ENTRY_GAP);
            }
            bullet(ui, entry);
        }
    }
    for line in notes.footnotes(coverage) {
        ui.add_space(GROUP_GAP);
        ui.add(
            egui::Label::new(kit::text(
                line,
                DETAIL_TEXT,
                REGULAR,
                theme::text_secondary(),
            ))
            .wrap(),
        );
    }
}

/// One change: a bullet, and its text wrapped to the width left beside it so a long line hangs
/// under its first word.
fn bullet(ui: &mut Ui, text: &str) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = BULLET_GAP;
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
                    DETAIL_TEXT,
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
