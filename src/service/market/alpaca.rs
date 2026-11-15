use super::catalog::Catalog;
use crate::model::Kline;
use crate::service::{
    MarketSource, ProviderError, SearchResult, bucket_start_ms, fold_calendar,
    next_bucket_start_ms, rank_results,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const BARS_URL: &str = "https://data.alpaca.markets/v2/stocks";
const DAY_MS: i64 = 86_400_000;
const MONTH_SECS: i64 = 2_592_000;

pub struct Alpaca;

pub const ALPACA: Alpaca = Alpaca;

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

/// Quarterly and yearly bars don't exist on Alpaca; they fold monthly ones.
fn monthly_fold(interval: &str) -> Option<i64> {
    match interval {
        "3M" => Some(7_776_000),
        "1Y" => Some(31_536_000),
        _ => None,
    }
}

/// A bar length Alpaca serves natively: minutes (1 to 59), hours (1 to 23), a day, a week or a
/// month. Any other interval is refused, never answered with a nearby one.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Bar {
    Minutes(i64),
    Hours(i64),
    Day,
    Week,
    Month,
}

impl Bar {
    fn parse(interval: &str) -> Result<Self, ProviderError> {
        let count = |suffix: char, limit: i64| {
            interval
                .strip_suffix(suffix)
                .and_then(|digits| digits.parse::<i64>().ok())
                .filter(|count| (1..limit).contains(count))
        };
        match interval {
            "1d" => Some(Self::Day),
            "1w" => Some(Self::Week),
            "1M" => Some(Self::Month),
            _ => count('m', 60)
                .map(Self::Minutes)
                .or_else(|| count('h', 24).map(Self::Hours)),
        }
        .ok_or_else(|| ProviderError::other(format!("Alpaca has no {interval} candles")))
    }

    /// The `timeframe` Alpaca's API takes.
    fn token(self) -> String {
        match self {
            Self::Minutes(count) => format!("{count}Min"),
            Self::Hours(count) => format!("{count}Hour"),
            Self::Day => "1Day".to_owned(),
            Self::Week => "1Week".to_owned(),
            Self::Month => "1Month".to_owned(),
        }
    }

    fn seconds(self) -> i64 {
        match self {
            Self::Minutes(count) => count * 60,
            Self::Hours(count) => count * 3_600,
            Self::Day => 86_400,
            Self::Week => 604_800,
            Self::Month => MONTH_SECS,
        }
    }

    /// How much wall-clock time a bar's worth of data spans on average. Markets are open about
    /// 6.5 of 24 hours and five days of seven, so intraday bars are far apart in time.
    fn gap_factor(self) -> f64 {
        match self {
            Self::Minutes(_) | Self::Hours(_) => 6.0,
            Self::Day => 2.0,
            Self::Week => 1.6,
            Self::Month => 1.5,
        }
    }

    /// The shortest window worth asking for: a long weekend or a holiday holds no intraday bar,
    /// and a request for the newest few bars must still reach the last session.
    fn min_lookback_ms(self) -> i64 {
        match self {
            Self::Minutes(_) | Self::Hours(_) => 7 * DAY_MS,
            _ => 0,
        }
    }

    /// Where a bar that opens at `open_ms` ends: a millisecond before the next opens. Alpaca
    /// stamps days, weeks and months at New York midnight, so the offset into the UTC day is
    /// kept; across a clock change it lands an hour off New York midnight.
    fn close_ms(self, open_ms: i64) -> i64 {
        let next = match self {
            Self::Month => {
                let day_start = open_ms.div_euclid(DAY_MS) * DAY_MS;
                let month_start = bucket_start_ms(day_start, MONTH_SECS);
                next_bucket_start_ms(month_start, MONTH_SECS) + (open_ms - day_start)
            }
            _ => open_ms + self.seconds() * 1_000,
        };
        next - 1
    }
}

/// Where a request for the newest `requested` bars starts looking: far enough back to hold them
/// through the gaps between sessions, and never before Alpaca's data begins.
fn lookback_start_ms(bar: Bar, requested: i64, now_ms: i64) -> i64 {
    let span = (bar.seconds() as f64 * requested.max(1) as f64 * bar.gap_factor() * 1_000.0) as i64;
    (now_ms - span.max(bar.min_lookback_ms())).max(EARLIEST_MS)
}

impl MarketSource for Alpaca {
    fn fetch_klines(
        &self,
        symbol: &str,
        interval: &str,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        let fold_seconds = monthly_fold(interval);
        let bar = Bar::parse(if fold_seconds.is_some() {
            "1M"
        } else {
            interval
        })?;
        let requested = fold_seconds
            .map(|_| (i64::from(limit) + 1) * 12)
            .unwrap_or(i64::from(limit));
        let bars = fetch_bars(
            symbol,
            bar,
            lookback_start_ms(bar, requested, crate::service::now_ms()),
            None,
            requested as usize,
        )?;
        let shown = match fold_seconds {
            Some(seconds) => fold_calendar(&bars, seconds, bars.len() >= requested as usize),
            None => bars,
        };
        let start = shown.len().saturating_sub(limit as usize);
        Ok(shown[start..].to_vec())
    }

    fn fetch_klines_range(
        &self,
        symbol: &str,
        interval: &str,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, ProviderError> {
        let fold_seconds = monthly_fold(interval);
        let bar = Bar::parse(if fold_seconds.is_some() {
            "1M"
        } else {
            interval
        })?;
        // Alpaca's data starts in the 2010s; an earlier start only costs it a refusal.
        let start = start_ms.max(EARLIEST_MS);
        let wanted = max * fold_seconds.map_or(1, |seconds| (seconds / 2_592_000) as usize + 1);
        let bars: Vec<Kline> = fetch_bars(symbol, bar, start, Some(end_ms), wanted)?
            .into_iter()
            .filter(|bar| bar.open_time < end_ms)
            .collect();
        let shown = match fold_seconds {
            Some(seconds) => fold_calendar(&bars, seconds, bars.len() >= wanted),
            None => bars,
        };
        let skip = shown.len().saturating_sub(max);
        Ok(shown[skip..].to_vec())
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

/// The newest `limit` bars of `bar` that open from `start_ms` to `end_ms` (or now), oldest
/// first.
fn fetch_bars(
    symbol: &str,
    bar: Bar,
    start_ms: i64,
    end_ms: Option<i64>,
    limit: usize,
) -> Result<Vec<Kline>, ProviderError> {
    let (key, secret) = credentials()?;
    let mut url = format!(
        "{BARS_URL}/{}/bars?timeframe={}&start={}&limit={}&adjustment=all&feed=iex&sort=desc",
        symbol.to_uppercase(),
        bar.token(),
        crate::format::iso8601(start_ms),
        limit.clamp(1, 10_000)
    );
    if let Some(end_ms) = end_ms {
        url.push_str(&format!("&end={}", crate::format::iso8601(end_ms)));
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
            "No data is available for {} on Alpaca's free IEX feed.",
            symbol.to_uppercase()
        )));
    }
    bars.sort_by_key(|candle| candle.open_time);
    Ok(bars)
}

/// `{"bars": [{"t": ISO, "o", "h", "l", "c", "v", "vw"}]}` newest first.
fn parse_bars(body: &str, length: Bar) -> Result<Vec<Kline>, ProviderError> {
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
            close_time: length.close_ms(t),
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
        let folded = fold_calendar(&bars, 7_776_000, false);
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
        let bars = parse_bars(&one_bar("2026-10-09T14:00:00Z"), Bar::Hours(1)).unwrap();
        assert_eq!(bars[0].close_time, bars[0].open_time + 3_600_000 - 1);
    }

    #[test]
    fn a_daily_bar_keeps_alpacas_new_york_midnight_offset() {
        let bars = parse_bars(&one_bar("2026-10-09T04:00:00Z"), Bar::Day).unwrap();
        assert_eq!(bars[0].open_time, day_ms(2026, 10, 9) + 4 * 3_600_000);
        assert_eq!(bars[0].close_time, day_ms(2026, 10, 10) + 4 * 3_600_000 - 1);
    }

    #[test]
    fn a_monthly_bar_closes_before_the_next_month_opens() {
        let bars = parse_bars(&one_bar("2026-11-01T04:00:00Z"), Bar::Month).unwrap();
        assert_eq!(bars[0].close_time, day_ms(2026, 12, 1) + 4 * 3_600_000 - 1);
        let february = parse_bars(&one_bar("2024-02-01T05:00:00Z"), Bar::Month).unwrap();
        assert_eq!(
            february[0].close_time,
            day_ms(2024, 3, 1) + 5 * 3_600_000 - 1,
            "a leap February has 29 days"
        );
        let december = parse_bars(&one_bar("2026-12-01T05:00:00Z"), Bar::Month).unwrap();
        assert_eq!(
            december[0].close_time,
            day_ms(2027, 1, 1) + 5 * 3_600_000 - 1
        );
    }

    #[test]
    fn a_weekly_bar_spans_seven_days() {
        let bars = parse_bars(&one_bar("2026-10-05T04:00:00Z"), Bar::Week).unwrap();
        assert_eq!(bars[0].close_time, bars[0].open_time + 7 * DAY_MS - 1);
    }

    #[test]
    fn intervals_map_to_alpacas_own_tokens() {
        let token = |interval| Bar::parse(interval).unwrap().token();
        assert_eq!(token("1m"), "1Min");
        assert_eq!(token("5m"), "5Min");
        assert_eq!(token("15m"), "15Min");
        assert_eq!(token("30m"), "30Min");
        assert_eq!(token("59m"), "59Min");
        assert_eq!(token("1h"), "1Hour");
        assert_eq!(token("4h"), "4Hour");
        assert_eq!(token("23h"), "23Hour");
        assert_eq!(token("1d"), "1Day");
        assert_eq!(token("1w"), "1Week");
        assert_eq!(token("1M"), "1Month");
    }

    #[test]
    fn an_interval_alpaca_has_no_bars_for_is_refused_not_replaced() {
        for interval in [
            "0m", "60m", "24h", "2d", "2w", "3M", "1Y", "1s", "m", "", "daily",
        ] {
            let error = Bar::parse(interval).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("Alpaca has no {interval} candles")
            );
        }
    }

    #[test]
    fn the_newest_few_intraday_bars_look_back_over_a_long_weekend() {
        let now = day_ms(2026, 10, 12) + 12 * 3_600_000;
        let start = lookback_start_ms(Bar::Hours(1), 4, now);
        assert_eq!(now - start, 7 * DAY_MS);
        let daily = lookback_start_ms(Bar::Day, 4, now);
        assert_eq!(now - daily, 8 * DAY_MS);
    }

    #[test]
    fn a_deep_lookback_stops_where_alpacas_data_begins() {
        let now = day_ms(2026, 10, 12);
        assert_eq!(lookback_start_ms(Bar::Month, 6_012, now), EARLIEST_MS);
    }
}
