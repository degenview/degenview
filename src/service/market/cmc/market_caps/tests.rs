use super::*;
use crate::model::market::cmc::interval::{MAX_WINDOWS, WINDOW_POINTS};
use crate::service::cache::disk::{DiskCache, testing::TempCache};
use crate::service::cache::lookup::LookupCache;
use crate::service::net::http;
use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

/// Two readings as the website answers them (2026-11-21), trimmed to the fields that are read.
const BODY: &str = r#"{"data":{"quotes":[
  {"timestamp":"2026-11-21T07:10:00.000Z","searchInterval":"5","btcDominance":58.4893,"ethDominance":9.9848,
   "activeCryptocurrencies":7952,
   "quote":[{"name":"USD","timestamp":"2026-11-21T07:10:00.000Z","totalMarketCap":2002891524988.97000000,
             "totalVolume24H":147854065217.62,"altcoinMarketCap":831413723206.62000000}]},
  {"timestamp":"2026-11-21T07:15:00.000Z","searchInterval":"5","btcDominance":58.4433,"ethDominance":9.9741,
   "quote":[{"name":"USD","timestamp":"2026-11-21T07:15:00.000Z","totalMarketCap":1999149660677.21000000,
             "altcoinMarketCap":830781546903.87000000}]}
 ],"totalCryptocurrencies":40244},
 "status":{"timestamp":"2026-11-21T08:07:39.787Z","error_code":"0","error_message":"SUCCESS","elapsed":"14","credit_count":0}}"#;

fn error_body(code: &str, message: &str) -> String {
    format!(r#"{{"status":{{"error_code":"{code}","error_message":"{message}"}}}}"#)
}

fn point(time_ms: i64, total_cap: f64) -> GlobalPoint {
    GlobalPoint {
        time_ms,
        total_cap,
        altcoin_cap: total_cap * 0.4,
        btc_dominance: 60.0,
        eth_dominance: 10.0,
    }
}

#[test]
fn a_response_is_read_into_readings_oldest_first() {
    let points = parse::readings(BODY).unwrap();
    assert_eq!(points.len(), 2);
    assert_eq!(points[0].time_ms, 1_795_245_000_000);
    assert_eq!(points[1].time_ms - points[0].time_ms, 300_000);
    assert!((points[0].total_cap - 2_002_891_524_988.97).abs() < 1.0);
    assert!((points[0].altcoin_cap - 831_413_723_206.62).abs() < 1.0);
    assert_eq!(points[0].btc_dominance, 58.4893);
    assert_eq!(points[0].eth_dominance, 9.9848);
}

#[test]
fn readings_arrive_in_any_order_and_once() {
    let body = r#"{"data":{"quotes":[
      {"timestamp":"2026-11-21T07:15:00.000Z","btcDominance":58.0,"quote":[{"totalMarketCap":2e12,"altcoinMarketCap":8e11}]},
      {"timestamp":"2026-11-21T07:10:00.000Z","btcDominance":58.1,"quote":[{"totalMarketCap":2e12,"altcoinMarketCap":8e11}]},
      {"timestamp":"2026-11-21T07:10:00.000Z","btcDominance":58.1,"quote":[{"totalMarketCap":2e12,"altcoinMarketCap":8e11}]}
    ]}}"#;
    let points = parse::readings(body).unwrap();
    assert_eq!(points.len(), 2);
    assert!(points[0].time_ms < points[1].time_ms);
    // The history starts before Ether: no dominance is none.
    assert_eq!(points[0].eth_dominance, 0.0);
}

#[test]
fn a_reading_without_its_figures_is_left_out_and_none_is_still_an_answer() {
    let body = r#"{"data":{"quotes":[
      {"timestamp":"not a time","btcDominance":58.0,"quote":[{"totalMarketCap":2e12,"altcoinMarketCap":8e11}]},
      {"timestamp":"2026-11-21T07:10:00.000Z","quote":[{"totalMarketCap":2e12,"altcoinMarketCap":8e11}]},
      {"timestamp":"2026-11-21T07:15:00.000Z","btcDominance":58.0,"quote":[]},
      {"timestamp":"2026-11-21T07:20:00.000Z","btcDominance":58.0,"quote":[{"totalMarketCap":null,"altcoinMarketCap":8e11}]},
      {"timestamp":"2026-11-21T07:25:00.000Z","btcDominance":58.0,"quote":[{"totalMarketCap":2e12,"altcoinMarketCap":8e11}]}
    ]}}"#;
    assert_eq!(parse::readings(body).unwrap().len(), 1);
    assert_eq!(parse::readings(r#"{"data":{"quotes":[]}}"#).unwrap(), []);
}

#[test]
fn a_busy_system_is_an_unavailable_provider_and_a_429_a_rate_limit() {
    // HTTP 200, and the failure is in the body.
    let busy = parse::readings(&error_body(
        "500",
        "The system is busy, please try again later!",
    ));
    assert_eq!(
        busy,
        Err(ProviderError::CoinMarketCap(http::Error::Status(500)))
    );
    assert!(!busy.unwrap_err().is_rate_limited());
    let limited = parse::readings(&error_body("429", "Too many requests")).unwrap_err();
    assert!(limited.is_rate_limited());
    // The window cap is a refusal too, not an empty chart.
    assert!(
        parse::readings(&error_body(
            "500",
            "Too many items requested. Limit is 2200"
        ))
        .is_err()
    );
}

#[test]
fn a_code_outside_http_keeps_the_apis_own_words() {
    assert_eq!(
        parse::readings(&error_body("1008", "Rate limit reached")),
        Err(ProviderError::other("Rate limit reached"))
    );
    assert_eq!(
        parse::readings(r#"{"status":{"error_code":1008}}"#),
        Err(ProviderError::other("CoinMarketCap error 1008"))
    );
}

#[test]
fn a_body_that_is_not_the_history_is_a_data_error() {
    for body in [
        "",
        "<html>blocked</html>",
        r#"{"data":{}}"#,
        r#"{"data":{"quotes":{}}}"#,
        &error_body("soon", "?"),
    ] {
        assert!(
            matches!(parse::readings(body), Err(ProviderError::Data(_))),
            "{body}"
        );
    }
}

#[test]
fn a_window_is_asked_for_by_whole_seconds_inside_the_request_cap() {
    for interval in Interval::ALL {
        let window = interval.windows(1_795_248_600_000, 1_795_248_600_000)[0];
        let url = rest::url(interval, &window);
        assert!(url.starts_with(
            "https://api.coinmarketcap.com/data-api/v3/global-metrics/quotes/historical?format=chart_altcoin&"
        ));
        assert!(url.contains(&format!("interval={}", interval.query())));
        let (from, to) = window.query_seconds();
        assert!(url.ends_with(&format!("timeStart={from}&timeEnd={to}")));
        let readings = (to - from) / (interval.spacing_ms() / 1_000) + 1;
        assert!(readings <= WINDOW_POINTS && WINDOW_POINTS < 2_200);
    }
}

const NOW: i64 = 1_795_248_600_000;

fn five_minute_points(count: i64) -> Vec<GlobalPoint> {
    (0..count)
        .map(|n| point(NOW - (count - 1 - n) * 300_000, 2e12 + n as f64 * 1e9))
        .collect()
}

#[test]
fn the_indices_become_candles_of_the_timeframe() {
    let points = five_minute_points(24);
    let load = |_: Interval, _: &[Window]| Ok(points.clone());
    let total = candles(GlobalIndex::Total, Timeframe::HOUR_1, 500, NOW, load).unwrap();
    // 24 readings of five minutes: two hours of candles, each closing at its last reading.
    assert!(total.len() >= 2 && total.len() <= 3);
    let last = total.last().unwrap();
    assert!((last.close - (2e12 + 23.0 * 1e9) / 1e9).abs() < 1e-6);
    assert_eq!(last.volume, 0.0);
    let dominance = candles(GlobalIndex::BtcDominance, Timeframe::HOUR_1, 500, NOW, load).unwrap();
    assert!(dominance.iter().all(|candle| candle.close == 60.0));
}

/// 2026-11-21T00:00:00Z.
const DAY_START: i64 = 1_795_219_200_000;

fn daily_points() -> Vec<GlobalPoint> {
    // The day's reading is taken at 10:10 UTC: yesterday's is the newest at 08:10 today.
    [(-2, 2.00e12), (-1, 2.02e12)]
        .map(|(days, cap)| point(DAY_START + days * 86_400_000 + 36_600_000, cap))
        .to_vec()
}

#[test]
fn the_newest_readings_follow_a_coarser_series_so_todays_candle_moves() {
    let daily = daily_points();
    let newest = five_minute_points(4);
    let points = with_newest(daily.clone(), Interval::Day1, || newest.clone());
    assert_eq!(points.len(), 6);
    let candles = candles(GlobalIndex::Total, Timeframe::DAY_1, 500, NOW, |_, _| {
        Ok(points.clone())
    })
    .unwrap();
    assert_eq!(
        candles.len(),
        3,
        "today has a candle before its own reading"
    );
    let today = candles.last().unwrap();
    assert_eq!(today.open_time, DAY_START);
    assert!((today.close - (2e12 + 3.0 * 1e9) / 1e9).abs() < 1e-6);
    assert_eq!(
        candles[1].open_time,
        DAY_START - 86_400_000,
        "yesterday stands"
    );

    // Without them the chart is a day behind.
    let behind = candles_without_newest(&daily);
    assert_eq!(behind.last().unwrap().open_time, DAY_START - 86_400_000);
}

fn candles_without_newest(daily: &[GlobalPoint]) -> Vec<Kline> {
    candles(GlobalIndex::Total, Timeframe::DAY_1, 500, NOW, |_, _| {
        Ok(daily.to_vec())
    })
    .unwrap()
}

#[test]
fn only_readings_after_the_last_are_added_and_five_minute_charts_need_none() {
    let daily = vec![point(NOW - 600_000, 2e12)];
    let newest = five_minute_points(4);
    // The readings at or before the last one are the series' own.
    let points = with_newest(daily.clone(), Interval::Hour1, || newest.clone());
    assert_eq!(
        points.iter().map(|point| point.time_ms).collect::<Vec<_>>(),
        [NOW - 600_000, NOW - 300_000, NOW]
    );
    let untouched = with_newest(daily.clone(), Interval::Minutes5, || {
        panic!("a five-minute series is the finest spacing")
    });
    assert_eq!(untouched, daily);
    // Nothing in the series: the newest readings are all there is.
    assert_eq!(
        with_newest(Vec::new(), Interval::Day1, || newest.clone()),
        newest
    );
}

#[test]
fn a_chart_gets_no_more_candles_than_it_asked_for() {
    let points = five_minute_points(100);
    let candles = candles(
        GlobalIndex::Total,
        Timeframe::parse("5m").unwrap(),
        10,
        NOW,
        |_, _| Ok(points.clone()),
    )
    .unwrap();
    assert_eq!(candles.len(), 10);
    assert_eq!(candles.last().unwrap().open_time, NOW);
}

#[test]
fn a_timeframe_the_readings_cannot_build_is_refused_before_any_request() {
    for timeframe in ["1m", "2m", "7m", "22m"] {
        let timeframe = Timeframe::parse(timeframe).unwrap();
        let error = candles(GlobalIndex::Total, timeframe, 500, NOW, |_, _| {
            panic!("no request is made")
        })
        .unwrap_err();
        assert_eq!(
            error,
            ProviderError::unsupported(DataSource::CoinMarketCap, timeframe)
        );
        assert!(error.is_unsupported_interval());
    }
}

#[test]
fn candles_of_a_length_ask_for_the_readings_that_divide_it() {
    for (timeframe, expected) in [
        ("5m", Interval::Minutes5),
        ("15m", Interval::Minutes5),
        ("1h", Interval::Hour1),
        ("4h", Interval::Hour1),
        ("1D", Interval::Day1),
        ("1W", Interval::Day1),
    ] {
        let asked = Cell::new(None);
        let _ = candles(
            GlobalIndex::Total,
            Timeframe::parse(timeframe).unwrap(),
            500,
            NOW,
            |interval, _| {
                asked.set(Some(interval));
                Ok(five_minute_points(3))
            },
        );
        assert_eq!(asked.get(), Some(expected), "{timeframe}");
    }
}

#[test]
fn a_long_span_is_asked_for_in_a_few_windows_never_over_the_cap() {
    let asked = Cell::new(0);
    let _ = candles(
        GlobalIndex::Total,
        Timeframe::parse("12h").unwrap(),
        500,
        NOW,
        |interval, windows| {
            asked.set(windows.len());
            for window in windows {
                let (from, to) = window.query_seconds();
                assert!((to - from) / 3_600 < 2_200, "{}", interval.query());
            }
            Ok(five_minute_points(3))
        },
    );
    // 500 candles of 12 hours are 6,000 hourly readings: three windows, four when the grid cuts
    // the range across one more.
    assert!((3..=MAX_WINDOWS).contains(&asked.get()));
}

#[test]
fn no_readings_is_an_error_not_an_empty_chart() {
    let error = candles(GlobalIndex::Total, Timeframe::HOUR_1, 500, NOW, |_, _| {
        Ok(Vec::new())
    })
    .unwrap_err();
    assert_eq!(error, ProviderError::other("No CoinMarketCap data"));
}

#[test]
fn a_failed_load_is_the_charts_error() {
    let busy = ProviderError::CoinMarketCap(http::Error::Status(500));
    let error = candles(GlobalIndex::Total, Timeframe::HOUR_1, 500, NOW, |_, _| {
        Err(busy.clone())
    })
    .unwrap_err();
    assert_eq!(error, busy);
}

/// The kept windows over a scratch directory, and a clock the test moves.
struct Kept {
    _temp: TempCache,
    clock: Arc<AtomicI64>,
    cache: LookupCache,
    fetches: Cell<u32>,
}

impl Kept {
    fn new() -> Self {
        let temp = TempCache::new();
        let disk: &'static DiskCache = Box::leak(Box::new(DiskCache::at(temp.path())));
        let clock = Arc::new(AtomicI64::new(NOW));
        let reader = Arc::clone(&clock);
        let cache = LookupCache::for_tests(disk, Box::new(move || reader.load(Ordering::Relaxed)));
        Self {
            _temp: temp,
            clock,
            cache,
            fetches: Cell::new(0),
        }
    }

    fn advance(&self, by: Duration) {
        self.clock
            .fetch_add(i64::try_from(by.as_millis()).unwrap(), Ordering::Relaxed);
    }

    fn now(&self) -> i64 {
        self.clock.load(Ordering::Relaxed)
    }

    /// The readings of the newest window, asking `answer` when the network is needed.
    fn newest(
        &self,
        answer: Result<Vec<GlobalPoint>, ProviderError>,
    ) -> Result<Vec<GlobalPoint>, ProviderError> {
        self.windows(Interval::Hour1, &[0], answer)
    }

    /// The readings of the windows `back` windows before the one holding now.
    fn windows(
        &self,
        interval: Interval,
        back: &[i64],
        answer: Result<Vec<GlobalPoint>, ProviderError>,
    ) -> Result<Vec<GlobalPoint>, ProviderError> {
        let current = interval.windows(self.now(), self.now())[0];
        let length = current.end_ms - current.start_ms;
        let windows: Vec<Window> = back
            .iter()
            .map(|back| Window {
                index: current.index - back,
                start_ms: current.start_ms - back * length,
                end_ms: current.end_ms - back * length,
            })
            .collect();
        windows::readings(&self.cache, interval, &windows, self.now(), |_, _| {
            self.fetches.set(self.fetches.get() + 1);
            answer.clone()
        })
    }
}

#[test]
fn the_newest_readings_are_asked_for_by_a_short_range_and_kept_for_a_minute() {
    let kept = Kept::new();
    let points = vec![point(NOW - 300_000, 2e12), point(NOW, 2e12)];
    let ranges = std::cell::RefCell::new(Vec::new());
    let newest = |kept: &Kept, answer: Result<Vec<GlobalPoint>, ProviderError>| {
        windows::newest(&kept.cache, kept.now(), |interval, window| {
            ranges.borrow_mut().push((interval, window.query_seconds()));
            answer.clone()
        })
    };
    assert_eq!(newest(&kept, Ok(points.clone())), points);
    assert_eq!(
        *ranges.borrow(),
        [(
            Interval::Minutes5,
            ((NOW - 30 * 60_000) / 1_000, NOW / 1_000 - 1)
        )]
    );
    kept.advance(Duration::from_secs(59));
    assert_eq!(newest(&kept, Ok(Vec::new())), points, "kept for a minute");
    assert_eq!(ranges.borrow().len(), 1);
    kept.advance(Duration::from_secs(2));
    let later = vec![point(kept.now(), 3e12)];
    assert_eq!(newest(&kept, Ok(later.clone())), later);
    // A failed request adds nothing, and an old copy is not offered in its place.
    kept.advance(Duration::from_secs(120));
    let failed = newest(&kept, Err(ProviderError::other("down")));
    assert!(failed.is_empty());
}

#[test]
fn a_window_that_is_over_is_fetched_once() {
    let kept = Kept::new();
    let points = vec![point(NOW - 86_400_000 * 100, 2e12)];
    for _ in 0..3 {
        // Two windows back: over and settled long ago.
        assert_eq!(
            kept.windows(Interval::Hour1, &[2], Ok(points.clone())),
            Ok(points.clone())
        );
        kept.advance(Duration::from_secs(86_400));
    }
    assert_eq!(kept.fetches.get(), 1);
}

#[test]
fn the_window_still_filling_is_asked_again_once_its_time_is_up() {
    let kept = Kept::new();
    let first = vec![point(NOW, 2e12)];
    let second = vec![point(NOW, 2e12), point(NOW + 3_600_000, 2.1e12)];
    assert_eq!(kept.newest(Ok(first.clone())), Ok(first.clone()));
    kept.advance(Duration::from_secs(120));
    assert_eq!(kept.newest(Ok(second.clone())), Ok(first));
    assert_eq!(kept.fetches.get(), 1);
    kept.advance(Duration::from_secs(300));
    assert_eq!(kept.newest(Ok(second.clone())), Ok(second));
    assert_eq!(kept.fetches.get(), 2);
}

#[test]
fn a_window_kept_while_it_filled_is_fetched_again_once_it_has_settled() {
    let kept = Kept::new();
    let partial = vec![point(NOW, 2e12)];
    let whole = vec![point(NOW, 2e12), point(NOW + 3_600_000, 2.1e12)];
    assert_eq!(
        kept.windows(Interval::Hour1, &[0], Ok(partial.clone())),
        Ok(partial)
    );
    // The window ended and a day passed: what was kept then was not the whole of it.
    kept.advance(Duration::from_secs(2_000 * 3_600 + 2 * 86_400));
    assert_eq!(
        kept.windows(Interval::Hour1, &[1], Ok(whole.clone())),
        Ok(whole)
    );
    assert_eq!(kept.fetches.get(), 2);
}

#[test]
fn a_failed_request_falls_back_on_what_was_kept_however_old() {
    let kept = Kept::new();
    let points = vec![point(NOW, 2e12)];
    assert_eq!(kept.newest(Ok(points.clone())), Ok(points.clone()));
    kept.advance(Duration::from_secs(3_600));
    let busy = ProviderError::CoinMarketCap(http::Error::Status(500));
    assert_eq!(kept.newest(Err(busy)), Ok(points));
    assert_eq!(kept.fetches.get(), 2);
}

#[test]
fn a_failed_request_with_nothing_kept_is_the_error() {
    let kept = Kept::new();
    let limited = ProviderError::CoinMarketCap(http::Error::Status(429));
    let error = kept.newest(Err(limited.clone())).unwrap_err();
    assert_eq!(error, limited);
    assert!(error.is_rate_limited());
    // Nothing is kept of a failure: the next refresh asks again.
    let points = vec![point(NOW, 2e12)];
    assert_eq!(kept.newest(Ok(points.clone())), Ok(points));
}

#[test]
fn the_windows_join_oldest_first() {
    let kept = Kept::new();
    let older = vec![point(NOW - 9_000_000_000, 1e12)];
    assert_eq!(
        kept.windows(Interval::Day1, &[1], Ok(older.clone())),
        Ok(older)
    );
    let both = kept.windows(Interval::Day1, &[1, 0], Ok(vec![point(NOW, 2e12)]));
    let both = both.unwrap();
    assert_eq!(both.len(), 2);
    assert!(both[0].time_ms < both[1].time_ms);
}

/// Needs the network: each index at 5 minutes, 1 hour and 1 day from CoinMarketCap's website data.
/// Run with `--ignored --nocapture` to see the newest values.
#[test]
#[ignore = "needs the network"]
fn live_each_index_at_five_minutes_one_hour_and_one_day() {
    use crate::service::MarketSource;
    use crate::service::market::cmc::COIN_MARKET_CAP;

    let timeframes = [
        Timeframe::parse("5m").unwrap(),
        Timeframe::HOUR_1,
        Timeframe::DAY_1,
    ];
    for timeframe in timeframes {
        let mut newest = Vec::new();
        for index in GlobalIndex::ALL {
            let candles = COIN_MARKET_CAP
                .fetch_klines(index.symbol(), timeframe, 500)
                .unwrap_or_else(|error| panic!("{} {timeframe}: {error}", index.symbol()));
            let last = candles.last().unwrap();
            println!(
                "{:<7} {:>3}: {:>4} candles, newest {} close {:.2}",
                index.symbol(),
                timeframe.label(),
                candles.len(),
                last.open_time,
                last.close
            );
            assert!(
                candles
                    .windows(2)
                    .all(|pair| pair[0].open_time < pair[1].open_time)
            );
            // A candle is a sample, not a range of its own.
            assert_eq!(last.volume, 0.0);
            newest.push(last.close);
        }
        let [total, total2, total3, dominance] = newest[..] else {
            panic!("four indices");
        };
        assert!(
            total > total2 && total2 > total3 && total3 > 0.0,
            "{newest:?}"
        );
        assert!((40.0..70.0).contains(&dominance), "BTC.D {dominance}");
    }
    // One minute is finer than any reading.
    assert!(
        COIN_MARKET_CAP
            .fetch_klines("TOTAL", Timeframe::MINUTE_1, 500)
            .unwrap_err()
            .is_unsupported_interval()
    );
}
