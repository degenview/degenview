//! A stock's higher timeframes follow its session, as TradingView cuts them (`NASDAQ:AAPL`,
//! 2026-11-25): a 4h candle is 09:30 to 13:30 and 13:30 to 16:00 New York, and a day, week or
//! month closes at the 16:00 of its last session, not at midnight. A request closes the chart bar
//! that ends where its candle does, so `gaps_on` shows the bucket's bars.

use crate::model::Kline;
use crate::model::market::bucket::Grid;
use crate::pine::compiler::NoLibraries;
use crate::pine::compiler::program::SymbolInfo;
use crate::pine::runtime::execution::ExecutionController;
use crate::pine::runtime::support::{SecurityDataProvider, SecurityKey};
use crate::pine::syntax::token::Limits;
use std::collections::HashMap;

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;
/// 24 Nov 2026 00:00 UTC; standard time, so the session is 14:30 to 21:00 UTC.
const TUESDAY: i64 = 1_795_564_800_000;

fn open(day: i64) -> i64 {
    TUESDAY + day * DAY + 14 * HOUR + 30 * MINUTE
}

fn candle(open_time: i64, length: i64) -> Kline {
    Kline {
        open_time,
        close_time: open_time + length - 1,
        open: 1.0,
        high: 1.0,
        low: 1.0,
        close: 1.0,
        volume: 1.0,
    }
}

/// The candles of `length` of `days` sessions of 09:30 to 16:00, the last one of a session cut at
/// the close.
fn sessions(days: i64, length: i64) -> Vec<Kline> {
    let mut candles = Vec::new();
    for day in 0..days {
        let mut start = open(day);
        let close = open(day) + 390 * MINUTE;
        while start < close {
            let end = (start + length).min(close);
            candles.push(candle(start, end - start));
            start = end;
        }
    }
    candles
}

fn stock() -> SymbolInfo {
    SymbolInfo {
        ticker: "AAPL".to_owned(),
        ticker_id: "NASDAQ:AAPL".to_owned(),
        kind: "stock".to_owned(),
        grid: Grid::UsEquities,
        ..SymbolInfo::default()
    }
}

/// What `source` plots on each of `bars` of `seconds`, read after the sessions are over.
fn plotted(
    symbol: SymbolInfo,
    source: &str,
    bars: &[Kline],
    seconds: f64,
    provider: Option<Box<dyn SecurityDataProvider>>,
) -> Vec<Option<f64>> {
    let mut chart = ExecutionController::new(
        source,
        HashMap::new(),
        symbol,
        None,
        Limits::default(),
        &NoLibraries,
        provider,
        None,
    )
    .expect("compiles")
    .with_bar_seconds(seconds);
    let now = bars.last().expect("bars").close_time + 12 * HOUR;
    let update = chart.update(bars, None, now);
    assert!(update.error.is_none(), "{:?}", update.error);
    update.output.plots[0].values.clone()
}

fn indices(values: &[Option<f64>]) -> Vec<usize> {
    values
        .iter()
        .enumerate()
        .filter_map(|(index, value)| value.map(|_| index))
        .collect()
}

#[test]
fn a_four_hour_candle_of_a_stock_closes_at_half_past_one_and_at_the_close() {
    let bars = sessions(1, 30 * MINUTE);
    assert_eq!(bars.len(), 13);
    let source = "//@version=6\nindicator(\"T\")\n\
        plot(request.security(syminfo.tickerid, \"240\", time, gaps = barmerge.gaps_on))";
    let values = plotted(stock(), source, &bars, 1_800.0, None);
    // The 13:00 bar ends at 13:30 and the 15:30 one at 16:00; each carries its candle's open.
    assert_eq!(indices(&values), [7, 12]);
    assert_eq!(values[7], Some(open(0) as f64));
    assert_eq!(values[12], Some((open(0) + 4 * HOUR) as f64));
}

#[test]
fn the_same_bars_on_the_utc_grid_close_where_midnight_cuts_them() {
    // Not a stock: the 4h candles are 12:00 to 16:00 and 16:00 to 20:00 UTC, so the bars of the
    // 09:30 session close them at 15:30 UTC (11:00 New York) and at 19:30 UTC.
    let bars = sessions(1, 30 * MINUTE);
    let source = "//@version=6\nindicator(\"T\")\n\
        plot(request.security(syminfo.tickerid, \"240\", time, gaps = barmerge.gaps_on))";
    let crypto = SymbolInfo {
        grid: Grid::Utc,
        ..stock()
    };
    let values = plotted(crypto, source, &bars, 1_800.0, None);
    assert_eq!(indices(&values), [2, 10]);
}

#[test]
fn a_stocks_day_closes_with_its_last_bar_and_not_at_midnight() {
    // Hourly bars of two sessions: 09:30, 10:30 ... 15:30 (half an hour long).
    let bars = sessions(2, HOUR);
    assert_eq!(bars.len(), 14);
    let source = "//@version=6\nindicator(\"T\")\n\
        plot(request.security(syminfo.tickerid, \"D\", time, gaps = barmerge.gaps_on))";
    let values = plotted(stock(), source, &bars, 3_600.0, None);
    // The session's last bar carries the day, the 16:00 TradingView closes it at.
    assert_eq!(indices(&values), [6, 13]);
}

/// Candles of `OTHER`, a stock: 4h candles cut on the session.
struct Other(Vec<Kline>);

impl SecurityDataProvider for Other {
    fn candles(&self, key: &SecurityKey) -> Option<Vec<Kline>> {
        (key.interval == 14_400.0).then(|| self.0.clone())
    }

    fn grid(&self, _: &str) -> Grid {
        Grid::UsEquities
    }
}

#[test]
fn another_symbols_series_is_cut_on_its_own_grid_not_the_charts() {
    // The chart is a crypto market on 30 minute bars; OTHER's 4h candles are 09:30 to 13:30 and
    // 13:30 to 16:00 New York, so they end at 18:30 and 21:00 UTC.
    let candles = vec![
        candle(open(0), 4 * HOUR),
        candle(open(0) + 4 * HOUR, 150 * MINUTE),
    ];
    let bars = sessions(1, 30 * MINUTE);
    let source = "//@version=6\nindicator(\"T\")\n\
        plot(request.security(\"OTHER\", \"240\", time, gaps = barmerge.gaps_on))";
    let chart = SymbolInfo {
        grid: Grid::Utc,
        ..stock()
    };
    let values = plotted(
        chart,
        source,
        &bars,
        1_800.0,
        Some(Box::new(Other(candles))),
    );
    assert_eq!(indices(&values), [7, 12]);
    assert_eq!(values[7], Some(open(0) as f64));
}
