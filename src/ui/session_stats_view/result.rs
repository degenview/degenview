//! The answer half of the Session statistics panel: the estimate and its interval, the older and
//! newer half, when in the day it happened, a row per year, and what the reader should know about
//! the bars behind it. Each is a card; none reads anything but the `Stats` it is given.

use super::parts::{
    callout, interval_bar, key_value, link, paragraph, section_header, share_row, text_width,
};
use super::text;
use crate::model::session_stats::{MarketHours, Note, Stats, Unavailable};
use crate::ui::icons::{self, Icon};
use crate::ui::{kit, theme};
use eframe::egui::{Sense, Ui, pos2, vec2};

/// Fewer qualifying days than this and the answer says the interval is wide.
pub const SMALL_SAMPLE: usize = 30;
const HERO_SIZE: f32 = 34.0;
const HERO_HEIGHT: f32 = 44.0;
const HALF_LABEL: f32 = 44.0;
const YEAR_LABEL: f32 = 36.0;

/// What the answer asks the panel to do.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Request {
    ClearConditions,
}

/// What the answer is of.
pub struct Setting {
    /// The bar length, to turn a bar index into hours.
    pub bar_ms: i64,
    /// Whether the chart is in a replay, so its bars stop at the replay position.
    pub replay: bool,
    /// Whether any condition is set, so an empty answer can offer to clear them.
    pub narrowed: bool,
}

fn card(ui: &mut Ui, contents: impl FnOnce(&mut Ui)) {
    kit::card_frame(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing = vec2(0.0, 4.0);
        contents(ui);
    });
}

/// The answer, or why there is none.
pub fn show(
    ui: &mut Ui,
    answer: Option<&Result<Stats, Unavailable>>,
    setting: &Setting,
) -> Option<Request> {
    match answer {
        None => {
            callout(
                ui,
                Icon::Clock,
                theme::text_secondary(),
                "Waiting for bars",
                "The chart has no bars yet. The answer appears once it has loaded some.",
            );
            None
        }
        Some(Err(reason)) => {
            let (icon, title) = match reason {
                Unavailable::NoBars => (Icon::Clock, "Nothing to count yet"),
                Unavailable::TooCoarse | Unavailable::NeedsIntraday => {
                    (Icon::Calendar, "Not on this timeframe")
                }
            };
            callout(ui, icon, kit::Tone::Warning.dot(), title, reason.text());
            None
        }
        Some(Ok(stats)) => answer_cards(ui, stats, setting),
    }
}

fn answer_cards(ui: &mut Ui, stats: &Stats, setting: &Setting) -> Option<Request> {
    let mut request = None;
    if stats.estimate.n == 0 {
        request = empty(ui, stats, setting);
    } else {
        estimate_card(ui, stats);
        if let Some(halves) = &stats.stability {
            stability_card(ui, halves);
        }
        if let Some(timing) = &stats.timing {
            timing_card(ui, timing, setting.bar_ms);
        }
        if !stats.per_year.is_empty() {
            years_card(ui, stats);
        }
    }
    for note in &stats.notes {
        if stats.estimate.n == 0 && *note == Note::NoGaps {
            // The empty card already says it.
            continue;
        }
        let tint = match note {
            Note::BoundaryOpen => kit::Tone::Warning.dot(),
            Note::NoGaps => theme::text_secondary(),
        };
        callout(ui, Icon::Triangle, tint, note.title(), note.text());
    }
    footer(ui, stats, setting);
    request
}

fn estimate_card(ui: &mut Ui, stats: &Stats) {
    card(ui, |ui| {
        let sentence = format!(
            "{} on {}",
            stats.outcome.question(),
            stats.filter.describe()
        );
        paragraph(
            ui,
            &sentence,
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_secondary(),
        );
        let estimate = &stats.estimate;
        let width = ui.available_width();
        let (hero, _) = ui.allocate_exact_size(vec2(width, HERO_HEIGHT), Sense::hover());
        let share = estimate.proportion().unwrap_or(0.0);
        let galley = kit::digits(
            ui,
            &text::percent(share),
            HERO_SIZE,
            kit::SEMIBOLD,
            theme::text_primary(),
            width,
        );
        ui.painter().galley(
            pos2(hero.left(), hero.center().y - galley.size().y / 2.0),
            galley,
            theme::text_primary(),
        );
        if estimate.n < SMALL_SAMPLE {
            let badge = kit::paint_status_badge(
                ui,
                hero.right(),
                hero.center().y,
                "Small sample",
                kit::Tone::Warning,
            );
            ui.interact(badge, ui.id().with("small-sample"), Sense::hover())
                .on_hover_text(format!(
                    "Fewer than {SMALL_SAMPLE} qualifying days: the interval is wide, and one \
                     more day would move the number."
                ));
        }
        interval_bar(ui, estimate);
        ui.add_space(4.0);
        key_value(ui, "Days counted", &text::count(estimate.n));
        key_value(ui, "Days it happened", &text::count(estimate.hits));
        if let Some((low, high)) = estimate.interval() {
            key_value(
                ui,
                "95% interval",
                &format!("{} – {}", text::percent(low), text::percent(high)),
            );
        }
    });
}

fn stability_card(ui: &mut Ui, halves: &crate::model::session_stats::Stability) {
    card(ui, |ui| {
        section_header(ui, "Older and newer half", |ui| {
            let (label, tone, tip) = if halves.agree {
                (
                    "Halves agree",
                    kit::Tone::Good,
                    "The two halves' 95% intervals overlap.",
                )
            } else {
                (
                    "Halves differ",
                    kit::Tone::Warning,
                    "The two halves' 95% intervals do not overlap: the pattern may have changed.",
                )
            };
            let (rect, _) =
                ui.allocate_exact_size(kit::status_badge_size(ui, label), Sense::hover());
            let badge = kit::paint_status_badge(ui, rect.right(), rect.center().y, label, tone);
            ui.interact(badge, ui.id().with("halves"), Sense::hover())
                .on_hover_text(tip);
        });
        let texts = [text::share(&halves.older), text::share(&halves.newer)];
        let column = text_width(ui, &texts);
        share_row(ui, "Older", HALF_LABEL, column, &halves.older, &texts[0]);
        share_row(ui, "Newer", HALF_LABEL, column, &halves.newer, &texts[1]);
    });
}

fn timing_card(ui: &mut Ui, timing: &crate::model::session_stats::Timing, bar_ms: i64) {
    card(ui, |ui| {
        section_header(ui, "When it happens", |ui| {
            ui.label(kit::text(
                format!("{} days", text::count(timing.n)),
                kit::size::SUBHEADLINE,
                kit::REGULAR,
                theme::text_tertiary(),
            ));
        });
        for (label, index) in [
            ("A quarter by", timing.p25),
            ("Half by", timing.median),
            ("Three quarters by", timing.p75),
            ("Nine in ten by", timing.p90),
        ] {
            key_value(ui, label, &text::after_open(index, bar_ms));
        }
    });
}

fn years_card(ui: &mut Ui, stats: &Stats) {
    card(ui, |ui| {
        section_header(ui, "By year", |_| {});
        let texts: Vec<String> = stats
            .per_year
            .iter()
            .map(|row| text::share(&row.estimate))
            .collect();
        let column = text_width(ui, &texts);
        for (row, text) in stats.per_year.iter().zip(&texts) {
            share_row(
                ui,
                &row.year.to_string(),
                YEAR_LABEL,
                column,
                &row.estimate,
                text,
            );
        }
    });
}

/// Nothing qualified: say so, and what to do about it.
fn empty(ui: &mut Ui, stats: &Stats, setting: &Setting) -> Option<Request> {
    let mut request = None;
    card(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(10.0);
            icons::inline(ui, Icon::CircleDashed, 22.0, theme::text_tertiary());
            ui.add_space(6.0);
            ui.label(kit::text(
                "No days qualify",
                kit::size::HEADLINE + 1.0,
                kit::SEMIBOLD,
                theme::text_primary(),
            ));
            let detail = if stats.notes.contains(&Note::NoGaps) {
                Note::NoGaps.text()
            } else if setting.narrowed {
                "No day in these bars matches every condition. Loosen one, or load more history."
            } else {
                "No day in these bars can answer this question."
            };
            paragraph(
                ui,
                detail,
                kit::size::SUBHEADLINE,
                kit::REGULAR,
                theme::text_secondary(),
            );
            if setting.narrowed {
                ui.add_space(4.0);
                if link(ui, "Clear conditions").clicked() {
                    request = Some(Request::ClearConditions);
                }
            }
            ui.add_space(8.0);
        });
    });
    request
}

fn footer(ui: &mut Ui, stats: &Stats, setting: &Setting) {
    ui.add_space(2.0);
    let mut lines = Vec::new();
    if let Some((first, last)) = stats.span {
        lines.push(format!(
            "{} sessions · {} to {}",
            text::count(stats.sessions),
            text::date(first),
            text::date(last)
        ));
    }
    lines.push(match stats.hours {
        MarketHours::UsStock => "A day is the regular session in New York.".to_owned(),
        MarketHours::Continuous => {
            "A day is a UTC date: this market trades round the clock.".to_owned()
        }
    });
    if setting.replay {
        lines.push("Replay is on: only bars up to the replay position are counted.".to_owned());
    }
    for line in lines {
        paragraph(
            ui,
            &line,
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            theme::text_tertiary(),
        );
    }
    ui.add_space(2.0);
    paragraph(
        ui,
        "Historical frequencies of this chart's bars, with their sample sizes. Not predictions, not advice.",
        kit::size::CAPTION + 0.5,
        kit::REGULAR,
        theme::text_tertiary(),
    );
}
