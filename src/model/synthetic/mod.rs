//! Bars a chart constructs from the provider's candles: Heikin Ashi, Renko, Line Break, Kagi,
//! Point & Figure and Range.
//!
//! The provider's candles stay canonical and are never touched. A builder reads them, oldest
//! first, and returns a [`SyntheticSeries`] with its own indexing: one source candle may yield
//! several bars (a Renko run) or none (a quiet day), so a synthetic index is never a source
//! index. Each bar remembers the source candles it was made from.
//!
//! Construction is deterministic and a function of the source series alone: the same candles
//! always give the same bars, wherever the view starts. Builders read candles in order and keep
//! no hidden state beyond what they return, so a replay that feeds a prefix of the history gets
//! exactly the prefix of the bars it would have had at that point.
//!
//! Bars made from a candle that is still forming are not confirmed, and neither is the figure
//! that can still grow (the last Kagi segment or Point & Figure column, a Range bar short of its
//! range). The chart draws those apart from history and scripts see them as the realtime bar.
//!
//! Where TradingView's rule is not documented the choice made here is noted on the builder and
//! listed in `docs/chart-types.md`; none of these claim tick-level parity, because candles do not
//! say in what order prices were traded inside them.

mod heikin_ashi;
mod kagi;
mod line_break;
mod point_figure;
mod range;
mod renko;
pub mod ticker;

use super::Kline;
use super::chart_type::{ChartStyle, ChartType, ConstructionSource, SizeRule};

/// Open time and close time of a bar that can still change: far enough ahead that a script's
/// runtime treats it as the forming bar.
pub const UNCONFIRMED_CLOSE_TIME: i64 = i64::MAX / 4;

/// What a bar is drawn as, beyond its open, high, low and close.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BarShape {
    /// A candle or bar: body from open to close, wicks to the extremes.
    Ohlc,
    /// A Kagi segment from `open` to `close`. `thick` is its thickness at the end; when `switch`
    /// is set the segment changed thickness there (the previous shoulder or waist was broken), so
    /// it started as `!thick`.
    Kagi { thick: bool, switch: Option<f64> },
    /// A Point & Figure column of `boxes` boxes, Xs when `rising`, Os otherwise.
    Column { rising: bool, boxes: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SyntheticBar {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    /// Index of the first source candle that contributed.
    pub first_source: usize,
    /// Index of the last source candle that contributed (or the one that completed the bar).
    pub last_source: usize,
    /// Open time of the first contributing candle.
    pub open_time: i64,
    /// Close time of the last contributing candle; the unconfirmed marker for a bar that can
    /// still change.
    pub close_time: i64,
    pub confirmed: bool,
    pub shape: BarShape,
}

impl SyntheticBar {
    #[cfg(test)]
    pub fn is_up(&self) -> bool {
        match self.shape {
            BarShape::Column { rising, .. } => rising,
            _ => self.close >= self.open,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SyntheticSeries {
    pub chart_type: ChartType,
    pub bars: Vec<SyntheticBar>,
    /// The brick, reversal, box or range size the construction resolved, where it has one.
    pub size: Option<f64>,
    /// How many source candles were read.
    pub source_len: usize,
}

impl SyntheticSeries {
    /// The bars as candles, for scripts, indicators and the price scale: strictly increasing
    /// open times, so bars made from the same source candle stay distinct and ordered. A later
    /// bar of the same candle is nudged one millisecond per place; that stays inside the
    /// candle's own interval and is only an ordering key. Unconfirmed bars carry
    /// [`UNCONFIRMED_CLOSE_TIME`].
    pub fn klines(&self) -> Vec<Kline> {
        let mut previous: Option<i64> = None;
        self.bars
            .iter()
            .map(|bar| {
                let open_time = match previous {
                    Some(previous) if bar.open_time <= previous => previous + 1,
                    _ => bar.open_time,
                };
                previous = Some(open_time);
                Kline {
                    open_time,
                    close_time: if bar.confirmed {
                        bar.close_time.max(open_time)
                    } else {
                        UNCONFIRMED_CLOSE_TIME
                    },
                    open: bar.open,
                    high: bar.high,
                    low: bar.low,
                    close: bar.close,
                    volume: bar.volume,
                }
            })
            .collect()
    }
}

/// What a price-based chart type builds its bars from: the type and the settings that change the
/// bars it makes. Colours and visibility are not part of it, so restyling a chart never rebuilds
/// its bars (or the scripts running over them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Construction {
    HeikinAshi,
    Renko {
        size: SizeRule,
        source: ConstructionSource,
        wicks: bool,
    },
    Kagi {
        size: SizeRule,
        source: ConstructionSource,
    },
    LineBreak {
        lines: u8,
    },
    PointFigure {
        size: SizeRule,
        reversal: u8,
        source: ConstructionSource,
    },
    Range {
        size: SizeRule,
    },
}

impl Construction {
    /// What `style`'s selected type constructs, or `None` for a type that draws the provider's
    /// candles as they are.
    pub fn of(style: &ChartStyle) -> Option<Self> {
        Some(match style.chart_type {
            ChartType::HeikinAshi => Self::HeikinAshi,
            ChartType::Renko => Self::Renko {
                size: style.renko.size,
                source: style.renko.source,
                wicks: style.renko.wicks,
            },
            ChartType::Kagi => Self::Kagi {
                size: style.kagi.size,
                source: style.kagi.source,
            },
            ChartType::LineBreak => Self::LineBreak {
                lines: style.line_break.lines.max(1),
            },
            ChartType::PointAndFigure => Self::PointFigure {
                size: style.point_figure.size,
                reversal: style.point_figure.reversal.max(1),
                source: style.point_figure.source,
            },
            ChartType::Range => Self::Range {
                size: style.range.size,
            },
            _ => return None,
        })
    }

    pub fn chart_type(self) -> ChartType {
        match self {
            Self::HeikinAshi => ChartType::HeikinAshi,
            Self::Renko { .. } => ChartType::Renko,
            Self::Kagi { .. } => ChartType::Kagi,
            Self::LineBreak { .. } => ChartType::LineBreak,
            Self::PointFigure { .. } => ChartType::PointAndFigure,
            Self::Range { .. } => ChartType::Range,
        }
    }
}

/// The bars `construction` makes from `source`.
pub fn build_with(construction: Construction, source: &[Kline], now_ms: i64) -> SyntheticSeries {
    let forming = forming_index(source, now_ms);
    let (bars, size) = match construction {
        Construction::HeikinAshi => (heikin_ashi::build(source, forming), None),
        Construction::Renko {
            size,
            source: price,
            wicks,
        } => {
            let size = size.resolve(source);
            (
                size.map(|size| renko::build(source, size, price, wicks, forming))
                    .unwrap_or_default(),
                size,
            )
        }
        Construction::LineBreak { lines } => (
            line_break::build(source, usize::from(lines.max(1)), forming),
            None,
        ),
        Construction::Kagi {
            size,
            source: price,
        } => {
            let size = size.resolve(source);
            (
                size.map(|size| kagi::build(source, size, price, forming))
                    .unwrap_or_default(),
                size,
            )
        }
        Construction::PointFigure {
            size,
            reversal,
            source: price,
        } => {
            let size = size.resolve(source);
            (
                size.map(|size| {
                    point_figure::build(source, size, u32::from(reversal.max(1)), price, forming)
                })
                .unwrap_or_default(),
                size,
            )
        }
        Construction::Range { size } => {
            let size = size.resolve(source);
            (
                size.map(|size| range::build(source, size, forming))
                    .unwrap_or_default(),
                size,
            )
        }
    };
    SyntheticSeries {
        chart_type: construction.chart_type(),
        bars,
        size,
        source_len: source.len(),
    }
}

/// The index of the candle still forming: the last one, when it has not closed yet.
fn forming_index(source: &[Kline], now_ms: i64) -> Option<usize> {
    let last = source.len().checked_sub(1)?;
    (source[last].close_time > now_ms).then_some(last)
}

/// The prices a candle is taken to have traded through, in order. Candles do not record the
/// order, so a rising candle is read as open, low, high, close and a falling one as open, high,
/// low, close, the usual convention. With the close source only the close counts.
fn price_path(bar: &Kline, source: ConstructionSource) -> ([f64; 4], usize) {
    match source {
        ConstructionSource::Close => ([bar.close, 0.0, 0.0, 0.0], 1),
        ConstructionSource::HighLow if bar.close >= bar.open => {
            ([bar.open, bar.low, bar.high, bar.close], 4)
        }
        ConstructionSource::HighLow => ([bar.open, bar.high, bar.low, bar.close], 4),
    }
}

/// What has piled up since a builder last emitted a bar: the candles' volume and extremes.
#[derive(Clone, Copy, Debug)]
struct Pending {
    first_source: usize,
    volume: f64,
    high: f64,
    low: f64,
}

impl Pending {
    fn start(index: usize, bar: &Kline) -> Self {
        Self {
            first_source: index,
            volume: bar.volume,
            high: bar.high,
            low: bar.low,
        }
    }

    fn absorb(&mut self, bar: &Kline) {
        self.volume += bar.volume;
        self.high = self.high.max(bar.high);
        self.low = self.low.min(bar.low);
    }
}

/// Relative slack so a price that lands on a threshold in decimal but a hair under it in binary
/// still counts as reaching it.
fn tolerance(size: f64) -> f64 {
    size * 1e-9
}

#[cfg(test)]
pub(crate) mod testing {
    use crate::model::Kline;

    pub const STEP: i64 = 3_600_000;

    pub fn candle(index: usize, open: f64, high: f64, low: f64, close: f64) -> Kline {
        Kline {
            open_time: index as i64 * STEP,
            close_time: (index as i64 + 1) * STEP - 1,
            open,
            high,
            low,
            close,
            volume: 10.0,
        }
    }

    /// Candles that open and close at the same level, `closes` apart one hour each.
    pub fn closes(values: &[f64]) -> Vec<Kline> {
        values
            .iter()
            .enumerate()
            .map(|(index, close)| candle(index, *close, *close, *close, *close))
            .collect()
    }

    /// A time after every candle of `source` closed.
    pub fn after(source: &[Kline]) -> i64 {
        source.last().map_or(0, |bar| bar.close_time + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style(chart_type: ChartType) -> ChartStyle {
        ChartStyle {
            chart_type,
            ..ChartStyle::default()
        }
    }

    #[test]
    fn standard_types_build_nothing() {
        for kind in ChartType::ALL
            .into_iter()
            .filter(|kind| !kind.is_synthetic())
        {
            assert!(Construction::of(&style(kind)).is_none(), "{kind:?}");
        }
        for kind in ChartType::ALL
            .into_iter()
            .filter(|kind| kind.is_synthetic())
        {
            let construction = Construction::of(&style(kind)).expect("constructs");
            assert_eq!(construction.chart_type(), kind);
        }
    }

    #[test]
    fn klines_keep_a_strict_order_when_a_candle_makes_several_bars() {
        let mut series = SyntheticSeries {
            chart_type: ChartType::Renko,
            bars: Vec::new(),
            size: Some(1.0),
            source_len: 1,
        };
        for index in 0..3 {
            series.bars.push(SyntheticBar {
                open: index as f64,
                high: index as f64 + 1.0,
                low: index as f64,
                close: index as f64 + 1.0,
                volume: 0.0,
                first_source: 0,
                last_source: 0,
                open_time: 5_000,
                close_time: 9_000,
                confirmed: index < 2,
                shape: BarShape::Ohlc,
            });
        }
        let klines = series.klines();
        let times: Vec<i64> = klines.iter().map(|bar| bar.open_time).collect();
        assert_eq!(times, vec![5_000, 5_001, 5_002]);
        assert_eq!(klines[0].close_time, 9_000);
        assert_eq!(klines[2].close_time, UNCONFIRMED_CLOSE_TIME);
    }

    /// `cargo test --release -- --ignored --nocapture construction_cost`: what rebuilding every
    /// constructed series costs on a long history, which is what each realtime tick pays.
    #[test]
    #[ignore = "a measurement, not a check"]
    fn construction_cost_on_a_long_history() {
        use std::time::Instant;
        let candles: Vec<Kline> = (0..5_000)
            .map(|index| {
                let base = 20_000.0
                    + (index as f64 * 0.013).sin() * 3_000.0
                    + (index as f64 * 0.31).sin() * 400.0;
                Kline {
                    open_time: index as i64 * 3_600_000,
                    close_time: (index as i64 + 1) * 3_600_000 - 1,
                    open: base,
                    high: base + 150.0,
                    low: base - 160.0,
                    close: base + 40.0,
                    volume: 10.0,
                }
            })
            .collect();
        for kind in ChartType::ALL
            .into_iter()
            .filter(|kind| kind.is_synthetic())
        {
            let construction = Construction::of(&style(kind)).unwrap();
            let started = Instant::now();
            let runs = 200;
            let mut bars = 0;
            for _ in 0..runs {
                let series = build_with(construction, &candles, i64::MAX / 2);
                bars = series.klines().len();
            }
            println!(
                "{:>16}: {:>6} bars from {} candles, {:?} per rebuild",
                kind.label(),
                bars,
                candles.len(),
                started.elapsed() / runs
            );
        }
    }

    #[test]
    fn the_order_flow_types_construct_nothing_so_scripts_and_indicators_read_the_real_candles() {
        use crate::model::chart_type::{ChartKind, ChartStyle, ChartType};
        for chart_type in [
            ChartType::VolumeFootprint,
            ChartType::TimePriceOpportunity,
            ChartType::SessionVolumeProfile,
        ] {
            let style = ChartStyle {
                chart_type,
                ..ChartStyle::default()
            };
            assert!(Construction::of(&style).is_none(), "{chart_type:?}");
            assert_eq!(
                chart_type.kind(),
                ChartKind::Standard,
                "chart.is_* stays standard"
            );
            assert!(!chart_type.is_synthetic());
        }
    }
}
