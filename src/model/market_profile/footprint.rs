//! The volume footprint: for each chart bar, the volume that traded at each price row, split into
//! what buyers and sellers did, with the point of control, value area and imbalances.
//!
//! **Where a bar's numbers come from**, best first ([`DataQuality`] says which, per bar):
//!
//! 1. *Trades*: every trade of the bar with the exchange's aggressor side, binned into rows. Exact.
//! 2. *Finer candles with the exchange's buy/sell split* (Binance taker-buy volume): each candle's
//!    split is exact, and its volume is spread evenly over its low-to-high (an estimate per price).
//! 3. *Finer candles*: each classed by TradingView's aggregated-intrabar rule (close above open is
//!    buy, below is sell, a doji follows the candle before it), volume spread evenly.
//! 4. *The chart's own candle*, the same rule on one candle: a coarse estimate.
//!
//! A bar is never built from trades *and* candles. **Imbalances exist only for trade-derived bars**:
//! an even spread makes every row of a candle look alike, so an imbalance computed on it would be an
//! artifact, not a signal.
//!
//! Imbalance test (TradingView): a *buy* imbalance compares a row's buy volume with the sell volume
//! of the row **below**, a *sell* imbalance a row's sell volume with the buy volume of the row
//! **above**, and holds when `max >= percent / 100 * min`. Worked without division, so a row against
//! an empty neighbour is an imbalance when it traded at all (`x >= ratio * 0`), never a NaN or
//! infinity, and the lowest row has no buy imbalance and the highest no sell imbalance.
//!
//! Pure: no egui, no clock, no network.

use super::flow::{CandleBook, FlowKline};
use super::price_grid::{MAX_ROWS, PriceGrid};
use super::quality::{DataQuality, Unsupported};
use super::tick::Tick;
use super::trades::{Trade, TradeBook};
use super::value_area;
use crate::model::chart_type::{FootprintShape, average_true_range};
use crate::model::kline::Kline;
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Most chart bars a set builds (the newest); older bars draw as plain candles.
pub const MAX_BARS: usize = 3_000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub grid: PriceGrid,
    /// The value-area share of a bar's volume, 0 to 1.
    pub value_area: f64,
    /// The imbalance ratio (`3.0` for TradingView's 300 %).
    pub imbalance: f64,
    /// Consecutive imbalanced rows of one side that make a stack.
    pub stacked_levels: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    Buy,
    Sell,
}

/// One price row of one bar.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub row: i64,
    pub buy: f64,
    pub sell: f64,
    pub buy_imbalance: bool,
    pub sell_imbalance: bool,
}

impl Cell {
    pub fn total(&self) -> f64 {
        self.buy + self.sell
    }

    pub fn delta(&self) -> f64 {
        self.buy - self.sell
    }
}

/// Consecutive imbalanced rows of one side (inclusive rows).
#[derive(Clone, Debug, PartialEq)]
pub struct Stack {
    pub side: Side,
    pub first_row: i64,
    pub last_row: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FootprintBar {
    pub open_time: i64,
    /// Exclusive end of the span the bar was built from.
    pub end_time: i64,
    pub quality: DataQuality,
    /// Every row from the lowest to the highest traded, lowest first; rows between that traded
    /// nothing are present with no volume.
    pub cells: Vec<Cell>,
    pub buy: f64,
    pub sell: f64,
    /// Index into `cells` of the point of control.
    pub poc: usize,
    /// Indexes into `cells` of the value area's lowest and highest rows.
    pub value_area: (usize, usize),
    pub stacks: Vec<Stack>,
}

impl FootprintBar {
    pub fn total(&self) -> f64 {
        self.buy + self.sell
    }

    pub fn delta(&self) -> f64 {
        self.buy - self.sell
    }

    /// Delta as a share of the bar's volume, in percent (0 for a bar with no volume).
    pub fn delta_percent(&self) -> f64 {
        let total = self.total();
        if total > 0.0 {
            100.0 * self.delta() / total
        } else {
            0.0
        }
    }

    pub fn cell_at(&self, row: i64) -> Option<&Cell> {
        let first = self.cells.first()?.row;
        usize::try_from(row - first)
            .ok()
            .and_then(|index| self.cells.get(index))
    }

    /// The busiest cell's total volume: what a profile-style cell width is measured against.
    pub fn peak(&self) -> f64 {
        self.cells.iter().map(Cell::total).fold(0.0, f64::max)
    }

    pub fn poc_row(&self) -> i64 {
        self.cells[self.poc].row
    }

    /// The highest and lowest rows of the value area.
    pub fn value_area_rows(&self) -> (i64, i64) {
        (
            self.cells[self.value_area.0].row,
            self.cells[self.value_area.1].row,
        )
    }
}

// MARK: Building one bar

#[derive(Default)]
struct Rows(BTreeMap<i64, (f64, f64)>);

impl Rows {
    fn add(&mut self, row: i64, buy: f64, sell: f64) {
        let entry = self.0.entry(row).or_insert((0.0, 0.0));
        entry.0 += buy;
        entry.1 += sell;
    }
}

fn finish(
    open_time: i64,
    end_time: i64,
    quality: DataQuality,
    rows: &Rows,
    params: &Params,
) -> Result<FootprintBar, Unsupported> {
    let (Some((&lowest, _)), Some((&highest, _))) =
        (rows.0.first_key_value(), rows.0.last_key_value())
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
    let mut cells: Vec<Cell> = (lowest..=highest)
        .map(|row| {
            let (buy, sell) = rows.0.get(&row).copied().unwrap_or((0.0, 0.0));
            Cell {
                row,
                buy,
                sell,
                buy_imbalance: false,
                sell_imbalance: false,
            }
        })
        .collect();
    let (buy, sell) = cells
        .iter()
        .fold((0.0, 0.0), |(b, s), cell| (b + cell.buy, s + cell.sell));
    if (buy + sell).is_nan() || buy + sell <= 0.0 {
        return Err(Unsupported::NoVolume);
    }
    let totals: Vec<f64> = cells.iter().map(Cell::total).collect();
    let middles: Vec<f64> = cells
        .iter()
        .map(|cell| params.grid.middle(cell.row))
        .collect();
    let range_middle = (params.grid.bottom(lowest) + params.grid.top(highest)) / 2.0;
    let (poc, value_area) = value_area::volume(&totals, &middles, range_middle, params.value_area)
        .ok_or(Unsupported::NoVolume)?;
    let mut stacks = Vec::new();
    if quality.has_real_rows() {
        mark_imbalances(&mut cells, params.imbalance);
        stacks = find_stacks(&cells, params.stacked_levels);
    }
    Ok(FootprintBar {
        open_time,
        end_time,
        quality,
        cells,
        buy,
        sell,
        poc,
        value_area,
        stacks,
    })
}

fn mark_imbalances(cells: &mut [Cell], ratio: f64) {
    let (buys, sells): (Vec<f64>, Vec<f64>) = cells.iter().map(|c| (c.buy, c.sell)).unzip();
    for (index, cell) in cells.iter_mut().enumerate() {
        // Buy volume here against the sell volume one row down.
        cell.buy_imbalance =
            index > 0 && buys[index] > 0.0 && buys[index] >= ratio * sells[index - 1];
        // Sell volume here against the buy volume one row up.
        cell.sell_imbalance = index + 1 < sells.len()
            && sells[index] > 0.0
            && sells[index] >= ratio * buys[index + 1];
    }
}

fn find_stacks(cells: &[Cell], levels: usize) -> Vec<Stack> {
    let mut stacks = Vec::new();
    for (side, flag) in [
        (Side::Buy, (|c: &Cell| c.buy_imbalance) as fn(&Cell) -> bool),
        (Side::Sell, |c: &Cell| c.sell_imbalance),
    ] {
        let mut run_start: Option<usize> = None;
        for index in 0..=cells.len() {
            let on = index < cells.len() && flag(&cells[index]);
            match (on, run_start) {
                (true, None) => run_start = Some(index),
                (false, Some(start)) => {
                    if index - start >= levels.max(2) {
                        stacks.push(Stack {
                            side,
                            first_row: cells[start].row,
                            last_row: cells[index - 1].row,
                        });
                    }
                    run_start = None;
                }
                _ => {}
            }
        }
    }
    stacks.sort_by_key(|stack| (stack.first_row, stack.side == Side::Sell));
    stacks
}

/// A bar from its trades: exact per price.
pub fn from_trades(
    open_time: i64,
    end_time: i64,
    trades: &[Trade],
    params: &Params,
) -> Result<FootprintBar, Unsupported> {
    let mut rows = Rows::default();
    for trade in trades {
        let Some(row) = params.grid.row_of(trade.price) else {
            continue;
        };
        if !(trade.quantity.is_finite() && trade.quantity > 0.0) {
            continue;
        }
        if trade.is_buy() {
            rows.add(row, trade.quantity, 0.0);
        } else {
            rows.add(row, 0.0, trade.quantity);
        }
    }
    finish(
        open_time,
        end_time,
        DataQuality::TradeDerived,
        &rows,
        params,
    )
}

/// TradingView's aggregated-intrabar class of a candle: buy when it closed above its open, sell
/// below, and a doji follows the candle before it (buy when there is none).
fn classify(open: f64, close: f64, previous: Option<(f64, bool)>) -> bool {
    if close > open {
        true
    } else if close < open {
        false
    } else {
        previous.is_none_or(|(_, buy)| buy)
    }
}

fn spread_into(
    rows: &mut Rows,
    grid: &PriceGrid,
    low: f64,
    high: f64,
    buy: f64,
    sell: f64,
) -> bool {
    for (volume, is_buy) in [(buy, true), (sell, false)] {
        if volume <= 0.0 {
            continue;
        }
        let Some(spread) = grid.spread(low, high, volume) else {
            return false;
        };
        for (row, share) in spread {
            if is_buy {
                rows.add(row, share, 0.0);
            } else {
                rows.add(row, 0.0, share);
            }
        }
    }
    true
}

/// A bar from the finer candles inside it. `previous_close` is the close of what came before the
/// first candle (the doji rule's reference).
pub fn from_candles<'a>(
    open_time: i64,
    end_time: i64,
    candles: impl IntoIterator<Item = &'a FlowKline>,
    params: &Params,
) -> Result<FootprintBar, Unsupported> {
    let mut rows = Rows::default();
    let mut previous: Option<(f64, bool)> = None;
    let (mut with_volume, mut with_split) = (0usize, 0usize);
    for candle in candles {
        let k = &candle.kline;
        if k.volume <= 0.0 {
            previous = Some((k.close, previous.is_none_or(|p| p.1)));
            continue;
        }
        with_volume += 1;
        let (buy, sell) = match candle.split() {
            Some(split) => {
                with_split += 1;
                split
            }
            None => {
                if classify(k.open, k.close, previous) {
                    (k.volume, 0.0)
                } else {
                    (0.0, k.volume)
                }
            }
        };
        previous = Some((k.close, buy >= sell));
        if !spread_into(&mut rows, &params.grid, k.low, k.high, buy, sell) {
            return Err(Unsupported::TooManyRows);
        }
    }
    let quality = if with_volume > 0 && with_split == with_volume {
        DataQuality::ExchangeSplit
    } else {
        DataQuality::Intrabar
    };
    finish(open_time, end_time, quality, &rows, params)
}

/// A bar from nothing but its own candle (no finer data): a coarse estimate.
pub fn from_chart_bar(
    bar: &Kline,
    end_time: i64,
    previous_close: Option<f64>,
    params: &Params,
) -> Result<FootprintBar, Unsupported> {
    let buy = classify(
        bar.open,
        bar.close,
        previous_close.map(|close| (close, true)),
    );
    let mut rows = Rows::default();
    if bar.volume > 0.0
        && !spread_into(
            &mut rows,
            &params.grid,
            bar.low,
            bar.high,
            if buy { bar.volume } else { 0.0 },
            if buy { 0.0 } else { bar.volume },
        )
    {
        return Err(Unsupported::TooManyRows);
    }
    finish(
        bar.open_time,
        end_time,
        DataQuality::ChartBars,
        &rows,
        params,
    )
}

// MARK: Which finer candles

/// The candle sizes a footprint is cut into, finest first.
pub const INTRABAR_SIZES: [(&str, i64); 5] = [
    ("1m", 60_000),
    ("5m", 300_000),
    ("15m", 900_000),
    ("30m", 1_800_000),
    ("1h", 3_600_000),
];

/// The finer candles a footprint of `bar_count` chart bars of `bar_ms` is built from, and how many
/// of the newest bars they cover.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntrabarPlan {
    pub interval: &'static str,
    pub interval_ms: i64,
    /// Bars (the newest) the candle budget covers; older bars are drawn from their own candle.
    pub bars: usize,
}

/// The finest candle size that divides the bar, is shorter than it, and keeps `bar_count` bars
/// within `cap` candles; failing that the coarsest that divides it, covering as many bars as `cap`
/// allows. `Err(BarTooShort)` when nothing finer than the bar divides it.
pub fn intrabar_plan(
    bar_ms: i64,
    bar_count: usize,
    cap: usize,
) -> Result<IntrabarPlan, Unsupported> {
    let fits: Vec<(&'static str, i64)> = INTRABAR_SIZES
        .iter()
        .copied()
        .filter(|(_, size)| *size < bar_ms && bar_ms % size == 0)
        .collect();
    let Some(coarsest) = fits.last().copied() else {
        return Err(Unsupported::BarTooShort);
    };
    let bar_count = bar_count.clamp(1, MAX_BARS);
    for (interval, interval_ms) in &fits {
        let per_bar = (bar_ms / interval_ms) as usize;
        if per_bar.saturating_mul(bar_count) <= cap {
            return Ok(IntrabarPlan {
                interval,
                interval_ms: *interval_ms,
                bars: bar_count,
            });
        }
    }
    let per_bar = (bar_ms / coarsest.1) as usize;
    Ok(IntrabarPlan {
        interval: coarsest.0,
        interval_ms: coarsest.1,
        bars: (cap / per_bar.max(1)).clamp(1, bar_count),
    })
}

// MARK: A set of bars

/// A stacked imbalance carried to the right until a later bar's range touches it.
#[derive(Clone, Debug, PartialEq)]
pub struct Projection {
    pub side: Side,
    /// Index (into the set's bars) of the bar the stack belongs to.
    pub from: usize,
    pub low: f64,
    pub high: f64,
    /// Index of the first later bar whose high-low range touches the stack; `None` while untouched.
    pub until: Option<usize>,
}

/// How many bars came from each source, for the chart's status line.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Counts {
    pub trades: usize,
    pub split: usize,
    pub intrabar: usize,
    pub chart_bars: usize,
    /// Bars that could not be built (and draw as plain candles).
    pub refused: usize,
}

/// Why a set has no cells at all.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetStatus {
    Ready,
    Unsupported(Unsupported),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FootprintSet {
    /// The open times of the bars the set was built for, oldest first (the chart's bars).
    pub open_times: Vec<i64>,
    /// One entry per `open_times`: the bar's footprint, `None` where it was not built.
    pub bars: Vec<Option<Arc<FootprintBar>>>,
    pub projections: Vec<Projection>,
    pub grid: Option<PriceGrid>,
    pub tick: Option<Tick>,
    pub counts: Counts,
    pub status: SetStatus,
    /// The first reason a bar was refused, if any (`TooManyRows` says to use a larger row size).
    pub first_refusal: Option<Unsupported>,
}

impl FootprintSet {
    /// A set with nothing built, for a chart whose data is not here (yet).
    pub fn empty(status: SetStatus) -> Self {
        Self {
            open_times: Vec::new(),
            bars: Vec::new(),
            projections: Vec::new(),
            grid: None,
            tick: None,
            counts: Counts::default(),
            status,
            first_refusal: None,
        }
    }

    /// The footprint of the chart bar opening at `open_time`.
    pub fn bar_at(&self, open_time: i64) -> Option<&Arc<FootprintBar>> {
        let index = self.open_times.binary_search(&open_time).ok()?;
        self.bars.get(index)?.as_ref()
    }
}

/// Bars already built, reused while what they were built from is unchanged: a completed bar is
/// computed once however many live trades arrive for the forming one.
#[derive(Default)]
pub struct Cache {
    key: Option<(PriceGrid, FootprintShape)>,
    bars: HashMap<i64, (u64, Result<Arc<FootprintBar>, Unsupported>)>,
}

/// Everything a set is built from.
pub struct SetInput<'a> {
    /// The chart's bars (already cut at the replay position), oldest first.
    pub bars: &'a [Kline],
    /// The nominal length of a bar, for the last one (the others end where the next opens).
    pub bar_ms: i64,
    pub candles: Option<&'a CandleBook>,
    pub trades: Option<&'a mut TradeBook>,
    /// The exchange's tick size, when it says.
    pub exchange_tick: Option<f64>,
    pub shape: FootprintShape,
}

fn fingerprint(parts: impl IntoIterator<Item = u64>) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for part in parts {
        part.hash(&mut hasher);
    }
    hasher.finish()
}

/// Builds the set. The newest [`MAX_BARS`] bars get footprints; the rest are left as `None`.
pub fn build_set(input: SetInput<'_>, cache: &mut Cache) -> FootprintSet {
    let SetInput {
        bars,
        bar_ms,
        candles,
        mut trades,
        exchange_tick,
        shape,
    } = input;
    let open_times: Vec<i64> = bars.iter().map(|bar| bar.open_time).collect();
    let none_built = |status| FootprintSet {
        open_times: open_times.clone(),
        bars: vec![None; open_times.len()],
        ..FootprintSet::empty(status)
    };
    if bars.is_empty() {
        return FootprintSet::empty(SetStatus::Ready);
    }
    if bars
        .iter()
        .all(|bar| bar.volume.is_nan() || bar.volume <= 0.0)
    {
        return none_built(SetStatus::Unsupported(Unsupported::NoVolume));
    }
    let Some(tick) = Tick::resolve(exchange_tick, bars) else {
        return none_built(SetStatus::Unsupported(Unsupported::NoTickSize));
    };
    let ticks_per_row = match shape.row_mode {
        crate::model::chart_type::RowMode::Manual => Some(shape.ticks_per_row.max(1)),
        crate::model::chart_type::RowMode::Auto => super::tick::auto_ticks_per_row(
            average_true_range(bars, shape.atr_length as usize),
            tick.size,
        ),
    };
    let Some(grid) = ticks_per_row.and_then(|per_row| PriceGrid::new(tick.size, per_row)) else {
        return none_built(SetStatus::Unsupported(Unsupported::NoTickSize));
    };
    let params = Params {
        grid,
        value_area: f64::from(shape.value_area_percent) / 100.0,
        imbalance: f64::from(shape.imbalance_percent) / 100.0,
        stacked_levels: shape.stacked_levels as usize,
    };
    if cache.key != Some((grid, shape)) {
        cache.bars.clear();
        cache.key = Some((grid, shape));
    }
    let first_built = bars.len().saturating_sub(MAX_BARS);
    let mut built: Vec<Option<Arc<FootprintBar>>> = vec![None; bars.len()];
    let mut counts = Counts::default();
    let mut first_refusal = None;
    let mut seen: Vec<i64> = Vec::new();
    for index in first_built..bars.len() {
        let bar = &bars[index];
        let end = bars
            .get(index + 1)
            .map_or(bar.open_time + bar_ms, |next| next.open_time);
        let previous_close = index.checked_sub(1).map(|i| bars[i].close);
        let trade_slice = match (shape.tick_based, trades.as_deref_mut()) {
            (true, Some(book)) => {
                if book.covers_from(bar.open_time) {
                    let slice = book.between(bar.open_time, end);
                    (!slice.is_empty()).then_some(slice)
                } else {
                    None
                }
            }
            _ => None,
        };
        let candle_slice: Vec<&FlowKline> = if trade_slice.is_none() {
            candles.map_or_else(Vec::new, |book| book.between(bar.open_time, end).collect())
        } else {
            Vec::new()
        };
        let signature = if let Some(slice) = trade_slice {
            fingerprint([
                1,
                slice.len() as u64,
                slice.first().map_or(0, |t| t.id),
                slice.last().map_or(0, |t| t.id),
                end as u64,
            ])
        } else if !candle_slice.is_empty() {
            fingerprint(
                [2, candle_slice.len() as u64, end as u64]
                    .into_iter()
                    .chain(candle_slice.iter().flat_map(|c| {
                        [
                            c.kline.open_time as u64,
                            c.kline.volume.to_bits(),
                            c.kline.high.to_bits(),
                            c.kline.low.to_bits(),
                            c.kline.close.to_bits(),
                            c.taker_buy.map_or(0, f64::to_bits),
                        ]
                    })),
            )
        } else {
            fingerprint([
                3,
                bar.open.to_bits(),
                bar.high.to_bits(),
                bar.low.to_bits(),
                bar.close.to_bits(),
                bar.volume.to_bits(),
                previous_close.map_or(0, f64::to_bits),
                end as u64,
            ])
        };
        seen.push(bar.open_time);
        let result = match cache.bars.get(&bar.open_time) {
            Some((cached, result)) if *cached == signature => result.clone(),
            _ => {
                let result = if let Some(slice) = trade_slice {
                    from_trades(bar.open_time, end, slice, &params)
                } else if !candle_slice.is_empty() {
                    from_candles(bar.open_time, end, candle_slice.iter().copied(), &params)
                } else {
                    from_chart_bar(bar, end, previous_close, &params)
                }
                .map(Arc::new);
                cache
                    .bars
                    .insert(bar.open_time, (signature, result.clone()));
                result
            }
        };
        match result {
            Ok(footprint) => {
                match footprint.quality {
                    DataQuality::TradeDerived => counts.trades += 1,
                    DataQuality::ExchangeSplit => counts.split += 1,
                    DataQuality::Intrabar => counts.intrabar += 1,
                    DataQuality::ChartBars => counts.chart_bars += 1,
                }
                built[index] = Some(footprint);
            }
            Err(Unsupported::NoVolume) => {}
            Err(reason) => {
                counts.refused += 1;
                first_refusal.get_or_insert(reason);
            }
        }
    }
    // Forget what is no longer in the window (a bar that scrolled out of the newest MAX_BARS).
    seen.sort_unstable();
    cache
        .bars
        .retain(|open_time, _| seen.binary_search(open_time).is_ok());
    let projections = project_stacks(bars, &built, &grid);
    FootprintSet {
        open_times,
        bars: built,
        projections,
        grid: Some(grid),
        tick: Some(tick),
        counts,
        status: SetStatus::Ready,
        first_refusal,
    }
}

/// Each stack carried right until a later bar's high-low range reaches it. A row owns the prices
/// from its bottom up to, not including, its top, so a bar that only touches the top edge of the
/// stack has not entered it.
fn project_stacks(
    bars: &[Kline],
    built: &[Option<Arc<FootprintBar>>],
    grid: &PriceGrid,
) -> Vec<Projection> {
    let mut projections = Vec::new();
    for (from, footprint) in built.iter().enumerate() {
        let Some(footprint) = footprint else { continue };
        for stack in &footprint.stacks {
            let (low, high) = (grid.bottom(stack.first_row), grid.top(stack.last_row));
            let until = (from + 1..bars.len())
                .find(|&later| bars[later].low < high && bars[later].high >= low);
            projections.push(Projection {
                side: stack.side,
                from,
                low,
                high,
                until,
            });
        }
    }
    projections
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::chart_type::{FootprintSettings, RowMode};

    const HOUR: i64 = 3_600_000;

    fn params(tick: f64, per_row: u32) -> Params {
        Params {
            grid: PriceGrid::new(tick, per_row).unwrap(),
            value_area: 0.7,
            imbalance: 3.0,
            stacked_levels: 3,
        }
    }

    fn trade(id: u64, time_ms: i64, price: f64, quantity: f64, buy: bool) -> Trade {
        Trade {
            id,
            time_ms,
            price,
            quantity,
            buyer_is_maker: !buy,
        }
    }

    /// One trade per `(row price, buy, sell)` entry side, ids counting up from `first_id`.
    fn trades_at(time_ms: i64, first_id: u64, cells: &[(f64, f64, f64)]) -> Vec<Trade> {
        let mut out = Vec::new();
        let mut id = first_id;
        for &(price, buy, sell) in cells {
            if buy > 0.0 {
                out.push(trade(id, time_ms, price, buy, true));
                id += 1;
            }
            if sell > 0.0 {
                out.push(trade(id, time_ms, price, sell, false));
                id += 1;
            }
        }
        out
    }

    /// A trade before everything else, so a book that starts here is complete from the first bar.
    fn lead() -> Trade {
        trade(1, -1, 100.0, 0.01, true)
    }

    fn flow(
        open_time: i64,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        volume: f64,
        taker: Option<f64>,
    ) -> FlowKline {
        FlowKline {
            kline: Kline {
                open_time,
                close_time: open_time + 59_999,
                open,
                high,
                low,
                close,
                volume,
            },
            taker_buy: taker,
        }
    }

    fn bar(open_time: i64, open: f64, high: f64, low: f64, close: f64, volume: f64) -> Kline {
        Kline {
            open_time,
            close_time: open_time + HOUR - 1,
            open,
            high,
            low,
            close,
            volume,
        }
    }

    fn total_of(footprint: &FootprintBar) -> f64 {
        footprint.cells.iter().map(Cell::total).sum()
    }

    // MARK: Trades

    #[test]
    fn trades_land_in_their_rows_with_their_side_and_nothing_is_lost() {
        let trades = trades_at(
            10,
            1,
            &[(100.0, 2.0, 1.0), (101.0, 4.0, 0.0), (103.0, 0.0, 5.0)],
        );
        let fp = from_trades(0, HOUR, &trades, &params(1.0, 1)).unwrap();
        assert_eq!(fp.quality, DataQuality::TradeDerived);
        // Rows 100..=103: row 102 traded nothing but is present.
        assert_eq!(
            fp.cells.iter().map(|c| c.row).collect::<Vec<_>>(),
            [100, 101, 102, 103]
        );
        assert_eq!((fp.cells[0].buy, fp.cells[0].sell), (2.0, 1.0));
        assert_eq!((fp.cells[2].buy, fp.cells[2].sell), (0.0, 0.0));
        assert_eq!((fp.buy, fp.sell), (6.0, 6.0));
        assert_eq!(fp.total(), 12.0);
        assert_eq!(total_of(&fp), 12.0, "volume is conserved");
        assert_eq!(fp.delta(), 0.0);
        assert_eq!(fp.delta_percent(), 0.0);
    }

    #[test]
    fn several_ticks_per_row_merge_trades_on_a_global_grid() {
        let trades = trades_at(
            0,
            1,
            &[(100.0, 1.0, 0.0), (101.0, 1.0, 0.0), (102.0, 0.0, 2.0)],
        );
        // Rows of 2 ticks of 1.0: 100 and 101 are row 50, 102 is row 51.
        let fp = from_trades(0, HOUR, &trades, &params(1.0, 2)).unwrap();
        assert_eq!(fp.cells.iter().map(|c| c.row).collect::<Vec<_>>(), [50, 51]);
        assert_eq!(fp.cells[0].buy, 2.0);
        assert_eq!(fp.cells[1].sell, 2.0);
    }

    #[test]
    fn a_float_price_on_a_boundary_goes_in_the_row_it_belongs_to() {
        let trades = trades_at(0, 1, &[(0.1 + 0.2, 1.0, 0.0), (0.3, 1.0, 0.0)]);
        let fp = from_trades(0, HOUR, &trades, &params(0.1, 1)).unwrap();
        assert_eq!(fp.cells.len(), 1, "both are row 3");
        assert_eq!(fp.cells[0].buy, 2.0);
    }

    #[test]
    fn the_poc_is_the_busiest_row_and_the_value_area_grows_around_it() {
        let trades = trades_at(
            0,
            1,
            &[
                (100.0, 1.0, 0.0),
                (101.0, 2.0, 1.0),
                (102.0, 9.0, 6.0), // 15: the POC
                (103.0, 3.0, 2.0),
                (104.0, 1.0, 0.0),
            ],
        );
        let fp = from_trades(0, HOUR, &trades, &params(1.0, 1)).unwrap();
        assert_eq!(fp.poc_row(), 102);
        let (low, high) = fp.value_area_rows();
        assert!(low <= 102 && 102 <= high);
        let held: f64 = fp.cells[fp.value_area.0..=fp.value_area.1]
            .iter()
            .map(Cell::total)
            .sum();
        assert!(
            held >= 0.7 * fp.total() - 1e-9,
            "the area holds at least 70 %"
        );
    }

    #[test]
    fn imbalances_compare_diagonally_and_the_edges_have_none() {
        // rows 0..3 (prices 100..103): buy/sell
        // r0 0/4, r1 12/1, r2 2/9, r3 3/0
        let trades = trades_at(
            0,
            1,
            &[
                (100.0, 0.0, 4.0),
                (101.0, 12.0, 1.0),
                (102.0, 2.0, 9.0),
                (103.0, 3.0, 0.0),
            ],
        );
        let fp = from_trades(0, HOUR, &trades, &params(1.0, 1)).unwrap();
        let buys: Vec<bool> = fp.cells.iter().map(|c| c.buy_imbalance).collect();
        let sells: Vec<bool> = fp.cells.iter().map(|c| c.sell_imbalance).collect();
        // r1 buys 12 against r0's sells 4: 12 >= 3 * 4, exactly on the threshold.
        assert_eq!(buys, [false, true, false, false]);
        // r2 sells 9 against r3's buys 3: 9 >= 3 * 3, exactly on the threshold.
        assert_eq!(sells, [false, false, true, false]);
    }

    #[test]
    fn just_under_the_threshold_is_not_an_imbalance() {
        let trades = trades_at(0, 1, &[(100.0, 0.0, 4.0), (101.0, 11.9, 0.0)]);
        let fp = from_trades(0, HOUR, &trades, &params(1.0, 1)).unwrap();
        assert!(!fp.cells[1].buy_imbalance);
    }

    #[test]
    fn a_row_against_an_empty_neighbour_is_an_imbalance_only_if_it_traded() {
        // r0 bought 1 and sold nothing; r1 buys 1 against r0's 0 sells: 1 >= 3 * 0, so yes. r2 has
        // no buys at all: no.
        let trades = trades_at(
            0,
            1,
            &[(100.0, 1.0, 0.0), (101.0, 1.0, 0.0), (102.0, 0.0, 2.0)],
        );
        let fp = from_trades(0, HOUR, &trades, &params(1.0, 1)).unwrap();
        assert!(fp.cells[1].buy_imbalance);
        assert!(!fp.cells[0].buy_imbalance);
        assert!(!fp.cells[2].buy_imbalance);
        // r1 sells nothing: not a sell imbalance whatever sits above it.
        assert!(!fp.cells[1].sell_imbalance);
    }

    #[test]
    fn three_consecutive_imbalances_of_one_side_make_a_stack_and_two_do_not() {
        // Sells at the bottom (4), then buys climbing 12, 12, 12 over sells of 4: buy imbalances at
        // rows 1, 2, 3 (each buy 12 against the sell 4 below).
        let stacked = trades_at(
            0,
            1,
            &[
                (100.0, 0.0, 4.0),
                (101.0, 12.0, 4.0),
                (102.0, 12.0, 4.0),
                (103.0, 12.0, 0.0),
            ],
        );
        let fp = from_trades(0, HOUR, &stacked, &params(1.0, 1)).unwrap();
        assert_eq!(
            fp.stacks,
            vec![Stack {
                side: Side::Buy,
                first_row: 101,
                last_row: 103
            }]
        );
        let two = trades_at(
            0,
            1,
            &[(100.0, 0.0, 4.0), (101.0, 12.0, 4.0), (102.0, 12.0, 0.0)],
        );
        let fp = from_trades(0, HOUR, &two, &params(1.0, 1)).unwrap();
        assert!(fp.stacks.is_empty());
    }

    #[test]
    fn a_zero_volume_or_empty_bar_is_refused_not_drawn() {
        assert_eq!(
            from_trades(0, HOUR, &[], &params(1.0, 1)),
            Err(Unsupported::NoVolume)
        );
        let nan = [
            trade(1, 0, f64::NAN, 1.0, true),
            trade(2, 0, 100.0, f64::NAN, true),
        ];
        assert_eq!(
            from_trades(0, HOUR, &nan, &params(1.0, 1)),
            Err(Unsupported::NoVolume)
        );
    }

    #[test]
    fn a_row_size_that_needs_too_many_rows_is_refused() {
        let trades = trades_at(0, 1, &[(100.0, 1.0, 0.0), (140.0, 0.0, 1.0)]);
        // 40 / 0.01 = 4000 rows.
        assert_eq!(
            from_trades(0, HOUR, &trades, &params(0.01, 1)),
            Err(Unsupported::TooManyRows)
        );
        assert!(from_trades(0, HOUR, &trades, &params(0.01, 10)).is_ok());
    }

    // MARK: Candles

    #[test]
    fn candles_with_the_exchange_split_keep_each_candles_buy_and_sell_exactly() {
        // One candle 99..101, volume 10, takers bought 4: rows 99 and 100 each get half.
        let candles = [flow(0, 100.0, 101.0, 99.0, 100.5, 10.0, Some(4.0))];
        let fp = from_candles(0, HOUR, &candles, &params(1.0, 1)).unwrap();
        assert_eq!(fp.quality, DataQuality::ExchangeSplit);
        assert!((fp.buy - 4.0).abs() < 1e-12 && (fp.sell - 6.0).abs() < 1e-12);
        assert!((total_of(&fp) - 10.0).abs() < 1e-12);
        assert_eq!(fp.cells.len(), 2);
        assert!((fp.cells[0].buy - 2.0).abs() < 1e-12);
        assert!((fp.cells[1].sell - 3.0).abs() < 1e-12);
        // An even spread would make imbalances artifacts: none are computed.
        assert!(
            fp.cells
                .iter()
                .all(|c| !c.buy_imbalance && !c.sell_imbalance)
        );
        assert!(fp.stacks.is_empty());
    }

    #[test]
    fn candles_without_a_split_use_the_close_versus_open_rule() {
        let candles = [
            flow(0, 100.0, 101.0, 99.0, 101.0, 4.0, None), // up: buy
            flow(60_000, 101.0, 102.0, 100.0, 100.0, 6.0, None), // down: sell
            flow(120_000, 100.0, 101.0, 100.0, 100.0, 2.0, None), // doji after a sell: sell
        ];
        let fp = from_candles(0, HOUR, &candles, &params(1.0, 1)).unwrap();
        assert_eq!(fp.quality, DataQuality::Intrabar);
        assert!((fp.buy - 4.0).abs() < 1e-9);
        assert!((fp.sell - 8.0).abs() < 1e-9);
        assert!((total_of(&fp) - 12.0).abs() < 1e-9);
    }

    #[test]
    fn a_leading_doji_is_a_buy_and_a_doji_after_a_buy_stays_a_buy() {
        let candles = [
            flow(0, 100.0, 100.0, 100.0, 100.0, 1.0, None),
            flow(60_000, 100.0, 101.0, 99.0, 101.0, 2.0, None),
            flow(120_000, 101.0, 101.0, 101.0, 101.0, 3.0, None),
        ];
        let fp = from_candles(0, HOUR, &candles, &params(1.0, 1)).unwrap();
        assert!((fp.buy - 6.0).abs() < 1e-9 && fp.sell == 0.0);
    }

    #[test]
    fn a_bar_mixing_split_and_unsplit_candles_is_only_an_intrabar_estimate() {
        let candles = [
            flow(0, 100.0, 101.0, 99.0, 100.5, 10.0, Some(4.0)),
            flow(60_000, 100.0, 101.0, 99.0, 100.5, 10.0, None),
        ];
        let fp = from_candles(0, HOUR, &candles, &params(1.0, 1)).unwrap();
        assert_eq!(fp.quality, DataQuality::Intrabar);
    }

    #[test]
    fn the_chart_candle_alone_is_a_coarse_estimate() {
        let b = bar(0, 100.0, 103.0, 99.0, 102.0, 8.0);
        let fp = from_chart_bar(&b, HOUR, None, &params(1.0, 1)).unwrap();
        assert_eq!(fp.quality, DataQuality::ChartBars);
        assert!((fp.buy - 8.0).abs() < 1e-9 && fp.sell == 0.0);
        assert!((total_of(&fp) - 8.0).abs() < 1e-9);
        let none = bar(0, 100.0, 103.0, 99.0, 102.0, 0.0);
        assert_eq!(
            from_chart_bar(&none, HOUR, None, &params(1.0, 1)),
            Err(Unsupported::NoVolume)
        );
    }

    // MARK: Sets

    fn shape(per_row: u32) -> FootprintShape {
        FootprintSettings {
            row_mode: RowMode::Manual,
            ticks_per_row: per_row,
            ..FootprintSettings::default()
        }
        .shape()
    }

    fn two_bars() -> Vec<Kline> {
        vec![
            bar(0, 100.0, 103.0, 100.0, 102.0, 10.0),
            bar(HOUR, 102.0, 104.0, 101.0, 103.0, 12.0),
        ]
    }

    #[test]
    fn a_bar_the_trade_book_covers_is_built_from_trades_and_the_rest_from_candles() {
        let bars = two_bars();
        // Trades only for the second bar, ids contiguous from 1.
        let mut book = TradeBook::default();
        book.insert(&[lead()]);
        book.insert(&trades_at(
            HOUR + 5,
            2,
            &[(102.0, 3.0, 1.0), (103.0, 2.0, 2.0)],
        ));
        let mut candles = CandleBook::new(60_000, 100);
        candles.insert([flow(0, 100.0, 102.0, 100.0, 101.0, 10.0, Some(6.0))]);
        let set = build_set(
            SetInput {
                bars: &bars,
                bar_ms: HOUR,
                candles: Some(&candles),
                trades: Some(&mut book),
                exchange_tick: Some(1.0),
                shape: shape(1),
            },
            &mut Cache::default(),
        );
        assert_eq!(set.status, SetStatus::Ready);
        assert_eq!(
            set.bars[0].as_ref().unwrap().quality,
            DataQuality::ExchangeSplit
        );
        assert_eq!(
            set.bars[1].as_ref().unwrap().quality,
            DataQuality::TradeDerived
        );
        assert_eq!((set.counts.trades, set.counts.split), (1, 1));
        assert_eq!(set.tick.unwrap().size, 1.0);
        assert_eq!(set.bar_at(HOUR).unwrap().open_time, HOUR);
        assert!(set.bar_at(7).is_none());
    }

    #[test]
    fn a_bar_with_no_finer_data_falls_back_to_the_chart_candle() {
        let bars = two_bars();
        let set = build_set(
            SetInput {
                bars: &bars,
                bar_ms: HOUR,
                candles: None,
                trades: None,
                exchange_tick: Some(1.0),
                shape: shape(1),
            },
            &mut Cache::default(),
        );
        assert_eq!(set.counts.chart_bars, 2);
        assert!(
            set.bars
                .iter()
                .all(|b| b.as_ref().unwrap().quality == DataQuality::ChartBars)
        );
    }

    #[test]
    fn turning_tick_based_off_ignores_the_trades() {
        let bars = two_bars();
        let mut book = TradeBook::default();
        book.insert(&trades_at(HOUR + 5, 1, &[(102.0, 3.0, 1.0)]));
        let mut off = FootprintSettings {
            row_mode: RowMode::Manual,
            ticks_per_row: 1,
            tick_based: false,
            ..FootprintSettings::default()
        };
        off.tick_based = false;
        let set = build_set(
            SetInput {
                bars: &bars,
                bar_ms: HOUR,
                candles: None,
                trades: Some(&mut book),
                exchange_tick: Some(1.0),
                shape: off.shape(),
            },
            &mut Cache::default(),
        );
        assert_eq!(set.counts.trades, 0);
    }

    #[test]
    fn a_trade_book_that_misses_the_start_of_a_bar_is_not_used_for_it() {
        let bars = two_bars();
        let mut book = TradeBook::default();
        // The book starts after the second bar's open (ids contiguous but late).
        book.insert(&trades_at(HOUR + 1_000_000, 1, &[(102.0, 3.0, 1.0)]));
        let set = build_set(
            SetInput {
                bars: &bars,
                bar_ms: HOUR,
                candles: None,
                trades: Some(&mut book),
                exchange_tick: Some(1.0),
                shape: shape(1),
            },
            &mut Cache::default(),
        );
        assert_eq!(
            set.counts.trades, 0,
            "partial trade coverage is never mixed in"
        );
    }

    #[test]
    fn completed_bars_are_reused_and_only_the_changed_one_is_rebuilt() {
        let bars = two_bars();
        let mut book = TradeBook::default();
        book.insert(&[lead()]);
        book.insert(&trades_at(5, 2, &[(100.0, 1.0, 1.0)])); // ids 2, 3
        book.insert(&trades_at(HOUR + 5, 4, &[(102.0, 3.0, 1.0)])); // ids 4, 5
        let mut cache = Cache::default();
        let build = |book: &mut TradeBook, cache: &mut Cache| {
            build_set(
                SetInput {
                    bars: &bars,
                    bar_ms: HOUR,
                    candles: None,
                    trades: Some(book),
                    exchange_tick: Some(1.0),
                    shape: shape(1),
                },
                cache,
            )
        };
        let first = build(&mut book, &mut cache);
        let again = build(&mut book, &mut cache);
        assert!(Arc::ptr_eq(
            first.bars[0].as_ref().unwrap(),
            again.bars[0].as_ref().unwrap()
        ));
        assert!(Arc::ptr_eq(
            first.bars[1].as_ref().unwrap(),
            again.bars[1].as_ref().unwrap()
        ));
        // A live trade for the forming (second) bar: only it is rebuilt.
        book.insert(&[trade(6, HOUR + 9, 103.0, 5.0, true)]);
        let live = build(&mut book, &mut cache);
        assert!(Arc::ptr_eq(
            first.bars[0].as_ref().unwrap(),
            live.bars[0].as_ref().unwrap()
        ));
        assert!(!Arc::ptr_eq(
            first.bars[1].as_ref().unwrap(),
            live.bars[1].as_ref().unwrap()
        ));
        assert_eq!(live.bars[1].as_ref().unwrap().buy, 8.0);
    }

    #[test]
    fn a_changed_row_size_rebuilds_everything() {
        let bars = two_bars();
        let mut cache = Cache::default();
        let mut build = |per_row| {
            build_set(
                SetInput {
                    bars: &bars,
                    bar_ms: HOUR,
                    candles: None,
                    trades: None,
                    exchange_tick: Some(1.0),
                    shape: shape(per_row),
                },
                &mut cache,
            )
        };
        let one = build(1);
        let two = build(2);
        assert!(!Arc::ptr_eq(
            one.bars[0].as_ref().unwrap(),
            two.bars[0].as_ref().unwrap()
        ));
        assert!(
            two.bars[0].as_ref().unwrap().cells.len() < one.bars[0].as_ref().unwrap().cells.len()
        );
    }

    #[test]
    fn a_set_built_from_a_prefix_of_the_bars_is_a_prefix_of_the_full_set() {
        // Replay hands the builder the bars up to the replay position: with a fixed row size, what
        // it builds for them must be what the full set holds for the same bars.
        let bars: Vec<Kline> = (0..6)
            .map(|i| {
                bar(
                    i * HOUR,
                    100.0 + i as f64,
                    104.0 + i as f64,
                    99.0 + i as f64,
                    102.0 + i as f64,
                    5.0 + i as f64,
                )
            })
            .collect();
        let build = |count: usize| {
            build_set(
                SetInput {
                    bars: &bars[..count],
                    bar_ms: HOUR,
                    candles: None,
                    trades: None,
                    exchange_tick: Some(1.0),
                    shape: shape(1),
                },
                &mut Cache::default(),
            )
        };
        let full = build(6);
        for count in 1..6 {
            let prefix = build(count);
            for i in 0..count {
                assert_eq!(prefix.bars[i], full.bars[i], "bar {i} of {count}");
            }
        }
    }

    #[test]
    fn a_stack_is_projected_until_a_later_bar_reaches_it() {
        let bars = vec![
            bar(0, 100.0, 103.0, 100.0, 102.0, 10.0),
            bar(HOUR, 104.0, 106.0, 104.0, 105.0, 10.0), // does not reach 101..104
            bar(2 * HOUR, 105.0, 105.0, 102.0, 103.0, 10.0), // low 102 reaches it
        ];
        let mut book = TradeBook::default();
        book.insert(&[lead()]);
        book.insert(&trades_at(
            5,
            2,
            &[
                (100.0, 0.0, 4.0),
                (101.0, 12.0, 4.0),
                (102.0, 12.0, 4.0),
                (103.0, 12.0, 0.0),
            ],
        ));
        let set = build_set(
            SetInput {
                bars: &bars,
                bar_ms: HOUR,
                candles: None,
                trades: Some(&mut book),
                exchange_tick: Some(1.0),
                shape: shape(1),
            },
            &mut Cache::default(),
        );
        assert_eq!(set.projections.len(), 1);
        let projection = &set.projections[0];
        assert_eq!(
            (projection.side, projection.from, projection.until),
            (Side::Buy, 0, Some(2))
        );
        assert_eq!((projection.low, projection.high), (101.0, 104.0));
    }

    #[test]
    fn nothing_is_built_for_a_market_without_volume_or_a_tick() {
        let flat = vec![bar(0, 100.0, 101.0, 99.0, 100.0, 0.0)];
        let no_volume = build_set(
            SetInput {
                bars: &flat,
                bar_ms: HOUR,
                candles: None,
                trades: None,
                exchange_tick: Some(1.0),
                shape: shape(1),
            },
            &mut Cache::default(),
        );
        assert_eq!(
            no_volume.status,
            SetStatus::Unsupported(Unsupported::NoVolume)
        );
        let odd = vec![bar(0, 1.234_567_890_123_4, 2.0, 1.0, 1.5, 3.0)];
        let no_tick = build_set(
            SetInput {
                bars: &odd,
                bar_ms: HOUR,
                candles: None,
                trades: None,
                exchange_tick: None,
                shape: shape(1),
            },
            &mut Cache::default(),
        );
        assert_eq!(
            no_tick.status,
            SetStatus::Unsupported(Unsupported::NoTickSize)
        );
    }

    #[test]
    fn an_oversized_row_count_refuses_the_bar_and_says_so() {
        let bars = vec![bar(0, 100.0, 140.0, 100.0, 120.0, 5.0)];
        let set = build_set(
            SetInput {
                bars: &bars,
                bar_ms: HOUR,
                candles: None,
                trades: None,
                exchange_tick: Some(0.01),
                shape: shape(1),
            },
            &mut Cache::default(),
        );
        assert_eq!(set.counts.refused, 1);
        assert_eq!(set.first_refusal, Some(Unsupported::TooManyRows));
        assert!(set.bars[0].is_none());
    }

    #[test]
    fn the_finest_candle_that_fits_the_budget_is_chosen() {
        // 1h bars, 500 of them: 1m is 60 per bar = 30 000 candles, within 60 000.
        let plan = intrabar_plan(HOUR, 500, 60_000).unwrap();
        assert_eq!((plan.interval, plan.bars), ("1m", 500));
        // 3000 hourly bars need 180 000 1m candles: too many; 5m is 36 000.
        let plan = intrabar_plan(HOUR, 3_000, 60_000).unwrap();
        assert_eq!((plan.interval, plan.bars), ("5m", 3_000));
        // Daily bars: 1m is 1440 per bar (720 000 for 500), 5m 288 (144 000), 15m 96 (48 000).
        let plan = intrabar_plan(24 * HOUR, 500, 60_000).unwrap();
        assert_eq!(plan.interval, "15m");
    }

    #[test]
    fn a_bar_too_long_for_the_budget_covers_only_the_newest() {
        // Weekly bars on 1h candles: 168 per bar; 60 000 / 168 = 357 bars.
        let plan = intrabar_plan(7 * 24 * HOUR, 3_000, 60_000).unwrap();
        assert_eq!(plan.interval, "1h");
        assert_eq!(plan.bars, 357);
    }

    #[test]
    fn a_bar_with_nothing_finer_has_no_plan() {
        assert_eq!(
            intrabar_plan(60_000, 10, 60_000),
            Err(Unsupported::BarTooShort)
        );
        assert_eq!(
            intrabar_plan(30_000, 10, 60_000),
            Err(Unsupported::BarTooShort)
        );
        // A bar no candle size divides.
        assert_eq!(
            intrabar_plan(90_000, 10, 60_000),
            Err(Unsupported::BarTooShort)
        );
    }

    #[test]
    fn the_automatic_row_size_follows_the_average_true_range() {
        // True range 10 every bar, tick 0.5: a fifth of 10 is 2.0 = 4 ticks.
        let bars: Vec<Kline> = (0..20)
            .map(|i| bar(i * HOUR, 100.0, 110.0, 100.0, 105.0, 4.0))
            .collect();
        let auto = FootprintSettings::default().shape();
        let set = build_set(
            SetInput {
                bars: &bars,
                bar_ms: HOUR,
                candles: None,
                trades: None,
                exchange_tick: Some(0.5),
                shape: auto,
            },
            &mut Cache::default(),
        );
        assert_eq!(set.grid.unwrap().ticks_per_row(), 4);
    }
}
