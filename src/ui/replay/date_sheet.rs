//! Modal "Start Replay" sheet: choose the date and time the chart rewinds to, with
//! presets and a live "snaps to bar" preview. Pure view — it renders an egui window
//! over `ctx` and returns a `ReplayDateSheetResult` only when the user commits or cancels.

use eframe::egui::{self, Color32, RichText, Sense, Stroke, Vec2};

use crate::format::{civil_from_days, days_from_civil};
use crate::service::timezone::local_utc_offset_secs;
use crate::ui::icons::Icon;
use crate::ui::kit::{self, time_text::date_text};
use crate::ui::kit::{sheet, time_text::time_text};
use crate::ui::theme;

/// Owned by the workspace; `show` mutates it. `open` gates the modal, `draft_ms` is the
/// UTC instant (epoch milliseconds) the fields currently hold.
#[derive(Default)]
pub struct ReplayDateSheetState {
    open: bool,
    draft_ms: i64,
    display_year: i64,
    display_month: i64,
}

impl ReplayDateSheetState {
    /// Whether the sheet is on screen. The workspace uses this to keep its global Esc /
    /// tool handling out of the way while the modal is up.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Open the sheet, seeding the draft to the midpoint of the loaded chart (or now
    /// when the chart has no data yet).
    pub fn open(&mut self, timeline: &[i64]) {
        self.open = true;
        self.draft_ms = match range_of(timeline) {
            Some((lo, hi)) => lo + (hi - lo) / 2,
            None => now_ms(),
        };
        (self.display_year, self.display_month, ..) = local_fields(self.draft_ms);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReplayDateSheetResult {
    /// Commit: the requested start instant, UTC, in epoch milliseconds.
    Start(i64),
    Cancel,
}

/// Render the sheet when `state.is_open()`. `timeline` is the loaded chart's bar open
/// times, sorted ascending; it bounds the picker and drives the snap preview.
pub fn show(
    ctx: &egui::Context,
    state: &mut ReplayDateSheetState,
    timeline: &[i64],
) -> Option<ReplayDateSheetResult> {
    if !state.open {
        return None;
    }
    let range = range_of(timeline);
    let mut result = None;
    sheet::show(ctx, egui::Id::new("start-replay"), 400.0, |ui| {
        egui::Frame::new()
            .inner_margin(egui::Margin::same(20))
            .show(ui, |ui| {
                ui.set_width(360.0);
                ui.spacing_mut().item_spacing.y = 14.0;
                header(ui);
                let before_preset = state.draft_ms;
                presets(ui, &mut state.draft_ms, range);
                if state.draft_ms != before_preset {
                    (state.display_year, state.display_month, ..) = local_fields(state.draft_ms);
                }
                picker(ui, state, range);
                snap_preview(ui, state.draft_ms, timeline);
                buttons(ui, &mut result, state.draft_ms, range.is_some());
            });
    });
    if result.is_none() && ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        result = Some(ReplayDateSheetResult::Cancel);
    }
    state.open = result.is_none();
    result
}

// MARK: Pieces

fn header(ui: &mut egui::Ui) {
    sheet::header(
        ui,
        Icon::ClockArrow,
        "Start Replay",
        "Pick the moment the chart rewinds to.",
        theme::REPLAY,
    );
}

fn presets(ui: &mut egui::Ui, draft_ms: &mut i64, range: Option<(i64, i64)>) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add_enabled_ui(range.is_some(), |ui| {
            for (label, fraction) in [
                ("Start of data", Some(0.0)),
                ("25%", Some(0.25)),
                ("50%", Some(0.5)),
                ("75%", Some(0.75)),
                ("Random", None),
            ] {
                let button = egui::Button::new(RichText::new(label).size(11.5))
                    .fill(theme::ink(14))
                    .corner_radius(7.0)
                    .stroke(Stroke::NONE);
                if ui.add(button).clicked()
                    && let Some((lo, hi)) = range
                {
                    let span = (hi - lo) as f64;
                    // "Random" leaves the last fifth free, like the bar's Random Bar.
                    let offset = match fraction {
                        Some(f) => span * f,
                        None => span * 0.8 * random_unit(),
                    };
                    *draft_ms = lo + offset as i64;
                }
            }
        });
    });
}

fn picker(ui: &mut egui::Ui, state: &mut ReplayDateSheetState, range: Option<(i64, i64)>) {
    let Some((lo, hi)) = range else {
        ui.label(RichText::new("Load a chart with data first.").weak());
        return;
    };
    egui::Frame::new()
        .fill(theme::surface())
        .stroke(Stroke::new(1.0, theme::separator().gamma_multiply(0.65)))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(336.0);
            ui.spacing_mut().item_spacing.y = 6.0;
            calendar_header(ui, state, lo, hi);
            ui.add_space(7.0);
            calendar_grid(ui, state, lo, hi);
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(8.0);
            time_picker(ui, state, lo, hi);
        });
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn calendar_header(ui: &mut egui::Ui, state: &mut ReplayDateSheetState, lo: i64, hi: i64) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!(
                "{} {}",
                MONTHS[(state.display_month - 1) as usize],
                state.display_year
            ))
            .strong()
            .size(14.0),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let upper = local_fields(hi);
            let lower = local_fields(lo);
            let current = month_index(state.display_year, state.display_month);
            if ui
                .add_enabled(
                    current < month_index(upper.0, upper.1),
                    egui::Button::new("›").frame(false),
                )
                .clicked()
            {
                shift_month(state, 1);
            }
            if ui
                .add_enabled(
                    current > month_index(lower.0, lower.1),
                    egui::Button::new("‹").frame(false),
                )
                .clicked()
            {
                shift_month(state, -1);
            }
        });
    });
}

fn calendar_grid(ui: &mut egui::Ui, state: &mut ReplayDateSheetState, lo: i64, hi: i64) {
    let first_day = days_from_civil(state.display_year, state.display_month, 1);
    let offset = (first_day + 3).rem_euclid(7) as usize; // Monday-first; 1970-01-01 was Thursday.
    let month_days = days_in_month(state.display_year, state.display_month) as usize;
    let selected = local_fields(state.draft_ms);
    egui::Grid::new("replay-calendar-grid")
        .spacing([4.0, 3.0])
        .show(ui, |ui| {
            for weekday in ["M", "T", "W", "T", "F", "S", "S"] {
                ui.add_sized(
                    [43.0, 22.0],
                    egui::Label::new(
                        RichText::new(weekday)
                            .size(11.0)
                            .color(theme::text_secondary()),
                    ),
                );
            }
            ui.end_row();
            for slot in 0..(offset + month_days).div_ceil(7) * 7 {
                if slot >= offset && slot - offset < month_days {
                    let day = (slot - offset + 1) as i64;
                    let (hour, minute) = (selected.3, selected.4);
                    let candidate =
                        from_local(state.display_year, state.display_month, day, hour, minute);
                    let enabled = from_local(state.display_year, state.display_month, day, 23, 59)
                        >= lo
                        && from_local(state.display_year, state.display_month, day, 0, 0) <= hi;
                    let active = selected.0 == state.display_year
                        && selected.1 == state.display_month
                        && selected.2 == day;
                    let button = egui::Button::new(
                        RichText::new(day.to_string()).size(12.0).color(if active {
                            Color32::WHITE
                        } else {
                            theme::text_primary()
                        }),
                    )
                    .fill(if active {
                        theme::REPLAY
                    } else {
                        Color32::TRANSPARENT
                    })
                    .stroke(Stroke::NONE)
                    .corner_radius(7.0);
                    if ui
                        .add_enabled(enabled, button.min_size(Vec2::new(43.0, 29.0)))
                        .clicked()
                    {
                        state.draft_ms = candidate.clamp(lo, hi);
                    }
                } else {
                    ui.allocate_space(Vec2::new(43.0, 29.0));
                }
                if slot % 7 == 6 {
                    ui.end_row();
                }
            }
        });
}

fn time_picker(ui: &mut egui::Ui, state: &mut ReplayDateSheetState, lo: i64, hi: i64) {
    let (year, month, day, mut hour, mut minute, _) = local_fields(state.draft_ms);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Time")
                .size(12.0)
                .color(theme::text_secondary()),
        );
        ui.add_space(8.0);
        let changed_hour = ui
            .add(
                egui::DragValue::new(&mut hour)
                    .range(0..=23)
                    .custom_formatter(|v, _| format!("{:02}", v as i64)),
            )
            .changed();
        ui.label(RichText::new(":").strong());
        let changed_minute = ui
            .add(
                egui::DragValue::new(&mut minute)
                    .range(0..=59)
                    .custom_formatter(|v, _| format!("{:02}", v as i64)),
            )
            .changed();
        if changed_hour || changed_minute {
            state.draft_ms = from_local(year, month, day, hour, minute).clamp(lo, hi);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new("Local time")
                    .size(11.0)
                    .color(theme::text_tertiary()),
            );
        });
    });
}

fn month_index(year: i64, month: i64) -> i64 {
    year * 12 + month - 1
}

fn shift_month(state: &mut ReplayDateSheetState, delta: i64) {
    let index = month_index(state.display_year, state.display_month) + delta;
    state.display_year = index.div_euclid(12);
    state.display_month = index.rem_euclid(12) + 1;
}

fn snap_preview(ui: &mut egui::Ui, draft_ms: i64, timeline: &[i64]) {
    let Some(bar) = snap_to(timeline, draft_ms) else {
        return;
    };
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        scope_icon(ui);
        let local_bar = bar + local_utc_offset_secs(bar) * 1_000;
        ui.label(
            RichText::new(format!(
                "Snaps to the bar at {} {}",
                date_text(local_bar),
                time_text(local_bar)
            ))
            .small()
            .weak(),
        );
    });
}

fn buttons(
    ui: &mut egui::Ui,
    result: &mut Option<ReplayDateSheetResult>,
    draft_ms: i64,
    has_range: bool,
) {
    // A row of its own height: a bare right-to-left layout takes all the height it is offered.
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        Vec2::new(width, 28.0),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            if kit::sheet_button(ui, "Start Replay", true, has_range).clicked() {
                *result = Some(ReplayDateSheetResult::Start(draft_ms));
            }
            if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                *result = Some(ReplayDateSheetResult::Cancel);
            }
        },
    );
}

// MARK: Helpers

/// A small scope glyph, matching the bar's "Choose a new starting point" affordance.
fn scope_icon(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
    let center = rect.center();
    let color = theme::REPLAY;
    let stroke = Stroke::new(1.2, color);
    let painter = ui.painter();
    painter.circle_stroke(center, 5.0, stroke);
    painter.circle_filled(center, 1.4, color);
    for (from, to) in [
        (Vec2::new(0.0, -7.5), Vec2::new(0.0, -3.5)),
        (Vec2::new(0.0, 3.5), Vec2::new(0.0, 7.5)),
        (Vec2::new(-7.5, 0.0), Vec2::new(-3.5, 0.0)),
        (Vec2::new(7.5, 0.0), Vec2::new(3.5, 0.0)),
    ] {
        painter.line_segment([center + from, center + to], Stroke::new(1.2, color));
    }
}

/// Split a UTC epoch-millisecond instant into calendar fields.
fn split_utc(epoch_ms: i64) -> (i64, i64, i64, i64, i64, i64) {
    let seconds = epoch_ms.div_euclid(1_000);
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let rest = seconds.rem_euclid(86_400);
    (year, month, day, rest / 3_600, rest % 3_600 / 60, rest % 60)
}

fn local_fields(epoch_ms: i64) -> (i64, i64, i64, i64, i64, i64) {
    split_utc(epoch_ms + local_utc_offset_secs(epoch_ms) * 1_000)
}

fn from_local(year: i64, month: i64, day: i64, hour: i64, minute: i64) -> i64 {
    let local_ms = from_utc(year, month, day, hour, minute);
    let first = local_ms - local_utc_offset_secs(local_ms) * 1_000;
    local_ms - local_utc_offset_secs(first) * 1_000
}

/// Build a UTC epoch-millisecond instant from calendar fields (seconds forced to zero).
fn from_utc(year: i64, month: i64, day: i64, hour: i64, minute: i64) -> i64 {
    (days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60) * 1_000
}

fn days_in_month(year: i64, month: i64) -> i64 {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    days_from_civil(next_year, next_month, 1) - days_from_civil(year, month, 1)
}

/// The `[min, max]` span the chart covers, if it has any bars.
fn range_of(timeline: &[i64]) -> Option<(i64, i64)> {
    Some((*timeline.iter().min()?, *timeline.iter().max()?))
}

/// The bar a given instant falls on: the latest bar whose open time is at or before it.
fn snap_to(timeline: &[i64], ms: i64) -> Option<i64> {
    let index = timeline
        .partition_point(|time| *time <= ms)
        .checked_sub(1)?;
    timeline.get(index).copied()
}

fn random_unit() -> f64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    (RandomState::new().build_hasher().finish() as f64) / (u64::MAX as f64)
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_fields_round_trip() {
        let ms = 1_700_000_000_000; // 2023-11-14 22:13:20 UTC
        assert_eq!(split_utc(ms), (2023, 11, 14, 22, 13, 20));
        // Seconds are dropped on the way back in.
        assert_eq!(from_utc(2023, 11, 14, 22, 13), 1_699_999_980_000);
    }

    #[test]
    fn month_lengths_follow_the_leap_year() {
        assert_eq!(days_in_month(2023, 2), 28);
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2024, 12), 31);
        assert_eq!(days_in_month(2024, 11), 30);
    }

    #[test]
    fn snap_lands_on_the_bar_at_or_before() {
        let timeline = vec![1_000, 1_100, 1_200, 1_300, 1_400];
        assert_eq!(snap_to(&timeline, 1_150), Some(1_100));
        assert_eq!(snap_to(&timeline, 1_200), Some(1_200));
        assert_eq!(snap_to(&timeline, 999), None);
        assert_eq!(snap_to(&timeline, 1_500), Some(1_400));
    }

    #[test]
    fn range_and_open_seed() {
        assert_eq!(range_of(&[]), None);
        assert_eq!(range_of(&[3_000, 1_000, 2_000]), Some((1_000, 3_000)));
        let mut state = ReplayDateSheetState::default();
        assert!(!state.is_open());
        state.open(&[1_000, 2_000, 3_000]);
        assert!(state.is_open());
        assert_eq!(state.draft_ms, 2_000);
    }

    #[test]
    fn calendar_month_navigation_crosses_year_boundaries() {
        let mut state = ReplayDateSheetState {
            display_year: 2026,
            display_month: 1,
            ..Default::default()
        };
        shift_month(&mut state, -1);
        assert_eq!((state.display_year, state.display_month), (2025, 12));
        shift_month(&mut state, 2);
        assert_eq!((state.display_year, state.display_month), (2026, 2));
    }

    /// The sheet is as tall as its content, not the window (a centred row inside a modal fills
    /// the height it is given).
    #[test]
    fn the_sheet_is_no_taller_than_its_content() {
        use crate::ui::render_tests::Screen;
        let timeline: Vec<i64> = (0..400)
            .map(|i| 1_700_000_000_000 + i * 3_600_000)
            .collect();
        let mut state = ReplayDateSheetState::default();
        state.open(&timeline);
        let mut screen = Screen::new(Vec2::new(900.0, 1_400.0));
        let mut last = None;
        for _ in 0..3 {
            last = Some(screen.run(vec![], |ui| {
                show(ui.ctx(), &mut state, &timeline);
            }));
        }
        let rect = screen
            .ctx
            .memory(|memory| memory.area_rect(egui::Id::new("start-replay")))
            .expect("the sheet is on screen");
        assert!(
            rect.height() < 700.0,
            "sheet is {} tall on a 1400pt screen",
            rect.height()
        );
        screen.save("replay-date-sheet", last.unwrap());
    }
}
