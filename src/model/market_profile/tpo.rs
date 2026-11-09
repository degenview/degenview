//! The time price opportunity (TPO, market profile) chart: for each period, which price rows were
//! visited in which block of time, as letters.
//!
//! A profile covers a **span** of time (a day, week or month, a custom session, or what a split or
//! merge made of them). Its time is cut into blocks of `block_ms` from the span's start; block 0 is
//! `A`, block 25 `Z`, block 26 `a`, and the letters repeat after `z`. A candle marks every price row
//! its high-to-low reaches with the letter of the block it opened in, so a row's TPO count is the
//! number of blocks that visited it.
//!
//! Built from **5-minute candles** (every block size TradingView offers is a multiple of five
//! minutes), so a block's range is exact to the candle: nothing is estimated except where inside a
//! candle price went, which a letter does not say. Candles are never taken from the chart's own bars
//! (a chart of daily bars cannot say which half-hour a price traded in).
//!
//! What is computed, and from what (TradingView's documented definitions where it gives one):
//!
//! - **Point of control**: the row with the most blocks; **value area**: grown one row at a time from
//!   it by block count ([`value_area::tpo`]), 70 % by default. Not volume.
//! - **Initial balance**: the range of the first `n` blocks.
//! - **Poor high / poor low**: the extreme row has more than one block (a flat top or bottom).
//! - **Single prints**: rows that are not the extremes and hold exactly one block.
//! - **Rotation factor** (summary): per block, `+1`/`-1`/`0` for its high against the previous block's
//!   high, and the same for lows, summed.
//! - **Volume profile** (beside the TPO): the volume of each candle spread evenly over its range, its
//!   own point of control and value area. Volume and TPO counts are separate measures; nothing here
//!   mixes them.
//!
//! The **midpoint** is the middle of the profile's range (high and low), which TradingView names but
//! does not define: inferred.

use super::flow::{CandleBook, FlowKline};
use super::footprint::SetStatus;
use super::price_grid::{MAX_ROWS, PriceGrid};
use super::quality::Unsupported;
use super::sessions::{self, Session, block_index};
use super::tick::Tick;
use super::value_area;
use crate::model::chart_type::{TpoShape, average_true_range};
use crate::model::kline::Kline;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Most profiles a set builds (the newest); older spans draw nothing.
pub const MAX_PROFILES: usize = 400;

/// A split or merge the user made to the profiles, kept per chart and replayed over the periods the
/// settings give. A profile is named by the instant it starts, which survives data arriving.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TpoEdit {
    /// Cuts the profile that holds `at_ms` in two; the second starts at `at_ms`.
    Split { at_ms: i64 },
    /// Joins the profile that starts at `at_ms` to the one before it.
    Merge { at_ms: i64 },
}

/// `spans` with `edits` applied in order. An edit that names a profile that is not there (the
/// periods changed under it) does nothing.
pub fn apply_edits(mut spans: Vec<Session>, edits: &[TpoEdit]) -> Vec<Session> {
    for edit in edits {
        match *edit {
            TpoEdit::Split { at_ms } => {
                if let Some(index) = spans
                    .iter()
                    .position(|span| span.start_ms < at_ms && at_ms < span.end_ms)
                {
                    let span = spans[index];
                    spans[index].end_ms = at_ms;
                    spans.insert(
                        index + 1,
                        Session {
                            start_ms: at_ms,
                            end_ms: span.end_ms,
                        },
                    );
                }
            }
            TpoEdit::Merge { at_ms } => {
                if let Some(index) = spans.iter().position(|span| span.start_ms == at_ms)
                    && index > 0
                    && spans[index - 1].end_ms == at_ms
                {
                    spans[index - 1].end_ms = spans[index].end_ms;
                    spans.remove(index);
                }
            }
        }
    }
    spans
}

/// The TPO row size TradingView's automatic mode picks from the newest bars' range: the range in
/// ticks over 80 rows, rounded to a step that grows with the number of ticks (5 up to 100 rows' worth,
/// 50 up to 1000, 500 up to 10 000, and so on). At least one tick. `None` for a range that is not a
/// positive number.
pub fn auto_ticks_per_row(high: f64, low: f64, tick: f64) -> Option<u32> {
    if !(high.is_finite() && low.is_finite() && tick.is_finite() && tick > 0.0 && high > low) {
        return None;
    }
    let row_ticks = (high - low) / tick / 80.0;
    // 1..=100 -> 5, 101..=1000 -> 50, 1001..=10000 -> 500, ...
    let digits = row_ticks.max(1.0).log10().ceil().max(2.0) as i32;
    let increment = 5.0 * 10f64.powi(digits - 2);
    let ticks = (row_ticks / increment).round() * increment;
    Some(ticks.clamp(1.0, f64::from(u32::MAX)) as u32)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub grid: PriceGrid,
    pub block_ms: i64,
    /// The value-area share, 0 to 1.
    pub value_area: f64,
    /// Blocks of the initial balance.
    pub initial_balance_blocks: u32,
}

/// One price row of one profile.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub row: i64,
    /// The blocks that visited the row, ascending.
    pub blocks: Vec<u32>,
    /// Volume spread over the row from the candles.
    pub volume: f64,
}

impl Row {
    pub fn count(&self) -> u32 {
        self.blocks.len() as u32
    }
}

/// A row that only one block visited, away from the extremes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SinglePrint {
    /// Index into `rows`.
    pub index: usize,
    pub block: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TpoProfile {
    pub start_ms: i64,
    pub end_ms: i64,
    /// Every row from the lowest to the highest visited, lowest first; a row nobody visited (a gap
    /// between candles) has no blocks.
    pub rows: Vec<Row>,
    /// Blocks elapsed: the last candle's block plus one.
    pub blocks: u32,
    pub poc: usize,
    pub value_area: (usize, usize),
    /// The volume profile's point of control and value area; `None` when no candle had volume.
    pub volume_poc: Option<usize>,
    pub volume_value_area: Option<(usize, usize)>,
    pub open: f64,
    pub close: f64,
    pub high: f64,
    pub low: f64,
    /// Low and high of the initial balance.
    pub initial_balance: Option<(f64, f64)>,
    pub poor_high: bool,
    pub poor_low: bool,
    pub single_prints: Vec<SinglePrint>,
    pub total: u32,
    pub above_poc: u32,
    pub below_poc: u32,
    pub rotation: i32,
    /// The span is not over (data runs up to now or the replay position).
    pub forming: bool,
}

impl TpoProfile {
    pub fn poc_row(&self) -> i64 {
        self.rows[self.poc].row
    }

    pub fn first_row(&self) -> i64 {
        self.rows[0].row
    }

    /// The most blocks any row holds: the profile's width in letters.
    pub fn width(&self) -> u32 {
        self.rows.iter().map(Row::count).max().unwrap_or(0)
    }

    pub fn row_at(&self, row: i64) -> Option<&Row> {
        usize::try_from(row - self.first_row())
            .ok()
            .and_then(|index| self.rows.get(index))
    }

    pub fn volume_total(&self) -> f64 {
        self.rows.iter().map(|row| row.volume).sum()
    }

    pub fn peak_volume(&self) -> f64 {
        self.rows.iter().map(|row| row.volume).fold(0.0, f64::max)
    }

    /// Runs of consecutive single prints, as `(first index, last index)`.
    pub fn single_print_runs(&self) -> Vec<(usize, usize)> {
        let mut runs: Vec<(usize, usize)> = Vec::new();
        for print in &self.single_prints {
            match runs.last_mut() {
                Some(run) if run.1 + 1 == print.index => run.1 = print.index,
                _ => runs.push((print.index, print.index)),
            }
        }
        runs
    }

    /// The single print of a run that was established first (the lowest block): the level
    /// TradingView highlights for a run.
    pub fn first_established(&self, run: (usize, usize)) -> Option<SinglePrint> {
        self.single_prints
            .iter()
            .filter(|print| (run.0..=run.1).contains(&print.index))
            .min_by_key(|print| print.block)
            .copied()
    }
}

/// The profile of `span` from the candles inside it that opened before `until_ms`.
pub fn build_profile<'a>(
    span: Session,
    candles: impl IntoIterator<Item = &'a FlowKline>,
    params: &Params,
    until_ms: i64,
) -> Result<TpoProfile, Unsupported> {
    let mut rows: BTreeMap<i64, (BTreeSet<u32>, f64)> = BTreeMap::new();
    let mut per_block: BTreeMap<u32, (f64, f64)> = BTreeMap::new();
    let (mut open, mut close, mut high, mut low) = (None, 0.0, f64::MIN, f64::MAX);
    let mut ib: Option<(f64, f64)> = None;
    let (mut last_open, mut last_block) = (i64::MIN, 0u32);
    for candle in candles {
        let k = &candle.kline;
        if k.open_time < span.start_ms || k.open_time >= span.end_ms || k.open_time >= until_ms {
            continue;
        }
        let Some(block) = block_index(span.start_ms, k.open_time, params.block_ms) else {
            continue;
        };
        let (first, last) = params
            .grid
            .rows_spanning(k.low, k.high)
            .ok_or(Unsupported::TooManyRows)?;
        for row in first..=last {
            rows.entry(row).or_default().0.insert(block);
        }
        if k.volume > 0.0 {
            for (row, share) in params
                .grid
                .spread(k.low, k.high, k.volume)
                .ok_or(Unsupported::TooManyRows)?
            {
                rows.entry(row).or_default().1 += share;
            }
        }
        if open.is_none() {
            open = Some(k.open);
        }
        if k.open_time >= last_open {
            (last_open, close) = (k.open_time, k.close);
        }
        last_block = last_block.max(block);
        high = high.max(k.high);
        low = low.min(k.low);
        let entry = per_block.entry(block).or_insert((k.high, k.low));
        entry.0 = entry.0.max(k.high);
        entry.1 = entry.1.min(k.low);
        if block < params.initial_balance_blocks {
            ib = Some(ib.map_or((k.low, k.high), |(l, h)| (l.min(k.low), h.max(k.high))));
        }
    }
    let (Some((&lowest, _)), Some((&highest, _)), Some(open)) =
        (rows.first_key_value(), rows.last_key_value(), open)
    else {
        return Err(Unsupported::NoVolume);
    };
    let count = highest
        .checked_sub(lowest)
        .and_then(|span| span.checked_add(1))
        .ok_or(Unsupported::TooManyRows)?;
    if count as usize > MAX_ROWS {
        return Err(Unsupported::TooManyRows);
    }
    let rows: Vec<Row> = (lowest..=highest)
        .map(|row| {
            let (blocks, volume) = rows.get(&row).cloned().unwrap_or_default();
            Row {
                row,
                blocks: blocks.into_iter().collect(),
                volume,
            }
        })
        .collect();
    let counts: Vec<u32> = rows.iter().map(Row::count).collect();
    let middles: Vec<f64> = rows.iter().map(|row| params.grid.middle(row.row)).collect();
    let range_middle = (params.grid.bottom(lowest) + params.grid.top(highest)) / 2.0;
    let (poc, value_area) = value_area::tpo(&counts, &middles, range_middle, params.value_area)
        .ok_or(Unsupported::NoVolume)?;
    let volumes: Vec<f64> = rows.iter().map(|row| row.volume).collect();
    let (volume_poc, volume_value_area) =
        match value_area::volume(&volumes, &middles, range_middle, params.value_area) {
            Some((poc, area)) => (Some(poc), Some(area)),
            None => (None, None),
        };
    let top = rows.len() - 1;
    let single_prints = rows
        .iter()
        .enumerate()
        .filter(|(index, row)| *index > 0 && *index < top && row.count() == 1)
        .map(|(index, row)| SinglePrint {
            index,
            block: row.blocks[0],
        })
        .collect();
    let total: u32 = counts.iter().sum();
    let above_poc = counts[poc + 1..].iter().sum();
    let below_poc = counts[..poc].iter().sum();
    let mut rotation = 0;
    let mut previous: Option<(f64, f64)> = None;
    for (block_high, block_low) in per_block.values() {
        if let Some((high_before, low_before)) = previous {
            rotation += sign(block_high - high_before) + sign(block_low - low_before);
        }
        previous = Some((*block_high, *block_low));
    }
    Ok(TpoProfile {
        start_ms: span.start_ms,
        end_ms: span.end_ms,
        poor_high: counts[top] > 1,
        poor_low: counts[0] > 1,
        rows,
        blocks: last_block + 1,
        poc,
        value_area,
        volume_poc,
        volume_value_area,
        open,
        close,
        high,
        low,
        initial_balance: ib,
        single_prints,
        total,
        above_poc,
        below_poc,
        rotation,
        forming: until_ms < span.end_ms,
    })
}

fn sign(difference: f64) -> i32 {
    if difference > 0.0 {
        1
    } else if difference < 0.0 {
        -1
    } else {
        0
    }
}

// MARK: A set of profiles

#[derive(Clone, Debug, PartialEq)]
pub struct TpoSet {
    /// The profiles that could be built, oldest first.
    pub profiles: Vec<Arc<TpoProfile>>,
    pub grid: Option<PriceGrid>,
    pub tick: Option<Tick>,
    pub block_ms: i64,
    pub status: SetStatus,
    /// Spans refused (and why, for the first), which draw nothing.
    pub refused: usize,
    pub first_refusal: Option<Unsupported>,
    /// The span of every profile boundary, for the split and merge menu: all spans that exist,
    /// built or not, so a merge can name a neighbour with no data.
    pub spans: Vec<Session>,
}

impl TpoSet {
    pub fn empty(status: SetStatus) -> Self {
        Self {
            profiles: Vec::new(),
            grid: None,
            tick: None,
            block_ms: 0,
            status,
            refused: 0,
            first_refusal: None,
            spans: Vec::new(),
        }
    }

    /// The profile that holds `time_ms`.
    pub fn profile_at(&self, time_ms: i64) -> Option<&Arc<TpoProfile>> {
        let index = self.profiles.partition_point(|p| p.end_ms <= time_ms);
        self.profiles
            .get(index)
            .filter(|profile| profile.start_ms <= time_ms)
    }

    /// The span before the one starting at `start_ms`, if it touches it (what a merge would join).
    pub fn previous_span(&self, start_ms: i64) -> Option<&Session> {
        let index = self.spans.iter().position(|s| s.start_ms == start_ms)?;
        index
            .checked_sub(1)
            .and_then(|before| self.spans.get(before))
            .filter(|before| before.end_ms == start_ms)
    }
}

/// A built profile (or why it could not be) with the fingerprint of what it was built from.
type Cached = (u64, Result<Arc<TpoProfile>, Unsupported>);

/// Built profiles reused while their candles and parameters are unchanged.
#[derive(Default)]
pub struct Cache {
    key: Option<(PriceGrid, TpoShape)>,
    profiles: HashMap<(i64, i64), Cached>,
}

pub struct SetInput<'a> {
    /// The chart's bars (cut at the replay position), oldest first: they say which time to cover and
    /// the price range for the automatic row size.
    pub bars: &'a [Kline],
    pub bar_ms: i64,
    pub candles: &'a CandleBook,
    pub exchange_tick: Option<f64>,
    pub shape: TpoShape,
    pub edits: &'a [TpoEdit],
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

/// The spans a chart of `bars` shows: the periods (or custom sessions) from the first bar to the
/// end of the last, oldest first, at most [`MAX_PROFILES`] of the newest.
pub fn spans_for(bars: &[Kline], bar_ms: i64, until_ms: i64, shape: &TpoShape) -> Vec<Session> {
    sessions::spans_covering(
        bars,
        bar_ms,
        until_ms,
        shape.period(),
        shape.custom_sessions().as_deref(),
        MAX_PROFILES,
    )
}

/// Builds the set. See [`SetInput`].
pub fn build_set(input: SetInput<'_>, cache: &mut Cache) -> TpoSet {
    let SetInput {
        bars,
        bar_ms,
        candles,
        exchange_tick,
        shape,
        edits,
        until_ms,
        now_ms,
    } = input;
    // What exists: never past the replay position, never past the clock.
    let until_ms = until_ms.min(now_ms);
    if bars.is_empty() {
        return TpoSet::empty(SetStatus::Ready);
    }
    let block_ms = i64::from(shape.block_minutes.max(5)) * 60_000;
    let empty = |status| TpoSet {
        block_ms,
        ..TpoSet::empty(status)
    };
    if bars.iter().all(|bar| bar.high.is_nan() || bar.high <= 0.0) {
        return empty(SetStatus::Unsupported(Unsupported::NoVolume));
    }
    let Some(tick) = Tick::resolve(exchange_tick, bars) else {
        return empty(SetStatus::Unsupported(Unsupported::NoTickSize));
    };
    let per_row = match shape.row_mode {
        crate::model::chart_type::RowMode::Manual => Some(shape.ticks_per_row.max(1)),
        crate::model::chart_type::RowMode::Auto => {
            // TradingView: the newest 300 bars' range.
            let recent = &bars[bars.len().saturating_sub(300)..];
            let high = recent.iter().map(|bar| bar.high).fold(f64::MIN, f64::max);
            let low = recent.iter().map(|bar| bar.low).fold(f64::MAX, f64::min);
            auto_ticks_per_row(high, low, tick.size).or_else(|| {
                // A flat range: fall back to the average true range like the footprint.
                super::tick::auto_ticks_per_row(average_true_range(bars, 14), tick.size)
            })
        }
    };
    let Some(grid) = per_row.and_then(|per_row| PriceGrid::new(tick.size, per_row)) else {
        return empty(SetStatus::Unsupported(Unsupported::NoTickSize));
    };
    let params = Params {
        grid,
        block_ms,
        value_area: f64::from(shape.value_area_percent) / 100.0,
        initial_balance_blocks: shape.initial_balance_blocks,
    };
    if cache.key != Some((grid, shape)) {
        cache.profiles.clear();
        cache.key = Some((grid, shape));
    }
    let spans = apply_edits(spans_for(bars, bar_ms, until_ms, &shape), edits);
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
                    build_profile(*span, slice.iter().copied(), &params, until_ms).map(Arc::new);
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
    TpoSet {
        profiles,
        grid: Some(grid),
        tick: Some(tick),
        block_ms,
        status: SetStatus::Ready,
        refused,
        first_refusal,
        spans,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::chart_type::{PeriodKind, SessionMode, TpoSettings};
    use crate::model::market_profile::sessions::letter;

    const MIN: i64 = 60_000;
    const HOUR: i64 = 60 * MIN;
    const DAY: i64 = 24 * HOUR;
    const BLOCK: i64 = 30 * MIN;

    fn candle(
        open_time: i64,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        volume: f64,
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
            taker_buy: None,
        }
    }

    fn params() -> Params {
        Params {
            grid: PriceGrid::new(1.0, 1).unwrap(),
            block_ms: BLOCK,
            value_area: 0.7,
            initial_balance_blocks: 2,
        }
    }

    fn span() -> Session {
        Session {
            start_ms: 0,
            end_ms: DAY,
        }
    }

    /// One candle per half-hour block with the given (low, high).
    fn blocks(ranges: &[(f64, f64)]) -> Vec<FlowKline> {
        ranges
            .iter()
            .enumerate()
            .map(|(i, &(low, high))| candle(i as i64 * BLOCK, low, high, low, high, 10.0))
            .collect()
    }

    fn build(candles: &[FlowKline]) -> TpoProfile {
        build_profile(span(), candles, &params(), i64::MAX).unwrap()
    }

    fn counts(profile: &TpoProfile) -> Vec<u32> {
        profile.rows.iter().map(Row::count).collect()
    }

    fn letters_of(row: &Row) -> String {
        row.blocks.iter().map(|b| letter(*b)).collect()
    }

    // MARK: Profile

    #[test]
    fn each_block_marks_every_row_its_candles_reached() {
        // A: 100-102, B: 101-103, C: 100-101. Rows 100..=103.
        let profile = build(&blocks(&[(100.0, 102.0), (101.0, 103.0), (100.0, 101.0)]));
        assert_eq!(profile.first_row(), 100);
        let by_row: Vec<String> = profile.rows.iter().map(letters_of).collect();
        // Row 100: A, C. Row 101: A, B, C. Row 102: A, B. Row 103: B.
        // (A reaches 100..=102 because its high of 102.0 is the bottom of row 102.)
        assert_eq!(by_row, ["AC", "ABC", "AB", "B"]);
        assert_eq!(counts(&profile), [2, 3, 2, 1]);
        assert_eq!(profile.blocks, 3);
        assert_eq!(profile.total, 8);
    }

    #[test]
    fn two_candles_of_one_block_mark_a_row_once() {
        let candles = vec![
            candle(0, 100.0, 101.0, 100.0, 101.0, 1.0),
            candle(5 * MIN, 101.0, 102.0, 100.0, 100.0, 1.0),
        ];
        let profile = build(&candles);
        assert_eq!(counts(&profile), [1, 1, 1]);
    }

    #[test]
    fn the_poc_is_the_row_with_most_blocks_and_the_value_area_follows_the_documented_steps() {
        let profile = build(&blocks(&[
            (100.0, 104.0),
            (101.0, 104.0),
            (102.0, 105.0),
            (102.0, 104.0),
        ]));
        // Counts: 100:1, 101:2, 102:4, 103:4, 104:4, 105:1 -> total 16, target 11.2.
        assert_eq!(counts(&profile), [1, 2, 4, 4, 4, 1]);
        // Ties on 4: nearest the middle of the range (102.5 + ... 100..106 -> 103), rows 102 and 103
        // are each 0.5 and 0.5 away... row 103's middle is 103.5, row 102's 102.5: equal distance,
        // so the lower.
        assert_eq!(profile.poc_row(), 102);
        let (low, high) = profile.value_area;
        assert!(low <= profile.poc && profile.poc <= high);
        let held: u32 = counts(&profile)[low..=high].iter().sum();
        assert!(f64::from(held) >= 0.7 * f64::from(profile.total) - 1e-9);
        assert_eq!(profile.above_poc + profile.below_poc + 4, profile.total);
    }

    #[test]
    fn the_letters_run_through_the_alphabet_and_start_again() {
        // 60 blocks, each visiting its own row: row k has block k.
        let ranges: Vec<(f64, f64)> = (0..60)
            .map(|i| (100.0 + i as f64, 100.5 + i as f64))
            .collect();
        // Sixty half-hour blocks are longer than a day: give the profile three.
        let long = Session {
            start_ms: 0,
            end_ms: 3 * DAY,
        };
        let profile = build_profile(long, &blocks(&ranges), &params(), i64::MAX).unwrap();
        let text: String = profile.rows.iter().map(letters_of).collect();
        assert!(text.starts_with("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyzABCDEFGH"));
        assert_eq!(profile.blocks, 60);
    }

    #[test]
    fn the_initial_balance_is_the_range_of_the_first_blocks() {
        let profile = build(&blocks(&[(100.0, 102.0), (101.0, 105.0), (99.0, 110.0)]));
        assert_eq!(profile.initial_balance, Some((100.0, 105.0)));
        // With a different count.
        let one = build_profile(
            span(),
            &blocks(&[(100.0, 102.0), (101.0, 105.0)]),
            &Params {
                initial_balance_blocks: 1,
                ..params()
            },
            i64::MAX,
        )
        .unwrap();
        assert_eq!(one.initial_balance, Some((100.0, 102.0)));
    }

    #[test]
    fn poor_highs_and_lows_are_extremes_more_than_one_block_reached() {
        // Both A and B reach the top row 104 and the bottom row 100: poor high and poor low.
        let poor = build(&blocks(&[(100.0, 104.0), (100.0, 104.0)]));
        assert!(poor.poor_high && poor.poor_low);
        // B alone reaches 105 and 99: the extremes have one block each.
        let clean = build(&blocks(&[(100.0, 104.0), (99.0, 105.0)]));
        assert!(!clean.poor_high && !clean.poor_low);
    }

    #[test]
    fn single_prints_are_inner_rows_with_one_block_and_runs_name_the_first_established() {
        // A covers 100..=110; B a spike to 106 only at 105..=106 doesn't make singles...
        // A: 100..=110 (all rows 1), B: 100..=103, C: 107..=110 -> rows 104..=106 are single (A only).
        let profile = build(&blocks(&[(100.0, 110.0), (100.0, 103.0), (107.0, 110.0)]));
        let rows: Vec<i64> = profile
            .single_prints
            .iter()
            .map(|p| profile.rows[p.index].row)
            .collect();
        assert_eq!(rows, [104, 105, 106]);
        assert!(profile.single_prints.iter().all(|p| p.block == 0));
        let runs = profile.single_print_runs();
        assert_eq!(runs.len(), 1);
        assert_eq!(profile.first_established(runs[0]).unwrap().block, 0);
        // The extremes are never single prints even with one block.
        assert!(!rows.contains(&100) && !rows.contains(&110));
    }

    #[test]
    fn the_rotation_factor_sums_the_block_highs_and_lows_against_the_block_before() {
        // Highs 10, 12, 11 and lows 5, 4, 6: block 1: +1 (higher high), -1 (lower low) = 0;
        // block 2: -1 (lower high), +1 (higher low) = 0 -> 0. Make it asymmetric: highs 10, 12, 13,
        // lows 5, 5, 6: block 1: +1, 0; block 2: +1, +1 -> 3.
        let profile = build(&blocks(&[(5.0, 10.0), (5.0, 12.0), (6.0, 13.0)]));
        assert_eq!(profile.rotation, 3);
        let down = build(&blocks(&[(5.0, 10.0), (4.0, 9.0)]));
        assert_eq!(down.rotation, -2);
    }

    #[test]
    fn open_close_high_and_low_come_from_the_first_and_last_candles_and_the_extremes() {
        let candles = vec![
            candle(0, 101.0, 105.0, 99.0, 103.0, 1.0),
            candle(BLOCK, 103.0, 108.0, 102.0, 104.5, 1.0),
        ];
        let profile = build(&candles);
        assert_eq!((profile.open, profile.close), (101.0, 104.5));
        assert_eq!((profile.high, profile.low), (108.0, 99.0));
    }

    #[test]
    fn candles_outside_the_span_or_after_the_replay_position_are_not_read() {
        let candles = blocks(&[(100.0, 101.0), (200.0, 201.0), (300.0, 301.0)]);
        let cut = build_profile(span(), &candles, &params(), 2 * BLOCK).unwrap();
        assert_eq!(cut.blocks, 2);
        assert_eq!(cut.high, 201.0, "the third block is in the future");
        assert!(cut.forming);
        let whole = build(&candles);
        assert!(!whole.forming);
        // A candle after the span ends is another profile's.
        let late = vec![candle(DAY + BLOCK, 1.0, 2.0, 1.0, 2.0, 1.0)];
        assert_eq!(
            build_profile(span(), &late, &params(), i64::MAX),
            Err(Unsupported::NoVolume)
        );
    }

    #[test]
    fn a_gap_between_candles_leaves_empty_rows_and_a_range_too_tall_is_refused() {
        let gap = build(&blocks(&[(100.0, 101.0), (110.0, 111.0)]));
        assert_eq!(counts(&gap)[2..9], [0, 0, 0, 0, 0, 0, 0]);
        let fine = Params {
            grid: PriceGrid::new(0.01, 1).unwrap(),
            ..params()
        };
        assert_eq!(
            build_profile(span(), &blocks(&[(100.0, 140.0)]), &fine, i64::MAX),
            Err(Unsupported::TooManyRows)
        );
    }

    #[test]
    fn the_volume_profile_is_separate_from_the_tpo_counts() {
        // One block touches rows 100..=101 with volume 10 spread over them; a second block touches
        // only 100 with volume 90: row 100 has the most volume and the most blocks.
        let candles = vec![
            candle(0, 100.0, 102.0, 100.0, 102.0, 10.0),
            candle(BLOCK, 100.0, 101.0, 100.0, 100.0, 90.0),
        ];
        let profile = build(&candles);
        // The first candle's high of 102.0 is the bottom of row 102, which it therefore visited.
        assert_eq!(counts(&profile), [2, 2, 1]);
        let volumes: Vec<f64> = profile.rows.iter().map(|r| r.volume).collect();
        assert!((volumes[0] - 95.0).abs() < 1e-9 && (volumes[1] - 5.0).abs() < 1e-9);
        assert_eq!(
            volumes[2], 0.0,
            "touching the bottom of a row adds no volume to it"
        );
        assert_eq!(profile.volume_poc, Some(0));
        assert!((profile.volume_total() - 100.0).abs() < 1e-9);
        // No volume: no volume profile, TPO unaffected.
        let silent = build(&[candle(0, 100.0, 102.0, 100.0, 101.0, 0.0)]);
        assert_eq!(silent.volume_poc, None);
        assert_eq!(counts(&silent), [1, 1, 1]);
    }

    // MARK: Edits

    fn day(n: i64) -> Session {
        Session {
            start_ms: n * DAY,
            end_ms: (n + 1) * DAY,
        }
    }

    #[test]
    fn a_split_cuts_a_profile_in_two_at_the_instant() {
        let spans = apply_edits(
            vec![day(0), day(1)],
            &[TpoEdit::Split {
                at_ms: DAY + 3 * HOUR,
            }],
        );
        assert_eq!(
            spans,
            [
                day(0),
                Session {
                    start_ms: DAY,
                    end_ms: DAY + 3 * HOUR
                },
                Session {
                    start_ms: DAY + 3 * HOUR,
                    end_ms: 2 * DAY
                },
            ]
        );
    }

    #[test]
    fn a_merge_joins_a_profile_to_the_one_before_and_the_first_cannot_merge() {
        let merged = apply_edits(
            vec![day(0), day(1), day(2)],
            &[TpoEdit::Merge { at_ms: DAY }],
        );
        assert_eq!(
            merged,
            [
                Session {
                    start_ms: 0,
                    end_ms: 2 * DAY
                },
                day(2)
            ]
        );
        let first = apply_edits(vec![day(0), day(1)], &[TpoEdit::Merge { at_ms: 0 }]);
        assert_eq!(first, [day(0), day(1)]);
    }

    #[test]
    fn edits_replay_in_order_and_ones_that_name_nothing_do_nothing() {
        let edits = [
            TpoEdit::Split { at_ms: DAY + HOUR },
            TpoEdit::Merge { at_ms: DAY + HOUR }, // undoes the split
            TpoEdit::Split { at_ms: 100 * DAY },  // no such profile
            TpoEdit::Merge { at_ms: 7 },          // no profile starts there
            TpoEdit::Split { at_ms: DAY },        // on a boundary: nothing to cut
        ];
        assert_eq!(apply_edits(vec![day(0), day(1)], &edits), [day(0), day(1)]);
    }

    #[test]
    fn edits_round_trip_through_json() {
        let edits = vec![TpoEdit::Split { at_ms: 5 }, TpoEdit::Merge { at_ms: 9 }];
        let text = serde_json::to_string(&edits).unwrap();
        assert_eq!(text, r#"[{"split":{"at_ms":5}},{"merge":{"at_ms":9}}]"#);
        assert_eq!(serde_json::from_str::<Vec<TpoEdit>>(&text).unwrap(), edits);
    }

    // MARK: Row size

    #[test]
    fn the_automatic_row_size_follows_tradingviews_steps() {
        // 80 rows over the range: range 800 ticks -> 10 ticks a row -> step 5 -> 10.
        assert_eq!(auto_ticks_per_row(108.0, 100.0, 0.01), Some(10));
        // 11 250 ticks a row -> step 5000 -> 10 000; 12 500 -> 2.5 steps, rounded up -> 15 000.
        assert_eq!(auto_ticks_per_row(9_000.0, 0.0, 0.01), Some(10_000));
        assert_eq!(auto_ticks_per_row(10_000.0, 0.0, 0.01), Some(15_000));
        // 150 ticks a row -> step 50 -> 150.
        assert_eq!(auto_ticks_per_row(120.0, 0.0, 0.01), Some(150));
        // A range that is barely anything is at least one tick.
        assert_eq!(auto_ticks_per_row(100.001, 100.0, 1.0), Some(1));
        assert_eq!(auto_ticks_per_row(100.0, 100.0, 1.0), None);
        assert_eq!(auto_ticks_per_row(f64::NAN, 1.0, 1.0), None);
    }

    // MARK: Sets

    fn settings() -> TpoSettings {
        TpoSettings {
            row_mode: crate::model::chart_type::RowMode::Manual,
            ticks_per_row: 1,
            ..TpoSettings::default()
        }
    }

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

    fn book_for(days: i64) -> CandleBook {
        let mut book = CandleBook::new(5 * MIN, 100_000);
        book.insert((0..days * 288).map(|i| {
            let p = 100.0 + (i % 5) as f64;
            candle(i * 5 * MIN, p, p + 1.0, p - 1.0, p, 1.0)
        }));
        book
    }

    fn build_days(days: i64, shape: TpoShape, edits: &[TpoEdit], cache: &mut Cache) -> TpoSet {
        let bars = day_bars(days);
        build_set(
            SetInput {
                bars: &bars,
                bar_ms: DAY,
                candles: &book_for(days),
                exchange_tick: Some(1.0),
                shape,
                edits,
                until_ms: i64::MAX,
                now_ms: i64::MAX,
            },
            cache,
        )
    }

    #[test]
    fn a_set_has_one_profile_per_day_with_data() {
        let set = build_days(3, settings().shape(), &[], &mut Cache::default());
        assert_eq!(set.status, SetStatus::Ready);
        assert_eq!(set.profiles.len(), 3);
        assert_eq!(set.profiles[1].start_ms, DAY);
        assert_eq!(set.profile_at(DAY + HOUR).unwrap().start_ms, DAY);
        assert!(set.profile_at(10 * DAY).is_none());
        assert_eq!(set.block_ms, BLOCK);
        assert_eq!(set.profiles[0].blocks, 48);
    }

    #[test]
    fn weeks_and_months_group_the_days() {
        let mut weekly = settings();
        weekly.period = PeriodKind::Week;
        let set = build_days(12, weekly.shape(), &[], &mut Cache::default());
        // The epoch is a Thursday: days 0-3 are one week, 4-10 the next, 11 starts a third.
        assert_eq!(set.profiles.len(), 3);
        let mut monthly = settings();
        monthly.period = PeriodKind::Month;
        let set = build_days(40, monthly.shape(), &[], &mut Cache::default());
        assert_eq!(
            set.profiles.len(),
            2,
            "January and the start of February 1970"
        );
    }

    #[test]
    fn a_block_size_change_rebuilds_and_edits_change_the_spans() {
        let mut cache = Cache::default();
        let a = build_days(2, settings().shape(), &[], &mut cache);
        let mut hourly = settings();
        hourly.block_minutes = 60;
        let b = build_days(2, hourly.shape(), &[], &mut cache);
        assert_eq!(a.profiles[0].blocks, 48);
        assert_eq!(b.profiles[0].blocks, 24);
        let split = build_days(
            2,
            settings().shape(),
            &[TpoEdit::Split { at_ms: 12 * HOUR }],
            &mut Cache::default(),
        );
        assert_eq!(split.profiles.len(), 3);
        assert_eq!(split.profiles[0].end_ms, 12 * HOUR);
        assert_eq!(split.profiles[1].start_ms, 12 * HOUR);
        assert_eq!(
            split.profiles[1].blocks, 24,
            "blocks count from the new start"
        );
        let merged = build_days(
            2,
            settings().shape(),
            &[TpoEdit::Merge { at_ms: DAY }],
            &mut Cache::default(),
        );
        assert_eq!(merged.profiles.len(), 1);
        assert_eq!(merged.profiles[0].blocks, 96);
        assert_eq!(
            merged.previous_span(DAY),
            None,
            "after the merge nothing starts there"
        );
        assert_eq!(split.previous_span(12 * HOUR).unwrap().start_ms, 0);
    }

    #[test]
    fn finished_days_are_reused_and_only_the_forming_one_is_rebuilt() {
        let bars = day_bars(2);
        let mut book = book_for(2);
        let mut cache = Cache::default();
        let shape = settings().shape();
        let mut run = |book: &CandleBook| {
            build_set(
                SetInput {
                    bars: &bars,
                    bar_ms: DAY,
                    candles: book,
                    exchange_tick: Some(1.0),
                    shape,
                    edits: &[],
                    until_ms: i64::MAX,
                    now_ms: i64::MAX,
                },
                &mut cache,
            )
        };
        let first = run(&book);
        let again = run(&book);
        assert!(Arc::ptr_eq(&first.profiles[0], &again.profiles[0]));
        assert!(Arc::ptr_eq(&first.profiles[1], &again.profiles[1]));
        // A newer version of the last candle (the forming one) changes only the second day.
        let last = 2 * 288 - 1;
        book.insert([candle(last * 5 * MIN, 100.0, 120.0, 90.0, 110.0, 5.0)]);
        let live = run(&book);
        assert!(Arc::ptr_eq(&first.profiles[0], &live.profiles[0]));
        assert!(!Arc::ptr_eq(&first.profiles[1], &live.profiles[1]));
        assert_eq!(live.profiles[1].high, 120.0);
    }

    #[test]
    fn replay_hides_the_blocks_after_it() {
        let bars = day_bars(1);
        let book = book_for(1);
        let shape = settings().shape();
        let set = |until| {
            build_set(
                SetInput {
                    bars: &bars,
                    bar_ms: DAY,
                    candles: &book,
                    exchange_tick: Some(1.0),
                    shape,
                    edits: &[],
                    until_ms: until,
                    now_ms: i64::MAX,
                },
                &mut Cache::default(),
            )
        };
        let early = set(3 * HOUR);
        let full = set(i64::MAX);
        assert_eq!(early.profiles[0].blocks, 6);
        assert!(early.profiles[0].forming);
        assert_eq!(full.profiles[0].blocks, 48);
        // What the early profile shows is what the full one held at that point.
        for (index, row) in early.profiles[0].rows.iter().enumerate() {
            let later = full.profiles[0].row_at(row.row).unwrap();
            assert!(
                row.blocks.iter().all(|b| *b < 6),
                "row {index} shows only early blocks"
            );
            let early_blocks: Vec<u32> = later.blocks.iter().copied().filter(|b| *b < 6).collect();
            assert_eq!(row.blocks, early_blocks);
        }
    }

    #[test]
    fn a_day_the_clock_has_not_finished_is_forming_and_the_candles_after_it_do_not_exist() {
        let bars = day_bars(1);
        let book = book_for(1);
        let at = |now_ms| {
            build_set(
                SetInput {
                    bars: &bars,
                    bar_ms: DAY,
                    candles: &book,
                    exchange_tick: Some(1.0),
                    shape: settings().shape(),
                    edits: &[],
                    until_ms: i64::MAX,
                    now_ms,
                },
                &mut Cache::default(),
            )
        };
        let midday = at(12 * HOUR);
        assert!(midday.profiles[0].forming);
        assert_eq!(
            midday.profiles[0].blocks, 24,
            "only the blocks that have happened"
        );
        let evening = at(DAY + HOUR);
        assert!(!evening.profiles[0].forming);
        assert_eq!(evening.profiles[0].blocks, 48);
    }

    #[test]
    fn custom_sessions_are_the_profiles() {
        let mut custom = settings();
        custom.sessions = SessionMode::Custom;
        custom.custom[0] = sessions::SessionSpan::parse("0900-1700");
        let set = build_days(2, custom.shape(), &[], &mut Cache::default());
        assert_eq!(set.profiles.len(), 2);
        assert_eq!(set.profiles[0].start_ms, 9 * HOUR);
        assert_eq!(set.profiles[0].end_ms, 17 * HOUR);
        assert_eq!(set.profiles[0].blocks, 16);
    }

    #[test]
    fn nothing_is_built_without_prices_or_a_tick_and_candle_gaps_leave_no_profile() {
        let bars = day_bars(1);
        let none = build_set(
            SetInput {
                bars: &[],
                bar_ms: DAY,
                candles: &book_for(1),
                exchange_tick: Some(1.0),
                shape: settings().shape(),
                edits: &[],
                until_ms: i64::MAX,
                now_ms: i64::MAX,
            },
            &mut Cache::default(),
        );
        assert!(none.profiles.is_empty());
        let no_tick = vec![Kline {
            open: 1.234_567_890_123_4,
            ..day_bars(1)[0].clone()
        }];
        let set = build_set(
            SetInput {
                bars: &no_tick,
                bar_ms: DAY,
                candles: &book_for(1),
                exchange_tick: None,
                shape: settings().shape(),
                edits: &[],
                until_ms: i64::MAX,
                now_ms: i64::MAX,
            },
            &mut Cache::default(),
        );
        assert_eq!(set.status, SetStatus::Unsupported(Unsupported::NoTickSize));
        // A day with no candles has no profile; the others are unaffected.
        let mut book = CandleBook::new(5 * MIN, 1000);
        book.insert(book_for(1).between(0, DAY).cloned());
        let two = day_bars(2);
        let set = build_set(
            SetInput {
                bars: &two,
                bar_ms: DAY,
                candles: &book,
                exchange_tick: Some(1.0),
                shape: settings().shape(),
                edits: &[],
                until_ms: i64::MAX,
                now_ms: i64::MAX,
            },
            &mut Cache::default(),
        );
        assert_eq!(set.profiles.len(), 1);
        let _ = bars;
    }
}
