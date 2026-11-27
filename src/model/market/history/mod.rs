//! Older candles for a chart that is scrolled back toward the start of what it holds.
//!
//! A refresh brings the newest candles and adds them to what is held; scrolling back asks the
//! provider for the page before the oldest one held. The rules here are pure: when to ask, how long
//! to wait after a failure, and how pages and refreshes merge without a candle twice or out of
//! order. The workspace owns the request.

use crate::model::{DataSource, Kline};
use std::time::{Duration, Instant};

/// Candles asked for at once.
pub const PAGE: usize = 1_000;
/// The most candles a chart keeps.
pub const MAX_CANDLES: usize = 100_000;
/// What a full series is cut back to, and where paging stops: a page asked for past it would be cut
/// again by the next refresh.
pub const FULL_CANDLES: usize = MAX_CANDLES - PAGE;
/// How many candles a chart with a script on it pages back to by itself, without the user scrolling
/// there: the 5,000 bars TradingView's Basic plan runs a script over (a Basic account's 5m chart
/// of OKX BTC-USDC held 6,063, 21 days). A script keeps state across
/// days (sessions, last week's high, the crossovers of the last five days), and over the 500 candles
/// a chart first loads it draws less than it does there.
pub const SCRIPT_DEPTH: usize = 5_000;
/// How long a failed request waits before the next one: scrolling must not hammer a provider.
pub const RETRY_AFTER: Duration = Duration::from_secs(15);

/// What a chart knows about the history behind its oldest candle.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OlderHistory {
    /// A page has been merged.
    loaded: bool,
    /// The provider has nothing older.
    exhausted: bool,
    in_flight: bool,
    retry_at: Option<Instant>,
}

impl OlderHistory {
    /// Whether the chart should ask for a page now: the view is near the oldest candle held and
    /// nothing else is in the way.
    pub fn should_request(
        &self,
        supported: bool,
        near_oldest: bool,
        held: usize,
        now: Instant,
    ) -> bool {
        supported
            && near_oldest
            && !self.exhausted
            && !self.in_flight
            && held < FULL_CANDLES
            && self.retry_at.is_none_or(|at| now >= at)
    }

    /// Whether nothing has been asked of the provider yet for the candles held: the first page is
    /// worth fetching ahead of the user, so scrolling back finds it already there.
    pub fn untouched(&self) -> bool {
        !self.loaded && !self.exhausted && !self.in_flight && self.retry_at.is_none()
    }

    pub fn started(&mut self) {
        self.in_flight = true;
    }

    /// Whether a request is out.
    #[cfg(test)]
    pub fn is_loading(&self) -> bool {
        self.in_flight
    }

    /// A page arrived. An empty one is the start of the provider's history.
    pub fn arrived(&mut self, added: usize) {
        self.in_flight = false;
        self.retry_at = None;
        if added == 0 {
            self.exhausted = true;
        } else {
            self.loaded = true;
        }
    }

    pub fn failed(&mut self, now: Instant) {
        self.in_flight = false;
        self.retry_at = Some(now + RETRY_AFTER);
    }

    /// A request was dropped (the market or timeframe changed under it).
    pub fn abandoned(&mut self) {
        self.in_flight = false;
    }
}

/// Whether a chart that runs scripts holds fewer candles than they are meant to run over.
pub fn short_for_scripts(runs_scripts: bool, held: usize) -> bool {
    runs_scripts && held < SCRIPT_DEPTH
}

/// The open time before which a page is wanted: the oldest candle held.
pub fn page_end(held: &[Kline]) -> Option<i64> {
    held.first().map(|candle| candle.open_time)
}

/// Puts `page` in front of `held`, dropping what `held` already has and anything out of order.
/// Returns how many candles were added.
pub fn prepend(held: &mut Vec<Kline>, mut page: Vec<Kline>) -> usize {
    let Some(oldest) = page_end(held) else {
        let added = page.len();
        *held = page;
        return added;
    };
    page.sort_by_key(|candle| candle.open_time);
    page.dedup_by_key(|candle| candle.open_time);
    page.retain(|candle| candle.open_time < oldest);
    let added = page.len();
    if added > 0 {
        page.append(held);
        *held = page;
    }
    added
}

/// What a refresh leaves of a series.
#[derive(Debug, PartialEq)]
pub struct Refreshed {
    pub candles: Vec<Kline>,
    /// The candles held are gone, so what was learnt of the history behind them no longer holds.
    pub restarted: bool,
}

/// `held` brought up to date with `fresh`, a fetch of the newest candles of `source`.
///
/// The fetch says nothing about the candles before its first, so they stay: a series that dropped
/// one for each that opens would be another history on every new bar, and scripts would start over
/// on it. They stay only if the series has the candle the fetch begins with. Without it what lies
/// between is unknown (the computer slept, or a feed added a candle after an outage), so the series
/// starts again with the fetch. A fetch of nothing says nothing, and neither does one that ends
/// before the newest candle held: it is an older view, from a server behind the others. A line chart is the window it was
/// asked for, with points that fall where the request put them: each answer replaces it.
pub fn refreshed(source: DataSource, held: &[Kline], mut fresh: Vec<Kline>) -> Refreshed {
    let kept = |candles| Refreshed {
        candles,
        restarted: false,
    };
    if source.is_line_chart() {
        return kept(fresh);
    }
    let Some(first) = fresh.first().map(|candle| candle.open_time) else {
        return kept(held.to_vec());
    };
    if held
        .last()
        .zip(fresh.last())
        .is_some_and(|(newest, answered)| answered.open_time < newest.open_time)
    {
        return kept(held.to_vec());
    }
    let older = held.partition_point(|candle| candle.open_time < first);
    if older == 0 {
        return kept(fresh);
    }
    if held
        .get(older)
        .is_none_or(|candle| candle.open_time != first)
    {
        return Refreshed {
            candles: fresh,
            restarted: true,
        };
    }
    let mut candles = Vec::with_capacity(older + fresh.len());
    candles.extend_from_slice(&held[..older]);
    candles.append(&mut fresh);
    // Cutting the front makes it another history, so a full series makes room for a page of new
    // candles at once rather than for each.
    if candles.len() > MAX_CANDLES {
        candles.drain(..candles.len() - FULL_CANDLES);
    }
    kept(candles)
}

#[cfg(test)]
mod tests;
