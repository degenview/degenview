//! The candles of an exchange whose REST kline endpoint answers a fixed number of rows older than a
//! cursor (OKX, Bybit, MEXC, Gate.io, Oanda). A venue declares what differs, a [`KlineEndpoint`]:
//! its paging, its name of a market and how one page is asked for. What every one of them shares
//! is here: the fold plan, the cap before the fold, the pages and the shape of the answer.

use super::candles::plan;
use super::paging::Paging;
use crate::model::market::bucket::FoldPlan;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::ProviderError;

/// One exchange's REST kline endpoint.
pub(in crate::service) trait KlineEndpoint {
    /// The source whose natives and grid plan the requests.
    const SOURCE: DataSource;
    /// How the endpoint's history is paged.
    const PAGING: Paging;

    /// The exchange's name of the market `symbol` stands for, worked out once per answer.
    fn id(&self, symbol: &str) -> String;

    /// Up to `limit` candles of length `base`, oldest first: the newest ones when `before` is
    /// `None`, otherwise those that open before it (exclusive, in milliseconds), however the
    /// exchange spells its own cursor.
    fn page(
        &self,
        id: &str,
        base: Timeframe,
        limit: usize,
        before: Option<i64>,
    ) -> Result<Vec<Kline>, ProviderError>;

    /// How many candles of `base` a request for those opening before `before_ms` may hold and
    /// stay within what the exchange reaches back to (it refuses a request that goes further
    /// back): none once the request would pass it. An answer that fills what is left is read as
    /// cut short, like one that fills the paging.
    fn reach(&self, _base: Timeframe, _before_ms: i64) -> usize {
        usize::MAX
    }
}

/// The newest `limit` candles of `timeframe`.
pub(in crate::service) fn latest<E: KlineEndpoint>(
    endpoint: &E,
    symbol: &str,
    timeframe: Timeframe,
    limit: usize,
) -> Result<Vec<Kline>, ProviderError> {
    let plan = plan(E::SOURCE, timeframe)?;
    let id = endpoint.id(symbol);
    let requested = E::PAGING.requested(plan, limit);
    if requested > E::PAGING.page {
        // More base candles than one request holds: page back from now.
        return page_back(endpoint, &id, plan, 0, crate::clock::now_ms() + 1, limit);
    }
    let rows = endpoint.page(&id, plan.base(), requested, None)?;
    Ok(plan.shape(rows, requested, limit))
}

/// The newest `max` candles of `timeframe` that open in `[start_ms, end_ms)`, oldest first.
pub(in crate::service) fn range<E: KlineEndpoint>(
    endpoint: &E,
    symbol: &str,
    timeframe: Timeframe,
    start_ms: i64,
    end_ms: i64,
    max: usize,
) -> Result<Vec<Kline>, ProviderError> {
    let plan = plan(E::SOURCE, timeframe)?;
    page_back(endpoint, &endpoint.id(symbol), plan, start_ms, end_ms, max)
}

/// The newest `max` candles of `plan`'s target that open in `[start_ms, end_ms)`, paged back from
/// `end_ms`.
fn page_back<E: KlineEndpoint>(
    endpoint: &E,
    id: &str,
    plan: FoldPlan,
    start_ms: i64,
    end_ms: i64,
    max: usize,
) -> Result<Vec<Kline>, ProviderError> {
    let base = plan.base();
    // What cannot be reached is not asked for: an answer that fills this is one the reach cut
    // short, so its oldest fold is dropped as a fragment; a shorter one is the market's start.
    let requested = E::PAGING
        .requested(plan, max)
        .min(endpoint.reach(base, end_ms));
    if requested == 0 {
        return Ok(Vec::new());
    }
    let candles = E::PAGING.back(start_ms, end_ms, requested, |after, wanted| {
        // A gap in the candles moves `after` back past what was counted: never ask for more
        // than the exchange reaches, the end of its history is an empty answer.
        let wanted = wanted.min(endpoint.reach(base, after));
        if wanted == 0 {
            return Ok(Vec::new());
        }
        endpoint.page(id, base, wanted, Some(after))
    })?;
    Ok(plan.shape_since(candles, requested, start_ms, max))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::time::Duration;

    const MINUTE: i64 = 60_000;

    /// Noon UTC on 2026-11-16.
    const NOON: i64 = 1_794_830_400_000;

    /// A market of one-minute candles (volume 1) that opens `listed` and runs up to the one that
    /// opened a minute before noon, behind an endpoint that records what it is asked.
    struct Venue {
        listed: i64,
        /// The oldest instant a request may reach, for an exchange that has such a limit.
        reaches_to: Option<i64>,
        asked: RefCell<Vec<(usize, Option<i64>)>>,
    }

    impl Venue {
        fn listed(listed: i64) -> Self {
            Self {
                listed,
                reaches_to: None,
                asked: RefCell::default(),
            }
        }

        fn asked(&self) -> Vec<(usize, Option<i64>)> {
            self.asked.borrow().clone()
        }
    }

    impl KlineEndpoint for Venue {
        const SOURCE: DataSource = DataSource::Okx;
        const PAGING: Paging = Paging {
            page: 100,
            max_candles: 1_000,
            pause: Duration::ZERO,
        };

        fn id(&self, symbol: &str) -> String {
            symbol.to_uppercase()
        }

        fn page(
            &self,
            id: &str,
            base: Timeframe,
            limit: usize,
            before: Option<i64>,
        ) -> Result<Vec<Kline>, ProviderError> {
            assert_eq!(id, "BTC");
            assert_eq!(base, Timeframe::MINUTE_1);
            self.asked.borrow_mut().push((limit, before));
            let before = before.unwrap_or(i64::MAX);
            let mut newest_first: Vec<Kline> = (1..)
                .map(|back| NOON - back * MINUTE)
                .skip_while(|open| *open >= before)
                .take_while(|open| *open >= self.listed)
                .take(limit)
                .map(|open_time| Kline {
                    open_time,
                    close_time: open_time + MINUTE - 1,
                    open: 1.0,
                    high: 1.0,
                    low: 1.0,
                    close: 1.0,
                    volume: 1.0,
                })
                .collect();
            newest_first.reverse();
            Ok(newest_first)
        }

        fn reach(&self, _base: Timeframe, before_ms: i64) -> usize {
            self.reaches_to.map_or(usize::MAX, |oldest| {
                ((before_ms - oldest) / MINUTE).max(0) as usize
            })
        }
    }

    fn latest_of(venue: &Venue, timeframe: &str, limit: usize) -> Vec<Kline> {
        let timeframe = Timeframe::parse(timeframe).unwrap();
        latest(venue, "btc", timeframe, limit).expect("candles")
    }

    #[test]
    fn a_request_that_fits_one_page_is_one_request_for_the_newest() {
        let venue = Venue::listed(NOON - 5_000 * MINUTE);
        let shown = latest_of(&venue, "1m", 80);
        assert_eq!(venue.asked(), [(80, None)]);
        assert_eq!(shown.len(), 80);
        assert_eq!(
            shown.last().map(|candle| candle.open_time),
            Some(NOON - MINUTE)
        );
    }

    #[test]
    fn a_longer_request_pages_back_from_now_a_page_at_a_time() {
        let venue = Venue::listed(NOON - 5_000 * MINUTE);
        let before = crate::clock::now_ms();
        let shown = latest_of(&venue, "1m", 250);
        let after = crate::clock::now_ms();
        let asked = venue.asked();
        let [(first, Some(from_now)), second, third] = asked[..] else {
            panic!("three pages expected: {asked:?}");
        };
        // The newest page is asked for from just after now, the exclusive end of the history.
        assert_eq!(first, 100);
        assert!((before + 1..=after + 1).contains(&from_now), "{from_now}");
        assert_eq!(second, (100, Some(NOON - 100 * MINUTE)));
        assert_eq!(third, (50, Some(NOON - 200 * MINUTE)));
        assert_eq!(shown.len(), 250);
        assert_eq!(shown[0].open_time, NOON - 250 * MINUTE);
        assert!(
            shown
                .windows(2)
                .all(|pair| pair[0].open_time < pair[1].open_time)
        );
    }

    #[test]
    fn a_range_is_paged_back_from_its_end_and_nothing_outside_it_is_kept() {
        let venue = Venue::listed(NOON - 5_000 * MINUTE);
        let (start, end) = (NOON - 600 * MINUTE, NOON - 10 * MINUTE);
        let shown = range(&venue, "btc", Timeframe::MINUTE_1, start, end, 250).expect("candles");
        assert_eq!(
            venue.asked(),
            [
                (100, Some(end)),
                (100, Some(end - 100 * MINUTE)),
                (50, Some(end - 200 * MINUTE)),
            ]
        );
        assert_eq!(shown.len(), 250);
        assert_eq!(
            shown.last().map(|candle| candle.open_time),
            Some(end - MINUTE)
        );

        // The start is not asked past: the range has 90 minutes, and the first page holds them.
        let venue = Venue::listed(NOON - 5_000 * MINUTE);
        let start = end - 90 * MINUTE;
        let shown = range(&venue, "btc", Timeframe::MINUTE_1, start, end, 250).expect("candles");
        assert_eq!(venue.asked(), [(100, Some(end))]);
        assert_eq!(shown.first().map(|candle| candle.open_time), Some(start));
        assert_eq!(shown.len(), 90);
    }

    #[test]
    fn a_short_page_is_the_start_of_the_history() {
        let venue = Venue::listed(NOON - 130 * MINUTE);
        let shown = latest_of(&venue, "1m", 300);
        assert_eq!(venue.asked().len(), 2, "{:?}", venue.asked());
        assert_eq!(shown.len(), 130);
        assert_eq!(shown[0].open_time, NOON - 130 * MINUTE);
    }

    #[test]
    fn a_request_is_capped_at_what_the_paging_reaches_before_it_is_folded() {
        // 500 candles of 79 minutes is 39,579 one-minute candles; the paging reaches 1,000. The
        // oldest of them, at 19:20, is 54 minutes into a bucket that began at 18:26.
        let venue = Venue::listed(NOON - 5_000 * MINUTE);
        let shown = latest_of(&venue, "79m", 500);
        let asked: usize = venue.asked().iter().map(|(limit, _)| limit).sum();
        assert_eq!(asked, Venue::PAGING.max_candles);
        assert_eq!(shown.first().map(|candle| candle.volume), Some(79.0));
        // The last bucket of a day (18 minutes) is short by design.
        assert!(shown.iter().all(|candle| candle.volume <= 79.0));
    }

    #[test]
    fn a_market_that_began_inside_the_cap_keeps_its_first_candle_as_it_is() {
        // Listed at 02:00, 41 minutes into its first bucket: that bucket is genuinely short.
        let venue = Venue::listed(NOON - 600 * MINUTE);
        let shown = latest_of(&venue, "79m", 500);
        assert!(shown.first().is_some_and(|candle| candle.volume < 79.0));
    }

    #[test]
    fn a_range_that_starts_inside_a_bucket_drops_that_fragment() {
        // 22-minute buckets restart at midnight: 11:00, 11:22, 11:44. A range from 11:10 holds
        // only the last 12 minutes of the 11:00 bucket, which is no candle of 22 minutes.
        let venue = Venue::listed(NOON - 5_000 * MINUTE);
        let fold = Timeframe::parse("22m").unwrap();
        let shown = range(&venue, "btc", fold, NOON - 50 * MINUTE, NOON, 10).expect("candles");
        assert_eq!(
            shown
                .first()
                .map(|candle| (candle.open_time, candle.volume)),
            Some((NOON - 38 * MINUTE, 22.0))
        );

        // A range inside one bucket is nothing but a fragment.
        let inside = range(
            &venue,
            "btc",
            fold,
            NOON - 55 * MINUTE,
            NOON - 45 * MINUTE,
            10,
        );
        assert_eq!(inside, Ok(Vec::new()));
    }

    #[test]
    fn a_request_is_never_asked_further_back_than_the_endpoint_reaches() {
        let mut venue = Venue::listed(NOON - 5_000 * MINUTE);
        venue.reaches_to = Some(NOON - 150 * MINUTE);
        let shown = range(&venue, "btc", Timeframe::MINUTE_1, 0, NOON, 400).expect("candles");
        // The endpoint reaches 150 candles back: the second page is cut to what is left of it.
        assert_eq!(
            venue.asked(),
            [(100, Some(NOON)), (50, Some(NOON - 100 * MINUTE))]
        );
        assert_eq!(shown[0].open_time, NOON - 150 * MINUTE);
        assert_eq!(shown.len(), 150);

        // Past the reach there is nothing to ask for.
        let asked = venue.asked().len();
        let past = range(
            &venue,
            "btc",
            Timeframe::MINUTE_1,
            0,
            NOON - 150 * MINUTE,
            50,
        );
        assert_eq!(past, Ok(Vec::new()));
        assert_eq!(venue.asked().len(), asked);
    }
}
