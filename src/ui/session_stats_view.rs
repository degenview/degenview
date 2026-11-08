//! Session statistics: "how often did this happen on the days this chart covers?", answered with
//! its sample size and a 95% interval.
//!
//! A docked panel over `model::session_stats`: a header (what it is of, copy, close), the question
//! with a line on when a day counts, the conditions that narrow the days, and the answer as cards.
//! It owns the question and the conditions (card state, never saved, in no template) and a cached
//! answer, recomputed only when the bars, the question or a condition change, never per frame. It
//! reads the bars the chart draws and writes nothing back; the only thing it puts anywhere is the
//! text of the answer on the clipboard when asked.
//!
//! The words and numbers are `text`, the painted parts `parts`, the answer cards `result`.

mod parts;
mod result;
mod text;

use crate::model::Kline;
use crate::model::session_stats::{
    self, Filter, MarketHours, Outcome, Side, Stats, Unavailable, WeekdaySet,
};
use crate::ui::icons::{self, Icon};
use crate::ui::{accessibility, kit, theme};
use eframe::egui::{
    self, Align, Layout, Rect, Sense, Stroke, Ui, WidgetInfo, WidgetType, pos2, vec2,
};
use parts::{centered_row, link, paragraph, section_header, section_label};
use std::sync::Arc;
use std::time::Duration;

const TOOLBAR_HEIGHT: f32 = 44.0;
const MARGIN: f32 = 16.0;
const CONTROL_HEIGHT: f32 = 32.0;
/// The column the control rows keep their labels in.
const LABEL_WIDTH: f32 = 72.0;
const DAY_HEIGHT: f32 = 28.0;
/// How long the copy button says "copied".
const COPIED_FOR: f64 = 1.6;

const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// What the panel asks for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Close,
}

/// The bars the panel reads, as the chart draws them.
#[derive(Clone, Copy)]
pub struct Input<'a> {
    pub chart_id: u64,
    pub bars: &'a [Kline],
    pub bar_ms: i64,
    pub hours: MarketHours,
    /// Whether the chart is in a replay, so its bars end at the replay position.
    pub replay: bool,
}

/// What identifies an answer: when none of it changed, the cached answer stands.
#[derive(Clone, Debug, PartialEq)]
struct Fingerprint {
    chart_id: u64,
    len: usize,
    first_open: i64,
    last_open: i64,
    last_close_bits: u64,
    bar_ms: i64,
    hours: MarketHours,
    outcome: Outcome,
    filter: Filter,
}

struct Cached {
    fingerprint: Fingerprint,
    answer: Arc<Result<Stats, Unavailable>>,
}

pub struct StatsViewState {
    pub outcome: Outcome,
    pub filter: Filter,
    gap_min: String,
    gap_max: String,
    cached: Option<Cached>,
    /// When the answer was last copied, for the button's "copied" mark.
    copied_at: Option<f64>,
}

impl Default for StatsViewState {
    fn default() -> Self {
        Self {
            outcome: Outcome::GapFill,
            filter: Filter::default(),
            gap_min: String::new(),
            gap_max: String::new(),
            cached: None,
            copied_at: None,
        }
    }
}

/// A percent typed into a field; empty or not a number is "no limit". A comma is a decimal point.
fn parse_percent(text: &str) -> Option<f64> {
    text.trim()
        .trim_end_matches('%')
        .trim()
        .replace(',', ".")
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0)
}

impl StatsViewState {
    /// The answer for `input` and the current question, worked again only when something changed.
    fn answer(&mut self, input: &Input) -> Arc<Result<Stats, Unavailable>> {
        let fingerprint = Fingerprint {
            chart_id: input.chart_id,
            len: input.bars.len(),
            first_open: input.bars.first().map_or(0, |bar| bar.open_time),
            last_open: input.bars.last().map_or(0, |bar| bar.open_time),
            last_close_bits: input.bars.last().map_or(0, |bar| bar.close.to_bits()),
            bar_ms: input.bar_ms,
            hours: input.hours,
            outcome: self.outcome,
            filter: self.filter,
        };
        if self
            .cached
            .as_ref()
            .is_none_or(|cached| cached.fingerprint != fingerprint)
        {
            let answer = session_stats::compute(
                input.bars,
                input.bar_ms,
                input.hours,
                self.outcome,
                &self.filter,
            );
            self.cached = Some(Cached {
                fingerprint,
                answer: Arc::new(answer),
            });
        }
        Arc::clone(&self.cached.as_ref().expect("just filled").answer)
    }

    fn clear_conditions(&mut self) {
        self.filter = Filter::default();
        self.gap_min.clear();
        self.gap_max.clear();
    }

    /// Whether a typed gap limit is not a number, so the field says so.
    fn gap_text_invalid(text: &str) -> bool {
        !text.trim().is_empty() && parse_percent(text).is_none()
    }
}

// MARK: Panel

pub fn show(
    ui: &mut Ui,
    state: &mut StatsViewState,
    title: &str,
    input: Option<Input>,
) -> Option<Action> {
    let mut action = None;
    // The content fills the panel however short it is.
    ui.set_min_height(ui.available_height());
    let answer = input.as_ref().map(|input| state.answer(input));
    let bar_ms = input.as_ref().map_or(0, |input| input.bar_ms);
    let copyable = match answer.as_deref() {
        Some(Ok(stats)) => Some(stats),
        _ => None,
    };
    toolbar(ui, state, title, copyable, bar_ms, &mut action);
    divider(ui);
    egui::ScrollArea::vertical()
        .id_salt("session-stats")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: MARGIN as i8,
                    right: MARGIN as i8,
                    top: 14,
                    bottom: 22,
                })
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                    question(ui, state);
                    ui.add_space(4.0);
                    conditions(ui, state);
                    ui.add_space(6.0);
                    let setting = result::Setting {
                        bar_ms,
                        replay: input.as_ref().is_some_and(|input| input.replay),
                        narrowed: state.filter.is_narrowed(),
                    };
                    if result::show(ui, answer.as_deref(), &setting)
                        == Some(result::Request::ClearConditions)
                    {
                        state.clear_conditions();
                    }
                });
        });
    action
}

fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, kit::hairline());
}

fn toolbar(
    ui: &mut Ui,
    state: &mut StatsViewState,
    title: &str,
    copyable: Option<&Stats>,
    bar_ms: i64,
    action: &mut Option<Action>,
) {
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        vec2(width, TOOLBAR_HEIGHT),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
            ui.add_space(MARGIN);
            ui.label(kit::text(
                "Session statistics",
                kit::size::HEADLINE,
                kit::SEMIBOLD,
                theme::text_primary(),
            ));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                let close = icons::button(
                    ui,
                    Icon::Xmark,
                    26.0,
                    theme::text_secondary(),
                    "Close session statistics",
                );
                if close.clicked() {
                    *action = Some(Action::Close);
                }
                if let Some(stats) = copyable {
                    copy_button(ui, state, title, stats, bar_ms);
                }
                ui.add_space(2.0);
                // What it is of, quietly, in the space between.
                let room = (ui.available_width() - 8.0).max(0.0);
                let galley = kit::galley(
                    ui,
                    title,
                    kit::size::SUBHEADLINE,
                    kit::REGULAR,
                    theme::text_tertiary(),
                    room,
                );
                let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
                ui.painter()
                    .galley(rect.min, galley, theme::text_tertiary());
            });
        },
    );
}

/// Copies the answer as text, and says so with a tick for a moment.
fn copy_button(ui: &mut Ui, state: &mut StatsViewState, title: &str, stats: &Stats, bar_ms: i64) {
    let now = ui.input(|input| input.time);
    let copied = state
        .copied_at
        .is_some_and(|at| now >= at && now - at < COPIED_FOR);
    if copied {
        ui.ctx()
            .request_repaint_after(Duration::from_secs_f64(COPIED_FOR));
    } else {
        state.copied_at = None;
    }
    let (icon, tip) = if copied {
        (Icon::Check, "Copied")
    } else {
        (Icon::CopyDoc, "Copy the answer")
    };
    if icons::button(ui, icon, 26.0, theme::text_secondary(), tip).clicked() {
        ui.ctx().copy_text(text::summary(title, stats, bar_ms));
        state.copied_at = Some(now);
    }
}

// MARK: Question

fn question(ui: &mut Ui, state: &mut StatsViewState) {
    section_label(ui, "Question");
    let labels: Vec<String> = Outcome::ALL
        .iter()
        .map(|outcome| outcome.label().to_owned())
        .collect();
    let selected = Outcome::ALL
        .iter()
        .position(|outcome| *outcome == state.outcome)
        .unwrap_or(0);
    let width = ui.available_width();
    if let Some(chosen) = kit::menu_picker(ui, width, &labels, selected, true) {
        state.outcome = Outcome::ALL[chosen];
    }
    paragraph(
        ui,
        state.outcome.hit_when(),
        kit::size::SUBHEADLINE,
        kit::REGULAR,
        theme::text_tertiary(),
    );
}

// MARK: Conditions

fn conditions(ui: &mut Ui, state: &mut StatsViewState) {
    let active = state.filter.active_count();
    section_header(ui, "Only count days that…", |ui| {
        if active > 0 && link(ui, "Clear").clicked() {
            state.clear_conditions();
        }
    });
    weekday_strip(ui, &mut state.filter.weekdays);
    if state.filter.weekdays.is_empty() {
        paragraph(
            ui,
            "No weekday is on, so no day qualifies.",
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_tertiary(),
        );
    }

    control_row(ui, "Gap", |ui, width| {
        kit::segmented_filling(
            ui,
            egui::Id::new("session-stats-gap"),
            &[(Side::Any, "Any"), (Side::Up, "Up"), (Side::Down, "Down")],
            &mut state.filter.gap,
            width,
        );
    });
    control_row(ui, "Gap size", |ui, width| {
        // The dash and the spacing round it come out of the width before it is shared.
        let dash = kit::galley(
            ui,
            "–",
            kit::size::BODY,
            kit::REGULAR,
            theme::text_tertiary(),
            f32::INFINITY,
        );
        let join = dash.size().x + ui.spacing().item_spacing.x * 2.0;
        let field = ((width - join) / 2.0 - 0.5).max(40.0);
        percent_field(
            ui,
            "session-stats-gap-min",
            &mut state.gap_min,
            "min",
            field,
        );
        ui.label(kit::text(
            "–",
            kit::size::BODY,
            kit::REGULAR,
            theme::text_tertiary(),
        ));
        percent_field(
            ui,
            "session-stats-gap-max",
            &mut state.gap_max,
            "max",
            field,
        );
    });
    state.filter.gap_min_pct = parse_percent(&state.gap_min);
    state.filter.gap_max_pct = parse_percent(&state.gap_max);
    if let (Some(least), Some(most)) = (state.filter.gap_min_pct, state.filter.gap_max_pct)
        && least > most
    {
        paragraph(
            ui,
            "The smallest gap is above the largest, so no gap qualifies.",
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            kit::Tone::Warning.text(),
        );
    }
    control_row(ui, "Prior day", |ui, width| {
        kit::segmented_filling(
            ui,
            egui::Id::new("session-stats-prior"),
            &[(Side::Any, "Any"), (Side::Up, "Up"), (Side::Down, "Down")],
            &mut state.filter.prior_day,
            width,
        );
    });
}

/// A label in a fixed column and a control filling the rest of the line.
fn control_row(ui: &mut Ui, label: &str, contents: impl FnOnce(&mut Ui, f32)) {
    centered_row(ui, CONTROL_HEIGHT, |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (rect, _) = ui.allocate_exact_size(vec2(LABEL_WIDTH, CONTROL_HEIGHT), Sense::hover());
        let galley = kit::galley(
            ui,
            label,
            kit::size::BODY,
            kit::REGULAR,
            theme::text_primary(),
            LABEL_WIDTH,
        );
        ui.painter().galley(
            pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
            galley,
            theme::text_primary(),
        );
        let width = ui.available_width();
        contents(ui, width);
    });
}

/// Seven equal toggles, Monday to Sunday, across the panel however wide it is.
fn weekday_strip(ui: &mut Ui, set: &mut WeekdaySet) {
    const GAP: f32 = 4.0;
    let width = ui.available_width();
    let cell = (width - GAP * 6.0) / 7.0;
    let (strip, _) = ui.allocate_exact_size(vec2(width, DAY_HEIGHT), Sense::hover());
    for (day, name) in WEEKDAYS.iter().enumerate() {
        let rect = Rect::from_min_size(
            pos2(strip.left() + day as f32 * (cell + GAP), strip.top()),
            vec2(cell, DAY_HEIGHT),
        );
        let on = set.contains(day);
        let response = ui
            .interact(
                rect,
                ui.id().with(("session-stats-day", day)),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(*name);
        response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, true, on, name));
        let painter = ui.painter();
        let fill = if on {
            theme::selected_tint(response.hovered())
        } else if response.hovered() {
            kit::primary(0.1)
        } else {
            kit::primary(0.06)
        };
        painter.rect_filled(rect, 7.0, fill);
        if on {
            painter.rect_stroke(
                rect,
                7.0,
                theme::selected_stroke(),
                egui::StrokeKind::Inside,
            );
        }
        let (weight, color) = if on {
            (kit::MEDIUM, theme::text_primary())
        } else {
            (kit::REGULAR, theme::text_tertiary())
        };
        let galley = kit::galley(
            ui,
            WeekdaySet::NAMES[day],
            kit::size::SUBHEADLINE,
            weight,
            color,
            cell,
        );
        ui.painter()
            .galley(rect.center() - galley.size() / 2.0, galley, color);
        if response.clicked() {
            *set = set.with(day, !on);
        }
    }
}

/// A percent field with its unit; a red outline and a tooltip when the text is not a number.
fn percent_field(ui: &mut Ui, id: &str, value: &mut String, hint: &str, width: f32) {
    let field_id = egui::Id::new(id);
    let focused = ui.memory(|memory| memory.has_focus(field_id));
    let invalid = StatsViewState::gap_text_invalid(value);
    let mut frame = kit::field_frame(focused).inner_margin(egui::Margin::symmetric(9, 5));
    if invalid {
        frame = frame.stroke(Stroke::new(1.0, kit::Tone::Bad.dot()));
    }
    frame.show(ui, |ui| {
        let inner = (width - 20.0).max(20.0);
        ui.set_width(inner);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let response = kit::plain_field(
                ui,
                field_id,
                value,
                hint,
                kit::size::BODY,
                kit::REGULAR,
                (inner - 16.0).max(10.0),
            );
            accessibility::label(response.clone(), format!("Gap {hint} percent"));
            if invalid {
                response.on_hover_text("Enter a percent, for example 0.5");
            }
            ui.label(kit::text(
                "%",
                kit::size::BODY,
                kit::REGULAR,
                theme::text_tertiary(),
            ));
        });
    });
}

#[cfg(test)]
mod tests;
