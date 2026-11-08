//! The words and numbers of the Session statistics panel: how a share, a count, a time and a whole
//! answer read. Pure, so the panel, the copy button and the tests all say the same thing.

use crate::format::group_thousands;
use crate::model::session_stats::{Date, Estimate, Stats};

/// A share as a percent with one decimal (`0.919` is `91.9%`).
pub fn percent(value: f64) -> String {
    format!("{:.1}%", value * 100.0)
}

/// A count with thousands separators.
pub fn count(value: usize) -> String {
    group_thousands(&value.to_string())
}

/// `hits/n`, as the per-year and per-half rows show it.
pub fn fraction(estimate: &Estimate) -> String {
    format!("{}/{}", count(estimate.hits), count(estimate.n))
}

/// A share and its counts, or "no days".
pub fn share(estimate: &Estimate) -> String {
    match estimate.proportion() {
        Some(p) => format!("{} · {}", percent(p), fraction(estimate)),
        None => "no days".to_owned(),
    }
}

/// Hours with no needless decimals (`2`, `0.5`).
fn hours(value: f64) -> String {
    if (value - value.round()).abs() < 1e-9 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.1}")
    }
}

/// Where in the day bar `index` is, for bars of `bar_ms`: the first bar, or the hours it covers.
pub fn after_open(index: usize, bar_ms: i64) -> String {
    if index == 0 {
        return "in the first bar".to_owned();
    }
    let per_bar = bar_ms as f64 / 3_600_000.0;
    format!(
        "bar {index} ({}–{} h in)",
        hours(index as f64 * per_bar),
        hours((index + 1) as f64 * per_bar)
    )
}

/// A session date as `2024-03-05`.
pub fn date(date: Date) -> String {
    format!("{:04}-{:02}-{:02}", date.0, date.1, date.2)
}

/// The answer as plain text for the clipboard: the question, the number with its sample and
/// interval, the halves, and the reminder that it is a count, not a forecast.
pub fn summary(title: &str, stats: &Stats, bar_ms: i64) -> String {
    let mut lines = vec![format!(
        "{title} · {} on {}",
        stats.outcome.label(),
        stats.filter.describe()
    )];
    match (stats.estimate.proportion(), stats.estimate.interval()) {
        (Some(p), Some((low, high))) => lines.push(format!(
            "{} ({} of {} days) · 95% CI {} – {}",
            percent(p),
            count(stats.estimate.hits),
            count(stats.estimate.n),
            percent(low),
            percent(high)
        )),
        _ => lines.push("No days qualify.".to_owned()),
    }
    if let Some(halves) = &stats.stability {
        lines.push(format!(
            "Older half {} · newer half {} · the halves {}",
            share(&halves.older),
            share(&halves.newer),
            if halves.agree { "agree" } else { "differ" }
        ));
    }
    if let Some(timing) = &stats.timing {
        lines.push(format!(
            "When (n = {}): half by {} · 90% by {}",
            count(timing.n),
            after_open(timing.median, bar_ms),
            after_open(timing.p90, bar_ms)
        ));
    }
    if let Some((first, last)) = stats.span {
        lines.push(format!(
            "{} sessions, {} to {} · {}",
            count(stats.sessions),
            date(first),
            date(last),
            stats.hours.label()
        ));
    }
    lines.push("Historical frequency of this chart's bars, not a prediction.".to_owned());
    lines.join("\n")
}
