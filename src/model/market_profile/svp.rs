//! The session volume profile: for each session, how much volume traded at each price row, split into
//! up and down volume, with the point of control, value area and how both developed through the
//! session.
//!
//! A profile covers a **span** (a UTC day, or a custom window). It is built from lower-timeframe
//! candles (the finest of 1, 5, 15, 30 or 60 minutes the budget allows), never from the chart's own
//! bars. Each candle's volume is spread over the rows its high-to-low overlaps, in proportion to the
//! overlap (what TradingView documents for the chart type), so the **total volume is conserved**. Up
//! and down: a candle with the exchange's taker-buy split (Binance) contributes that split exactly;
//! any other candle is up volume when it closed above its open, down when below, and a doji takes the
//! side of the candle before it. A profile's quality says which.
//!
//! **Rows.** *Number of rows* (the default, 24) picks ticks per row as the session's range in ticks over
//! the number of rows, rounded, and the rows are laid on the global price grid, so the count can be
//! one more than asked (TradingView says extra rows may be added). *Ticks per row* is as it says.
//!
//! **Value area:** the volume profile's method ([`value_area::volume`]), 70 % by default.
//!
//! **Developing levels** are the point of control and value-area edges *as they stood* after each
//! candle, from the volume that had traded by then and no more: nothing of the finished profile is
//! read to draw them, so they are what a replay would have shown. (The row grid of a finished session
//! is chosen from its whole range, so the rows are the same at every step; the volume in them is not.)
//! At most [`MAX_DEVELOPING`] points are kept per session.

use super::flow::{CandleBook, FlowKline};
use super::footprint::SetStatus;
use super::price_grid::{MAX_ROWS, PriceGrid};
use super::quality::{DataQuality, Unsupported};
use super::sessions::{self, Session};
use super::tick::Tick;
use super::value_area;
use crate::model::Kline;
use crate::model::chart_type::{RowsLayout, SvpShape};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Most profiles a set builds (the newest).
pub const MAX_PROFILES: usize = 400;
/// Most developing points kept for a session.
pub const MAX_DEVELOPING: usize = 400;

/// The candle sizes a profile is cut into, finest first.
pub const SIZES: [(&str, i64); 5] = [
    ("1m", 60_000),
    ("5m", 300_000),
    ("15m", 900_000),
    ("30m", 1_800_000),
    ("1h", 3_600_000),
];

/// The candle size for `spans` within `cap` candles, and the oldest instant it reaches: the finest
/// size whose candles for every span fit, else the coarsest, reaching back as far as `cap` allows.
pub fn candle_plan(spans: &[Session], until_ms: i64, cap: usize) -> (&'static str, i64, i64) {
    let count_for = |size: i64, spans: &[Session]| -> i64 {
        spans
            .iter()
            .map(|span| (span.end_ms.min(until_ms) - span.start_ms).max(0) / size + 1)
            .sum()
    };
    for (interval, size) in SIZES {
        if count_for(size, spans) <= cap as i64 {
            return (
                interval,
                size,
                spans.first().map_or(0, |span| span.start_ms),
            );
        }
    }
    let (interval, size) = SIZES[SIZES.len() - 1];
    let mut total = 0;
    let mut from = spans.last().map_or(0, |span| span.start_ms);
    for span in spans.iter().rev() {
        total += count_for(size, std::slice::from_ref(span));
        if total > cap as i64 {
            break;
        }
        from = span.start_ms;
    }
    (interval, size, from)
}

/// One price row of one profile.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub row: i64,
    pub up: f64,
    pub down: f64,
}

impl Row {
    pub fn total(&self) -> f64 {
        self.up + self.down
    }

    pub fn delta(&self) -> f64 {
        self.up - self.down
    }
}

/// Where the point of control and value area stood after a candle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Developing {
    /// The end of the candle the levels are as of.
    pub time_ms: i64,
    pub poc: f64,
    pub vah: f64,
    pub val: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SvpProfile {
    pub start_ms: i64,
    pub end_ms: i64,
    pub grid: PriceGrid,
    /// Every row from the lowest to the highest traded, lowest first.
    pub rows: Vec<Row>,
    pub poc: usize,
    pub value_area: (usize, usize),
    pub developing: Vec<Developing>,
    pub quality: DataQuality,
    pub high: f64,
    pub low: f64,
    /// The span is not over (the clock or the replay position is inside it).
    pub forming: bool,
}

impl SvpProfile {
    pub fn up(&self) -> f64 {
        self.rows.iter().map(|row| row.up).sum()
    }

    pub fn down(&self) -> f64 {
        self.rows.iter().map(|row| row.down).sum()
    }

    pub fn total(&self) -> f64 {
        self.up() + self.down()
    }

    /// The busiest row's total: what the bars are measured against.
    pub fn peak(&self) -> f64 {
        self.rows.iter().map(Row::total).fold(0.0, f64::max)
    }

    /// The largest absolute delta of any row (the bars' scale in delta mode).
    pub fn peak_delta(&self) -> f64 {
        self.rows
            .iter()
            .map(|row| row.delta().abs())
            .fold(0.0, f64::max)
    }

    pub fn first_row(&self) -> i64 {
        self.rows[0].row
    }

    pub fn row_at(&self, row: i64) -> Option<&Row> {
        usize::try_from(row - self.first_row())
            .ok()
            .and_then(|index| self.rows.get(index))
    }

    pub fn poc_price(&self) -> f64 {
        self.grid.middle(self.rows[self.poc].row)
    }

    pub fn vah(&self) -> f64 {
        self.grid.top(self.rows[self.value_area.1].row)
    }

    pub fn val(&self) -> f64 {
        self.grid.bottom(self.rows[self.value_area.0].row)
    }
}

/// TradingView's number-of-rows rule: ticks per row is the range in ticks over `rows`, rounded, at
/// least one.
pub fn ticks_per_row(high: f64, low: f64, tick: f64, rows: u32) -> u32 {
    let ticks = ((high - low) / tick / f64::from(rows.max(1))).round();
    ticks.clamp(1.0, f64::from(u32::MAX)) as u32
}

/// The class of a candle without an exchange split: up when it closed above its open, down below,
/// and a doji follows the one before it (up when there is none).
fn is_up(open: f64, close: f64, previous: Option<bool>) -> bool {
    if close > open {
        true
    } else if close < open {
        false
    } else {
        previous.unwrap_or(true)
    }
}

/// The profile of `span` from the candles inside it that open before `until_ms`.
pub fn build_profile<'a>(
    span: Session,
    candles: impl IntoIterator<Item = &'a FlowKline>,
    tick: f64,
    shape: &SvpShape,
    until_ms: i64,
) -> Result<SvpProfile, Unsupported> {
    let candles: Vec<&FlowKline> = candles
        .into_iter()
        .filter(|c| {
            c.kline.open_time >= span.start_ms
                && c.kline.open_time < span.end_ms
                && c.kline.open_time < until_ms
        })
        .collect();
    let with_volume: Vec<&&FlowKline> = candles.iter().filter(|c| c.kline.volume > 0.0).collect();
    if with_volume.is_empty() {
        return Err(Unsupported::NoVolume);
    }
    let high = with_volume
        .iter()
        .map(|c| c.kline.high)
        .fold(f64::MIN, f64::max);
    let low = with_volume
        .iter()
        .map(|c| c.kline.low)
        .fold(f64::MAX, f64::min);
    let per_row = match shape.layout {
        RowsLayout::NumberOfRows => ticks_per_row(high, low, tick, shape.row_size),
        RowsLayout::TicksPerRow => shape.row_size.max(1),
    };
    let grid = PriceGrid::new(tick, per_row).ok_or(Unsupported::NoTickSize)?;
    let (first, last) = grid
        .rows_spanning(low, high)
        .ok_or(Unsupported::TooManyRows)?;
    let count = (last - first + 1) as usize;
    debug_assert!(count <= MAX_ROWS);
    let mut rows: Vec<Row> = (first..=last)
        .map(|row| Row {
            row,
            up: 0.0,
            down: 0.0,
        })
        .collect();
    let fraction = f64::from(shape.value_area_percent) / 100.0;
    let step = candles.len().div_ceil(MAX_DEVELOPING).max(1);
    let mut developing = Vec::new();
    let (mut previous, mut split, mut unsplit) = (None, 0usize, 0usize);
    let (mut lowest, mut highest) = (usize::MAX, 0usize);
    for (index, candle) in candles.iter().enumerate() {
        let k = &candle.kline;
        if k.volume > 0.0 {
            let (up, down) = match candle.split() {
                Some(exchange) => {
                    split += 1;
                    exchange
                }
                None => {
                    unsplit += 1;
                    if is_up(k.open, k.close, previous) {
                        (k.volume, 0.0)
                    } else {
                        (0.0, k.volume)
                    }
                }
            };
            previous = Some(up >= down);
            for (volume, is_up_side) in [(up, true), (down, false)] {
                if volume <= 0.0 {
                    continue;
                }
                for (row, share) in grid
                    .spread(k.low, k.high, volume)
                    .ok_or(Unsupported::TooManyRows)?
                {
                    let at = (row - first) as usize;
                    if is_up_side {
                        rows[at].up += share;
                    } else {
                        rows[at].down += share;
                    }
                    lowest = lowest.min(at);
                    highest = highest.max(at);
                }
            }
        }
        // The levels as of the end of this candle, from the rows so far and nothing later.
        let sample = shape.developing
            && highest >= lowest
            && lowest != usize::MAX
            && (index % step == 0 || index + 1 == candles.len());
        if sample {
            let seen = &rows[lowest..=highest];
            let totals: Vec<f64> = seen.iter().map(Row::total).collect();
            let middles: Vec<f64> = seen.iter().map(|row| grid.middle(row.row)).collect();
            let range_middle =
                (grid.bottom(seen[0].row) + grid.top(seen[seen.len() - 1].row)) / 2.0;
            if let Some((poc, (bottom, top))) =
                value_area::volume(&totals, &middles, range_middle, fraction)
            {
                developing.push(Developing {
                    time_ms: k.close_time.saturating_add(1),
                    poc: middles[poc],
                    vah: grid.top(seen[top].row),
                    val: grid.bottom(seen[bottom].row),
                });
            }
        }
    }
    let totals: Vec<f64> = rows.iter().map(Row::total).collect();
    let middles: Vec<f64> = rows.iter().map(|row| grid.middle(row.row)).collect();
    let range_middle = (grid.bottom(first) + grid.top(last)) / 2.0;
    let (poc, value_area) = value_area::volume(&totals, &middles, range_middle, fraction)
        .ok_or(Unsupported::NoVolume)?;
    Ok(SvpProfile {
        start_ms: span.start_ms,
        end_ms: span.end_ms,
        grid,
        rows,
        poc,
        value_area,
        developing,
        quality: if split > 0 && unsplit == 0 {
            DataQuality::ExchangeSplit
        } else {
            DataQuality::Intrabar
        },
        high,
        low,
        forming: until_ms < span.end_ms,
    })
}

// MARK: A set of profiles

#[derive(Clone, Debug, PartialEq)]
pub struct SvpSet {
    pub profiles: Vec<Arc<SvpProfile>>,
    pub tick: Option<Tick>,
    pub status: SetStatus,
    pub refused: usize,
    pub first_refusal: Option<Unsupported>,
    /// The candle size the profiles were built from, in minutes.
    pub candle_minutes: i64,
}

impl SvpSet {
    pub fn empty(status: SetStatus) -> Self {
        Self {
            profiles: Vec::new(),
            tick: None,
            status,
            refused: 0,
            first_refusal: None,
            candle_minutes: 0,
        }
    }
}

type Cached = (u64, Result<Arc<SvpProfile>, Unsupported>);

/// Built profiles reused while their candles and parameters are unchanged.
#[derive(Default)]
pub struct Cache {
    key: Option<(u64, SvpShape)>,
    profiles: HashMap<(i64, i64), Cached>,
}

pub struct SetInput<'a> {
    /// The chart's bars (cut at the replay position): they say which time to cover.
    pub bars: &'a [Kline],
    pub bar_ms: i64,
    pub candles: &'a CandleBook,
    pub exchange_tick: Option<f64>,
    pub shape: SvpShape,
    /// Candles opening at or after this are not read (the replay position).
    pub until_ms: i64,
    /// The clock: a span that has not ended by it is still forming. Nothing here reads the time.
    pub now_ms: i64,
}

fn fingerprint(parts: impl IntoIterator<Item = u64>) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for part in parts {
        part.hash(&mut hasher);
    }
    hasher.finish()
}

/// The spans a chart of `bars` shows (UTC days, or the custom windows).
pub fn spans_for(bars: &[Kline], bar_ms: i64, until_ms: i64, shape: &SvpShape) -> Vec<Session> {
    sessions::spans_covering(
        bars,
        bar_ms,
        until_ms,
        sessions::Period::Days(1),
        shape.custom_sessions().as_deref(),
        MAX_PROFILES,
    )
}

/// Builds the set. See [`SetInput`].
pub fn build_set(input: SetInput<'_>, cache: &mut Cache) -> SvpSet {
    let SetInput {
        bars,
        bar_ms,
        candles,
        exchange_tick,
        shape,
        until_ms,
        now_ms,
    } = input;
    if bars.is_empty() {
        return SvpSet::empty(SetStatus::Ready);
    }
    let until_ms = until_ms.min(now_ms);
    let candle_minutes = candles.interval_ms() / 60_000;
    let empty = |status| SvpSet {
        candle_minutes,
        ..SvpSet::empty(status)
    };
    let Some(tick) = Tick::resolve(exchange_tick, bars) else {
        return empty(SetStatus::Unsupported(Unsupported::NoTickSize));
    };
    let tick_key = tick.size.to_bits();
    if cache.key != Some((tick_key, shape)) {
        cache.profiles.clear();
        cache.key = Some((tick_key, shape));
    }
    let spans = spans_for(bars, bar_ms, until_ms, &shape);
    let mut profiles = Vec::new();
    let (mut refused, mut first_refusal) = (0, None);
    let mut seen = Vec::new();
    for span in &spans {
        let slice: Vec<&FlowKline> = candles
            .between(span.start_ms, span.end_ms.min(until_ms))
            .collect();
        if slice.is_empty() {
            continue;
        }
        let signature = fingerprint(
            [
                slice.len() as u64,
                span.end_ms.min(until_ms) as u64,
                span.end_ms as u64,
            ]
            .into_iter()
            .chain(slice.iter().rev().take(2).flat_map(|c| {
                [
                    c.kline.open_time as u64,
                    c.kline.high.to_bits(),
                    c.kline.low.to_bits(),
                    c.kline.close.to_bits(),
                    c.kline.volume.to_bits(),
                    c.taker_buy.map_or(0, f64::to_bits),
                ]
            }))
            .chain([slice.iter().map(|c| c.kline.volume).sum::<f64>().to_bits()]),
        );
        let key = (span.start_ms, span.end_ms);
        seen.push(key);
        let result = match cache.profiles.get(&key) {
            Some((cached, result)) if *cached == signature => result.clone(),
            _ => {
                let result =
                    build_profile(*span, slice.iter().copied(), tick.size, &shape, until_ms)
                        .map(Arc::new);
                cache.profiles.insert(key, (signature, result.clone()));
                result
            }
        };
        match result {
            Ok(profile) => profiles.push(profile),
            Err(Unsupported::NoVolume) => {}
            Err(reason) => {
                refused += 1;
                first_refusal.get_or_insert(reason);
            }
        }
    }
    cache.profiles.retain(|key, _| seen.contains(key));
    SvpSet {
        profiles,
        tick: Some(tick),
        status: SetStatus::Ready,
        refused,
        first_refusal,
        candle_minutes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::chart_type::{SessionMode, SvpSettings};

    const MIN: i64 = 60_000;
    const HOUR: i64 = 60 * MIN;
    const DAY: i64 = 24 * HOUR;

    fn candle(
        open_time: i64,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        volume: f64,
        taker_buy: Option<f64>,
    ) -> FlowKline {
        FlowKline {
            kline: Kline {
                open_time,
                close_time: open_time + 5 * MIN - 1,
                open,
                high,
                low,
                close,
                volume,
            },
            taker_buy,
        }
    }

    fn shape(rows: u32) -> SvpShape {
        SvpSettings {
            layout: RowsLayout::TicksPerRow,
            row_size: rows,
            ..SvpSettings::default()
        }
        .shape()
    }

    fn span() -> Session {
        Session {
            start_ms: 0,
            end_ms: DAY,
        }
    }

    fn build(candles: &[FlowKline], shape: &SvpShape) -> SvpProfile {
        build_profile(span(), candles, 1.0, shape, i64::MAX).unwrap()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    // MARK: Profile

    #[test]
    fn a_candles_volume_is_spread_over_the_rows_it_overlaps_and_nothing_is_lost() {
        // 10.5 to 12.5, volume 8: a quarter to row 10, half to 11, a quarter to 12.
        let candles = [candle(0, 10.5, 12.5, 10.5, 12.5, 8.0, None)];
        let profile = build(&candles, &shape(1));
        let totals: Vec<f64> = profile.rows.iter().map(Row::total).collect();
        assert!(close(totals[0], 2.0) && close(totals[1], 4.0) && close(totals[2], 2.0));
        assert!(close(profile.total(), 8.0));
    }

    #[test]
    fn up_and_down_come_from_the_exchange_split_or_close_against_open() {
        let candles = [
            candle(0, 10.0, 11.0, 10.0, 11.0, 6.0, None),       // up
            candle(5 * MIN, 11.0, 11.0, 10.0, 10.0, 4.0, None), // down
            candle(10 * MIN, 10.0, 11.0, 10.0, 10.5, 10.0, Some(7.0)), // exchange: 7 up, 3 down
        ];
        let profile = build(&candles, &shape(1));
        assert!(close(profile.up(), 6.0 + 7.0));
        assert!(close(profile.down(), 4.0 + 3.0));
        assert!(close(profile.total(), 20.0));
        assert_eq!(
            profile.quality,
            DataQuality::Intrabar,
            "one candle had no split"
        );
        let all_split = [candle(0, 10.0, 11.0, 10.0, 10.5, 10.0, Some(4.0))];
        assert_eq!(
            build(&all_split, &shape(1)).quality,
            DataQuality::ExchangeSplit
        );
    }

    #[test]
    fn a_doji_follows_the_candle_before_it_and_a_leading_doji_is_up() {
        let doji_first = [candle(0, 10.0, 11.0, 10.0, 10.0, 5.0, None)];
        assert!(close(build(&doji_first, &shape(1)).up(), 5.0));
        let after_down = [
            candle(0, 11.0, 11.0, 10.0, 10.0, 5.0, None),
            candle(5 * MIN, 10.0, 11.0, 10.0, 10.0, 3.0, None),
        ];
        let profile = build(&after_down, &shape(1));
        assert!(close(profile.down(), 8.0) && close(profile.up(), 0.0));
    }

    #[test]
    fn the_poc_is_the_busiest_row_and_the_value_area_holds_the_share() {
        let candles = [
            candle(0, 10.0, 10.5, 10.0, 10.5, 1.0, None),
            candle(5 * MIN, 11.0, 11.5, 11.0, 11.5, 9.0, None),
            candle(10 * MIN, 12.0, 12.5, 12.0, 12.5, 2.0, None),
            candle(15 * MIN, 13.0, 13.5, 13.0, 13.5, 1.0, None),
        ];
        let profile = build(&candles, &shape(1));
        assert_eq!(profile.rows[profile.poc].row, 11);
        assert!(close(profile.poc_price(), 11.5));
        let (low, high) = profile.value_area;
        let held: f64 = profile.rows[low..=high].iter().map(Row::total).sum();
        assert!(held >= 0.7 * profile.total() - 1e-9);
        assert!(profile.val() <= 11.0 && profile.vah() >= 12.0);
    }

    #[test]
    fn the_number_of_rows_rule_rounds_the_row_size_and_may_add_a_row() {
        // TradingView's example: a 100-tick span with 25 rows is 4 ticks a row; with 30 it is 3
        // ticks a row, which makes 34 rows.
        assert_eq!(ticks_per_row(100.0, 0.0, 1.0, 25), 4);
        assert_eq!(ticks_per_row(100.0, 0.0, 1.0, 30), 3);
        assert_eq!(ticks_per_row(0.5, 0.0, 1.0, 24), 1, "never under one tick");
        let candles = [candle(0, 0.0, 100.0, 0.5, 100.0, 100.0, None)];
        let layout = SvpSettings {
            row_size: 30,
            ..SvpSettings::default()
        }
        .shape();
        let profile = build_profile(span(), &candles, 1.0, &layout, i64::MAX).unwrap();
        assert_eq!(profile.grid.ticks_per_row(), 3);
        assert!(
            profile.rows.len() >= 33 && profile.rows.len() <= 35,
            "{}",
            profile.rows.len()
        );
    }

    #[test]
    fn ticks_per_row_is_taken_as_it_says_and_a_row_count_too_tall_is_refused() {
        let candles = [candle(0, 0.0, 100.0, 0.0, 100.0, 10.0, None)];
        let profile = build(&candles, &shape(10));
        assert_eq!(profile.grid.ticks_per_row(), 10);
        let fine = SvpSettings {
            layout: RowsLayout::TicksPerRow,
            row_size: 1,
            ..SvpSettings::default()
        }
        .shape();
        let tall = [candle(0, 0.0, 5_000.0, 0.0, 5_000.0, 10.0, None)];
        assert_eq!(
            build_profile(span(), &tall, 1.0, &fine, i64::MAX),
            Err(Unsupported::TooManyRows)
        );
    }

    #[test]
    fn a_session_without_volume_has_no_profile_and_candles_outside_it_are_not_read() {
        let silent = [candle(0, 10.0, 11.0, 10.0, 10.5, 0.0, None)];
        assert_eq!(
            build_profile(span(), &silent, 1.0, &shape(1), i64::MAX),
            Err(Unsupported::NoVolume)
        );
        let other_day = [candle(DAY + 5 * MIN, 10.0, 11.0, 10.0, 10.5, 5.0, None)];
        assert_eq!(
            build_profile(span(), &other_day, 1.0, &shape(1), i64::MAX),
            Err(Unsupported::NoVolume)
        );
    }

    // MARK: Developing levels

    fn developing_shape() -> SvpShape {
        SvpSettings {
            layout: RowsLayout::TicksPerRow,
            row_size: 1,
            developing_poc: true,
            developing_value_area: true,
            ..SvpSettings::default()
        }
        .shape()
    }

    /// Volume piles up at 10 first, then moves to 20: the point of control moves with it.
    fn migrating() -> Vec<FlowKline> {
        let mut candles = Vec::new();
        for i in 0..6 {
            candles.push(candle(i * 5 * MIN, 10.0, 10.5, 10.0, 10.5, 10.0, None));
        }
        for i in 6..20 {
            candles.push(candle(i * 5 * MIN, 20.0, 20.5, 20.0, 20.5, 10.0, None));
        }
        candles
    }

    #[test]
    fn the_developing_poc_is_where_it_stood_at_each_moment_and_not_where_it_ended() {
        let candles = migrating();
        let profile = build(&candles, &developing_shape());
        assert_eq!(profile.developing.len(), candles.len());
        assert!(close(profile.poc_price(), 20.5), "the day ended at 20");
        // Early on the point of control was 10; it stayed there until 20 overtook it.
        assert!(close(profile.developing[0].poc, 10.5));
        assert!(close(profile.developing[5].poc, 10.5));
        // Seven candles of 20 (70) beat six of 10 (60): the move happens at the 13th candle.
        assert!(close(profile.developing[11].poc, 10.5));
        assert!(close(profile.developing[12].poc, 20.5));
        assert!(close(
            profile.developing.last().unwrap().poc,
            profile.poc_price()
        ));
        // The value area's edges were narrow at first and only widened to include both piles later.
        assert!(profile.developing[0].vah - profile.developing[0].val <= 1.0 + 1e-9);
    }

    #[test]
    fn the_developing_levels_at_a_moment_equal_the_profile_built_from_the_candles_so_far() {
        // The no-lookahead property: cutting the candles at any point and building the whole profile
        // gives the point of control the developing series holds for that point.
        let candles = migrating();
        let developing = build(&candles, &developing_shape()).developing;
        for cut in [1usize, 4, 8, 13, 20] {
            let prefix = build(&candles[..cut], &developing_shape());
            let at = &developing[cut - 1];
            assert!(close(at.poc, prefix.poc_price()), "cut {cut}");
            assert!(
                close(at.vah, prefix.vah()) && close(at.val, prefix.val()),
                "cut {cut}"
            );
        }
    }

    #[test]
    fn the_developing_levels_are_only_worked_out_when_asked_for_and_stay_bounded() {
        let none = build(&migrating(), &shape(1));
        assert!(none.developing.is_empty());
        let many: Vec<FlowKline> = (0..1_000)
            .map(|i| {
                let price = 10.0 + (i % 7) as f64;
                candle(i * MIN, price, price + 1.0, price, price + 1.0, 1.0, None)
            })
            .collect();
        let profile = build(&many, &developing_shape());
        assert!(
            profile.developing.len() <= MAX_DEVELOPING + 1,
            "{}",
            profile.developing.len()
        );
        assert!(profile.developing.len() > 100);
        // The last candle is always a point.
        assert!(close(
            profile.developing.last().unwrap().poc,
            profile.poc_price()
        ));
    }

    #[test]
    fn replay_hides_the_candles_after_it() {
        let candles = migrating();
        let cut = build_profile(span(), &candles, 1.0, &developing_shape(), 6 * 5 * MIN).unwrap();
        assert!(cut.forming);
        assert!(
            close(cut.poc_price(), 10.5),
            "only the first six candles exist"
        );
        assert!(cut.high < 11.0);
        let full = build(&candles, &developing_shape());
        assert!(!full.forming);
    }

    // MARK: Sets

    fn day_bars(n: i64) -> Vec<Kline> {
        (0..n)
            .map(|i| Kline {
                open_time: i * DAY,
                close_time: (i + 1) * DAY - 1,
                open: 100.0,
                high: 104.0,
                low: 99.0,
                close: 102.0,
                volume: 50.0,
            })
            .collect()
    }

    fn book(days: i64) -> CandleBook {
        let mut book = CandleBook::new(5 * MIN, 100_000);
        book.insert((0..days * 288).map(|i| {
            let p = 100.0 + (i % 5) as f64;
            candle(
                i * 5 * MIN,
                p,
                p + 1.0,
                p - 1.0,
                p + 0.5,
                1.0 + (i % 3) as f64,
                None,
            )
        }));
        book
    }

    fn set_of(days: i64, shape: SvpShape, cache: &mut Cache) -> SvpSet {
        let bars = day_bars(days);
        build_set(
            SetInput {
                bars: &bars,
                bar_ms: DAY,
                candles: &book(days),
                exchange_tick: Some(1.0),
                shape,
                until_ms: i64::MAX,
                now_ms: i64::MAX,
            },
            cache,
        )
    }

    #[test]
    fn a_set_has_one_profile_per_session_with_volume() {
        let set = set_of(3, SvpSettings::default().shape(), &mut Cache::default());
        assert_eq!(set.status, SetStatus::Ready);
        assert_eq!(set.profiles.len(), 3);
        assert_eq!(set.profiles[1].start_ms, DAY);
        assert_eq!(set.candle_minutes, 5);
        assert!(set.profiles.iter().all(|p| !p.forming));
    }

    #[test]
    fn custom_windows_are_the_sessions_and_overnight_ones_belong_to_the_day_they_opened() {
        let settings = SvpSettings {
            sessions: SessionMode::Custom,
            custom: [
                crate::model::market_profile::sessions::SessionSpan::parse("0900-1700"),
                None,
                None,
                None,
                None,
            ],
            ..SvpSettings::default()
        };
        let set = set_of(2, settings.shape(), &mut Cache::default());
        assert_eq!(set.profiles.len(), 2);
        assert_eq!(set.profiles[0].start_ms, 9 * HOUR);
        assert_eq!(set.profiles[0].end_ms, 17 * HOUR);
        let overnight = SvpSettings {
            sessions: SessionMode::Custom,
            custom: [
                crate::model::market_profile::sessions::SessionSpan::parse("2200-0600"),
                None,
                None,
                None,
                None,
            ],
            ..SvpSettings::default()
        };
        let set = set_of(3, overnight.shape(), &mut Cache::default());
        assert_eq!(
            set.profiles[0].start_ms,
            22 * HOUR - DAY,
            "the window that opened the evening before"
        );
        assert_eq!(set.profiles[0].end_ms, 6 * HOUR);
    }

    #[test]
    fn finished_sessions_are_reused_and_only_the_forming_one_is_rebuilt() {
        let bars = day_bars(2);
        let mut candles = book(2);
        let mut cache = Cache::default();
        let shape = SvpSettings::default().shape();
        let mut run = |candles: &CandleBook, now_ms| {
            build_set(
                SetInput {
                    bars: &bars,
                    bar_ms: DAY,
                    candles,
                    exchange_tick: Some(1.0),
                    shape,
                    until_ms: i64::MAX,
                    now_ms,
                },
                &mut cache,
            )
        };
        let first = run(&candles, i64::MAX);
        let again = run(&candles, i64::MAX);
        assert!(Arc::ptr_eq(&first.profiles[0], &again.profiles[0]));
        assert!(Arc::ptr_eq(&first.profiles[1], &again.profiles[1]));
        let last = 2 * 288 - 1;
        candles.insert([candle(
            last * 5 * MIN,
            100.0,
            140.0,
            90.0,
            110.0,
            50.0,
            None,
        )]);
        let live = run(&candles, i64::MAX);
        assert!(Arc::ptr_eq(&first.profiles[0], &live.profiles[0]));
        assert!(!Arc::ptr_eq(&first.profiles[1], &live.profiles[1]));
        assert_eq!(live.profiles[1].high, 140.0);
        // The clock: at midday the second day is forming and holds only what had happened.
        let midday = run(&candles, DAY + 12 * HOUR);
        assert!(midday.profiles[1].forming);
        assert!(midday.profiles[1].high < 140.0);
    }

    #[test]
    fn a_changed_row_size_rebuilds_and_nothing_is_built_without_a_tick_or_candles() {
        let mut cache = Cache::default();
        let a = set_of(1, shape(1), &mut cache);
        let b = set_of(1, shape(2), &mut cache);
        assert!(!Arc::ptr_eq(&a.profiles[0], &b.profiles[0]));
        assert!(b.profiles[0].rows.len() < a.profiles[0].rows.len());
        let odd = vec![Kline {
            open: 1.234_567_890_123_4,
            ..day_bars(1)[0].clone()
        }];
        let no_tick = build_set(
            SetInput {
                bars: &odd,
                bar_ms: DAY,
                candles: &book(1),
                exchange_tick: None,
                shape: shape(1),
                until_ms: i64::MAX,
                now_ms: i64::MAX,
            },
            &mut Cache::default(),
        );
        assert_eq!(
            no_tick.status,
            SetStatus::Unsupported(Unsupported::NoTickSize)
        );
        let none = build_set(
            SetInput {
                bars: &[],
                bar_ms: DAY,
                candles: &book(1),
                exchange_tick: Some(1.0),
                shape: shape(1),
                until_ms: i64::MAX,
                now_ms: i64::MAX,
            },
            &mut Cache::default(),
        );
        assert!(none.profiles.is_empty());
    }

    // MARK: Candle plan

    fn days(n: i64) -> Vec<Session> {
        (0..n)
            .map(|i| Session {
                start_ms: i * DAY,
                end_ms: (i + 1) * DAY,
            })
            .collect()
    }

    #[test]
    fn the_finest_candle_that_fits_is_chosen_and_a_long_history_reaches_back_only_as_far_as_it_can()
    {
        // 20 days of 1-minute candles is 28 800: within 60 000.
        let (interval, size, from) = candle_plan(&days(20), i64::MAX, 60_000);
        assert_eq!((interval, size, from), ("1m", MIN, 0));
        // 100 days: 1m is 144 000, 5m is 28 800.
        let (interval, ..) = candle_plan(&days(100), i64::MAX, 60_000);
        assert_eq!(interval, "5m");
        // 3000 days: even hourly (72 000) is too many; it reaches back 2500 days.
        let (interval, _, from) = candle_plan(&days(3_000), i64::MAX, 60_000);
        assert_eq!(interval, "1h");
        assert!(from > 400 * DAY && from < 3_000 * DAY);
    }

    #[test]
    fn a_forming_session_counts_only_the_candles_that_can_exist() {
        let spans = vec![Session {
            start_ms: 0,
            end_ms: 1000 * DAY,
        }];
        let (_, size, _) = candle_plan(&spans, 5 * DAY, 60_000);
        assert!(size <= 5 * MIN, "5 days of candles fit finely");
    }
}
