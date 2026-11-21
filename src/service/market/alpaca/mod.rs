use super::candles::plan;
mod ticker;

use super::catalog::Catalog;
use super::venue::Venue;
use crate::model::market::bucket;
use crate::model::market::feed::AlpacaFeed;
use crate::model::{DataSource, Kline, TimeUnit, Timeframe};
use crate::service::watchlist_quotes::provider::rejected;
use crate::service::watchlist_quotes::{Outcome, QuoteFailure, QuoteRequest};
use crate::service::{MarketSource, ProviderError, SearchResult, rank_results};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const BARS_URL: &str = "https://data.alpaca.markets/v2/stocks";
const DAY_MS: i64 = 86_400_000;
/// How far behind real time the free plan serves SIP data. A SIP request ending later than this
/// is refused on that plan, one without an end is silently cut here. A minute of margin over
/// Alpaca's 15.
const SIP_DELAY_MS: i64 = 16 * 60_000;

pub struct Alpaca;

pub const ALPACA: Alpaca = Alpaca;

impl Venue for Alpaca {
    fn candles(&self) -> &dyn MarketSource {
        self
    }

    fn rest_quotes(&self, requests: &[QuoteRequest]) -> Result<Outcome, QuoteFailure> {
        ticker::fetch_quotes(requests).map(Outcome::from_quotes)
    }

    fn classify_status(&self, code: u16) -> Option<QuoteFailure> {
        match code {
            401 | 403 => Some(QuoteFailure::Failed(
                "Alpaca refused your API key. Check it in Settings".to_owned(),
            )),
            400 => Some(rejected(400)),
            _ => None,
        }
    }
}

/// What a search or a fetch says when no Alpaca keys are saved.
pub const MISSING_KEY: &str = "Set up your Alpaca API key before using stock charts.";

fn credentials() -> Result<(String, String), ProviderError> {
    crate::service::keychain::api_keys::alpaca_credentials().ok_or(ProviderError::MissingKey)
}

/// Settings ▸ Alpaca ▸ Test Connection: one bar of one stock, asked for with the given keys (the
/// ones typed in the fields, saved or not), which is what stock charts need the keys for.
pub fn test_connection(key_id: &str, secret_key: &str) -> Result<(), ProviderError> {
    let url = format!("{BARS_URL}/AAPL/bars?timeframe=1Day&limit=1&feed=iex");
    crate::service::net::http::get_text(
        &url,
        &[
            ("APCA-API-KEY-ID", key_id.trim()),
            ("APCA-API-SECRET-KEY", secret_key.trim()),
        ],
        crate::service::net::http::Retry::Never,
    )
    .map(|_| ())
    .map_err(ProviderError::Alpaca)
}

/// The most bars one request returns.
const MAX_BARS: usize = 10_000;

/// The `timeframe` token Alpaca's API takes for a length it serves natively.
fn token(timeframe: Timeframe) -> String {
    let count = timeframe.count();
    match timeframe.unit() {
        TimeUnit::Minute => format!("{count}Min"),
        TimeUnit::Hour => format!("{count}Hour"),
        TimeUnit::Day => format!("{count}Day"),
        TimeUnit::Week => format!("{count}Week"),
        TimeUnit::Month => format!("{count}Month"),
    }
}

/// How much wall-clock time a bar's worth of data spans on average. Markets are open about
/// 6.5 of 24 hours and five days of seven, so intraday bars are far apart in time.
fn gap_factor(timeframe: Timeframe) -> f64 {
    match timeframe.unit() {
        TimeUnit::Minute | TimeUnit::Hour => 6.0,
        TimeUnit::Day => 2.0,
        TimeUnit::Week => 1.6,
        TimeUnit::Month => 1.5,
    }
}

/// The shortest window worth asking for: a long weekend or a holiday holds no intraday bar,
/// and a request for the newest few bars must still reach the last session.
fn min_lookback_ms(timeframe: Timeframe) -> i64 {
    if timeframe.is_intraday() {
        7 * DAY_MS
    } else {
        0
    }
}

/// Where a bar of `timeframe` that opens at `open_ms` ends: a millisecond before the next opens.
/// Alpaca stamps days, weeks and months at New York midnight, so the offset into the UTC day is
/// kept; across a clock change it lands an hour off New York midnight.
fn close_ms(timeframe: Timeframe, open_ms: i64) -> i64 {
    let next = match timeframe.unit() {
        TimeUnit::Month => {
            let day_start = open_ms.div_euclid(DAY_MS) * DAY_MS;
            bucket::next_bucket_start_ms(timeframe, day_start) + (open_ms - day_start)
        }
        _ => open_ms + timeframe.nominal_ms(),
    };
    next - 1
}

/// Where a request for the newest `requested` bars of `timeframe` starts looking: far enough
/// back to hold them through the gaps between sessions, and never before Alpaca's data begins.
fn lookback_start_ms(timeframe: Timeframe, requested: i64, now_ms: i64) -> i64 {
    let span =
        (timeframe.nominal_ms() as f64 * requested.max(1) as f64 * gap_factor(timeframe)) as i64;
    (now_ms - span.max(min_lookback_ms(timeframe))).max(EARLIEST_MS)
}

impl MarketSource for Alpaca {
    fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        let plan = plan(DataSource::Alpaca, timeframe)?;
        let requested = plan.base_count(limit as usize).min(MAX_BARS);
        let since = lookback_start_ms(plan.base(), requested as i64, crate::clock::now_ms());
        let bars = fetch_bars(symbol, plan.base(), since, None, requested)?;
        Ok(plan.shape_since(bars, requested, since, limit as usize))
    }

    fn fetch_klines_range(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, ProviderError> {
        let plan = plan(DataSource::Alpaca, timeframe)?;
        // Alpaca's data starts in the 2010s; an earlier start only costs it a refusal.
        let start = start_ms.max(EARLIEST_MS);
        let requested = plan.base_count(max).min(MAX_BARS);
        let bars: Vec<Kline> = fetch_bars(symbol, plan.base(), start, Some(end_ms), requested)?
            .into_iter()
            .filter(|bar| bar.open_time < end_ms)
            .collect();
        Ok(plan.shape_since(bars, requested, start, max))
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        // Credentials first: with none saved, a cached list must not make search seem available.
        let (key, secret) = credentials()?;
        let assets = CATALOG.get(|| download_assets(&key, &secret))?;

        let needle = query.trim().to_uppercase();
        let results: Vec<SearchResult> = assets
            .iter()
            .filter(|asset| {
                asset.symbol.to_uppercase().contains(&needle)
                    || asset.name.to_uppercase().contains(&needle)
            })
            .map(|asset| {
                SearchResult::new(
                    crate::model::DataSource::Alpaca,
                    format!(
                        "{}{}{}",
                        asset.symbol,
                        crate::service::NAME_SEPARATOR,
                        asset.name
                    ),
                    asset.symbol.clone(),
                )
            })
            .collect();
        Ok(rank_results(results, query).into_iter().take(30).collect())
    }
}

/// One tradable US equity from `/v2/assets`: all the search needs of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Asset {
    symbol: String,
    name: String,
}

/// Every tradable asset (the response is tens of megabytes), downloaded at most once a day.
static CATALOG: Catalog<Asset> = Catalog::new("catalog_alpaca");

/// Forgets the list held in memory (after the disk cache was cleared).
pub(in crate::service) fn clear_catalog() {
    CATALOG.clear_memory();
}

fn download_assets(key: &str, secret: &str) -> Result<Vec<Asset>, ProviderError> {
    let url = "https://paper-api.alpaca.markets/v2/assets?status=active&asset_class=us_equity";
    let body = crate::service::net::http::get_text_with_limit(
        url,
        &[("APCA-API-KEY-ID", key), ("APCA-API-SECRET-KEY", secret)],
        crate::service::net::http::Retry::Transient,
        32 * 1024 * 1024,
    )
    .map_err(ProviderError::Alpaca)?;
    parse_assets(&body)
}

fn parse_assets(body: &str) -> Result<Vec<Asset>, ProviderError> {
    let value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    Ok(value
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|asset| asset.get("tradable").and_then(Value::as_bool) == Some(true))
        .filter_map(|asset| {
            Some(Asset {
                symbol: asset.get("symbol")?.as_str()?.to_owned(),
                name: asset.get("name")?.as_str()?.to_owned(),
            })
        })
        .collect())
}

/// 2000-01-01.
const EARLIEST_MS: i64 = 946_684_800_000;

/// The window a request on `feed` asks Alpaca for, as `(start, end)`; `None` when no request is
/// worth making: the caller's range is empty (`end` not after `start`), or a SIP `start` falls
/// inside the free plan's delay.
///
/// Alpaca refuses a SIP `start` that recent on the free plan (403, with or without an `end`,
/// measured on `1Day` and `1Min`). SIP only serves bars of a day and longer, and such a bar opens
/// at the session start, so a bar opening at or after a `start` in the last minutes does not exist
/// yet on any plan: skipping the request loses nothing.
///
/// A SIP `end` inside the free plan's delay is refused (403), so it is left out and Alpaca applies
/// the account's own entitlement: real time on a paid plan, cut at the delay on Basic. Capping the
/// `end` instead would hold a paid account 16 minutes behind, and differently from a request with
/// no `end`. The caller drops the bars its range does not hold.
fn bars_window(
    feed: AlpacaFeed,
    start_ms: i64,
    end_ms: Option<i64>,
    now_ms: i64,
) -> Option<(i64, Option<i64>)> {
    if end_ms.is_some_and(|end| end <= start_ms) {
        return None;
    }
    if feed == AlpacaFeed::Sip && start_ms > now_ms - SIP_DELAY_MS {
        return None;
    }
    let end = match (feed, end_ms) {
        (AlpacaFeed::Sip, Some(end)) if end > now_ms - SIP_DELAY_MS => None,
        (_, end) => end,
    };
    Some((start_ms, end))
}

/// The newest `limit` bars of `bar` that open from `start_ms` to `end_ms` (or now), oldest
/// first.
fn fetch_bars(
    symbol: &str,
    bar: Timeframe,
    start_ms: i64,
    end_ms: Option<i64>,
    limit: usize,
) -> Result<Vec<Kline>, ProviderError> {
    let (key, secret) = credentials()?;
    let feed = AlpacaFeed::for_bars(bar);
    let Some((start, end)) = bars_window(feed, start_ms, end_ms, crate::clock::now_ms()) else {
        return Ok(Vec::new());
    };
    let mut url = format!(
        "{BARS_URL}/{}/bars?timeframe={}&start={}&limit={}&adjustment=all&feed={}&sort=desc",
        symbol.to_uppercase(),
        token(bar),
        crate::format::iso8601(start),
        limit.clamp(1, MAX_BARS),
        feed.query()
    );
    if let Some(end) = end {
        url.push_str(&format!("&end={}", crate::format::iso8601(end)));
    }
    let body = crate::service::net::http::get_text(
        &url,
        &[("APCA-API-KEY-ID", &key), ("APCA-API-SECRET-KEY", &secret)],
        crate::service::net::http::Retry::Transient,
    )
    .map_err(ProviderError::Alpaca)?;
    let mut bars = parse_bars(&body, bar)?;
    // The newest page of a market with no bars at all is an error; an older page with none is
    // the start of its history.
    if bars.is_empty() && end_ms.is_none() {
        return Err(ProviderError::Other(format!(
            "No data is available for {} on Alpaca's free {} feed.",
            symbol.to_uppercase(),
            feed.query().to_uppercase()
        )));
    }
    bars.sort_by_key(|candle| candle.open_time);
    Ok(bars)
}

/// `{"bars": [{"t": ISO, "o", "h", "l", "c", "v", "vw"}]}` newest first.
fn parse_bars(body: &str, length: Timeframe) -> Result<Vec<Kline>, ProviderError> {
    let value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    let bars = value
        .get("bars")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut out = Vec::with_capacity(bars.len());
    for bar in bars {
        let (Some(t), Some(o), Some(h), Some(l), Some(c)) = (
            bar.get("t").and_then(Value::as_str).and_then(parse_iso8601),
            bar.get("o").and_then(Value::as_f64),
            bar.get("h").and_then(Value::as_f64),
            bar.get("l").and_then(Value::as_f64),
            bar.get("c").and_then(Value::as_f64),
        ) else {
            continue;
        };
        let volume = bar.get("v").and_then(Value::as_f64).unwrap_or(0.0);
        out.push(Kline {
            open_time: t,
            close_time: close_ms(length, t),
            open: o,
            high: h,
            low: l,
            close: c,
            volume,
        });
    }
    Ok(out)
}

/// Alpaca stamps ISO-8601 with optional fractional seconds; milliseconds are plenty.
fn parse_iso8601(text: &str) -> Option<i64> {
    let text = text.trim_end_matches('Z');
    let (date, time) = text.split_once('T')?;
    let mut d = date.split('-');
    let year: i64 = d.next()?.parse().ok()?;
    let month: i64 = d.next()?.parse().ok()?;
    let day: i64 = d.next()?.parse().ok()?;
    let mut t = time.split(':');
    let hour: i64 = t.next()?.parse().ok()?;
    let minute: i64 = t.next()?.parse().ok()?;
    let second: i64 = t.next()?.split('.').next()?.parse().ok()?;
    let days = crate::format::days_from_civil(year, month, day);
    Some((days * 86_400 + hour * 3_600 + minute * 60 + second) * 1_000)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_catalog_keeps_only_tradable_named_assets() {
        let body = r#"[
            {"symbol":"AAPL","name":"Apple Inc. Common Stock","tradable":true},
            {"symbol":"HALT","name":"Halted Corp","tradable":false},
            {"symbol":"NONAME","tradable":true}
        ]"#;
        let assets = parse_assets(body).expect("assets");
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].symbol, "AAPL");
        assert_eq!(assets[0].name, "Apple Inc. Common Stock");
        assert!(parse_assets("[").is_err());
    }

    use super::*;

    #[test]
    fn parses_iso_timestamps() {
        assert_eq!(
            parse_iso8601("2026-10-09T00:00:00Z"),
            Some(1_791_504_000_000)
        );
        assert_eq!(
            parse_iso8601("2026-10-09T00:00:00.123456Z"),
            Some(1_791_504_000_000)
        );
    }

    #[test]
    fn folds_monthly_into_quarterly() {
        let jan = crate::format::days_from_civil(2026, 1, 1) * 86_400_000;
        let feb = crate::format::days_from_civil(2026, 2, 1) * 86_400_000;
        let apr = crate::format::days_from_civil(2026, 4, 1) * 86_400_000;
        let bars = vec![
            Kline {
                open_time: jan,
                close_time: jan,
                open: 10.0,
                high: 11.0,
                low: 9.0,
                close: 10.5,
                volume: 1.0,
            },
            Kline {
                open_time: feb,
                close_time: feb,
                open: 10.5,
                high: 12.0,
                low: 10.0,
                close: 11.0,
                volume: 2.0,
            },
            Kline {
                open_time: apr,
                close_time: apr,
                open: 11.0,
                high: 11.5,
                low: 10.8,
                close: 11.2,
                volume: 1.0,
            },
        ];
        let folded = bucket::fold(&bars, Timeframe::MONTH_3, false);
        assert_eq!(folded.len(), 2);
        assert_eq!(folded[0].open_time, jan);
        assert_eq!(folded[0].high, 12.0);
        assert_eq!(folded[0].low, 9.0);
        assert_eq!(folded[1].open_time, apr);
    }

    fn day_ms(year: i64, month: i64, day: i64) -> i64 {
        crate::format::days_from_civil(year, month, day) * DAY_MS
    }

    fn one_bar(time: &str) -> String {
        format!(r#"{{"bars":[{{"t":"{time}","o":1,"h":2,"l":0.5,"c":1.5,"v":10}}]}}"#)
    }

    #[test]
    fn an_hourly_bar_closes_a_millisecond_before_the_next_hour() {
        let bars = parse_bars(&one_bar("2026-10-09T14:00:00Z"), Timeframe::HOUR_1).unwrap();
        assert_eq!(bars[0].close_time, bars[0].open_time + 3_600_000 - 1);
    }

    #[test]
    fn a_daily_bar_keeps_alpacas_new_york_midnight_offset() {
        let bars = parse_bars(&one_bar("2026-10-09T04:00:00Z"), Timeframe::DAY_1).unwrap();
        assert_eq!(bars[0].open_time, day_ms(2026, 10, 9) + 4 * 3_600_000);
        assert_eq!(bars[0].close_time, day_ms(2026, 10, 10) + 4 * 3_600_000 - 1);
    }

    #[test]
    fn a_monthly_bar_closes_before_the_next_month_opens() {
        let bars = parse_bars(&one_bar("2026-11-01T04:00:00Z"), Timeframe::MONTH_1).unwrap();
        assert_eq!(bars[0].close_time, day_ms(2026, 12, 1) + 4 * 3_600_000 - 1);
        let february = parse_bars(&one_bar("2024-02-01T05:00:00Z"), Timeframe::MONTH_1).unwrap();
        assert_eq!(
            february[0].close_time,
            day_ms(2024, 3, 1) + 5 * 3_600_000 - 1,
            "a leap February has 29 days"
        );
        let december = parse_bars(&one_bar("2026-12-01T05:00:00Z"), Timeframe::MONTH_1).unwrap();
        assert_eq!(
            december[0].close_time,
            day_ms(2027, 1, 1) + 5 * 3_600_000 - 1
        );
    }

    #[test]
    fn a_weekly_bar_spans_seven_days() {
        let bars = parse_bars(&one_bar("2026-10-05T04:00:00Z"), Timeframe::WEEK_1).unwrap();
        assert_eq!(bars[0].close_time, bars[0].open_time + 7 * DAY_MS - 1);
    }

    #[test]
    fn natives_map_to_alpacas_own_tokens() {
        let tokens: Vec<String> = DataSource::Alpaca
            .natives()
            .iter()
            .map(|native| token(*native))
            .collect();
        assert_eq!(
            tokens,
            [
                "1Min", "5Min", "15Min", "30Min", "1Hour", "1Day", "1Week", "1Month"
            ]
        );
    }

    fn request(timeframe: &str, limit: usize) -> (String, usize) {
        let plan = plan(DataSource::Alpaca, Timeframe::parse(timeframe).unwrap()).unwrap();
        (token(plan.base()), plan.base_count(limit))
    }

    #[test]
    fn other_lengths_fold_the_coarsest_native_that_nests() {
        assert_eq!(request("1h", 100), ("1Hour".to_owned(), 100));
        assert_eq!(request("4h", 100), ("1Hour".to_owned(), 101 * 4));
        // 3Hour and 20Min exist on Alpaca but are not on our grid: a fold of the hour is.
        assert_eq!(request("3h", 100), ("1Hour".to_owned(), 101 * 3));
        assert_eq!(request("45m", 100), ("15Min".to_owned(), 101 * 3));
        assert_eq!(request("22m", 100), ("1Min".to_owned(), 101 * 22));
        assert_eq!(request("2d", 100), ("1Day".to_owned(), 101 * 2));
        assert_eq!(request("2w", 100), ("1Week".to_owned(), 101 * 2));
        assert_eq!(request("3M", 100), ("1Month".to_owned(), 101 * 3));
        assert_eq!(request("1Y", 500), ("1Month".to_owned(), 501 * 12));
    }

    #[test]
    fn the_newest_few_intraday_bars_look_back_over_a_long_weekend() {
        let now = day_ms(2026, 10, 12) + 12 * 3_600_000;
        let start = lookback_start_ms(Timeframe::HOUR_1, 4, now);
        assert_eq!(now - start, 7 * DAY_MS);
        let daily = lookback_start_ms(Timeframe::DAY_1, 4, now);
        assert_eq!(now - daily, 8 * DAY_MS);
    }

    #[test]
    fn a_deep_lookback_stops_where_alpacas_data_begins() {
        let now = day_ms(2026, 10, 12);
        assert_eq!(
            lookback_start_ms(Timeframe::MONTH_1, 6_012, now),
            EARLIEST_MS
        );
    }

    const MINUTE_MS: i64 = 60_000;

    fn now() -> i64 {
        day_ms(2026, 11, 19) + 20 * 3_600_000
    }

    #[test]
    fn a_sip_end_inside_the_delay_is_left_to_the_accounts_entitlement() {
        let start = now() - DAY_MS;
        for end in [now(), now() - 5 * MINUTE_MS, now() - SIP_DELAY_MS + 1] {
            assert_eq!(
                bars_window(AlpacaFeed::Sip, start, Some(end), now()),
                Some((start, None)),
                "an end {} minutes back",
                (now() - end) / MINUTE_MS
            );
        }
    }

    #[test]
    fn an_end_the_plan_already_serves_is_sent() {
        let start = now() - 3 * DAY_MS;
        let older = now() - 20 * MINUTE_MS;
        assert_eq!(
            bars_window(AlpacaFeed::Sip, start, Some(older), now()),
            Some((start, Some(older)))
        );
        assert_eq!(
            bars_window(AlpacaFeed::Sip, start, None, now()),
            Some((start, None))
        );
        assert_eq!(
            bars_window(AlpacaFeed::Iex, start, Some(now()), now()),
            Some((start, Some(now())))
        );
    }

    #[test]
    fn a_sip_start_inside_the_delay_asks_for_nothing() {
        // The free plan answers 403 to these, with an end or without one.
        for start in [now() - 5 * MINUTE_MS, now() - SIP_DELAY_MS + 1, now()] {
            for end in [None, Some(now())] {
                assert_eq!(
                    bars_window(AlpacaFeed::Sip, start, end, now()),
                    None,
                    "a start {} minutes back",
                    (now() - start) / MINUTE_MS
                );
            }
        }
    }

    #[test]
    fn a_sip_start_just_outside_the_delay_is_sent_and_iex_is_never_held_back() {
        let start = now() - SIP_DELAY_MS;
        assert_eq!(
            bars_window(AlpacaFeed::Sip, start, None, now()),
            Some((start, None))
        );
        let recent = now() - MINUTE_MS;
        assert_eq!(
            bars_window(AlpacaFeed::Iex, recent, None, now()),
            Some((recent, None))
        );
    }

    #[test]
    fn an_empty_range_asks_for_nothing() {
        let start = now() - DAY_MS;
        assert_eq!(
            bars_window(AlpacaFeed::Sip, start, Some(start), now()),
            None
        );
        assert_eq!(
            bars_window(AlpacaFeed::Iex, start, Some(start - 1), now()),
            None
        );
        assert_eq!(
            bars_window(
                AlpacaFeed::Sip,
                now() - 10 * MINUTE_MS,
                Some(now() - 17 * MINUTE_MS),
                now()
            ),
            None
        );
    }
}
