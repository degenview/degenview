//! The profiler's two surfaces besides the editor's column: the control in the Script Manager's
//! toolbar and the Profile tab of the drawer. Both draw what they are handed and return what was
//! asked of them; neither runs or measures anything.

use eframe::egui::{
    self, Align, Color32, CursorIcon, Layout, Margin, Rect, RichText, ScrollArea, Sense, Ui, Vec2,
    pos2, vec2,
};

use super::profile_marks::{HotRow, count};
use super::profiler::{Profile, Status};
use crate::pine::profile::RegionKind;
use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::shortcut;
use crate::ui::theme;

/// What a surface asked for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    /// Turn the profiler on or off.
    Toggle,
    /// Profile again (or for the first time).
    Run,
    /// End the run in flight.
    Cancel,
    /// Show a one-based line in the editor.
    Reveal(usize),
}

// MARK: Toolbar

/// The toolbar's profiler control: the button that turns it on and off, and, while it is on, a chip
/// that says what it is doing and does the obvious next thing when clicked. Added in a
/// right-to-left layout, so the chip lands left of the button.
pub fn toolbar(ui: &mut Ui, status: &Status) -> Vec<Action> {
    let mut actions = Vec::new();
    let on = *status != Status::Off;
    let tip = if on {
        shortcut::PROFILER_TOGGLE.hint("Turn the profiler off")
    } else {
        shortcut::PROFILER_TOGGLE
            .hint("Turn the profiler on: see which lines of the script take the time")
    };
    let color = if on {
        kit::ACCENT
    } else {
        theme::text_primary()
    };
    if icons::button(ui, Icon::Gauge, 28.0, color, &tip).clicked() {
        actions.push(Action::Toggle);
    }
    if !on {
        return actions;
    }
    let (label, tone, tooltip, action) = chip(status);
    if chip_button(ui, &label, tone, &tooltip, action.is_some()).clicked()
        && let Some(action) = action
    {
        actions.push(action);
    }
    actions
}

/// The chip's words, colour, tooltip, and what a click does.
fn chip(status: &Status) -> (String, Color32, String, Option<Action>) {
    match status {
        Status::Off => (String::new(), theme::text_secondary(), String::new(), None),
        Status::Idle => (
            "Run profile".to_owned(),
            theme::text_secondary(),
            "Run the script on the preview chart's bars and time every line".to_owned(),
            Some(Action::Run),
        ),
        Status::Waiting(why) => (
            "Waiting for chart\u{2026}".to_owned(),
            theme::text_secondary(),
            (*why).to_owned(),
            None,
        ),
        Status::Running {
            done,
            total,
            cancelling,
        } => (
            if *cancelling {
                "Cancelling\u{2026}".to_owned()
            } else {
                format!("Profiling\u{2026} {}/{}", count(*done), count(*total))
            },
            theme::text_secondary(),
            "Cancel profiling; what was measured so far is kept".to_owned(),
            (!*cancelling).then_some(Action::Cancel),
        ),
        Status::Ready => (
            "Profile ready".to_owned(),
            theme::text_secondary(),
            "Profile again".to_owned(),
            Some(Action::Run),
        ),
        Status::Partial => (
            "Partial profile".to_owned(),
            theme::profiler_caution(),
            "The run did not finish; see the Profile tab. Profile again".to_owned(),
            Some(Action::Run),
        ),
        Status::Stale => (
            "Profile stale \u{b7} Run again".to_owned(),
            theme::profiler_caution(),
            "The script, its inputs or the chart changed since this profile. Profile again"
                .to_owned(),
            Some(Action::Run),
        ),
        Status::Failed(message) => (
            "Profile failed".to_owned(),
            kit::RED,
            format!("{message} Click to try again."),
            Some(Action::Run),
        ),
    }
}

fn chip_button(
    ui: &mut Ui,
    label: &str,
    tone: Color32,
    tooltip: &str,
    clickable: bool,
) -> egui::Response {
    let galley = kit::galley(ui, label, 11.0, kit::MEDIUM, tone, 400.0);
    let size = vec2(galley.size().x + 16.0, 22.0);
    let (rect, response) = ui.allocate_exact_size(
        size,
        if clickable {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let response = crate::ui::accessibility::label(response, label).on_hover_text(tooltip);
    if clickable && response.hovered() {
        ui.painter().rect_filled(rect, 6.0, theme::ink(14));
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    ui.painter().galley(
        pos2(rect.min.x + 8.0, rect.center().y - galley.size().y / 2.0),
        galley,
        tone,
    );
    response
}

// MARK: Drawer tab

/// What the Profile tab draws.
pub struct TabView<'a> {
    pub status: &'a Status,
    pub profile: Option<&'a Profile>,
    /// The profile is of the text in the editor, so its line numbers can be followed.
    pub lines_match: bool,
}

/// The Profile tab: what the profiler is doing, the summary of the run, and the hottest lines.
pub fn tab(ui: &mut Ui, view: &TabView<'_>) -> Vec<Action> {
    let mut actions = Vec::new();
    if *view.status == Status::Off {
        off(ui, &mut actions);
        return actions;
    }
    header(ui, view, &mut actions);
    let Some(profile) = view.profile else {
        waiting_body(ui, view.status);
        return actions;
    };
    let current = view.lines_match;
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(Margin::symmetric(12, 4))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    if let Some(caveat) = &profile.marks.caveat {
                        note(ui, caveat, theme::profiler_caution());
                    }
                    if *view.status == Status::Stale {
                        note(
                            ui,
                            if view.lines_match {
                                "This profile is of earlier inputs or data. Its lines still match the editor."
                            } else {
                                "This profile is of earlier text. Its lines no longer match the editor."
                            },
                            theme::profiler_caution(),
                        );
                    }
                    ui.label(kit::text(
                        profile.marks.headline.clone(),
                        12.0,
                        kit::SEMIBOLD,
                        if current {
                            theme::text_primary()
                        } else {
                            theme::text_secondary()
                        },
                    ));
                    ui.label(kit::text(
                        profile.label.clone(),
                        10.5,
                        kit::REGULAR,
                        theme::text_tertiary(),
                    ));
                    ui.add_space(6.0);
                    hottest(ui, profile, current, &mut actions);
                    ui.add_space(8.0);
                    summary(ui, profile, current);
                });
        });
    actions
}

fn off(ui: &mut Ui, actions: &mut Vec<Action>) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() / 2.0 - 50.0).max(8.0));
        icons::inline(ui, Icon::Gauge, 20.0, theme::text_tertiary());
        ui.add_space(4.0);
        ui.label(kit::text(
            "Find what makes a script slow",
            11.0,
            kit::SEMIBOLD,
            theme::text_primary(),
        ));
        ui.add_space(2.0);
        ui.label(
            RichText::new("Times every line on the preview chart's bars, and shows where it goes.")
                .size(10.0)
                .color(theme::text_secondary()),
        );
        ui.add_space(8.0);
        if ui
            .add(
                egui::Button::new(kit::text(
                    "Start profiling",
                    11.0,
                    kit::MEDIUM,
                    Color32::WHITE,
                ))
                .fill(theme::SYSTEM_ACCENT)
                .corner_radius(6.0),
            )
            .clicked()
        {
            actions.push(Action::Toggle);
        }
    });
}

fn header(ui: &mut Ui, view: &TabView<'_>, actions: &mut Vec<Action>) {
    let (label, tone, _, action) = chip(view.status);
    egui::Frame::new()
        .inner_margin(Margin::symmetric(12, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(kit::text(
                    "Profile",
                    11.0,
                    kit::SEMIBOLD,
                    theme::text_primary(),
                ));
                ui.label(kit::text(label, 11.0, kit::REGULAR, tone));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(action) = action {
                        let (icon, tip) = match action {
                            Action::Cancel => (Icon::Xmark, "Cancel profiling"),
                            _ => (Icon::Repeat, "Profile again"),
                        };
                        if icons::button(ui, icon, 24.0, theme::text_primary(), tip).clicked() {
                            actions.push(action);
                        }
                    }
                });
            });
            if let Status::Running { done, total, .. } = view.status {
                let fraction = if *total == 0 {
                    0.0
                } else {
                    *done as f32 / *total as f32
                };
                let (rect, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 3.0), Sense::hover());
                ui.painter().rect_filled(rect, 1.5, theme::profiler_track());
                ui.painter().rect_filled(
                    Rect::from_min_size(
                        rect.min,
                        vec2(rect.width() * fraction.clamp(0.0, 1.0), 3.0),
                    ),
                    1.5,
                    theme::SYSTEM_ACCENT,
                );
            }
        });
}

fn waiting_body(ui: &mut Ui, status: &Status) {
    let text = match status {
        Status::Failed(message) => message.clone(),
        Status::Waiting(why) => (*why).to_owned(),
        Status::Running { .. } => {
            "Running the script on the preview chart's bars\u{2026}".to_owned()
        }
        _ => "Press Run profile to time the script on the preview chart's bars.".to_owned(),
    };
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() / 2.0 - 24.0).max(8.0));
        ui.label(
            RichText::new(text)
                .size(11.0)
                .color(theme::text_secondary()),
        );
    });
}

fn note(ui: &mut Ui, text: &str, tone: Color32) {
    ui.label(kit::text(text, 11.0, kit::REGULAR, tone));
}

fn summary(ui: &mut Ui, profile: &Profile, current: bool) {
    let tone = if current {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    ui.label(kit::text(
        "Details",
        11.0,
        kit::SEMIBOLD,
        theme::text_primary(),
    ));
    egui::Grid::new("profile-summary")
        .num_columns(2)
        .spacing(Vec2::new(14.0, 5.0))
        .show(ui, |ui| {
            for row in &profile.marks.summary {
                ui.label(kit::text(
                    row.label,
                    11.0,
                    kit::REGULAR,
                    theme::text_secondary(),
                ));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(kit::text(row.value.clone(), 11.0, kit::MEDIUM, tone));
                    if let Some(note) = &row.note {
                        ui.add(
                            egui::Label::new(kit::text(
                                note.clone(),
                                10.0,
                                kit::REGULAR,
                                theme::text_tertiary(),
                            ))
                            .wrap(),
                        );
                    }
                });
                ui.end_row();
            }
        });
}

fn hottest(ui: &mut Ui, profile: &Profile, current: bool, actions: &mut Vec<Action>) {
    ui.label(kit::text(
        "Hottest lines",
        11.0,
        kit::SEMIBOLD,
        theme::text_primary(),
    ));
    if profile.marks.hot.is_empty() {
        ui.label(kit::text(
            "No line took a measurable share of the time.",
            11.0,
            kit::REGULAR,
            theme::text_secondary(),
        ));
        return;
    }
    for row in &profile.marks.hot {
        if hot_row(ui, row, current).clicked() && current {
            actions.push(Action::Reveal(row.line));
        }
    }
}

/// One ranked line: where it is, what it says, how long it took itself and how often it ran, with
/// a bar of its share. A click goes to the line while the profile still describes the editor.
fn hot_row(ui: &mut Ui, row: &HotRow, enabled: bool) -> egui::Response {
    let height = 22.0;
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), height),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let response = crate::ui::accessibility::label(
        response,
        format!("Line {}, {} of line time", row.line, row.line_time),
    );
    if enabled && response.hovered() {
        ui.painter().rect_filled(rect, 4.0, theme::ink(12));
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let painter = ui.painter().with_clip_rect(rect);
    let tone = if enabled {
        theme::text_primary()
    } else {
        theme::text_tertiary()
    };
    let mid = rect.center().y;
    // The share, as a bar along the row's foot.
    let fraction = row.line_share.clamp(0.0, 1.0);
    painter.rect_filled(
        Rect::from_min_size(
            pos2(rect.min.x + 4.0, rect.max.y - 3.0),
            vec2((rect.width() - 8.0) * fraction, 2.0),
        ),
        1.0,
        theme::profiler_heat(fraction.sqrt()),
    );
    painter.text(
        pos2(rect.min.x + 8.0, mid - 1.0),
        egui::Align2::LEFT_CENTER,
        format!("L{}", row.line),
        egui::FontId::monospace(10.5),
        theme::text_secondary(),
    );
    let kind = match row.kind {
        RegionKind::Loop => "loop ",
        RegionKind::Function => "fn ",
        _ => "",
    };
    let right = format!("{}   {}\u{d7}", row.line_time, row.executions);
    let right_galley = painter.layout_no_wrap(
        right,
        egui::FontId::monospace(10.5),
        theme::text_secondary(),
    );
    let right_x = rect.max.x - 8.0 - right_galley.size().x;
    painter.galley(
        pos2(right_x, mid - right_galley.size().y / 2.0 - 1.0),
        right_galley,
        theme::text_secondary(),
    );
    let left = rect.min.x + 52.0;
    let room = (right_x - 10.0 - left).max(0.0);
    let mut job = egui::text::LayoutJob::simple_singleline(
        format!("{kind}{}", row.snippet),
        egui::FontId::monospace(11.0),
        tone,
    );
    job.wrap = egui::text::TextWrapping {
        max_width: room,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('\u{2026}'),
    };
    let snippet_galley = painter.layout_job(job);
    painter.galley(
        pos2(left, mid - snippet_galley.size().y / 2.0 - 1.0),
        snippet_galley,
        tone,
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(status: Status) -> (String, Option<Action>) {
        let (label, _, _, action) = chip(&status);
        (label, action)
    }

    #[test]
    fn the_chip_names_each_state_and_a_click_does_the_obvious_next_thing() {
        assert_eq!(
            words(Status::Idle),
            ("Run profile".into(), Some(Action::Run))
        );
        assert_eq!(
            words(Status::Running {
                done: 240,
                total: 500,
                cancelling: false
            }),
            ("Profiling\u{2026} 240/500".into(), Some(Action::Cancel))
        );
        assert_eq!(
            words(Status::Running {
                done: 240,
                total: 500,
                cancelling: true
            }),
            ("Cancelling\u{2026}".into(), None),
            "a second click has nothing to cancel"
        );
        assert_eq!(
            words(Status::Ready),
            ("Profile ready".into(), Some(Action::Run))
        );
        assert_eq!(
            words(Status::Partial),
            ("Partial profile".into(), Some(Action::Run))
        );
        assert_eq!(
            words(Status::Stale),
            ("Profile stale \u{b7} Run again".into(), Some(Action::Run))
        );
        assert_eq!(
            words(Status::Failed("x".into())),
            ("Profile failed".into(), Some(Action::Run))
        );
        assert_eq!(words(Status::Waiting("w")).1, None);
    }

    #[test]
    fn the_toolbar_draws_in_every_state_and_asks_for_nothing_by_itself() {
        let ctx = egui::Context::default();
        for status in [
            Status::Off,
            Status::Idle,
            Status::Waiting("wait"),
            Status::Running {
                done: 1,
                total: 2,
                cancelling: false,
            },
            Status::Ready,
            Status::Partial,
            Status::Stale,
            Status::Failed("no".into()),
        ] {
            let mut asked = Vec::new();
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                asked = toolbar(ui, &status);
            });
            output.textures_delta.clear();
            assert!(asked.is_empty(), "{status:?}: {asked:?}");
        }
    }

    #[test]
    fn the_tab_draws_in_every_state() {
        let ctx = egui::Context::default();
        for status in [
            Status::Off,
            Status::Idle,
            Status::Running {
                done: 1,
                total: 2,
                cancelling: false,
            },
            Status::Failed("no".into()),
        ] {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let view = TabView {
                    status: &status,
                    profile: None,
                    lines_match: false,
                };
                assert!(tab(ui, &view).is_empty());
            });
            output.textures_delta.clear();
        }
    }
}
