//! Session statistics: "how often did this happen on the days this chart covers?", answered with
//! its sample size and a 95% interval.
//!
//! A docked panel over `model::session_stats`: a question, the conditions that narrow the days,
//! and the answer. It owns the question and the conditions (card state, never saved, in no
//! template) and a cached answer, recomputed only when the bars or the question change, never
//! per frame. It reads the bars the chart draws and writes nothing back.

use crate::format::group_thousands;
use crate::model::Kline;
use crate::model::session_stats::{
    self, Estimate, Filter, MarketHours, Outcome, Side, Stats, Unavailable, WeekdaySet,
};
use crate::ui::icons::{self, Icon};
use crate::ui::{kit, theme};
use eframe::egui::{self, Align, Layout, Sense, Ui, vec2};

const TOOLBAR_HEIGHT: f32 = 44.0;
const MARGIN: f32 = 16.0;
const ROW_HEIGHT: f32 = 36.0;
/// Fewer qualifying days than this and the panel says the interval is wide.
const SMALL_SAMPLE: usize = 30;

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
    answer: Result<Stats, Unavailable>,
}

pub struct StatsViewState {
    pub outcome: Outcome,
    pub filter: Filter,
    gap_min: String,
    gap_max: String,
    cached: Option<Cached>,
}

impl Default for StatsViewState {
    fn default() -> Self {
        Self {
            outcome: Outcome::GapFill,
            filter: Filter::default(),
            gap_min: String::new(),
            gap_max: String::new(),
            cached: None,
        }
    }
}

/// A percent typed into a field; empty or not a number is "no limit".
fn parse_percent(text: &str) -> Option<f64> {
    text.trim()
        .trim_end_matches('%')
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0)
}

impl StatsViewState {
    /// The answer for `input` and the current question, worked again only when something changed.
    fn answer(&mut self, input: &Input) -> &Result<Stats, Unavailable> {
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
                answer,
            });
        }
        &self.cached.as_ref().expect("just filled").answer
    }
}

// MARK: Text

fn percent(value: f64) -> String {
    format!("{:.1}%", value * 100.0)
}

fn count(value: usize) -> String {
    group_thousands(&value.to_string())
}

fn estimate_line(estimate: &Estimate) -> String {
    match estimate.proportion() {
        Some(p) => format!(
            "{} ({} of {})",
            percent(p),
            count(estimate.hits),
            count(estimate.n)
        ),
        None => "no days".to_owned(),
    }
}

/// How long after the day's first bar opened a bar began, for an hourly chart.
fn after_open(index: usize, bar_ms: i64) -> String {
    if index == 0 {
        return "in the first bar".to_owned();
    }
    let hours = |bars: usize| bars as f64 * bar_ms as f64 / 3_600_000.0;
    format!("bar {index} ({}–{} h in)", hours(index), hours(index + 1))
}

fn date(date: (i64, i64, i64)) -> String {
    format!("{:04}-{:02}-{:02}", date.0, date.1, date.2)
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
    toolbar(ui, title, &mut action);
    divider(ui);
    egui::ScrollArea::vertical()
        .id_salt("session-stats")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(MARGIN as i8, 12))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                    question(ui, state);
                    conditions(ui, state);
                    ui.add_space(4.0);
                    let Some(input) = input else {
                        quiet(ui, "There are no bars to count yet.");
                        return;
                    };
                    let bar_ms = input.bar_ms;
                    match state.answer(&input).clone() {
                        Ok(stats) => result(ui, &stats, bar_ms),
                        Err(reason) => quiet(ui, reason.text()),
                    }
                });
        });
    action
}

fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, kit::hairline());
}

fn toolbar(ui: &mut Ui, title: &str, action: &mut Option<Action>) {
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
                ui.add_space(2.0);
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

fn caption(ui: &mut Ui, label: &str) {
    ui.label(kit::text(
        label,
        kit::size::SUBHEADLINE,
        kit::MEDIUM,
        theme::text_secondary(),
    ));
}

fn quiet(ui: &mut Ui, message: &str) {
    let galley = kit::paragraph(
        ui,
        message,
        kit::size::SUBHEADLINE,
        kit::REGULAR,
        theme::text_tertiary(),
        ui.available_width(),
    );
    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
    ui.painter()
        .galley(rect.min, galley, theme::text_tertiary());
}

// MARK: Question and conditions

fn question(ui: &mut Ui, state: &mut StatsViewState) {
    caption(ui, "Question");
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
}

fn side_picker(ui: &mut Ui, id: &str, side: &mut Side) {
    kit::segmented(
        ui,
        egui::Id::new(id),
        &[(Side::Any, "Any"), (Side::Up, "Up"), (Side::Down, "Down")],
        side,
    );
}

/// A row whose label and control share a centre line, however tall the control is.
fn centered_row(ui: &mut Ui, contents: impl FnOnce(&mut Ui)) {
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        vec2(width, ROW_HEIGHT),
        Layout::left_to_right(Align::Center),
        contents,
    );
}

fn percent_field(ui: &mut Ui, id: &str, text: &mut String, hint: &str) {
    let focused = ui.memory(|memory| memory.has_focus(egui::Id::new(id)));
    kit::field_frame(focused).show(ui, |ui| {
        kit::plain_field(
            ui,
            egui::Id::new(id),
            text,
            hint,
            kit::size::BODY,
            kit::REGULAR,
            64.0,
        );
    });
}

fn conditions(ui: &mut Ui, state: &mut StatsViewState) {
    caption(ui, "On days that");
    // Weekdays: a chip each, all on meaning no condition.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        for (day, name) in WeekdaySet::NAMES.iter().enumerate() {
            let on = state.filter.weekdays.contains(day);
            if kit::value_chip(ui, name, on).clicked() {
                state.filter.weekdays = state.filter.weekdays.with(day, !on);
            }
        }
    });
    if state.filter.weekdays.is_empty() {
        quiet(ui, "No weekday is on, so no day qualifies.");
    }

    centered_row(ui, |ui| {
        ui.label(kit::text(
            "Gap",
            kit::size::BODY,
            kit::REGULAR,
            theme::text_primary(),
        ));
        side_picker(ui, "session-stats-gap", &mut state.filter.gap);
    });
    centered_row(ui, |ui| {
        ui.label(kit::text(
            "Gap size",
            kit::size::BODY,
            kit::REGULAR,
            theme::text_primary(),
        ));
        percent_field(ui, "session-stats-gap-min", &mut state.gap_min, "min %");
        ui.label(kit::text(
            "to",
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_tertiary(),
        ));
        percent_field(ui, "session-stats-gap-max", &mut state.gap_max, "max %");
    });
    state.filter.gap_min_pct = parse_percent(&state.gap_min);
    state.filter.gap_max_pct = parse_percent(&state.gap_max);
    centered_row(ui, |ui| {
        ui.label(kit::text(
            "Prior day",
            kit::size::BODY,
            kit::REGULAR,
            theme::text_primary(),
        ));
        side_picker(ui, "session-stats-prior", &mut state.filter.prior_day);
    });
    if state.filter.is_narrowed() && ui.link_like("Clear conditions").clicked() {
        state.filter = Filter::default();
        state.gap_min.clear();
        state.gap_max.clear();
    }
}

/// A gray underlined text button: a link in this app is never blue.
trait LinkLike {
    fn link_like(&mut self, label: &str) -> egui::Response;
}

impl LinkLike for Ui {
    fn link_like(&mut self, label: &str) -> egui::Response {
        let galley = kit::galley(
            self,
            label,
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_secondary(),
            f32::INFINITY,
        );
        let (rect, response) = self.allocate_exact_size(galley.size(), Sense::click());
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        let color = theme::text_secondary();
        self.painter().galley(rect.min, galley, color);
        let y = rect.bottom() - 1.0;
        self.painter()
            .hline(rect.left()..=rect.right(), y, egui::Stroke::new(1.0, color));
        response
    }
}

// MARK: Result

fn result(ui: &mut Ui, stats: &Stats, bar_ms: i64) {
    kit::card_frame(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        let question = format!(
            "{} on {}",
            stats.outcome.question(),
            stats.filter.describe()
        );
        let galley = kit::paragraph(
            ui,
            &question,
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_secondary(),
            ui.available_width(),
        );
        let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
        ui.painter()
            .galley(rect.min, galley, theme::text_secondary());

        let Some(proportion) = stats.estimate.proportion() else {
            ui.label(kit::text(
                "No days qualify",
                kit::size::TITLE2,
                kit::SEMIBOLD,
                theme::text_primary(),
            ));
            for note in &stats.notes {
                quiet(ui, note.text());
            }
            return;
        };
        ui.label(kit::text(
            percent(proportion),
            28.0,
            kit::SEMIBOLD,
            theme::text_primary(),
        ));
        let (low, high) = stats.estimate.interval().unwrap_or((0.0, 1.0));
        line(
            ui,
            &format!(
                "N = {} days · {} hits · 95% CI {} – {}",
                count(stats.estimate.n),
                count(stats.estimate.hits),
                percent(low),
                percent(high),
            ),
        );
        if stats.estimate.n < SMALL_SAMPLE {
            quiet(
                ui,
                "A small sample: the interval is wide, and one more day would move the number.",
            );
        }
        if let Some(halves) = &stats.stability {
            line(
                ui,
                &format!(
                    "Older half {} · newer half {} · {}",
                    estimate_line(&halves.older),
                    estimate_line(&halves.newer),
                    if halves.agree {
                        "the halves agree"
                    } else {
                        "the halves differ"
                    },
                ),
            );
        }
        if let Some(timing) = &stats.timing {
            line(
                ui,
                &format!(
                    "When it happened (n = {}): median {} · 25% {} · 75% {} · 90% {}",
                    count(timing.n),
                    after_open(timing.median, bar_ms),
                    after_open(timing.p25, bar_ms),
                    after_open(timing.p75, bar_ms),
                    after_open(timing.p90, bar_ms),
                ),
            );
        }
        if !stats.per_year.is_empty() {
            ui.add_space(4.0);
            caption(ui, "Per year");
            for row in &stats.per_year {
                line(
                    ui,
                    &format!("{}   {}", row.year, estimate_line(&row.estimate)),
                );
            }
        }
        for note in &stats.notes {
            quiet(ui, note.text());
        }
        if let Some((first, last)) = stats.span {
            ui.add_space(4.0);
            quiet(
                ui,
                &format!(
                    "{} sessions, {} to {} · {}",
                    count(stats.sessions),
                    date(first),
                    date(last),
                    stats.hours.label(),
                ),
            );
        }
    });
    ui.add_space(4.0);
    quiet(
        ui,
        "Historical frequencies of this chart's bars, with sample sizes. Not predictions, not advice.",
    );
}

fn line(ui: &mut Ui, text: &str) {
    let galley = kit::paragraph(
        ui,
        text,
        kit::size::SUBHEADLINE,
        kit::REGULAR,
        theme::text_primary(),
        ui.available_width(),
    );
    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
    ui.painter().galley(rect.min, galley, theme::text_primary());
}

#[cfg(test)]
mod tests;
