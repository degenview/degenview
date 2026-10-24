//! Older candles for a chart that is scrolled back toward the start of what it holds.
//!
//! A refresh brings the newest candles; scrolling back asks the provider for the page before the
//! oldest one held. The rules here are pure: when to ask, how long to wait after a failure, and how
//! pages and refreshes merge without a candle twice or out of order. The workspace owns the request.

use crate::model::Kline;
use std::time::{Duration, Instant};

/// Candles asked for at once.
pub const PAGE: usize = 1_000;
/// The most candles a chart keeps; paging stops there rather than growing without bound.
pub const MAX_CANDLES: usize = 100_000;
/// How long a failed request waits before the next one: scrolling must not hammer a provider.
pub const RETRY_AFTER: Duration = Duration::from_secs(15);

/// What a chart knows about the history behind its oldest candle.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OlderHistory {
    /// A page has been merged: refreshes must keep what they did not bring back.
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
            && held < MAX_CANDLES
            && self.retry_at.is_none_or(|at| now >= at)
    }

    /// Whether more history may still come: the provider has not said it has nothing older and
    /// the chart is under its cap.
    pub fn may_have_more(&self, held: usize) -> bool {
        !self.exhausted && held < MAX_CANDLES
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

    /// Whether a refresh must keep the older candles it did not bring back.
    pub fn keeps_older(&self) -> bool {
        self.loaded
    }
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

/// What a refresh leaves on the chart: the candles it brought, with the older ones the chart
/// already held in front. Without loaded history the refresh simply replaces them.
pub fn with_older(held: &[Kline], mut fresh: Vec<Kline>, keep_older: bool) -> Vec<Kline> {
    let Some(first) = fresh.first().map(|candle| candle.open_time) else {
        return fresh;
    };
    if !keep_older {
        return fresh;
    }
    let older = held.partition_point(|candle| candle.open_time < first);
    if older == 0 {
        return fresh;
    }
    let mut merged = Vec::with_capacity(older + fresh.len());
    merged.extend_from_slice(&held[..older]);
    merged.append(&mut fresh);
    if merged.len() > MAX_CANDLES {
        merged.drain(..merged.len() - MAX_CANDLES);
    }
    merged
}

#[cfg(test)]
mod tests;
