//! The profiler's two surfaces besides the editor's column: the stopwatch in the Script Manager's
//! toolbar and the Profile tab of the drawer. Both draw what they are handed and return what was
//! asked of them; neither runs or measures anything.
//!
//! The tab is laid out like the Logs tab beside it: a toolbar row (what the profiler is doing, and
//! the one button that does the next thing), a hairline, then the body: an introduction while it is
//! off, a message while there is nothing to show, the result otherwise.

use std::cell::RefCell;

use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2,
    vec2,
};

use super::profile_marks::{HotRow, ProfileMarks, SummaryGroup, SummaryRow, count};
use super::profiler::{Profile, Status};
use super::tab_chrome::{self as chrome, BUTTON, banner, caption, card, hairline};
use crate::pine::profile::RegionKind;
use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::pine_logs_view::{centered, intro, step};
use crate::ui::shortcut;
use crate::ui::theme;

const ROW_HEIGHT: f32 = 26.0;

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

// MARK: What the profiler is doing, in words

/// How a state is coloured: the dot in the tab, the stopwatch in the toolbar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tone {
    Quiet,
    Active,
    Good,
    Caution,
    Bad,
}

impl Tone {
    fn color(self) -> Color32 {
        match self {
            Self::Quiet => theme::text_secondary(),
            Self::Active => kit::ACCENT,
            Self::Good => kit::GREEN,
            Self::Caution => theme::profiler_caution(),
            Self::Bad => kit::RED,
        }
    }
}

/// A state's words and what the one button beside them does.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Description {
    pub label: String,
    pub tone: Tone,
    pub action: Option<Action>,
    /// What the button does, as its tooltip.
    pub button: &'static str,
}

pub fn describe(status: &Status) -> Description {
    let run = |label: &str, tone, button| Description {
        label: label.to_owned(),
        tone,
        action: Some(Action::Run),
        button,
    };
    match status {
        Status::Off => Description {
            label: "Profiler off".to_owned(),
            tone: Tone::Quiet,
            action: None,
            button: "",
        },
        Status::Idle => run("Not profiled yet", Tone::Quiet, "Run the profiler"),
        Status::Waiting(_) => Description {
            label: "Waiting for the chart\u{2026}".to_owned(),
            tone: Tone::Quiet,
            action: None,
            button: "",
        },
        Status::Running {
            done,
            total,
            cancelling,
        } => Description {
            label: if *cancelling {
                "Cancelling\u{2026}".to_owned()
            } else {
                format!(
                    "Profiling\u{2026} {} of {} bars",
                    count(*done),
                    count(*total)
                )
            },
            tone: Tone::Active,
            action: (!*cancelling).then_some(Action::Cancel),
            button: "Cancel profiling; what was measured so far is kept",
        },
        Status::Ready => run("Profile ready", Tone::Good, "Profile again"),
        Status::Partial => run("Partial profile", Tone::Caution, "Profile again"),
        Status::Stale => run("Profile out of date", Tone::Caution, "Profile again"),
        Status::Failed(_) => run("Profile failed", Tone::Bad, "Try again"),
    }
}

// MARK: Top bar

/// The toolbar's profiler control: the stopwatch that turns it on and off. It carries the state in
/// its colour and its tooltip; the words are in the Profile tab.
pub fn toolbar(ui: &mut Ui, status: &Status) -> Vec<Action> {
    let mut actions = Vec::new();
    let on = *status != Status::Off;
    let description = describe(status);
    let tip = if on {
        format!(
            "{}\n{}",
            shortcut::PROFILER_TOGGLE.hint("Turn the profiler off"),
            description.label
        )
    } else {
        shortcut::PROFILER_TOGGLE
            .hint("Turn the profiler on: see which lines of the script take the time")
    };
    let color = match (on, description.tone) {
        (false, _) => theme::text_primary(),
        (true, Tone::Caution) => Tone::Caution.color(),
        (true, Tone::Bad) => Tone::Bad.color(),
        (true, _) => kit::ACCENT,
    };
    if icons::button(ui, Icon::Gauge, 28.0, color, &tip).clicked() {
        actions.push(Action::Toggle);
    }
    actions
}

// MARK: The tab

/// What the Profile tab draws.
pub struct TabView<'a> {
    pub status: &'a Status,
    pub profile: Option<&'a Profile>,
    /// The profile is of the text in the editor, so its line numbers can be followed.
    pub lines_match: bool,
}

/// The Profile tab.
pub fn tab(ui: &mut Ui, view: &TabView<'_>) -> Vec<Action> {
    let mut actions = Vec::new();
    ui.set_min_height(ui.available_height());
    toolbar_row(ui, view, &mut actions);
    match (view.status, view.profile) {
        (Status::Off, _) => introduction(ui, &mut actions),
        (Status::Running { done, total, .. }, _) => running(ui, *done, *total, &mut actions),
        (Status::Idle, None) => message(
            ui,
            "Ready to profile",
            "Runs the script once on the preview chart's bars and times every line.",
            Some(("Run profile", Icon::PlayCircle)),
            &mut actions,
        ),
        (Status::Waiting(why), None) => {
            message(ui, "Waiting for the chart", why, None, &mut actions);
        }
        (Status::Failed(why), None) => message(
            ui,
            "This script could not be profiled",
            why,
            Some(("Try again", Icon::Repeat)),
            &mut actions,
        ),
        (_, Some(profile)) => result(ui, view, profile, &mut actions),
        (_, None) => message(ui, "Nothing profiled yet", "", None, &mut actions),
    }
    actions
}

/// The state, in a field like the Logs tab's script picker, and the button that does the next thing.
fn toolbar_row(ui: &mut Ui, view: &TabView<'_>, actions: &mut Vec<Action>) {
    let description = describe(view.status);
    chrome::toolbar(ui, |ui, width| {
        let field = chrome::field_width(width, usize::from(description.action.is_some()));
        // The chart the profile is of, at the field's right edge, if there is room beside the state.
        let chart = view
            .profile
            .filter(|_| *view.status != Status::Off)
            .map(|profile| (profile.label.as_str(), theme::text_secondary()));
        chrome::state_field(
            ui,
            field,
            description.tone.color(),
            &description.label,
            chart,
        );
        if let Some(action) = description.action {
            ui.add_space(6.0);
            let icon = if action == Action::Cancel {
                Icon::Xmark
            } else {
                Icon::Repeat
            };
            if icons::button(
                ui,
                icon,
                BUTTON,
                theme::text_secondary(),
                description.button,
            )
            .clicked()
            {
                actions.push(action);
            }
        }
    });
}

// MARK: Off, waiting, failed, running

/// While the profiler is off: what it is, how it works, and a button, laid out like the Logs tab's
/// introduction.
fn introduction(ui: &mut Ui, actions: &mut Vec<Action>) {
    centered(ui, |ui| {
        intro(
            ui,
            "Find what makes a script slow",
            "See where its time goes, line by line.",
            |ui| {
                step(ui, 1, |ui| {
                    ui.label(muted("Turn the profiler on with the stopwatch."));
                });
                step(ui, 2, |ui| {
                    ui.label(muted("It runs the script on the preview chart's bars."));
                });
                step(ui, 3, |ui| {
                    ui.label(muted("Bars and shares appear beside the code."));
                });
            },
            example,
            illustration,
            |ui| {
                ui.add_space(14.0);
                if kit::push_button(ui, Some(Icon::Gauge), "Start profiling", true).clicked() {
                    actions.push(Action::Toggle);
                }
            },
        );
    });
}

/// A heading, a line under it, and optionally the button that does what it says.
fn message(
    ui: &mut Ui,
    heading: &str,
    detail: &str,
    button: Option<(&str, Icon)>,
    actions: &mut Vec<Action>,
) {
    centered(ui, |ui| {
        ui.label(kit::text(
            heading,
            kit::size::HEADLINE,
            kit::SEMIBOLD,
            theme::text_primary(),
        ));
        if !detail.is_empty() {
            ui.add_space(2.0);
            ui.label(muted(detail));
        }
        if let Some((label, icon)) = button {
            ui.add_space(10.0);
            if kit::push_button(ui, Some(icon), label, true).clicked() {
                actions.push(Action::Run);
            }
        }
    });
}

fn running(ui: &mut Ui, done: u64, total: u64, actions: &mut Vec<Action>) {
    centered(ui, |ui| {
        ui.label(kit::text(
            "Profiling\u{2026}",
            kit::size::HEADLINE,
            kit::SEMIBOLD,
            theme::text_primary(),
        ));
        ui.add_space(8.0);
        let fraction = if total == 0 {
            0.0
        } else {
            (done as f32 / total as f32).clamp(0.0, 1.0)
        };
        let (rect, _) = ui.allocate_exact_size(vec2(180.0, 4.0), Sense::hover());
        ui.painter().rect_filled(rect, 2.0, theme::profiler_track());
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min, vec2(rect.width() * fraction, 4.0)),
            2.0,
            kit::ACCENT,
        );
        ui.add_space(6.0);
        ui.label(muted(&format!("{} of {} bars", count(done), count(total))));
        ui.add_space(10.0);
        if kit::push_button(ui, Some(Icon::Xmark), "Cancel", true).clicked() {
            actions.push(Action::Cancel);
        }
    });
}

// MARK: A result

fn result(ui: &mut Ui, view: &TabView<'_>, profile: &Profile, actions: &mut Vec<Action>) {
    let navigable = view.lines_match;
    chrome::scroll(ui, "profile-result", |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if let Some(caveat) = &profile.marks.caveat {
            banner(ui, caveat);
        }
        if *view.status == Status::Stale {
            banner(
                ui,
                if navigable {
                    "This profile is of earlier inputs or data. Its lines still match the editor."
                } else {
                    "This profile is of earlier text. Its lines no longer match the editor."
                },
            );
        }
        totals(ui, &profile.marks, navigable);
        ui.add_space(16.0);
        let hottest_lines = |ui: &mut Ui, actions: &mut Vec<Action>| {
            caption(ui, "Hottest lines");
            hottest(ui, profile, navigable, actions);
        };
        let groups = |ui: &mut Ui, actions: &mut Vec<Action>, first: bool| {
            let mut first = first;
            for group in [SummaryGroup::Hotspots, SummaryGroup::Measurement] {
                let rows: Vec<&SummaryRow> = profile
                    .marks
                    .summary
                    .iter()
                    .filter(|row| row.group == group)
                    .collect();
                if rows.is_empty() {
                    continue;
                }
                if !std::mem::take(&mut first) {
                    ui.add_space(16.0);
                }
                caption(ui, group.title());
                details(ui, &rows, navigable, actions);
            }
        };
        let any_group = profile.marks.summary.iter().any(|row| {
            matches!(
                row.group,
                SummaryGroup::Hotspots | SummaryGroup::Measurement
            )
        });
        if chrome::wide(ui) && any_group {
            // The ranked lines beside the figures, so the drawer's height goes to whichever is longer.
            let actions = RefCell::new(&mut *actions);
            chrome::columns(
                ui,
                0.56,
                |ui| hottest_lines(ui, &mut actions.borrow_mut()),
                |ui| groups(ui, &mut actions.borrow_mut(), true),
            );
        } else {
            hottest_lines(ui, actions);
            if any_group {
                ui.add_space(16.0);
            }
            groups(ui, actions, true);
        }
    });
}

/// The side of the chip that holds a total's icon.
const TILE_ICON: f32 = 24.0;
/// Narrower than this, a tile keeps its whole label and leaves the icon out.
const TILE_ICON_MIN_WIDTH: f32 = 120.0;

/// The run's three figures, side by side.
fn totals(ui: &mut Ui, marks: &ProfileMarks, current: bool) {
    let gap = 8.0;
    let width = ((ui.available_width() - gap * 2.0) / 3.0).max(40.0);
    let tone = if current {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for (icon, tint, label, value) in [
            (Icon::Clock, kit::ACCENT, "Total time", &marks.totals.time),
            (Icon::BarChart, kit::GREEN, "Bars", &marks.totals.bars),
            (
                Icon::Repeat,
                kit::ORANGE,
                "Executions",
                &marks.totals.executions,
            ),
        ] {
            let (rect, _) = ui.allocate_exact_size(vec2(width, 52.0), Sense::hover());
            ui.painter().rect_filled(rect, 10.0, theme::ink(7));
            ui.painter().rect_stroke(
                rect,
                10.0,
                Stroke::new(1.0, theme::ink(14)),
                StrokeKind::Inside,
            );
            // A tinted chip in the corner says what the figure counts, when the tile has room
            // for it beside the label.
            let chip_room = if width >= TILE_ICON_MIN_WIDTH {
                TILE_ICON + 6.0
            } else {
                0.0
            };
            let top = rect.top() + 8.0;
            if chip_room > 0.0 {
                let chip = Rect::from_min_size(
                    pos2(rect.right() - 10.0 - TILE_ICON, top),
                    Vec2::splat(TILE_ICON),
                );
                ui.painter()
                    .rect_filled(chip, 7.0, theme::with_alpha(tint, 0.14));
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(chip.center(), Vec2::splat(14.0)),
                    icon,
                    tint,
                );
            }
            let name = kit::galley(
                ui,
                label,
                kit::size::CAPTION,
                kit::REGULAR,
                theme::text_secondary(),
                width - 20.0 - chip_room,
            );
            ui.painter().galley(
                pos2(
                    rect.left() + 10.0,
                    top + TILE_ICON / 2.0 - name.size().y / 2.0,
                ),
                name,
                theme::text_secondary(),
            );
            let figure = kit::galley(ui, value, 16.0, kit::SEMIBOLD, tone, width - 20.0);
            ui.painter().galley(
                pos2(rect.left() + 10.0, rect.bottom() - 10.0 - figure.size().y),
                figure,
                tone,
            );
        }
    });
}

fn hottest(ui: &mut Ui, profile: &Profile, navigable: bool, actions: &mut Vec<Action>) {
    if profile.marks.hot.is_empty() {
        ui.label(muted("No line took a measurable share of the time."));
        return;
    }
    card(ui, |ui| {
        for (index, row) in profile.marks.hot.iter().enumerate() {
            if index > 0 {
                hairline(ui);
            }
            if hot_row(ui, row, navigable).clicked() && navigable {
                actions.push(Action::Reveal(row.line));
            }
        }
    });
}

/// One ranked line: where it is, what it says, and on the right how long it took itself and how
/// often it ran. A thin bar along its foot is its share of the script's time.
fn hot_row(ui: &mut Ui, row: &HotRow, navigable: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), ROW_HEIGHT),
        if navigable {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let response = crate::ui::accessibility::label(
        response,
        format!("Line {}, {} of line time", row.line, row.line_time),
    );
    if navigable && response.hovered() {
        ui.painter().rect_filled(rect, 6.0, theme::ink(10));
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let painter = ui.painter().with_clip_rect(rect);
    let primary = if navigable {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    let secondary = theme::text_secondary();
    let mid = rect.center().y;
    let fraction = row.line_share.clamp(0.0, 1.0);
    painter.rect_filled(
        Rect::from_min_size(
            pos2(rect.min.x + 12.0, rect.max.y - 3.0),
            vec2((rect.width() - 24.0) * fraction, 2.0),
        ),
        1.0,
        theme::profiler_heat(fraction.sqrt()),
    );
    painter.text(
        pos2(rect.min.x + 12.0, mid - 1.0),
        Align2::LEFT_CENTER,
        format!("L{}", row.line),
        FontId::monospace(10.5),
        secondary,
    );
    let kind = match row.kind {
        RegionKind::Loop => "loop ",
        RegionKind::Function => "fn ",
        _ => "",
    };
    let right = painter.layout_no_wrap(
        format!("{}   {}\u{d7}", row.line_time, row.executions),
        FontId::monospace(10.5),
        secondary,
    );
    let right_x = rect.max.x - 12.0 - right.size().x;
    painter.galley(
        pos2(right_x, mid - right.size().y / 2.0 - 1.0),
        right,
        secondary,
    );
    let left = rect.min.x + 52.0;
    let mut job = egui::text::LayoutJob::simple_singleline(
        format!("{kind}{}", row.snippet),
        FontId::monospace(11.0),
        primary,
    );
    job.wrap = egui::text::TextWrapping {
        max_width: (right_x - 10.0 - left).max(0.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('\u{2026}'),
    };
    let snippet = painter.layout_job(job);
    painter.galley(
        pos2(left, mid - snippet.size().y / 2.0 - 1.0),
        snippet,
        primary,
    );
    response
}

/// A card of label-and-value rows: the label and what it is of on the left, the figure on the
/// right, a hairline between rows. A row that names a line goes to it.
fn details(ui: &mut Ui, rows: &[&SummaryRow], navigable: bool, actions: &mut Vec<Action>) {
    card(ui, |ui| {
        for (index, row) in rows.iter().enumerate() {
            if index > 0 {
                hairline(ui);
            }
            let clickable = navigable && row.line.is_some();
            let response = detail_row(ui, row, clickable, navigable);
            if response.clicked()
                && let Some(line) = row.line
            {
                actions.push(Action::Reveal(line));
            }
        }
    });
}

fn detail_row(ui: &mut Ui, row: &SummaryRow, clickable: bool, current: bool) -> egui::Response {
    const PAD_X: f32 = 12.0;
    const PAD_Y: f32 = 8.0;
    let width = ui.available_width();
    let figure_color = if current {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    let value = kit::galley(
        ui,
        &row.value,
        kit::size::BODY,
        kit::MEDIUM,
        figure_color,
        width * 0.45,
    );
    let text_width = (width - PAD_X * 2.0 - value.size().x - 14.0).max(40.0);
    let label = kit::galley(
        ui,
        row.label,
        kit::size::BODY,
        kit::REGULAR,
        theme::text_primary(),
        text_width,
    );
    let note = row.note.as_ref().map(|note| {
        ui.painter().layout(
            note.clone(),
            FontId::proportional(kit::size::CAPTION + 0.5),
            theme::text_secondary(),
            text_width,
        )
    });
    let height =
        PAD_Y * 2.0 + label.size().y + note.as_ref().map_or(0.0, |note| 2.0 + note.size().y);
    let (rect, response) = ui.allocate_exact_size(
        vec2(width, height),
        if clickable {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let response =
        crate::ui::accessibility::label(response, format!("{}, {}", row.label, row.value));
    if clickable && response.hovered() {
        ui.painter().rect_filled(rect, 6.0, theme::ink(10));
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let painter = ui.painter();
    painter.galley(
        pos2(rect.left() + PAD_X, rect.top() + PAD_Y),
        label.clone(),
        theme::text_primary(),
    );
    if let Some(note) = note {
        painter.galley(
            pos2(
                rect.left() + PAD_X,
                rect.top() + PAD_Y + label.size().y + 2.0,
            ),
            note,
            theme::text_secondary(),
        );
    }
    painter.galley(
        pos2(
            rect.right() - PAD_X - value.size().x,
            rect.top() + PAD_Y + (label.size().y - value.size().y) / 2.0,
        ),
        value,
        figure_color,
    );
    response
}

// MARK: Pieces

/// The introduction's sentences: secondary, never lighter, so they read in both appearances.
fn muted(text: &str) -> egui::RichText {
    kit::text(
        text,
        kit::size::SUBHEADLINE,
        kit::REGULAR,
        theme::text_secondary(),
    )
}

/// A card of three bars on a soft disc, one with a flame: the column the profiler adds to the editor.
fn illustration(ui: &Ui, rect: Rect) {
    let painter = ui.painter();
    let centre = rect.center();
    painter.circle_filled(centre, 40.0, kit::ACCENT.gamma_multiply(0.10));
    let card = Rect::from_center_size(centre, vec2(64.0, 54.0));
    painter.rect_filled(card, 9.0, theme::chrome());
    painter.rect_stroke(
        card,
        9.0,
        Stroke::new(1.5, theme::text_secondary()),
        StrokeKind::Inside,
    );
    for (index, (length, heat)) in [(0.35, 0.1), (0.9, 1.0), (0.55, 0.45)]
        .into_iter()
        .enumerate()
    {
        let y = card.top() + 14.0 + index as f32 * 13.0;
        let left = card.left() + 20.0;
        let full = card.width() - 32.0;
        painter.rect_filled(
            Rect::from_min_size(pos2(left, y - 2.5), vec2(full, 5.0)),
            2.5,
            theme::profiler_track(),
        );
        painter.rect_filled(
            Rect::from_min_size(pos2(left, y - 2.5), vec2(full * length, 5.0)),
            2.5,
            theme::profiler_heat(heat),
        );
        if index == 1 {
            icons::paint(
                painter,
                Rect::from_center_size(pos2(card.left() + 11.0, y), vec2(9.0, 9.0)),
                Icon::Flame,
                theme::profiler_flame(),
            );
        }
    }
}

/// A line of what the editor will show: a flame, a bar, a share and the line it is of.
fn example(ui: &mut Ui, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 34.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 8.0, theme::well_fill());
    let mid = rect.center().y;
    icons::paint(
        painter,
        Rect::from_center_size(pos2(rect.left() + 14.0, mid), vec2(11.0, 11.0)),
        Icon::Flame,
        theme::profiler_flame(),
    );
    let track = Rect::from_min_size(pos2(rect.left() + 26.0, mid - 3.0), vec2(40.0, 6.0));
    painter.rect_filled(track, 2.0, theme::profiler_track());
    painter.rect_filled(
        Rect::from_min_size(track.min, vec2(28.0, 6.0)),
        2.0,
        theme::profiler_heat(0.9),
    );
    painter.text(
        pos2(track.right() + 8.0, mid),
        Align2::LEFT_CENTER,
        "58.3%",
        FontId::monospace(10.0),
        theme::text_primary(),
    );
    painter.text(
        pos2(track.right() + 52.0, mid),
        Align2::LEFT_CENTER,
        "w = weight(i)",
        FontId::monospace(11.0),
        theme::text_primary(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(status: Status) -> (String, Option<Action>) {
        let description = describe(&status);
        (description.label, description.action)
    }

    #[test]
    fn each_state_has_words_and_a_button_that_does_the_obvious_next_thing() {
        assert_eq!(
            words(Status::Idle),
            ("Not profiled yet".into(), Some(Action::Run))
        );
        assert_eq!(
            words(Status::Running {
                done: 240,
                total: 500,
                cancelling: false
            }),
            (
                "Profiling\u{2026} 240 of 500 bars".into(),
                Some(Action::Cancel)
            )
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
            ("Profile out of date".into(), Some(Action::Run))
        );
        assert_eq!(
            words(Status::Failed("x".into())),
            ("Profile failed".into(), Some(Action::Run))
        );
        assert_eq!(words(Status::Waiting("w")).1, None);
        assert_eq!(words(Status::Off).1, None);
    }

    #[test]
    fn a_state_is_coloured_by_how_much_it_wants_looking_at() {
        let tone = |status: Status| describe(&status).tone;
        assert_eq!(tone(Status::Ready), Tone::Good);
        assert_eq!(tone(Status::Stale), Tone::Caution);
        assert_eq!(tone(Status::Partial), Tone::Caution);
        assert_eq!(tone(Status::Failed("x".into())), Tone::Bad);
        assert_eq!(tone(Status::Idle), Tone::Quiet);
    }

    fn frame(ctx: &egui::Context, body: impl FnOnce(&mut Ui)) {
        let mut body = Some(body);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            if let Some(body) = body.take() {
                body(ui);
            }
        });
        output.textures_delta.clear();
    }

    fn every_status() -> Vec<Status> {
        vec![
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
        ]
    }

    #[test]
    fn the_stopwatch_asks_for_nothing_by_itself_and_takes_the_same_room_in_every_state() {
        let ctx = egui::Context::default();
        let mut widths = Vec::new();
        for status in every_status() {
            let mut used = 0.0;
            frame(&ctx, |ui| {
                assert!(toolbar(ui, &status).is_empty(), "{status:?}");
                used = ui.min_rect().width();
            });
            widths.push(used.round());
        }
        assert!(
            widths.windows(2).all(|pair| pair[0] == pair[1]),
            "{widths:?}"
        );
    }

    #[test]
    fn the_tab_draws_in_every_state_without_a_profile() {
        let ctx = egui::Context::default();
        for status in every_status() {
            frame(&ctx, |ui| {
                let view = TabView {
                    status: &status,
                    profile: None,
                    lines_match: false,
                };
                assert!(tab(ui, &view).is_empty(), "{status:?}");
            });
        }
    }
}
