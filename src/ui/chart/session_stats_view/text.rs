//! The words and numbers of the Session statistics panel: how a share, a count, a time and a whole
//! answer read. Pure, so the panel, the copy button and the tests all say the same thing.

use crate::format::group_thousands;
use crate::i18n::tr;
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
        None => tr!("no days").to_owned(),
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
        return tr!("in the first bar").to_owned();
    }
    let per_bar = bar_ms as f64 / 3_600_000.0;
    tr!(
        "bar {index} ({from}–{to} h in)",
        index = index,
        from = hours(index as f64 * per_bar),
        to = hours((index + 1) as f64 * per_bar)
    )
}

/// A session date as `2024-03-05`.
pub fn date(date: Date) -> String {
    format!("{:04}-{:02}-{:02}", date.0, date.1, date.2)
}

/// The answer as plain text for the clipboard: the question, the number with its sample and
/// interval, the halves, and the reminder that it is a count, not a forecast.
pub fn summary(title: &str, stats: &Stats, bar_ms: i64) -> String {
    let mut lines = vec![tr!(
        "{title} · {outcome} on {filter}",
        title = title,
        outcome = stats.outcome.label(),
        filter = stats.filter.display_describe()
    )];
    match (stats.estimate.proportion(), stats.estimate.interval()) {
        (Some(p), Some((low, high))) => lines.push(tr!(
            "{share} ({hits} of {days} days) · 95% CI {low} – {high}",
            share = percent(p),
            hits = count(stats.estimate.hits),
            days = count(stats.estimate.n),
            low = percent(low),
            high = percent(high)
        )),
        _ => lines.push(tr!("No days qualify.").to_owned()),
    }
    if let Some(halves) = &stats.stability {
        let (older, newer) = (share(&halves.older), share(&halves.newer));
        lines.push(if halves.agree {
            tr!(
                "Older half {older} · newer half {newer} · the halves agree",
                older = older,
                newer = newer
            )
        } else {
            tr!(
                "Older half {older} · newer half {newer} · the halves differ",
                older = older,
                newer = newer
            )
        });
    }
    if let Some(timing) = &stats.timing {
        lines.push(tr!(
            "When (n = {days}): half by {median} · 90% by {p90}",
            days = count(timing.n),
            median = after_open(timing.median, bar_ms),
            p90 = after_open(timing.p90, bar_ms)
        ));
    }
    if let Some((first, last)) = stats.span {
        lines.push(tr!(
            "{sessions} sessions, {first} to {last} · {hours}",
            sessions = count(stats.sessions),
            first = date(first),
            last = date(last),
            hours = stats.hours.display_label()
        ));
    }
    lines.push(tr!("Historical frequency of this chart's bars, not a prediction.").to_owned());
    lines.join("\n")
}
