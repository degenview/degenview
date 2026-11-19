//! Paging a market's candles back from now, for the exchanges whose candle request holds a fixed
//! number of rows and takes an end cursor (OKX, Bybit).

use crate::model::Kline;
use crate::service::ProviderError;
use degenview_market::bucket::FoldPlan;
use std::time::Duration;

/// How an exchange's history is paged.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Paging {
    /// The most rows one request returns.
    pub page: usize,
    /// The most base candles one request pages back to: what the exchange lets a request reach,
    /// or what is worth fetching.
    pub max_candles: usize,
    /// Between pages, for an exchange that limits how fast they may be asked.
    pub pause: Duration,
}

impl Paging {
    /// The base candles to ask for to end with `limit` candles of `plan`'s target: what the fold
    /// needs, at most what the paging reaches. Capped here, before the answer is folded, because a
    /// short answer is read as the start of the market's history and its first fold is kept: a
    /// request the paging cut short must come back as long as it asked for, so that fragment is
    /// dropped.
    pub fn requested(self, plan: FoldPlan, limit: usize) -> usize {
        plan.base_count(limit).min(self.max_candles)
    }

    /// The newest `requested` candles that open in `[start_ms, end_ms)`, oldest first, paged back
    /// from `end_ms`. `fetch(after, wanted)` answers up to `wanted` candles older than `after`,
    /// oldest first; a short page is the start of the market's history.
    pub fn back(
        self,
        start_ms: i64,
        end_ms: i64,
        requested: usize,
        mut fetch: impl FnMut(i64, usize) -> Result<Vec<Kline>, ProviderError>,
    ) -> Result<Vec<Kline>, ProviderError> {
        let mut candles: Vec<Kline> = Vec::new();
        let mut after = end_ms;
        for page in 0..self.max_candles.div_ceil(self.page) {
            if candles.len() >= requested || after <= start_ms {
                break;
            }
            if page > 0 && !self.pause.is_zero() {
                std::thread::sleep(self.pause);
            }
            let wanted = (requested - candles.len()).min(self.page);
            let batch = fetch(after, wanted)?;
            let exhausted = batch.len() < wanted;
            let batch: Vec<Kline> = batch
                .into_iter()
                .filter(|candle| candle.open_time >= start_ms && candle.open_time < after)
                .collect();
            let Some(oldest) = batch.first().map(|candle| candle.open_time) else {
                break;
            };
            after = oldest;
            candles.splice(0..0, batch);
            if exhausted {
                break;
            }
        }
        if candles.len() > requested {
            candles.drain(..candles.len() - requested);
        }
        Ok(candles)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DataSource, Timeframe};
    use crate::service::market::candles::plan;

    const PAGING: Paging = Paging {
        page: 300,
        max_candles: 36_000,
        pause: Duration::ZERO,
    };

    fn candle(open_time: i64) -> Kline {
        Kline {
            open_time,
            close_time: open_time + 59,
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: 1.0,
        }
    }

    /// A market with one candle per 60 ms from 0 to `last`, answering as an exchange's history does.
    fn history(last: i64) -> impl FnMut(i64, usize) -> Result<Vec<Kline>, ProviderError> {
        move |after, wanted| {
            let mut newest_first: Vec<Kline> = (0..=last / 60)
                .rev()
                .map(|index| candle(index * 60))
                .filter(|candle| candle.open_time < after)
                .take(wanted)
                .collect();
            newest_first.reverse();
            Ok(newest_first)
        }
    }

    #[test]
    fn pages_back_until_it_has_what_was_asked() {
        let candles = PAGING.back(0, 700 * 60, 650, history(699 * 60)).unwrap();
        assert_eq!(candles.len(), 650);
        assert_eq!(candles[0].open_time, 50 * 60);
        assert_eq!(candles.last().unwrap().open_time, 699 * 60);
        assert!(
            candles
                .windows(2)
                .all(|pair| pair[0].open_time < pair[1].open_time)
        );
    }

    #[test]
    fn a_short_page_is_the_start_of_history() {
        let candles = PAGING.back(0, 1_000 * 60, 900, history(99 * 60)).unwrap();
        assert_eq!(candles.len(), 100);
        assert_eq!(candles[0].open_time, 0);
    }

    #[test]
    fn nothing_before_the_start_is_kept_or_asked_for() {
        let mut asked = Vec::new();
        let mut inner = history(699 * 60);
        let candles = PAGING
            .back(300 * 60, 700 * 60, 1_000, |after, wanted| {
                asked.push(after);
                inner(after, wanted)
            })
            .unwrap();
        assert_eq!(candles[0].open_time, 300 * 60);
        assert_eq!(candles.len(), 400);
        assert_eq!(asked, [700 * 60, 400 * 60]);
    }

    #[test]
    fn a_failed_page_is_the_answer() {
        let error = PAGING.back(0, 600, 10, |_, _| Err(ProviderError::other("down")));
        assert_eq!(error, Err(ProviderError::other("down")));
    }

    const MINUTE: i64 = 60_000;

    /// `count` one-minute candles of volume 1 that end at `end_ms`, oldest first.
    fn minutes_ending(end_ms: i64, count: usize) -> Vec<Kline> {
        (0..count as i64)
            .rev()
            .map(|index| {
                let open_time = end_ms - (index + 1) * MINUTE;
                Kline {
                    open_time,
                    close_time: open_time + MINUTE - 1,
                    ..candle(open_time)
                }
            })
            .collect()
    }

    /// Noon UTC on 2026-11-16.
    const NOON: i64 = 1_794_830_400_000;

    fn okx_plan(text: &str) -> FoldPlan {
        plan(DataSource::Okx, Timeframe::parse(text).unwrap()).unwrap()
    }

    #[test]
    fn a_request_is_capped_at_what_the_paging_reaches() {
        assert_eq!(PAGING.requested(okx_plan("79m"), 500), PAGING.max_candles);
        assert_eq!(PAGING.requested(okx_plan("1h"), 500), 500);
        assert_eq!(PAGING.requested(okx_plan("22m"), 500), 501 * 22);
    }

    #[test]
    fn an_answer_cut_by_the_paging_cap_loses_its_first_fragment() {
        // 500 candles of 79 minutes is 39,579 one-minute candles; paging reaches 36,000. The
        // oldest of them, at 12:00, is nine minutes into a bucket that began at 11:51.
        let plan = okx_plan("79m");
        let candles = minutes_ending(NOON, PAGING.max_candles);
        let shown = plan.shape(candles, PAGING.requested(plan, 500), 500);
        assert_eq!(shown.first().map(|candle| candle.volume), Some(79.0));
        // The last bucket of each day (18 minutes) and the one forming at noon are short by design.
        assert!(shown.iter().all(|candle| candle.volume <= 79.0));
    }

    #[test]
    fn a_market_that_began_inside_the_cap_keeps_its_first_candle_as_it_is() {
        let plan = okx_plan("79m");
        // Listed at 12:00 with a thousand minutes of history: the first bucket is genuinely short.
        let candles = minutes_ending(NOON + 1_000 * MINUTE, 1_000);
        let shown = plan.shape(candles, PAGING.requested(plan, 500), 500);
        assert!(shown.first().is_some_and(|candle| candle.volume < 79.0));
    }
}
