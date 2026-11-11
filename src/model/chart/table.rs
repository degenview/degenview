//! The data behind Table view: one row per bar the chart draws, with the values the chart's
//! visible indicators and scripts have on it.
//!
//! The rows are the chart's own bars (constructed ones for a price-based chart type, cut at the
//! replay position during a replay), so the table can never show a bar the chart does not, and
//! its numbers are formatted the way the chart prints them. Pure: the chart card builds the
//! feed, the view draws it.

use crate::format;
use crate::model::watchlist::quote::compact_number;
use crate::model::{Kline, PriceFormat};

/// How a column's numbers read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueKind {
    /// A price: written with the chart's price format.
    Price,
    /// A plain reading (RSI, an oscillator): two decimals.
    Plain,
    /// A trend flip: `1` bullish, `-1` bearish.
    Flip,
}

/// One indicator or script plot, a value per bar (or none).
#[derive(Clone, Debug, PartialEq)]
pub struct Extra {
    pub title: String,
    pub kind: ValueKind,
    /// One entry per bar of the feed, oldest first.
    pub values: Vec<Option<f64>>,
}

/// A table column.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Column {
    Time,
    Open,
    High,
    Low,
    Close,
    Volume,
    Extra(usize),
}

/// Which end the table starts from.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Order {
    #[default]
    NewestFirst,
    OldestFirst,
}

impl Order {
    pub fn flipped(self) -> Self {
        match self {
            Self::NewestFirst => Self::OldestFirst,
            Self::OldestFirst => Self::NewestFirst,
        }
    }
}

/// What the chart card hands the table each frame it is open.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableFeed {
    /// What the bars are ("Candles", "Renko"), for the header.
    pub bars_label: String,
    /// Whether the bars have an open, high and low of their own (a line chart's do not).
    pub ohlc: bool,
    pub volume: bool,
    /// The bars the chart draws, oldest first.
    pub bars: Vec<Kline>,
    pub extras: Vec<Extra>,
}

impl TableFeed {
    pub fn len(&self) -> usize {
        self.bars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bars.is_empty()
    }

    /// The columns, in reading order.
    pub fn columns(&self) -> Vec<Column> {
        let mut columns = vec![Column::Time];
        if self.ohlc {
            columns.extend([Column::Open, Column::High, Column::Low]);
        }
        columns.push(Column::Close);
        if self.volume {
            columns.push(Column::Volume);
        }
        columns.extend((0..self.extras.len()).map(Column::Extra));
        columns
    }

    pub fn title(&self, column: Column) -> String {
        match column {
            Column::Time => "Time".to_owned(),
            Column::Open => "Open".to_owned(),
            Column::High => "High".to_owned(),
            Column::Low => "Low".to_owned(),
            Column::Close => "Close".to_owned(),
            Column::Volume => "Volume".to_owned(),
            Column::Extra(index) => self
                .extras
                .get(index)
                .map(|extra| extra.title.clone())
                .unwrap_or_default(),
        }
    }

    /// The bar shown on row `row` (0 is the top row).
    pub fn bar_at(&self, row: usize, order: Order) -> Option<usize> {
        if row >= self.bars.len() {
            return None;
        }
        Some(match order {
            Order::NewestFirst => self.bars.len() - 1 - row,
            Order::OldestFirst => row,
        })
    }

    /// The text of one cell, written as the chart writes the same number.
    pub fn cell(&self, bar: usize, column: Column, price: PriceFormat) -> String {
        let Some(kline) = self.bars.get(bar) else {
            return String::new();
        };
        match column {
            Column::Time => format::timestamp(kline.open_time),
            Column::Open => price.format(kline.open),
            Column::High => price.format(kline.high),
            Column::Low => price.format(kline.low),
            Column::Close => price.format(kline.close),
            Column::Volume => compact_number(kline.volume),
            Column::Extra(index) => {
                let Some(extra) = self.extras.get(index) else {
                    return String::new();
                };
                match extra.values.get(bar).copied().flatten() {
                    None => "—".to_owned(),
                    Some(value) => match extra.kind {
                        ValueKind::Price => price.format(value),
                        ValueKind::Plain => format!("{value:.2}"),
                        ValueKind::Flip if value > 0.0 => "Bullish".to_owned(),
                        ValueKind::Flip => "Bearish".to_owned(),
                    },
                }
            }
        }
    }
}

/// A script's per-bar values, which end on the newest bar, laid onto `bars` bars (oldest
/// first); the bars before the script's first value are empty.
pub fn align_right(values: &[Option<f64>], bars: usize) -> Vec<Option<f64>> {
    let mut aligned = vec![None; bars];
    let offset = bars as isize - values.len() as isize;
    for (index, value) in values.iter().enumerate() {
        let slot = index as isize + offset;
        if slot >= 0 && (slot as usize) < bars {
            aligned[slot as usize] = *value;
        }
    }
    aligned
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(open_time: i64, open: f64, close: f64) -> Kline {
        Kline {
            open_time,
            close_time: open_time + 59_999,
            open,
            high: open.max(close) + 1.0,
            low: open.min(close) - 1.0,
            close,
            volume: 1_500.0,
        }
    }

    fn feed() -> TableFeed {
        TableFeed {
            bars_label: "Candles".to_owned(),
            ohlc: true,
            volume: true,
            bars: vec![
                bar(0, 100.0, 101.0),
                bar(60_000, 101.0, 99.5),
                bar(120_000, 99.5, 102.0),
            ],
            extras: vec![
                Extra {
                    title: "EMA 21".to_owned(),
                    kind: ValueKind::Price,
                    values: vec![None, Some(100.0), Some(100.5)],
                },
                Extra {
                    title: "RSI".to_owned(),
                    kind: ValueKind::Plain,
                    values: vec![None, None, Some(55.456)],
                },
                Extra {
                    title: "Supertrend".to_owned(),
                    kind: ValueKind::Flip,
                    values: vec![None, Some(-1.0), Some(1.0)],
                },
            ],
        }
    }

    #[test]
    fn newest_bars_come_first_by_default() {
        let feed = feed();
        assert_eq!(feed.bar_at(0, Order::NewestFirst), Some(2));
        assert_eq!(feed.bar_at(2, Order::NewestFirst), Some(0));
        assert_eq!(feed.bar_at(0, Order::OldestFirst), Some(0));
        assert_eq!(feed.bar_at(3, Order::NewestFirst), None);
        assert_eq!(Order::default(), Order::NewestFirst);
        assert_eq!(Order::NewestFirst.flipped(), Order::OldestFirst);
    }

    #[test]
    fn columns_follow_the_data_the_chart_has() {
        let mut feed = feed();
        assert_eq!(
            feed.columns(),
            [
                Column::Time,
                Column::Open,
                Column::High,
                Column::Low,
                Column::Close,
                Column::Volume,
                Column::Extra(0),
                Column::Extra(1),
                Column::Extra(2),
            ]
        );
        // A line chart has a value, not an open, high and low; no volume source has no volume.
        feed.ohlc = false;
        feed.volume = false;
        feed.extras.clear();
        assert_eq!(feed.columns(), [Column::Time, Column::Close]);
    }

    #[test]
    fn cells_are_written_the_way_the_chart_writes_them() {
        let feed = feed();
        let price = PriceFormat::AUTO;
        assert_eq!(feed.cell(2, Column::Time, price), "1970-01-01 00:02");
        assert_eq!(feed.cell(2, Column::Close, price), "102.00");
        assert_eq!(feed.cell(2, Column::Volume, price), "1.5K");
        assert_eq!(feed.cell(2, Column::Extra(0), price), "100.50");
        assert_eq!(feed.cell(2, Column::Extra(1), price), "55.46");
        assert_eq!(feed.cell(2, Column::Extra(2), price), "Bullish");
        assert_eq!(feed.cell(1, Column::Extra(2), price), "Bearish");
        assert_eq!(feed.cell(0, Column::Extra(0), price), "—");
        assert_eq!(feed.cell(9, Column::Close, price), "");
    }

    #[test]
    fn a_pinned_price_format_is_used() {
        let feed = feed();
        let pinned = PriceFormat {
            decimals: Some(4),
            ..PriceFormat::AUTO
        };
        assert_eq!(feed.cell(2, Column::Close, pinned), "102.0000");
    }

    #[test]
    fn titles_name_the_columns() {
        let feed = feed();
        assert_eq!(feed.title(Column::Close), "Close");
        assert_eq!(feed.title(Column::Extra(1)), "RSI");
        assert_eq!(feed.title(Column::Extra(9)), "");
    }

    #[test]
    fn a_script_series_ends_on_the_newest_bar() {
        let values = [Some(1.0), Some(2.0)];
        assert_eq!(align_right(&values, 4), [None, None, Some(1.0), Some(2.0)]);
        // A longer series is cut at its oldest end.
        let long = [Some(1.0), Some(2.0), Some(3.0)];
        assert_eq!(align_right(&long, 2), [Some(2.0), Some(3.0)]);
        assert_eq!(align_right(&[], 2), [None, None]);
    }
}
