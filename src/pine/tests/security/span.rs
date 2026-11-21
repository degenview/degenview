//! `request.security` for a timeframe the chart's bars do not fold into (a 45 minute candle on 30
//! minute bars), `calc_bars_count`, and a chart that opens inside a higher-timeframe candle.

use std::collections::HashMap;

use super::lookahead::{HOUR, hourly, hourly_bar, plot_values, session_with};
use crate::model::Kline;
use crate::pine::compiler::{self, NoLibraries};
use crate::pine::runtime::session::RunError;
use crate::pine::runtime::support::{SecurityDataProvider, SecurityKey};
use crate::pine::tests::bench::symbol;

/// Canned candles by key, for a provider that serves exactly what a test asks for.
#[derive(Clone, Default)]
struct CannedData(HashMap<SecurityKey, Vec<Kline>>);

impl CannedData {
    fn with(
        mut self,
        symbol: &str,
        interval: f64,
        recent_bars: usize,
        candles: Vec<Kline>,
    ) -> Self {
        let key = SecurityKey {
            symbol: symbol.to_owned(),
            interval,
            recent_bars,
        };
        self.0.insert(key, candles);
        self
    }
}

impl SecurityDataProvider for CannedData {
    fn candles(&self, key: &SecurityKey) -> Option<Vec<Kline>> {
        self.0.get(key).cloned()
    }
}

fn candle(open_time: i64, length: i64, open: f64, close: f64, volume: f64) -> Kline {
    Kline {
        open_time,
        close_time: open_time + length - 1,
        open,
        high: open.max(close),
        low: open.min(close),
        close,
        volume,
    }
}

fn plotted(source: &str, chart: &[Kline], data: CannedData) -> Vec<Option<f64>> {
    let source = format!("//@version=6\nindicator(\"T\")\nplot({source}, \"x\")");
    let mut session = session_with(&source, Some(Box::new(data)));
    session.evaluate(chart).expect("script runs");
    plot_values(&session, "x")
}

const MINUTE: i64 = 60_000;

#[test]
fn a_candle_the_chart_bars_straddle_comes_from_the_data_source() {
    // 30 minute bars from 00:00; the 45 minute candles open at 00:00, 00:45, 01:30, 02:15, 03:00.
    let chart: Vec<Kline> = (0..6)
        .map(|index| candle(index * 30 * MINUTE, 30 * MINUTE, 1.0, 1.0, 1.0))
        .collect();
    let candles = [10.0, 20.0, 30.0, 40.0, 50.0]
        .iter()
        .enumerate()
        .map(|(index, close)| candle(index as i64 * 45 * MINUTE, 45 * MINUTE, *close, *close, 1.0))
        .collect();
    let data = CannedData::default().with(&symbol().ticker_id, 2_700.0, 0, candles);
    let values = plotted(
        "request.security(syminfo.tickerid, \"45\", close)",
        &chart,
        data,
    );
    // Each candle is read on the chart bar that closes at or after its end.
    assert_eq!(
        values,
        [
            None,
            Some(10.0),
            Some(20.0),
            Some(20.0),
            Some(30.0),
            Some(40.0)
        ]
    );
}

#[test]
fn a_candle_the_chart_bars_straddle_and_the_source_lacks_is_an_error() {
    let chart: Vec<Kline> = (0..4)
        .map(|index| candle(index * 30 * MINUTE, 30 * MINUTE, 1.0, 1.0, 1.0))
        .collect();
    let source =
        "//@version=6\nindicator(\"T\")\nplot(request.security(syminfo.tickerid, \"45\", close))";
    let mut session = session_with(source, Some(Box::new(CannedData::default())));
    let Err(RunError::Script(diagnostic)) = session.evaluate(&chart) else {
        panic!("a script error is expected: no 45 minute candles");
    };
    assert_eq!(diagnostic.code, "PINE4021");
    assert!(
        diagnostic.message.contains("45") && diagnostic.message.contains("30"),
        "{}",
        diagnostic.message
    );
}

#[test]
fn bars_that_nest_in_the_candle_are_still_folded() {
    // 90 minutes on 30 minute bars nests: no provider candles are asked for.
    let chart: Vec<Kline> = (0..6)
        .map(|index| {
            candle(
                index * 30 * MINUTE,
                30 * MINUTE,
                1.0,
                1.0 + index as f64,
                1.0,
            )
        })
        .collect();
    let values = plotted(
        "request.security(syminfo.tickerid, \"90\", close)",
        &chart,
        CannedData::default(),
    );
    assert_eq!(
        values[2],
        Some(3.0),
        "the first 90 minute candle ends on bar 2"
    );
    assert_eq!(values[5], Some(6.0));
}

#[test]
fn calc_bars_count_evaluates_only_the_newest_candles() {
    let chart = hourly(12);
    let values = plotted(
        "request.security(syminfo.tickerid, \"240\", ta.cum(1), calc_bars_count = 2)",
        &chart,
        CannedData::default(),
    );
    // The newest two 4h candles are hours 4-7 and 8-11; the one before them is not evaluated.
    assert_eq!(values[3], None);
    assert_eq!(values[6], None, "nothing completed in the window yet");
    assert_eq!(values[7], Some(1.0));
    assert_eq!(values[10], Some(1.0));
    assert_eq!(values[11], Some(2.0));
    let all = plotted(
        "request.security(syminfo.tickerid, \"240\", ta.cum(1))",
        &chart,
        CannedData::default(),
    );
    assert_eq!(all[11], Some(3.0), "without the limit every candle counts");
}

#[test]
fn calc_bars_count_asks_the_source_for_that_many_candles_of_another_symbol() {
    let chart = hourly(12);
    let candles = [10.0, 20.0, 30.0]
        .iter()
        .enumerate()
        .map(|(index, close)| candle(index as i64 * 4 * HOUR, 4 * HOUR, *close, *close, 1.0))
        .collect();
    // Only a request carrying the count is answered.
    let data = CannedData::default().with("ETHUSDT", 14_400.0, 2, candles);
    let values = plotted(
        "request.security(\"ETHUSDT\", \"240\", close, calc_bars_count = 2)",
        &chart,
        data,
    );
    let mut expected = vec![None; 7];
    expected.extend([Some(20.0), Some(20.0), Some(20.0), Some(20.0), Some(30.0)]);
    assert_eq!(values, expected);
}

#[test]
fn calc_bars_count_in_a_request_is_not_reported_as_ignored() {
    let program = compiler::compile(
        "//@version=6\nindicator(\"T\")\nplot(request.security(syminfo.tickerid, \"D\", close, calc_bars_count = 10))",
        crate::pine::syntax::token::Limits::default(),
        &NoLibraries,
    );
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    assert!(
        program
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "PINE7001"),
        "{:?}",
        program.diagnostics
    );
}

/// The 4h candles of the chart's own symbol, as the data source holds them: the first opens at
/// 00:00 with an open (100) and a volume (999) no bar of a chart that starts at 02:00 has.
fn source_candles() -> CannedData {
    let first = candle(0, 4 * HOUR, 100.0, 105.0, 999.0);
    let second = candle(4 * HOUR, 4 * HOUR, 777.0, 778.0, 888.0);
    CannedData::default().with(&symbol().ticker_id, 14_400.0, 0, vec![first, second])
}

#[test]
fn a_chart_that_opens_inside_a_candle_gets_the_sources_whole_candle() {
    // The chart starts at 02:00, in the middle of the 00:00 candle.
    let chart: Vec<Kline> = (2..12).map(hourly_bar).collect();
    let read = |what: &str, options: &str| {
        plotted(
            &format!("request.security(syminfo.tickerid, \"240\", {what}{options})"),
            &chart,
            source_candles(),
        )
    };
    let open = read("open", "");
    // The candle completes on the chart bar of 03:00 (index 1) with the source's open and volume,
    // not the 02:00 bar's.
    assert_eq!(open[1], Some(100.0));
    assert_eq!(read("volume", "")[1], Some(999.0));
    // The next candle is folded from the chart's own bars (04:00-07:00), as before.
    assert_eq!(open[5], Some(hourly_bar(4).open));
    let ahead = read("open", ", lookahead = barmerge.lookahead_on");
    assert_eq!(ahead[0], Some(100.0), "from the chart's first bar on");
    assert_eq!(ahead[4], Some(hourly_bar(4).open));
}

#[test]
fn the_sources_whole_candle_is_not_taken_before_the_chart_has_all_of_it() {
    // Bars 01:00 and 02:00 only: the run does not have the candle's last hour, and the source's
    // candle, which does, must not leak into it.
    let chart: Vec<Kline> = (1..3).map(hourly_bar).collect();
    let values = plotted(
        "request.security(syminfo.tickerid, \"240\", open, lookahead = barmerge.lookahead_on)",
        &chart,
        source_candles(),
    );
    assert_eq!(values, [Some(hourly_bar(1).open); 2]);
}
