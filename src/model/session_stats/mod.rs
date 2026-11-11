//! Session statistics: how often something happened on the days a chart's bars cover.
//!
//! Every answer is one question, `P(outcome | conditions)`: "on days that gapped up by 0.1% to
//! 0.6% on a Tuesday, how often was the gap filled?". The result is the count of days that
//! qualified (`n`), the days the outcome happened on (`hits`), the proportion and a 95% Wilson
//! interval around it, a split of the sample into its older and newer half, and a row per year.
//! The question and the idea of reporting the sample size with every number are LuxAlgo's
//! `edge-stats` (MIT); this is an independent implementation over the bars the chart draws.
//!
//! **These are historical frequencies of the bars given, not predictions.** The interval says how
//! much a count this small could move by chance; it does not say the pattern will hold, and a
//! filter chosen after looking at the data (any filter is) overstates what it finds.
//!
//! **What the bars can and cannot say.**
//! - A "day" is a trading session. A US stock's session is the regular hours in New York, a bar
//!   belonging to it when it overlaps 09:30–16:00 on a weekday; every other market is taken to
//!   trade round the clock and a day is the UTC date (the same two kinds of symbol the Pine
//!   `session.*` variables know). Daily bars are one day each, by their UTC date.
//! - This build has no bars under an hour. On hourly bars the day opens with the first bar's open,
//!   so for a US stock the first bar may begin before 09:30 and its open may be an extended-hours
//!   price; that is flagged in [`Note::BoundaryOpen`] rather than corrected. Daily bars of a stock
//!   are the provider's own.
//! - A gap is the day's open against the previous day's close, so a market that trades round the
//!   clock has almost none and its gap statistics are mostly empty. A gap is "filled" when a later
//!   bar's range touches the previous close; the day it opens on counts, the bar it opens in counts.
//!   On hourly bars the time to fill is only known to the bar, so it is reported in bars.
//! - The opening range outcomes use the day's first bar as the range, which on hourly bars is an
//!   hour, not the thirty minutes the usual definition uses. They need hourly bars.
//! - A previous day more than [`MAX_GAP_DAYS`] earlier means the data has a hole, so that day has
//!   no previous day and anything that compares with it leaves the day out.
//!
//! Pure: no egui types, nothing read but the slice of bars it is given. Replay works because the
//! chart hands over the bars it draws.

use crate::format::days_from_civil;
use crate::model::market::kline::Kline;
use crate::pine::time::calendar;
use crate::pine::time::trading_session::MarketSessions;

/// A half needs this many days before the older-versus-newer comparison is shown.
pub const MIN_HALF_SAMPLE: usize = 10;
/// The most calendar days between a day and the one it is compared with before the comparison is
/// refused (a long weekend with a holiday is four).
pub const MAX_GAP_DAYS: i64 = 5;
/// The z-value of a two-sided 95% interval.
const Z95: f64 = 1.959_963_984_540_054;
const DAY_MS: i64 = 86_400_000;

/// The length of a bar of `timeframe`, as [`compute`] wants it. Anything over a day is only
/// approximate (a month is not 30 days); [`compute`] refuses all of them alike.
pub fn bar_ms(timeframe: super::Timeframe) -> i64 {
    use crate::model::Timeframe;
    match timeframe {
        Timeframe::Hour => 3_600_000,
        Timeframe::Day => DAY_MS,
        Timeframe::Week => 7 * DAY_MS,
        Timeframe::Month => 30 * DAY_MS,
        Timeframe::Quarter => 91 * DAY_MS,
        Timeframe::Year => 365 * DAY_MS,
    }
}

// MARK: Questions

/// What is being counted.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Outcome {
    /// The day's range touched the previous close (days with a gap only).
    GapFill,
    /// The day closed above its open.
    DayUp,
    /// The day's high went above the previous day's high.
    PriorHighBreak,
    /// The day's low went below the previous day's low.
    PriorLowBreak,
    /// A later bar traded above the first bar's high (hourly bars).
    FirstBarHighBreak,
    /// A later bar traded below the first bar's low (hourly bars).
    FirstBarLowBreak,
}

impl Outcome {
    pub const ALL: [Self; 6] = [
        Self::GapFill,
        Self::DayUp,
        Self::PriorHighBreak,
        Self::PriorLowBreak,
        Self::FirstBarHighBreak,
        Self::FirstBarLowBreak,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::GapFill => "Gap fill",
            Self::DayUp => "Day closes up",
            Self::PriorHighBreak => "Breaks prior day high",
            Self::PriorLowBreak => "Breaks prior day low",
            Self::FirstBarHighBreak => "Breaks first bar high",
            Self::FirstBarLowBreak => "Breaks first bar low",
        }
    }

    /// The sentence the result reads as: "<this> on days that …".
    pub const fn question(self) -> &'static str {
        match self {
            Self::GapFill => "How often the gap was filled",
            Self::DayUp => "How often the day closed above its open",
            Self::PriorHighBreak => "How often the day's high broke the prior day's high",
            Self::PriorLowBreak => "How often the day's low broke the prior day's low",
            Self::FirstBarHighBreak => "How often a later bar broke the first bar's high",
            Self::FirstBarLowBreak => "How often a later bar broke the first bar's low",
        }
    }

    /// When a day counts as a hit, in a sentence for the panel to show under the question.
    pub const fn hit_when(self) -> &'static str {
        match self {
            Self::GapFill => {
                "A later bar's range touches the previous close. Only days that gapped count."
            }
            Self::DayUp => "The day closes above its open. A flat day is not up.",
            Self::PriorHighBreak => "The day's high goes above the previous day's high.",
            Self::PriorLowBreak => "The day's low goes below the previous day's low.",
            Self::FirstBarHighBreak => {
                "A later bar trades above the day's first bar high. Needs the 1H timeframe."
            }
            Self::FirstBarLowBreak => {
                "A later bar trades below the day's first bar low. Needs the 1H timeframe."
            }
        }
    }

    /// A stable name for tools and tests.
    pub const fn slug(self) -> &'static str {
        match self {
            Self::GapFill => "gap_fill",
            Self::DayUp => "day_up",
            Self::PriorHighBreak => "prior_high_break",
            Self::PriorLowBreak => "prior_low_break",
            Self::FirstBarHighBreak => "first_bar_high_break",
            Self::FirstBarLowBreak => "first_bar_low_break",
        }
    }

    /// Whether the outcome needs more than one bar a day.
    pub const fn needs_intraday(self) -> bool {
        matches!(self, Self::FirstBarHighBreak | Self::FirstBarLowBreak)
    }

    /// Whether the outcome happens at some bar of the day, so the bar can be reported.
    const fn is_timed(self) -> bool {
        matches!(
            self,
            Self::GapFill | Self::FirstBarHighBreak | Self::FirstBarLowBreak
        )
    }
}

/// Monday … Sunday, as a set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WeekdaySet(u8);

impl WeekdaySet {
    pub const ALL: Self = Self(0b111_1111);
    pub const NAMES: [&'static str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

    pub const fn contains(self, weekday: usize) -> bool {
        weekday < 7 && self.0 & (1 << weekday) != 0
    }

    /// The set with `weekday` (0 = Monday) switched on or off.
    pub const fn with(self, weekday: usize, on: bool) -> Self {
        if weekday >= 7 {
            return self;
        }
        if on {
            Self(self.0 | (1 << weekday))
        } else {
            Self(self.0 & !(1 << weekday))
        }
    }

    pub const fn is_all(self) -> bool {
        self.0 == Self::ALL.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn names(self) -> Vec<&'static str> {
        (0..7)
            .filter(|&day| self.contains(day))
            .map(|day| Self::NAMES[day])
            .collect()
    }

    /// A set from the three-letter names; `None` for a name that is not one.
    pub fn from_names(names: &[&str]) -> Option<Self> {
        names.iter().try_fold(Self(0), |set, name| {
            let day = Self::NAMES
                .iter()
                .position(|candidate| candidate.eq_ignore_ascii_case(name))?;
            Some(set.with(day, true))
        })
    }
}

/// Up, down or either, for the gap and for the previous day.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Side {
    #[default]
    Any,
    Up,
    Down,
}

impl Side {
    fn admits(self, positive: bool) -> bool {
        match self {
            Self::Any => true,
            Self::Up => positive,
            Self::Down => !positive,
        }
    }
}

/// Which days qualify. Every field narrows the sample; the default is every day.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Filter {
    pub weekdays: WeekdaySet,
    /// Which way the day opened against the previous close.
    pub gap: Side,
    /// The smallest and largest gap, in percent of the previous close, ignoring its sign.
    pub gap_min_pct: Option<f64>,
    pub gap_max_pct: Option<f64>,
    /// Whether the previous day closed above or below its open.
    pub prior_day: Side,
}

impl Default for Filter {
    fn default() -> Self {
        Self {
            weekdays: WeekdaySet::ALL,
            gap: Side::Any,
            gap_min_pct: None,
            gap_max_pct: None,
            prior_day: Side::Any,
        }
    }
}

impl Filter {
    /// Whether any condition is set.
    pub fn is_narrowed(&self) -> bool {
        self.active_count() > 0
    }

    /// How many of the conditions are set: the weekdays, the gap's side, its size and the previous
    /// day each count once.
    pub fn active_count(&self) -> usize {
        usize::from(!self.weekdays.is_all())
            + usize::from(self.gap != Side::Any)
            + usize::from(self.gap_min_pct.is_some() || self.gap_max_pct.is_some())
            + usize::from(self.prior_day != Side::Any)
    }

    fn needs_gap(&self) -> bool {
        self.gap != Side::Any || self.gap_min_pct.is_some() || self.gap_max_pct.is_some()
    }

    /// The conditions as words ("Tue, Wed · gap up 0.1%–0.6% · prior day down").
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if !self.weekdays.is_all() {
            parts.push(self.weekdays.names().join(", "));
        }
        if self.gap != Side::Any || self.gap_min_pct.is_some() || self.gap_max_pct.is_some() {
            let side = match self.gap {
                Side::Any => "gap",
                Side::Up => "gap up",
                Side::Down => "gap down",
            };
            parts.push(match (self.gap_min_pct, self.gap_max_pct) {
                (Some(low), Some(high)) => format!("{side} {low}%–{high}%"),
                (Some(low), None) => format!("{side} ≥ {low}%"),
                (None, Some(high)) => format!("{side} ≤ {high}%"),
                (None, None) => side.to_owned(),
            });
        }
        match self.prior_day {
            Side::Any => {}
            Side::Up => parts.push("prior day up".to_owned()),
            Side::Down => parts.push("prior day down".to_owned()),
        }
        if parts.is_empty() {
            "every day".to_owned()
        } else {
            parts.join(" · ")
        }
    }
}

/// When the market trades, which decides what a day is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarketHours {
    /// Trades round the clock; a day is the UTC date.
    Continuous,
    /// A US stock: the regular session in New York.
    UsStock,
}

impl MarketHours {
    pub fn for_source(source: super::DataSource) -> Self {
        match source {
            super::DataSource::Alpaca => Self::UsStock,
            _ => Self::Continuous,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Continuous => "UTC days",
            Self::UsStock => "US regular hours",
        }
    }
}

// MARK: Results

/// A count of days and how many had the outcome.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Estimate {
    pub hits: usize,
    pub n: usize,
}

impl Estimate {
    pub fn proportion(&self) -> Option<f64> {
        (self.n > 0).then(|| self.hits as f64 / self.n as f64)
    }

    /// The 95% Wilson score interval; `None` for an empty sample.
    pub fn interval(&self) -> Option<(f64, f64)> {
        let proportion = self.proportion()?;
        let n = self.n as f64;
        let z2 = Z95 * Z95;
        let denominator = 1.0 + z2 / n;
        let center = (proportion + z2 / (2.0 * n)) / denominator;
        let half =
            Z95 * (proportion * (1.0 - proportion) / n + z2 / (4.0 * n * n)).sqrt() / denominator;
        Some(((center - half).max(0.0), (center + half).min(1.0)))
    }
}

/// The older half of the sample against the newer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stability {
    pub older: Estimate,
    pub newer: Estimate,
    /// Whether the two halves' intervals overlap. Inferred: edge-stats says its halves "agree" and
    /// does not say by what rule.
    pub agree: bool,
}

/// One year's count.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct YearRow {
    pub year: i64,
    pub estimate: Estimate,
}

/// Where in the day the outcome happened, as the index of the bar (0 is the day's first bar), over
/// the days it happened on. Nearest-rank percentiles.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Timing {
    pub n: usize,
    pub median: usize,
    pub p25: usize,
    pub p75: usize,
    pub p90: usize,
}

/// A caveat the result carries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Note {
    /// A stock's first hourly bar may start before the open, so the day's open may be a pre-market
    /// price.
    BoundaryOpen,
    /// Nothing qualified; for a gap outcome on a market that trades round the clock, that is why.
    NoGaps,
}

impl Note {
    /// A few words for the callout's heading.
    pub const fn title(self) -> &'static str {
        match self {
            Self::BoundaryOpen => "Hourly bars blur the open",
            Self::NoGaps => "No gaps on this market",
        }
    }

    pub const fn text(self) -> &'static str {
        match self {
            Self::BoundaryOpen => {
                "Hourly bars: a day's open is its first bar's open, which may be a pre-market price."
            }
            Self::NoGaps => {
                "This market trades round the clock, so a day opens where the last one closed and there are almost no gaps."
            }
        }
    }
}

/// Why there is no answer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Unavailable {
    NoBars,
    /// The bars are longer than a day.
    TooCoarse,
    /// The outcome needs hourly bars.
    NeedsIntraday,
}

impl Unavailable {
    pub const fn text(self) -> &'static str {
        match self {
            Self::NoBars => "There are no bars to count yet.",
            Self::TooCoarse => "Session statistics need hourly or daily bars.",
            Self::NeedsIntraday => "This question needs hourly bars; switch the timeframe to 1H.",
        }
    }
}

/// The answer to one question.
#[derive(Clone, Debug, PartialEq)]
pub struct Stats {
    pub outcome: Outcome,
    pub filter: Filter,
    pub hours: MarketHours,
    pub intraday: bool,
    /// Sessions found in the bars, qualifying or not.
    pub sessions: usize,
    /// The first and last session date, as `(year, month, day)`.
    pub span: Option<(Date, Date)>,
    pub estimate: Estimate,
    pub stability: Option<Stability>,
    pub per_year: Vec<YearRow>,
    pub timing: Option<Timing>,
    pub notes: Vec<Note>,
}

// MARK: Sessions

/// A session's date as `(year, month, day)`.
pub type Date = (i64, i64, i64);

/// One session's bars and summary.
#[derive(Clone, Copy, Debug)]
struct Day {
    date: Date,
    /// Days since the epoch of `date`.
    number: i64,
    /// Monday is 0.
    weekday: usize,
    start: usize,
    end: usize,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
}

impl Day {
    fn is_up(&self) -> bool {
        self.close > self.open
    }
}

fn usable(bar: &Kline) -> bool {
    [bar.open, bar.high, bar.low, bar.close]
        .iter()
        .all(|value| value.is_finite())
        && bar.high >= bar.low
}

/// Groups the usable bars into sessions. `bars` is oldest first, as the chart holds it.
fn sessions(bars: &[Kline], bar_ms: i64, hours: MarketHours, intraday: bool) -> Vec<Day> {
    let us = MarketSessions::for_kind("stock");
    let mut days: Vec<Day> = Vec::new();
    for (index, bar) in bars.iter().enumerate() {
        if !usable(bar) {
            continue;
        }
        let date = if !intraday {
            // A daily bar is its UTC date: Alpaca stamps it 04:00 or 05:00 UTC, crypto 00:00.
            let c = calendar::components(bar.open_time);
            (c.year, c.month, c.day)
        } else if hours == MarketHours::UsStock {
            let last = bar.open_time + bar_ms - 1;
            if !(us.is_regular(bar.open_time) || us.is_regular(last)) {
                continue;
            }
            let offset = crate::pine::time::zone_offset_secs(Some(us.zone()), bar.open_time);
            let c = calendar::components(bar.open_time + offset * 1_000);
            (c.year, c.month, c.day)
        } else {
            let c = calendar::components(bar.open_time);
            (c.year, c.month, c.day)
        };
        match days.last_mut() {
            Some(day) if day.date == date => {
                day.end = index + 1;
                day.high = day.high.max(bar.high);
                day.low = day.low.min(bar.low);
                day.close = bar.close;
            }
            _ => {
                let number = days_from_civil(date.0, date.1, date.2);
                days.push(Day {
                    date,
                    number,
                    // 1970-01-01 (day 0) was a Thursday, which is 3 from Monday.
                    weekday: (number + 3).rem_euclid(7) as usize,
                    start: index,
                    end: index + 1,
                    open: bar.open,
                    high: bar.high,
                    low: bar.low,
                    close: bar.close,
                });
            }
        }
    }
    days
}

/// What one day says about a question: `None` when it does not qualify, else whether the outcome
/// happened and at which bar.
struct Verdict {
    hit: bool,
    at_bar: Option<usize>,
}

fn judge(
    outcome: Outcome,
    filter: &Filter,
    day: &Day,
    previous: Option<&Day>,
    bars: &[Kline],
) -> Option<Verdict> {
    if !filter.weekdays.contains(day.weekday) {
        return None;
    }
    let previous = previous.filter(|before| day.number - before.number <= MAX_GAP_DAYS);
    let gap_pct = previous.map(|before| (day.open - before.close) / before.close * 100.0);
    if filter.needs_gap() {
        let gap = gap_pct.filter(|gap| gap.is_finite())?;
        if gap == 0.0 || !filter.gap.admits(gap > 0.0) {
            return None;
        }
        if filter.gap_min_pct.is_some_and(|least| gap.abs() < least)
            || filter.gap_max_pct.is_some_and(|most| gap.abs() > most)
        {
            return None;
        }
    }
    if filter.prior_day != Side::Any && !filter.prior_day.admits(previous?.is_up()) {
        return None;
    }
    let day_bars = &bars[day.start..day.end];
    match outcome {
        Outcome::DayUp => Some(Verdict {
            hit: day.is_up(),
            at_bar: None,
        }),
        Outcome::PriorHighBreak => Some(Verdict {
            hit: day.high > previous?.high,
            at_bar: None,
        }),
        Outcome::PriorLowBreak => Some(Verdict {
            hit: day.low < previous?.low,
            at_bar: None,
        }),
        Outcome::GapFill => {
            let before = previous?;
            let gap = gap_pct.filter(|gap| gap.is_finite() && *gap != 0.0)?;
            let touched = |bar: &Kline| {
                if gap > 0.0 {
                    bar.low <= before.close
                } else {
                    bar.high >= before.close
                }
            };
            let at = day_bars.iter().position(touched);
            Some(Verdict {
                hit: at.is_some(),
                at_bar: at,
            })
        }
        Outcome::FirstBarHighBreak | Outcome::FirstBarLowBreak => {
            let (first, rest) = day_bars.split_first()?;
            if rest.is_empty() {
                return None;
            }
            let at = rest.iter().position(|bar| {
                if outcome == Outcome::FirstBarHighBreak {
                    bar.high > first.high
                } else {
                    bar.low < first.low
                }
            });
            Some(Verdict {
                hit: at.is_some(),
                at_bar: at.map(|position| position + 1),
            })
        }
    }
}

/// Answers `outcome` over `bars` for the days `filter` admits.
///
/// `bar_ms` is the bar length: under a day the bars are grouped into sessions, at a day each bar
/// is one session, and anything longer is refused.
pub fn compute(
    bars: &[Kline],
    bar_ms: i64,
    hours: MarketHours,
    outcome: Outcome,
    filter: &Filter,
) -> Result<Stats, Unavailable> {
    if bars.is_empty() {
        return Err(Unavailable::NoBars);
    }
    if bar_ms > DAY_MS {
        return Err(Unavailable::TooCoarse);
    }
    let intraday = bar_ms < DAY_MS;
    if outcome.needs_intraday() && !intraday {
        return Err(Unavailable::NeedsIntraday);
    }
    let days = sessions(bars, bar_ms, hours, intraday);
    if days.is_empty() {
        return Err(Unavailable::NoBars);
    }

    // The qualifying days, oldest first: the year, whether it hit, and at which bar.
    let mut samples: Vec<(i64, bool, Option<usize>)> = Vec::new();
    for (index, day) in days.iter().enumerate() {
        let previous = index.checked_sub(1).map(|before| &days[before]);
        if let Some(verdict) = judge(outcome, filter, day, previous, bars) {
            samples.push((day.date.0, verdict.hit, verdict.at_bar));
        }
    }

    let count = |slice: &[(i64, bool, Option<usize>)]| Estimate {
        hits: slice.iter().filter(|sample| sample.1).count(),
        n: slice.len(),
    };
    let estimate = count(&samples);

    let half = samples.len() / 2;
    let stability =
        (half >= MIN_HALF_SAMPLE && samples.len() - half >= MIN_HALF_SAMPLE).then(|| {
            let older = count(&samples[..half]);
            let newer = count(&samples[half..]);
            let agree = match (older.interval(), newer.interval()) {
                (Some(a), Some(b)) => a.0 <= b.1 && b.0 <= a.1,
                _ => false,
            };
            Stability {
                older,
                newer,
                agree,
            }
        });

    let mut per_year: Vec<YearRow> = Vec::new();
    for &(year, hit, _) in &samples {
        match per_year.last_mut() {
            Some(row) if row.year == year => {
                row.estimate.n += 1;
                row.estimate.hits += usize::from(hit);
            }
            _ => per_year.push(YearRow {
                year,
                estimate: Estimate {
                    hits: usize::from(hit),
                    n: 1,
                },
            }),
        }
    }

    let timing = (intraday && outcome.is_timed())
        .then(|| {
            let mut at: Vec<usize> = samples.iter().filter_map(|sample| sample.2).collect();
            at.sort_unstable();
            let rank = |p: f64| at[((p * at.len() as f64).ceil() as usize).clamp(1, at.len()) - 1];
            (!at.is_empty()).then(|| Timing {
                n: at.len(),
                median: rank(0.5),
                p25: rank(0.25),
                p75: rank(0.75),
                p90: rank(0.9),
            })
        })
        .flatten();

    let mut notes = Vec::new();
    if intraday && hours == MarketHours::UsStock {
        notes.push(Note::BoundaryOpen);
    }
    if estimate.n == 0 && outcome == Outcome::GapFill && hours == MarketHours::Continuous {
        notes.push(Note::NoGaps);
    }

    Ok(Stats {
        outcome,
        filter: *filter,
        hours,
        intraday,
        sessions: days.len(),
        span: days.first().zip(days.last()).map(|(a, b)| (a.date, b.date)),
        estimate,
        stability,
        per_year,
        timing,
        notes,
    })
}

#[cfg(test)]
mod tests;
