//! Volume analysis over the bars the chart draws: the anchored VWAP and the volume profile (with
//! its point of control and value area).
//!
//! **What the data can and cannot say.** Both are worked from the bars' volume. A bar carries one
//! volume and a high and a low, not the trades inside it, so a profile spreads each bar's volume
//! evenly over the prices from its low to its high (the usual approximation) and calls it up
//! volume when the bar closed above its open and down volume otherwise (TradingView's own rule for a
//! single bar). TradingView refines this with lower-timeframe bars (down to 1 second); this build
//! has none under an hour, so what it shows is a distribution of the chart's own bars, not the
//! tick-accurate one, and must not be read as such. A market with no volume (some providers send
//! none) has no VWAP and no profile, and says so rather than inventing one.
//!
//! Pure: no egui types, nothing read but the slice of bars it is given.

use super::chart_type::PriceSource;
use super::kline::Kline;
use serde::{Deserialize, Serialize};

/// The most rows a profile is built with, however many are asked for.
pub const MAX_ROWS: u32 = 500;
/// The fewest.
pub const MIN_ROWS: u32 = 1;

// MARK: Anchored VWAP

/// The volume-weighted average price from the anchor bar to one bar, and the volume-weighted
/// standard deviation of the source price around it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VwapPoint {
    /// Index into the bars given.
    pub index: usize,
    pub vwap: f64,
    /// The band half-width of one standard deviation (`band = vwap ± multiplier × deviation`).
    pub deviation: f64,
}

/// The anchored VWAP of `source` from bar `anchor` to the last bar: at each bar
/// `Σ(price × volume) / Σ(volume)` over the bars from the anchor to it, the anchor bar included.
/// The deviation is the volume-weighted standard deviation `√(Σ(volume × price²) / Σ(volume) −
/// VWAP²)`, which is what the bands are multiples of.
///
/// A bar before any volume has traded has no VWAP, so the first points are skipped; a series with
/// no volume at all is empty. `anchor` past the last bar is empty. Non-finite bars are skipped
/// rather than poisoning every later point.
pub fn anchored_vwap(bars: &[Kline], anchor: usize, source: PriceSource) -> Vec<VwapPoint> {
    let mut points = Vec::new();
    let (mut volume, mut weighted, mut squared) = (0.0, 0.0, 0.0);
    for (index, bar) in bars.iter().enumerate().skip(anchor) {
        let price = source.value(bar);
        if !(price.is_finite() && bar.volume.is_finite() && bar.volume >= 0.0) {
            continue;
        }
        volume += bar.volume;
        weighted += price * bar.volume;
        squared += price * price * bar.volume;
        if volume > 0.0 {
            let vwap = weighted / volume;
            // Rounding can leave a hair under zero where every price is the same.
            let variance = (squared / volume - vwap * vwap).max(0.0);
            if vwap.is_finite() && variance.is_finite() {
                points.push(VwapPoint {
                    index,
                    vwap,
                    deviation: variance.sqrt(),
                });
            }
        }
    }
    points
}

// MARK: Volume profile

/// How the profile's rows are sized.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RowSize {
    /// This many rows between the lowest low and the highest high.
    Count(u32),
    /// Rows this tall, in price; as many as it takes.
    Height(f64),
}

/// One row of a profile: a band of prices and the volume traded in it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row {
    pub low: f64,
    pub high: f64,
    pub up: f64,
    pub down: f64,
}

impl Row {
    pub fn total(&self) -> f64 {
        self.up + self.down
    }

    pub fn middle(&self) -> f64 {
        (self.low + self.high) / 2.0
    }
}

/// A volume profile: the rows from the lowest price up, the row of most volume (the point of
/// control) and the rows the value area spans.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub rows: Vec<Row>,
    /// Index of the point of control.
    pub poc: usize,
    /// The lowest and highest rows of the value area (inclusive); `low <= poc <= high`.
    pub value_area: (usize, usize),
    pub total: f64,
}

impl Profile {
    /// The top of the value area.
    pub fn value_area_high(&self) -> f64 {
        self.rows[self.value_area.1].high
    }

    /// The bottom of the value area.
    pub fn value_area_low(&self) -> f64 {
        self.rows[self.value_area.0].low
    }

    /// The price of the point of control: the middle of its row.
    pub fn poc_price(&self) -> f64 {
        self.rows[self.poc].middle()
    }

    /// The most volume any row holds, the scale the histogram is drawn to.
    pub fn peak(&self) -> f64 {
        self.rows[self.poc].total()
    }
}

/// The profile of the bars `first..=last` (indexes into `bars`, `last` clamped to the last bar).
///
/// - The rows span the lowest low to the highest high of those bars.
/// - Each bar's volume is spread evenly over its own low-to-high, so a row gets the share of the bar
///   its prices overlap; a bar with no range puts all its volume in the row its price is in. A
///   bar is up volume when it closed **above** its open, down volume otherwise.
/// - The **point of control** is the row of most volume; two equal rows are told apart by which is
///   nearer the middle of the whole range, then the lower (**inferred**: nothing found says).
/// - The **value area** starts at the point of control and grows by whichever pair of rows beside it
///   holds more volume (two rows above against two below; one when only one is left) until it holds
///   at least `value_area` (a fraction, 0.7 is TradingView's default) of the total volume. This is the
///   classical market-profile method; TradingView does not document its own, so it is **inferred**,
///   and it is not simply the middle 70 % of the price range. A tie grows upward.
///
/// `None` when there is nothing to profile: no bars in range, no volume, a range that is not a
/// number.
pub fn profile(
    bars: &[Kline],
    first: usize,
    last: usize,
    size: RowSize,
    value_area: f64,
) -> Option<Profile> {
    if bars.is_empty() || first >= bars.len() {
        return None;
    }
    let last = last.min(bars.len() - 1);
    if last < first {
        return None;
    }
    let slice = &bars[first..=last];
    let usable = |bar: &&Kline| {
        bar.high.is_finite() && bar.low.is_finite() && bar.volume.is_finite() && bar.volume > 0.0
    };
    let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
    for bar in slice.iter().filter(usable) {
        low = low.min(bar.low.min(bar.high));
        high = high.max(bar.high.max(bar.low));
    }
    if !(low.is_finite() && high.is_finite()) {
        return None;
    }
    let span = high - low;
    let count = match size {
        RowSize::Count(rows) => rows.clamp(MIN_ROWS, MAX_ROWS),
        RowSize::Height(height) => {
            if !(height.is_finite() && height > 0.0) {
                return None;
            }
            ((span / height).ceil() as u32).clamp(MIN_ROWS, MAX_ROWS)
        }
    } as usize;
    // A range with no height (every bar flat on one price) is one row of no height.
    let count = if span > 0.0 { count } else { 1 };
    // Rows of a given height are that tall, the last one only as tall as what is left; only when
    // that would be more than MAX_ROWS are they stretched to fit.
    let height = match size {
        _ if span <= 0.0 => 0.0,
        RowSize::Height(wanted) if (span / wanted).ceil() <= f64::from(MAX_ROWS) => wanted,
        _ => span / count as f64,
    };
    let mut rows: Vec<Row> = (0..count)
        .map(|index| Row {
            low: low + height * index as f64,
            high: if index + 1 == count {
                high
            } else {
                low + height * (index + 1) as f64
            },
            up: 0.0,
            down: 0.0,
        })
        .collect();
    let row_of = |price: f64| -> usize {
        if height > 0.0 {
            (((price - low) / height) as usize).min(count - 1)
        } else {
            0
        }
    };
    for bar in slice.iter().filter(usable) {
        let (bottom, top) = (bar.low.min(bar.high), bar.high.max(bar.low));
        let up = bar.close > bar.open;
        let put = |rows: &mut [Row], row: usize, volume: f64| {
            if up {
                rows[row].up += volume;
            } else {
                rows[row].down += volume;
            }
        };
        if top - bottom <= 0.0 || height <= 0.0 {
            put(&mut rows, row_of(bottom), bar.volume);
            continue;
        }
        let (from, to) = (row_of(bottom), row_of(top));
        for row in from..=to {
            // The part of the bar's range inside this row.
            let overlap = (rows[row].high.min(top) - rows[row].low.max(bottom)).max(0.0);
            put(&mut rows, row, bar.volume * overlap / (top - bottom));
        }
    }
    let total: f64 = rows.iter().map(Row::total).sum();
    if !(total.is_finite() && total > 0.0) {
        return None;
    }
    let middle = (low + high) / 2.0;
    let poc = (0..count)
        .max_by(|a, b| {
            rows[*a]
                .total()
                .total_cmp(&rows[*b].total())
                // The nearer the middle wins a tie, so it compares greater; then the lower row.
                .then_with(|| {
                    (rows[*b].middle() - middle)
                        .abs()
                        .total_cmp(&(rows[*a].middle() - middle).abs())
                })
                .then_with(|| b.cmp(a))
        })
        .unwrap_or(0);
    let target = total * value_area.clamp(0.0, 1.0);
    let (mut bottom, mut top) = (poc, poc);
    let mut held = rows[poc].total();
    while held < target && (bottom > 0 || top + 1 < count) {
        let above: f64 = (top + 1..count.min(top + 3)).map(|i| rows[i].total()).sum();
        let below: f64 = (bottom.saturating_sub(2)..bottom)
            .map(|i| rows[i].total())
            .sum();
        let has_above = top + 1 < count;
        let has_below = bottom > 0;
        if has_above && (!has_below || above >= below) {
            let added = count.min(top + 3) - (top + 1);
            held += above;
            top += added;
        } else {
            let added = bottom - bottom.saturating_sub(2);
            held += below;
            bottom -= added;
        }
    }
    Some(Profile {
        rows,
        poc,
        value_area: (bottom, top),
        total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(open: f64, high: f64, low: f64, close: f64, volume: f64) -> Kline {
        Kline {
            open_time: 0,
            close_time: 0,
            open,
            high,
            low,
            close,
            volume,
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    // MARK: VWAP

    #[test]
    fn a_vwap_is_the_volume_weighted_mean_from_the_anchor() {
        // Close as the source. Bars: price 10 vol 1, price 20 vol 3, price 30 vol 6.
        let bars = [
            bar(0.0, 0.0, 0.0, 10.0, 1.0),
            bar(0.0, 0.0, 0.0, 20.0, 3.0),
            bar(0.0, 0.0, 0.0, 30.0, 6.0),
        ];
        let points = anchored_vwap(&bars, 0, PriceSource::Close);
        assert_eq!(points.len(), 3);
        assert!(close(points[0].vwap, 10.0));
        // (10 + 60) / 4 = 17.5 and (10 + 60 + 180) / 10 = 25.
        assert!(close(points[1].vwap, 17.5));
        assert!(close(points[2].vwap, 25.0));
        // Anchored one bar on, the first bar is not in it: 20 (3), then (60 + 180) / 9.
        let later = anchored_vwap(&bars, 1, PriceSource::Close);
        assert_eq!(later[0].index, 1);
        assert!(close(later[0].vwap, 20.0));
        assert!(close(later[1].vwap, 240.0 / 9.0));
    }

    #[test]
    fn the_bands_are_the_volume_weighted_standard_deviation() {
        // Two bars at 10 and 20 with equal volume: the mean is 15 and each is 5 from it, so the
        // deviation of the pair is exactly 5. One bar alone has none.
        let bars = [bar(0.0, 0.0, 0.0, 10.0, 2.0), bar(0.0, 0.0, 0.0, 20.0, 2.0)];
        let points = anchored_vwap(&bars, 0, PriceSource::Close);
        assert!(close(points[0].deviation, 0.0));
        assert!(close(points[1].vwap, 15.0));
        assert!(close(points[1].deviation, 5.0));
        // Weights matter: volumes 1 and 3 at 10 and 20: mean 17.5, variance
        // (1 x 7.5^2 + 3 x 2.5^2) / 4 = 18.75.
        let skewed = [bar(0.0, 0.0, 0.0, 10.0, 1.0), bar(0.0, 0.0, 0.0, 20.0, 3.0)];
        let skewed = anchored_vwap(&skewed, 0, PriceSource::Close);
        assert!(close(skewed[1].deviation, 18.75f64.sqrt()));
    }

    #[test]
    fn the_source_is_the_price_that_is_averaged() {
        let bars = [bar(10.0, 14.0, 6.0, 12.0, 1.0)];
        let value = |source| anchored_vwap(&bars, 0, source)[0].vwap;
        assert!(close(value(PriceSource::Close), 12.0));
        assert!(close(value(PriceSource::Open), 10.0));
        assert!(close(value(PriceSource::Hl2), 10.0));
        assert!(close(value(PriceSource::Hlc3), 32.0 / 3.0));
        assert!(close(value(PriceSource::Ohlc4), 42.0 / 4.0));
    }

    #[test]
    fn bars_that_traded_nothing_before_the_first_volume_have_no_vwap() {
        let bars = [
            bar(0.0, 0.0, 0.0, 10.0, 0.0),
            bar(0.0, 0.0, 0.0, 12.0, 0.0),
            bar(0.0, 0.0, 0.0, 14.0, 2.0),
            bar(0.0, 0.0, 0.0, 16.0, 0.0),
        ];
        let points = anchored_vwap(&bars, 0, PriceSource::Close);
        // From the third bar on, and a bar of no volume afterwards just repeats the average.
        assert_eq!(points.iter().map(|p| p.index).collect::<Vec<_>>(), [2, 3]);
        assert!(close(points[0].vwap, 14.0) && close(points[1].vwap, 14.0));
    }

    #[test]
    fn a_market_with_no_volume_has_no_vwap_and_bad_bars_are_skipped() {
        let silent = [bar(0.0, 0.0, 0.0, 10.0, 0.0), bar(0.0, 0.0, 0.0, 11.0, 0.0)];
        assert!(anchored_vwap(&silent, 0, PriceSource::Close).is_empty());
        assert!(anchored_vwap(&silent, 9, PriceSource::Close).is_empty());
        assert!(anchored_vwap(&[], 0, PriceSource::Close).is_empty());
        let broken = [
            bar(0.0, 0.0, 0.0, f64::NAN, 1.0),
            bar(0.0, 0.0, 0.0, 10.0, 1.0),
            bar(0.0, 0.0, 0.0, 20.0, f64::INFINITY),
            bar(0.0, 0.0, 0.0, 30.0, 1.0),
        ];
        let points = anchored_vwap(&broken, 0, PriceSource::Close);
        assert_eq!(points.len(), 2);
        assert!(close(points[1].vwap, 20.0), "(10 + 30) / 2");
    }

    #[test]
    fn a_vwap_of_one_price_has_no_deviation_not_nan() {
        let flat: Vec<Kline> = (0..50).map(|_| bar(0.0, 0.0, 0.0, 0.1, 0.3)).collect();
        let points = anchored_vwap(&flat, 0, PriceSource::Close);
        assert!(
            points
                .iter()
                .all(|p| p.deviation.is_finite() && p.deviation < 1e-6)
        );
    }

    // MARK: Profile

    #[test]
    fn each_bars_volume_is_spread_over_its_range_by_overlap() {
        // One bar from 10 to 14 with volume 8, up. Four rows of height 1: 2 volume each.
        let bars = [bar(11.0, 14.0, 10.0, 13.0, 8.0)];
        let profile = profile(&bars, 0, 0, RowSize::Count(4), 0.7).unwrap();
        assert_eq!(profile.rows.len(), 4);
        for row in &profile.rows {
            assert!(close(row.up, 2.0) && close(row.down, 0.0), "{row:?}");
        }
        assert!(close(profile.total, 8.0));
        assert!(close(profile.rows[0].low, 10.0) && close(profile.rows[3].high, 14.0));
    }

    #[test]
    fn a_bar_is_up_volume_only_when_it_closed_above_its_open() {
        let bars = [
            bar(10.0, 12.0, 10.0, 12.0, 2.0), // up
            bar(12.0, 12.0, 10.0, 10.0, 2.0), // down
            bar(11.0, 12.0, 10.0, 11.0, 2.0), // close = open: down
        ];
        let profile = profile(&bars, 0, 2, RowSize::Count(1), 0.7).unwrap();
        assert!(close(profile.rows[0].up, 2.0));
        assert!(close(profile.rows[0].down, 4.0));
    }

    #[test]
    fn a_row_gets_the_share_of_a_bar_its_prices_overlap() {
        // A bar 10 to 12 (volume 6) over rows covering 10 to 14 in two rows [10, 12) and [12, 14]:
        // all of it in the first. And a bar 11 to 13 (volume 4): half in each.
        let bars = [
            bar(10.0, 12.0, 10.0, 11.0, 6.0),
            bar(11.0, 13.0, 11.0, 12.0, 4.0),
            bar(13.0, 14.0, 13.0, 13.5, 0.0001),
            bar(10.0, 10.0, 10.0, 10.0, 0.0001),
        ];
        let profile = profile(&bars, 0, 3, RowSize::Count(2), 0.7).unwrap();
        assert!(
            close(profile.rows[0].total(), 6.0 + 2.0 + 0.0001),
            "{:?}",
            profile.rows
        );
        assert!(
            close(profile.rows[1].total(), 2.0 + 0.0001),
            "{:?}",
            profile.rows
        );
    }

    #[test]
    fn a_bar_with_no_range_puts_its_volume_in_one_row() {
        let bars = [
            bar(10.0, 20.0, 10.0, 20.0, 1.0),
            bar(15.0, 15.0, 15.0, 15.0, 5.0),
        ];
        let profile = profile(&bars, 0, 1, RowSize::Count(2), 0.7).unwrap();
        // 15 is the bottom of the upper row.
        assert!(close(profile.rows[1].total(), 5.5));
        assert!(close(profile.rows[0].total(), 0.5));
    }

    #[test]
    fn the_point_of_control_is_the_row_of_most_volume() {
        // Volume piles up at 12: three flat bars there, one each at 10 and 14.
        let bars = [
            bar(10.0, 10.0, 10.0, 10.0, 1.0),
            bar(12.0, 12.0, 12.0, 12.0, 3.0),
            bar(12.0, 12.0, 12.0, 12.0, 3.0),
            bar(14.0, 14.0, 14.0, 14.0, 1.0),
        ];
        let profile = profile(&bars, 0, 3, RowSize::Count(4), 0.7).unwrap();
        // Rows [10, 11), [11, 12), [12, 13), [13, 14].
        assert_eq!(profile.poc, 2);
        assert!(close(profile.poc_price(), 12.5));
        assert!(close(profile.peak(), 6.0));
    }

    #[test]
    fn two_equal_rows_are_told_apart_by_the_middle_of_the_range() {
        // Equal volume at 10 and at 20 and some at 15: the rows at the ends tie; the one nearer the
        // middle (neither: equal) then the lower wins.
        let bars = [
            bar(10.0, 10.0, 10.0, 10.0, 5.0),
            bar(20.0, 20.0, 20.0, 20.0, 5.0),
        ];
        let tied = profile(&bars, 0, 1, RowSize::Count(2), 0.7).unwrap();
        assert_eq!(tied.poc, 0);
        // With one end nearer the middle of a wider range, that row wins.
        let wide = [
            bar(10.0, 10.0, 10.0, 10.0, 5.0),
            bar(14.0, 14.0, 14.0, 14.0, 5.0),
            bar(30.0, 30.0, 30.0, 30.0, 1.0),
        ];
        let wide = profile(&wide, 0, 2, RowSize::Count(10), 0.7).unwrap();
        // Range 10..30, middle 20: the row at 14 is nearer than the one at 10.
        assert!(close(wide.poc_price(), wide.rows[wide.poc].middle()));
        assert!(wide.rows[wide.poc].low <= 14.0 && 14.0 <= wide.rows[wide.poc].high);
    }

    #[test]
    fn the_value_area_grows_from_the_poc_by_the_heavier_pair_of_rows() {
        // Ten rows 0..10 holding volumes 1, 2, 4, 8, 16, 9, 5, 3, 2, 1 (total 51): the point of
        // control is row 4 (16). 70 % of 51 is 35.7.
        // Above it: rows 5 and 6 hold 14; below: rows 3 and 2 hold 12. Above is heavier:
        // 16 + 14 = 30 (rows 4 to 6). Next above: rows 7 and 8 hold 5; below 12: below wins:
        // 30 + 12 = 42 >= 35.7 (rows 2 to 6).
        let volumes = [1.0, 2.0, 4.0, 8.0, 16.0, 9.0, 5.0, 3.0, 2.0, 1.0];
        let mut bars: Vec<Kline> = volumes
            .iter()
            .enumerate()
            .map(|(i, v)| {
                bar(
                    i as f64 + 0.5,
                    i as f64 + 0.5,
                    i as f64 + 0.5,
                    i as f64 + 0.5,
                    *v,
                )
            })
            .collect();
        // Stretch the range to exactly 0..10 so the rows are one price each.
        bars[0].low = 0.0;
        bars[0].high = 0.0;
        bars[9].low = 10.0;
        bars[9].high = 10.0;
        let profile = profile(&bars, 0, 9, RowSize::Count(10), 0.7).unwrap();
        assert!(close(profile.total, 51.0));
        assert_eq!(profile.poc, 4);
        assert_eq!(profile.value_area, (2, 6));
        assert!(close(profile.value_area_low(), 2.0));
        assert!(close(profile.value_area_high(), 7.0));
    }

    #[test]
    fn the_value_area_is_not_just_the_middle_of_the_range() {
        // Nearly all the volume at the very bottom: the area hugs it, however much price there is.
        let mut bars = vec![bar(1.0, 1.0, 1.0, 1.0, 100.0)];
        bars.push(bar(50.0, 50.0, 50.0, 50.0, 1.0));
        let profile = profile(&bars, 0, 1, RowSize::Count(10), 0.7).unwrap();
        assert_eq!(profile.poc, 0);
        assert_eq!(profile.value_area.0, 0);
        assert!(profile.value_area.1 <= 1, "{:?}", profile.value_area);
    }

    #[test]
    fn a_value_area_of_everything_spans_every_row_and_of_nothing_is_the_poc() {
        let volumes = [1.0, 3.0, 2.0];
        let bars: Vec<Kline> = volumes
            .iter()
            .enumerate()
            .map(|(i, v)| bar(i as f64, i as f64, i as f64, i as f64, *v))
            .collect();
        let all = profile(&bars, 0, 2, RowSize::Count(3), 1.0).unwrap();
        assert_eq!(all.value_area, (0, 2));
        let none = profile(&bars, 0, 2, RowSize::Count(3), 0.0).unwrap();
        assert_eq!(none.value_area, (none.poc, none.poc));
        // Out of range fractions are clamped, never panic.
        assert_eq!(
            profile(&bars, 0, 2, RowSize::Count(3), 7.0)
                .unwrap()
                .value_area,
            (0, 2)
        );
        assert_eq!(
            profile(&bars, 0, 2, RowSize::Count(3), -1.0)
                .unwrap()
                .value_area,
            (none.poc, none.poc)
        );
    }

    #[test]
    fn rows_can_be_sized_by_their_height() {
        let bars = [bar(10.0, 20.0, 10.0, 15.0, 10.0)];
        // Rows of height 2.5 over 10: four rows.
        let tall = profile(&bars, 0, 0, RowSize::Height(2.5), 0.7).unwrap();
        assert_eq!(tall.rows.len(), 4);
        // A height that does not divide the range adds a row so all of it is shown.
        let odd = profile(&bars, 0, 0, RowSize::Height(3.0), 0.7).unwrap();
        assert_eq!(odd.rows.len(), 4);
        assert!(close(odd.rows.last().unwrap().high, 20.0));
        // Silly sizes are brought into range.
        assert_eq!(
            profile(&bars, 0, 0, RowSize::Count(0), 0.7)
                .unwrap()
                .rows
                .len(),
            1
        );
        assert_eq!(
            profile(&bars, 0, 0, RowSize::Count(1_000_000), 0.7)
                .unwrap()
                .rows
                .len(),
            MAX_ROWS as usize
        );
        assert!(profile(&bars, 0, 0, RowSize::Height(0.0), 0.7).is_none());
        assert!(profile(&bars, 0, 0, RowSize::Height(f64::NAN), 0.7).is_none());
        assert_eq!(
            profile(&bars, 0, 0, RowSize::Height(1e-12), 0.7)
                .unwrap()
                .rows
                .len(),
            MAX_ROWS as usize
        );
    }

    #[test]
    fn the_range_is_the_bars_asked_for_and_nothing_outside_it() {
        let bars = [
            bar(100.0, 200.0, 100.0, 150.0, 50.0),
            bar(10.0, 12.0, 10.0, 11.0, 2.0),
            bar(11.0, 13.0, 11.0, 12.0, 2.0),
            bar(500.0, 900.0, 500.0, 600.0, 50.0),
        ];
        let profile = profile(&bars, 1, 2, RowSize::Count(3), 0.7).unwrap();
        assert!(close(profile.rows[0].low, 10.0) && close(profile.rows[2].high, 13.0));
        assert!(close(profile.total, 4.0));
        // `last` past the data is the last bar.
        let to_end = super::profile(&bars, 2, 99, RowSize::Count(3), 0.7).unwrap();
        assert!(to_end.rows.last().unwrap().high >= 900.0);
    }

    #[test]
    fn nothing_to_profile_is_none() {
        let silent = [bar(1.0, 2.0, 1.0, 1.5, 0.0)];
        assert!(profile(&silent, 0, 0, RowSize::Count(4), 0.7).is_none());
        assert!(profile(&[], 0, 0, RowSize::Count(4), 0.7).is_none());
        assert!(profile(&silent, 5, 9, RowSize::Count(4), 0.7).is_none());
        let backwards = [bar(1.0, 2.0, 1.0, 1.5, 1.0)];
        assert!(profile(&backwards, 1, 0, RowSize::Count(4), 0.7).is_none());
        let broken = [bar(1.0, f64::NAN, 1.0, 1.5, 1.0)];
        assert!(profile(&broken, 0, 0, RowSize::Count(4), 0.7).is_none());
    }

    #[test]
    fn a_flat_market_is_one_row_of_no_height() {
        let bars = [bar(5.0, 5.0, 5.0, 5.0, 3.0), bar(5.0, 5.0, 5.0, 5.0, 4.0)];
        let profile = profile(&bars, 0, 1, RowSize::Count(24), 0.7).unwrap();
        assert_eq!(profile.rows.len(), 1);
        assert!(close(profile.total, 7.0));
        assert_eq!(profile.value_area, (0, 0));
        assert!(profile.poc_price().is_finite());
    }

    #[test]
    fn the_volume_in_the_rows_is_the_volume_of_the_bars() {
        // Whatever the rows, nothing is lost or made up.
        let bars: Vec<Kline> = (0..40)
            .map(|i| {
                let base = 100.0 + (i % 7) as f64 * 3.0;
                bar(
                    base,
                    base + 2.0 + (i % 3) as f64,
                    base - 1.0,
                    base + 1.0,
                    1.0 + i as f64,
                )
            })
            .collect();
        let expected: f64 = bars.iter().map(|b| b.volume).sum();
        for rows in [1, 2, 7, 24, 100, 500] {
            let profile = profile(&bars, 0, 39, RowSize::Count(rows), 0.7).unwrap();
            assert!((profile.total - expected).abs() < 1e-6, "{rows} rows");
            assert!(
                profile
                    .rows
                    .windows(2)
                    .all(|pair| close(pair[0].high, pair[1].low))
            );
            assert!(profile.value_area.0 <= profile.poc && profile.poc <= profile.value_area.1);
        }
    }
}
