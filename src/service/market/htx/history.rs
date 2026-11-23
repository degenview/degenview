//! Paging HTX's candles back from now.
//!
//! REST holds only the newest 2,000 candles of a period (`rest.rs`); older ones come over the
//! socket, whose `req` answers a window of at most 900 lengths and reaches back to a market's first
//! candle (measured on 2026-11-21: BTCUSDT's 1m and 4h to 2017-11-08). So a page is asked over
//! REST while it lies inside those 2,000 (a chart's first answer and its refreshes, which cost no
//! connection) and over a socket, opened the first time one is needed and kept for the rest of the
//! fetch, when it does not.
//!
//! HTX fills the gaps of a market nobody traded with flat candles, so a short answer is the start of
//! the market, which is what [`Paging`] reads it as.

use super::parse::history_reply;
use super::period;
use super::rest::{REST_BARS, recent};
use super::socket::{history_request, session};
use crate::model::market::bucket::bucket_start_ms;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::ProviderError;
use crate::service::market::paging::Paging;
use crate::service::net::http;
use crate::service::net::ws::{Conversation, ConversationError};
use std::time::Duration;

/// A chart reaches back at most 30,000 base candles in one request, which is all of BTCUSDT's four
/// hours (about 20,000 since 2017) and 20 days of its minutes; the older ones come as the chart
/// scrolls to them.
pub(super) const PAGING: Paging = Paging {
    page: REST_BARS,
    max_candles: 30_000,
    pause: Duration::ZERO,
};

/// The most candles one `req` holds: a window of 900 lengths (901 is refused).
const SOCKET_BARS: usize = 900;

/// How long an answer to a `req` is awaited; they come in well under a second.
const PATIENCE: Duration = Duration::from_secs(10);

/// A fetch's way to the history: REST, and a socket once one is needed.
pub(super) struct History {
    conversation: Option<Conversation>,
}

impl History {
    pub(super) fn new() -> Self {
        Self { conversation: None }
    }

    /// The newest `wanted` candles of `base` that open before `after_ms`, oldest first; fewer only
    /// when the market began later.
    pub(super) fn page_before(
        &mut self,
        id: &str,
        base: Timeframe,
        after_ms: i64,
        wanted: usize,
    ) -> Result<Vec<Kline>, ProviderError> {
        page_before_with(
            base,
            after_ms,
            wanted,
            crate::clock::now_ms(),
            |count| recent(id, base, count),
            |from_s, to_s| self.window(id, base, from_s, to_s),
        )
    }

    /// The candles that open from `from_s` to `to_s` (seconds, both included), over the socket. A
    /// connection that fails is dropped: the next window opens a new one.
    fn window(
        &mut self,
        id: &str,
        base: Timeframe,
        from_s: i64,
        to_s: i64,
    ) -> Result<Vec<Kline>, ProviderError> {
        let period =
            period(base).ok_or_else(|| ProviderError::unsupported(DataSource::Htx, base))?;
        let conversation = match &mut self.conversation {
            Some(open) => open,
            None => self
                .conversation
                .insert(session(Vec::new()).converse().map_err(network)?),
        };
        let reply = conversation.ask(
            &history_request(id, period, from_s, to_s),
            PATIENCE,
            |text| history_reply(text, base),
        );
        match reply {
            Ok(candles) => candles,
            Err(error) => {
                self.conversation = None;
                Err(network(error))
            }
        }
    }
}

fn network(error: ConversationError) -> ProviderError {
    ProviderError::Network(http::Error::Network(error.to_string()))
}

/// The page, from `recent(count)` (the newest `count` candles, oldest first) when the candles
/// wanted are among the newest [`REST_BARS`] as of `now_ms`, else from `window(from_s, to_s)` (the
/// candles that open in a window of seconds, both ends included) in as many windows as it takes.
fn page_before_with(
    base: Timeframe,
    after_ms: i64,
    wanted: usize,
    now_ms: i64,
    recent: impl FnOnce(usize) -> Result<Vec<Kline>, ProviderError>,
    mut window: impl FnMut(i64, i64) -> Result<Vec<Kline>, ProviderError>,
) -> Result<Vec<Kline>, ProviderError> {
    let step = base.nominal_ms();
    // The newest candle wanted opens in this bucket; newer ones lie between it and the forming one.
    let newest = bucket_start_ms(base, after_ms - 1);
    let newer = ((bucket_start_ms(base, now_ms) - newest) / step).max(0) as usize;
    if newer + wanted <= REST_BARS {
        let mut candles = recent(newer + wanted)?;
        candles.retain(|candle| candle.open_time < after_ms);
        candles.drain(..candles.len().saturating_sub(wanted));
        return Ok(candles);
    }
    let mut candles: Vec<Kline> = Vec::new();
    let mut newest = newest;
    while candles.len() < wanted {
        let length = (wanted - candles.len()).min(SOCKET_BARS);
        let oldest = newest - (length as i64 - 1) * step;
        let mut batch = window(oldest / 1_000, newest / 1_000)?;
        batch.retain(|candle| candle.open_time <= newest);
        let short = batch.len() < length;
        let Some(first) = batch.first().map(|candle| candle.open_time) else {
            break;
        };
        candles.splice(0..0, batch);
        // A short window is the start of the market.
        if short {
            break;
        }
        newest = first - step;
    }
    Ok(candles)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: i64 = 60_000;

    fn candle(open_time: i64) -> Kline {
        Kline {
            open_time,
            close_time: open_time + MINUTE - 1,
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: 1.0,
        }
    }

    /// A market whose candles of one minute open from `first` minute to `last` minute (numbered
    /// from 0), answering as HTX does: the newest `count` for REST, a window of seconds for `req`.
    struct Market {
        first: i64,
        last: i64,
        rests: std::cell::RefCell<Vec<usize>>,
        windows: std::cell::RefCell<Vec<(i64, i64)>>,
    }

    impl Market {
        fn new(first: i64, last: i64) -> Self {
            Self {
                first,
                last,
                rests: Default::default(),
                windows: Default::default(),
            }
        }

        fn recent(&self, count: usize) -> Result<Vec<Kline>, ProviderError> {
            self.rests.borrow_mut().push(count);
            Ok((self.first.max(self.last - count as i64 + 1)..=self.last)
                .map(|minute| candle(minute * MINUTE))
                .collect())
        }

        fn window(&self, from_s: i64, to_s: i64) -> Result<Vec<Kline>, ProviderError> {
            self.windows.borrow_mut().push((from_s, to_s));
            assert!(
                (to_s - from_s) / 60 < SOCKET_BARS as i64,
                "HTX refuses a window of 900 lengths or more"
            );
            Ok((from_s / 60..=to_s / 60)
                .filter(|minute| (self.first..=self.last).contains(minute))
                .map(|minute| candle(minute * MINUTE))
                .collect())
        }

        fn page(&self, after_minute: i64, wanted: usize) -> Result<Vec<Kline>, ProviderError> {
            page_before_with(
                Timeframe::MINUTE_1,
                after_minute * MINUTE,
                wanted,
                (self.last * MINUTE) + 17_000,
                |count| self.recent(count),
                |from, to| self.window(from, to),
            )
        }
    }

    fn opens(candles: &[Kline]) -> (i64, i64, usize) {
        (
            candles
                .first()
                .map_or(-1, |candle| candle.open_time / MINUTE),
            candles
                .last()
                .map_or(-1, |candle| candle.open_time / MINUTE),
            candles.len(),
        )
    }

    #[test]
    fn the_newest_candles_are_one_request_over_rest() {
        let market = Market::new(0, 100_000);
        // `after` is just past the forming candle: nothing is newer than the page.
        let page = market.page(100_001, 500).unwrap();
        assert_eq!(opens(&page), (99_501, 100_000, 500));
        assert_eq!(*market.rests.borrow(), [500]);
        assert!(market.windows.borrow().is_empty());
    }

    #[test]
    fn a_page_inside_the_newest_two_thousand_is_asked_for_with_the_newer_ones_it_skips() {
        let market = Market::new(0, 100_000);
        // 300 candles lie between the page and now: 300 + 600 are asked, and the newer 300 go.
        let page = market.page(99_701, 600).unwrap();
        assert_eq!(opens(&page), (99_101, 99_700, 600));
        assert_eq!(*market.rests.borrow(), [900]);
        assert!(market.windows.borrow().is_empty());
    }

    #[test]
    fn a_page_past_the_newest_two_thousand_is_asked_over_the_socket_in_windows_of_900() {
        let market = Market::new(0, 100_000);
        let page = market.page(97_000, 2_000).unwrap();
        // Joined to the page after it, no gap and no repeat.
        assert_eq!(opens(&page), (95_000, 96_999, 2_000));
        assert!(market.rests.borrow().is_empty());
        let windows = market.windows.borrow().clone();
        assert_eq!(
            windows,
            [
                (96_100 * 60, 96_999 * 60),
                (95_200 * 60, 96_099 * 60),
                (95_000 * 60, 95_199 * 60),
            ]
        );
    }

    #[test]
    fn a_short_window_is_the_start_of_the_market_and_the_last_one_asked() {
        let market = Market::new(96_500, 100_000);
        let page = market.page(97_000, 2_000).unwrap();
        assert_eq!(opens(&page), (96_500, 96_999, 500));
        assert_eq!(market.windows.borrow().len(), 1);
        // Before the market began there is nothing, and one window finds that out.
        let market = Market::new(96_500, 100_000);
        assert!(market.page(96_500, 100).unwrap().is_empty());
        assert_eq!(market.windows.borrow().len(), 1);
    }

    #[test]
    fn a_young_market_is_short_over_rest_too() {
        let market = Market::new(99_990, 100_000);
        let page = market.page(100_001, 500).unwrap();
        assert_eq!(opens(&page), (99_990, 100_000, 11));
    }

    #[test]
    fn a_refused_window_is_the_answer() {
        let result = page_before_with(
            Timeframe::MINUTE_1,
            1_000 * MINUTE,
            100,
            5_100 * MINUTE,
            |_| Ok(Vec::new()),
            |_, _| Err(ProviderError::other("HTX: down")),
        );
        // 4,100 minutes before now is not among the newest 2,000: the socket is asked and fails.
        assert_eq!(result, Err(ProviderError::other("HTX: down")));
    }

    #[test]
    fn four_hour_pages_are_cut_on_four_hour_opens() {
        const HOUR: i64 = 3_600_000;
        let four = Timeframe::parse("4h").unwrap();
        let after = 7_000 * 4 * HOUR;
        let now = 9_999 * 4 * HOUR + 100_000;
        let mut asked = Vec::new();
        let page = page_before_with(
            four,
            after,
            1_000,
            now,
            |_| unreachable!("2,999 candles are newer"),
            |from_s, to_s| {
                asked.push((from_s, to_s));
                Ok((from_s..=to_s)
                    .step_by(4 * 3_600)
                    .map(|open| candle(open * 1_000))
                    .collect())
            },
        )
        .unwrap();
        assert_eq!(page.len(), 1_000);
        assert_eq!(page.last().unwrap().open_time, after - 4 * HOUR);
        assert_eq!(asked[0].1, (after - 4 * HOUR) / 1_000);
        assert!(
            asked
                .iter()
                .all(|(from, to)| (to - from) % (4 * 3_600) == 0)
        );
    }
}
